// Art meshes on the system clipboard, so Ctrl+V works (egui only sends a
// paste when the clipboard holds text) and meshes paste into another
// project. The text is a marker line and JSON: each mesh with every
// keyform, its ArtPath, the IDs (not indices) of its parameters, parents
// and masks, its positions as drawn, and its artwork as a PNG crop.
//
// Pasting into the project it came from keeps its texture and parents as
// they are. Into another one: parameters match by ID (a missing one is
// created with the same range), parents by ID, and the artwork becomes a
// new texture page. A mesh whose parent deformer is not there loses its
// keys and sits where it was drawn, since its points only meant something
// in that deformer's space.

use std::path::Path;

use ayatsuri_format::model::{ArtMesh, ArtPath, Model};
use base64::Engine;
use serde::{Deserialize, Serialize};

use crate::session::Session;

/// The first line of the clipboard text.
pub(crate) const MARK: &str = "Ayatsuri2D meshes v1";

#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct ClipFile {
    /// Which project it came from: pasting back into it keeps texture and
    /// parents as they are.
    project: String,
    pub(crate) meshes: Vec<ClipMesh>,
}

#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct ClipMesh {
    pub(crate) name: String,
    pub(crate) mesh: ArtMesh,
    pub(crate) path: Option<ArtPath>,
    /// Per binding axis, in order: the parameter's id, min, default, max.
    params: Vec<(String, f32, f32, f32)>,
    part: Option<String>,
    deformer: Option<String>,
    masks: Vec<String>,
    /// Positions as drawn when copied, model units in the canvas frame.
    drawn: Vec<f32>,
    /// The mesh's patch of its texture page, as PNG, and where it was:
    /// x, y, width, height in page pixels.
    png: String,
    patch: [u32; 4],
    /// The size of the page it was cut from, in pixels.
    page: [u32; 2],
}

/// What identifies a project on the clipboard.
pub(crate) fn project_key(s: &Session) -> String {
    s.aom_path
        .as_deref()
        .map_or_else(|| format!("unsaved:{}", s.name), |p: &Path| p.display().to_string())
}

/// The clipboard file for some meshes of a session.
pub(crate) fn copy(s: &Session, meshes: &[usize]) -> ClipFile {
    let model = s.puppet.model();
    let sizes = s.page_sizes();
    let out = meshes
        .iter()
        .filter_map(|&mi| {
            let mesh = model.art_meshes.get(mi)?.clone();
            let (pw, ph) = sizes.get(mesh.texture as usize).copied().unwrap_or((1, 1));
            let (u0, v0, u1, v1) =
                mesh.uvs.chunks(2).fold((f32::MAX, f32::MAX, f32::MIN, f32::MIN), |b, c| match c {
                    [u, v] => (b.0.min(*u), b.1.min(*v), b.2.max(*u), b.3.max(*v)),
                    _ => b,
                });
            let x = (u0 * pw as f32).floor().max(0.0) as u32;
            let y = (v0 * ph as f32).floor().max(0.0) as u32;
            let w = ((u1 * pw as f32).ceil() as u32).min(pw).saturating_sub(x).max(1);
            let h = ((v1 * ph as f32).ceil() as u32).min(ph).saturating_sub(y).max(1);
            let png = s
                .texture_pngs
                .get(mesh.texture as usize)
                .and_then(|p| image::load_from_memory(p).ok())
                .map(|img| {
                    let crop = img.crop_imm(x, y, w, h);
                    let mut out = Vec::new();
                    let _ =
                        crop.write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png);
                    base64::engine::general_purpose::STANDARD.encode(out)
                })
                .unwrap_or_default();
            let id_of = |i: u32| model.art_meshes.get(i as usize).map(|m| m.id.clone());
            Some(ClipMesh {
                name: s.mesh_name(mi),
                path: ayatsuri_core::edit::art_path_of(model, mi).cloned(),
                params: mesh
                    .binding
                    .axes
                    .iter()
                    .filter_map(|a| {
                        let p = model.parameters.get(a.parameter as usize)?;
                        Some((p.id.clone(), p.min, p.default, p.max))
                    })
                    .collect(),
                part: mesh
                    .parent_part
                    .and_then(|p| model.parts.get(p as usize))
                    .map(|p| p.id.clone()),
                deformer: mesh
                    .parent_deformer
                    .and_then(|d| model.deformers.get(d as usize))
                    .map(|d| d.id.clone()),
                masks: mesh.masks.iter().filter_map(|&m| id_of(m)).collect(),
                drawn: s.puppet.mesh_drawn_positions(mi).unwrap_or_default(),
                png,
                patch: [x, y, w, h],
                page: [pw, ph],
                mesh,
            })
        })
        .collect();
    ClipFile { project: project_key(s), meshes: out }
}

impl ClipFile {
    /// The clipboard text: the marker line, then JSON.
    pub(crate) fn to_text(&self) -> String {
        format!("{MARK}\n{}", serde_json::to_string(self).unwrap_or_default())
    }

    /// Back from clipboard text, if it is ours.
    pub(crate) fn from_text(text: &str) -> Option<Self> {
        let json = text.strip_prefix(MARK)?.trim_start();
        serde_json::from_str(json).ok()
    }
}

/// Paste into a session as one undo step; returns the new meshes' indices
/// with the names they should carry.
pub(crate) fn paste(
    s: &mut Session,
    ctx: &egui::Context,
    clip: &ClipFile,
) -> Result<Vec<(usize, String)>, String> {
    let home = clip.project == project_key(s);
    let mut next: Model = s.puppet.model().clone();
    let mut pngs = s.texture_pngs.clone();
    let mut added = Vec::new();
    for c in &clip.meshes {
        let mut mesh = c.mesh.clone();
        let kept = !next.art_meshes.iter().any(|m| m.id == mesh.id);
        let mut path = c.path.clone();
        if !home {
            // parameters by id, created when missing
            for (axis, (id, min, default, max)) in mesh.binding.axes.iter_mut().zip(&c.params) {
                let at = match next.parameters.iter().position(|p| &p.id == id) {
                    Some(i) => i,
                    None => ayatsuri_core::edit::create_parameter(
                        &mut next,
                        id.clone(),
                        *min,
                        *default,
                        *max,
                    )
                    .ok_or_else(|| format!("could not make parameter {id}"))?,
                };
                axis.parameter = at as u32;
            }
            mesh.parent_part = c
                .part
                .as_ref()
                .and_then(|id| next.parts.iter().position(|p| &p.id == id))
                .map(|i| i as u32);
            let parent =
                c.deformer.as_ref().and_then(|id| next.deformers.iter().position(|d| &d.id == id));
            if parent.is_none() && c.deformer.is_some() {
                // its points only meant something in that deformer's space:
                // it keeps the pose it was drawn in, without keys
                mesh.parent_deformer = None;
                mesh.binding = Default::default();
                mesh.positions = c.drawn.clone();
                mesh.opacities.truncate(1);
                mesh.draw_orders.truncate(1);
            } else {
                mesh.parent_deformer = parent.map(|i| i as u32);
            }
            mesh.masks = c
                .masks
                .iter()
                .filter_map(|id| next.art_meshes.iter().position(|m| &m.id == id).map(|i| i as u32))
                .collect();
            // the artwork comes along as a page of its own
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(&c.png)
                .map_err(|e| e.to_string())?;
            let [x, y, w, h] = c.patch;
            let (pw, ph) = (c.page[0].max(1) as f32, c.page[1].max(1) as f32);
            for uv in mesh.uvs.chunks_mut(2) {
                if let [u, v] = uv {
                    *u = ((*u * pw - x as f32) / w.max(1) as f32).clamp(0.0, 1.0);
                    *v = ((*v * ph - y as f32) / h.max(1) as f32).clamp(0.0, 1.0);
                }
            }
            mesh.texture = pngs.len() as u32;
            pngs.push(bytes);
            // a path's colours live on the swatch page, which does not come
            // along: the mesh keeps its pixels (the crop) and drops the path
            path = None;
        }
        let beside = if home { Some(mesh.id.clone()) } else { None };
        let i = ayatsuri_core::edit::insert_art_mesh(&mut next, mesh, path, beside.as_deref());
        let name = if kept || !home {
            c.name.clone()
        } else {
            crate::i18n::trf("{name} copy", &[("name", &c.name)])
        };
        added.push((i, name));
    }
    if home {
        // same texture pages: an ordinary edit
        if !s.edit(|m| {
            *m = next;
            true
        }) {
            return Err("nothing to paste".into());
        }
    } else {
        s.swap_in_pub(ctx, next, pngs)?;
    }
    Ok(added)
}
