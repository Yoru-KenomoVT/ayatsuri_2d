// From the extract to a model: one padded layer per art mesh goes through
// the layer importer (which packs the art and cuts the quads), then each
// mesh takes its real points, triangles, uvs and keyforms, the deformers
// and parameters come in with theirs, and physics follows physics3's
// shape. At the end the model is evaluated at rest and every mesh is
// checked against where the project says it sits.
//
// Ayatsuri's top level is model units ((px - origin) / pixels per unit)
// and a rotation's offsets are in those units too, so pixels divide by
// pixels per unit there; under a warp the 0..1 grid space is shared.

use ayatsuri_format::model::{
    Binding, BindingAxis, Deformer, DeformerKind, Model, Parameter, PhysicsGroup, PhysicsInput,
    PhysicsOutput, PhysicsRange, PhysicsRig, RotationData, WarpData,
};

use super::extract::{self, Extract, Form};
use crate::ImportError;
use crate::layers::{self, Document, Imported, Layer};

fn err(msg: impl Into<String>) -> ImportError {
    ImportError::Export(format!("cmo3: {}", msg.into()))
}

/// Transparent room around each layer, so a mesh's margin reads empty
/// texture, never a neighbour's.
const PAD: u32 = 32;

/// Which space a form's points are in, from its parent.
#[derive(Clone, Copy)]
enum Space {
    Root,
    Warp,
    Rotation,
    /// A rotation that itself sits inside a warp.
    RotationInWarp,
}

/// What comes back from a project.
pub struct Cmo3 {
    pub imported: Imported,
    /// Display names, parallel to the model's parameters.
    pub param_names: Vec<String>,
    /// Parameter folders in palette order: name and member indices.
    pub param_groups: Vec<(String, Vec<u32>)>,
    /// Meshes within a pixel of the project at rest, and meshes checked.
    pub within: (usize, usize),
    /// The meshes that landed elsewhere, worst first: pixels off and id.
    pub off: Vec<(f32, String)>,
}

/// A binding and where each keyform goes in Ayatsuri's order (the first
/// parameter changes fastest, as in Cubism).
fn binding_of(
    bindings: &[extract::Binding],
    forms: &[Form],
    params: &[String],
) -> Result<(Binding, Vec<usize>), String> {
    let mut axes = Vec::new();
    for b in bindings {
        let p = params
            .iter()
            .position(|q| *q == b.param)
            .ok_or(format!("unknown parameter {}", b.param))?;
        axes.push(BindingAxis { parameter: p as u32, keys: b.keys.clone() });
    }
    let lens: Vec<usize> = axes.iter().map(|a| a.keys.len().max(1)).collect();
    let slots = forms
        .iter()
        .map(|f| {
            let mut idx = 0;
            let mut stride = 1;
            for (i, c) in f.coords.iter().enumerate() {
                idx += c * stride;
                stride *= lens.get(i).copied().unwrap_or(1);
            }
            idx
        })
        .collect();
    Ok((Binding { axes }, slots))
}

pub fn build(ex: Extract, name: &str) -> Result<Cmo3, ImportError> {
    let (cw, ch) = ex.canvas;

    // 1. one padded layer per art mesh, named by its unique id, so the
    //    importer packs the art and cuts the parts
    let mut layers_in = Vec::with_capacity(ex.meshes.len());
    for m in &ex.meshes {
        let img =
            image::load_from_memory(&m.png).map_err(|e| err(format!("{}: {e}", m.id)))?.to_rgba8();
        let (rx, ry) = (
            m.rect.first().copied().unwrap_or(0) as f32,
            m.rect.get(1).copied().unwrap_or(0) as f32,
        );
        let (rw, rh) = (img.width() as f32, img.height() as f32);
        // a Cubism mesh may reach well past its art: pad each side by how
        // far this mesh goes beyond the layer, plus the margin
        let (mut lo_x, mut lo_y, mut hi_x, mut hi_y) = (rx, ry, rx + rw, ry + rh);
        for p in m.points.as_chunks::<2>().0 {
            lo_x = lo_x.min(p[0]);
            lo_y = lo_y.min(p[1]);
            hi_x = hi_x.max(p[0]);
            hi_y = hi_y.max(p[1]);
        }
        let side = |reach: f32| reach.clamp(0.0, 16384.0).ceil() as u32 + PAD;
        let (left, top) = (side(rx - lo_x), side(ry - lo_y));
        let (right, bottom) = (side(hi_x - (rx + rw)), side(hi_y - (ry + rh)));
        let (w, h) = (img.width() + left + right, img.height() + top + bottom);
        if (w as u64) * (h as u64) > 64 << 20 {
            return Err(err(format!("{}: layer too large", m.id)));
        }
        let mut padded = image::RgbaImage::new(w, h);
        image::imageops::replace(&mut padded, &img, left as i64, top as i64);
        // an all but invisible pixel in each corner keeps the trim from
        // cutting the padding away
        for (x, y) in [(0, 0), (w - 1, 0), (0, h - 1), (w - 1, h - 1)] {
            padded.put_pixel(x, y, image::Rgba([0, 0, 0, 1]));
        }
        layers_in.push(Layer {
            name: m.id.clone(),
            groups: m.parts.clone(),
            x: rx as i32 - left as i32,
            y: ry as i32 - top as i32,
            width: w,
            height: h,
            rgba: padded.into_raw(),
            visible: true,
            opacity: 1.0,
            clip_to_below: false,
        });
    }
    let document = Document { width: cw, height: ch, layers: layers_in };
    let imported = layers::build(&document)?;
    let mut model: Model = imported.model;
    let c = model.canvas;
    let ppu = c.pixels_per_unit.max(1e-6);
    let to = |space: Space, x: f32, y: f32| match space {
        Space::Root => ((x - c.origin_x) / ppu, (y - c.origin_y) / ppu),
        Space::Warp => (x, y),
        Space::Rotation | Space::RotationInWarp => (x / ppu, y / ppu),
    };
    let convert = |space: Space, pts: &[f32]| -> Vec<f32> {
        pts.as_chunks::<2>()
            .0
            .iter()
            .flat_map(|p| {
                let (x, y) = to(space, p[0], p[1]);
                [x, y]
            })
            .collect()
    };

    // 2. parameters and their folders
    let param_ids: Vec<String> = ex.params.iter().map(|p| p.id.clone()).collect();
    let mut param_names = Vec::with_capacity(ex.params.len());
    let mut folders: Vec<(i64, String, Vec<u32>)> = Vec::new();
    for (i, p) in ex.params.iter().enumerate() {
        model.parameters.push(Parameter {
            id: p.id.clone(),
            min: p.min,
            max: p.max,
            default: p.default,
            repeat: p.repeat,
            decimals: p.decimals,
        });
        param_names.push(p.name.clone());
        if p.folder.is_empty() {
            continue;
        }
        match folders.iter_mut().find(|f| f.1 == p.folder) {
            Some(f) => f.2.push(i as u32),
            None => folders.push((p.folder_rank, p.folder.clone(), vec![i as u32])),
        }
    }
    folders.sort_by_key(|f| f.0);
    let param_groups: Vec<(String, Vec<u32>)> =
        folders.into_iter().map(|(_, n, v)| (n, v)).collect();

    // 3. deformers, parents first, each keyform in its parent's space
    let uuids: Vec<&str> = ex.deformers.iter().map(|d| d.uuid.as_str()).collect();
    let mut kinds: Vec<Space> = Vec::with_capacity(ex.deformers.len());
    let parent_of = |parent: Option<&str>, kinds: &[Space]| -> (Option<u32>, Space) {
        match parent.and_then(|u| uuids.iter().position(|x| *x == u)) {
            Some(i) => (Some(i as u32), kinds.get(i).copied().unwrap_or(Space::Root)),
            None => (None, Space::Root),
        }
    };
    for d in &ex.deformers {
        let (parent, space) = parent_of(d.parent.as_deref(), &kinds);
        if parent.is_some_and(|p| p as usize >= kinds.len()) {
            return Err(err(format!("{}: parent after child", d.id)));
        }
        let (binding, slots) = binding_of(&d.bindings, &d.forms, &param_ids).map_err(err)?;
        let n = slots.iter().max().map_or(1, |m| m + 1);
        if n > 1 << 16 {
            return Err(err(format!("{}: too many keyforms", d.id)));
        }
        let mut opacities = vec![1.0; n];
        for (f, &s) in d.forms.iter().zip(&slots) {
            if let Some(o) = opacities.get_mut(s) {
                *o = f.opacity;
            }
        }
        let kind = match d.kind {
            extract::DeformerKind::Warp { cols, rows } => {
                let per = ((rows + 1) * (cols + 1) * 2) as usize;
                let mut positions = vec![0.0; per * n];
                for (f, &s) in d.forms.iter().zip(&slots) {
                    let pts = convert(space, &f.positions);
                    if pts.len() != per {
                        return Err(err(format!(
                            "{}: keyform has {} values, grid wants {per}",
                            d.id,
                            pts.len()
                        )));
                    }
                    if let Some(dst) = positions.get_mut(s * per..(s + 1) * per) {
                        dst.copy_from_slice(&pts);
                    }
                }
                kinds.push(Space::Warp);
                DeformerKind::Warp(WarpData { rows, cols, quad: true, opacities, positions })
            }
            extract::DeformerKind::Rotation => {
                let mut r = RotationData {
                    base_angle: 0.0,
                    opacities,
                    angles: vec![0.0; n],
                    origins_x: vec![0.0; n],
                    origins_y: vec![0.0; n],
                    scales: vec![1.0; n],
                    reflect_x: vec![false; n],
                    reflect_y: vec![false; n],
                };
                for (f, &s) in d.forms.iter().zip(&slots) {
                    let (ox, oy) = to(space, f.origin[0], f.origin[1]);
                    let set = |v: &mut Vec<f32>, x: f32| {
                        if let Some(slot) = v.get_mut(s) {
                            *slot = x;
                        }
                    };
                    set(&mut r.angles, f.angle);
                    set(&mut r.origins_x, ox);
                    set(&mut r.origins_y, oy);
                    set(&mut r.scales, f.scale);
                    if let Some(slot) = r.reflect_x.get_mut(s) {
                        *slot = f.reflect[0];
                    }
                    if let Some(slot) = r.reflect_y.get_mut(s) {
                        *slot = f.reflect[1];
                    }
                }
                kinds.push(match space {
                    Space::Warp => Space::RotationInWarp,
                    _ => Space::Rotation,
                });
                DeformerKind::Rotation(r)
            }
        };
        model.deformers.push(Deformer {
            id: d.id.clone(),
            parent_part: None,
            parent_deformer: parent,
            visible: true,
            enabled: true,
            binding,
            kind,
        });
    }

    // 4. meshes: uvs from the layer the importer packed, then every keyform
    let index_of = |id: &str| imported.mesh_names.iter().position(|n| n == id);
    let mut names = imported.mesh_names.clone();
    for m in &ex.meshes {
        let k = index_of(&m.id).ok_or_else(|| err(format!("{}: lost in import", m.id)))?;
        let (parent, space) = parent_of(m.deformer.as_deref(), &kinds);
        let (binding, slots) = binding_of(&m.bindings, &m.forms, &param_ids).map_err(err)?;
        let am = model.art_meshes.get_mut(k).ok_or_else(|| err("mesh index"))?;
        // the importer's quad maps model units to atlas uvs; read each
        // point's uv off it at the mesh's resting canvas position
        let q = |v: &[f32], i: usize| v.get(i).copied().unwrap_or(0.0);
        let (p0, p2) = (
            (q(&am.positions, 0), q(&am.positions, 1)),
            (q(&am.positions, 4), q(&am.positions, 5)),
        );
        let (t0, t2) = ((q(&am.uvs, 0), q(&am.uvs, 1)), (q(&am.uvs, 4), q(&am.uvs, 5)));
        let (dx, dy) = (
            (p2.0 - p0.0).abs().max(1e-6).copysign(p2.0 - p0.0),
            (p2.1 - p0.1).abs().max(1e-6).copysign(p2.1 - p0.1),
        );
        let mut uvs = Vec::with_capacity(m.points.len());
        for p in m.points.as_chunks::<2>().0 {
            let (x, y) = to(Space::Root, p[0], p[1]);
            uvs.push(t0.0 + (x - p0.0) / dx * (t2.0 - t0.0));
            uvs.push(t0.1 + (y - p0.1) / dy * (t2.1 - t0.1));
        }
        let per = m.points.len();
        if per == 0 || per / 2 > u16::MAX as usize {
            return Err(err(format!("{}: {} points", m.id, per / 2)));
        }
        let n = slots.iter().max().map_or(1, |s| s + 1);
        if n > 1 << 16 {
            return Err(err(format!("{}: too many keyforms", m.id)));
        }
        let mut positions = vec![0.0; per * n];
        let mut opacities = vec![1.0; n];
        let mut draw_orders = vec![500.0; n];
        for (f, &s) in m.forms.iter().zip(&slots) {
            let pts = convert(space, &f.positions);
            if pts.len() != per {
                return Err(err(format!(
                    "{}: keyform has {} values, mesh has {per}",
                    m.id,
                    pts.len()
                )));
            }
            if let Some(dst) = positions.get_mut(s * per..(s + 1) * per) {
                dst.copy_from_slice(&pts);
            }
            if let Some(o) = opacities.get_mut(s) {
                *o = f.opacity;
            }
            if let Some(o) = draw_orders.get_mut(s) {
                *o = f.draw_order;
            }
        }
        if m.forms.is_empty() {
            positions = convert(space, &m.points);
        }
        let count = per / 2;
        if m.triangles.iter().any(|&t| t as usize >= count) {
            return Err(err(format!("{}: a triangle points past its {count} points", m.id)));
        }
        am.vertex_count = count as u32;
        am.positions = positions;
        am.uvs = uvs;
        am.indices = m.triangles.clone();
        am.binding = binding;
        am.opacities = opacities;
        am.draw_orders = draw_orders;
        am.parent_deformer = parent;
        am.id = m.id.clone();
        // Cubism's eye toggle is a view setting; hidden meshes still export
        am.visible = true;
        am.masks = m
            .clip
            .iter()
            .filter_map(|&i| ex.meshes.get(i))
            .filter_map(|other| index_of(&other.id))
            .map(|i| i as u32)
            .collect();
        if let Some(slot) = names.get_mut(k) {
            *slot = m.name.clone();
        }
    }

    // draw order: Cubism sorts every mesh together, by draw order and then
    // by the Parts palette; the extract is in palette order bottom first,
    // which is the mesh order here, so the runtime's sort breaks ties the
    // same way

    // 5. physics, group for group; Cubism Editor keeps no gravity or wind
    //    in the project, so physics3's defaults apply
    let range = |r: [f32; 3]| PhysicsRange { min: r[0], default: r[1], max: r[2] };
    let groups: Vec<PhysicsGroup> = ex
        .physics
        .iter()
        .map(|g| PhysicsGroup {
            id: g.id.clone(),
            name: g.name.clone(),
            inputs: g
                .inputs
                .iter()
                .map(|i| PhysicsInput {
                    param: i.param.clone(),
                    weight: i.weight,
                    kind: i.kind,
                    reflect: i.reflect,
                })
                .collect(),
            outputs: g
                .outputs
                .iter()
                .map(|o| PhysicsOutput {
                    param: o.param.clone(),
                    vertex: o.vertex,
                    scale: o.scale,
                    weight: o.weight,
                    kind: o.kind,
                    reflect: o.reflect,
                })
                .collect(),
            nodes: g.nodes.clone(),
            norm_position: range(g.position),
            norm_angle: range(g.angle),
        })
        .collect();
    if !groups.is_empty() {
        model.physics = Some(PhysicsRig {
            fps: ex.physics_fps,
            gravity_x: 0.0,
            gravity_y: -1.0,
            wind_x: 0.0,
            wind_y: 0.0,
            groups,
        });
    }

    // 6. the check: at default parameter values every mesh lands where the
    //    project says it rests
    let mut puppet = ayatsuri_core::Puppet::new(model.clone());
    puppet.update();
    let drawn: std::collections::HashMap<usize, Vec<f32>> =
        puppet.drawables().map(|d| (d.mesh, d.positions.to_vec())).collect();
    let mut within = 0usize;
    let mut checked = 0usize;
    let mut worst: Vec<(f32, String)> = Vec::new();
    for m in &ex.meshes {
        let Some(k) = index_of(&m.id) else { continue };
        let Some(got) = drawn.get(&k) else { continue };
        let want = convert(Space::Root, &m.points);
        let e = got.iter().zip(&want).map(|(a, b)| (a - b).abs()).fold(0.0, f32::max) * ppu;
        checked += 1;
        if e <= 1.0 {
            within += 1;
        } else {
            worst.push((e, m.id.clone()));
        }
    }
    worst.sort_by(|a, b| b.0.total_cmp(&a.0));
    if !worst.is_empty() {
        let shown: Vec<String> =
            worst.iter().take(5).map(|(e, id)| format!("{id} {e:.1}px")).collect();
        log::warn!(
            "cmo3 {name}: {}/{checked} meshes off their rest by over a pixel: {}",
            worst.len(),
            shown.join(", ")
        );
    }

    Ok(Cmo3 {
        imported: Imported {
            model,
            textures: imported.textures,
            part_names: imported.part_names,
            mesh_names: names,
            skipped: imported.skipped,
            scaled: imported.scaled,
        },
        param_names,
        param_groups,
        within: (within, checked),
        off: worst,
    })
}
