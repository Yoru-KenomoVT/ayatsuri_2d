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
    #[serde(rename = "Groups", default)]
    groups: Vec<Group3>,
}

/// One model3.json group: what it targets, its name, the ids in it.
#[derive(Deserialize)]
struct Group3 {
    #[serde(rename = "Target", default)]
    target: String,
    #[serde(rename = "Name", default)]
    name: String,
    #[serde(rename = "Ids", default)]
    ids: Vec<String>,
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
    #[serde(rename = "Expressions", default)]
    expressions: Vec<ExpRef>,
    /// Motion groups: group name to its files.
    #[serde(rename = "Motions", default)]
    motions: std::collections::BTreeMap<String, Vec<MotionRef>>,
}

#[derive(Deserialize)]
struct MotionRef {
    #[serde(rename = "File", default)]
    file: String,
}

/// One model3.json expression entry.
#[derive(Deserialize)]
struct ExpRef {
    #[serde(rename = "Name", default)]
    name: String,
    #[serde(rename = "File", default)]
    file: String,
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

/// Label colours travel as 0xRRGGBB, -1 for none.
fn pack_rgb(c: Option<[u8; 3]>) -> i32 {
    match c {
        Some([r, g, b]) => ((r as i32) << 16) | ((g as i32) << 8) | (b as i32),
        None => -1,
    }
}

fn unpack_rgb(c: i32) -> Option<[u8; 3]> {
    (c >= 0).then_some([((c >> 16) & 0xff) as u8, ((c >> 8) & 0xff) as u8, (c & 0xff) as u8])
}

/// One group of the Parameter palette, in display order.
pub struct ParamGroup {
    pub name: String,
    pub params: Vec<usize>,
    pub open: bool,
    /// Label colour on the folder header, like a parameter's.
    pub color: Option<[u8; 3]>,
}

pub struct Session {
    pub name: String,
    /// Credited in the .aom meta block.
    pub author: String,
    /// Unsaved model changes; the close dialog hangs off this.
    pub modified: bool,
    /// Each mesh's uvs and triangles as the file had them, for Reset mesh.
    pub original_meshes: Vec<(Vec<f32>, Vec<u16>)>,
    /// Downscaled pixel copies (512px on the long side) for CPU previews.
    pub preview_tex: Vec<ayatsuri_render::TexData>,
    /// Rendered Parts palette previews, filled lazily a few per frame and
    /// dropped whenever the model is rebuilt.
    pub part_previews: HashMap<usize, Option<TextureHandle>>,
    /// The same for the Deformer palette: everything a deformer moves.
    pub deformer_previews: HashMap<usize, Option<TextureHandle>>,
    /// One art mesh alone, for the Parts palette's hover preview.
    pub mesh_previews: HashMap<usize, Option<TextureHandle>>,
    /// Animation clips; they ride inside the .aom payload.
    pub animations: Vec<ayatsuri_format::Animation>,
    /// Expressions (exp3.json) that ride with the model.
    pub expressions: Vec<ayatsuri_format::Expression>,
    pub current_anim: Option<usize>,
    pub puppet: Puppet,
    /// Downscaled for the Texture palette and the non-GPU fallback view.
    pub textures: Vec<Option<TextureHandle>>,
    /// Full-resolution pixels waiting to go to the GPU renderer, taken on
    /// the first frame after load.
    /// Textures for the GPU renderer, re-sent every frame until it confirms
    /// the upload, then dropped to free the memory.
    pub gpu_pending: Option<std::sync::Arc<Vec<ayatsuri_render::TexData>>>,
    pub gpu_uploaded: std::sync::Arc<std::sync::atomic::AtomicBool>,
    /// Single pages changed since (the ArtPath swatch), waiting for the GPU.
    pub gpu_pages: Option<std::sync::Arc<Vec<(usize, ayatsuri_render::TexData)>>>,
    pub gpu_pages_uploaded: std::sync::Arc<std::sync::atomic::AtomicBool>,
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
    /// Display names per art mesh (the PSD layer names); IDs stay ASCII.
    pub mesh_names: Vec<String>,
    /// Parameters the user is dragging this frame; physics stays off them.
    pub params_held: Vec<usize>,
    /// Parameters picked in the palette (Ctrl/Shift+click), in pick order.
    pub param_picked: Vec<usize>,
    /// Parameter palette grouping, cdi3 order; one trailing group holds the rest.
    pub groups: Vec<ParamGroup>,
    /// Every key value each parameter has anywhere in the model, sorted,
    /// for the key dots on the sliders.
    pub param_keys: Vec<Vec<f32>>,
    /// Per-part and per-mesh show/hide from the Parts palette eye column.
    pub part_shown: Vec<bool>,
    pub mesh_shown: Vec<bool>,
    /// Name, visibility and reset shape of meshes an edit removed, by id,
    /// so undoing the delete gets them back.
    departed: HashMap<String, MeshExtras>,
    pub deformer_shown: Vec<bool>,
    /// Label colors on parameter rows, palette only, saved in .aom.
    pub param_colors: Vec<Option<[u8; 3]>>,
    /// Full-resolution egui textures, decoded only when something actually
    /// zooms into them (mesh edit); the palette thumbs stay 256px.
    full_tex: Vec<Option<TextureHandle>>,
    /// Parameter pairs shown as one 2D pad (from cdi3 CombinedParameters).
    pub combined: Vec<(usize, usize)>,
    /// The model3.json Groups the model came with (target, name, ids);
    /// None when it had none, and export then writes the standard ones.
    pub model_groups: Option<Vec<(String, String, Vec<String>)>>,
    /// Guide lines on the canvas: (vertical, canvas pixels). Saved with the
    /// model, not part of undo.
    pub guides: Vec<(bool, f32)>,
    undo: Vec<Model>,
    redo: Vec<Model>,
}

/// What the session keeps beside each mesh: display name, shown, and the
/// (uvs, indices) Reset mesh returns to.
type MeshExtras = (String, bool, (Vec<f32>, Vec<u16>));

/// Names from an import, one per new row: missing ones are empty (the
/// palette then shows the id).
fn pad(mut names: Vec<String>, n: usize) -> Vec<String> {
    names.resize(n, String::new());
    names
}

/// The first `{prefix}{n}` id not in `taken`, which then holds it.
fn free_id(prefix: &str, taken: &mut std::collections::HashSet<String>) -> String {
    let mut n = 0usize;
    loop {
        let id = format!("{prefix}{n}");
        if taken.insert(id.clone()) {
            return id;
        }
        n += 1;
    }
}

/// Add an imported paint document's parts and meshes to `base`, on top of
/// what is there. Texture pages are numbered after `tex_offset`, ids are
/// made unique, and positions move from the document's canvas onto the
/// model's (top-left pixels lined up). A model with no art and no
/// deformers yet takes the document's canvas as is.
pub(crate) fn merge_paint(base: &mut Model, add: Model, tex_offset: u32) {
    let fresh = base.art_meshes.is_empty() && base.deformers.is_empty();
    if fresh {
        base.canvas = add.canvas;
    }
    let (from, to) = (add.canvas, base.canvas);
    let part_offset = base.parts.len() as u32;
    let mesh_offset = base.art_meshes.len() as u32;

    let mut part_ids: std::collections::HashSet<String> =
        base.parts.iter().map(|p| p.id.clone()).collect();
    for mut part in add.parts {
        part.id = free_id("Part", &mut part_ids);
        part.parent_part = part.parent_part.map(|p| p + part_offset);
        base.parts.push(part);
    }
    let mut mesh_ids: std::collections::HashSet<String> =
        base.art_meshes.iter().map(|m| m.id.clone()).collect();
    let mut orders: Vec<i32> = Vec::new();
    for mut mesh in add.art_meshes {
        mesh.id = free_id("ArtMesh", &mut mesh_ids);
        mesh.parent_part = mesh.parent_part.map(|p| p + part_offset);
        mesh.texture += tex_offset;
        for m in &mut mesh.masks {
            *m += mesh_offset;
        }
        if !fresh {
            for [x, y] in mesh.positions.as_chunks_mut::<2>().0 {
                let px = *x * from.pixels_per_unit + from.origin_x;
                let py = *y * from.pixels_per_unit + from.origin_y;
                *x = (px - to.origin_x) / to.pixels_per_unit.max(1e-6);
                *y = (py - to.origin_y) / to.pixels_per_unit.max(1e-6);
            }
        }
        orders.extend(mesh.draw_orders.iter().map(|o| o.round() as i32));
        base.art_meshes.push(mesh);
    }
    // a model read from moc3 orders meshes through its draw order groups;
    // new meshes join the root group or they would never be drawn
    let added = base.art_meshes.len() as u32 - mesh_offset;
    if let Some(root) = base.draw_groups.first_mut()
        && added > 0
    {
        for i in mesh_offset..base.art_meshes.len() as u32 {
            root.items.push(ayatsuri_format::model::DrawItem {
                part: false,
                object: i,
                child_group: None,
            });
        }
        root.total += added;
        if let (Some(lo), Some(hi)) = (orders.iter().min(), orders.iter().max()) {
            root.min_order = root.min_order.min(*lo);
            root.max_order = root.max_order.max(*hi);
        }
    }
}

/// What a re-import did: meshes given new art, layers with no mesh to go
/// to, and meshes whose layer is gone from the file.
#[derive(Debug, Default)]
pub struct Reimported {
    pub file: String,
    pub updated: usize,
    pub new_layers: Vec<String>,
    pub missing: Vec<String>,
}

/// Transparent room kept around re-imported art past what the mesh reaches.
const REIMPORT_PAD: u32 = 8;

/// The model side of `reimport_paint`, for a document already read.
fn reimport_into(
    s: &mut Session,
    ctx: &Context,
    doc: ayatsuri_import::Document,
) -> Result<Reimported, String> {
    let model = s.puppet.model().clone();
    // each mesh's key: its folder path and name, plus which repeat it is
    let part_path = |part: Option<u32>| -> String {
        let mut names = Vec::new();
        let mut at = part;
        let mut hops = 0;
        while let Some(p) = at {
            names.push(s.part_names.get(p as usize).cloned().unwrap_or_default());
            at = model.parts.get(p as usize).and_then(|x| x.parent_part);
            hops += 1;
            if hops > model.parts.len() {
                break;
            }
        }
        names.reverse();
        names.join("/")
    };
    let mut seen: HashMap<String, usize> = HashMap::new();
    let mut mesh_by_key: HashMap<(String, usize), usize> = HashMap::new();
    for (i, m) in model.art_meshes.iter().enumerate() {
        let key = format!("{}/{}", part_path(m.parent_part), s.mesh_name(i));
        let n = seen.entry(key.clone()).or_insert(0);
        mesh_by_key.insert((key, *n), i);
        *n += 1;
    }
    // where each mesh rests, in canvas pixels, to line the new art up with
    let c = model.canvas;
    let mut rest = Puppet::new(model.clone());
    rest.update();
    let rest_px = |i: usize| -> Vec<(f32, f32)> {
        rest.mesh_geometry(i)
            .map(|g| {
                g.positions
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .map(|&[x, y]| {
                        (x * c.pixels_per_unit + c.origin_x, y * c.pixels_per_unit + c.origin_y)
                    })
                    .collect()
            })
            .unwrap_or_default()
    };

    let mut report = Reimported::default();
    let mut matched_meshes = std::collections::HashSet::new();
    let mut layers = Vec::new();
    let mut seen: HashMap<String, usize> = HashMap::new();
    for layer in doc.layers {
        if !layer.visible || layer.opacity <= 0.0 {
            continue;
        }
        let key = format!("{}/{}", layer.groups.join("/"), layer.name);
        let n = seen.entry(key.clone()).or_insert(0);
        let hit = mesh_by_key.get(&(key.clone(), *n)).copied();
        *n += 1;
        let Some(mi) = hit else {
            report.new_layers.push(key);
            continue;
        };
        matched_meshes.insert(mi);
        // pad the layer so every point of its mesh lands on this art
        let pts = rest_px(mi);
        let (lx, ly) = (layer.x as f32, layer.y as f32);
        let (lw, lh) = (layer.width as f32, layer.height as f32);
        let reach = |a: f32| a.max(0.0).ceil() as u32 + REIMPORT_PAD;
        let left = reach(pts.iter().map(|p| lx - p.0).fold(0.0, f32::max));
        let top = reach(pts.iter().map(|p| ly - p.1).fold(0.0, f32::max));
        let right = reach(pts.iter().map(|p| p.0 - (lx + lw)).fold(0.0, f32::max));
        let bottom = reach(pts.iter().map(|p| p.1 - (ly + lh)).fold(0.0, f32::max));
        let (w, h) = (layer.width + left + right, layer.height + top + bottom);
        let mut rgba = vec![0u8; (w * h * 4) as usize];
        for row in 0..layer.height {
            let src = (row * layer.width * 4) as usize;
            let dst = (((row + top) * w + left) * 4) as usize;
            let len = (layer.width * 4) as usize;
            if let (Some(from), Some(to)) =
                (layer.rgba.get(src..src + len), rgba.get_mut(dst..dst + len))
            {
                to.copy_from_slice(from);
            }
        }
        // an all but invisible pixel in each corner keeps the importer's
        // trim from cutting the padding away
        for (x, y) in [(0, 0), (w - 1, 0), (0, h - 1), (w - 1, h - 1)] {
            if let Some(a) = rgba.get_mut(((y * w + x) * 4 + 3) as usize) {
                *a = 1;
            }
        }
        layers.push(ayatsuri_import::Layer {
            name: mi.to_string(),
            groups: Vec::new(),
            x: layer.x - left as i32,
            y: layer.y - top as i32,
            width: w,
            height: h,
            rgba,
            visible: true,
            opacity: layer.opacity,
            clip_to_below: false,
        });
    }
    report.missing = (0..model.art_meshes.len())
        .filter(|i| !matched_meshes.contains(i))
        .map(|i| s.mesh_name(i))
        .collect();
    if layers.is_empty() {
        return Err("no layer in the file matches a mesh of this model (by folder and name)".into());
    }

    let built = ayatsuri_import::from_layers(&ayatsuri_import::Document {
        width: doc.width,
        height: doc.height,
        layers,
    })
    .map_err(|e| e.to_string())?;
    let nc = built.model.canvas;
    let page_offset = s.texture_pngs.len() as u32;
    let mut next = model.clone();
    let mut originals = s.original_meshes.clone();
    for (j, quad) in built.model.art_meshes.iter().enumerate() {
        let Some(mi) = built.mesh_names.get(j).and_then(|n| n.parse::<usize>().ok()) else {
            continue;
        };
        // the quad's top-left and bottom-right corners (points 0 and 2)
        let (pos, _) = quad.positions.as_chunks::<2>();
        let (tex, _) = quad.uvs.as_chunks::<2>();
        let (Some(&[x0, y0]), Some(&[x2, y2]), Some(&[u0, v0]), Some(&[u2, v2])) =
            (pos.first(), pos.get(2), tex.first(), tex.get(2))
        else {
            continue;
        };
        // the quad's corners in canvas pixels, and the uv at any pixel
        let to_px = |x: f32, y: f32| {
            (x * nc.pixels_per_unit + nc.origin_x, y * nc.pixels_per_unit + nc.origin_y)
        };
        let (p0, p2) = (to_px(x0, y0), to_px(x2, y2));
        let Some(am) = next.art_meshes.get_mut(mi) else { continue };
        am.uvs = rest_px(mi)
            .iter()
            .flat_map(|&(px, py)| {
                [
                    u0 + (px - p0.0) / (p2.0 - p0.0) * (u2 - u0),
                    v0 + (py - p0.1) / (p2.1 - p0.1) * (v2 - v0),
                ]
            })
            .collect();
        am.texture = page_offset + quad.texture;
        // Reset mesh goes back to the new art's quad
        if let Some(slot) = originals.get_mut(mi) {
            *slot = (quad.uvs.clone(), quad.indices.clone());
        }
        report.updated += 1;
    }
    let mut pngs = s.texture_pngs.clone();
    pngs.extend(built.textures);
    s.swap_in(ctx, next, pngs)?;
    s.original_meshes = originals;
    Ok(report)
}

impl Session {
    /// A mesh's display name: its layer name, else its ID.
    pub fn mesh_name(&self, i: usize) -> String {
        self.mesh_names
            .get(i)
            .filter(|n| !n.is_empty())
            .cloned()
            .or_else(|| self.puppet.model().art_meshes.get(i).map(|m| m.id.clone()))
            .unwrap_or_default()
    }

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
        crate::gpu_memory_ok();
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
        let mesh_names = imported.mesh_names.clone();
        let mut s =
            Self::from_parts(ctx, name, imported.model, imported.textures, tex_names, None, path)?;
        if part_names.len() == s.part_names.len() {
            s.part_names = part_names;
        }
        if mesh_names.len() == s.mesh_names.len() {
            s.mesh_names = mesh_names;
        }
        s.modified = true;
        Ok(s)
    }

    /// Import a paint document into this model: its groups and layers are
    /// added as new parts and meshes on new texture pages, on top of what
    /// is there. A model with no art yet takes the document's canvas. One
    /// undo step takes the new art back out. Returns the file name.
    pub fn import_paint(&mut self, ctx: &Context, path: &Path) -> Result<String, String> {
        let ext = path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
        let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let imported = if ext == "kra" {
            ayatsuri_import::from_kra(&bytes)
        } else {
            ayatsuri_import::from_psd(&bytes)
        }
        .map_err(|e| format!("{}: {e}", path.display()))?;
        let file = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        self.add_imported(ctx, imported, path)?;
        Ok(file)
    }

    /// Re-import an updated paint document onto the meshes already made from
    /// it: each layer is matched to its mesh by folder path and name (the nth
    /// layer of a repeated name to the nth such mesh), and the mesh takes the
    /// new art on new texture pages, lined up at its resting place. Points,
    /// faces, keyforms and deformers are left exactly as they are. One undo
    /// step puts the old art back.
    pub fn reimport_paint(&mut self, ctx: &Context, path: &Path) -> Result<Reimported, String> {
        let ext = path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
        let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let (doc, _) = ayatsuri_import::read_document(&bytes, ext == "kra")
            .map_err(|e| format!("{}: {e}", path.display()))?;
        let mut report = reimport_into(self, ctx, doc)?;
        report.file =
            path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        Ok(report)
    }

    /// Swap in a model and texture set while keeping everything beside the
    /// model (names, visibility, animations, folders, undo), as one undo step.
    fn swap_in(&mut self, ctx: &Context, model: Model, pngs: Vec<Vec<u8>>) -> Result<(), String> {
        let before = self.puppet.model().clone();
        let tex_names = (0..pngs.len()).map(|i| format!("texture_{i:02}")).collect();
        let source =
            self.aom_path.clone().unwrap_or_else(|| PathBuf::from(format!("{}.aom", self.name)));
        let next = Self::from_parts(ctx, self.name.clone(), model, pngs, tex_names, None, &source)?;
        let old = std::mem::replace(self, next);
        self.part_names = old.part_names;
        self.mesh_names = old.mesh_names;
        self.part_shown = old.part_shown;
        self.mesh_shown = old.mesh_shown;
        self.original_meshes = old.original_meshes;
        self.aom_path = old.aom_path;
        self.author = old.author;
        self.animations = old.animations;
        self.expressions = old.expressions;
        self.current_anim = old.current_anim;
        self.param_names = old.param_names;
        self.param_colors = old.param_colors;
        self.groups = old.groups;
        self.combined = old.combined;
        self.model_groups = old.model_groups;
        self.guides = old.guides;
        self.deformer_shown = old.deformer_shown;
        self.departed = old.departed;
        self.undo = old.undo;
        self.undo.push(before);
        if self.undo.len() > 16 {
            self.undo.remove(0);
        }
        self.redo.clear();
        self.modified = true;
        for i in 0..old.puppet.model().parameters.len() {
            self.puppet.set_value(i, old.puppet.value(i));
        }
        for (i, on) in self.part_shown.iter().enumerate() {
            self.puppet.set_part_opacity(i, if *on { 1.0 } else { 0.0 });
        }
        for (i, on) in self.mesh_shown.iter().enumerate() {
            self.puppet.set_mesh_visible(i, *on);
        }
        self.dirty = true;
        Ok(())
    }

    /// The model-side half of `import_paint`, for an import already built.
    /// Returns how many (meshes, parts) came in.
    pub(crate) fn add_imported(
        &mut self,
        ctx: &Context,
        imported: ayatsuri_import::Imported,
        path: &Path,
    ) -> Result<(usize, usize), String> {
        let before = self.puppet.model().clone();
        let mut model = before.clone();
        let added_meshes = imported.model.art_meshes.len();
        let added_parts = imported.model.parts.len();
        merge_paint(&mut model, imported.model, self.texture_pngs.len() as u32);

        let mut pngs = self.texture_pngs.clone();
        pngs.extend(imported.textures);
        let tex_names = (0..pngs.len()).map(|i| format!("texture_{i:02}")).collect();
        let source = self.aom_path.clone().unwrap_or_else(|| path.to_path_buf());
        let next = Self::from_parts(ctx, self.name.clone(), model, pngs, tex_names, None, &source)?;

        // everything beside the model carries over; the new rows are added
        let old = std::mem::replace(self, next);
        let mut part_names = old.part_names.clone();
        part_names.extend(pad(imported.part_names, added_parts));
        let mut mesh_names = old.mesh_names.clone();
        mesh_names.extend(pad(imported.mesh_names, added_meshes));
        let mut part_shown = old.part_shown.clone();
        part_shown.resize(part_shown.len() + added_parts, true);
        let mut mesh_shown = old.mesh_shown.clone();
        mesh_shown.resize(mesh_shown.len() + added_meshes, true);
        let mut original_meshes = old.original_meshes.clone();
        original_meshes.extend(
            self.puppet
                .model()
                .art_meshes
                .iter()
                .skip(old.original_meshes.len())
                .map(|m| (m.uvs.clone(), m.indices.clone())),
        );
        self.part_names = part_names;
        self.mesh_names = mesh_names;
        self.part_shown = part_shown;
        self.mesh_shown = mesh_shown;
        self.original_meshes = original_meshes;
        self.aom_path = old.aom_path;
        self.author = old.author;
        self.animations = old.animations;
        self.expressions = old.expressions;
        self.current_anim = old.current_anim;
        self.param_names = old.param_names;
        self.param_colors = old.param_colors;
        self.groups = old.groups;
        self.combined = old.combined;
        self.model_groups = old.model_groups;
        self.guides = old.guides;
        self.deformer_shown = old.deformer_shown;
        self.departed = old.departed;
        self.undo = old.undo;
        self.undo.push(before);
        if self.undo.len() > 16 {
            self.undo.remove(0);
        }
        self.redo.clear();
        self.modified = true;
        for (i, v) in (0..old.puppet.model().parameters.len()).map(|i| (i, old.puppet.value(i))) {
            self.puppet.set_value(i, v);
        }
        for (i, on) in self.part_shown.iter().enumerate() {
            self.puppet.set_part_opacity(i, if *on { 1.0 } else { 0.0 });
        }
        for (i, on) in self.mesh_shown.iter().enumerate() {
            self.puppet.set_mesh_visible(i, *on);
        }
        self.dirty = true;
        info!("imported {added_meshes} layer(s) in {added_parts} folder(s)");
        Ok((added_meshes, added_parts))
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
        s.expressions = payload.expressions;
        if let Some(d) = payload.display {
            s.model_groups = d.model_groups.clone();
            s.guides = d.guides.clone();
            let n = s.puppet.model().parameters.len();
            if !d.part_names.is_empty() && d.part_names.len() == s.part_names.len() {
                s.part_names = d.part_names.clone();
            }
            if !d.mesh_names.is_empty() && d.mesh_names.len() == s.mesh_names.len() {
                s.mesh_names = d.mesh_names.clone();
            }
            if let Some(pairs) = &d.combined {
                s.combined = pairs
                    .iter()
                    .map(|&(a, b)| (a as usize, b as usize))
                    .filter(|&(a, b)| a < n && b < n && a != b)
                    .collect();
            }
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
                let colors = d.group_colors;
                s.groups = d
                    .groups
                    .into_iter()
                    .enumerate()
                    .map(|(gi, (name, params))| ParamGroup {
                        name,
                        params: params.into_iter().map(|p| p as usize).filter(|&p| p < n).collect(),
                        open: true,
                        color: colors.get(gi).copied().and_then(unpack_rgb),
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
        let groups = bundle.groups.clone();
        let mut expressions = Vec::new();
        for (ename, epath) in &bundle.expressions {
            match std::fs::read(epath)
                .map_err(|e| e.to_string())
                .and_then(|b| ayatsuri_import::from_exp3(&b, ename).map_err(|e| e.to_string()))
            {
                Ok(e) => expressions.push(e),
                Err(e) => warn!("expression {}: {e}", epath.display()),
            }
        }
        if !expressions.is_empty() {
            info!("{} expression(s)", expressions.len());
        }
        let mut animations = Vec::new();
        for mpath in &bundle.motions {
            match std::fs::read(mpath).map_err(|e| e.to_string()).and_then(|b| {
                ayatsuri_import::from_motion3(&b, &motion_name(mpath)).map_err(|e| e.to_string())
            }) {
                Ok(a) => animations.push(a),
                Err(e) => warn!("motion {}: {e}", mpath.display()),
            }
        }
        if !animations.is_empty() {
            info!("{} motion(s)", animations.len());
        }
        let mut s = Self::from_parts(ctx, name, model, pngs, names, cdi, path)?;
        s.model_groups = groups;
        s.expressions = expressions;
        s.animations = animations;
        Ok(s)
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
        let model_mesh_ids: Vec<String> = model.art_meshes.iter().map(|m| m.id.clone()).collect();
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
                    color: None,
                });
            }
        }
        let rest: Vec<usize> = (0..model.parameters.len())
            .filter(|&i| !grouped.get(i).copied().unwrap_or(false))
            .collect();
        if !rest.is_empty() {
            let name = if groups.is_empty() { String::new() } else { "Other".into() };
            groups.push(ParamGroup { name, params: rest, open: true, color: None });
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
        let original_meshes: Vec<(Vec<f32>, Vec<u16>)> =
            model.art_meshes.iter().map(|m| (m.uvs.clone(), m.indices.clone())).collect();
        let n_params = model.parameters.len();
        let n_tex = texture_names.len();
        let mut puppet = Puppet::new(model);
        puppet.update();
        crate::thumb::ensure(&puppet, &full, source);
        Ok(Self {
            name,
            puppet,
            textures,
            gpu_pending: Some(std::sync::Arc::new(full)),
            gpu_uploaded: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
            gpu_pages: None,
            gpu_pages_uploaded: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
            texture_names,
            texture_pngs: pngs,
            aom_path: None,
            dirty: false,
            last_update_ms: 0.0,
            param_names,
            mesh_names: model_mesh_ids,
            params_held: Vec::new(),
            param_picked: Vec::new(),
            part_names,

            groups,
            param_keys,
            author: String::new(),
            modified: false,
            preview_tex,
            original_meshes,
            part_previews: HashMap::new(),
            deformer_previews: HashMap::new(),
            mesh_previews: HashMap::new(),
            animations: Vec::new(),
            expressions: Vec::new(),
            current_anim: None,
            part_shown: vec![true; n_parts],
            mesh_shown: vec![true; n_meshes],
            departed: HashMap::new(),
            deformer_shown: vec![true; n_defs],
            param_colors: vec![None; n_params],
            full_tex: vec![None; n_tex],
            combined,
            model_groups: None,
            guides: Vec::new(),
            undo: Vec::new(),
            redo: Vec::new(),
        })
    }

    /// Let go of the full-size Mesh Edit page, if one is held.
    pub fn drop_full_textures(&mut self) {
        for slot in &mut self.full_tex {
            if slot.take().is_some() {
                debug!("full-size texture released");
            }
        }
    }

    /// The full-size texture for one slot, decoding it on first use; the
    /// one decoded before it is let go.
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
            // one full-size page at a time: each is a second GPU copy of a
            // page (64 MB at 4096), so keeping every page ever edited would
            // fill a small GPU's memory
            for slot in &mut self.full_tex {
                *slot = None;
            }
            let handle = ctx.load_texture(format!("full-{i}"), color, opts);
            debug!("full texture {i} decoded: {}x{}", size[0], size[1]);
            if let Some(slot) = self.full_tex.get_mut(i) {
                *slot = Some(handle);
            }
        }
        self.full_tex.get(i).and_then(|t| t.as_ref())
    }

    /// Put one texture page in place, or add it at the end, from raw RGBA:
    /// saved as PNG with the model, shown in the Texture palette, and sent
    /// to the GPU on its own.
    pub fn set_page(
        &mut self,
        ctx: &Context,
        index: usize,
        width: u32,
        height: u32,
        rgba: Vec<u8>,
    ) -> Result<(), String> {
        if index > self.texture_pngs.len() {
            return Err(format!("texture page {index} is past the end"));
        }
        let img = image::RgbaImage::from_raw(width, height, rgba)
            .ok_or_else(|| "page pixels don't match its size".to_string())?;
        let mut png = Vec::new();
        img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .map_err(|e| e.to_string())?;
        let name = format!("texture_{index:02}");
        let (thumb, data) = decode_texture(ctx, &name, &png)?;
        fn place<T>(list: &mut Vec<T>, index: usize, item: T) {
            match list.get_mut(index) {
                Some(slot) => *slot = item,
                None => list.push(item),
            }
        }
        place(&mut self.texture_pngs, index, png);
        place(&mut self.textures, index, Some(thumb));
        place(&mut self.texture_names, index, name);
        place(&mut self.preview_tex, index, data.clone());
        place(&mut self.full_tex, index, None);
        let mut pages = self.gpu_pages.take().map(|p| (*p).clone()).unwrap_or_default();
        pages.retain(|(i, _)| *i != index);
        pages.push((index, data));
        self.gpu_pages = Some(std::sync::Arc::new(pages));
        self.gpu_pages_uploaded.store(false, std::sync::atomic::Ordering::Release);
        self.modified = true;
        Ok(())
    }

    /// The page ArtPaths take their colours from: the one existing paths
    /// use, else a new page at the end.
    pub fn art_path_page(&self) -> usize {
        let m = self.puppet.model();
        let pages = self.texture_pngs.len();
        m.art_path_page
            .map(|p| p as usize)
            .filter(|&p| p < pages)
            .or_else(|| {
                m.art_paths.iter().find_map(|p| {
                    m.art_meshes.iter().find(|a| a.id == p.mesh).map(|a| a.texture as usize)
                })
            })
            .unwrap_or(pages)
    }

    /// An ArtPath edit as one undo step (`f` gets the swatch page), then the
    /// swatch page repainted if the colours on it changed.
    pub fn art_path_edit(
        &mut self,
        ctx: &Context,
        f: impl FnOnce(&mut Model, u32) -> bool,
    ) -> bool {
        let page = self.art_path_page();
        let missing = page >= self.texture_pngs.len();
        let colors_before = self.puppet.model().art_path_colors.clone();
        if !self.edit(|m| {
            m.art_path_page = Some(page as u32);
            f(m, page as u32)
        }) {
            return false;
        }
        let colors = self.puppet.model().art_path_colors.clone();
        if missing || colors != colors_before {
            let size = ayatsuri_core::artpath::SWATCH_SIZE;
            let rgba = ayatsuri_core::artpath::swatch_rgba(&colors);
            if let Err(e) = self.set_page(ctx, page, size, size, rgba) {
                warn!("ArtPath colours: {e}");
            }
        }
        true
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

    /// How many steps undo holds; the crash handler compares this against
    /// the start of the frame to find an edit the crash interrupted.
    pub fn undo_depth(&self) -> usize {
        self.undo.len()
    }

    /// After a crash: roll back the edit made this frame (and drop its redo,
    /// since it may be what crashed), or with no edit, rebuild the runtime
    /// from the model so half-applied previews are gone.
    pub fn crash_rollback(&mut self, depth_at_frame_start: usize) {
        if self.undo.len() > depth_at_frame_start {
            self.undo();
            self.redo.clear();
            log::warn!("rolled back the edit that was running when it crashed");
        } else {
            let model = self.puppet.model().clone();
            self.rebuild_from(model);
            log::warn!("rebuilt the model runtime after the crash");
        }
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

    /// Names, visibility and reset shapes sit beside the model, one per
    /// mesh. When an edit (or its undo) adds or removes meshes, carry them
    /// over by mesh id so each stays with its own mesh.
    fn follow_mesh_ids(&mut self, next: &Model) {
        let now = &self.puppet.model().art_meshes;
        if now.len() == next.art_meshes.len()
            && now.iter().zip(&next.art_meshes).all(|(a, b)| a.id == b.id)
        {
            return;
        }
        for (i, m) in now.iter().enumerate() {
            let extras = (
                self.mesh_names.get(i).cloned().unwrap_or_default(),
                self.mesh_shown.get(i).copied().unwrap_or(true),
                self.original_meshes.get(i).cloned().unwrap_or_default(),
            );
            self.departed.insert(m.id.clone(), extras);
        }
        let mut names = Vec::with_capacity(next.art_meshes.len());
        let mut shown = Vec::with_capacity(next.art_meshes.len());
        let mut originals = Vec::with_capacity(next.art_meshes.len());
        for m in &next.art_meshes {
            match self.departed.remove(&m.id) {
                Some((name, on, original)) => {
                    names.push(name);
                    shown.push(on);
                    originals.push(original);
                }
                None => {
                    names.push(String::new());
                    shown.push(true);
                    originals.push((m.uvs.clone(), m.indices.clone()));
                }
            }
        }
        self.mesh_names = names;
        self.mesh_shown = shown;
        self.original_meshes = originals;
    }

    /// Swap in an edited model, carrying over values and visibility.
    fn rebuild_from(&mut self, model: Model) {
        let values: Vec<f32> =
            (0..self.puppet.model().parameters.len()).map(|i| self.puppet.value(i)).collect();
        self.param_keys = keys_of(&model);
        self.follow_mesh_ids(&model);
        let mut puppet = Puppet::new(model);
        for (i, v) in values.iter().enumerate() {
            puppet.set_value(i, *v);
        }
        // keep each part's view opacity (the eye sets 0 or 1, the slider anything)
        for (i, shown) in self.part_shown.iter().enumerate() {
            let now = self.puppet.part_user_opacity(i);
            puppet.set_part_opacity(i, if *shown { now.max(0.01) } else { 0.0 });
        }
        for (i, shown) in self.mesh_shown.iter().enumerate() {
            puppet.set_mesh_visible(i, *shown);
        }
        // the rebuilt model may have gained deformers
        self.deformer_shown.resize(puppet.model().deformers.len(), true);
        self.part_previews.clear();
        self.deformer_previews.clear();
        self.mesh_previews.clear();
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
            expressions: self.expressions.clone(),
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
                group_colors: self.groups.iter().map(|g| pack_rgb(g.color)).collect(),
                part_names: self.part_names.clone(),
                mesh_names: self.mesh_names.clone(),
                combined: Some(self.combined.iter().map(|&(a, b)| (a as u32, b as u32)).collect()),
                model_groups: self.model_groups.clone(),
                guides: self.guides.clone(),
            }),
        }
    }

    /// The periodic safety copy: snapshot the payload here, then encode and
    /// write it on a thread so the frame never waits on deflate. One file
    /// per model, overwritten each time.
    /// Where this model's periodic safety copy goes.
    pub fn autosave_path(&self) -> Option<PathBuf> {
        let safe: String = self
            .name
            .chars()
            .map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '_' })
            .collect();
        Some(crate::recovery::dir()?.join(format!("{safe}-autosave.aom")))
    }

    /// The work is safe (saved, or knowingly discarded): its autosave is
    /// stale and must not be offered for recovery next time.
    pub fn discard_autosave(&self) {
        if let Some(p) = self.autosave_path() {
            let _ = std::fs::remove_file(p);
        }
    }

    pub fn autosave(&self) {
        let Some(path) = self.autosave_path() else { return };
        let payload = self.payload();
        std::thread::spawn(move || {
            if let Some(dir) = path.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
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
        let dir = crate::recovery::dir()?;
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
    /// Groups from the model3.json, when the bundle had one.
    groups: Option<Vec<(String, String, Vec<String>)>>,
    /// Expression files: (name, path), the listed ones first.
    expressions: Vec<(String, PathBuf)>,
    /// Motion files: the listed ones first, then any found beside the model.
    motions: Vec<PathBuf>,
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
            groups: Some(m.groups.into_iter().map(|g| (g.target, g.name, g.ids)).collect()),
            expressions: find_expressions(
                dir,
                m.file_references
                    .expressions
                    .into_iter()
                    .map(|e| (e.name, dir.join(e.file)))
                    .collect(),
            ),
            motions: find_beside(
                dir,
                m.file_references
                    .motions
                    .into_values()
                    .flatten()
                    .map(|r| (String::new(), dir.join(r.file)))
                    .collect(),
                ".motion3.json",
            )
            .into_iter()
            .map(|(_, p)| p)
            .collect(),
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
    Ok(Bundle {
        moc: path.to_path_buf(),
        textures: Vec::new(),
        display_info: None,
        physics: None,
        groups: None,
        expressions: find_expressions(dir, Vec::new()),
        motions: find_beside(dir, Vec::new(), ".motion3.json")
            .into_iter()
            .map(|(_, p)| p)
            .collect(),
    })
}

/// A scene name from a motion file: the file name without `.motion3.json`.
pub(crate) fn motion_name(path: &Path) -> String {
    let file = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    file.strip_suffix(".motion3.json").map(str::to_string).unwrap_or(file)
}

/// Expression files for a bundle; see `find_beside`.
fn find_expressions(dir: &Path, listed: Vec<(String, PathBuf)>) -> Vec<(String, PathBuf)> {
    find_beside(dir, listed, ".exp3.json")
}

/// Files of one kind for a bundle: the ones its model3.json lists, then any
/// other `*<suffix>` in the folder or one level down, which is where
/// VTube Studio finds them for models that list none. At most 256, every
/// path inside the model folder. Found ones are named by their file stem.
fn find_beside(
    dir: &Path,
    mut listed: Vec<(String, PathBuf)>,
    suffix: &str,
) -> Vec<(String, PathBuf)> {
    listed.retain(|(_, p)| p.starts_with(dir) && p.is_file());
    let mut seen: Vec<PathBuf> = listed.iter().map(|(_, p)| p.clone()).collect();
    let mut found = Vec::new();
    let mut scan = |d: &Path, found: &mut Vec<(String, PathBuf)>| {
        let Ok(entries) = std::fs::read_dir(d) else { return };
        let mut paths: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
        paths.sort();
        for p in paths {
            let file = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            if let Some(stem) = file.strip_suffix(suffix)
                && p.is_file()
                && !seen.contains(&p)
            {
                seen.push(p.clone());
                found.push((stem.to_string(), p));
            }
        }
    };
    scan(dir, &mut found);
    if let Ok(entries) = std::fs::read_dir(dir) {
        let mut subdirs: Vec<PathBuf> =
            entries.flatten().map(|e| e.path()).filter(|p| p.is_dir()).collect();
        subdirs.sort();
        for sub in subdirs {
            scan(&sub, &mut found);
        }
    }
    listed.extend(found);
    listed.truncate(256);
    listed
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

// models load on a worker thread so the window keeps answering; that
// needs a Session to be movable between threads
#[cfg(test)]
mod send_check {
    fn is_send<T: Send>() {}
    #[test]
    fn session_can_cross_threads() {
        is_send::<super::Session>();
    }
}

#[cfg(test)]
mod crash_tests {
    use super::*;

    fn session() -> Session {
        Session::create(&egui::Context::default(), "t".into(), String::new(), Model::new()).unwrap()
    }

    fn add_param(s: &mut Session, id: &str) {
        s.edit(|m| {
            m.parameters.push(ayatsuri_format::model::Parameter {
                id: id.into(),
                min: 0.0,
                max: 1.0,
                default: 0.0,
                repeat: false,
                decimals: 3,
            });
            true
        });
    }

    #[test]
    fn crash_rolls_back_the_frames_edit_only() {
        let mut s = session();
        add_param(&mut s, "Kept");
        // the frame that crashes starts here
        let depth = s.undo_depth();
        add_param(&mut s, "Crashed");
        s.crash_rollback(depth);
        let ids: Vec<_> = s.puppet.model().parameters.iter().map(|p| p.id.clone()).collect();
        assert_eq!(ids, vec!["Kept"]);
        // the crashed edit is not offered for redo
        assert!(!s.can_redo());
    }

    #[test]
    fn crash_without_an_edit_keeps_the_model() {
        let mut s = session();
        add_param(&mut s, "Kept");
        let depth = s.undo_depth();
        s.crash_rollback(depth);
        assert_eq!(s.puppet.model().parameters.len(), 1);
        assert!(s.can_undo());
    }
}

#[cfg(test)]
mod import_paint_tests {
    use super::*;
    use ayatsuri_import::{Document, Layer};

    /// A document of solid squares, one per (name, group, x, y, size).
    fn doc(width: u32, height: u32, layers: &[(&str, &str, i32, i32, u32)]) -> Document {
        Document {
            width,
            height,
            layers: layers
                .iter()
                .map(|&(name, group, x, y, size)| Layer {
                    name: name.into(),
                    groups: if group.is_empty() { vec![] } else { vec![group.into()] },
                    x,
                    y,
                    width: size,
                    height: size,
                    rgba: vec![255; (size * size * 4) as usize],
                    visible: true,
                    opacity: 1.0,
                    clip_to_below: false,
                })
                .collect(),
        }
    }

    /// Where a mesh's first vertex sits in the model's canvas pixels.
    fn pixel_of(s: &Session, mesh: usize) -> (f32, f32) {
        let m = s.puppet.model();
        let c = m.canvas;
        let xs = m.art_meshes[mesh].positions.chunks(2).map(|p| p[0]).fold(f32::MAX, f32::min);
        let ys = m.art_meshes[mesh].positions.chunks(2).map(|p| p[1]).fold(f32::MAX, f32::min);
        (xs * c.pixels_per_unit + c.origin_x, ys * c.pixels_per_unit + c.origin_y)
    }

    #[test]
    fn an_empty_model_takes_the_psd_as_is() {
        let ctx = Context::default();
        let mut s = Session::create(&ctx, "t".into(), String::new(), Model::new()).unwrap();
        let first =
            ayatsuri_import::from_layers(&doc(400, 300, &[("Eye", "Face", 40, 50, 20)])).unwrap();
        let canvas = first.model.canvas;
        assert_eq!(s.add_imported(&ctx, first, Path::new("a.psd")).unwrap(), (1, 1));
        assert_eq!(s.puppet.model().canvas.width, canvas.width);
        assert_eq!(s.mesh_name(0), "Eye");
        let (px, py) = pixel_of(&s, 0);
        assert!((px - 40.0).abs() < 0.6 && (py - 50.0).abs() < 0.6, "{px},{py}");
        // one undo step takes it back out
        s.undo();
        assert!(s.puppet.model().art_meshes.is_empty());
    }

    #[test]
    fn a_second_psd_lands_on_top_with_its_own_ids_and_pages() {
        let ctx = Context::default();
        let mut s = Session::create(&ctx, "t".into(), String::new(), Model::new()).unwrap();
        let first =
            ayatsuri_import::from_layers(&doc(400, 300, &[("Body", "Torso", 10, 10, 30)])).unwrap();
        s.add_imported(&ctx, first, Path::new("a.psd")).unwrap();
        s.mesh_shown[0] = false;
        s.animations.push(ayatsuri_format::Animation::default());
        // a smaller document: its pixels line up with the model's top left
        let second = ayatsuri_import::from_layers(&doc(
            200,
            100,
            &[("Hair", "Head", 60, 20, 16), ("Bang", "Head", 5, 5, 8)],
        ))
        .unwrap();
        assert_eq!(s.add_imported(&ctx, second, Path::new("b.psd")).unwrap(), (2, 1));
        let m = s.puppet.model();
        let ids: Vec<&str> = m.art_meshes.iter().map(|a| a.id.as_str()).collect();
        assert_eq!(ids, ["ArtMesh0", "ArtMesh1", "ArtMesh2"]);
        // Torso was part 0, so the new Head folder is part 1
        assert_eq!(m.parts.iter().map(|p| p.id.as_str()).collect::<Vec<_>>(), ["Part0", "Part1"]);
        assert_eq!(s.part_names, vec!["Torso".to_string(), "Head".to_string()]);
        assert!(m.art_meshes[1].texture >= 1, "new art goes on new pages");
        assert_eq!(m.art_meshes[1].parent_part, Some(1));
        assert_eq!(s.mesh_name(0), "Body");
        assert_eq!(s.mesh_name(1), "Hair");
        assert_eq!(s.mesh_shown, vec![false, true, true]);
        assert_eq!(s.animations.len(), 1);
        assert_eq!(s.texture_pngs.len(), s.textures.len());
        let (px, py) = pixel_of(&s, 1);
        assert!((px - 60.0).abs() < 0.6 && (py - 20.0).abs() < 0.6, "{px},{py}");
        s.undo();
        assert_eq!(s.puppet.model().art_meshes.len(), 1);
        assert_eq!(s.mesh_name(0), "Body");
    }

    #[test]
    fn reimport_swaps_art_and_keeps_the_rig() {
        let ctx = Context::default();
        let mut s = Session::create(&ctx, "t".into(), String::new(), Model::new()).unwrap();
        let first = ayatsuri_import::from_layers(&doc(
            400,
            300,
            &[("Body", "Torso", 10, 10, 30), ("Arm", "Torso", 100, 40, 20)],
        ))
        .unwrap();
        s.add_imported(&ctx, first, Path::new("a.psd")).unwrap();
        let before = s.puppet.model().clone();
        let pages = s.texture_pngs.len();
        // the updated file: Body redrawn, Arm renamed to Sleeve
        let mut updated =
            doc(400, 300, &[("Body", "Torso", 10, 10, 30), ("Sleeve", "Torso", 100, 40, 20)]);
        for px in updated.layers[0].rgba.as_chunks_mut::<4>().0 {
            px[0] = 10;
        }
        let r = reimport_into(&mut s, &ctx, updated).unwrap();
        assert_eq!(r.updated, 1);
        assert_eq!(r.new_layers, vec!["Torso/Sleeve".to_string()]);
        assert_eq!(r.missing, vec!["Arm".to_string()]);
        let m = s.puppet.model();
        // Body reads new art from a new page; its points did not move
        assert!(m.art_meshes[0].texture as usize >= pages);
        assert_eq!(m.art_meshes[0].positions, before.art_meshes[0].positions);
        assert_ne!(m.art_meshes[0].uvs, before.art_meshes[0].uvs);
        // Arm kept its old art
        assert_eq!(m.art_meshes[1].texture, before.art_meshes[1].texture);
        assert_eq!(m.art_meshes[1].uvs, before.art_meshes[1].uvs);
        assert_eq!(s.mesh_name(0), "Body");
        assert_eq!(s.texture_pngs.len(), s.textures.len());
        // one undo step puts the old art back
        s.undo();
        assert_eq!(s.puppet.model().art_meshes[0].texture, before.art_meshes[0].texture);
        assert_eq!(s.puppet.model().art_meshes[0].uvs, before.art_meshes[0].uvs);
    }

    #[test]
    fn moc3_draw_groups_get_the_new_meshes() {
        use ayatsuri_format::model::{DrawGroup, DrawItem};
        let mut base =
            ayatsuri_import::from_layers(&doc(100, 100, &[("A", "", 0, 0, 10)])).unwrap().model;
        base.draw_groups = vec![DrawGroup {
            total: 1,
            min_order: 500,
            max_order: 500,
            items: vec![DrawItem { part: false, object: 0, child_group: None }],
        }];
        let add =
            ayatsuri_import::from_layers(&doc(100, 100, &[("B", "", 0, 0, 10)])).unwrap().model;
        merge_paint(&mut base, add, 1);
        assert_eq!(base.draw_groups[0].total, 2);
        assert_eq!(base.draw_groups[0].items.len(), 2);
        let mut p = Puppet::new(base);
        p.update();
        assert_eq!(p.drawables().count(), 2);
    }
}
