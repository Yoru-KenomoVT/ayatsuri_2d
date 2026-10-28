//! A loaded puppet and everything the panels show about it. Opens both the
//! native .aom and Live2D's .moc3 (bare or through a .model3.json bundle),
//! and saves .aom.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use ayatsuri_core::Puppet;
use ayatsuri_format::model::{ArtMesh, Binding, flags};
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
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
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
    /// Compositions: layers of scenes over one clock; they ride in the
    /// .aom too.
    pub compositions: Vec<ayatsuri_format::Composition>,
    /// Physics playlists, played in Physics Settings.
    pub playlists: Vec<ayatsuri_format::Playlist>,
    /// Sound files the compositions use, decoded once, by the path the
    /// layer carries.
    pub audio: HashMap<String, std::sync::Arc<crate::audio::Clip>>,
    pub current_anim: Option<usize>,
    /// The composition the Timeline shows instead of a scene, when it
    /// shows one.
    pub current_comp: Option<usize>,
    pub puppet: Puppet,
    /// Downscaled for the Texture palette and the non-GPU fallback view.
    pub textures: Vec<Option<TextureHandle>>,
    /// Full-resolution pixels waiting to go to the GPU renderer, taken on
    /// the first frame after load.
    /// Textures for the GPU renderer, re-sent every frame until it confirms
    /// the upload, then dropped to free the memory.
    pub gpu_pending: Option<std::sync::Arc<Vec<ayatsuri_render::TexData>>>,
    pub gpu_uploaded: std::sync::Arc<std::sync::atomic::AtomicBool>,
    pub gpu_pages: Option<std::sync::Arc<Vec<(usize, ayatsuri_render::TexData)>>>,
    pub gpu_pages_uploaded: std::sync::Arc<std::sync::atomic::AtomicBool>,
    /// Full pages the renderer should let go of this frame.
    pub gpu_drop: Vec<usize>,
    /// Which pages are on the GPU at full size, and the loading of them.
    pub residency: crate::residency::Residency,
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
    /// Deformer names, apart from their IDs; empty shows the ID.
    pub deformer_names: Vec<String>,
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
    /// Cubism's Solo: the meshes (by id, so moves and undo can't confuse
    /// them) drawn in front at full strength and alone pickable; the rest
    /// sit behind the inactive color. None when Solo is off.
    pub solo: Option<std::collections::HashSet<String>>,
    /// Name, visibility and reset shape of meshes an edit removed, by id,
    /// so undoing the delete gets them back.
    departed: HashMap<String, MeshExtras>,
    pub deformer_shown: Vec<bool>,
    /// Label colors on parameter rows, palette only, saved in .aom.
    pub param_colors: Vec<Option<[u8; 3]>>,
    /// Colours picked for single keys on the sliders: parameter, key value,
    /// colour. Saved in .aom.
    pub key_colors: Vec<(usize, f32, [u8; 3])>,
    /// Full-resolution egui textures, decoded only when something actually
    /// zooms into them (mesh edit); the palette thumbs stay 256px.
    full_tex: Vec<Option<TextureHandle>>,
    /// Parameter pairs shown as one 2D pad (from cdi3 CombinedParameters).
    pub combined: Vec<(usize, usize)>,
    /// The model3.json Groups the model came with (target, name, ids);
    /// None when it had none, and export then writes the standard ones.
    pub model_groups: Option<Vec<(String, String, Vec<String>)>>,
    pub guides: Vec<ayatsuri_format::GuideLine>,
    /// Records each step of the model into `<file>.timelapse`.
    pub timelapse: crate::timelapse::Recorder,
    pub timelapse_on: bool,
    /// The project's own Record timelapse setting; None follows Settings.
    pub timelapse_pref: Option<bool>,
    /// Per texture page: it holds one paint layer, as it came in. Meshes
    /// drawing from such pages wait for the texture atlas, which is made
    /// when the rigger asks (Texture Atlas, or the export's button), as in
    /// Cubism. Pages are never dropped while the model is open (undo may
    /// point back at any of them); saving and exporting leave out the ones
    /// no mesh uses.
    pub layer_pages: Vec<bool>,
    /// Reset shapes by (mesh ID, page), see `stash_art`.
    art_by_page: HashMap<(String, u32), (Vec<f32>, Vec<u16>)>,
    pub bezier_nets: HashMap<(usize, usize), ayatsuri_core::bezier::Lattice>,
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
    let group_offset = base.clip_groups.len() as u32;
    for mut g in add.clip_groups {
        for m in &mut g.masks {
            *m += mesh_offset;
        }
        base.clip_groups.push(g);
    }

    let mut part_ids: std::collections::HashSet<String> =
        base.parts.iter().map(|p| p.id.clone()).collect();
    for mut part in add.parts {
        part.id = free_id("Part", &mut part_ids);
        part.parent_part = part.parent_part.map(|p| p + part_offset);
        if let Some(o) = &mut part.offscreen {
            for m in &mut o.masks {
                *m += mesh_offset;
            }
        }
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
        mesh.clip_group = mesh.clip_group.map(|g| g + group_offset);
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

const UNDO_BUDGET: usize = 256 * 1024 * 1024;
const UNDO_STEPS: usize = 16;

/// Roughly what a model clone costs: its bulk arrays.
fn model_bytes(m: &Model) -> usize {
    use ayatsuri_format::model::DeformerKind;
    let f = std::mem::size_of::<f32>();
    let meshes: usize = m
        .art_meshes
        .iter()
        .map(|a| {
            (a.positions.len() + a.uvs.len() + a.opacities.len() + a.draw_orders.len()) * f
                + a.indices.len() * 2
        })
        .sum();
    let deformers: usize = m
        .deformers
        .iter()
        .map(|d| match &d.kind {
            DeformerKind::Warp(w) => (w.positions.len() + w.opacities.len()) * f,
            DeformerKind::Rotation(r) => (r.angles.len() * 6) * f,
        })
        .sum();
    let glues: usize = m.glues.iter().map(|g| g.weights.len() * f + g.indices.len() * 2).sum();
    meshes + deformers + glues + 4096
}

/// Drop the oldest steps past the count or the byte budget (keeping two).
fn trim_history(steps: &mut Vec<Model>) {
    trim_history_to(steps, UNDO_STEPS, UNDO_BUDGET);
}

fn trim_history_to(steps: &mut Vec<Model>, max_steps: usize, budget: usize) {
    while steps.len() > max_steps {
        steps.remove(0);
    }
    let mut total: usize = steps.iter().map(model_bytes).sum();
    while steps.len() > 2 && total > budget {
        total = total.saturating_sub(model_bytes(&steps.remove(0)));
    }
}

/// Drop texture pages no mesh draws from (and that aren't the ArtPath
/// colour page while paths exist), renumbering the meshes' pages. Returns
/// the pages kept and how many went.
pub(crate) fn prune_pages(model: &mut Model, pngs: Vec<Vec<u8>>) -> (Vec<Vec<u8>>, usize) {
    let flags = vec![false; pngs.len()];
    let (kept, _, dropped) = prune_pages_with(model, pngs, flags);
    (kept, dropped)
}

/// `prune_pages`, carrying a flag per page along with the pages kept.
pub(crate) fn prune_pages_with<T>(
    model: &mut Model,
    pngs: Vec<Vec<u8>>,
    flags: Vec<T>,
) -> (Vec<Vec<u8>>, Vec<T>, usize) {
    let mut used = vec![false; pngs.len()];
    for m in &model.art_meshes {
        if let Some(u) = used.get_mut(m.texture as usize) {
            *u = true;
        }
    }
    if !model.art_paths.is_empty()
        && let Some(u) = model.art_path_page.and_then(|p| used.get_mut(p as usize))
    {
        *u = true;
    }
    let dropped = used.iter().filter(|u| !**u).count();
    if dropped == 0 || used.iter().all(|u| !*u) {
        return (pngs, flags, 0);
    }
    let mut new_index = vec![0u32; used.len()];
    let mut next = 0u32;
    for (i, u) in used.iter().enumerate() {
        if *u && let Some(slot) = new_index.get_mut(i) {
            *slot = next;
            next += 1;
        }
    }
    for m in &mut model.art_meshes {
        m.texture = new_index.get(m.texture as usize).copied().unwrap_or(0);
    }
    model.art_path_page = model
        .art_path_page
        .filter(|p| used.get(*p as usize).copied().unwrap_or(false))
        .and_then(|p| new_index.get(p as usize).copied());
    let flags = flags.into_iter().zip(&used).filter(|(_, u)| **u).map(|(f, _)| f).collect();
    let kept = pngs.into_iter().zip(used).filter(|(_, u)| *u).map(|(p, _)| p).collect();
    (kept, flags, dropped)
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
            blend: layer.blend,
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
        folders: doc.folders.clone(),
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
        // the layer's blend mode as the file has it now
        let bits = flags::BLEND_ADDITIVE | flags::BLEND_MULTIPLICATIVE;
        am.flags = (am.flags & !bits) | (quad.flags & bits);
        am.blend = quad.blend;
        // Reset mesh goes back to the new art's quad
        if let Some(slot) = originals.get_mut(mi) {
            *slot = (quad.uvs.clone(), quad.indices.clone());
        }
        report.updated += 1;
    }
    // the old art's pages stay while the model is open, for undo
    let mut pngs = s.texture_pngs.clone();
    pngs.extend(built.textures);
    let (old, all) = (s.texture_pngs.len(), pngs.len());
    s.stash_art();
    s.swap_in(ctx, next, pngs)?;
    s.original_meshes = originals;
    s.stash_art();
    // the new layers each came in on a page of their own
    s.layer_pages.truncate(old);
    s.layer_pages.resize(all, true);
    Ok(report)
}

impl Session {
    /// Each mesh's own art on its page, (u0, v0, u1, v1): the box its
    /// points had when its art came in (import, or the last atlas layout).
    /// A mesh grown past it shows nothing there, not a neighbour's art.
    /// ArtPath meshes, drawn from the swatch page, are not clipped.
    pub fn art_boxes(&self) -> Vec<[f32; 4]> {
        let model = self.puppet.model();
        model
            .art_meshes
            .iter()
            .enumerate()
            .map(|(i, m)| {
                if model.art_path_page.is_some_and(|p| p == m.texture) {
                    return ayatsuri_render::NO_CLIP;
                }
                let uvs = self.original_meshes.get(i).map_or(&m.uvs, |o| &o.0);
                let mut b = [f32::MAX, f32::MAX, f32::MIN, f32::MIN];
                for uv in uvs.as_chunks::<2>().0 {
                    b = [b[0].min(uv[0]), b[1].min(uv[1]), b[2].max(uv[0]), b[3].max(uv[1])];
                }
                // a hair over, so the art's own edge pixels stay whole
                let e = 1e-4;
                if b[0] <= b[2] && b[1] <= b[3] {
                    [b[0] - e, b[1] - e, b[2] + e, b[3] + e]
                } else {
                    ayatsuri_render::NO_CLIP
                }
            })
            .collect()
    }

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
            "psd" | "kra" | "clip" => Self::load_paint(ctx, path, &ext)?,
            "cmo3" => Self::load_cmo3(ctx, path)?,
            _ => Self::load_moc3(ctx, path)?,
        };
        info!("opened {} in {:.0} ms", path.display(), started.elapsed().as_secs_f32() * 1000.0);
        Ok(session)
    }

    /// A paint document becomes a brand new model: groups as parts, layers
    /// as quad meshes, each layer's pixels on a texture of its own until
    /// the texture atlas is made. It starts unsaved, since
    /// the .psd, .kra or .clip is the source and not a model file.
    fn load_paint(ctx: &Context, path: &Path, ext: &str) -> Result<Self, String> {
        let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let imported = ayatsuri_import::from_paint(&bytes, ext)
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
        s.layer_pages = vec![true; s.texture_pngs.len()];
        if part_names.len() == s.part_names.len() {
            s.part_names = part_names;
        }
        if mesh_names.len() == s.mesh_names.len() {
            s.mesh_names = mesh_names;
        }
        s.modified = true;
        Ok(s)
    }

    /// A Cubism Editor project becomes a model with everything the project
    /// had: art, meshes, parts, deformers, parameters in their folders,
    /// keyforms, physics and clipping. It starts unsaved, as a paint
    /// document does.
    fn load_cmo3(ctx: &Context, path: &Path) -> Result<Self, String> {
        let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let name = path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "model".into());
        let out = ayatsuri_import::from_cmo3(&bytes, &name)
            .map_err(|e| format!("{}: {e}", path.display()))?;
        let (within, checked) = out.within;
        info!(
            "cmo3: {} meshes, {} deformers, {} parameters in {} folders, {} physics groups; {within}/{checked} meshes on their edit points at rest",
            out.imported.model.art_meshes.len(),
            out.imported.model.deformers.len(),
            out.imported.model.parameters.len(),
            out.param_groups.len(),
            out.imported.model.physics.as_ref().map_or(0, |p| p.groups.len())
        );
        // the project's parameter names and folders ride in as a cdi would
        let mut cdi = Cdi3::default();
        for (gi, (gname, members)) in out.param_groups.iter().enumerate() {
            let gid = format!("ParamGroup{gi}");
            cdi.parameter_groups.push(Cdi3Entry {
                id: gid.clone(),
                group_id: String::new(),
                name: gname.clone(),
            });
            for &pi in members {
                if let Some(p) = out.imported.model.parameters.get(pi as usize) {
                    cdi.parameters.push(Cdi3Entry {
                        id: p.id.clone(),
                        group_id: gid.clone(),
                        name: out.param_names.get(pi as usize).cloned().unwrap_or_default(),
                    });
                }
            }
        }
        for (pi, p) in out.imported.model.parameters.iter().enumerate() {
            if !cdi.parameters.iter().any(|e| e.id == p.id) {
                cdi.parameters.push(Cdi3Entry {
                    id: p.id.clone(),
                    group_id: String::new(),
                    name: out.param_names.get(pi).cloned().unwrap_or_default(),
                });
            }
        }
        let deformer_names = out.deformer_names;
        let imported = out.imported;
        let tex_names = (0..imported.textures.len()).map(|i| format!("texture_{i:02}")).collect();
        let part_names = imported.part_names.clone();
        let mesh_names = imported.mesh_names.clone();
        let mut s = Self::from_parts(
            ctx,
            name,
            imported.model,
            imported.textures,
            tex_names,
            Some(cdi),
            path,
        )?;
        if part_names.len() == s.part_names.len() {
            s.part_names = part_names;
        }
        if mesh_names.len() == s.mesh_names.len() {
            s.mesh_names = mesh_names;
        }
        if deformer_names.len() == s.deformer_names.len() {
            s.deformer_names = deformer_names;
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
        let imported = ayatsuri_import::from_paint(&bytes, &ext)
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
        let (doc, _) = ayatsuri_import::read_document(&bytes, &ext)
            .map_err(|e| format!("{}: {e}", path.display()))?;
        let mut report = reimport_into(self, ctx, doc)?;
        report.file =
            path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        Ok(report)
    }

    /// Swap in a model and texture set while keeping everything beside the
    /// model (names, visibility, animations, folders, undo), as one undo step.
    /// `swap_in` for a paste that brings texture pages: the lists kept per
    /// mesh and per parameter grow with what came in.
    pub(crate) fn swap_in_pub(
        &mut self,
        ctx: &Context,
        model: Model,
        pngs: Vec<Vec<u8>>,
    ) -> Result<(), String> {
        self.swap_in(ctx, model, pngs)?;
        let m = self.puppet.model();
        let (meshes, params) = (m.art_meshes.len(), m.parameters.len());
        let originals: Vec<(Vec<f32>, Vec<u16>)> = m
            .art_meshes
            .iter()
            .skip(self.original_meshes.len())
            .map(|a| (a.uvs.clone(), a.indices.clone()))
            .collect();
        self.mesh_names.resize(meshes, String::new());
        self.mesh_shown.resize(meshes, true);
        self.original_meshes.extend(originals);
        self.param_colors.resize(params, None);
        self.param_names.resize(params, String::new());
        Ok(())
    }

    fn swap_in(&mut self, ctx: &Context, model: Model, pngs: Vec<Vec<u8>>) -> Result<(), String> {
        let before = self.puppet.model().clone();
        let tex_names = (0..pngs.len()).map(|i| format!("texture_{i:02}")).collect();
        let source =
            self.aom_path.clone().unwrap_or_else(|| PathBuf::from(format!("{}.aom", self.name)));
        let next = Self::from_parts(ctx, self.name.clone(), model, pngs, tex_names, None, &source)?;
        let old = std::mem::replace(self, next);
        self.deformer_names =
            follow_deformer_names(&old.deformer_names, &before, self.puppet.model());
        self.part_names = old.part_names;
        self.mesh_names = old.mesh_names;
        self.part_shown = old.part_shown;
        self.mesh_shown = old.mesh_shown;
        self.original_meshes = old.original_meshes;
        self.aom_path = old.aom_path;
        self.author = old.author;
        self.animations = old.animations;
        self.expressions = old.expressions;
        self.compositions = old.compositions;
        self.playlists = old.playlists;
        self.audio = old.audio;
        self.current_anim = old.current_anim;
        self.current_comp = old.current_comp;
        self.param_names = old.param_names;
        self.param_colors = old.param_colors;
        self.key_colors = old.key_colors;
        self.groups = old.groups;
        self.combined = old.combined;
        self.model_groups = old.model_groups;
        self.guides = old.guides;
        self.timelapse = old.timelapse;
        self.timelapse_on = old.timelapse_on;
        self.timelapse_pref = old.timelapse_pref;
        // pages that came in with the new set hold atlas pages unless the
        // caller says otherwise
        self.layer_pages = old.layer_pages;
        self.layer_pages.resize(self.texture_pngs.len(), false);
        self.art_by_page = old.art_by_page;
        self.bezier_nets = old.bezier_nets;
        self.deformer_shown = old.deformer_shown;
        self.departed = old.departed;
        self.undo = old.undo;
        self.push_undo(before);
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
        self.record_step();
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
        self.deformer_names =
            follow_deformer_names(&old.deformer_names, old.puppet.model(), self.puppet.model());
        self.part_names = part_names;
        self.mesh_names = mesh_names;
        self.part_shown = part_shown;
        self.mesh_shown = mesh_shown;
        self.original_meshes = original_meshes;
        self.aom_path = old.aom_path;
        self.author = old.author;
        self.animations = old.animations;
        self.expressions = old.expressions;
        self.compositions = old.compositions;
        self.playlists = old.playlists;
        self.audio = old.audio;
        self.current_anim = old.current_anim;
        self.current_comp = old.current_comp;
        self.param_names = old.param_names;
        self.param_colors = old.param_colors;
        self.key_colors = old.key_colors;
        self.groups = old.groups;
        self.combined = old.combined;
        self.model_groups = old.model_groups;
        self.guides = old.guides;
        self.timelapse = old.timelapse;
        self.timelapse_on = old.timelapse_on;
        self.timelapse_pref = old.timelapse_pref;
        // the document's layers each came in on a page of their own
        self.layer_pages = old.layer_pages;
        self.layer_pages.resize(old.texture_pngs.len(), false);
        self.layer_pages.resize(self.texture_pngs.len(), true);
        self.art_by_page = old.art_by_page;
        self.bezier_nets = old.bezier_nets;
        self.deformer_shown = old.deformer_shown;
        self.departed = old.departed;
        self.undo = old.undo;
        self.push_undo(before);
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
        self.record_step();
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
        let mut model = payload.model;
        let mut layer_pages = vec![false; pngs.len()];
        for &i in &payload.meta.layer_pages {
            if let Some(l) = layer_pages.get_mut(i as usize) {
                *l = true;
            }
        }
        let (pngs, layer_pages, dropped) = prune_pages_with(&mut model, pngs, layer_pages);
        let tex_names = (0..pngs.len()).map(|i| format!("texture_{i:02}")).collect();
        let mut s = Self::from_parts(ctx, name, model, pngs, tex_names, None, path)?;
        s.layer_pages = layer_pages;
        if dropped > 0 {
            info!("left out {dropped} texture page(s) no mesh uses; saving makes it stick");
            s.modified = true;
        }
        s.aom_path = Some(path.to_path_buf());
        s.timelapse = crate::timelapse::Recorder::open(Some(path));
        s.timelapse.record(s.puppet.model());
        s.author = payload.meta.author;
        s.timelapse_pref = payload.meta.timelapse;
        s.animations = payload.animations;
        s.expressions = payload.expressions;
        s.compositions = payload.compositions;
        s.playlists = payload.playlists;
        s.load_all_audio();
        if let Some(d) = payload.display {
            s.model_groups = d.model_groups.clone();
            s.guides = d.guides.clone();
            let index_of: HashMap<&str, usize> = s
                .puppet
                .model()
                .deformers
                .iter()
                .enumerate()
                .map(|(i, d)| (d.id.as_str(), i))
                .collect();
            for (id, k, bw, bh, flat) in &d.bezier_nets {
                let Some(&di) = index_of.get(id.as_str()) else { continue };
                let net: Vec<(f32, f32)> =
                    flat.as_chunks::<2>().0.iter().map(|&[x, y]| (x, y)).collect();
                let (bw, bh) = (*bw as usize, *bh as usize);
                if net.len() == (3 * bw + 1) * (3 * bh + 1) {
                    s.bezier_nets
                        .insert((di, *k as usize), ayatsuri_core::bezier::Lattice { bw, bh, net });
                }
            }
            let n = s.puppet.model().parameters.len();
            if !d.part_names.is_empty() && d.part_names.len() == s.part_names.len() {
                s.part_names = d.part_names.clone();
            }
            if !d.mesh_names.is_empty() && d.mesh_names.len() == s.mesh_names.len() {
                s.mesh_names = d.mesh_names.clone();
            }
            if d.deformer_names.len() == s.deformer_names.len() {
                s.deformer_names = d.deformer_names.clone();
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
            s.key_colors = d
                .key_colors
                .iter()
                .filter(|&&(p, _, c)| (p as usize) < n && c >= 0)
                .map(|&(p, k, c)| {
                    (
                        p as usize,
                        k,
                        [((c >> 16) & 0xff) as u8, ((c >> 8) & 0xff) as u8, (c & 0xff) as u8],
                    )
                })
                .collect();
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
            .and_then(|p| match std::fs::read(p) {
                Ok(b) => Some((p, b)),
                Err(e) => {
                    warn!("display names: could not read {}: {e}", p.display());
                    None
                }
            })
            .and_then(|(p, b)| match serde_json::from_slice(&b) {
                Ok(cdi) => Some(cdi),
                Err(e) => {
                    warn!("display names: {} is not valid ({e}); using the ids", p.display());
                    None
                }
            });

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
        // pages are decoded one at a time and only their proxies are kept:
        // the full pixels stream to the GPU later, page by page, as the
        // view needs them (see residency.rs)
        let mut textures = Vec::with_capacity(pngs.len());
        let mut preview_tex = Vec::with_capacity(pngs.len());
        let mut sizes = Vec::with_capacity(pngs.len());
        for (i, png) in pngs.iter().enumerate() {
            match decode_texture(
                ctx,
                texture_names.get(i).map(String::as_str).unwrap_or("texture"),
                png,
            ) {
                Ok((thumb, data)) => {
                    debug!("texture {i}: {}x{} decoded", data.width, data.height);
                    textures.push(Some(thumb));
                    sizes.push((data.width, data.height));
                    preview_tex.push(shrink_for_preview(data));
                }
                Err(e) => {
                    warn!("texture {i}: {e}");
                    textures.push(None);
                    sizes.push((1, 1));
                    preview_tex.push(ayatsuri_render::TexData {
                        width: 1,
                        height: 1,
                        rgba: vec![255; 4],
                    });
                }
            }
        }
        if textures.is_empty() {
            warn!("no textures found; meshes draw untextured");
        }

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
        let n_deformers = model.deformers.len();
        let mut puppet = Puppet::new(model);
        puppet.update();
        crate::thumb::ensure(&puppet, &preview_tex, source);
        Ok(Self {
            name,
            puppet,
            textures,
            gpu_pending: Some(std::sync::Arc::new(preview_tex.clone())),
            residency: crate::residency::Residency::new(&sizes, 512),
            gpu_uploaded: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
            gpu_pages: None,
            gpu_pages_uploaded: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
            gpu_drop: Vec::new(),
            texture_names,
            texture_pngs: pngs,
            aom_path: None,
            dirty: false,
            last_update_ms: 0.0,
            param_names,
            mesh_names: model_mesh_ids,
            deformer_names: vec![String::new(); n_deformers],
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
            compositions: Vec::new(),
            playlists: Vec::new(),
            audio: HashMap::new(),
            current_comp: None,
            current_anim: None,
            part_shown: vec![true; n_parts],
            mesh_shown: vec![true; n_meshes],
            solo: None,
            departed: HashMap::new(),
            deformer_shown: vec![true; n_defs],
            param_colors: vec![None; n_params],
            key_colors: Vec::new(),
            full_tex: vec![None; n_tex],
            combined,
            model_groups: None,
            guides: Vec::new(),
            timelapse: crate::timelapse::Recorder::default(),
            timelapse_on: true,
            timelapse_pref: None,
            layer_pages: Vec::new(),
            art_by_page: HashMap::new(),
            bezier_nets: HashMap::new(),
            undo: Vec::new(),
            redo: Vec::new(),
        })
    }

    /// Let go of the full-size Mesh Edit page, if one is held.
    /// A page's full width in pixels, as recorded at load.
    pub fn residency_page_width(&self, index: usize) -> u32 {
        self.residency.page_size(index).map_or(1, |(w, _)| w)
    }

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
        place(&mut self.full_tex, index, None);
        let (w, h) = (data.width, data.height);
        place(&mut self.preview_tex, index, shrink_for_preview(data.clone()));
        self.residency.set_page(index, w, h);
        let mut pages = self.gpu_pages.take().map(|p| (*p).clone()).unwrap_or_default();
        pages.retain(|(i, _)| *i != index);
        pages.push((index, data));
        self.gpu_pages = Some(std::sync::Arc::new(pages));
        self.gpu_pages_uploaded.store(false, std::sync::atomic::Ordering::Release);
        self.modified = true;
        Ok(())
    }

    /// Every page's size in pixels, as recorded at load.
    pub fn page_sizes(&self) -> Vec<(u32, u32)> {
        (0..self.texture_pngs.len())
            .map(|i| self.residency.page_size(i).unwrap_or((1, 1)))
            .collect()
    }

    /// Some mesh still draws from a page holding one paint layer: the
    /// texture atlas has not been made since that art came in.
    pub fn atlas_pending(&self) -> bool {
        let m = self.puppet.model();
        m.art_meshes
            .iter()
            .any(|am| self.layer_pages.get(am.texture as usize).copied().unwrap_or(false))
    }

    /// Lay the texture atlas out again: the pieces go where the layout says,
    /// on fresh pages of its size after the old ones, and every mesh's uvs
    /// follow. One undo step.
    pub fn apply_atlas(
        &mut self,
        ctx: &Context,
        pieces: &[crate::atlas::Piece],
        layout: &crate::atlas::Layout,
    ) -> Result<(), String> {
        if layout.pages == 0 {
            return Err("the layout has no pages".into());
        }
        let sizes = self.page_sizes();
        let pngs = self.texture_pngs.clone();
        let decode = |i: usize| -> Option<image::RgbaImage> {
            pngs.get(i)
                .and_then(|png| image::load_from_memory(png).ok())
                .map(|img| img.into_rgba8())
        };
        let pages = crate::atlas::build_pages(pieces, layout, decode);
        // the new pages go after the old ones, which stay for undo (and the
        // ArtPath swatch page keeps its place)
        let first_new = self.texture_pngs.len() as u32;
        let mut out = self.texture_pngs.clone();
        for img in pages {
            let mut png = Vec::new();
            img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
                .map_err(|e| e.to_string())?;
            out.push(png);
        }
        let mut next = self.puppet.model().clone();
        crate::atlas::apply_to_model(&mut next, pieces, layout, &sizes, first_new);
        self.stash_art();
        self.swap_in(ctx, next, out)?;
        // the atlas is a fresh set of quads underneath every mesh
        self.original_meshes = self
            .puppet
            .model()
            .art_meshes
            .iter()
            .map(|m| (m.uvs.clone(), m.indices.clone()))
            .collect();
        self.stash_art();
        info!("atlas: {} page(s) of {} px", layout.pages, layout.page_size);
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

    /// Keep `before` as an undo step. Undo holds at most 16 steps and,
    /// since a step is a whole model (tens of MB on a big rig), at most
    /// UNDO_BUDGET bytes, always keeping the last two.
    fn push_undo(&mut self, before: Model) {
        self.undo.push(before);
        trim_history(&mut self.undo);
    }

    /// One step of the timelapse, when recording is on.
    fn record_step(&mut self) {
        if self.timelapse_on {
            self.timelapse.record(self.puppet.model());
        }
    }

    pub fn edit(&mut self, f: impl FnOnce(&mut Model) -> bool) -> bool {
        let before = self.puppet.model().clone();
        let mut after = before.clone();
        if !f(&mut after) {
            return false;
        }
        log_edit(&before, &after);
        self.push_undo(before);
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

    /// Note each mesh's reset shape (and so its art box) under the page it
    /// draws from, so an undo that moves meshes back to older pages (atlas,
    /// re-import) finds the art boxes those pages had.
    fn stash_art(&mut self) {
        let m = self.puppet.model();
        for (am, orig) in m.art_meshes.iter().zip(&self.original_meshes) {
            self.art_by_page.insert((am.id.clone(), am.texture), orig.clone());
        }
    }

    /// After undo or redo: each mesh now on another page than `was` takes
    /// the reset shape noted for that page.
    fn unstash_art(&mut self, was: &HashMap<String, u32>) {
        let m = self.puppet.model();
        for (am, orig) in m.art_meshes.iter().zip(self.original_meshes.iter_mut()) {
            if was.get(&am.id) == Some(&am.texture) {
                continue;
            }
            if let Some(o) = self.art_by_page.get(&(am.id.clone(), am.texture)) {
                *orig = o.clone();
            }
        }
    }

    fn pages_now(&self) -> HashMap<String, u32> {
        self.puppet.model().art_meshes.iter().map(|m| (m.id.clone(), m.texture)).collect()
    }

    pub fn undo(&mut self) {
        if let Some(previous) = self.undo.pop() {
            self.redo.push(self.puppet.model().clone());
            trim_history(&mut self.redo);
            let was = self.pages_now();
            self.rebuild_from(previous);
            self.unstash_art(&was);
            self.modified = true;
            log::info!("undo ({} left)", self.undo.len());
        }
    }

    pub fn redo(&mut self) {
        if let Some(next) = self.redo.pop() {
            self.undo.push(self.puppet.model().clone());
            trim_history(&mut self.undo);
            let was = self.pages_now();
            self.rebuild_from(next);
            self.unstash_art(&was);
            self.modified = true;
            log::info!("redo");
        }
    }

    /// Names, visibility and reset shapes sit beside the model, one per
    /// mesh. When an edit (or its undo) adds or removes meshes, carry them
    /// over by mesh id so each stays with its own mesh.
    fn follow_mesh_ids(&mut self, next: &Model) {
        let now = &self.puppet.model().art_meshes;
        // the same meshes in the same places, at most some given new IDs
        // (Rename, and its undo): each stays where it is
        let renamed = |a: &ArtMesh, b: &ArtMesh| {
            a.id == b.id
                || (!now.iter().any(|m| m.id == b.id)
                    && !next.art_meshes.iter().any(|m| m.id == a.id))
        };
        if now.len() == next.art_meshes.len()
            && now.iter().zip(&next.art_meshes).all(|(a, b)| renamed(a, b))
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
        self.deformer_names =
            follow_deformer_names(&self.deformer_names, self.puppet.model(), &model);
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
        self.record_step();
    }

    /// Save this session as `.aom`, atomically, and remember the path.
    /// A sound file's path as a layer carries it: relative to the .aom's
    /// folder when it sits beside or under it, else as given.
    pub fn audio_rel(&self, path: &Path) -> String {
        let base = self.aom_path.as_ref().and_then(|p| p.parent());
        match base.and_then(|b| path.strip_prefix(b).ok()) {
            Some(rel) => rel.to_string_lossy().replace('\\', "/"),
            None => path.to_string_lossy().into_owned(),
        }
    }

    /// Where a layer's sound file is on disk.
    pub fn audio_abs(&self, rel: &str) -> PathBuf {
        let p = Path::new(rel);
        if p.is_absolute() {
            return p.to_path_buf();
        }
        match self.aom_path.as_ref().and_then(|a| a.parent()) {
            Some(base) => base.join(p),
            None => p.to_path_buf(),
        }
    }

    /// Decode a layer's sound file into the cache, once.
    pub fn load_audio(&mut self, rel: &str) -> Result<(), String> {
        if self.audio.contains_key(rel) {
            return Ok(());
        }
        let clip = crate::audio::decode(&self.audio_abs(rel))?;
        info!("audio {rel}: {:.2} s, {} Hz, {} channel(s)", clip.len(), clip.rate, clip.channels);
        self.audio.insert(rel.to_string(), std::sync::Arc::new(clip));
        Ok(())
    }

    /// Every sound the compositions refer to, decoded; a missing file is
    /// logged and its layer stays silent.
    pub fn load_all_audio(&mut self) {
        let rels: Vec<String> = self
            .compositions
            .iter()
            .flat_map(|c| c.layers.iter())
            .filter_map(|l| match &l.source {
                ayatsuri_format::LayerSource::Audio(p) => Some(p.clone()),
                _ => None,
            })
            .collect();
        for rel in rels {
            if let Err(e) = self.load_audio(&rel) {
                warn!("audio {rel}: {e}");
            }
        }
    }

    /// What compositions can reach: every scene and composition here.
    pub fn comp_scope(&self) -> ayatsuri_core::comp::Scope<'_> {
        ayatsuri_core::comp::Scope { scenes: &self.animations, comps: &self.compositions }
    }

    /// A composition flattened into a scene over the model's rest pose,
    /// for everything that plays or writes scenes.
    pub fn bake_comp(&self, comp: &ayatsuri_format::Composition) -> ayatsuri_format::Animation {
        let model = self.puppet.model();
        ayatsuri_core::comp::bake(&self.comp_scope(), comp, |target, id| match target {
            ayatsuri_format::CurveTarget::Parameter => {
                model.parameters.iter().find(|p| p.id == id).map_or(0.0, |p| p.default)
            }
            _ => 1.0,
        })
    }

    /// Every composition baked, in order.
    pub fn baked_comps(&self) -> Vec<ayatsuri_format::Animation> {
        self.compositions.iter().map(|c| self.bake_comp(c)).collect()
    }

    /// The model, textures, animations and display info as one payload.
    fn payload(&self) -> AomPayload {
        // pages no mesh draws from any more stay behind
        let mut model = self.puppet.model().clone();
        let (pngs, layer_pages, _) =
            prune_pages_with(&mut model, self.texture_pngs.clone(), self.layer_pages.clone());
        AomPayload {
            meta: Meta {
                name: self.name.clone(),
                author: self.author.clone(),
                tool: format!("Ayatsuri2D {}", env!("CARGO_PKG_VERSION")),
                timelapse: self.timelapse_pref,
                layer_pages: layer_pages
                    .iter()
                    .enumerate()
                    .filter(|(_, l)| **l)
                    .map(|(i, _)| i as u32)
                    .collect(),
            },
            model,
            textures: pngs.into_iter().map(serde_bytes::ByteBuf::from).collect(),
            animations: self.animations.clone(),
            compositions: self.compositions.clone(),
            playlists: self.playlists.clone(),
            expressions: self.expressions.clone(),
            display: Some(ayatsuri_format::DisplayInfo {
                param_names: self.param_names.clone(),
                key_colors: self
                    .key_colors
                    .iter()
                    .map(|&(p, k, [r, g, b])| {
                        (p as u32, k, ((r as i32) << 16) | ((g as i32) << 8) | (b as i32))
                    })
                    .collect(),
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
                deformer_names: self.deformer_names.clone(),
                combined: Some(self.combined.iter().map(|&(a, b)| (a as u32, b as u32)).collect()),
                model_groups: self.model_groups.clone(),
                guides: self.guides.clone(),
                bezier_nets: {
                    let defs = &self.puppet.model().deformers;
                    let mut nets: Vec<(String, u32, u32, u32, Vec<f32>)> = self
                        .bezier_nets
                        .iter()
                        .filter_map(|(&(d, k), l)| {
                            let id = defs.get(d)?.id.clone();
                            let flat = l.net.iter().flat_map(|&(x, y)| [x, y]).collect();
                            Some((id, k as u32, l.bw as u32, l.bh as u32, flat))
                        })
                        .collect();
                    nets.sort_by(|a, b| (&a.0, a.1).cmp(&(&b.0, b.1)));
                    nets
                },
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
        if let Some(p) = self.autosave_path()
            && let Err(e) = std::fs::remove_file(&p)
            && e.kind() != std::io::ErrorKind::NotFound
        {
            log::warn!("could not remove the stale autosave {}: {e}", p.display());
        }
    }

    pub fn autosave(&self) {
        let Some(path) = self.autosave_path() else { return };
        let payload = self.payload();
        std::thread::spawn(move || {
            if let Some(dir) = path.parent()
                && let Err(e) = std::fs::create_dir_all(dir)
            {
                warn!("autosave: could not make {}: {e}", dir.display());
            }
            let tmp = path.with_extension("aom.tmp");
            match ayatsuri_format::write(&payload) {
                Ok(bytes) => {
                    match std::fs::write(&tmp, bytes).and_then(|_| std::fs::rename(&tmp, &path)) {
                        Ok(()) => debug!("autosaved to {}", path.display()),
                        Err(e) => warn!("autosave to {} failed: {e}", path.display()),
                    }
                }
                Err(e) => warn!("autosave failed: {e}"),
            }
        });
    }

    /// Write a recovery copy without touching the session's own path, so a
    /// crash never costs the user their work. Returns where it went.
    pub fn write_recovery(&self) -> Option<PathBuf> {
        let Some(dir) = crate::recovery::dir() else {
            warn!("recovery copy: no folder to keep it in");
            return None;
        };
        if let Err(e) = std::fs::create_dir_all(&dir) {
            warn!("recovery copy: could not make {}: {e}", dir.display());
            return None;
        }
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
        let bytes = match ayatsuri_format::write(&self.payload()) {
            Ok(b) => b,
            Err(e) => {
                warn!("recovery copy: could not encode the model: {e}");
                return None;
            }
        };
        if let Err(e) = std::fs::write(&path, bytes) {
            warn!("recovery copy: could not write {}: {e}", path.display());
            return None;
        }
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
        self.timelapse.saved_as(path);
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

/// A copy no longer than 512 px on its long side: the CPU previews and
/// the GPU proxy draw from it, and it is all that stays in memory of a
/// page between uses.
fn shrink_for_preview(t: ayatsuri_render::TexData) -> ayatsuri_render::TexData {
    let long = t.width.max(t.height).max(1);
    if long <= 512 {
        return t;
    }
    let k = 512.0 / long as f32;
    let (w, h) = (((t.width as f32 * k) as u32).max(1), ((t.height as f32 * k) as u32).max(1));
    match image::RgbaImage::from_raw(t.width, t.height, t.rgba) {
        Some(img) => {
            let small = image::imageops::resize(&img, w, h, image::imageops::FilterType::Triangle);
            ayatsuri_render::TexData { width: w, height: h, rgba: small.into_raw() }
        }
        None => ayatsuri_render::TexData { width: 1, height: 1, rgba: vec![255; 4] },
    }
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
    // blend shape keys, with the base key at 0.0 among them
    for shape in model.art_meshes.iter().flat_map(|m| &m.blend_shapes) {
        if let Some(keys) = param_keys.get_mut(shape.parameter as usize) {
            keys.extend_from_slice(&shape.keys);
            keys.push(0.0);
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

/// Deformer names beside an edited model. The same count keeps them by
/// index, so changing a deformer's ID (or undoing that) keeps its name;
/// when deformers were added or removed they follow by ID, and a new one
/// starts without a name.
fn follow_deformer_names(names: &[String], now: &Model, next: &Model) -> Vec<String> {
    if now.deformers.len() == next.deformers.len() {
        let mut out = names.to_vec();
        out.resize(next.deformers.len(), String::new());
        return out;
    }
    let by_id: HashMap<&str, &String> =
        now.deformers.iter().zip(names).map(|(d, n)| (d.id.as_str(), n)).collect();
    next.deformers
        .iter()
        .map(|d| by_id.get(d.id.as_str()).map_or_else(String::new, |n| (*n).clone()))
        .collect()
}

impl Session {
    /// What a deformer is called on screen: its name, or its ID when it has
    /// none.
    pub fn deformer_name(&self, i: usize) -> String {
        match self.deformer_names.get(i).filter(|n| !n.trim().is_empty()) {
            Some(n) => n.clone(),
            None => self.puppet.model().deformers.get(i).map(|d| d.id.clone()).unwrap_or_default(),
        }
    }
}

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
                blend_shape: false,
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
                    blend: ayatsuri_import::Blend::NORMAL,
                })
                .collect(),
            folders: Vec::new(),
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
    fn bezier_handles_are_saved_with_the_model() {
        use ayatsuri_format::model::{Binding, Deformer, DeformerKind, WarpData};
        let mut m = Model::new();
        let grid: Vec<f32> =
            (0..9).flat_map(|i| [(i % 3) as f32 * 50.0, (i / 3) as f32 * 50.0]).collect();
        m.deformers.push(Deformer {
            id: "Warp7".into(),
            parent_part: None,
            parent_deformer: None,
            visible: true,
            enabled: true,
            binding: Binding::default(),
            kind: DeformerKind::Warp(WarpData {
                rows: 2,
                cols: 2,
                quad: false,
                opacities: vec![1.0],
                positions: grid,
            }),
            multiply: Vec::new(),
            screen: Vec::new(),
            blend_shapes: Vec::new(),
        });
        let ctx = Context::default();
        let mut s = Session::create(&ctx, "t".into(), String::new(), m).unwrap();
        let net: Vec<(f32, f32)> = (0..16).map(|i| (i as f32, -(i as f32))).collect();
        s.bezier_nets
            .insert((0, 0), ayatsuri_core::bezier::Lattice { bw: 1, bh: 1, net: net.clone() });
        let path = std::env::temp_dir().join(format!("ay-bez-{}.aom", std::process::id()));
        s.save_aom(&path).unwrap();
        let back = Session::load(&ctx, &path).unwrap();
        let _ = std::fs::remove_file(&path);
        let got = back.bezier_nets.get(&(0, 0)).expect("handles came back");
        assert_eq!((got.bw, got.bh), (1, 1));
        assert_eq!(got.net, net);
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
    fn a_new_mesh_id_keeps_its_name_visibility_and_undo_too() {
        let ctx = Context::default();
        let mut s = Session::create(&ctx, "t".into(), String::new(), Model::new()).unwrap();
        let art = ayatsuri_import::from_layers(&doc(
            200,
            100,
            &[("Hair", "Head", 60, 20, 16), ("Bang", "Head", 5, 5, 8)],
        ))
        .unwrap();
        s.add_imported(&ctx, art, Path::new("a.psd")).unwrap();
        s.mesh_shown[1] = false;
        assert!(s.edit(|m| ayatsuri_core::edit::rename_art_mesh(m, 1, "ArtMeshBang")));
        assert_eq!(s.puppet.model().art_meshes[1].id, "ArtMeshBang");
        assert_eq!((s.mesh_name(0), s.mesh_name(1)), ("Hair".into(), "Bang".into()));
        assert_eq!(s.mesh_shown, vec![true, false]);
        s.undo();
        assert_eq!(s.puppet.model().art_meshes[1].id, "ArtMesh1");
        assert_eq!(s.mesh_name(1), "Bang");
        assert_eq!(s.mesh_shown, vec![true, false]);
        // taken IDs are refused
        assert!(!s.edit(|m| ayatsuri_core::edit::rename_art_mesh(m, 1, "ArtMesh0")));
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
    fn undo_history_is_capped_by_steps_and_by_size() {
        let step = |n: usize| {
            let mut m = Model::new();
            m.art_meshes.push(ayatsuri_format::model::ArtMesh {
                id: format!("M{n}"),
                parent_part: None,
                parent_deformer: None,
                visible: true,
                enabled: true,
                texture: 0,
                flags: 0,
                blend: Default::default(),
                vertex_count: 0,
                uvs: vec![],
                indices: vec![],
                masks: vec![],
                binding: ayatsuri_format::model::Binding::default(),
                opacities: vec![1.0],
                draw_orders: vec![500.0],
                positions: vec![0.0; 25_000],
                blend_shapes: Vec::new(),
                clip_group: None,
                clip_inherit: false,
                multiply: Vec::new(),
                screen: Vec::new(),
            });
            m
        };
        let mut h: Vec<Model> = (0..20).map(step).collect();
        trim_history_to(&mut h, 16, usize::MAX);
        assert_eq!(h.len(), 16);
        assert_eq!(h[0].art_meshes[0].id, "M4", "the oldest went first");
        trim_history_to(&mut h, 16, 350_000);
        assert_eq!(h.len(), 3);
        assert_eq!(h[2].art_meshes[0].id, "M19", "the newest stays");
        trim_history_to(&mut h, 16, 1);
        assert_eq!(h.len(), 2);
    }

    #[test]
    fn pages_no_mesh_uses_are_dropped_and_the_rest_renumbered() {
        use ayatsuri_format::model::{ArtMesh, Binding};
        let mesh = |id: &str, texture: u32| ArtMesh {
            id: id.into(),
            parent_part: None,
            parent_deformer: None,
            visible: true,
            enabled: true,
            texture,
            flags: 0,
            blend: Default::default(),
            vertex_count: 0,
            uvs: vec![],
            indices: vec![],
            masks: vec![],
            binding: Binding::default(),
            opacities: vec![1.0],
            draw_orders: vec![500.0],
            positions: vec![],
            blend_shapes: Vec::new(),
            clip_group: None,
            clip_inherit: false,
            multiply: Vec::new(),
            screen: Vec::new(),
        };
        let mut m = Model::new();
        m.art_meshes = vec![mesh("A", 0), mesh("B", 2), mesh("C", 2)];
        let pngs = vec![vec![0u8], vec![1u8], vec![2u8], vec![3u8]];
        let (kept, dropped) = prune_pages(&mut m, pngs);
        assert_eq!(dropped, 2);
        assert_eq!(kept, vec![vec![0u8], vec![2u8]]);
        let pages: Vec<u32> = m.art_meshes.iter().map(|a| a.texture).collect();
        assert_eq!(pages, vec![0, 1, 1]);
    }

    #[test]
    fn layers_wait_on_their_own_pages_until_the_atlas_is_made() {
        let ctx = Context::default();
        let mut s = Session::create(&ctx, "t".into(), String::new(), Model::new()).unwrap();
        let art = ayatsuri_import::from_layers(&doc(
            400,
            300,
            &[("Body", "Torso", 10, 10, 30), ("Arm", "Torso", 100, 40, 20)],
        ))
        .unwrap();
        s.add_imported(&ctx, art, Path::new("a.psd")).unwrap();
        assert_eq!(s.texture_pngs.len(), 2, "one page per layer");
        assert!(s.atlas_pending());
        let loose = s.puppet.model().clone();
        let loose_art = s.art_boxes();

        let sizes = s.page_sizes();
        let pieces = crate::atlas::pieces(s.puppet.model(), &sizes, &s.art_boxes());
        let layout = crate::atlas::auto_layout(&pieces, 256, false).unwrap();
        s.apply_atlas(&ctx, &pieces, &layout).unwrap();
        assert!(!s.atlas_pending());
        let packed = s.puppet.model().clone();
        assert!(packed.art_meshes.iter().all(|m| m.texture == 2), "both on the new page");

        // undo finds the layer pages and their art boxes again
        s.undo();
        assert!(s.atlas_pending());
        for (m, l) in s.puppet.model().art_meshes.iter().zip(&loose.art_meshes) {
            assert_eq!((m.texture, &m.uvs), (l.texture, &l.uvs));
        }
        assert_eq!(s.art_boxes(), loose_art);
        s.redo();
        assert!(!s.atlas_pending());

        // saving leaves the old pages out; the layer pages stay marked
        let dir = std::env::temp_dir().join(format!("ayatsuri-atlas-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("t.aom");
        s.save_aom(&path).unwrap();
        let back = Session::load_aom(&ctx, &path).unwrap();
        assert_eq!(back.texture_pngs.len(), 1);
        assert!(!back.atlas_pending());
        s.undo();
        s.save_aom(&path).unwrap();
        let back = Session::load_aom(&ctx, &path).unwrap();
        assert_eq!(back.texture_pngs.len(), 2);
        assert!(back.atlas_pending(), "still waiting for the atlas after reopening");
        let _ = std::fs::remove_dir_all(&dir);
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
        // and set to Multiply in the paint program
        updated.layers[0].blend.color = ayatsuri_import::ColorBlend::MultiplyCompatible;
        let r = reimport_into(&mut s, &ctx, updated).unwrap();
        assert_eq!(r.updated, 1);
        assert_eq!(r.new_layers, vec!["Torso/Sleeve".to_string()]);
        assert_eq!(r.missing, vec!["Arm".to_string()]);
        let m = s.puppet.model();
        // Body reads new art from a new page; its points did not move
        assert!(m.art_meshes[0].texture as usize >= pages);
        assert_eq!(m.art_meshes[0].positions, before.art_meshes[0].positions);
        assert_ne!(m.art_meshes[0].uvs, before.art_meshes[0].uvs);
        assert_eq!(
            m.art_meshes[0].flags & flags::BLEND_MULTIPLICATIVE,
            flags::BLEND_MULTIPLICATIVE
        );
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

/// One line per model edit, in the words the timelapse player uses.
fn log_edit(before: &Model, after: &Model) {
    use crate::model_diff::{Kind, What};
    let changes = crate::model_diff::diff(before, after);
    if changes.is_empty() {
        debug!("model edit with no visible change");
        return;
    }
    let words: Vec<String> = changes
        .iter()
        .take(4)
        .map(|c| {
            let what = match c.what {
                What::Canvas => "canvas",
                What::Parameter => "parameter",
                What::Part => "part",
                What::Warp => "warp deformer",
                What::Rotation => "rotation deformer",
                What::Mesh => "mesh",
                What::Glue => "glue",
                What::Physics => "physics",
                What::ArtPath => "ArtPath",
            };
            let how = match c.kind {
                Kind::Added => "added",
                Kind::Removed => "removed",
                Kind::Edited => c.detail,
            };
            match c.what {
                What::Canvas | What::Physics => format!("{what}: {how}"),
                _ => format!("{what} {}: {how}", c.name),
            }
        })
        .collect();
    let more = changes.len().saturating_sub(words.len());
    if more > 0 {
        info!("edit: {} and {more} more", words.join(", "));
    } else {
        info!("edit: {}", words.join(", "));
    }
}

#[cfg(test)]
mod deformer_name_tests {
    use super::*;
    use ayatsuri_format::model::{Deformer, DeformerKind, WarpData};

    fn with(ids: &[&str]) -> Model {
        let mut m = Model::new();
        m.deformers = ids
            .iter()
            .map(|id| Deformer {
                id: (*id).to_string(),
                parent_part: None,
                parent_deformer: None,
                visible: true,
                enabled: true,
                binding: Default::default(),
                kind: DeformerKind::Warp(WarpData {
                    rows: 1,
                    cols: 1,
                    quad: false,
                    opacities: vec![1.0],
                    positions: vec![0.0; 8],
                }),
                multiply: Vec::new(),
                screen: Vec::new(),
                blend_shapes: Vec::new(),
            })
            .collect();
        m
    }

    fn names(n: &[&str]) -> Vec<String> {
        n.iter().map(|s| (*s).to_string()).collect()
    }

    #[test]
    fn a_new_id_keeps_the_name() {
        let out = follow_deformer_names(
            &names(&["Hair", "Face"]),
            &with(&["A", "B"]),
            &with(&["A", "X"]),
        );
        assert_eq!(out, names(&["Hair", "Face"]));
    }

    #[test]
    fn names_follow_their_deformer_when_one_is_added_or_removed() {
        let now = with(&["A", "B", "C"]);
        let gone = follow_deformer_names(&names(&["a", "b", "c"]), &now, &with(&["A", "C"]));
        assert_eq!(gone, names(&["a", "c"]));
        let added =
            follow_deformer_names(&names(&["a", "b", "c"]), &now, &with(&["N", "A", "B", "C"]));
        assert_eq!(added, names(&["", "a", "b", "c"]));
    }
}
