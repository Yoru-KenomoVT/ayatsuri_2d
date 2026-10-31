// Art meshes, and deformers with everything under them, on the system
// clipboard, as Cubism copies objects. Art meshes go so Ctrl+V works (egui only sends a
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

use ayatsuri_format::model::{ArtMesh, ArtPath, Deformer, DeformerKind, Model};
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
    /// Deformers copied with their children, parents first.
    #[serde(default)]
    pub(crate) deformers: Vec<ClipDeformer>,
    pub(crate) meshes: Vec<ClipMesh>,
}

#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct ClipDeformer {
    pub(crate) name: String,
    pub(crate) deformer: Deformer,
    /// Per binding axis, then per blend shape: parameter id, min, default,
    /// max, and whether it is a blend shape parameter.
    params: Vec<(String, f32, f32, f32, bool)>,
    shape_params: Vec<(String, f32, f32, f32, bool)>,
    part: Option<String>,
    /// Its parent deformer's id, when it has one.
    parent: Option<String>,
    /// The same deformer carried out of its parent into canvas space at the
    /// rest pose, for a paste where that parent is missing.
    rooted: Option<Deformer>,
}

#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct ClipMesh {
    pub(crate) name: String,
    pub(crate) mesh: ArtMesh,
    pub(crate) path: Option<ArtPath>,
    /// Per binding axis, in order: the parameter's id, min, default, max.
    params: Vec<(String, f32, f32, f32)>,
    /// Per blend shape: its parameter's id, min, default, max, and that it
    /// is a blend shape parameter.
    #[serde(default)]
    shape_params: Vec<(String, f32, f32, f32, bool)>,
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

/// A parameter as the clipboard names it.
fn param_entry(model: &Model, p: u32) -> Option<(String, f32, f32, f32, bool)> {
    let q = model.parameters.get(p as usize)?;
    Some((q.id.clone(), q.min, q.default, q.max, q.blend_shape))
}

/// Deformer `d` carried from its parent's space into the canvas at the rest
/// pose: its grid points or rotation origins, in every keyform and shape.
fn rooted(rest: &ayatsuri_core::Puppet, d: &Deformer) -> Option<Deformer> {
    let parent = d.parent_deformer? as usize;
    let map = |x: f32, y: f32| rest.deformer_point(parent, x, y);
    let mut out = d.clone();
    out.parent_deformer = None;
    let map_pairs = |v: &mut Vec<f32>| {
        for p in v.as_chunks_mut::<2>().0 {
            let (x, y) = map(p[0], p[1]);
            p[0] = x;
            p[1] = y;
        }
    };
    match &mut out.kind {
        DeformerKind::Warp(w) => {
            map_pairs(&mut w.positions);
            for b in &mut out.blend_shapes {
                map_pairs(&mut b.positions);
            }
        }
        DeformerKind::Rotation(r) => {
            for (x, y) in r.origins_x.iter_mut().zip(r.origins_y.iter_mut()) {
                (*x, *y) = map(*x, *y);
            }
        }
    }
    Some(out)
}

/// The clipboard file for deformers (each with every deformer and art mesh
/// under it) and art meshes of a session.
pub(crate) fn copy_objects(s: &Session, deformers: &[usize], meshes: &[usize]) -> ClipFile {
    let model = s.puppet.model();
    // the deformers and everything under them
    let mut defs: Vec<usize> = Vec::new();
    for (i, d) in model.deformers.iter().enumerate() {
        let mut cur = Some(i as u32);
        let mut hops = 0;
        while let Some(c) = cur {
            if deformers.contains(&(c as usize)) {
                defs.push(i);
                break;
            }
            cur = model.deformers.get(c as usize).and_then(|x| x.parent_deformer);
            hops += 1;
            if hops > 1024 {
                break;
            }
        }
        let _ = d;
    }
    let mut all_meshes: Vec<usize> = meshes.to_vec();
    for (mi, m) in model.art_meshes.iter().enumerate() {
        if m.parent_deformer.is_some_and(|p| defs.contains(&(p as usize)))
            && !all_meshes.contains(&mi)
        {
            all_meshes.push(mi);
        }
    }
    all_meshes.sort_unstable();
    let mut clip = copy(s, &all_meshes);
    let rest = ayatsuri_core::Puppet::new(model.clone());
    let mut rest = rest;
    rest.update();
    clip.deformers = defs
        .iter()
        .filter_map(|&di| {
            let d = model.deformers.get(di)?;
            let parent = d.parent_deformer.and_then(|p| model.deformers.get(p as usize));
            Some(ClipDeformer {
                name: s
                    .deformer_names
                    .get(di)
                    .cloned()
                    .filter(|n| !n.trim().is_empty())
                    .unwrap_or_else(|| d.id.clone()),
                params: d
                    .binding
                    .axes
                    .iter()
                    .filter_map(|a| param_entry(model, a.parameter))
                    .collect(),
                shape_params: d
                    .blend_shapes
                    .iter()
                    .filter_map(|b| param_entry(model, b.parameter))
                    .collect(),
                part: d.parent_part.and_then(|p| model.parts.get(p as usize)).map(|p| p.id.clone()),
                parent: parent.map(|p| p.id.clone()),
                // only a deformer whose parent is not copied along can lose it
                rooted: d
                    .parent_deformer
                    .filter(|p| !defs.contains(&(*p as usize)))
                    .and_then(|_| rooted(&rest, d)),
                deformer: d.clone(),
            })
        })
        .collect();
    clip
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
                shape_params: mesh
                    .blend_shapes
                    .iter()
                    .filter_map(|b| param_entry(model, b.parameter))
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
    ClipFile { project: project_key(s), deformers: Vec::new(), meshes: out }
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

/// What a paste made: the new art meshes' and deformers' indices, each
/// with the name it should carry.
pub(crate) struct Pasted {
    pub(crate) meshes: Vec<(usize, String)>,
    pub(crate) deformers: Vec<(usize, String)>,
}

/// Paste into a session as one undo step.
pub(crate) fn paste(
    s: &mut Session,
    ctx: &egui::Context,
    clip: &ClipFile,
) -> Result<Pasted, String> {
    let home = clip.project == project_key(s);
    let mut next: Model = s.puppet.model().clone();
    let mut pngs = s.texture_pngs.clone();
    let mut added = Vec::new();
    // a parameter by id, made with the same range when missing
    let param = |next: &mut Model, e: &(String, f32, f32, f32, bool)| -> Result<u32, String> {
        let (id, min, default, max, blend) = e;
        if let Some(i) = next.parameters.iter().position(|p| &p.id == id) {
            return Ok(i as u32);
        }
        let i = ayatsuri_core::edit::create_parameter(next, id.clone(), *min, *default, *max)
            .ok_or_else(|| format!("could not make parameter {id}"))?;
        if let Some(p) = next.parameters.get_mut(i) {
            p.blend_shape = *blend;
        }
        Ok(i as u32)
    };
    // the deformers, after every one there, parents first as they came: each
    // pasted one's index by the id it was copied with
    let mut pasted_defs: std::collections::HashMap<String, usize> =
        std::collections::HashMap::new();
    let mut added_defs = Vec::new();
    for c in &clip.deformers {
        let copied_parent = c.parent.as_ref().and_then(|id| pasted_defs.get(id).copied());
        let there =
            c.parent.as_ref().and_then(|id| next.deformers.iter().position(|d| &d.id == id));
        let mut d = match (copied_parent, there, &c.rooted) {
            (None, None, Some(r)) => r.clone(),
            _ => c.deformer.clone(),
        };
        d.parent_deformer = copied_parent.or(there).map(|i| i as u32);
        if c.parent.is_some() && d.parent_deformer.is_none() && c.rooted.is_none() {
            d.parent_deformer = None;
        }
        if !home {
            for (axis, e) in d.binding.axes.iter_mut().zip(&c.params) {
                axis.parameter = param(&mut next, e)?;
            }
            for (b, e) in d.blend_shapes.iter_mut().zip(&c.shape_params) {
                b.parameter = param(&mut next, e)?;
            }
            d.parent_part = c
                .part
                .as_ref()
                .and_then(|id| next.parts.iter().position(|p| &p.id == id))
                .map(|i| i as u32);
        }
        // a fresh id: the original's with _copy, numbered when taken
        let base = format!("{}_copy", c.deformer.id);
        let mut id = base.clone();
        let mut n = 2;
        while next.deformers.iter().any(|x| x.id == id) {
            id = format!("{base}{n}");
            n += 1;
        }
        d.id = id;
        next.deformers.push(d);
        let i = next.deformers.len() - 1;
        pasted_defs.insert(c.deformer.id.clone(), i);
        added_defs.push((i, crate::i18n::trf("{name} copy", &[("name", &c.name)]).to_string()));
    }
    // each pasted mesh's index, by the id it was copied with
    let mut copies: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for c in &clip.meshes {
        let mut mesh = c.mesh.clone();
        // under a copied deformer: under its copy, keys and all
        let under_copy = c.deformer.as_ref().and_then(|id| pasted_defs.get(id).copied());
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
            // blend shapes name their parameter by index too
            for (b, e) in mesh.blend_shapes.iter_mut().zip(&c.shape_params) {
                b.parameter = param(&mut next, e)?;
            }
            mesh.parent_part = c
                .part
                .as_ref()
                .and_then(|id| next.parts.iter().position(|p| &p.id == id))
                .map(|i| i as u32);
            let parent = under_copy.or_else(|| {
                c.deformer.as_ref().and_then(|id| next.deformers.iter().position(|d| &d.id == id))
            });
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
        if home && let Some(k) = under_copy {
            mesh.parent_deformer = Some(k as u32);
        }
        let beside = if home { Some(mesh.id.clone()) } else { None };
        let original = mesh.id.clone();
        let i = ayatsuri_core::edit::insert_art_mesh(&mut next, mesh, path, beside.as_deref());
        copies.insert(original, i);
        let name = if kept || !home {
            c.name.clone()
        } else {
            crate::i18n::trf("{name} copy", &[("name", &c.name)])
        };
        added.push((i, name));
    }
    // a mask copied along clips the copies (an eye's white clips the copied
    // iris, not the old one); the rest stay on the meshes they named. Such a
    // copy leaves its clip group, whose masks are the originals'.
    for (c, &(i, _)) in clip.meshes.iter().zip(&added) {
        let mut moved = false;
        let masks: Vec<u32> = c
            .masks
            .iter()
            .filter_map(|id| match copies.get(id) {
                Some(&k) => {
                    moved = true;
                    Some(k)
                }
                None => next.art_meshes.iter().position(|m| &m.id == id),
            })
            .filter(|&k| k != i)
            .map(|k| k as u32)
            .collect();
        if let Some(m) = next.art_meshes.get_mut(i) {
            m.masks = masks;
            if moved {
                m.clip_group = None;
            }
        }
    }
    // at a tied draw order the later mesh draws in front, and a pasted one
    // is the last of all: the copies move, as one block in the order they
    // came, to just after the front-most of their originals, so they stack
    // right over them and not over everything (hair over a copied eye)
    let before = next.art_meshes.len() - added.len();
    let front = clip
        .meshes
        .iter()
        .filter_map(|c| next.art_meshes.iter().take(before).position(|m| m.id == c.mesh.id))
        .max();
    if let Some(front) = front {
        let pasted: Vec<usize> = added.iter().map(|&(i, _)| i).collect();
        let mut order: Vec<usize> = (0..=front).collect();
        order.extend(&pasted);
        order.extend((front + 1..next.art_meshes.len()).filter(|i| !pasted.contains(i)));
        if ayatsuri_core::palette::reorder_meshes(&mut next, &order) {
            for (k, (i, _)) in added.iter_mut().enumerate() {
                *i = front + 1 + k;
            }
        }
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
    Ok(Pasted { meshes: added, deformers: added_defs })
}

/// The art meshes a Clip IDs text names, for `mesh` to clip by: IDs split
/// by commas, semicolons or spaces, in the order typed, each once. Also
/// gives back the words that named no other mesh (unknown IDs, or the
/// mesh's own).
pub(crate) fn resolve_ids(model: &Model, mesh: usize, text: &str) -> (Vec<u32>, Vec<String>) {
    let ids: Vec<&str> = model.art_meshes.iter().map(|m| m.id.as_str()).collect();
    resolve_in(&ids, mesh, text)
}

fn resolve_in(ids: &[&str], mesh: usize, text: &str) -> (Vec<u32>, Vec<String>) {
    let (mut found, mut unknown) = (Vec::new(), Vec::new());
    for word in text.split([',', ';', ' ', '\t', '\n', '\r']).filter(|w| !w.is_empty()) {
        match ids.iter().position(|&id| id == word) {
            Some(mi) if mi != mesh => {
                if !found.contains(&(mi as u32)) {
                    found.push(mi as u32);
                }
            }
            _ => unknown.push(word.to_string()),
        }
    }
    (found, unknown)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clip_ids_resolve_in_order_once() {
        let ids = ["Eye", "Iris", "Hi"];
        let (found, unknown) = resolve_in(&ids, 2, "Iris, Eye;Iris  Nope,Hi");
        assert_eq!(found, vec![1, 0]);
        assert_eq!(unknown, vec!["Nope".to_string(), "Hi".to_string()]);
        assert_eq!(resolve_in(&ids, 0, " , ").0, Vec::<u32>::new());
    }

    #[test]
    fn a_copied_deformer_pastes_with_its_mesh_under_the_copy() {
        use ayatsuri_core::edit::{Child, NewDeformer};
        use ayatsuri_format::model::Binding;
        let mut m = Model::new();
        m.art_meshes.push(ArtMesh {
            id: "Hair".into(),
            parent_part: None,
            parent_deformer: None,
            visible: true,
            enabled: true,
            texture: 0,
            flags: 0,
            blend: Default::default(),
            vertex_count: 3,
            uvs: vec![0.0, 0.0, 1.0, 0.0, 0.0, 1.0],
            indices: vec![0, 1, 2],
            masks: vec![],
            binding: Binding::default(),
            opacities: vec![1.0],
            draw_orders: vec![500.0],
            positions: vec![0.0, 0.0, 1.0, 0.0, 0.0, 1.0],
            blend_shapes: Vec::new(),
            clip_group: None,
            clip_inherit: false,
            multiply: Vec::new(),
            screen: Vec::new(),
        });
        let warp = NewDeformer::Warp { rows: 2, cols: 2 };
        ayatsuri_core::edit::create_deformer(
            &mut m,
            warp,
            &[Child::Mesh(0)],
            None,
            (0.0, 0.0, 1.0, 1.0),
            None,
            "HairWarp".into(),
        )
        .unwrap();
        let ctx = egui::Context::default();
        let mut s = Session::create(&ctx, "t".into(), String::new(), m).unwrap();
        let clip = copy_objects(&s, &[0], &[]);
        assert_eq!((clip.deformers.len(), clip.meshes.len()), (1, 1));
        let pasted = paste(&mut s, &ctx, &clip).unwrap();
        let model = s.puppet.model();
        assert_eq!(model.deformers.len(), 2);
        let (d, _) = pasted.deformers[0];
        assert_eq!(model.deformers[d].id, "HairWarp_copy");
        let (mi, _) = pasted.meshes[0];
        assert_eq!(model.art_meshes[mi].parent_deformer, Some(d as u32));
        // the original keeps its own
        let orig = model.art_meshes.iter().position(|a| a.id == "Hair").unwrap();
        assert_eq!(model.art_meshes[orig].parent_deformer, Some(0));
        // drawn where the original is
        let mut p = ayatsuri_core::Puppet::new(model.clone());
        p.update();
        assert_eq!(p.mesh_drawn_positions(orig), p.mesh_drawn_positions(mi));
    }
}
