//! A loaded puppet and everything the panels show about it. Opens both the
//! native .aom and Live2D's .moc3 (bare or through a .model3.json bundle),
//! and saves .aom.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use ayatsuri_core::Puppet;
use ayatsuri_format::model::Binding;
use ayatsuri_format::{AomPayload, Meta, Model};
use egui::{ColorImage, Context, TextureHandle, TextureOptions};
use log::{debug, info, warn};
use serde::Deserialize;

#[derive(Deserialize)]
struct Model3 {
    #[serde(rename = "FileReferences")]
    file_references: FileReferences,
}

#[derive(Deserialize)]
struct FileReferences {
    #[serde(rename = "Moc", default)]
    moc: Option<String>,
    #[serde(rename = "Textures", default)]
    textures: Vec<String>,
    #[serde(rename = "DisplayInfo", default)]
    display_info: Option<String>,
    #[serde(rename = "Physics", default)]
    physics: Option<String>,
}

/// The `.cdi3.json` display info: ids to human names and parameter groups.
#[derive(Deserialize, Default)]
struct Cdi3 {
    #[serde(rename = "Parameters", default)]
    parameters: Vec<Cdi3Entry>,
    #[serde(rename = "ParameterGroups", default)]
    parameter_groups: Vec<Cdi3Entry>,
    #[serde(rename = "Parts", default)]
    parts: Vec<Cdi3Entry>,
    #[serde(rename = "CombinedParameters", default)]
    combined: Vec<Vec<String>>,
}

#[derive(Deserialize, Default, Clone)]
struct Cdi3Entry {
    #[serde(rename = "Id", default)]
    id: String,
    #[serde(rename = "GroupId", default)]
    group_id: String,
    #[serde(rename = "Name", default)]
    name: String,
}

/// What is selected across the palettes.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Sel {
    Part(usize),
    Deformer(usize),
    Mesh(usize),
    Parameter(usize),
}

/// One group of the Parameter palette, in display order.
pub struct ParamGroup {
    pub name: String,
    pub params: Vec<usize>,
    pub open: bool,
}

pub struct Session {
    pub name: String,
    /// Credited in the .aom meta block.
    pub author: String,
    /// Unsaved model changes; the close dialog hangs off this.
    pub modified: bool,
    /// Downscaled pixel copies (512px on the long side) for CPU previews.
    pub preview_tex: Vec<ayatsuri_render::TexData>,
    /// Rendered Parts palette previews, filled lazily a few per frame and
    /// dropped whenever the model is rebuilt.
    pub part_previews: HashMap<usize, Option<TextureHandle>>,
    /// The same for the Deformer palette: everything a deformer moves.
    pub deformer_previews: HashMap<usize, Option<TextureHandle>>,
    /// Animation clips; they ride inside the .aom payload.
    pub animations: Vec<ayatsuri_format::Animation>,
    pub current_anim: Option<usize>,
    pub puppet: Puppet,
    /// Downscaled for the Texture palette and the non-GPU fallback view.
    pub textures: Vec<Option<TextureHandle>>,
    /// Full-resolution pixels waiting to go to the GPU renderer, taken on
    /// the first frame after load.
    pub gpu_pending: Option<Vec<ayatsuri_render::TexData>>,
    pub texture_names: Vec<String>,
    /// The encoded images as imported, kept so saving .aom is lossless.
    pub texture_pngs: Vec<Vec<u8>>,
    /// Where this session saves; set by opening or saving an .aom.
    pub aom_path: Option<PathBuf>,
    /// Parameters changed since the last evaluate.
    pub dirty: bool,
    /// How long the last puppet evaluate took, for the Settings readout.
    pub last_update_ms: f32,
    /// Display names from the cdi3, by model index.
    pub param_names: Vec<String>,
    pub part_names: Vec<String>,
    /// Parameter palette grouping, cdi3 order; one trailing group holds the rest.
    pub groups: Vec<ParamGroup>,
    /// Every key value each parameter has anywhere in the model, sorted,
    /// for the key dots on the sliders.
    pub param_keys: Vec<Vec<f32>>,
    /// Per-part and per-mesh show/hide from the Parts palette eye column.
    pub part_shown: Vec<bool>,
    pub mesh_shown: Vec<bool>,
    pub deformer_shown: Vec<bool>,
    /// Label colors on parameter rows, palette only, saved in .aom.
    pub param_colors: Vec<Option<[u8; 3]>>,
    /// Full-resolution egui textures, decoded only when something actually
    /// zooms into them (mesh edit); the palette thumbs stay 256px.
    full_tex: Vec<Option<TextureHandle>>,
    /// Parameter pairs shown as one 2D pad (from cdi3 CombinedParameters).
    pub combined: Vec<(usize, usize)>,
    undo: Vec<Model>,
    redo: Vec<Model>,
}

impl Session {
    /// A fresh model straight from the New Model dialog: no textures yet,
    /// just the canvas and whatever starter parameters were asked for.
    pub fn create(
        ctx: &Context,
        name: String,
        author: String,
        model: Model,
    ) -> Result<Self, String> {
        let source = PathBuf::from(format!("{name}.aom"));
        let mut s = Self::from_parts(ctx, name, model, Vec::new(), Vec::new(), None, &source)?;
        s.author = author;
        s.modified = true;
        Ok(s)
    }

    /// Open a model: `.aom`, a bare `.moc3`, or a `.model3.json` bundle.
    pub fn load(ctx: &Context, path: &Path) -> Result<Self, String> {
        let started = std::time::Instant::now();
        let ext = path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
        let session = match ext.as_str() {
            "aom" => Self::load_aom(ctx, path)?,
            "psd" | "kra" => Self::load_paint(ctx, path, &ext)?,
            _ => Self::load_moc3(ctx, path)?,
        };
        info!("opened {} in {:.0} ms", path.display(), started.elapsed().as_secs_f32() * 1000.0);
        Ok(session)
    }

    /// A paint document becomes a brand new model: groups as parts, layers
    /// as quad meshes, pixels packed into atlases. It starts unsaved, since
    /// the .psd or .kra is the source and not a model file.
    fn load_paint(ctx: &Context, path: &Path, ext: &str) -> Result<Self, String> {
        let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let imported = if ext == "kra" {
            ayatsuri_import::from_kra(&bytes)
        } else {
            ayatsuri_import::from_psd(&bytes)
        }
        .map_err(|e| format!("{}: {e}", path.display()))?;
        info!(
            "{} layers became {} meshes in {} parts on {} texture page(s){}{}",
            ext.to_uppercase(),
            imported.model.art_meshes.len(),
            imported.model.parts.len(),
            imported.textures.len(),
            if imported.skipped > 0 {
                format!(", {} empty/hidden/unsupported layer(s) skipped", imported.skipped)
            } else {
                String::new()
            },
            if imported.scaled > 0 {
                format!(", {} oversized layer(s) downscaled", imported.scaled)
            } else {
                String::new()
            }
        );
        let name = path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "model".into());
        let tex_names = (0..imported.textures.len()).map(|i| format!("texture_{i:02}")).collect();
        let part_names = imported.part_names.clone();
        let mut s =
            Self::from_parts(ctx, name, imported.model, imported.textures, tex_names, None, path)?;
        if part_names.len() == s.part_names.len() {
            s.part_names = part_names;
        }
        s.modified = true;
        Ok(s)
    }

    fn load_aom(ctx: &Context, path: &Path) -> Result<Self, String> {
        let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let payload =
            ayatsuri_format::read(&bytes).map_err(|e| format!("{}: {e}", path.display()))?;
        debug!("aom payload: {} bytes on disk, {} textures", bytes.len(), payload.textures.len());
        let name = if payload.meta.name.is_empty() {
            path.file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_else(|| "model".into())
        } else {
            payload.meta.name.clone()
        };
        let pngs: Vec<Vec<u8>> = payload.textures.into_iter().map(|b| b.into_vec()).collect();
        let tex_names = (0..pngs.len()).map(|i| format!("texture_{i:02}")).collect();
        let mut s = Self::from_parts(ctx, name, payload.model, pngs, tex_names, None, path)?;
        s.aom_path = Some(path.to_path_buf());
        s.author = payload.meta.author;
        s.animations = payload.animations;
        if let Some(d) = payload.display {
            let n = s.puppet.model().parameters.len();
            if d.param_names.len() == n {
                s.param_names = d.param_names;
            }
            if d.param_colors.len() == n {
                s.param_colors = d
                    .param_colors
                    .iter()
                    .map(|&c| {
                        (c >= 0).then_some([
                            ((c >> 16) & 0xff) as u8,
                            ((c >> 8) & 0xff) as u8,
                            (c & 0xff) as u8,
                        ])
                    })
                    .collect();
            }
            if !d.groups.is_empty() {
                s.groups = d
                    .groups
                    .into_iter()
                    .map(|(name, params)| ParamGroup {
                        name,
                        params: params.into_iter().map(|p| p as usize).filter(|&p| p < n).collect(),
                        open: true,
                    })
                    .collect();
            }
        }
        Ok(s)
    }

    fn load_moc3(ctx: &Context, path: &Path) -> Result<Self, String> {
        let bundle = resolve_bundle(path)?;
        let bytes =
            std::fs::read(&bundle.moc).map_err(|e| format!("{}: {e}", bundle.moc.display()))?;
        let mut model = ayatsuri_import::from_moc3(&bytes)
            .map_err(|e| format!("{}: {e}", bundle.moc.display()))?;
        // the pendulum rig rides along in physics3.json when the bundle has one
        let physics_path = bundle.physics.clone().or_else(|| {
            let stem = bundle.moc.file_stem()?.to_string_lossy().into_owned();
            let sibling = bundle.moc.with_file_name(format!("{stem}.physics3.json"));
            sibling.exists().then_some(sibling)
        });
        if let Some(pp) = physics_path {
            match std::fs::read(&pp)
                .map_err(|e| e.to_string())
                .and_then(|b| ayatsuri_import::from_physics3(&b).map_err(|e| e.to_string()))
            {
                Ok(rig) => {
                    info!(
                        "physics: {} groups, {} fps from {}",
                        rig.groups.len(),
                        rig.fps,
                        pp.display()
                    );
                    model.physics = Some(rig);
                }
                Err(e) => warn!("physics {}: {e}", pp.display()),
            }
        }
        info!(
            "imported {}: {} parts, {} deformers, {} meshes, {} parameters, {} glues",
            bundle.moc.display(),
            model.parts.len(),
            model.deformers.len(),
            model.art_meshes.len(),
            model.parameters.len(),
            model.glues.len()
        );

        let mut pngs = Vec::with_capacity(bundle.textures.len());
        let mut names = Vec::with_capacity(bundle.textures.len());
        for p in &bundle.textures {
            match std::fs::read(p) {
                Ok(b) => pngs.push(b),
                Err(e) => {
                    warn!("texture {}: {e}", p.display());
                    pngs.push(Vec::new());
                }
            }
            names.push(p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default());
        }

        let cdi: Option<Cdi3> = bundle
            .display_info
            .as_deref()
            .and_then(|p| std::fs::read(p).ok())
            .and_then(|b| serde_json::from_slice(&b).ok());

        let name = bundle
            .moc
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "model".into());
        Self::from_parts(ctx, name, model, pngs, names, cdi, path)
    }

    /// Build the session from a model and its encoded textures, whatever
    /// file they came from.
    fn from_parts(
        ctx: &Context,
        name: String,
        model: Model,
        pngs: Vec<Vec<u8>>,
        texture_names: Vec<String>,
        cdi: Option<Cdi3>,
        source: &Path,
    ) -> Result<Self, String> {
        let mut textures = Vec::with_capacity(pngs.len());
        let mut full = Vec::with_capacity(pngs.len());
        for (i, png) in pngs.iter().enumerate() {
            match decode_texture(
                ctx,
                texture_names.get(i).map(String::as_str).unwrap_or("texture"),
                png,
            ) {
                Ok((thumb, data)) => {
                    debug!("texture {i}: {}x{} decoded", data.width, data.height);
                    textures.push(Some(thumb));
                    full.push(data);
                }
                Err(e) => {
                    warn!("texture {i}: {e}");
                    textures.push(None);
                    full.push(ayatsuri_render::TexData { width: 1, height: 1, rgba: vec![255; 4] });
                }
            }
        }
        if textures.is_empty() {
            warn!("no textures found; meshes draw untextured");
        }

        // small copies for the CPU previews; the full set goes to the GPU
        let preview_tex: Vec<ayatsuri_render::TexData> = full
            .iter()
            .map(|t| {
                let long = t.width.max(t.height).max(1);
                if long <= 512 {
                    return t.clone();
                }
                let k = 512.0 / long as f32;
                let (w, h) =
                    (((t.width as f32 * k) as u32).max(1), ((t.height as f32 * k) as u32).max(1));
                match image::RgbaImage::from_raw(t.width, t.height, t.rgba.clone()) {
                    Some(img) => {
                        let small = image::imageops::resize(
                            &img,
                            w,
                            h,
                            image::imageops::FilterType::Triangle,
                        );
                        ayatsuri_render::TexData { width: w, height: h, rgba: small.into_raw() }
                    }
                    None => t.clone(),
                }
            })
            .collect();

        let cdi = cdi.unwrap_or_default();
        let param_name_of: HashMap<&str, &Cdi3Entry> =
            cdi.parameters.iter().map(|e| (e.id.as_str(), e)).collect();
        let part_name_of: HashMap<&str, &str> =
            cdi.parts.iter().map(|e| (e.id.as_str(), e.name.as_str())).collect();
        let param_names: Vec<String> = model
            .parameters
            .iter()
            .map(|p| {
                param_name_of
                    .get(p.id.as_str())
                    .filter(|e| !e.name.is_empty())
                    .map_or(p.id.clone(), |e| e.name.clone())
            })
            .collect();
        let part_names: Vec<String> = model
            .parts
            .iter()
            .map(|p| {
                part_name_of
                    .get(p.id.as_str())
                    .filter(|n| !n.is_empty())
                    .map_or(p.id.clone(), |n| (*n).to_string())
            })
            .collect();

        // groups in cdi3 order; parameters without one trail in "Other"
        let index_of: HashMap<&str, usize> =
            model.parameters.iter().enumerate().map(|(i, p)| (p.id.as_str(), i)).collect();
        let mut groups: Vec<ParamGroup> = Vec::new();
        let mut grouped = vec![false; model.parameters.len()];
        for g in &cdi.parameter_groups {
            let params: Vec<usize> = cdi
                .parameters
                .iter()
                .filter(|e| e.group_id == g.id)
                .filter_map(|e| index_of.get(e.id.as_str()).copied())
                .collect();
            for &p in &params {
                if let Some(f) = grouped.get_mut(p) {
                    *f = true;
                }
            }
            if !params.is_empty() {
                groups.push(ParamGroup {
                    name: if g.name.is_empty() { g.id.clone() } else { g.name.clone() },
                    params,
                    open: true,
                });
            }
        }
        let rest: Vec<usize> = (0..model.parameters.len())
            .filter(|&i| !grouped.get(i).copied().unwrap_or(false))
            .collect();
        if !rest.is_empty() {
            let name = if groups.is_empty() { String::new() } else { "Other".into() };
            groups.push(ParamGroup { name, params: rest, open: true });
        }

        let param_keys = keys_of(&model);

        let mut combined: Vec<(usize, usize)> = cdi
            .combined
            .iter()
            .filter_map(|pair| {
                let a = index_of.get(pair.first()?.as_str())?;
                let b = index_of.get(pair.get(1)?.as_str())?;
                Some((*a, *b))
            })
            .collect();
        // the conventional X/Y pairs combine even when the cdi3 is silent
        for (ida, idb) in [
            ("ParamAngleX", "ParamAngleY"),
            ("ParamBodyAngleX", "ParamBodyAngleY"),
            ("ParamEyeBallX", "ParamEyeBallY"),
        ] {
            if let (Some(&a), Some(&b)) = (index_of.get(ida), index_of.get(idb))
                && !combined.iter().any(|&(x, y)| x == a || y == a || x == b || y == b)
            {
                combined.push((a, b));
            }
        }

        let n_parts = model.parts.len();
        let n_meshes = model.art_meshes.len();
        let n_defs = model.deformers.len();
        let n_params = model.parameters.len();
        let n_tex = texture_names.len();
        let mut puppet = Puppet::new(model);
        puppet.update();
        crate::thumb::ensure(&puppet, &full, source);
        Ok(Self {
            name,
            puppet,
            textures,
            gpu_pending: Some(full),
            texture_names,
            texture_pngs: pngs,
            aom_path: None,
            dirty: false,
            last_update_ms: 0.0,
            param_names,
            part_names,
            groups,
            param_keys,
            author: String::new(),
            modified: false,
            preview_tex,
            part_previews: HashMap::new(),
            deformer_previews: HashMap::new(),
            animations: Vec::new(),
            current_anim: None,
            part_shown: vec![true; n_parts],
            mesh_shown: vec![true; n_meshes],
            deformer_shown: vec![true; n_defs],
            param_colors: vec![None; n_params],
            full_tex: vec![None; n_tex],
            combined,
            undo: Vec::new(),
            redo: Vec::new(),
        })
    }

    /// Run a model edit with undo. The closure returns whether it changed
    /// anything; unchanged edits cost nothing.
    /// The full-size texture for one slot, decoding it on first use.
    pub fn full_texture(&mut self, ctx: &Context, i: usize) -> Option<&TextureHandle> {
        if self.full_tex.get(i).is_some_and(|t| t.is_none())
            && let Some(png) = self.texture_pngs.get(i)
            && let Ok(img) = image::load_from_memory(png)
        {
            let rgba = img.to_rgba8();
            let size = [rgba.width() as usize, rgba.height() as usize];
            let color = ColorImage::from_rgba_unmultiplied(size, rgba.as_raw());
            let opts = TextureOptions {
                mipmap_mode: Some(egui::TextureFilter::Linear),
                ..TextureOptions::LINEAR
            };
            let handle = ctx.load_texture(format!("full-{i}"), color, opts);
            debug!("full texture {i} decoded: {}x{}", size[0], size[1]);
            if let Some(slot) = self.full_tex.get_mut(i) {
                *slot = Some(handle);
            }
        }
        self.full_tex.get(i).and_then(|t| t.as_ref())
    }

    pub fn edit(&mut self, f: impl FnOnce(&mut Model) -> bool) -> bool {
        let before = self.puppet.model().clone();
        let mut after = before.clone();
        if !f(&mut after) {
            return false;
        }
        self.undo.push(before);
        if self.undo.len() > 16 {
            self.undo.remove(0);
        }
        self.redo.clear();
        self.rebuild_from(after);
        self.modified = true;
        true
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    pub fn undo(&mut self) {
        if let Some(previous) = self.undo.pop() {
            self.redo.push(self.puppet.model().clone());
            self.rebuild_from(previous);
            self.modified = true;
            log::info!("undo ({} left)", self.undo.len());
        }
    }

    pub fn redo(&mut self) {
        if let Some(next) = self.redo.pop() {
            self.undo.push(self.puppet.model().clone());
            self.rebuild_from(next);
            self.modified = true;
            log::info!("redo");
        }
    }

    /// Swap in an edited model, carrying over values and visibility.
    fn rebuild_from(&mut self, model: Model) {
        let values: Vec<f32> =
            (0..self.puppet.model().parameters.len()).map(|i| self.puppet.value(i)).collect();
        self.param_keys = keys_of(&model);
        let mut puppet = Puppet::new(model);
        for (i, v) in values.iter().enumerate() {
            puppet.set_value(i, *v);
        }
        for (i, shown) in self.part_shown.iter().enumerate() {
            puppet.set_part_opacity(i, if *shown { 1.0 } else { 0.0 });
        }
        for (i, shown) in self.mesh_shown.iter().enumerate() {
            puppet.set_mesh_visible(i, *shown);
        }
        // the rebuilt model may have gained deformers
        self.deformer_shown.resize(puppet.model().deformers.len(), true);
        self.part_previews.clear();
        self.deformer_previews.clear();
        self.param_colors.resize(puppet.model().parameters.len(), None);
        for (i, shown) in self.deformer_shown.iter().enumerate() {
            puppet.set_deformer_visible(i, *shown);
        }
        puppet.update();
        self.puppet = puppet;
        self.dirty = false;
    }

    /// Save this session as `.aom`, atomically, and remember the path.
    /// The model, textures, animations and display info as one payload.
    fn payload(&self) -> AomPayload {
        AomPayload {
            meta: Meta {
                name: self.name.clone(),
                author: self.author.clone(),
                tool: format!("Ayatsuri2D {}", env!("CARGO_PKG_VERSION")),
            },
            model: self.puppet.model().clone(),
            textures: self
                .texture_pngs
                .iter()
                .map(|b| serde_bytes::ByteBuf::from(b.clone()))
                .collect(),
            animations: self.animations.clone(),
            display: Some(ayatsuri_format::DisplayInfo {
                param_names: self.param_names.clone(),
                param_colors: self
                    .param_colors
                    .iter()
                    .map(|c| match c {
                        Some([r, g, b]) => ((*r as i32) << 16) | ((*g as i32) << 8) | (*b as i32),
                        None => -1,
                    })
                    .collect(),
                groups: self
                    .groups
                    .iter()
                    .map(|g| (g.name.clone(), g.params.iter().map(|&p| p as u32).collect()))
                    .collect(),
            }),
        }
    }

    /// The periodic safety copy: snapshot the payload here, then encode and
    /// write it on a thread so the frame never waits on deflate. One file
    /// per model, overwritten each time.
    pub fn autosave(&self) {
        let Some(dir) =
            crate::logging::log_path().and_then(|p| p.parent().map(|d| d.join("recovery")))
        else {
            return;
        };
        let safe: String = self
            .name
            .chars()
            .map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '_' })
            .collect();
        let payload = self.payload();
        std::thread::spawn(move || {
            let _ = std::fs::create_dir_all(&dir);
            let path = dir.join(format!("{safe}-autosave.aom"));
            let tmp = path.with_extension("aom.tmp");
            match ayatsuri_format::write(&payload) {
                Ok(bytes) => {
                    if std::fs::write(&tmp, bytes)
                        .and_then(|_| std::fs::rename(&tmp, &path))
                        .is_ok()
                    {
                        debug!("autosaved to {}", path.display());
                    }
                }
                Err(e) => warn!("autosave failed: {e}"),
            }
        });
    }

    /// Write a recovery copy without touching the session's own path, so a
    /// crash never costs the user their work. Returns where it went.
    pub fn write_recovery(&self) -> Option<PathBuf> {
        let dir = crate::logging::log_path()?.parent()?.join("recovery");
        std::fs::create_dir_all(&dir).ok()?;
        let secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let safe: String = self
            .name
            .chars()
            .map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '_' })
            .collect();
        let path = dir.join(format!("{safe}-{secs}.aom"));
        let bytes = ayatsuri_format::write(&self.payload()).ok()?;
        std::fs::write(&path, bytes).ok()?;
        Some(path)
    }

    pub fn save_aom(&mut self, path: &Path) -> Result<(), String> {
        let started = std::time::Instant::now();
        let payload = self.payload();
        let bytes = ayatsuri_format::write(&payload).map_err(|e| e.to_string())?;
        let tmp = path.with_extension("aom.tmp");
        std::fs::write(&tmp, &bytes).map_err(|e| format!("{}: {e}", tmp.display()))?;
        std::fs::rename(&tmp, path).map_err(|e| format!("{}: {e}", path.display()))?;
        info!(
            "saved {} ({:.1} MB) in {:.0} ms",
            path.display(),
            bytes.len() as f32 / (1024.0 * 1024.0),
            started.elapsed().as_secs_f32() * 1000.0
        );
        self.aom_path = Some(path.to_path_buf());
        self.modified = false;
        Ok(())
    }
}

struct Bundle {
    moc: PathBuf,
    textures: Vec<PathBuf>,
    display_info: Option<PathBuf>,
    physics: Option<PathBuf>,
}

/// From whichever file was picked, find the `.moc3`, the texture list and the
/// display info: a `.model3.json` names all three; a bare `.moc3` looks for a
/// sibling json that references it.
fn resolve_bundle(path: &Path) -> Result<Bundle, String> {
    let from_json = |json_path: &Path| -> Result<Bundle, String> {
        let dir = json_path.parent().unwrap_or(Path::new("."));
        let m: Model3 =
            serde_json::from_slice(&std::fs::read(json_path).map_err(|e| e.to_string())?)
                .map_err(|e| format!("{}: {e}", json_path.display()))?;
        let moc = m
            .file_references
            .moc
            .ok_or_else(|| format!("{}: no Moc reference", json_path.display()))?;
        Ok(Bundle {
            moc: dir.join(moc),
            textures: m.file_references.textures.iter().map(|t| dir.join(t)).collect(),
            display_info: m.file_references.display_info.map(|d| dir.join(d)),
            physics: m.file_references.physics.map(|d| dir.join(d)),
        })
    };

    if path.extension().is_some_and(|e| e.eq_ignore_ascii_case("json")) {
        return from_json(path);
    }

    // a bare .moc3: scan the directory for a model3.json that points at it
    let dir = path.parent().unwrap_or(Path::new("."));
    let file_name = path.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let p = entry.path();
            if !p.file_name().is_some_and(|n| n.to_string_lossy().ends_with(".model3.json")) {
                continue;
            }
            if let Ok(b) = from_json(&p)
                && b.moc.file_name().is_some_and(|n| n.to_string_lossy() == file_name)
            {
                debug!("bundle from {}", p.display());
                return Ok(b);
            }
        }
    }
    Ok(Bundle { moc: path.to_path_buf(), textures: Vec::new(), display_info: None, physics: None })
}

fn decode_texture(
    ctx: &Context,
    name: &str,
    bytes: &[u8],
) -> Result<(TextureHandle, ayatsuri_render::TexData), String> {
    if bytes.is_empty() {
        return Err("empty".into());
    }
    let img = image::load_from_memory(bytes).map_err(|e| e.to_string())?.into_rgba8();
    let data = ayatsuri_render::TexData {
        width: img.width(),
        height: img.height(),
        rgba: img.as_raw().clone(),
    };
    let thumb = image::imageops::resize(&img, 256, 256, image::imageops::FilterType::Triangle);
    let color = ColorImage::from_rgba_unmultiplied([256, 256], thumb.as_raw());
    // mipmaps matter here: the editor draws these egui-side in mesh edit
    // mode and CPU fallback, usually well below 1:1 scale
    let opts =
        TextureOptions { mipmap_mode: Some(egui::TextureFilter::Linear), ..TextureOptions::LINEAR };
    Ok((ctx.load_texture(name.to_string(), color, opts), data))
}

/// Every key each parameter has in any binding axis, sorted and deduped;
/// the dots on the sliders.
fn keys_of(model: &Model) -> Vec<Vec<f32>> {
    let mut param_keys: Vec<Vec<f32>> = vec![Vec::new(); model.parameters.len()];
    {
        let mut add = |binding: &Binding| {
            for axis in &binding.axes {
                if let Some(keys) = param_keys.get_mut(axis.parameter as usize) {
                    keys.extend_from_slice(&axis.keys);
                }
            }
        };
        for p in &model.parts {
            add(&p.binding);
        }
        for d in &model.deformers {
            add(&d.binding);
        }
        for m in &model.art_meshes {
            add(&m.binding);
        }
        for g in &model.glues {
            add(&g.binding);
        }
    }
    for keys in &mut param_keys {
        keys.sort_by(f32::total_cmp);
        keys.dedup_by(|a, b| (*a - *b).abs() < 1e-5);
    }
    param_keys
}
