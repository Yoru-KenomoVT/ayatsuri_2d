// Keyform-level edits that work on a whole parameter axis at once: the
// Parameter palette's Reflect / Adjust / Change, Cubism's Motion Mirroring
// and Automatic Generation of Four Corner Forms, and copy/paste of a single
// form. Every function leaves the model untouched when it returns false or
// None.

use ayatsuri_format::model::{Binding, DeformerKind, Model};

use crate::edit::{KeySrc, Target, axis_lens, product, remap, with_object};

/// A keyform's coordinates on each axis, first axis fastest.
fn coords_of(lens: &[usize], mut k: usize) -> Vec<usize> {
    lens.iter()
        .map(|&len| {
            let len = len.max(1);
            let c = k % len;
            k /= len;
            c
        })
        .collect()
}

fn index_of(lens: &[usize], coords: &[usize]) -> usize {
    let mut idx = 0usize;
    let mut stride = 1usize;
    for (i, &len) in lens.iter().enumerate() {
        idx += coords.get(i).copied().unwrap_or(0).min(len.saturating_sub(1)) * stride;
        stride *= len.max(1);
    }
    idx
}

fn nearest_key(keys: &[f32], v: f32) -> Option<usize> {
    keys.iter()
        .enumerate()
        .min_by(|(_, a), (_, b)| (*a - v).abs().total_cmp(&(*b - v).abs()))
        .map(|(i, _)| i)
}

fn binding_of(model: &Model, target: Target) -> Option<&Binding> {
    Some(match target {
        Target::ArtMesh(i) => &model.art_meshes.get(i)?.binding,
        Target::Deformer(i) => &model.deformers.get(i)?.binding,
        Target::Part(i) => &model.parts.get(i)?.binding,
    })
}

/// What a float channel holds, which decides how mirroring treats it.
#[derive(Clone, Copy)]
enum Kind {
    /// xy pairs; warps carry their lattice size so mirroring can swap
    /// columns and rows.
    Points {
        grid: Option<(usize, usize)>,
    },
    Angle,
    OriginX,
    OriginY,
    /// Opacity, draw order, scale: carried over unchanged.
    Plain,
}

struct Chan<'a> {
    data: &'a mut Vec<f32>,
    elems: usize,
    kind: Kind,
}

fn channels(model: &mut Model, target: Target) -> Option<(Binding, Vec<Chan<'_>>)> {
    match target {
        Target::ArtMesh(i) => {
            let m = model.art_meshes.get_mut(i)?;
            let n = m.vertex_count as usize * 2;
            Some((
                m.binding.clone(),
                vec![
                    Chan { data: &mut m.positions, elems: n, kind: Kind::Points { grid: None } },
                    Chan { data: &mut m.opacities, elems: 1, kind: Kind::Plain },
                    Chan { data: &mut m.draw_orders, elems: 1, kind: Kind::Plain },
                ],
            ))
        }
        Target::Deformer(i) => {
            let d = model.deformers.get_mut(i)?;
            let binding = d.binding.clone();
            let chans = match &mut d.kind {
                DeformerKind::Warp(w) => {
                    let grid = Some((w.cols as usize + 1, w.rows as usize + 1));
                    let n = w.grid_points() * 2;
                    vec![
                        Chan { data: &mut w.positions, elems: n, kind: Kind::Points { grid } },
                        Chan { data: &mut w.opacities, elems: 1, kind: Kind::Plain },
                    ]
                }
                DeformerKind::Rotation(r) => vec![
                    Chan { data: &mut r.angles, elems: 1, kind: Kind::Angle },
                    Chan { data: &mut r.origins_x, elems: 1, kind: Kind::OriginX },
                    Chan { data: &mut r.origins_y, elems: 1, kind: Kind::OriginY },
                    Chan { data: &mut r.scales, elems: 1, kind: Kind::Plain },
                    Chan { data: &mut r.opacities, elems: 1, kind: Kind::Plain },
                ],
            };
            Some((binding, chans))
        }
        Target::Part(i) => {
            let p = model.parts.get_mut(i)?;
            Some((
                p.binding.clone(),
                vec![Chan { data: &mut p.draw_orders, elems: 1, kind: Kind::Plain }],
            ))
        }
    }
}

/// The keys `target` has on `param`, if it is bound on it.
pub fn keys_on(model: &Model, target: Target, param: usize) -> Option<Vec<f32>> {
    binding_of(model, target)?
        .axes
        .iter()
        .find(|a| a.parameter as usize == param)
        .map(|a| a.keys.clone())
}

/// Every keyform at the key of `param` that its current value sits on, across
/// all the object's other parameters: "Bald = 1" whatever the hair does.
/// None when the object is not bound on `param` or the value is between
/// two of its keys.
pub fn keyforms_on_key(
    model: &Model,
    target: Target,
    param: usize,
    values: &[f32],
) -> Option<Vec<usize>> {
    let binding = binding_of(model, target)?;
    let axis_at = binding.axes.iter().position(|a| a.parameter as usize == param)?;
    let axis = binding.axes.get(axis_at)?;
    let p = model.parameters.get(param)?;
    let v = values.get(param).copied().unwrap_or(p.default);
    let k = nearest_key(&axis.keys, v)?;
    let tol = ((p.max - p.min).abs() * 0.01).max(1e-4);
    if (axis.keys.get(k)? - v).abs() > tol {
        return None;
    }
    let lens = axis_lens(binding);
    let total = product(&lens);
    Some((0..total).filter(|&i| coords_of(&lens, i).get(axis_at) == Some(&k)).collect())
}

/// The keyform the current parameter values sit on, when every bound axis
/// is on a key (within a hundredth of its range). Copy and paste need one.
pub fn keyform_at(model: &Model, target: Target, values: &[f32]) -> Option<usize> {
    let binding = binding_of(model, target)?;
    let lens = axis_lens(binding);
    let mut coords = Vec::with_capacity(lens.len());
    for axis in &binding.axes {
        let p = model.parameters.get(axis.parameter as usize)?;
        let v = values.get(axis.parameter as usize).copied().unwrap_or(p.default);
        let k = nearest_key(&axis.keys, v)?;
        let tol = ((p.max - p.min).abs() * 0.01).max(1e-4);
        if (axis.keys.get(k)? - v).abs() > tol {
            return None;
        }
        coords.push(k);
    }
    Some(index_of(&lens, &coords))
}

/// One keyform's worth of every float channel, in channel order.
#[derive(Clone, Debug, PartialEq)]
pub struct Form {
    pub target_kind: u8,
    pub data: Vec<Vec<f32>>,
}

fn kind_tag(target: Target, model: &Model) -> u8 {
    match target {
        Target::ArtMesh(_) => 0,
        Target::Deformer(i) => match model.deformers.get(i).map(|d| &d.kind) {
            Some(DeformerKind::Warp(_)) => 1,
            _ => 2,
        },
        Target::Part(_) => 3,
    }
}

/// Copy Form: the shape of one keyform.
pub fn copy_form(model: &mut Model, target: Target, keyform: usize) -> Option<Form> {
    let tag = kind_tag(target, model);
    let (_, chans) = channels(model, target)?;
    let mut data = Vec::with_capacity(chans.len());
    for c in &chans {
        data.push(c.data.get(keyform * c.elems..(keyform + 1) * c.elems)?.to_vec());
    }
    Some(Form { target_kind: tag, data })
}

/// Paste Form: write a copied shape into one keyform. The shapes must match
/// (same object kind, same vertex or grid size), or nothing changes.
pub fn paste_form(model: &mut Model, target: Target, keyform: usize, form: &Form) -> bool {
    if kind_tag(target, model) != form.target_kind {
        return false;
    }
    let Some((_, mut chans)) = channels(model, target) else { return false };
    if chans.len() != form.data.len()
        || chans
            .iter()
            .zip(&form.data)
            .any(|(c, d)| d.len() != c.elems || c.data.len() < (keyform + 1) * c.elems)
    {
        return false;
    }
    for (c, d) in chans.iter_mut().zip(&form.data) {
        if let Some(dst) = c.data.get_mut(keyform * c.elems..(keyform + 1) * c.elems) {
            dst.copy_from_slice(d);
        }
    }
    true
}

/// The palette's Reflect: key values flip across the parameter range, so a
/// mouth keyed open at 0 and shut at 1 becomes shut at 0 and open at 1.
pub fn reflect_keys(model: &mut Model, target: Target, param: usize) -> bool {
    let Some((min, max)) = model.parameters.get(param).map(|p| (p.min, p.max)) else {
        return false;
    };
    with_object(model, target, |binding, mut channels| {
        let lens = axis_lens(binding);
        let Some(axis_at) = binding.axes.iter().position(|a| a.parameter as usize == param) else {
            return false;
        };
        let Some(axis) = binding.axes.get_mut(axis_at) else { return false };
        let n = axis.keys.len();
        let map: Vec<KeySrc> = (0..n).map(|i| KeySrc::Copy(n - 1 - i)).collect();
        axis.keys = axis.keys.iter().rev().map(|k| min + max - k).collect();
        for ch in &mut channels {
            remap(&lens, axis_at, &map, ch);
        }
        true
    })
    .unwrap_or(false)
}

/// The palette's Adjust: move the key nearest `from` to `to`, keeping its
/// form. Refused when it would cross a neighbouring key.
pub fn adjust_key(model: &mut Model, target: Target, param: usize, from: f32, to: f32) -> bool {
    let Some(binding) = (match target {
        Target::ArtMesh(i) => model.art_meshes.get_mut(i).map(|m| &mut m.binding),
        Target::Deformer(i) => model.deformers.get_mut(i).map(|d| &mut d.binding),
        Target::Part(i) => model.parts.get_mut(i).map(|p| &mut p.binding),
    }) else {
        return false;
    };
    let Some(axis) = binding.axes.iter_mut().find(|a| a.parameter as usize == param) else {
        return false;
    };
    let Some(k) = nearest_key(&axis.keys, from) else { return false };
    let lo = k.checked_sub(1).and_then(|i| axis.keys.get(i)).copied();
    let hi = axis.keys.get(k + 1).copied();
    if lo.is_some_and(|l| to <= l + 1e-5) || hi.is_some_and(|h| to >= h - 1e-5) {
        return false;
    }
    match axis.keys.get_mut(k) {
        Some(slot) => {
            *slot = to;
            true
        }
        None => false,
    }
}

/// The palette's Change: move a binding from one parameter to another the
/// object is not bound on yet. Keys outside the new range are clamped
/// proportionally into it.
pub fn change_parameter(model: &mut Model, target: Target, from: usize, to: usize) -> bool {
    let (Some(a), Some(b)) =
        (model.parameters.get(from).cloned(), model.parameters.get(to).cloned())
    else {
        return false;
    };
    let Some(binding) = (match target {
        Target::ArtMesh(i) => model.art_meshes.get_mut(i).map(|m| &mut m.binding),
        Target::Deformer(i) => model.deformers.get_mut(i).map(|d| &mut d.binding),
        Target::Part(i) => model.parts.get_mut(i).map(|p| &mut p.binding),
    }) else {
        return false;
    };
    if from == to || binding.axes.iter().any(|x| x.parameter as usize == to) {
        return false;
    }
    let Some(axis) = binding.axes.iter_mut().find(|x| x.parameter as usize == from) else {
        return false;
    };
    axis.parameter = to as u32;
    let inside = axis.keys.iter().all(|k| *k >= b.min - 1e-5 && *k <= b.max + 1e-5);
    if !inside {
        let span = (a.max - a.min).abs().max(1e-6);
        for k in &mut axis.keys {
            *k = b.min + (*k - a.min) / span * (b.max - b.min);
        }
    }
    true
}

/// Where Automatic Generation of Four Corner Forms takes its centre form.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CornerRef {
    Default,
    Centre,
}

/// Automatic Generation of Four Corner Forms on the parameters `pa` (left
/// to right) and `pb` (bottom to top): every corner keyform becomes
/// "its `pa` side + its `pb` side - the centre", for each corner whose
/// overwrite flag is on, in Cubism's order [top left, top right, bottom
/// left, bottom right]. Other axes the object is bound on are kept apart:
/// each of their keys gets its own corners.
pub fn four_corners(
    model: &mut Model,
    target: Target,
    pa: usize,
    pb: usize,
    reference: CornerRef,
    overwrite: [bool; 4],
) -> bool {
    let centre_value = |p: usize| {
        model.parameters.get(p).map(|q| match reference {
            CornerRef::Default => q.default,
            CornerRef::Centre => (q.min + q.max) * 0.5,
        })
    };
    let (Some(va), Some(vb)) = (centre_value(pa), centre_value(pb)) else { return false };
    let Some((binding, mut chans)) = channels(model, target) else { return false };
    let (Some(xa), Some(xb)) = (
        binding.axes.iter().position(|a| a.parameter as usize == pa),
        binding.axes.iter().position(|a| a.parameter as usize == pb),
    ) else {
        return false;
    };
    if xa == xb {
        return false;
    }
    let keys = |x: usize| binding.axes.get(x).map(|a| a.keys.clone()).unwrap_or_default();
    let (ka, kb) = (keys(xa), keys(xb));
    let (Some(ca), Some(cb)) = (nearest_key(&ka, va), nearest_key(&kb, vb)) else { return false };
    if ka.len() < 2 || kb.len() < 2 {
        return false;
    }
    let lens = axis_lens(&binding);
    let total = product(&lens);
    let mut wrote = false;
    for c in &mut chans {
        if c.data.len() < total * c.elems {
            continue;
        }
        let old = c.data.clone();
        for k in 0..total {
            let mut co = coords_of(&lens, k);
            let (ia, ib) = (co.get(xa).copied().unwrap_or(0), co.get(xb).copied().unwrap_or(0));
            if ia == ca || ib == cb {
                continue;
            }
            // top means the higher pb key, the way Angle Y reads
            let slot = match (ia < ca, ib > cb) {
                (true, true) => 0,
                (false, true) => 1,
                (true, false) => 2,
                (false, false) => 3,
            };
            if !overwrite.get(slot).copied().unwrap_or(false) {
                continue;
            }
            if let Some(x) = co.get_mut(xb) {
                *x = cb;
            }
            let side_a = index_of(&lens, &co);
            if let Some(x) = co.get_mut(xa) {
                *x = ca;
            }
            let centre = index_of(&lens, &co);
            if let Some(x) = co.get_mut(xb) {
                *x = ib;
            }
            let side_b = index_of(&lens, &co);
            for e in 0..c.elems {
                let g = |f: usize| old.get(f * c.elems + e).copied().unwrap_or(0.0);
                if let Some(dst) = c.data.get_mut(k * c.elems + e) {
                    *dst = match c.kind {
                        Kind::Plain => g(side_a),
                        _ => g(side_a) + g(side_b) - g(centre),
                    };
                }
            }
            wrote = true;
        }
    }
    wrote
}

/// Each point's mirror twin on a mesh: the point nearest its reflection
/// across the middle of the mesh's bounding box (left and right for
/// `horizontal`, top and bottom for `vertical`), as Cubism pairs them. A
/// symmetric mesh pairs exactly; a lopsided one with the closest it has.
fn mirror_twins(rest: &[[f32; 2]], horizontal: bool, vertical: bool) -> Vec<usize> {
    let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
    for &[x, y] in rest {
        (x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x), y1.max(y));
    }
    let (mx, my) = ((x0 + x1) / 2.0, (y0 + y1) / 2.0);
    rest.iter()
        .enumerate()
        .map(|(i, &[x, y])| {
            let wx = if horizontal { 2.0 * mx - x } else { x };
            let wy = if vertical { 2.0 * my - y } else { y };
            rest.iter()
                .enumerate()
                .map(|(j, &[qx, qy])| (j, (qx - wx).powi(2) + (qy - wy).powi(2)))
                .min_by(|a, b| a.1.total_cmp(&b.1))
                .map_or(i, |(j, _)| j)
        })
        .collect()
}

/// Motion Mirroring: the key nearest `from` is mirrored across the
/// parameter default and written into the opposite key. Movement away from
/// the default key is what flips, so a face turned left becomes the same
/// face turned right; warp lattices also swap columns (or rows) around the
/// deformer's centre, and rotation deformers flip their angle and origin.
/// Returns the key value written. Fails (None) when the source is the
/// default key or no opposite key exists, like Cubism.
pub fn mirror_motion(
    model: &mut Model,
    target: Target,
    param: usize,
    from: f32,
    horizontal: bool,
    vertical: bool,
) -> Option<f32> {
    let default = model.parameters.get(param)?.default;
    let (binding, mut chans) = channels(model, target)?;
    let axis_at = binding.axes.iter().position(|a| a.parameter as usize == param)?;
    let keys = &binding.axes.get(axis_at)?.keys;
    let src = nearest_key(keys, from)?;
    let src_v = *keys.get(src)?;
    let want = 2.0 * default - src_v;
    let dst = nearest_key(keys, want)?;
    let centre = nearest_key(keys, default)?;
    let dst_v = *keys.get(dst)?;
    let span = keys.iter().fold(0.0f32, |m, k| m.max((k - default).abs())).max(1e-6);
    if dst == src || src == centre || (dst_v - want).abs() > span * 0.01 {
        return None;
    }
    let lens = axis_lens(&binding);
    let total = product(&lens);
    // each keyform on the destination key, with its source and centre
    // partners (same coordinates on every other axis)
    let triples: Vec<(usize, usize, usize)> = (0..total)
        .filter_map(|k| {
            let mut c = coords_of(&lens, k);
            if c.get(axis_at).copied() != Some(dst) {
                return None;
            }
            *c.get_mut(axis_at)? = src;
            let s = index_of(&lens, &c);
            *c.get_mut(axis_at)? = centre;
            Some((k, s, index_of(&lens, &c)))
        })
        .collect();
    let (sx, sy) = (if horizontal { -1.0 } else { 1.0 }, if vertical { -1.0 } else { 1.0 });
    let flips_angle = horizontal != vertical;
    for c in &mut chans {
        if c.data.len() < total * c.elems {
            continue;
        }
        let old = c.data.clone();
        let g = |k: usize, e: usize| old.get(k * c.elems + e).copied().unwrap_or(0.0);
        // a mesh's twins, per centre keyform: the point opposite each one
        let mut twins: std::collections::HashMap<usize, Vec<usize>> = Default::default();
        for &(d, s, ce) in &triples {
            match c.kind {
                Kind::Points { grid } => {
                    let n = c.elems / 2;
                    if grid.is_none() && !twins.contains_key(&ce) {
                        let rest: Vec<[f32; 2]> =
                            (0..n).map(|p| [g(ce, p * 2), g(ce, p * 2 + 1)]).collect();
                        twins.insert(ce, mirror_twins(&rest, horizontal, vertical));
                    }
                    for p in 0..n {
                        // the point's mirror twin: the opposite column/row
                        // on a lattice, the point across the centre on a mesh
                        let q = match grid {
                            Some((cols, rows)) if cols > 0 => {
                                let (r, col) = (p / cols, p % cols);
                                let col = if horizontal { cols - 1 - col } else { col };
                                let r = if vertical { rows.saturating_sub(1) - r } else { r };
                                r * cols + col
                            }
                            _ => twins.get(&ce).and_then(|t| t.get(p).copied()).unwrap_or(p),
                        };
                        for (e, sign) in [(0usize, sx), (1, sy)] {
                            let v = g(ce, p * 2 + e) + sign * (g(s, q * 2 + e) - g(ce, q * 2 + e));
                            if let Some(slot) = c.data.get_mut(d * c.elems + p * 2 + e) {
                                *slot = v;
                            }
                        }
                    }
                }
                Kind::Angle => {
                    let (cv, sv) = (g(ce, 0), g(s, 0));
                    if let Some(slot) = c.data.get_mut(d) {
                        *slot = if flips_angle { cv - (sv - cv) } else { sv };
                    }
                }
                Kind::OriginX | Kind::OriginY => {
                    let sign = if matches!(c.kind, Kind::OriginX) { sx } else { sy };
                    let (cv, sv) = (g(ce, 0), g(s, 0));
                    if let Some(slot) = c.data.get_mut(d) {
                        *slot = cv + sign * (sv - cv);
                    }
                }
                Kind::Plain => {
                    for e in 0..c.elems {
                        let v = g(s, e);
                        if let Some(slot) = c.data.get_mut(d * c.elems + e) {
                            *slot = v;
                        }
                    }
                }
            }
        }
    }
    Some(dst_v)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ayatsuri_format::model::{ArtMesh, BindingAxis, Canvas, Parameter};

    fn param(id: &str, min: f32, default: f32, max: f32) -> Parameter {
        Parameter { id: id.into(), min, max, default, repeat: false, decimals: 3 }
    }

    // one triangle, bound on X (-1, 0, 1) and Y (-1, 0, 1)
    fn model() -> Model {
        let base = [0.0f32, 0.0, 1.0, 0.0, 0.0, 1.0];
        let mut positions = Vec::new();
        for y in [-1.0f32, 0.0, 1.0] {
            for x in [-1.0f32, 0.0, 1.0] {
                for p in base.chunks(2) {
                    positions.push(p[0] + x * 0.5);
                    positions.push(p[1] + y * 0.25);
                }
            }
        }
        Model {
            canvas: Canvas { pixels_per_unit: 1.0, ..Canvas::default() },
            parameters: vec![param("X", -1.0, 0.0, 1.0), param("Y", -1.0, 0.0, 1.0)],
            parts: vec![],
            deformers: vec![],
            art_meshes: vec![ArtMesh {
                id: "m".into(),
                parent_part: None,
                parent_deformer: None,
                visible: true,
                enabled: true,
                texture: 0,
                flags: 0,
                vertex_count: 3,
                uvs: base.to_vec(),
                indices: vec![0, 1, 2],
                masks: vec![],
                binding: Binding {
                    axes: vec![
                        BindingAxis { parameter: 0, keys: vec![-1.0, 0.0, 1.0] },
                        BindingAxis { parameter: 1, keys: vec![-1.0, 0.0, 1.0] },
                    ],
                },
                opacities: vec![1.0; 9],
                draw_orders: vec![500.0; 9],
                positions,
            }],
            ..Model::new()
        }
    }

    fn form_x(m: &Model, k: usize) -> f32 {
        m.art_meshes[0].positions[k * 6]
    }

    #[test]
    fn mirror_flips_movement() {
        let mut m = model();
        // make the right side (X=1, Y=0: keyform 5) unlike the left
        m.art_meshes[0].positions[5 * 6] = 0.9;
        let wrote = mirror_motion(&mut m, Target::ArtMesh(0), 0, 1.0, true, false);
        assert_eq!(wrote, Some(-1.0));
        // left (keyform 3): point 0's movement lands on its twin across the
        // middle (point 1, at x = 1), flipped: 1.0 - 0.9
        let p1_x = m.art_meshes[0].positions[3 * 6 + 2];
        assert!((p1_x - 0.1).abs() < 1e-5);
        // the default key cannot be mirrored
        assert_eq!(mirror_motion(&mut m, Target::ArtMesh(0), 0, 0.0, true, false), None);
    }

    #[test]
    fn a_mesh_mirrors_each_point_into_its_twin_across_the_middle() {
        // four points, two on the left (x = -1) and two on the right (x = 1)
        let rest = [-1.0f32, 0.0, -1.0, 1.0, 1.0, 0.0, 1.0, 1.0];
        let mut m = model();
        let mesh = &mut m.art_meshes[0];
        mesh.vertex_count = 4;
        mesh.uvs = rest.to_vec();
        mesh.indices = vec![0, 1, 2, 1, 3, 2];
        mesh.binding.axes = vec![BindingAxis { parameter: 0, keys: vec![-1.0, 0.0, 1.0] }];
        mesh.opacities = vec![1.0; 3];
        mesh.draw_orders = vec![500.0; 3];
        // keyform 2 (X = 1): the left points move 0.5 right, the right ones 0.1
        let mut turned = rest;
        for (i, dx) in [(0, 0.5), (2, 0.5), (4, 0.1), (6, 0.1)] {
            turned[i] += dx;
        }
        mesh.positions = [rest, rest, turned].concat();
        assert_eq!(mirror_motion(&mut m, Target::ArtMesh(0), 0, 1.0, true, false), Some(-1.0));
        let left_key = &m.art_meshes[0].positions[..8];
        // a real mirror: the right points take the left ones' big move,
        // flipped, and the left points the right ones' small one
        let dx = |p: usize| left_key[p * 2] - rest[p * 2];
        assert!((dx(2) + 0.5).abs() < 1e-5 && (dx(3) + 0.5).abs() < 1e-5);
        assert!((dx(0) + 0.1).abs() < 1e-5 && (dx(1) + 0.1).abs() < 1e-5);
    }

    #[test]
    fn corners_add_the_sides() {
        let mut m = model();
        // scramble a corner, then regenerate only the top right
        m.art_meshes[0].positions[8 * 6] = 9.0;
        m.art_meshes[0].positions[6 * 6] = 9.0;
        assert!(four_corners(
            &mut m,
            Target::ArtMesh(0),
            0,
            1,
            CornerRef::Default,
            [false, true, false, false]
        ));
        // top right = X side (0.5) + Y side (0.0) - centre (0.0)
        assert!((form_x(&m, 8) - 0.5).abs() < 1e-5);
        // top left was not ticked
        assert_eq!(form_x(&m, 6), 9.0);
    }

    #[test]
    fn reflect_adjust_change() {
        let mut m = model();
        let before = form_x(&m, 0);
        assert!(reflect_keys(&mut m, Target::ArtMesh(0), 0));
        // the old X=-1 form now sits at X=1
        assert_eq!(form_x(&m, 2), before);
        assert!(adjust_key(&mut m, Target::ArtMesh(0), 0, 1.0, 0.5));
        assert!(!adjust_key(&mut m, Target::ArtMesh(0), 0, 0.5, -2.0));
        assert_eq!(m.art_meshes[0].binding.axes[0].keys, vec![-1.0, 0.0, 0.5]);
        // already bound on Y, so X cannot become Y
        assert!(!change_parameter(&mut m, Target::ArtMesh(0), 0, 1));
    }

    #[test]
    fn copy_paste_form() {
        let mut m = model();
        let f = copy_form(&mut m, Target::ArtMesh(0), 8).unwrap();
        assert!(paste_form(&mut m, Target::ArtMesh(0), 0, &f));
        assert_eq!(form_x(&m, 0), form_x(&m, 8));
        assert_eq!(keyform_at(&m, Target::ArtMesh(0), &[1.0, 1.0]), Some(8));
        assert_eq!(keyform_at(&m, Target::ArtMesh(0), &[0.5, 1.0]), None);
    }
}
