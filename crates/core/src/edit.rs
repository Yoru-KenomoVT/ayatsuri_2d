// The first real editing operations: adding and removing parameter keys on
// an object's binding. Keyform arrays are cartesian products (first axis
// fastest), so every change resamples the product: inserted keys take the
// interpolated shape, so the puppet looks identical until someone moves it.

use ayatsuri_format::model::{Binding, BindingAxis, BlendShape, DeformerKind, Model};

use crate::{blend, blendshape};

/// Which rig object an edit touches.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Target {
    Part(usize),
    Deformer(usize),
    ArtMesh(usize),
}

/// Where each key of a changed axis takes its data from.
#[derive(Clone, Copy)]
pub(crate) enum KeySrc {
    Copy(usize),
    Lerp(usize, usize, f32),
    /// Up to four keys, weighted (extended interpolation's baked keys).
    Mix([(usize, f32); 4]),
}

pub(crate) enum Channel<'a> {
    F32 { elems: usize, data: &'a mut Vec<f32> },
    Bool { data: &'a mut Vec<bool> },
}

pub(crate) fn axis_lens(binding: &Binding) -> Vec<usize> {
    binding.axes.iter().map(|a| a.keys.len().max(1)).collect()
}

pub(crate) fn product(lens: &[usize]) -> usize {
    lens.iter().product::<usize>().max(1)
}

// Walk every keyform of the new product and pull its data from the old one
// through `map` on the changed axis.
pub(crate) fn remap(old_lens: &[usize], axis: usize, map: &[KeySrc], channel: &mut Channel<'_>) {
    let mut new_lens = old_lens.to_vec();
    if let Some(l) = new_lens.get_mut(axis) {
        *l = map.len().max(1);
    }
    let total = product(&new_lens);

    let src_index = |coords: &[usize], axis_coord: usize| -> usize {
        let mut idx = 0usize;
        let mut stride = 1usize;
        for (i, &len) in old_lens.iter().enumerate() {
            let c = if i == axis { axis_coord } else { coords.get(i).copied().unwrap_or(0) };
            idx += c.min(len.saturating_sub(1)) * stride;
            stride *= len;
        }
        idx
    };

    let mut coords = vec![0usize; new_lens.len()];
    match channel {
        Channel::F32 { elems, data } => {
            let elems = *elems;
            let mut out = Vec::with_capacity(total * elems);
            for _ in 0..total {
                let c = coords.get(axis).copied().unwrap_or(0);
                match map.get(c).copied().unwrap_or(KeySrc::Copy(0)) {
                    KeySrc::Copy(sc) => {
                        let at = src_index(&coords, sc) * elems;
                        out.extend(data.get(at..at + elems).unwrap_or(&[]).iter().copied());
                    }
                    KeySrc::Lerp(a, b, t) => {
                        let pa = src_index(&coords, a) * elems;
                        let pb = src_index(&coords, b) * elems;
                        for e in 0..elems {
                            let va = data.get(pa + e).copied().unwrap_or(0.0);
                            let vb = data.get(pb + e).copied().unwrap_or(0.0);
                            out.push(va + (vb - va) * t);
                        }
                    }
                    KeySrc::Mix(ws) => {
                        let at: Vec<(usize, f32)> = ws
                            .iter()
                            .filter(|w| w.1 != 0.0)
                            .map(|&(k, w)| (src_index(&coords, k) * elems, w))
                            .collect();
                        for e in 0..elems {
                            out.push(
                                at.iter()
                                    .map(|&(p, w)| data.get(p + e).copied().unwrap_or(0.0) * w)
                                    .sum(),
                            );
                        }
                    }
                }
                advance(&mut coords, &new_lens);
            }
            **data = out;
        }
        Channel::Bool { data } => {
            let mut out = Vec::with_capacity(total);
            for _ in 0..total {
                let c = coords.get(axis).copied().unwrap_or(0);
                let sc = match map.get(c).copied().unwrap_or(KeySrc::Copy(0)) {
                    KeySrc::Copy(sc) => sc,
                    KeySrc::Lerp(a, b, t) => {
                        if t < 0.5 {
                            a
                        } else {
                            b
                        }
                    }
                    KeySrc::Mix(ws) => {
                        ws.iter().max_by(|x, y| x.1.total_cmp(&y.1)).map_or(0, |w| w.0)
                    }
                };
                out.push(data.get(src_index(&coords, sc)).copied().unwrap_or(false));
                advance(&mut coords, &new_lens);
            }
            **data = out;
        }
    }
}

fn advance(coords: &mut [usize], lens: &[usize]) {
    for (c, &l) in coords.iter_mut().zip(lens) {
        *c += 1;
        if *c < l.max(1) {
            return;
        }
        *c = 0;
    }
}

// Tile data K times for a freshly appended axis (it varies slowest, so the
// whole old product just repeats).
fn tile(channel: &mut Channel<'_>, k: usize) {
    match channel {
        Channel::F32 { data, .. } => {
            let one = data.clone();
            for _ in 1..k {
                data.extend_from_slice(&one);
            }
        }
        Channel::Bool { data } => {
            let one = data.clone();
            for _ in 1..k {
                data.extend_from_slice(&one);
            }
        }
    }
}

pub(crate) fn with_object<R>(
    model: &mut Model,
    target: Target,
    f: impl FnOnce(&mut Binding, Vec<Channel<'_>>) -> R,
) -> Option<R> {
    match target {
        Target::Part(i) => {
            let p = model.parts.get_mut(i)?;
            let mut channels = vec![Channel::F32 { elems: 1, data: &mut p.draw_orders }];
            if let Some(o) = &mut p.offscreen {
                channels.push(Channel::F32 { elems: 1, data: &mut o.opacities });
            }
            Some(f(&mut p.binding, channels))
        }
        Target::ArtMesh(i) => {
            let m = model.art_meshes.get_mut(i)?;
            let elems = (m.vertex_count as usize) * 2;
            let channels = vec![
                Channel::F32 { elems: 1, data: &mut m.opacities },
                Channel::F32 { elems: 1, data: &mut m.draw_orders },
                Channel::F32 { elems, data: &mut m.positions },
            ];
            Some(f(&mut m.binding, channels))
        }
        Target::Deformer(i) => {
            let d = model.deformers.get_mut(i)?;
            match &mut d.kind {
                DeformerKind::Warp(w) => {
                    let elems = w.grid_points() * 2;
                    let channels = vec![
                        Channel::F32 { elems: 1, data: &mut w.opacities },
                        Channel::F32 { elems, data: &mut w.positions },
                    ];
                    Some(f(&mut d.binding, channels))
                }
                DeformerKind::Rotation(r) => {
                    let channels = vec![
                        Channel::F32 { elems: 1, data: &mut r.opacities },
                        Channel::F32 { elems: 1, data: &mut r.angles },
                        Channel::F32 { elems: 1, data: &mut r.origins_x },
                        Channel::F32 { elems: 1, data: &mut r.origins_y },
                        Channel::F32 { elems: 1, data: &mut r.scales },
                        Channel::Bool { data: &mut r.reflect_x },
                        Channel::Bool { data: &mut r.reflect_y },
                    ];
                    Some(f(&mut d.binding, channels))
                }
            }
        }
    }
}

/// Set (or with `None` clear) extended interpolation on `target`'s
/// parameter `param`. Needs the object to be keyed on it. Returns true on
/// change.
pub fn set_curve(
    model: &mut Model,
    target: Target,
    param: usize,
    kind: Option<ayatsuri_format::model::CurveKind>,
) -> bool {
    with_object(model, target, |binding, _| {
        if !binding.axes.iter().any(|a| a.parameter as usize == param) {
            return false;
        }
        let before = binding.curve(param as u32);
        binding.curves.retain(|c| c.parameter as usize != param);
        if let Some(kind) = kind {
            binding
                .curves
                .push(ayatsuri_format::model::AxisCurve { parameter: param as u32, kind });
        }
        before != kind
    })
    .unwrap_or(false)
}

/// Whether any object uses extended interpolation.
pub fn has_curves(model: &Model) -> bool {
    model.parts.iter().any(|p| !p.binding.curves.is_empty())
        || model.deformers.iter().any(|d| !d.binding.curves.is_empty())
        || model.art_meshes.iter().any(|m| !m.binding.curves.is_empty())
}

/// Turn every extended interpolation into plain keys, the way Cubism hands
/// it to the runtime: `points` keys between each pair, their keyforms taken
/// from the curve, so linear blending between them follows it. For moc3
/// and cmo3 export; the curves themselves are dropped.
pub fn bake_curves(model: &mut Model) {
    let targets: Vec<Target> = (0..model.parts.len())
        .map(Target::Part)
        .chain((0..model.deformers.len()).map(Target::Deformer))
        .chain((0..model.art_meshes.len()).map(Target::ArtMesh))
        .collect();
    for t in targets {
        with_object(model, t, |binding, mut channels| {
            let curves = std::mem::take(&mut binding.curves);
            for c in curves {
                let Some(axis_at) = binding.axes.iter().position(|a| a.parameter == c.parameter)
                else {
                    continue;
                };
                let lens = axis_lens(binding);
                let Some(axis) = binding.axes.get_mut(axis_at) else { continue };
                if axis.keys.len() < 3 {
                    continue;
                }
                let steps = usize::from(c.kind.points().clamp(1, 20)) + 1;
                let mut keys = Vec::new();
                let mut map = Vec::new();
                let mut ws = Vec::with_capacity(4);
                for i in 0..axis.keys.len() {
                    keys.push(axis.keys.get(i).copied().unwrap_or(0.0));
                    map.push(KeySrc::Copy(i));
                    let (Some(&lo), Some(&hi)) = (axis.keys.get(i), axis.keys.get(i + 1)) else {
                        continue;
                    };
                    for s in 1..steps {
                        let t = s as f32 / steps as f32;
                        crate::blend::curve_weights(&axis.keys, i, t, c.kind, &mut ws);
                        let mut mix = [(0usize, 0.0f32); 4];
                        for (slot, w) in mix.iter_mut().zip(&ws) {
                            *slot = *w;
                        }
                        keys.push(lo + (hi - lo) * t);
                        map.push(KeySrc::Mix(mix));
                    }
                }
                axis.keys = keys;
                for ch in &mut channels {
                    remap(&lens, axis_at, &map, ch);
                }
            }
        });
    }
}

/// Run `f` with every mesh's blend shape forms joined onto the end of its
/// keyform positions, then split back off. Edits that rewrite every
/// keyform block alike (topology changes, reparenting, all-keyform moves)
/// carry the blend forms along without knowing about them. A nested call
/// finds the forms already joined and leaves them to the outer one.
fn with_shapes<R>(model: &mut Model, f: impl FnOnce(&mut Model) -> R) -> R {
    // per joined mesh: its index, normal keyform count, blocks per shape
    let mut joined: Vec<(usize, usize, Vec<usize>)> = Vec::new();
    for (mi, m) in model.art_meshes.iter_mut().enumerate() {
        let stride = (m.vertex_count as usize).saturating_mul(2);
        if stride == 0 || m.blend_shapes.is_empty() || m.positions.len() % stride != 0 {
            continue;
        }
        let whole = m.blend_shapes.iter().all(|s| {
            !s.keys.is_empty() && s.positions.len() == s.keys.len().saturating_mul(stride)
        });
        if !whole {
            continue;
        }
        let normal = m.positions.len() / stride;
        let blocks = m.blend_shapes.iter().map(|s| s.keys.len()).collect();
        for s in &mut m.blend_shapes {
            m.positions.append(&mut s.positions);
        }
        joined.push((mi, normal, blocks));
    }
    let out = f(model);
    for (mi, normal, blocks) in joined {
        let Some(m) = model.art_meshes.get_mut(mi) else { continue };
        let stride = (m.vertex_count as usize).saturating_mul(2);
        let mut tail = m.positions.split_off(normal.saturating_mul(stride).min(m.positions.len()));
        for (s, n) in m.blend_shapes.iter_mut().zip(blocks) {
            let rest = tail.split_off(n.saturating_mul(stride).min(tail.len()));
            s.positions = tail;
            tail = rest;
        }
    }
    out
}

fn is_blend_shape(model: &Model, param: usize) -> bool {
    model.parameters.get(param).is_some_and(|p| p.blend_shape)
}

/// A mesh's blend at every parameter's default: the form its blend shapes
/// are differences from. None when its binding does not cover the defaults.
pub fn base_form(model: &Model, mesh: usize) -> Option<Vec<f32>> {
    let m = model.art_meshes.get(mesh)?;
    let stride = (m.vertex_count as usize).saturating_mul(2);
    let mut corners = Vec::new();
    let at_default = blend::corners(
        &m.binding,
        |p| model.parameters.get(p).map_or(0.0, |q| q.default),
        |p| {
            let snap = 0.1f32.powi(model.parameters.get(p).map_or(3, |q| q.decimals).clamp(0, 10));
            (snap, snap * 1.5)
        },
        &mut corners,
    );
    if !at_default {
        return None;
    }
    let mut base = vec![0.0; stride];
    for &(k, w) in &corners {
        let start = k.saturating_mul(stride);
        let Some(src) = m.positions.get(start..start.saturating_add(stride)) else { continue };
        for (b, s) in base.iter_mut().zip(src) {
            *b += s * w;
        }
    }
    Some(base)
}

/// Blend shape keys: every value off the base key gains a keyform on the
/// mesh's blend shape for `param` (made when it has none), holding the form
/// the mesh shows there now, so nothing moves. Art meshes only for now.
fn add_blend_keys(model: &mut Model, target: Target, param: usize, values: &[f32]) -> bool {
    let Target::ArtMesh(mi) = target else { return false };
    let Some(base) = base_form(model, mi) else { return false };
    let Some(m) = model.art_meshes.get_mut(mi) else { return false };
    let stride = base.len();
    if stride == 0 {
        return false;
    }
    let at = match m.blend_shapes.iter().position(|s| s.parameter as usize == param) {
        Some(at) => at,
        None => {
            m.blend_shapes.push(BlendShape {
                parameter: param as u32,
                keys: Vec::new(),
                positions: Vec::new(),
            });
            m.blend_shapes.len() - 1
        }
    };
    let Some(shape) = m.blend_shapes.get_mut(at) else { return false };
    let mut changed = false;
    for &v in values {
        if v.abs() < 1e-5 || shape.keys.iter().any(|k| (k - v).abs() < 1e-5) {
            continue;
        }
        // the base plus whatever the shape already adds at `v`
        let mut form = base.clone();
        for (k, w) in blendshape::weights(v, &shape.keys) {
            let start = k.saturating_mul(stride);
            if w > 0.0
                && let Some(f) = shape.positions.get(start..start.saturating_add(stride))
            {
                blendshape::add(&mut form, f, &base, w);
            }
        }
        let i = shape.keys.partition_point(|&k| k < v);
        let start = i.saturating_mul(stride).min(shape.positions.len());
        shape.keys.insert(i, v);
        shape.positions.splice(start..start, form);
        changed = true;
    }
    if shape.keys.is_empty() {
        m.blend_shapes.remove(at);
    }
    changed
}

/// Remove the blend shape key nearest `value`; the base key cannot go. The
/// last key going takes the blend shape with it.
fn delete_blend_key(model: &mut Model, target: Target, param: usize, value: f32) -> bool {
    let Target::ArtMesh(mi) = target else { return false };
    let Some(m) = model.art_meshes.get_mut(mi) else { return false };
    let stride = (m.vertex_count as usize).saturating_mul(2);
    let Some(at) = m.blend_shapes.iter().position(|s| s.parameter as usize == param) else {
        return false;
    };
    let Some(shape) = m.blend_shapes.get_mut(at) else { return false };
    let Some((i, k)) = shape
        .keys
        .iter()
        .copied()
        .enumerate()
        .min_by(|(_, a), (_, b)| (a - value).abs().total_cmp(&(b - value).abs()))
    else {
        return false;
    };
    if value.abs() < (k - value).abs() {
        return false;
    }
    let start = i.saturating_mul(stride).min(shape.positions.len());
    let end = start.saturating_add(stride).min(shape.positions.len());
    shape.keys.remove(i);
    shape.positions.drain(start..end);
    if shape.keys.is_empty() {
        m.blend_shapes.remove(at);
    }
    true
}

/// Add keys at `values` to `target`'s binding on `param`. Objects not yet
/// bound on the parameter gain a new axis; existing shapes are preserved
/// exactly (new keys take the interpolated form). Returns true on change.
pub fn add_keys(model: &mut Model, target: Target, param: usize, values: &[f32]) -> bool {
    if is_blend_shape(model, param) {
        return add_blend_keys(model, target, param, values);
    }
    with_object(model, target, |binding, mut channels| {
        let mut changed = false;
        let axis_at = binding.axes.iter().position(|a| a.parameter as usize == param);
        let axis_at = match axis_at {
            Some(a) => a,
            None => {
                // a fresh axis varies slowest: the data tiles
                let keys: Vec<f32> = {
                    let mut k: Vec<f32> = values.to_vec();
                    k.sort_by(f32::total_cmp);
                    k.dedup_by(|a, b| (*a - *b).abs() < 1e-5);
                    k
                };
                if keys.is_empty() {
                    return false;
                }
                let k = keys.len();
                binding.axes.push(BindingAxis { parameter: param as u32, keys });
                for ch in &mut channels {
                    tile(ch, k);
                }
                return true;
            }
        };
        let curve = binding.curve(param as u32);
        for &v in values {
            let lens = axis_lens(binding);
            let Some(axis) = binding.axes.get_mut(axis_at) else { break };
            if axis.keys.iter().any(|k| (k - v).abs() < 1e-5) {
                continue;
            }
            let pos = axis.keys.iter().take_while(|k| **k < v).count();
            let old_len = axis.keys.len();
            let mut map = Vec::with_capacity(old_len + 1);
            for i in 0..=old_len {
                if i < pos {
                    map.push(KeySrc::Copy(i));
                } else if i == pos {
                    if pos == 0 {
                        map.push(KeySrc::Copy(0));
                    } else if pos == old_len {
                        map.push(KeySrc::Copy(old_len.saturating_sub(1)));
                    } else {
                        let a = axis.keys.get(pos - 1).copied().unwrap_or(0.0);
                        let b = axis.keys.get(pos).copied().unwrap_or(1.0);
                        let t = if (b - a).abs() > 1e-9 { (v - a) / (b - a) } else { 0.0 };
                        match curve.filter(|_| old_len >= 3) {
                            // on a curved axis the new key takes the curve's
                            // shape, so adding it changes nothing on screen
                            Some(kind) => {
                                let mut ws = Vec::with_capacity(4);
                                crate::blend::curve_weights(&axis.keys, pos - 1, t, kind, &mut ws);
                                let mut mix = [(0usize, 0.0f32); 4];
                                for (slot, w) in mix.iter_mut().zip(&ws) {
                                    *slot = *w;
                                }
                                map.push(KeySrc::Mix(mix));
                            }
                            None => map.push(KeySrc::Lerp(pos - 1, pos, t)),
                        }
                    }
                } else {
                    map.push(KeySrc::Copy(i - 1));
                }
            }
            axis.keys.insert(pos, v);
            for ch in &mut channels {
                remap(&lens, axis_at, &map, ch);
            }
            changed = true;
        }
        changed
    })
    .unwrap_or(false)
}

/// Delete the key nearest `value` on `target`'s axis for `param`. Deleting
/// the last key removes the binding itself. Returns true on change.
pub fn delete_key(model: &mut Model, target: Target, param: usize, value: f32) -> bool {
    if is_blend_shape(model, param) {
        return delete_blend_key(model, target, param, value);
    }
    with_object(model, target, |binding, mut channels| {
        let Some(axis_at) = binding.axes.iter().position(|a| a.parameter as usize == param) else {
            return false;
        };
        let lens = axis_lens(binding);
        let Some(axis) = binding.axes.get_mut(axis_at) else { return false };
        let Some(pos) = axis
            .keys
            .iter()
            .enumerate()
            .min_by(|(_, a), (_, b)| (*a - value).abs().total_cmp(&(*b - value).abs()))
            .map(|(i, _)| i)
        else {
            return false;
        };
        if axis.keys.len() <= 1 {
            // last key: the binding goes away, keyform 0 stays
            let map = [KeySrc::Copy(0)];
            for ch in &mut channels {
                remap(&lens, axis_at, &map, ch);
            }
            binding.axes.remove(axis_at);
            return true;
        }
        let map: Vec<KeySrc> =
            (0..axis.keys.len()).filter(|&i| i != pos).map(KeySrc::Copy).collect();
        axis.keys.remove(pos);
        for ch in &mut channels {
            remap(&lens, axis_at, &map, ch);
        }
        true
    })
    .unwrap_or(false)
}

/// Remove `target`'s whole binding on `param`, keeping the current-default
/// shape (the keyform at the key nearest the parameter default).
pub fn delete_binding(model: &mut Model, target: Target, param: usize, default: f32) -> bool {
    if is_blend_shape(model, param) {
        // at its base key a blend shape adds nothing: dropping it keeps the
        // default shape
        let Target::ArtMesh(mi) = target else { return false };
        let Some(m) = model.art_meshes.get_mut(mi) else { return false };
        let before = m.blend_shapes.len();
        m.blend_shapes.retain(|s| s.parameter as usize != param);
        return m.blend_shapes.len() != before;
    }
    with_object(model, target, |binding, mut channels| {
        let Some(axis_at) = binding.axes.iter().position(|a| a.parameter as usize == param) else {
            return false;
        };
        let lens = axis_lens(binding);
        let keep = binding
            .axes
            .get(axis_at)
            .and_then(|a| {
                a.keys
                    .iter()
                    .enumerate()
                    .min_by(|(_, x), (_, y)| (*x - default).abs().total_cmp(&(*y - default).abs()))
                    .map(|(i, _)| i)
            })
            .unwrap_or(0);
        let map = [KeySrc::Copy(keep)];
        for ch in &mut channels {
            remap(&lens, axis_at, &map, ch);
        }
        binding.axes.remove(axis_at);
        true
    })
    .unwrap_or(false)
}

/// The bounding box of every keyform position of the given meshes, in their
/// (shared) parent coordinate space.
fn meshes_bbox(model: &Model, meshes: &[usize]) -> Option<(f32, f32, f32, f32)> {
    let mut bb: Option<(f32, f32, f32, f32)> = None;
    for &mi in meshes {
        let m = model.art_meshes.get(mi)?;
        let (pairs, _) = m.positions.as_chunks::<2>();
        for [x, y] in pairs {
            bb = Some(match bb {
                None => (*x, *y, *x, *y),
                Some((x0, y0, x1, y1)) => (x0.min(*x), y0.min(*y), x1.max(*x), y1.max(*y)),
            });
        }
    }
    bb
}

/// The parent deformer shared by all the meshes, or None when they sit at
/// the root. Mixed parents make a new deformer ambiguous, so that fails.
fn common_parent(model: &Model, meshes: &[usize]) -> Result<Option<u32>, ()> {
    let mut it = meshes.iter();
    let first = model.art_meshes.get(*it.next().ok_or(())?).ok_or(())?.parent_deformer;
    for &mi in it {
        if model.art_meshes.get(mi).ok_or(())?.parent_deformer != first {
            return Err(());
        }
    }
    Ok(first)
}

/// Create a warp deformer whose identity grid covers the selection, and
/// re-parent the meshes into it. Shapes do not move: the grid maps its
/// normalized domain onto the old parent space one-to-one, and every mesh
/// keyform is re-expressed in that domain. Returns the new deformer index.
pub fn create_warp_deformer(
    model: &mut Model,
    meshes: &[usize],
    rows: u32,
    cols: u32,
    id: String,
) -> Option<usize> {
    with_shapes(model, |model| {
        if meshes.is_empty() {
            return None;
        }
        let parent = common_parent(model, meshes).ok()?;
        let (x0, y0, x1, y1) = meshes_bbox(model, meshes)?;
        // a little margin so edge vertices sit inside the grid
        let (w, h) = ((x1 - x0).max(1e-4), (y1 - y0).max(1e-4));
        let (x0, y0) = (x0 - w * 0.05, y0 - h * 0.05);
        let (w, h) = (w * 1.1, h * 1.1);

        let rows = rows.clamp(1, 64);
        let cols = cols.clamp(1, 64);
        let mut grid = Vec::with_capacity(((rows + 1) * (cols + 1) * 2) as usize);
        for r in 0..=rows {
            for c in 0..=cols {
                grid.push(x0 + w * (c as f32 / cols as f32));
                grid.push(y0 + h * (r as f32 / rows as f32));
            }
        }

        let parent_part =
            meshes.first().and_then(|&m| model.art_meshes.get(m)).and_then(|m| m.parent_part);
        let di = model.deformers.len();
        model.deformers.push(ayatsuri_format::model::Deformer {
            id,
            parent_part,
            parent_deformer: parent,
            visible: true,
            enabled: true,
            binding: Binding::default(),
            kind: DeformerKind::Warp(ayatsuri_format::model::WarpData {
                rows,
                cols,
                quad: true,
                opacities: alloc_one(),
                positions: grid,
            }),
        });

        for &mi in meshes {
            let Some(m) = model.art_meshes.get_mut(mi) else { continue };
            let (pairs, _) = m.positions.as_chunks_mut::<2>();
            for p in pairs {
                p[0] = (p[0] - x0) / w;
                p[1] = (p[1] - y0) / h;
            }
            m.parent_deformer = Some(di as u32);
        }
        Some(di)
    })
}

/// Create a rotation deformer at `origin` (selection center when None) and
/// re-parent the meshes; their coordinates become origin-relative, so
/// nothing moves until the deformer does.
pub fn create_rotation_deformer(
    model: &mut Model,
    meshes: &[usize],
    origin: Option<(f32, f32)>,
    id: String,
) -> Option<usize> {
    with_shapes(model, |model| {
        if meshes.is_empty() {
            return None;
        }
        let parent = common_parent(model, meshes).ok()?;
        let (ox, oy) = match origin {
            Some(o) => o,
            None => {
                let (x0, y0, x1, y1) = meshes_bbox(model, meshes)?;
                ((x0 + x1) * 0.5, (y0 + y1) * 0.5)
            }
        };

        let parent_part =
            meshes.first().and_then(|&m| model.art_meshes.get(m)).and_then(|m| m.parent_part);
        let di = model.deformers.len();
        model.deformers.push(ayatsuri_format::model::Deformer {
            id,
            parent_part,
            parent_deformer: parent,
            visible: true,
            enabled: true,
            binding: Binding::default(),
            kind: DeformerKind::Rotation(ayatsuri_format::model::RotationData {
                base_angle: 0.0,
                opacities: alloc_one(),
                angles: vec![0.0],
                origins_x: vec![ox],
                origins_y: vec![oy],
                scales: vec![1.0],
                reflect_x: vec![false],
                reflect_y: vec![false],
            }),
        });

        for &mi in meshes {
            let Some(m) = model.art_meshes.get_mut(mi) else { continue };
            let (pairs, _) = m.positions.as_chunks_mut::<2>();
            for p in pairs {
                p[0] -= ox;
                p[1] -= oy;
            }
            m.parent_deformer = Some(di as u32);
        }
        Some(di)
    })
}

/// Re-expresses a parent-space point in a new deformer's space.
type PointMap = dyn Fn(f32, f32) -> (f32, f32);

/// What a new deformer is.
#[derive(Clone, Copy, Debug)]
pub enum NewDeformer {
    Warp {
        rows: u32,
        cols: u32,
    },
    /// Origin in the parent's space; None centres it on the children.
    Rotation {
        origin: Option<(f32, f32)>,
    },
}

/// One object a new deformer takes over.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Child {
    Mesh(usize),
    Deformer(usize),
}

/// The parent deformer of a mesh or deformer.
fn parent_of(model: &Model, c: Child) -> Option<Option<u32>> {
    match c {
        Child::Mesh(i) => model.art_meshes.get(i).map(|m| m.parent_deformer),
        Child::Deformer(i) => model.deformers.get(i).map(|d| d.parent_deformer),
    }
}

/// Objects under different deformers, made ready to share one new parent:
/// each is replaced by its own ancestor that sits directly under the
/// selection's nearest common parent, so a new deformer wraps them all
/// without pulling anything out of its rig (a toggle warp above several
/// physics warps keeps every one of them). Repeats collapse. None when an
/// object does not exist.
pub fn lift_to_common_parent(model: &Model, children: &[Child]) -> Option<Vec<Child>> {
    if children.is_empty() {
        return Some(Vec::new());
    }
    // each object's parents, nearest first, ending at the top (None)
    let chain = |c: Child| -> Option<Vec<Option<u32>>> {
        let mut out = vec![parent_of(model, c)?];
        let mut guard = 0;
        while let Some(Some(d)) = out.last().copied() {
            out.push(model.deformers.get(d as usize)?.parent_deformer);
            guard += 1;
            if guard > model.deformers.len() + 1 {
                return None;
            }
        }
        Some(out)
    };
    let chains: Vec<Vec<Option<u32>>> =
        children.iter().map(|&c| chain(c)).collect::<Option<_>>()?;
    let first = chains.first()?;
    // the nearest parent every chain passes through (the top always does)
    let common = first.iter().copied().find(|p| chains.iter().all(|c| c.contains(p)))?;
    let mut out: Vec<Child> = Vec::with_capacity(children.len());
    for &c in children {
        let mut at = c;
        let mut guard = 0;
        while parent_of(model, at)? != common {
            let up = parent_of(model, at)??;
            at = Child::Deformer(up as usize);
            guard += 1;
            if guard > model.deformers.len() + 1 {
                return None;
            }
        }
        if !out.contains(&at) {
            out.push(at);
        }
    }
    Some(out)
}

/// Every point a child stores in its parent's space, all keyforms.
fn child_points(model: &Model, c: Child) -> Vec<(f32, f32)> {
    let pairs = |v: &[f32]| v.as_chunks::<2>().0.iter().map(|p| (p[0], p[1])).collect();
    match c {
        Child::Mesh(i) => model.art_meshes.get(i).map(|m| pairs(&m.positions)).unwrap_or_default(),
        Child::Deformer(i) => match model.deformers.get(i).map(|d| &d.kind) {
            Some(DeformerKind::Warp(w)) => pairs(&w.positions),
            Some(DeformerKind::Rotation(r)) => {
                r.origins_x.iter().zip(&r.origins_y).map(|(&x, &y)| (x, y)).collect()
            }
            None => Vec::new(),
        },
    }
}

/// Re-express a child's parent-space points through `f`.
fn map_child(model: &mut Model, c: Child, f: &dyn Fn(f32, f32) -> (f32, f32)) {
    let map_pairs = |v: &mut Vec<f32>| {
        let (pairs, _) = v.as_chunks_mut::<2>();
        for p in pairs {
            let (x, y) = f(p[0], p[1]);
            p[0] = x;
            p[1] = y;
        }
    };
    match c {
        Child::Mesh(i) => {
            if let Some(m) = model.art_meshes.get_mut(i) {
                map_pairs(&mut m.positions);
            }
        }
        Child::Deformer(i) => match model.deformers.get_mut(i).map(|d| &mut d.kind) {
            Some(DeformerKind::Warp(w)) => map_pairs(&mut w.positions),
            Some(DeformerKind::Rotation(r)) => {
                for (x, y) in r.origins_x.iter_mut().zip(r.origins_y.iter_mut()) {
                    let (nx, ny) = f(*x, *y);
                    *x = nx;
                    *y = ny;
                }
            }
            None => {}
        },
    }
}

/// Whether `d` is `target` or sits somewhere under it.
fn deformer_under(model: &Model, d: usize, target: usize) -> bool {
    let mut cur = Some(d as u32);
    let mut hops = 0;
    while let Some(c) = cur {
        if c as usize == target {
            return true;
        }
        cur = model.deformers.get(c as usize).and_then(|x| x.parent_deformer);
        hops += 1;
        if hops > 1024 {
            break;
        }
    }
    false
}

/// Cubism's deformer creation. With children, the new deformer takes their
/// shared parent and wraps them, every child re-expressed in its space so
/// nothing moves (As Parent of Selected Object; As Child of a deformer is
/// the same call with that deformer's children). With no children it is
/// created empty under `empty_parent` (Set Parent Manually), sized to
/// `empty_extent` in that parent's space. Returns the new index.
pub fn create_deformer(
    model: &mut Model,
    kind: NewDeformer,
    children: &[Child],
    empty_parent: Option<u32>,
    empty_extent: (f32, f32, f32, f32),
    part: Option<u32>,
    id: String,
) -> Option<usize> {
    with_shapes(model, |model| {
        // objects under different deformers: wrap their branches instead
        let lifted = lift_to_common_parent(model, children)?;
        let children = lifted.as_slice();
        let parent = match children.first() {
            Some(&first) => {
                let p = parent_of(model, first)?;
                if children.iter().any(|&c| parent_of(model, c) != Some(p)) {
                    return None;
                }
                p
            }
            None => empty_parent,
        };
        // the extent the deformer covers, in the parent's space
        let (x0, y0, x1, y1) = if children.is_empty() {
            empty_extent
        } else {
            children
                .iter()
                .flat_map(|&c| child_points(model, c))
                .fold((f32::MAX, f32::MAX, f32::MIN, f32::MIN), |(a, b, c, d), (x, y)| {
                    (a.min(x), b.min(y), c.max(x), d.max(y))
                })
        };
        if !(x0 <= x1 && y0 <= y1) {
            return None;
        }
        let span = (x1 - x0).max(y1 - y0).max(1e-3);
        let (deformer_kind, map): (DeformerKind, Box<PointMap>) = match kind {
            NewDeformer::Warp { rows, cols } => {
                // a little margin so edge points sit inside the grid
                let (w, h) = ((x1 - x0).max(span * 0.05), (y1 - y0).max(span * 0.05));
                let (gx, gy, gw, gh) = (x0 - w * 0.05, y0 - h * 0.05, w * 1.1, h * 1.1);
                let (rows, cols) = (rows.clamp(1, 64), cols.clamp(1, 64));
                let mut grid = Vec::with_capacity(((rows + 1) * (cols + 1) * 2) as usize);
                for r in 0..=rows {
                    for c in 0..=cols {
                        grid.push(gx + gw * (c as f32 / cols as f32));
                        grid.push(gy + gh * (r as f32 / rows as f32));
                    }
                }
                (
                    DeformerKind::Warp(ayatsuri_format::model::WarpData {
                        rows,
                        cols,
                        quad: true,
                        opacities: alloc_one(),
                        positions: grid,
                    }),
                    Box::new(move |x, y| ((x - gx) / gw, (y - gy) / gh)),
                )
            }
            NewDeformer::Rotation { origin } => {
                let (ox, oy) = origin.unwrap_or(((x0 + x1) * 0.5, (y0 + y1) * 0.5));
                (
                    DeformerKind::Rotation(ayatsuri_format::model::RotationData {
                        base_angle: 0.0,
                        opacities: alloc_one(),
                        angles: vec![0.0],
                        origins_x: vec![ox],
                        origins_y: vec![oy],
                        scales: vec![1.0],
                        reflect_x: vec![false],
                        reflect_y: vec![false],
                    }),
                    Box::new(move |x, y| (x - ox, y - oy)),
                )
            }
        };
        if let Some(p) = parent
            && model.deformers.get(p as usize).is_none()
        {
            return None;
        }
        // deformers evaluate in index order, parents first (the moc3 rule too),
        // so a new parent of existing deformers goes in just before the first
        // of them and every deformer reference past that point shifts up one
        let di = children
            .iter()
            .filter_map(|&c| if let Child::Deformer(d) = c { Some(d) } else { None })
            .min()
            .unwrap_or(model.deformers.len())
            .min(model.deformers.len());
        let bump = |r: &mut Option<u32>| {
            if let Some(x) = r
                && *x as usize >= di
            {
                *x += 1;
            }
        };
        for m in &mut model.art_meshes {
            bump(&mut m.parent_deformer);
        }
        for d in &mut model.deformers {
            bump(&mut d.parent_deformer);
        }
        let shift = |i: usize| if i >= di { i + 1 } else { i };
        let children: Vec<Child> = children
            .iter()
            .map(|&c| match c {
                Child::Deformer(d) => Child::Deformer(shift(d)),
                m => m,
            })
            .collect();
        let mut parent = parent;
        bump(&mut parent);
        model.deformers.insert(
            di,
            ayatsuri_format::model::Deformer {
                id,
                parent_part: part,
                parent_deformer: parent,
                visible: true,
                enabled: true,
                binding: Binding::default(),
                kind: deformer_kind,
            },
        );
        for &c in &children {
            // a deformer can't become a child of its own descendant
            if let Child::Deformer(d) = c
                && parent.is_some_and(|p| deformer_under(model, p as usize, d))
            {
                continue;
            }
            map_child(model, c, &*map);
            match c {
                Child::Mesh(i) => {
                    if let Some(m) = model.art_meshes.get_mut(i) {
                        m.parent_deformer = Some(di as u32);
                    }
                }
                Child::Deformer(i) => {
                    if let Some(d) = model.deformers.get_mut(i) {
                        d.parent_deformer = Some(di as u32);
                    }
                }
            }
        }
        Some(di)
    })
}

/// Where a point in `deformer`'s local space lands on the canvas, and the
/// inverse: a canvas point back into the local space of `deformer` (or the
/// canvas itself for None). Newton steps on the forward map with a finite
/// difference jacobian; rotations are affine and land in one step, warps in
/// a handful.
pub(crate) fn to_local(
    puppet: &crate::Puppet,
    deformer: Option<usize>,
    world: (f32, f32),
) -> (f32, f32) {
    let Some(d) = deformer else { return world };
    let fwd = |x: f32, y: f32| puppet.deformer_point(d, x, y);
    // a warp's local space is its unit square; a rotation's is canvas-like
    let mut p = match puppet.model().deformers.get(d).map(|d| &d.kind) {
        Some(DeformerKind::Warp(_)) => (0.5, 0.5),
        _ => (0.0, 0.0),
    };
    for _ in 0..40 {
        let (fx, fy) = fwd(p.0, p.1);
        let (ex, ey) = (world.0 - fx, world.1 - fy);
        if ex.abs() + ey.abs() < 1e-6 {
            break;
        }
        let h = 1e-3;
        let (ax, ay) = fwd(p.0 + h, p.1);
        let (bx, by) = fwd(p.0, p.1 + h);
        let (j00, j10, j01, j11) = ((ax - fx) / h, (ay - fy) / h, (bx - fx) / h, (by - fy) / h);
        let det = j00 * j11 - j01 * j10;
        if det.abs() < 1e-12 || !det.is_finite() {
            break;
        }
        let dx = (j11 * ex - j01 * ey) / det;
        let dy = (-j10 * ex + j00 * ey) / det;
        p = (p.0 + dx, p.1 + dy);
    }
    p
}

/// A puppet of `model` evaluated at every parameter's default.
pub(crate) fn rest_puppet(model: &Model) -> crate::Puppet {
    let mut puppet = crate::Puppet::new(model.clone());
    for (i, p) in model.parameters.iter().enumerate() {
        puppet.set_value(i, p.default);
    }
    puppet.update();
    puppet
}

/// Put deformers back in "parents first" order (the evaluator and moc3
/// both need it), keeping everything else in its current order, and
/// renumber every reference. Returns old index -> new index.
fn sort_parents_first(model: &mut Model) -> Vec<usize> {
    let n = model.deformers.len();
    let mut placed = vec![false; n];
    let mut order: Vec<usize> = Vec::with_capacity(n);
    // repeatedly take, in index order, every deformer whose parent is placed
    while order.len() < n {
        let before = order.len();
        for i in 0..n {
            if placed.get(i).copied().unwrap_or(true) {
                continue;
            }
            let ready = match model.deformers.get(i).and_then(|d| d.parent_deformer) {
                None => true,
                Some(p) => placed.get(p as usize).copied().unwrap_or(true),
            };
            if ready {
                if let Some(f) = placed.get_mut(i) {
                    *f = true;
                }
                order.push(i);
            }
        }
        if order.len() == before {
            // a cycle (never made by these edits): keep the rest as it was
            order.extend((0..n).filter(|&i| !placed.get(i).copied().unwrap_or(true)));
            break;
        }
    }
    let mut new_of = vec![0usize; n];
    for (new, &old) in order.iter().enumerate() {
        if let Some(slot) = new_of.get_mut(old) {
            *slot = new;
        }
    }
    if order.iter().enumerate().all(|(new, &old)| new == old) {
        return new_of;
    }
    let mut old = core::mem::take(&mut model.deformers).into_iter().map(Some).collect::<Vec<_>>();
    for &i in &order {
        if let Some(d) = old.get_mut(i).and_then(Option::take) {
            model.deformers.push(d);
        }
    }
    let remap = |r: &mut Option<u32>| {
        if let Some(x) = r {
            *x = new_of.get(*x as usize).copied().unwrap_or(*x as usize) as u32;
        }
    };
    for m in &mut model.art_meshes {
        remap(&mut m.parent_deformer);
    }
    for d in &mut model.deformers {
        remap(&mut d.parent_deformer);
    }
    new_of
}

/// Give `child` a new parent deformer (None: no deformer) without moving
/// it on screen. Its points go out through the old parent at the default
/// pose and back in through the new one; a rotation deformer also keeps
/// its angle and scale, which it would otherwise inherit differently. A
/// deformer can't move under itself or its own descendants. Deformers are
/// re-sorted parents-first when needed; returns the child's new index.
pub fn set_parent(model: &mut Model, child: Child, new_parent: Option<usize>) -> Option<Child> {
    with_shapes(model, |model| {
        let old_parent = parent_of(model, child)?;
        if new_parent.is_some_and(|p| model.deformers.get(p).is_none()) {
            return None;
        }
        if let (Child::Deformer(d), Some(p)) = (child, new_parent)
            && deformer_under(model, p, d)
        {
            return None;
        }
        if old_parent.map(|p| p as usize) == new_parent {
            return Some(child);
        }
        let before = rest_puppet(model);
        let old_frame = match child {
            Child::Deformer(d) => before.rotation_state(d).map(|r| (r.angle, r.scale)),
            Child::Mesh(_) => None,
        };
        let remap = |x: f32, y: f32| {
            let world = match old_parent {
                Some(p) => before.deformer_point(p as usize, x, y),
                None => (x, y),
            };
            to_local(&before, new_parent, world)
        };
        map_child(model, child, &remap);
        let np = new_parent.map(|p| p as u32);
        match child {
            Child::Mesh(i) => {
                if let Some(m) = model.art_meshes.get_mut(i) {
                    m.parent_deformer = np;
                }
            }
            Child::Deformer(i) => {
                if let Some(d) = model.deformers.get_mut(i) {
                    d.parent_deformer = np;
                }
            }
        }
        let new_of = sort_parents_first(model);
        let child = match child {
            Child::Deformer(d) => Child::Deformer(new_of.get(d).copied().unwrap_or(d)),
            m => m,
        };
        // a rotation reads its parents' turn and scale; give back the difference
        if let (Child::Deformer(d), Some((old_angle, old_scale))) = (child, old_frame) {
            let after = rest_puppet(model);
            if let (Some(now), Some(def)) = (after.rotation_state(d), model.deformers.get_mut(d))
                && let DeformerKind::Rotation(r) = &mut def.kind
            {
                r.base_angle += old_angle - now.angle;
                if now.scale.abs() > 1e-6 && old_scale.is_finite() {
                    let ratio = old_scale / now.scale;
                    for s in &mut r.scales {
                        *s *= ratio;
                    }
                }
            }
        }
        Some(child)
    })
}

/// Delete a deformer. Its meshes and child deformers move up to its parent
/// and keep their shape (see `set_parent`); only the deleted deformer's own
/// motion goes with it. References past it shift down.
pub fn delete_deformer(model: &mut Model, deformer: usize) -> bool {
    with_shapes(model, |model| {
        let Some(gone) = model.deformers.get(deformer) else { return false };
        let parent = gone.parent_deformer.map(|p| p as usize);
        let meshes: Vec<usize> = model
            .art_meshes
            .iter()
            .enumerate()
            .filter(|(_, m)| m.parent_deformer == Some(deformer as u32))
            .map(|(i, _)| i)
            .collect();
        for i in meshes {
            set_parent(model, Child::Mesh(i), parent);
        }
        // a deformer's parent comes before it, and the deleted one's parent
        // comes before the deleted one, so moving children up never re-sorts
        let kids: Vec<usize> = model
            .deformers
            .iter()
            .enumerate()
            .filter(|(_, d)| d.parent_deformer == Some(deformer as u32))
            .map(|(i, _)| i)
            .collect();
        for i in kids {
            set_parent(model, Child::Deformer(i), parent);
        }
        if model.deformers.get(deformer).is_none() {
            return false;
        }
        model.deformers.remove(deformer);
        let shift = |r: &mut Option<u32>| {
            if let Some(x) = r
                && *x as usize > deformer
            {
                *x -= 1;
            }
        };
        for m in &mut model.art_meshes {
            shift(&mut m.parent_deformer);
        }
        for d in &mut model.deformers {
            shift(&mut d.parent_deformer);
        }
        true
    })
}

/// Delete art meshes. Glues that hold one of them go too; other meshes
/// stop clipping by them; draw order groups lose their items (and every
/// group above counts one mesh fewer). References past them shift down.
pub fn delete_art_meshes(model: &mut Model, meshes: &[usize]) -> bool {
    let mut gone: Vec<usize> =
        meshes.iter().copied().filter(|&i| i < model.art_meshes.len()).collect();
    gone.sort_unstable();
    gone.dedup();
    if gone.is_empty() {
        return false;
    }
    let is_gone = |x: u32| gone.binary_search(&(x as usize)).is_ok();
    // where an index lands once the deleted ones are out
    let new_index = |x: u32| x - gone.partition_point(|&g| g < x as usize) as u32;

    // which group holds which as a child, to fix the totals up the tree
    let mut parent_group: Vec<Option<usize>> = vec![None; model.draw_groups.len()];
    for (gi, g) in model.draw_groups.iter().enumerate() {
        for it in &g.items {
            if let Some(slot) = it.child_group.and_then(|c| parent_group.get_mut(c as usize)) {
                *slot = Some(gi);
            }
        }
    }
    let mut removed_from: Vec<u32> = vec![0; model.draw_groups.len()];
    for (gi, g) in model.draw_groups.iter_mut().enumerate() {
        let before = g.items.len();
        g.items.retain(|it| it.part || !is_gone(it.object));
        if let Some(r) = removed_from.get_mut(gi) {
            *r = (before - g.items.len()) as u32;
        }
        for it in &mut g.items {
            if !it.part {
                it.object = new_index(it.object);
            }
        }
    }
    for (gi, &n) in removed_from.clone().iter().enumerate() {
        let mut at = Some(gi);
        // a malformed cycle must not spin forever
        let mut hops = 0;
        while let Some(g) = at
            && hops <= removed_from.len()
        {
            if let Some(group) = model.draw_groups.get_mut(g) {
                group.total = group.total.saturating_sub(n);
            }
            at = parent_group.get(g).copied().flatten();
            hops += 1;
        }
    }

    model.glues.retain(|g| !is_gone(g.art_mesh_a) && !is_gone(g.art_mesh_b));
    for g in &mut model.glues {
        g.art_mesh_a = new_index(g.art_mesh_a);
        g.art_mesh_b = new_index(g.art_mesh_b);
    }
    let mut index = 0usize;
    model.art_meshes.retain(|_| {
        let keep = gone.binary_search(&index).is_err();
        index += 1;
        keep
    });
    let remap = |masks: &mut Vec<u32>| {
        masks.retain(|&x| !is_gone(x));
        for x in masks.iter_mut() {
            *x = new_index(*x);
        }
    };
    for m in &mut model.art_meshes {
        remap(&mut m.masks);
    }
    for g in &mut model.clip_groups {
        remap(&mut g.masks);
    }
    for o in model.parts.iter_mut().filter_map(|p| p.offscreen.as_mut()) {
        remap(&mut o.masks);
    }
    let ids: std::collections::HashSet<&str> =
        model.art_meshes.iter().map(|m| m.id.as_str()).collect();
    let paths = std::mem::take(&mut model.art_paths);
    model.art_paths = paths.into_iter().filter(|p| ids.contains(p.mesh.as_str())).collect();
    true
}

/// Put an art mesh (a copy from the clipboard) into the model and return its
/// index. It keeps its id when that is free (a cut and paste is a move),
/// else takes `<id>_copy`, `<id>_copy2`...; it draws next to the mesh named
/// `beside` when that is still there, else among its part's meshes, else
/// at the end of the top group. Its ArtPath comes with it. Masks pointing
/// past the end are dropped; glue is not copied.
pub fn insert_art_mesh(
    model: &mut Model,
    mut mesh: ayatsuri_format::model::ArtMesh,
    path: Option<ayatsuri_format::model::ArtPath>,
    beside: Option<&str>,
) -> usize {
    let taken = |id: &str, m: &Model| m.art_meshes.iter().any(|x| x.id == id);
    if taken(&mesh.id, model) {
        let base = format!("{}_copy", mesh.id);
        let mut id = base.clone();
        let mut n = 2;
        while taken(&id, model) {
            id = format!("{base}{n}");
            n += 1;
        }
        mesh.id = id;
    }
    let count = model.art_meshes.len() as u32;
    mesh.masks.retain(|&x| x < count);
    if mesh.clip_group.is_some_and(|g| g as usize >= model.clip_groups.len()) {
        mesh.clip_group = None;
    }
    if mesh.parent_part.is_some_and(|p| p as usize >= model.parts.len()) {
        mesh.parent_part = None;
    }
    if mesh.parent_deformer.is_some_and(|d| d as usize >= model.deformers.len()) {
        mesh.parent_deformer = None;
    }
    let index = model.art_meshes.len();
    let next_to = beside.and_then(|id| model.art_meshes.iter().position(|m| m.id == id));
    let part = mesh.parent_part;

    // the group and slot the copy draws in
    let mut place: Option<(usize, usize)> = None;
    for (gi, g) in model.draw_groups.iter().enumerate() {
        for (ii, it) in g.items.iter().enumerate() {
            if it.part {
                continue;
            }
            let o = it.object as usize;
            if Some(o) == next_to {
                place = Some((gi, ii + 1));
            } else if place.is_none()
                && next_to.is_none()
                && model.art_meshes.get(o).is_some_and(|m| m.parent_part == part)
            {
                place = Some((gi, g.items.len()));
            }
        }
    }
    let (group, slot) =
        place.unwrap_or((0, model.draw_groups.first().map_or(0, |g| g.items.len())));
    if let Some(path) = path {
        model.art_paths.push(ayatsuri_format::model::ArtPath { mesh: mesh.id.clone(), ..path });
    }
    model.art_meshes.push(mesh);
    if model.draw_groups.is_empty() {
        return index;
    }
    let mut parent_group: Vec<Option<usize>> = vec![None; model.draw_groups.len()];
    for (gi, g) in model.draw_groups.iter().enumerate() {
        for it in &g.items {
            if let Some(slot) = it.child_group.and_then(|c| parent_group.get_mut(c as usize)) {
                *slot = Some(gi);
            }
        }
    }
    if let Some(g) = model.draw_groups.get_mut(group) {
        let slot = slot.min(g.items.len());
        g.items.insert(
            slot,
            ayatsuri_format::model::DrawItem {
                part: false,
                object: index as u32,
                child_group: None,
            },
        );
    }
    let mut at = Some(group);
    let mut hops = 0;
    while let Some(g) = at
        && hops <= parent_group.len()
    {
        if let Some(gr) = model.draw_groups.get_mut(g) {
            gr.total += 1;
        }
        at = parent_group.get(g).copied().flatten();
        hops += 1;
    }
    index
}

/// The ArtPath behind a mesh, if it was drawn as one.
pub fn art_path_of(model: &Model, mesh: usize) -> Option<&ayatsuri_format::model::ArtPath> {
    let id = &model.art_meshes.get(mesh)?.id;
    model.art_paths.iter().find(|p| &p.mesh == id)
}

/// A colour's cell on the swatch page, adding it when new. A full page
/// reuses the nearest colour already on it.
fn swatch_cell(model: &mut Model, c: [u8; 4]) -> usize {
    if let Some(i) = model.art_path_colors.iter().position(|x| *x == c) {
        return i;
    }
    if model.art_path_colors.len() < crate::artpath::SWATCH_CELLS {
        model.art_path_colors.push(c);
        return model.art_path_colors.len() - 1;
    }
    let far =
        |x: &[u8; 4]| -> i32 { x.iter().zip(c).map(|(a, b)| (*a as i32 - b as i32).pow(2)).sum() };
    model.art_path_colors.iter().enumerate().min_by_key(|(_, x)| far(x)).map_or(0, |(i, _)| i)
}

/// A path's triangles placed for `parent` (canvas pixels at rest -> model
/// units -> the deformer's space), with uvs on the swatch page.
fn art_path_geometry(
    model: &mut Model,
    path: &ayatsuri_format::model::ArtPath,
    parent: Option<usize>,
) -> Option<(Vec<f32>, Vec<f32>, Vec<u16>)> {
    let built = crate::artpath::build(path);
    if built.indices.is_empty() {
        return None;
    }
    let stroke = swatch_cell(model, path.stroke_color);
    let fill = swatch_cell(model, path.fill_color);
    let c = model.canvas;
    let ppu = c.pixels_per_unit.max(1e-6);
    let rest = parent.map(|_| rest_puppet(model));
    let mut positions = Vec::with_capacity(built.points.len() * 2);
    for p in &built.points {
        let world = ((p[0] - c.origin_x) / ppu, (p[1] - c.origin_y) / ppu);
        let (x, y) = match &rest {
            Some(r) => to_local(r, parent, world),
            None => world,
        };
        positions.extend([x, y]);
    }
    let uvs = built
        .roles
        .iter()
        .flat_map(|r| crate::artpath::swatch_uv(if r.fill { fill } else { stroke }, r.fade))
        .collect();
    Some((positions, uvs, built.indices))
}

/// Add a mesh drawn from `path`, in `part`, textured from swatch page
/// `page`, on top of the draw order. Returns its index.
pub fn create_art_path(
    model: &mut Model,
    mut path: ayatsuri_format::model::ArtPath,
    part: Option<u32>,
    page: u32,
) -> Option<usize> {
    let (positions, uvs, indices) = art_path_geometry(model, &path, None)?;
    let taken: std::collections::HashSet<&str> =
        model.art_meshes.iter().map(|m| m.id.as_str()).collect();
    let id = (0..).map(|n| format!("ArtPath{n}")).find(|id| !taken.contains(id.as_str()))?;
    let order = 500.0;
    model.art_meshes.push(ayatsuri_format::model::ArtMesh {
        id: id.clone(),
        parent_part: part,
        parent_deformer: None,
        visible: true,
        enabled: true,
        texture: page,
        flags: 0,
        blend: Default::default(),
        vertex_count: (positions.len() / 2) as u32,
        uvs,
        indices,
        masks: Vec::new(),
        binding: Binding::default(),
        opacities: vec![1.0],
        draw_orders: vec![order],
        positions,
        blend_shapes: Vec::new(),
        clip_group: None,
        clip_inherit: false,
    });
    let index = model.art_meshes.len() - 1;
    if let Some(root) = model.draw_groups.first_mut() {
        root.items.push(ayatsuri_format::model::DrawItem {
            part: false,
            object: index as u32,
            child_group: None,
        });
        root.total += 1;
        root.min_order = root.min_order.min(order as i32);
        root.max_order = root.max_order.max(order as i32);
    }
    path.mesh = id;
    model.art_paths.push(path);
    Some(index)
}

/// Redraw an ArtPath mesh from new path settings. The mesh keeps its place,
/// parent, masks, opacity and draw order; every keyform takes the new rest
/// shape (a different point count can't carry the old shapes over), and
/// glues on it go, since their points are gone.
pub fn update_art_path(
    model: &mut Model,
    mesh: usize,
    mut path: ayatsuri_format::model::ArtPath,
) -> bool {
    let Some(m) = model.art_meshes.get(mesh) else { return false };
    let id = m.id.clone();
    let parent = m.parent_deformer.map(|p| p as usize);
    let keyforms = m.binding.keyform_count().max(1);
    let Some((rest, uvs, indices)) = art_path_geometry(model, &path, parent) else {
        return false;
    };
    let was_mask_only = model.art_paths.iter().find(|p| p.mesh == id).is_some_and(|p| p.mask_only);
    let Some(m) = model.art_meshes.get_mut(mesh) else { return false };
    m.vertex_count = (rest.len() / 2) as u32;
    m.positions = rest.repeat(keyforms);
    m.uvs = uvs;
    m.indices = indices;
    if path.mask_only != was_mask_only {
        let to = if path.mask_only { 0.0 } else { 1.0 };
        m.opacities.fill(to);
    }
    while let Some(g) = model
        .glues
        .iter()
        .position(|g| g.art_mesh_a as usize == mesh || g.art_mesh_b as usize == mesh)
    {
        model.glues.remove(g);
    }
    path.mesh = id.clone();
    match model.art_paths.iter_mut().find(|p| p.mesh == id) {
        Some(slot) => *slot = path,
        None => model.art_paths.push(path),
    }
    true
}

/// Set which art meshes clip `mesh` (its masks). Itself, unknown indices
/// and repeats are dropped. A mesh in a clip group sets the group's masks,
/// so every mesh in it follows. False when nothing changed.
pub fn set_masks(model: &mut Model, mesh: usize, masks: &[u32]) -> bool {
    if let Some(g) = model.art_meshes.get(mesh).and_then(|m| m.clip_group) {
        return set_group_masks(model, g as usize, masks);
    }
    let clean = clean_masks(model, masks, Some(mesh));
    let Some(m) = model.art_meshes.get_mut(mesh) else { return false };
    if m.masks == clean {
        return false;
    }
    m.masks = clean;
    true
}

/// `masks` without unknown meshes, repeats, or `own` (no mesh clips itself).
fn clean_masks(model: &Model, masks: &[u32], own: Option<usize>) -> Vec<u32> {
    let n = model.art_meshes.len();
    let mut clean: Vec<u32> = Vec::with_capacity(masks.len());
    for &x in masks {
        if (x as usize) < n && Some(x as usize) != own && !clean.contains(&x) {
            clean.push(x);
        }
    }
    clean
}

/// Copy every clip group's masks into the meshes in it: groups are the
/// source, `ArtMesh::masks` what every renderer and export reads.
fn sync_clip_groups(model: &mut Model) {
    let groups = model.clip_groups.clone();
    for (mi, m) in model.art_meshes.iter_mut().enumerate() {
        let Some(g) = m.clip_group.and_then(|g| groups.get(g as usize)) else { continue };
        m.masks = g.masks.iter().copied().filter(|&x| x as usize != mi).collect();
    }
}

/// Set a clip group's masks; every mesh in it follows. False when nothing
/// changed.
pub fn set_group_masks(model: &mut Model, group: usize, masks: &[u32]) -> bool {
    let clean = clean_masks(model, masks, None);
    let Some(g) = model.clip_groups.get_mut(group) else { return false };
    if g.masks == clean {
        return false;
    }
    g.masks = clean;
    sync_clip_groups(model);
    true
}

/// Make a clip group named `name` from `mesh`'s masks, with the mesh in it.
/// The new group's index, or None when the mesh has no masks to share.
pub fn create_clip_group(model: &mut Model, mesh: usize, name: String) -> Option<usize> {
    let masks = model.art_meshes.get(mesh)?.masks.clone();
    if masks.is_empty() {
        return None;
    }
    model.clip_groups.push(ayatsuri_format::model::ClipGroup { name, masks });
    let g = model.clip_groups.len() - 1;
    if let Some(m) = model.art_meshes.get_mut(mesh) {
        m.clip_group = Some(g as u32);
    }
    Some(g)
}

/// Put `mesh` in clip group `group` (its masks become the group's), or
/// with None take it out (it keeps the masks it has). False when nothing
/// changed.
pub fn set_clip_group(model: &mut Model, mesh: usize, group: Option<usize>) -> bool {
    if group.is_some_and(|g| g >= model.clip_groups.len()) {
        return false;
    }
    let Some(m) = model.art_meshes.get_mut(mesh) else { return false };
    let group = group.map(|g| g as u32);
    if m.clip_group == group {
        return false;
    }
    m.clip_group = group;
    sync_clip_groups(model);
    true
}

/// Delete a clip group. Its meshes leave it keeping their masks; later
/// groups shift down by one.
pub fn delete_clip_group(model: &mut Model, group: usize) -> bool {
    if group >= model.clip_groups.len() {
        return false;
    }
    model.clip_groups.remove(group);
    let g32 = group as u32;
    for m in &mut model.art_meshes {
        m.clip_group = match m.clip_group {
            Some(g) if g == g32 => None,
            Some(g) if g > g32 => Some(g - 1),
            other => other,
        };
    }
    true
}

/// Also clip `mesh` by what its masks are clipped by (see `clip`). False
/// when nothing changed.
pub fn set_clip_inherit(model: &mut Model, mesh: usize, on: bool) -> bool {
    let Some(m) = model.art_meshes.get_mut(mesh) else { return false };
    if m.clip_inherit == on {
        return false;
    }
    m.clip_inherit = on;
    true
}

/// Set the opacity an art mesh or deformer has on one keyform (the one the
/// parameters sit on, from `keyform::keyform_at`). Parts have no opacity
/// keyforms in the model, so they are refused.
pub fn set_opacity(model: &mut Model, target: Target, keyform: usize, opacity: f32) -> bool {
    let slot = match target {
        Target::ArtMesh(i) => {
            model.art_meshes.get_mut(i).and_then(|m| m.opacities.get_mut(keyform))
        }
        Target::Deformer(i) => model.deformers.get_mut(i).and_then(|d| match &mut d.kind {
            DeformerKind::Warp(w) => w.opacities.get_mut(keyform),
            DeformerKind::Rotation(r) => r.opacities.get_mut(keyform),
        }),
        Target::Part(i) => {
            model.parts.get_mut(i).and_then(|p| p.offscreen.as_mut()?.opacities.get_mut(keyform))
        }
    };
    let Some(slot) = slot else { return false };
    let opacity = opacity.clamp(0.0, 1.0);
    if (*slot - opacity).abs() < 1e-6 {
        return false;
    }
    *slot = opacity;
    true
}

/// Turn offscreen drawing on a part on (opaque, no masks, Normal blend) or
/// off. True on change. Turning it on also sets the part's draw order to
/// its lowest mesh's, since it now sorts as one item at its own order: the
/// group stays where its first mesh was.
pub fn set_offscreen(model: &mut Model, part: usize, on: bool) -> bool {
    let p32 = part as u32;
    let lowest = model
        .art_meshes
        .iter()
        .filter(|m| {
            let mut at = m.parent_part;
            for _ in 0..model.parts.len() {
                match at {
                    Some(pi) if pi == p32 => return true,
                    Some(pi) => at = model.parts.get(pi as usize).and_then(|p| p.parent_part),
                    None => break,
                }
            }
            false
        })
        .filter_map(|m| m.draw_orders.first().copied())
        .min_by(f32::total_cmp);
    let Some(p) = model.parts.get_mut(part) else { return false };
    if p.offscreen.is_some() == on {
        return false;
    }
    if on && let Some(order) = lowest {
        for d in &mut p.draw_orders {
            *d = order.round();
        }
    }
    p.offscreen = on.then(|| ayatsuri_format::model::Offscreen {
        masks: Vec::new(),
        inverted: false,
        blend: Default::default(),
        opacities: vec![1.0; p.draw_orders.len().max(1)],
    });
    true
}

/// Rename a deformer's id; refused when another deformer already has it.
pub fn rename_deformer(model: &mut Model, deformer: usize, id: &str) -> bool {
    let id = id.trim();
    if id.is_empty() || model.deformers.iter().enumerate().any(|(i, d)| i != deformer && d.id == id)
    {
        return false;
    }
    match model.deformers.get_mut(deformer) {
        Some(d) => {
            d.id = id.to_string();
            true
        }
        None => false,
    }
}

/// Change a warp deformer's division to `cols` x `rows` after it was
/// made: every keyform's grid is sampled again at the new division, so
/// what the warp holds keeps its shape (exactly when the division gets
/// finer). Its children are placed by the warp's 0 to 1 space and need no
/// change. False when it is not a warp, the division is out of range
/// (1 to 64) or already that.
pub fn set_warp_division(model: &mut Model, deformer: usize, cols: u32, rows: u32) -> bool {
    if !(1..=64).contains(&cols) || !(1..=64).contains(&rows) {
        return false;
    }
    let Some(DeformerKind::Warp(w)) = model.deformers.get_mut(deformer).map(|d| &mut d.kind) else {
        return false;
    };
    if (w.cols, w.rows) == (cols, rows) {
        return false;
    }
    let stride = w.grid_points() * 2;
    if stride == 0 || w.positions.len() % stride != 0 {
        return false;
    }
    let positions = w
        .positions
        .chunks(stride)
        .flat_map(|k| {
            crate::bezier::resample_grid(
                k,
                w.cols as usize,
                w.rows as usize,
                cols as usize,
                rows as usize,
            )
        })
        .collect();
    w.positions = positions;
    (w.cols, w.rows) = (cols, rows);
    true
}

/// Mesh Edit's vertex move: shift it by (dx, dy) in the parent's space on
/// every keyform and give it the texture coordinate `uv` that was under its
/// new spot, so the vertex slides over the art instead of dragging it.
pub fn slide_vertex(
    model: &mut Model,
    mesh: usize,
    vertex: usize,
    dx: f32,
    dy: f32,
    uv: (f32, f32),
) -> bool {
    if !uv.0.is_finite() || !uv.1.is_finite() || !move_vertex(model, mesh, vertex, dx, dy) {
        return false;
    }
    let Some(m) = model.art_meshes.get_mut(mesh) else { return false };
    let at = vertex * 2;
    match (m.uvs.get(at).is_some(), m.uvs.get(at + 1).is_some()) {
        (true, true) => {
            if let Some(u) = m.uvs.get_mut(at) {
                *u = uv.0;
            }
            if let Some(v) = m.uvs.get_mut(at + 1) {
                *v = uv.1;
            }
            true
        }
        _ => false,
    }
}

/// Shift one vertex by (dx, dy) in the mesh's parent space, across every
/// keyform. Blending is affine, so the evaluated vertex moves by exactly the
/// same amount whatever the current pose.
pub fn move_vertex(model: &mut Model, mesh: usize, vertex: usize, dx: f32, dy: f32) -> bool {
    with_shapes(model, |model| {
        let Some(m) = model.art_meshes.get_mut(mesh) else { return false };
        let stride = (m.vertex_count as usize).saturating_mul(2);
        if stride == 0 || vertex >= m.vertex_count as usize {
            return false;
        }
        let at = vertex * 2;
        let mut touched = false;
        for form in m.positions.chunks_exact_mut(stride) {
            if let Some(x) = form.get_mut(at) {
                *x += dx;
            }
            if let Some(y) = form.get_mut(at + 1) {
                *y += dy;
                touched = true;
            }
        }
        touched
    })
}

/// Shift several vertices at once, each by its own parent-space offset.
/// The deform path and brush tools land their falloff-weighted drags here.
pub fn move_vertices(model: &mut Model, mesh: usize, moves: &[(usize, f32, f32)]) -> bool {
    with_shapes(model, |model| {
        let Some(m) = model.art_meshes.get_mut(mesh) else { return false };
        let count = m.vertex_count as usize;
        let stride = count.saturating_mul(2);
        if stride == 0 {
            return false;
        }
        let mut touched = false;
        for form in m.positions.chunks_exact_mut(stride) {
            for &(vertex, dx, dy) in moves {
                if vertex >= count {
                    continue;
                }
                let at = vertex * 2;
                if let Some(x) = form.get_mut(at) {
                    *x += dx;
                }
                if let Some(y) = form.get_mut(at + 1) {
                    *y += dy;
                    touched = true;
                }
            }
        }
        touched
    })
}

/// Split the triangle `tri` of a mesh at the barycentric point (w0, w1, w2).
/// The new vertex is appended, so nothing else moves; its uv and every
/// keyform position are the same weighted mix of the triangle's corners,
/// which keeps all existing deformation intact.
pub fn add_vertex(model: &mut Model, mesh: usize, tri: usize, w: [f32; 3]) -> Option<usize> {
    with_shapes(model, |model| {
        let m = model.art_meshes.get_mut(mesh)?;
        let n = m.vertex_count as usize;
        if n >= u16::MAX as usize - 1 {
            return None;
        }
        let at = tri.checked_mul(3)?;
        let corners = [
            m.indices.get(at).copied()? as usize,
            m.indices.get(at + 1).copied()? as usize,
            m.indices.get(at + 2).copied()? as usize,
        ];
        if corners.iter().any(|&c| c >= n) {
            return None;
        }

        let mix = |pool: &[f32], base: usize| -> Option<(f32, f32)> {
            let mut x = 0.0;
            let mut y = 0.0;
            for (&c, &wc) in corners.iter().zip(w.iter()) {
                x += pool.get(base + c * 2).copied()? * wc;
                y += pool.get(base + c * 2 + 1).copied()? * wc;
            }
            Some((x, y))
        };

        let (u, v) = mix(&m.uvs, 0)?;
        let stride = n * 2;
        let keyforms = m.positions.len().checked_div(stride).unwrap_or(0);
        let mut positions = Vec::with_capacity(keyforms * (stride + 2));
        for k in 0..keyforms {
            let base = k * stride;
            let (x, y) = mix(&m.positions, base)?;
            positions.extend_from_slice(m.positions.get(base..base + stride)?);
            positions.push(x);
            positions.push(y);
        }

        let new = n as u16;
        m.positions = positions;
        m.uvs.push(u);
        m.uvs.push(v);
        m.vertex_count += 1;
        // the split: one triangle becomes three fanned around the new vertex
        let (a, b, c) = (corners[0] as u16, corners[1] as u16, corners[2] as u16);
        if let Some(slot) = m.indices.get_mut(at..at + 3) {
            slot.copy_from_slice(&[a, b, new]);
        }
        m.indices.extend_from_slice(&[b, c, new, c, a, new]);
        Some(n)
    })
}

/// Add a vertex anywhere, placed by barycentric weights against three
/// existing points.
/// Weights may be negative: outside the mesh the triangle's affine map is
/// extrapolated, so uv and every keyform stay consistent. With `connect`
/// the new vertex closes a triangle onto that edge; without it the vertex
/// starts loose, ready for the edge tool.
pub fn add_free_vertex(
    model: &mut Model,
    mesh: usize,
    corners: [usize; 3],
    w: [f32; 3],
    connect: Option<(u16, u16)>,
) -> Option<usize> {
    with_shapes(model, |model| {
        let m = model.art_meshes.get_mut(mesh)?;
        let n = m.vertex_count as usize;
        if n >= u16::MAX as usize - 1 {
            return None;
        }
        // any three points of the mesh work as the affine basis; they need not
        // form a face, which is what lets a mesh with no faces still grow
        if corners.iter().any(|&c| c >= n) {
            return None;
        }
        if let Some((a, b)) = connect
            && (a as usize >= n || b as usize >= n || a == b)
        {
            return None;
        }
        let mix = |pool: &[f32], base: usize| -> Option<(f32, f32)> {
            let mut x = 0.0;
            let mut y = 0.0;
            for (&c, &wc) in corners.iter().zip(w.iter()) {
                x += pool.get(base + c * 2).copied()? * wc;
                y += pool.get(base + c * 2 + 1).copied()? * wc;
            }
            Some((x, y))
        };
        let (u, v) = mix(&m.uvs, 0)?;
        let stride = n * 2;
        let keyforms = m.positions.len().checked_div(stride).unwrap_or(0);
        let mut positions = Vec::with_capacity(keyforms * (stride + 2));
        for k in 0..keyforms {
            let base = k * stride;
            let (x, y) = mix(&m.positions, base)?;
            positions.extend_from_slice(m.positions.get(base..base + stride)?);
            positions.push(x);
            positions.push(y);
        }
        m.positions = positions;
        m.uvs.push(u);
        m.uvs.push(v);
        m.vertex_count += 1;
        if let Some((a, b)) = connect {
            m.indices.extend_from_slice(&[a, b, n as u16]);
        }
        Some(n)
    })
}

/// Add a vertex placed on `reference`, the mesh as it was when Mesh Edit
/// began: weights `w` on its points `corners` give the new point's uv and
/// its position on every keyform and blend shape key, so a mesh with fewer
/// than three points (or none) can still grow, and every point lands on
/// the art where it was clicked. `connect` closes a face onto two existing
/// points. None when the reference no longer matches the mesh's keyforms.
pub fn add_vertex_from(
    model: &mut Model,
    mesh: usize,
    reference: &ayatsuri_format::model::ArtMesh,
    corners: [usize; 3],
    w: [f32; 3],
    connect: Option<(u16, u16)>,
) -> Option<usize> {
    let m = model.art_meshes.get_mut(mesh)?;
    let n = m.vertex_count as usize;
    let rn = reference.vertex_count as usize;
    if n >= u16::MAX as usize - 1 || corners.iter().any(|&c| c >= rn) {
        return None;
    }
    if let Some((a, b)) = connect
        && (a as usize >= n || b as usize >= n || a == b)
    {
        return None;
    }
    let mix = |pool: &[f32], base: usize| -> Option<(f32, f32)> {
        let mut x = 0.0;
        let mut y = 0.0;
        for (&c, &wc) in corners.iter().zip(w.iter()) {
            x += pool.get(base + c * 2).copied()? * wc;
            y += pool.get(base + c * 2 + 1).copied()? * wc;
        }
        Some((x, y))
    };
    // the point appended to each block of `count` blocks of `pool`, mixed
    // from the reference's matching blocks
    let grow = |pool: &[f32], from: &[f32], count: usize| -> Option<Vec<f32>> {
        let (stride, rstride) = (n * 2, rn * 2);
        let mut out = Vec::with_capacity(count * (stride + 2));
        for k in 0..count {
            out.extend_from_slice(pool.get(k * stride..k * stride + stride)?);
            let (x, y) = mix(from, k * rstride)?;
            out.push(x);
            out.push(y);
        }
        Some(out)
    };
    let keyforms = reference.positions.len().checked_div(rn * 2)?;
    if n > 0 && m.positions.len() != keyforms * n * 2 {
        return None;
    }
    if m.blend_shapes.len() != reference.blend_shapes.len() {
        return None;
    }
    let (u, v) = mix(&reference.uvs, 0)?;
    let positions = grow(&m.positions, &reference.positions, keyforms)?;
    let mut shapes = Vec::with_capacity(m.blend_shapes.len());
    for (s, r) in m.blend_shapes.iter().zip(&reference.blend_shapes) {
        let keys = r.positions.len().checked_div(rn * 2)?;
        if n > 0 && s.positions.len() != keys * n * 2 {
            return None;
        }
        shapes.push(grow(&s.positions, &r.positions, keys)?);
    }
    m.positions = positions;
    for (s, p) in m.blend_shapes.iter_mut().zip(shapes) {
        s.positions = p;
    }
    m.uvs.push(u);
    m.uvs.push(v);
    m.vertex_count += 1;
    if let Some((a, b)) = connect {
        m.indices.extend_from_slice(&[a, b, n as u16]);
    }
    Some(n)
}

/// Where a pasted point goes in the target mesh.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PastedPoint {
    /// It lands on this existing vertex and merges into it.
    Existing(usize),
    /// A new vertex placed from three of the mesh's points with these
    /// weights, on every keyform; its texture coordinate is read the same way.
    At([usize; 3], [f32; 3]),
}

/// Paste points into a mesh (Mesh Edit's Ctrl+V): new ones are placed as
/// `add_free_vertex` places them, then `faces` (indices into `points`) are
/// added between them, skipping any that collapse or already exist.
/// Returns each point's vertex in the mesh, or None when a placement is
/// out of range (nothing is changed then).
pub fn paste_points(
    model: &mut Model,
    mesh: usize,
    points: &[PastedPoint],
    faces: &[[usize; 3]],
) -> Option<Vec<usize>> {
    with_shapes(model, |model| {
        let n = model.art_meshes.get(mesh)?.vertex_count as usize;
        let valid = points.iter().all(|p| match *p {
            PastedPoint::Existing(v) => v < n,
            PastedPoint::At(c, _) => c.iter().all(|&x| x < n),
        });
        if !valid || n + points.len() >= u16::MAX as usize {
            return None;
        }
        let mut out = Vec::with_capacity(points.len());
        for p in points {
            let v = match *p {
                PastedPoint::Existing(v) => v,
                PastedPoint::At(c, w) => add_free_vertex(model, mesh, c, w, None)?,
            };
            out.push(v);
        }
        let m = model.art_meshes.get_mut(mesh)?;
        let mut have: std::collections::HashSet<[u16; 3]> = m
            .indices
            .as_chunks::<3>()
            .0
            .iter()
            .map(|t| {
                let mut k = *t;
                k.sort_unstable();
                k
            })
            .collect();
        for f in faces {
            let (Some(&a), Some(&b), Some(&c)) = (out.get(f[0]), out.get(f[1]), out.get(f[2]))
            else {
                continue;
            };
            if a == b || b == c || a == c {
                continue;
            }
            let tri = [a as u16, b as u16, c as u16];
            let mut key = tri;
            key.sort_unstable();
            if have.insert(key) {
                m.indices.extend_from_slice(&tri);
            }
        }
        Some(out)
    })
}

/// The vertices joined to both `a` and `b`, by a face edge or one of the
/// editor's pending lines.
fn common_neighbours(indices: &[u16], extra: &[(u16, u16)], a: u16, b: u16) -> Vec<u16> {
    let (tris, _) = indices.as_chunks::<3>();
    let mut na = Vec::new();
    let mut nb = Vec::new();
    for t in tris {
        if t.contains(&a) {
            na.extend(t.iter().copied().filter(|&v| v != a));
        }
        if t.contains(&b) {
            nb.extend(t.iter().copied().filter(|&v| v != b));
        }
    }
    for &(x, y) in extra {
        if x == a {
            na.push(y);
        } else if y == a {
            na.push(x);
        }
        if x == b {
            nb.push(y);
        } else if y == b {
            nb.push(x);
        }
    }
    let mut out: Vec<u16> =
        na.into_iter().filter(|v| nb.contains(v) && *v != a && *v != b).collect();
    out.sort_unstable();
    out.dedup();
    out
}

/// Join two vertices. A mesh edge only lives on as part of a face, so this
/// closes a triangle with the nearest common neighbour on each side of the
/// new edge (sides judged in uv space), skipping faces that already exist.
/// `pending` are lines the editor holds that have not closed a face yet;
/// they count as connections. True when at least one face was added.
pub fn add_edge(model: &mut Model, mesh: usize, a: u16, b: u16, pending: &[(u16, u16)]) -> bool {
    let Some(m) = model.art_meshes.get_mut(mesh) else { return false };
    let n = m.vertex_count as usize;
    if a == b || a as usize >= n || b as usize >= n {
        return false;
    }
    let uv = |i: u16| -> (f32, f32) {
        (
            m.uvs.get(i as usize * 2).copied().unwrap_or(0.0),
            m.uvs.get(i as usize * 2 + 1).copied().unwrap_or(0.0),
        )
    };
    let (ax, ay) = uv(a);
    let (bx, by) = uv(b);
    let (tris, _) = m.indices.as_chunks::<3>();
    let has_face = |c: u16| tris.iter().any(|t| t.contains(&a) && t.contains(&b) && t.contains(&c));
    // best candidate per side: smallest perimeter wins
    let mut best: [Option<(u16, f32)>; 2] = [None, None];
    for c in common_neighbours(&m.indices, pending, a, b) {
        if has_face(c) {
            continue;
        }
        let (cx, cy) = uv(c);
        let side = (bx - ax) * (cy - ay) - (by - ay) * (cx - ax);
        if side.abs() < 1e-12 {
            continue;
        }
        let k = usize::from(side > 0.0);
        let d = ((cx - ax).powi(2) + (cy - ay).powi(2)).sqrt()
            + ((cx - bx).powi(2) + (cy - by).powi(2)).sqrt();
        if let Some(slot) = best.get_mut(k)
            && slot.is_none_or(|(_, bd)| d < bd)
        {
            *slot = Some((c, d));
        }
    }
    let mut added = false;
    for (c, _) in best.into_iter().flatten() {
        m.indices.extend_from_slice(&[a, b, c]);
        added = true;
    }
    added
}

/// Cut the edge between two vertices: every face using both goes.
pub fn remove_edge(model: &mut Model, mesh: usize, a: u16, b: u16) -> bool {
    let Some(m) = model.art_meshes.get_mut(mesh) else { return false };
    let before = m.indices.len();
    let (tris, _) = m.indices.as_chunks::<3>();
    let kept: Vec<u16> = tris
        .iter()
        .filter(|t| !(t.contains(&a) && t.contains(&b)))
        .flat_map(|t| t.iter().copied())
        .collect();
    m.indices = kept;
    m.indices.len() != before
}

/// Whether a and b share at least one face.
pub fn has_edge(model: &Model, mesh: usize, a: u16, b: u16) -> bool {
    model.art_meshes.get(mesh).is_some_and(|m| {
        let (tris, _) = m.indices.as_chunks::<3>();
        tris.iter().any(|t| t.contains(&a) && t.contains(&b))
    })
}

/// Remove one vertex: its triangles go with it and the pools compact. Glue
/// pairs pointing at it are dropped, the rest re-indexed.
pub fn delete_vertex(model: &mut Model, mesh: usize, vertex: usize) -> bool {
    delete_vertices(model, mesh, &[vertex])
}

/// Remove a set of vertices at once (the erase brush): every face touching
/// one goes, keyform positions and uvs compact, glue pairs on removed
/// points are dropped and the rest re-indexed. Every point may go: Mesh
/// Edit places new ones from the mesh as it was on entering, and won't
/// finish until there are three points and a face again.
pub fn delete_vertices(model: &mut Model, mesh: usize, vertices: &[usize]) -> bool {
    with_shapes(model, |model| {
        let Some(m) = model.art_meshes.get_mut(mesh) else { return false };
        let n = m.vertex_count as usize;
        let mut gone = vec![false; n];
        for &v in vertices {
            if let Some(g) = gone.get_mut(v) {
                *g = true;
            }
        }
        let removed = gone.iter().filter(|&&g| g).count();
        if removed == 0 {
            return false;
        }
        // old index to new index, None for removed points
        let mut remap: Vec<Option<u16>> = Vec::with_capacity(n);
        let mut next = 0u16;
        for &g in &gone {
            if g {
                remap.push(None);
            } else {
                remap.push(Some(next));
                next += 1;
            }
        }
        let map = |i: u16| remap.get(i as usize).copied().flatten();

        let (tris, _) = m.indices.as_chunks::<3>();
        let mut indices = Vec::with_capacity(m.indices.len());
        for t in tris {
            if let (Some(a), Some(b), Some(c)) = (map(t[0]), map(t[1]), map(t[2])) {
                indices.extend_from_slice(&[a, b, c]);
            }
        }
        let stride = n * 2;
        let keyforms = m.positions.len().checked_div(stride).unwrap_or(0);
        let mut positions = Vec::with_capacity(keyforms * (n - removed) * 2);
        for k in 0..keyforms {
            let base = k * stride;
            for (vi, &g) in gone.iter().enumerate() {
                if !g {
                    positions.push(m.positions.get(base + vi * 2).copied().unwrap_or(0.0));
                    positions.push(m.positions.get(base + vi * 2 + 1).copied().unwrap_or(0.0));
                }
            }
        }
        let uvs: Vec<f32> = gone
            .iter()
            .enumerate()
            .filter(|(_, g)| !**g)
            .flat_map(|(vi, _)| {
                [
                    m.uvs.get(vi * 2).copied().unwrap_or(0.0),
                    m.uvs.get(vi * 2 + 1).copied().unwrap_or(0.0),
                ]
            })
            .collect();
        m.indices = indices;
        m.positions = positions;
        m.uvs = uvs;
        m.vertex_count = (n - removed) as u32;

        let mesh32 = mesh as u32;
        for g in &mut model.glues {
            if g.art_mesh_a != mesh32 && g.art_mesh_b != mesh32 {
                continue;
            }
            // pairs are (a, b) interleaved in weights/indices
            let on_this = |slot: usize| {
                (slot.is_multiple_of(2) && g.art_mesh_a == mesh32)
                    || (!slot.is_multiple_of(2) && g.art_mesh_b == mesh32)
            };
            let (pairs, _) = g.indices.as_chunks::<2>();
            let mut new_w = Vec::with_capacity(g.weights.len());
            let mut new_i = Vec::with_capacity(g.indices.len());
            for (pi, pair) in pairs.iter().enumerate() {
                let mut out = [0u16; 2];
                let mut keep = true;
                for (slot, &i) in pair.iter().enumerate() {
                    let v = if on_this(slot) { map(i) } else { Some(i) };
                    match (v, out.get_mut(slot)) {
                        (Some(v), Some(o)) => *o = v,
                        _ => keep = false,
                    }
                }
                if keep {
                    new_i.extend_from_slice(&out);
                    new_w.push(g.weights.get(pi * 2).copied().unwrap_or(0.0));
                    new_w.push(g.weights.get(pi * 2 + 1).copied().unwrap_or(0.0));
                }
            }
            g.weights = new_w;
            g.indices = new_i;
        }
        true
    })
}

/// `remesh` for a mesh Mesh Edit may have cleared: with no points left,
/// the mesh as it was on entering (`reference`) stands in for the old one
/// the new points are placed on.
pub fn remesh_from(
    model: &mut Model,
    mesh: usize,
    reference: Option<&ayatsuri_format::model::ArtMesh>,
    uvs: &[f32],
    indices: &[u16],
) -> bool {
    if let Some(r) = reference
        && let Some(m) = model.art_meshes.get_mut(mesh)
        && m.vertex_count == 0
        && m.blend_shapes.len() == r.blend_shapes.len()
    {
        m.vertex_count = r.vertex_count;
        m.uvs.clone_from(&r.uvs);
        m.indices.clone_from(&r.indices);
        m.positions.clone_from(&r.positions);
        for (s, rs) in m.blend_shapes.iter_mut().zip(&r.blend_shapes) {
            s.positions.clone_from(&rs.positions);
        }
    }
    remesh(model, mesh, uvs, indices)
}

/// Swap in a whole new triangulation. Every new vertex samples the old mesh
/// in uv space: barycentric inside whichever old triangle holds it, nearest
/// old vertex otherwise, so every keyform carries over. Glue on this mesh
/// cannot survive a retriangulation and is cleared.
pub fn remesh(model: &mut Model, mesh: usize, uvs: &[f32], indices: &[u16]) -> bool {
    with_shapes(model, |model| {
        let new_n = uvs.len() / 2;
        if new_n < 3
            || new_n >= u16::MAX as usize
            || indices.len() < 3
            || !indices.len().is_multiple_of(3)
        {
            return false;
        }
        if indices.iter().any(|&i| i as usize >= new_n) {
            return false;
        }
        let Some(m) = model.art_meshes.get_mut(mesh) else { return false };
        let old_n = m.vertex_count as usize;
        let stride = old_n * 2;
        if stride == 0 || m.positions.len() < stride {
            return false;
        }
        let keyforms = m.positions.len().checked_div(stride).unwrap_or(0);

        let old_uv = |i: usize| -> (f32, f32) {
            (m.uvs.get(i * 2).copied().unwrap_or(0.0), m.uvs.get(i * 2 + 1).copied().unwrap_or(0.0))
        };

        // where each new vertex sits on the old mesh
        enum Src {
            Tri([usize; 3], [f32; 3]),
            Vert(usize),
        }
        let mut sources = Vec::with_capacity(new_n);
        for vi in 0..new_n {
            let u = uvs.get(vi * 2).copied().unwrap_or(0.0);
            let v = uvs.get(vi * 2 + 1).copied().unwrap_or(0.0);
            let mut found = None;
            // outside every old face: the nearest face carried on past its
            // edge (exact for the imported quad, so a margin beyond the art
            // keeps its place instead of piling onto the old corners)
            let mut nearest: Option<([usize; 3], [f32; 3], f32)> = None;
            let (tris, _) = m.indices.as_chunks::<3>();
            for &[ta, tb, tc] in tris {
                let (a, b, c) = (ta as usize, tb as usize, tc as usize);
                let (x0, y0) = old_uv(a);
                let (x1, y1) = old_uv(b);
                let (x2, y2) = old_uv(c);
                let den = (y1 - y2) * (x0 - x2) + (x2 - x1) * (y0 - y2);
                if den.abs() < 1e-12 {
                    continue;
                }
                let w0 = ((y1 - y2) * (u - x2) + (x2 - x1) * (v - y2)) / den;
                let w1 = ((y2 - y0) * (u - x2) + (x0 - x2) * (v - y2)) / den;
                let w2 = 1.0 - w0 - w1;
                if w0 >= -0.02 && w1 >= -0.02 && w2 >= -0.02 {
                    found = Some(Src::Tri([a, b, c], [w0, w1, w2]));
                    break;
                }
                let worst = w0.min(w1).min(w2);
                if nearest.is_none_or(|n| worst > n.2) {
                    nearest = Some(([a, b, c], [w0, w1, w2], worst));
                }
            }
            let found = found.or_else(|| nearest.map(|(t, w, _)| Src::Tri(t, w)));
            let src = found.unwrap_or_else(|| {
                let mut best = (0usize, f32::MAX);
                for ovi in 0..old_n {
                    let (ou, ov) = old_uv(ovi);
                    let d = (ou - u) * (ou - u) + (ov - v) * (ov - v);
                    if d < best.1 {
                        best = (ovi, d);
                    }
                }
                Src::Vert(best.0)
            });
            sources.push(src);
        }

        let mut positions = Vec::with_capacity(keyforms * new_n * 2);
        for k in 0..keyforms {
            let base = k * stride;
            for src in &sources {
                let (x, y) = match src {
                    Src::Vert(vi) => (
                        m.positions.get(base + vi * 2).copied().unwrap_or(0.0),
                        m.positions.get(base + vi * 2 + 1).copied().unwrap_or(0.0),
                    ),
                    Src::Tri(c, w) => {
                        let mut x = 0.0;
                        let mut y = 0.0;
                        for (&vi, &wc) in c.iter().zip(w.iter()) {
                            x += m.positions.get(base + vi * 2).copied().unwrap_or(0.0) * wc;
                            y += m.positions.get(base + vi * 2 + 1).copied().unwrap_or(0.0) * wc;
                        }
                        (x, y)
                    }
                };
                positions.push(x);
                positions.push(y);
            }
        }

        m.positions = positions;
        m.uvs = uvs.to_vec();
        m.indices = indices.to_vec();
        m.vertex_count = new_n as u32;

        let mesh32 = mesh as u32;
        for g in &mut model.glues {
            if g.art_mesh_a == mesh32 || g.art_mesh_b == mesh32 {
                g.weights.clear();
                g.indices.clear();
            }
        }
        true
    })
}

/// Cubism's four corner forms: on a two-axis binding, each diagonal corner
/// keyform becomes "x extreme + y extreme - centre" so the two movements
/// combine instead of needing hand-made corners. Runs on positions only.
pub fn four_corner_forms(model: &mut Model, target: Target) -> bool {
    let (binding, stride, positions): (Binding, usize, &mut Vec<f32>) = match target {
        Target::ArtMesh(i) => {
            let Some(m) = model.art_meshes.get_mut(i) else { return false };
            (m.binding.clone(), (m.vertex_count as usize) * 2, &mut m.positions)
        }
        Target::Deformer(i) => {
            let Some(d) = model.deformers.get_mut(i) else { return false };
            let binding = d.binding.clone();
            match &mut d.kind {
                DeformerKind::Warp(w) => {
                    let pts = w.grid_points() * 2;
                    (binding, pts, &mut w.positions)
                }
                DeformerKind::Rotation(_) => return false,
            }
        }
        Target::Part(_) => return false,
    };
    if binding.axes.len() != 2 || stride == 0 {
        return false;
    }
    let (nx, ny) = (
        binding.axes.first().map(|a| a.keys.len()).unwrap_or(0),
        binding.axes.get(1).map(|a| a.keys.len()).unwrap_or(0),
    );
    if nx < 2 || ny < 2 || positions.len() < nx * ny * stride {
        return false;
    }
    // the centre key is the one nearest the parameter default
    let centre = |axis: &BindingAxis| -> usize {
        let default =
            model.parameters.get(axis.parameter as usize).map(|p| p.default).unwrap_or(0.0);
        let mut best = (0usize, f32::MAX);
        for (i, &k) in axis.keys.iter().enumerate() {
            let d = (k - default).abs();
            if d < best.1 {
                best = (i, d);
            }
        }
        best.0
    };
    let (cx, cy) = (
        binding.axes.first().map(centre).unwrap_or(0),
        binding.axes.get(1).map(centre).unwrap_or(0),
    );

    let form = |ix: usize, iy: usize| (iy * nx + ix) * stride;
    let mut out = positions.clone();
    for iy in 0..ny {
        for ix in 0..nx {
            if ix == cx || iy == cy {
                continue;
            }
            let (fx, fy, fc, dst) = (form(ix, cy), form(cx, iy), form(cx, cy), form(ix, iy));
            for e in 0..stride {
                let x = positions.get(fx + e).copied().unwrap_or(0.0);
                let y = positions.get(fy + e).copied().unwrap_or(0.0);
                let c = positions.get(fc + e).copied().unwrap_or(0.0);
                if let Some(slot) = out.get_mut(dst + e) {
                    *slot = x + y - c;
                }
            }
        }
    }
    *positions = out;
    true
}

/// Shift one warp grid control point by (dx, dy) in the deformer's parent
/// space, across every keyform, mirroring how mesh vertices move.
pub fn move_warp_point(model: &mut Model, deformer: usize, point: usize, dx: f32, dy: f32) -> bool {
    let Some(d) = model.deformers.get_mut(deformer) else { return false };
    let DeformerKind::Warp(w) = &mut d.kind else { return false };
    let stride = w.grid_points() * 2;
    if stride == 0 || point * 2 + 1 >= stride {
        return false;
    }
    let at = point * 2;
    let mut touched = false;
    for form in w.positions.chunks_exact_mut(stride) {
        if let Some(x) = form.get_mut(at) {
            *x += dx;
        }
        if let Some(y) = form.get_mut(at + 1) {
            *y += dy;
            touched = true;
        }
    }
    touched
}

/// Move mesh vertices, each by its own parent-space offset, in one keyform
/// (form editing with the arrow tool) or in all of them (None).
pub fn move_vertices_in(
    model: &mut Model,
    mesh: usize,
    keyform: Option<usize>,
    moves: &[(usize, f32, f32)],
) -> bool {
    let Some(k) = keyform else { return move_vertices(model, mesh, moves) };
    let Some(m) = model.art_meshes.get_mut(mesh) else { return false };
    let count = m.vertex_count as usize;
    let Some(form) = crate::keyform::mesh_form_mut(m, k) else { return false };
    let mut touched = false;
    for &(vertex, dx, dy) in moves {
        if vertex >= count {
            continue;
        }
        if let Some(x) = form.get_mut(vertex * 2) {
            *x += dx;
        }
        if let Some(y) = form.get_mut(vertex * 2 + 1) {
            *y += dy;
            touched = true;
        }
    }
    touched
}

/// Move warp grid points, each by its own parent-space offset, in one
/// keyform (form editing with the arrow tool) or in all of them (None).
pub fn move_warp_points(
    model: &mut Model,
    deformer: usize,
    keyform: Option<usize>,
    moves: &[(usize, f32, f32)],
) -> bool {
    let Some(d) = model.deformers.get_mut(deformer) else { return false };
    let DeformerKind::Warp(w) = &mut d.kind else { return false };
    let count = w.grid_points();
    let stride = count * 2;
    if stride == 0 {
        return false;
    }
    let mut touched = false;
    for (k, form) in w.positions.chunks_exact_mut(stride).enumerate() {
        if keyform.is_some_and(|want| want != k) {
            continue;
        }
        for &(p, dx, dy) in moves {
            if p >= count {
                continue;
            }
            if let Some(x) = form.get_mut(p * 2) {
                *x += dx;
            }
            if let Some(y) = form.get_mut(p * 2 + 1) {
                *y += dy;
                touched = true;
            }
        }
    }
    touched
}

/// Edit Form > Flip: mirror a mesh or warp across its own centre, every
/// keyform, horizontally or vertically. A warp's children flip with it.
/// Rotation deformers and parts are refused (false).
pub fn flip_object(model: &mut Model, target: Target, horizontal: bool) -> bool {
    let pos: &mut Vec<f32> = match target {
        Target::ArtMesh(i) => match model.art_meshes.get_mut(i) {
            Some(m) => &mut m.positions,
            None => return false,
        },
        Target::Deformer(i) => match model.deformers.get_mut(i).map(|d| &mut d.kind) {
            Some(DeformerKind::Warp(w)) => &mut w.positions,
            _ => return false,
        },
        Target::Part(_) => return false,
    };
    let axis = if horizontal { 0 } else { 1 };
    let (lo, hi) = pos
        .iter()
        .skip(axis)
        .step_by(2)
        .fold((f32::MAX, f32::MIN), |(a, b), &v| (a.min(v), b.max(v)));
    if lo > hi {
        return false;
    }
    let mid2 = lo + hi;
    for v in pos.iter_mut().skip(axis).step_by(2) {
        *v = mid2 - *v;
    }
    true
}

/// Slide a rotation deformer's origin by (dx, dy) in parent space, across
/// every keyform.
pub fn move_rotation_origin(model: &mut Model, deformer: usize, dx: f32, dy: f32) -> bool {
    let Some(d) = model.deformers.get_mut(deformer) else { return false };
    let DeformerKind::Rotation(r) = &mut d.kind else { return false };
    for x in &mut r.origins_x {
        *x += dx;
    }
    for y in &mut r.origins_y {
        *y += dy;
    }
    !r.origins_x.is_empty()
}

/// Move a rotation deformer's pivot without moving what it holds: keyform
/// k's origin shifts by `origin_deltas[k]` (parent space) while every child
/// shifts by `child` (the deformer's local space). The caller picks them so
/// the pivot stays on the same spot of the mesh in every keyform; see the
/// editor's origin drag.
pub fn move_rotation_pivot(
    model: &mut Model,
    deformer: usize,
    origin_deltas: &[(f32, f32)],
    child: (f32, f32),
) -> bool {
    let Some(d) = model.deformers.get_mut(deformer) else { return false };
    let DeformerKind::Rotation(r) = &mut d.kind else { return false };
    if origin_deltas.len() != r.origins_x.len() || r.origins_x.len() != r.origins_y.len() {
        return false;
    }
    for ((x, y), (dx, dy)) in r.origins_x.iter_mut().zip(r.origins_y.iter_mut()).zip(origin_deltas)
    {
        *x += dx;
        *y += dy;
    }
    let shift = |x: f32, y: f32| (x + child.0, y + child.1);
    let kids: Vec<Child> = model
        .art_meshes
        .iter()
        .enumerate()
        .filter(|(_, m)| m.parent_deformer == Some(deformer as u32))
        .map(|(i, _)| Child::Mesh(i))
        .chain(
            model
                .deformers
                .iter()
                .enumerate()
                .filter(|(_, d)| d.parent_deformer == Some(deformer as u32))
                .map(|(i, _)| Child::Deformer(i)),
        )
        .collect();
    for c in kids {
        map_child(model, c, &shift);
    }
    true
}

/// Turn a rotation deformer by `degrees` on every keyform.
pub fn rotate_rotation(model: &mut Model, deformer: usize, degrees: f32) -> bool {
    let Some(d) = model.deformers.get_mut(deformer) else { return false };
    let DeformerKind::Rotation(r) = &mut d.kind else { return false };
    for a in &mut r.angles {
        *a += degrees;
    }
    !r.angles.is_empty()
}

/// A brand new parameter. It appends, so no index anywhere shifts.
pub fn create_parameter(
    model: &mut Model,
    id: String,
    min: f32,
    default: f32,
    max: f32,
) -> Option<usize> {
    if id.is_empty() || model.parameters.iter().any(|p| p.id == id) {
        return None;
    }
    let (min, max) = if min <= max { (min, max) } else { (max, min) };
    model.parameters.push(ayatsuri_format::model::Parameter {
        id,
        min,
        max,
        default: default.clamp(min, max),
        repeat: false,
        decimals: 3,
        blend_shape: false,
    });
    Some(model.parameters.len() - 1)
}

/// Remove a parameter nothing uses. Any binding, glue or physics line that
/// still points at it blocks the delete; axes above it shift down by one.
pub fn delete_parameter(model: &mut Model, param: usize) -> bool {
    let Some(meta) = model.parameters.get(param) else { return false };
    let id = meta.id.clone();
    let p32 = param as u32;
    let uses = |b: &Binding| b.axes.iter().any(|a| a.parameter == p32);
    let shaped = |s: &[BlendShape]| s.iter().any(|s| s.parameter == p32);
    let used = model.parts.iter().any(|p| uses(&p.binding))
        || model.deformers.iter().any(|d| uses(&d.binding))
        || model.art_meshes.iter().any(|m| uses(&m.binding) || shaped(&m.blend_shapes))
        || model.glues.iter().any(|g| uses(&g.binding))
        || model.physics.as_ref().is_some_and(|rig| {
            rig.groups.iter().any(|g| {
                g.inputs.iter().any(|i| i.param == id) || g.outputs.iter().any(|o| o.param == id)
            })
        });
    if used {
        return false;
    }
    model.parameters.remove(param);
    let shift = |b: &mut Binding| {
        for a in &mut b.axes {
            if a.parameter > p32 {
                a.parameter -= 1;
            }
        }
    };
    for p in &mut model.parts {
        shift(&mut p.binding);
    }
    for d in &mut model.deformers {
        shift(&mut d.binding);
    }
    for m in &mut model.art_meshes {
        shift(&mut m.binding);
        for s in &mut m.blend_shapes {
            if s.parameter > p32 {
                s.parameter -= 1;
            }
        }
    }
    for g in &mut model.glues {
        shift(&mut g.binding);
    }
    true
}

/// What still uses a parameter: (objects bound on it, physics inputs and
/// outputs that name it).
pub fn param_users(model: &Model, param: usize) -> (usize, usize) {
    let p32 = param as u32;
    let uses = |b: &Binding| b.axes.iter().any(|a| a.parameter == p32);
    let objects = model.parts.iter().filter(|p| uses(&p.binding)).count()
        + model.deformers.iter().filter(|d| uses(&d.binding)).count()
        + model
            .art_meshes
            .iter()
            .filter(|m| uses(&m.binding) || m.blend_shapes.iter().any(|s| s.parameter == p32))
            .count()
        + model.glues.iter().filter(|g| uses(&g.binding)).count();
    let id = model.parameters.get(param).map(|p| p.id.as_str()).unwrap_or_default();
    let physics = model.physics.as_ref().map_or(0, |rig| {
        rig.groups
            .iter()
            .map(|g| {
                g.inputs.iter().filter(|i| i.param == id).count()
                    + g.outputs.iter().filter(|o| o.param == id).count()
            })
            .sum()
    });
    (objects, physics)
}

/// Take a parameter out of everything that uses it, so it can be deleted:
/// every object keeps the form it has at the parameter's default (see
/// `delete_binding`), physics inputs and outputs naming it are dropped, and
/// a physics group left with no input or no output goes too, as it could no
/// longer do anything. True when anything changed.
pub fn unbind_parameter_everywhere(model: &mut Model, param: usize) -> bool {
    let Some(meta) = model.parameters.get(param) else { return false };
    let (id, default) = (meta.id.clone(), meta.default);
    let p32 = param as u32;
    let uses = |b: &Binding| b.axes.iter().any(|a| a.parameter == p32);
    let mut targets: Vec<Target> = Vec::new();
    targets.extend(
        model
            .parts
            .iter()
            .enumerate()
            .filter(|(_, p)| uses(&p.binding))
            .map(|(i, _)| Target::Part(i)),
    );
    targets.extend(
        model
            .deformers
            .iter()
            .enumerate()
            .filter(|(_, d)| uses(&d.binding))
            .map(|(i, _)| Target::Deformer(i)),
    );
    targets.extend(
        model
            .art_meshes
            .iter()
            .enumerate()
            .filter(|(_, m)| uses(&m.binding) || m.blend_shapes.iter().any(|s| s.parameter == p32))
            .map(|(i, _)| Target::ArtMesh(i)),
    );
    let mut changed = false;
    for t in targets {
        changed |= delete_binding(model, t, param, default);
    }
    // glue intensity keyforms: keep the one nearest the default
    for g in &mut model.glues {
        if let Some(at) = g.binding.axes.iter().position(|a| a.parameter == p32) {
            let keep = g.binding.axes.get(at).map_or(0, |a| {
                a.keys
                    .iter()
                    .enumerate()
                    .min_by(|x, y| (x.1 - default).abs().total_cmp(&(y.1 - default).abs()))
                    .map_or(0, |(i, _)| i)
            });
            let value = g.intensities.get(keep).copied().unwrap_or(1.0);
            g.binding.axes.remove(at);
            let n = g.binding.axes.iter().map(|a| a.keys.len().max(1)).product::<usize>().max(1);
            g.intensities = vec![value; n];
            changed = true;
        }
    }
    if let Some(rig) = &mut model.physics {
        for g in &mut rig.groups {
            let before = g.inputs.len() + g.outputs.len();
            g.inputs.retain(|i| i.param != id);
            g.outputs.retain(|o| o.param != id);
            changed |= g.inputs.len() + g.outputs.len() != before;
        }
        let before = rig.groups.len();
        rig.groups.retain(|g| !g.inputs.is_empty() && !g.outputs.is_empty());
        changed |= rig.groups.len() != before;
    }
    changed
}

/// Cubism's Auto connect: triangulate the mesh's points (in uv space, so
/// the result does not depend on the pose), keeping every line drawn in
/// `lines` as an edge. With `rebuild` every face is redone from the points
/// and lines; without it the faces already there stay, their edges kept too,
/// and only the gaps fill. `opaque` (the texture's alpha at a uv) turns on
/// Protect edge exterior: when the lines close off a perimeter nothing goes
/// outside it, and otherwise a face goes when it holds no art, or little
/// art with an edge stretched across a gap in it. Returns the faces added.
pub fn auto_connect(
    model: &mut Model,
    mesh: usize,
    rebuild: bool,
    lines: &[(u16, u16)],
    opaque: Option<&dyn Fn(f32, f32) -> bool>,
) -> usize {
    let Some(m) = model.art_meshes.get_mut(mesh) else { return 0 };
    let (uvp, _) = m.uvs.as_chunks::<2>();
    let pts: Vec<(f32, f32)> = uvp.iter().map(|&[u, v]| (u, v)).collect();
    let existing: Vec<[u16; 3]> = if rebuild {
        Vec::new()
    } else {
        let (t, _) = m.indices.as_chunks::<3>();
        t.to_vec()
    };
    let mut forced: Vec<(usize, usize)> =
        lines.iter().map(|&(a, b)| (a as usize, b as usize)).collect();
    for t in &existing {
        for (a, b) in [(t[0], t[1]), (t[1], t[2]), (t[2], t[0])] {
            forced.push((a as usize, b as usize));
        }
    }
    let fresh = crate::automesh::constrained(&pts, &forced);
    let p = |i: usize| pts.get(i).copied().unwrap_or((0.0, 0.0));
    let mut drop = vec![false; fresh.len()];
    if let Some(opaque) = opaque {
        // a drawn perimeter is the outline to keep (Cubism asks for one)
        let fenced = !lines.is_empty() && {
            let out = crate::automesh::outside(&fresh, &forced);
            let any_in = out.iter().any(|o| !o);
            if any_in {
                drop = out;
            }
            any_in
        };
        if !fenced {
            // no perimeter: the art decides. An edge whose middle runs over
            // empty texture between art on both ends bridges a gap (the
            // eyelash chord over the eye); the allowance is most of a
            // typical edge, so outline edges just off the art still pass
            let mut lens: Vec<f32> = fresh
                .iter()
                .flat_map(|t| [(t[0], t[1]), (t[1], t[2]), (t[2], t[0])])
                .map(|(a, b)| ((p(a).0 - p(b).0).powi(2) + (p(a).1 - p(b).1).powi(2)).sqrt())
                .collect();
            lens.sort_by(f32::total_cmp);
            let allowance = lens.get(lens.len() / 2).copied().unwrap_or(0.0) * 0.75;
            let gap = |a: (f32, f32), b: (f32, f32)| {
                const STEPS: usize = 48;
                let solid: Vec<bool> = (0..=STEPS)
                    .map(|k| {
                        let f = k as f32 / STEPS as f32;
                        opaque(a.0 + (b.0 - a.0) * f, a.1 + (b.1 - a.1) * f)
                    })
                    .collect();
                let (Some(first), Some(last)) =
                    (solid.iter().position(|&o| o), solid.iter().rposition(|&o| o))
                else {
                    return 0.0;
                };
                let mut longest = 0usize;
                let mut run = 0usize;
                for &o in solid.get(first..=last).unwrap_or(&[]) {
                    run = if o { 0 } else { run + 1 };
                    longest = longest.max(run);
                }
                let len = ((a.0 - b.0).powi(2) + (a.1 - b.1).powi(2)).sqrt();
                len * longest as f32 / STEPS as f32
            };
            // how much of a triangle is art, sampled on a barycentric grid
            let coverage = |a: (f32, f32), b: (f32, f32), c: (f32, f32)| {
                const N: usize = 12;
                let (mut hit, mut all) = (0usize, 0usize);
                for i in 0..=N {
                    for j in 0..=N - i {
                        let (wa, wb) = (i as f32 / N as f32, j as f32 / N as f32);
                        let wc = 1.0 - wa - wb;
                        all += 1;
                        if opaque(a.0 * wa + b.0 * wb + c.0 * wc, a.1 * wa + b.1 * wb + c.1 * wc) {
                            hit += 1;
                        }
                    }
                }
                hit as f32 / all.max(1) as f32
            };
            // a face holding art stays, so no art is ever cut off; only one
            // on empty texture goes, or a mostly empty one bridging a gap
            for (t, d) in fresh.iter().zip(drop.iter_mut()) {
                let (a, b, c) = (p(t[0]), p(t[1]), p(t[2]));
                let art = coverage(a, b, c);
                *d = art == 0.0
                    || (art < 0.35
                        && [(a, b), (b, c), (c, a)].iter().any(|&(x, y)| gap(x, y) > allowance));
            }
        }
    }
    // inside a kept face: with its edges forced in, a centroid inside means
    // the new triangle lies within it
    let in_face = |x: f32, y: f32| {
        existing.iter().any(|t| {
            let (a, b, c) = (p(t[0] as usize), p(t[1] as usize), p(t[2] as usize));
            let s =
                |p: (f32, f32), q: (f32, f32)| (q.0 - p.0) * (y - p.1) - (q.1 - p.1) * (x - p.0);
            let (s1, s2, s3) = (s(a, b), s(b, c), s(c, a));
            (s1 >= 0.0 && s2 >= 0.0 && s3 >= 0.0) || (s1 <= 0.0 && s2 <= 0.0 && s3 <= 0.0)
        })
    };
    let mut indices: Vec<u16> = existing.iter().flat_map(|t| t.iter().copied()).collect();
    let mut added = 0;
    for (t, d) in fresh.iter().zip(&drop) {
        if *d {
            continue;
        }
        let (a, b, c) = (p(t[0]), p(t[1]), p(t[2]));
        if !existing.is_empty() && in_face((a.0 + b.0 + c.0) / 3.0, (a.1 + b.1 + c.1) / 3.0) {
            continue;
        }
        indices.extend_from_slice(&[t[0] as u16, t[1] as u16, t[2] as u16]);
        added += 1;
    }
    m.indices = indices;
    added
}

/// Cubism's Mesh 4 Division: every face splits into four through its edge
/// midpoints, shape untouched. New points append, so existing indices (and
/// any glue on them) stay valid.
pub fn subdivide4(model: &mut Model, mesh: usize) -> bool {
    with_shapes(model, |model| {
        let Some(m) = model.art_meshes.get_mut(mesh) else { return false };
        let n = m.vertex_count as usize;
        let stride = n * 2;
        let keyforms = m.positions.len().checked_div(stride).unwrap_or(0);
        let (tris, _) = m.indices.as_chunks::<3>();
        let tris = tris.to_vec();
        if tris.is_empty() {
            return false;
        }
        let mut mid_of: std::collections::HashMap<(u16, u16), u16> =
            std::collections::HashMap::new();
        let mut mids: Vec<(u16, u16)> = Vec::new();
        for t in &tris {
            for (a, b) in [(t[0], t[1]), (t[1], t[2]), (t[2], t[0])] {
                let key = (a.min(b), a.max(b));
                if let std::collections::hash_map::Entry::Vacant(slot) = mid_of.entry(key) {
                    let id = n + mids.len();
                    if id >= u16::MAX as usize {
                        return false;
                    }
                    slot.insert(id as u16);
                    mids.push(key);
                }
            }
        }
        let half = |pool: &[f32], base: usize, (a, b): (u16, u16)| {
            let g = |i: u16, k: usize| pool.get(base + i as usize * 2 + k).copied().unwrap_or(0.0);
            [(g(a, 0) + g(b, 0)) * 0.5, (g(a, 1) + g(b, 1)) * 0.5]
        };
        let new_n = n + mids.len();
        let mut positions = Vec::with_capacity(keyforms * new_n * 2);
        for k in 0..keyforms {
            let base = k * stride;
            positions.extend_from_slice(m.positions.get(base..base + stride).unwrap_or(&[]));
            for &e in &mids {
                positions.extend_from_slice(&half(&m.positions, base, e));
            }
        }
        let mut uvs = m.uvs.clone();
        for &e in &mids {
            let h = half(&m.uvs, 0, e);
            uvs.extend_from_slice(&h);
        }
        let mid = |a: u16, b: u16| mid_of.get(&(a.min(b), a.max(b))).copied().unwrap_or(a);
        let mut indices = Vec::with_capacity(tris.len() * 12);
        for t in &tris {
            let (a, b, c) = (t[0], t[1], t[2]);
            let (ab, bc, ca) = (mid(a, b), mid(b, c), mid(c, a));
            indices.extend_from_slice(&[a, ab, ca, ab, b, bc, ca, bc, c, ab, bc, ca]);
        }
        m.positions = positions;
        m.uvs = uvs;
        m.indices = indices;
        m.vertex_count = new_n as u32;
        true
    })
}

/// Bind two meshes with glue, Cubism style: vertex pairs that overlap (in
/// the given world positions, within `tolerance` model units) are attached,
/// nearest pairs first, each vertex used once, weights starting at an even
/// 50:50. Binding the same two meshes again adds the new pairs to the glue
/// they already share. Returns the glue's index and the pairs added.
pub fn bind_glue(
    model: &mut Model,
    a: usize,
    b: usize,
    world_a: &[f32],
    world_b: &[f32],
    tolerance: f32,
) -> Option<(usize, usize)> {
    if a == b || a >= model.art_meshes.len() || b >= model.art_meshes.len() {
        return None;
    }
    let (pa, _) = world_a.as_chunks::<2>();
    let (pb, _) = world_b.as_chunks::<2>();
    let t2 = tolerance * tolerance;
    let mut candidates: Vec<(f32, u16, u16)> = Vec::new();
    for (i, p) in pa.iter().enumerate() {
        for (j, q) in pb.iter().enumerate() {
            let d = (p[0] - q[0]).powi(2) + (p[1] - q[1]).powi(2);
            if d <= t2 {
                candidates.push((d, i as u16, j as u16));
            }
        }
    }
    candidates.sort_by(|x, y| x.0.total_cmp(&y.0));
    // an existing glue between these two, in either order, gets the pairs
    let existing = model.glues.iter().position(|g| {
        (g.art_mesh_a as usize, g.art_mesh_b as usize) == (a, b)
            || (g.art_mesh_a as usize, g.art_mesh_b as usize) == (b, a)
    });
    let flipped =
        existing.and_then(|gi| model.glues.get(gi)).is_some_and(|g| g.art_mesh_a as usize == b);
    let (mut used_a, mut used_b): (Vec<u16>, Vec<u16>) = existing
        .and_then(|gi| model.glues.get(gi))
        .map(|g| {
            let (pairs, _) = g.indices.as_chunks::<2>();
            if flipped {
                (pairs.iter().map(|p| p[1]).collect(), pairs.iter().map(|p| p[0]).collect())
            } else {
                (pairs.iter().map(|p| p[0]).collect(), pairs.iter().map(|p| p[1]).collect())
            }
        })
        .unwrap_or_default();
    let mut fresh: Vec<(u16, u16)> = Vec::new();
    for (_, i, j) in candidates {
        if used_a.contains(&i) || used_b.contains(&j) {
            continue;
        }
        used_a.push(i);
        used_b.push(j);
        fresh.push((i, j));
    }
    if fresh.is_empty() {
        return None;
    }
    let added = fresh.len();
    let gi = match existing {
        Some(gi) => gi,
        None => {
            model.glues.push(ayatsuri_format::model::Glue {
                id: format!("Glue{}", model.glues.len()),
                art_mesh_a: a as u32,
                art_mesh_b: b as u32,
                binding: Binding::default(),
                intensities: vec![1.0],
                weights: Vec::new(),
                indices: Vec::new(),
            });
            model.glues.len() - 1
        }
    };
    let g = model.glues.get_mut(gi)?;
    for (i, j) in fresh {
        let (x, y) = if flipped { (j, i) } else { (i, j) };
        g.indices.extend_from_slice(&[x, y]);
        g.weights.extend_from_slice(&[0.5, 0.5]);
    }
    Some((gi, added))
}

/// Remove a glue outright.
pub fn unbind_glue(model: &mut Model, glue: usize) -> bool {
    if glue < model.glues.len() {
        model.glues.remove(glue);
        true
    } else {
        false
    }
}

/// How a glue weight brush stroke changes a pair. Weights are pulls: how
/// far each side moves toward the other, so a side's Cubism "priority" is
/// one minus its pull.
#[derive(Clone, Copy, Debug)]
pub enum GlueBrush {
    /// Shift the balance, total kept at 100%: toward A's priority, or B's.
    Balance {
        favor_a: bool,
    },
    /// Strengthen only A (or only B): its point lets go of the other.
    OnlyA,
    OnlyB,
    /// Bring a separated pair back to a full 100% total.
    Remerge,
}

/// Apply a brush to some of a glue's pairs: (pair index, strength 0..1).
pub fn glue_brush(model: &mut Model, glue: usize, hits: &[(usize, f32)], brush: GlueBrush) -> bool {
    let Some(g) = model.glues.get_mut(glue) else { return false };
    let mut any = false;
    for &(pair, k) in hits {
        let k = k.clamp(0.0, 1.0);
        let (Some(&w0), Some(&w1)) = (g.weights.get(pair * 2), g.weights.get(pair * 2 + 1)) else {
            continue;
        };
        let (n0, n1) = match brush {
            GlueBrush::Balance { favor_a } => {
                let t = if favor_a { (w0 - k).max(0.0) } else { (w0 + k).min(1.0) };
                (t, 1.0 - t)
            }
            GlueBrush::OnlyA => ((w0 - k).max(0.0), w1),
            GlueBrush::OnlyB => (w0, (w1 - k).max(0.0)),
            GlueBrush::Remerge => {
                let missing = (1.0 - w0 - w1).max(0.0) * k;
                let total = (w0 + w1).max(1e-6);
                (
                    (w0 + missing * if total > 1e-5 { w0 / total } else { 0.5 }).min(1.0),
                    (w1 + missing * if total > 1e-5 { w1 / total } else { 0.5 }).min(1.0),
                )
            }
        };
        if let Some(s) = g.weights.get_mut(pair * 2) {
            *s = n0;
        }
        if let Some(s) = g.weights.get_mut(pair * 2 + 1) {
            *s = n1;
        }
        any = true;
    }
    any
}

/// Cubism's glue compatibility, set on every keyform (a keyed glue still
/// varies by parameter afterwards through its own keyforms).
pub fn set_glue_compatibility(model: &mut Model, glue: usize, value: f32) -> bool {
    let Some(g) = model.glues.get_mut(glue) else { return false };
    for v in &mut g.intensities {
        *v = value.clamp(0.0, 1.0);
    }
    true
}

fn alloc_one() -> Vec<f32> {
    vec![1.0]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Puppet;
    use ayatsuri_format::model::{ArtMesh, Canvas, CurveKind, Parameter};

    fn square_path() -> ayatsuri_format::model::ArtPath {
        ayatsuri_format::model::ArtPath {
            anchors: vec![
                [10.0, 10.0, 0.0, 0.0, 0.0, 0.0],
                [30.0, 10.0, 0.0, 0.0, 0.0, 0.0],
                [30.0, 30.0, 0.0, 0.0, 0.0, 0.0],
                [10.0, 30.0, 0.0, 0.0, 0.0, 0.0],
            ],
            closed: true,
            fill: true,
            step: 1000.0,
            ..Default::default()
        }
    }

    #[test]
    fn an_art_path_becomes_a_mesh_and_goes_with_it() {
        let mut m = test_model();
        let before = m.art_meshes.len();
        let mi = create_art_path(&mut m, square_path(), None, 3).expect("drawn");
        assert_eq!(mi, before);
        let mesh = &m.art_meshes[mi];
        assert_eq!(mesh.texture, 3);
        assert_eq!(mesh.positions.len(), mesh.uvs.len());
        assert!(mesh.uvs.iter().all(|u| (0.0..=1.0).contains(u)));
        assert_eq!(m.art_path_colors.len(), 2);
        assert_eq!(art_path_of(&m, mi).map(|p| p.mesh.as_str()), Some(mesh.id.as_str()));
        assert!(delete_art_meshes(&mut m, &[mi]));
        assert!(m.art_paths.is_empty());
    }

    #[test]
    fn a_mask_only_path_draws_at_zero_and_comes_back() {
        let mut m = test_model();
        let mi = create_art_path(&mut m, square_path(), None, 0).expect("drawn");
        let hidden = ayatsuri_format::model::ArtPath { mask_only: true, ..square_path() };
        assert!(update_art_path(&mut m, mi, hidden));
        assert!(m.art_meshes[mi].opacities.iter().all(|o| *o == 0.0));
        assert!(!m.art_meshes[mi].indices.is_empty());
        assert!(update_art_path(&mut m, mi, square_path()));
        assert!(m.art_meshes[mi].opacities.iter().all(|o| *o == 1.0));
    }

    #[test]
    fn redrawing_an_art_path_keeps_its_keyform_count() {
        let mut m = test_model();
        let mi = create_art_path(&mut m, square_path(), None, 0).expect("drawn");
        assert!(add_keys(&mut m, Target::ArtMesh(mi), 0, &[0.0, 1.0]));
        let mut wider = square_path();
        if let Some(a) = wider.anchors.get_mut(1) {
            a[0] = 60.0;
        }
        wider.stroke = false;
        assert!(update_art_path(&mut m, mi, wider));
        let mesh = &m.art_meshes[mi];
        assert_eq!(mesh.binding.keyform_count(), 2);
        assert_eq!(mesh.positions.len(), 2 * mesh.vertex_count as usize * 2);
        let max_x = mesh.positions.iter().step_by(2).fold(f32::MIN, |a, &b| a.max(b));
        assert!(max_x > 59.0, "the moved corner shows: {max_x}");
    }

    fn test_model() -> Model {
        Model {
            canvas: Canvas { pixels_per_unit: 1.0, ..Canvas::default() },
            parameters: vec![
                Parameter {
                    id: "A".into(),
                    min: 0.0,
                    max: 1.0,
                    default: 0.0,
                    repeat: false,
                    decimals: 3,
                    blend_shape: false,
                },
                Parameter {
                    id: "B".into(),
                    min: 0.0,
                    max: 1.0,
                    default: 0.0,
                    repeat: false,
                    decimals: 3,
                    blend_shape: false,
                },
            ],
            parts: vec![],
            deformers: vec![],
            art_meshes: vec![ArtMesh {
                id: "M".into(),
                parent_part: None,
                parent_deformer: None,
                visible: true,
                enabled: true,
                texture: 0,
                flags: 0,
                blend: Default::default(),
                vertex_count: 1,
                uvs: vec![0.0, 0.0],
                indices: vec![0, 0, 0],
                masks: vec![],
                binding: Binding {
                    curves: Vec::new(),
                    axes: vec![BindingAxis { parameter: 0, keys: vec![0.0, 1.0] }],
                },
                opacities: vec![1.0, 1.0],
                draw_orders: vec![500.0, 500.0],
                // the single vertex slides from x=0 to x=2
                positions: vec![0.0, 0.0, 2.0, 0.0],
                blend_shapes: Vec::new(),
                clip_group: None,
                clip_inherit: false,
            }],
            glues: vec![],
            draw_groups: vec![],
            physics: None,
            art_paths: Vec::new(),
            art_path_colors: Vec::new(),
            art_path_page: None,
            clip_groups: Vec::new(),
        }
    }

    fn x_at(model: &Model, a: f32, b: f32) -> f32 {
        let mut p = Puppet::new(model.clone());
        p.set_value(0, a);
        p.set_value(1, b);
        p.update();
        p.drawables().next().map(|d| d.positions[0]).unwrap_or(f32::NAN)
    }

    #[test]
    fn inserted_key_preserves_the_shape() {
        let mut m = test_model();
        let before: Vec<f32> = (0..=4).map(|i| x_at(&m, i as f32 / 4.0, 0.0)).collect();
        assert!(add_keys(&mut m, Target::ArtMesh(0), 0, &[0.5]));
        assert_eq!(m.art_meshes[0].binding.axes[0].keys, vec![0.0, 0.5, 1.0]);
        assert_eq!(m.art_meshes[0].positions.len(), 6);
        let after: Vec<f32> = (0..=4).map(|i| x_at(&m, i as f32 / 4.0, 0.0)).collect();
        for (x, y) in before.iter().zip(&after) {
            assert!((x - y).abs() < 1e-5, "{before:?} vs {after:?}");
        }
    }

    #[test]
    fn new_axis_tiles_and_preserves() {
        let mut m = test_model();
        let before = x_at(&m, 0.7, 0.3);
        assert!(add_keys(&mut m, Target::ArtMesh(0), 1, &[0.0, 1.0]));
        assert_eq!(m.art_meshes[0].binding.axes.len(), 2);
        assert_eq!(m.art_meshes[0].positions.len(), 8);
        let after = x_at(&m, 0.7, 0.3);
        assert!((before - after).abs() < 1e-5);
    }

    #[test]
    fn delete_key_round_trips() {
        let mut m = test_model();
        assert!(add_keys(&mut m, Target::ArtMesh(0), 0, &[0.5]));
        assert!(delete_key(&mut m, Target::ArtMesh(0), 0, 0.5));
        assert_eq!(m.art_meshes[0].binding.axes[0].keys, vec![0.0, 1.0]);
        assert_eq!(m.art_meshes[0].positions, vec![0.0, 0.0, 2.0, 0.0]);
    }

    #[test]
    fn delete_binding_keeps_default_shape() {
        let mut m = test_model();
        assert!(delete_binding(&mut m, Target::ArtMesh(0), 0, 0.0));
        assert!(m.art_meshes[0].binding.axes.is_empty());
        assert_eq!(m.art_meshes[0].positions, vec![0.0, 0.0]);
        assert_eq!(m.art_meshes[0].opacities.len(), 1);
    }

    #[test]
    fn new_warp_deformer_preserves_shapes() {
        let mut m = test_model();
        let before: Vec<f32> = (0..=4).map(|i| x_at(&m, i as f32 / 4.0, 0.0)).collect();
        let di = create_warp_deformer(&mut m, &[0], 3, 3, "W".into()).unwrap();
        assert_eq!(m.art_meshes[0].parent_deformer, Some(di as u32));
        let after: Vec<f32> = (0..=4).map(|i| x_at(&m, i as f32 / 4.0, 0.0)).collect();
        for (x, y) in before.iter().zip(&after) {
            assert!((x - y).abs() < 1e-4, "{before:?} vs {after:?}");
        }
    }

    #[test]
    fn new_rotation_deformer_preserves_shapes() {
        let mut m = test_model();
        let before: Vec<f32> = (0..=4).map(|i| x_at(&m, i as f32 / 4.0, 0.0)).collect();
        let di = create_rotation_deformer(&mut m, &[0], Some((0.3, 0.1)), "R".into()).unwrap();
        assert_eq!(m.art_meshes[0].parent_deformer, Some(di as u32));
        let after: Vec<f32> = (0..=4).map(|i| x_at(&m, i as f32 / 4.0, 0.0)).collect();
        for (x, y) in before.iter().zip(&after) {
            assert!((x - y).abs() < 1e-4, "{before:?} vs {after:?}");
        }
    }

    fn tri_mesh() -> Model {
        // one triangle, two keyforms on parameter A translating it right
        let mut m = test_model();
        if let Some(am) = m.art_meshes.get_mut(0) {
            am.vertex_count = 3;
            am.uvs = vec![0.0, 0.0, 1.0, 0.0, 0.0, 1.0];
            am.indices = vec![0, 1, 2];
            am.opacities = vec![1.0, 1.0];
            am.draw_orders = vec![500.0, 500.0];
            am.positions = vec![
                0.0, 0.0, 2.0, 0.0, 0.0, 2.0, // key 0
                1.0, 0.0, 3.0, 0.0, 1.0, 2.0, // key 1, shifted +1 in x
            ];
        }
        m
    }

    /// The triangle turned -30, 0 and 30 degrees about the origin on
    /// parameter A (-1 to 1): a sway made of keyforms, no rotation deformer.
    fn swaying_mesh() -> Model {
        let mut m = tri_mesh();
        if let Some(p) = m.parameters.get_mut(0) {
            p.min = -1.0;
            p.max = 1.0;
        }
        if let Some(am) = m.art_meshes.get_mut(0) {
            am.opacities = vec![1.0; 3];
            am.draw_orders = vec![500.0; 3];
            am.positions.clear();
            for deg in [-30.0f32, 0.0, 30.0] {
                let (s, c) = deg.to_radians().sin_cos();
                for (x, y) in [(0.0, 0.0), (2.0, 0.0), (0.0, 2.0)] {
                    am.positions.push(c * x - s * y);
                    am.positions.push(s * x + c * y);
                }
            }
            am.binding = Binding {
                axes: vec![BindingAxis { parameter: 0, keys: vec![-1.0, 0.0, 1.0] }],
                curves: Vec::new(),
            };
        }
        m
    }

    fn eval_mesh(m: &Model, a: f32) -> Vec<f32> {
        let mut p = Puppet::new(m.clone());
        p.set_value(0, a);
        p.update();
        p.drawables().next().map(|d| d.positions.to_vec()).unwrap()
    }

    #[test]
    fn extended_interpolation_keeps_the_size() {
        let mut m = swaying_mesh();
        let radius = |m: &Model| {
            let pos = eval_mesh(m, 0.5);
            (pos[2] * pos[2] + pos[3] * pos[3]).sqrt()
        };
        // a straight line between turned keys shrinks the art mid-way
        let linear = radius(&m);
        assert!(linear < 1.94, "{linear}");
        assert!(set_curve(&mut m, Target::ArtMesh(0), 0, Some(CurveKind::Sns { points: 5 })));
        let curved = radius(&m);
        assert!((curved - 2.0).abs() < 0.01, "{curved}");
        // on a key nothing changes
        let on_key = eval_mesh(&m, 1.0);
        assert!((on_key[2] - 2.0 * 30f32.to_radians().cos()).abs() < 1e-4);
    }

    #[test]
    fn baked_curve_matches_the_live_one() {
        let mut m = swaying_mesh();
        set_curve(
            &mut m,
            Target::ArtMesh(0),
            0,
            Some(CurveKind::Ellipse { points: 4, scale: 1.0 }),
        );
        let mut baked = m.clone();
        bake_curves(&mut baked);
        let am = baked.art_meshes.first().unwrap();
        assert!(am.binding.curves.is_empty());
        let keys = am.binding.axes[0].keys.clone();
        // two spans, four extra keys in each
        assert_eq!(keys.len(), 11);
        for &k in &keys {
            let (live, flat) = (eval_mesh(&m, k), eval_mesh(&baked, k));
            for (a, b) in live.iter().zip(&flat) {
                assert!((a - b).abs() < 1e-4, "at {k}: {a} vs {b}");
            }
        }
    }

    #[test]
    fn a_key_added_on_a_curve_keeps_the_shape() {
        let mut m = swaying_mesh();
        set_curve(&mut m, Target::ArtMesh(0), 0, Some(CurveKind::Sns { points: 5 }));
        let before = eval_mesh(&m, 0.3);
        assert!(add_keys(&mut m, Target::ArtMesh(0), 0, &[0.5]));
        let after = eval_mesh(&m, 0.5);
        let live = {
            let mut c = swaying_mesh();
            set_curve(&mut c, Target::ArtMesh(0), 0, Some(CurveKind::Sns { points: 5 }));
            eval_mesh(&c, 0.5)
        };
        for (a, b) in after.iter().zip(&live) {
            assert!((a - b).abs() < 1e-4);
        }
        assert_eq!(before.len(), after.len());
    }

    #[test]
    fn add_vertex_keeps_every_keyform() {
        let mut m = tri_mesh();
        let vi = add_vertex(&mut m, 0, 0, [0.5, 0.25, 0.25]);
        assert_eq!(vi, Some(3));
        let am = m.art_meshes.first().unwrap();
        assert_eq!(am.vertex_count, 4);
        assert_eq!(am.indices.len(), 9);
        // key 0: 0.5*(0,0) + 0.25*(2,0) + 0.25*(0,2) = (0.5, 0.5)
        assert!((am.positions[6] - 0.5).abs() < 1e-6);
        assert!((am.positions[7] - 0.5).abs() < 1e-6);
        // key 1 is the same mix of the shifted corners
        assert!((am.positions[14] - 1.5).abs() < 1e-6);
        assert!((am.positions[15] - 0.5).abs() < 1e-6);
    }

    #[test]
    fn delete_vertex_compacts_pools() {
        let mut m = tri_mesh();
        add_vertex(&mut m, 0, 0, [0.4, 0.3, 0.3]);
        assert!(delete_vertex(&mut m, 0, 1));
        let am = m.art_meshes.first().unwrap();
        assert_eq!(am.vertex_count, 3);
        assert_eq!(am.uvs.len(), 6);
        assert_eq!(am.positions.len(), 12);
        // only triangles that never touched vertex 1 remain, re-indexed
        assert!(am.indices.iter().all(|&i| (i as usize) < 3));
    }

    #[test]
    fn remesh_resamples_deformation() {
        let mut m = tri_mesh();
        // a new vertex mid-edge between corners 0 and 1 (uv 0.5, 0)
        let uvs = vec![0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.5, 0.0];
        let indices = vec![0, 3, 2, 3, 1, 2];
        assert!(remesh(&mut m, 0, &uvs, &indices));
        let am = m.art_meshes.first().unwrap();
        assert_eq!(am.vertex_count, 4);
        // key 0 midpoint of (0,0)-(2,0) is (1,0); key 1 shifts it to (2,0)
        assert!((am.positions[6] - 1.0).abs() < 1e-4);
        assert!((am.positions[14] - 2.0).abs() < 1e-4);
    }

    #[test]
    fn four_corners_combine_the_axes() {
        let mut m = tri_mesh();
        if let Some(am) = m.art_meshes.get_mut(0) {
            am.binding = Binding {
                curves: Vec::new(),
                axes: vec![
                    BindingAxis { parameter: 0, keys: vec![0.0, 1.0] },
                    BindingAxis { parameter: 1, keys: vec![0.0, 1.0] },
                ],
            };
            am.opacities = vec![1.0; 4];
            am.draw_orders = vec![500.0; 4];
            // centre, +x form, +y form, and a stale corner
            am.positions = vec![
                0.0, 0.0, 2.0, 0.0, 0.0, 2.0, // (0,0) centre
                1.0, 0.0, 3.0, 0.0, 1.0, 2.0, // (1,0) x moves +1
                0.0, 1.0, 2.0, 1.0, 0.0, 3.0, // (0,1) y moves +1
                9.0, 9.0, 9.0, 9.0, 9.0, 9.0, // (1,1) garbage
            ];
        }
        assert!(four_corner_forms(&mut m, Target::ArtMesh(0)));
        let am = m.art_meshes.first().unwrap();
        // corner = x form + y form - centre: moved +1 in both
        assert!((am.positions[18] - 1.0).abs() < 1e-6);
        assert!((am.positions[19] - 1.0).abs() < 1e-6);
    }

    #[test]
    fn free_vertex_extrapolates_and_connects() {
        let mut m = tri_mesh();
        // outside the triangle, past the (0,0)-(2,0) edge: w sums to one
        let vi = add_free_vertex(&mut m, 0, [0, 1, 2], [0.5, 0.75, -0.25], Some((0, 1)));
        assert_eq!(vi, Some(3));
        let am = m.art_meshes.first().unwrap();
        assert_eq!(am.indices.len(), 6);
        // key 0: 0.5*(0,0) + 0.75*(2,0) - 0.25*(0,2) = (1.5, -0.5)
        assert!((am.positions[6] - 1.5).abs() < 1e-6);
        assert!((am.positions[7] + 0.5).abs() < 1e-6);
        // key 1 is the same affine mix of the shifted triangle
        assert!((am.positions[14] - 2.5).abs() < 1e-6);
    }

    #[test]
    fn edges_add_and_remove() {
        let mut m = tri_mesh();
        // a loose fourth vertex beyond the 1-2 edge, joined to 1 and 2
        add_free_vertex(&mut m, 0, [0, 1, 2], [-0.5, 0.75, 0.75], None).unwrap();
        assert!(!has_edge(&m, 0, 1, 3));
        if let Some(am) = m.art_meshes.get_mut(0) {
            am.indices.extend_from_slice(&[1, 3, 2]);
        }
        assert!(has_edge(&m, 0, 1, 3));
        assert!(remove_edge(&mut m, 0, 1, 3));
        assert!(!has_edge(&m, 0, 1, 3));
        // 1 and 2 still share vertex 0, and adding 1-2 again finds no
        // missing face, while 0-3 has no common neighbour at all
        assert!(!add_edge(&mut m, 0, 0, 3, &[]));
        // a pending 3-0 line gives 1 and 3 a common neighbour, so joining
        // them closes the face 1-3-0
        assert!(add_edge(&mut m, 0, 1, 3, &[(3, 0)]));
        assert!(has_edge(&m, 0, 0, 3));
    }

    #[test]
    fn erase_many_keeps_pools_consistent() {
        let mut m = tri_mesh();
        // grow the mesh to seven points, then erase three of them at once
        for w in [[0.5, 0.25, 0.25], [0.25, 0.5, 0.25], [0.25, 0.25, 0.5], [0.4, 0.3, 0.3]] {
            add_vertex(&mut m, 0, 0, w).unwrap();
        }
        assert!(delete_vertices(&mut m, 0, &[1, 4, 6]));
        let am = m.art_meshes.first().unwrap();
        assert_eq!(am.vertex_count, 4);
        assert_eq!(am.uvs.len(), 8);
        assert_eq!(am.positions.len(), 16);
        assert!(am.indices.iter().all(|&i| (i as usize) < 4));
        // the single-point form works through the batch
        assert!(delete_vertex(&mut m, 0, 3));
        assert_eq!(m.art_meshes.first().unwrap().vertex_count, 3);
        // and a mesh may be erased down to nothing, keyforms and all
        assert!(delete_vertices(&mut m, 0, &[0, 1, 2]));
        let am = m.art_meshes.first().unwrap();
        assert_eq!((am.vertex_count, am.uvs.len(), am.positions.len()), (0, 0, 0));
        // nothing left to erase
        assert!(!delete_vertices(&mut m, 0, &[0]));
    }

    #[test]
    fn auto_connect_triangulates_loose_points() {
        let mut m = tri_mesh();
        // a fourth point beyond the 1-2 edge, loose, then auto connect
        add_free_vertex(&mut m, 0, [0, 1, 2], [-0.5, 0.75, 0.75], None).unwrap();
        if let Some(am) = m.art_meshes.get_mut(0) {
            am.indices.clear();
        }
        let added = auto_connect(&mut m, 0, true, &[], None);
        assert_eq!(added, 2);
        // keeping the existing faces adds nothing that overlaps them
        assert_eq!(auto_connect(&mut m, 0, false, &[], None), 0);
    }

    #[test]
    fn remesh_carries_points_past_the_old_faces_on() {
        // the triangle spans uv (0,0) (1,0) (0,1) and position 2x that; a
        // new point at uv (1, 1), outside it, lands at (2, 2), not on a corner
        let mut m = tri_mesh();
        assert!(remesh(&mut m, 0, &[0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 1.0, 1.0], &[0, 1, 2, 1, 3, 2]));
        let am = &m.art_meshes[0];
        assert!((am.positions[6] - 2.0).abs() < 1e-5 && (am.positions[7] - 2.0).abs() < 1e-5);
        // and a cleared mesh takes a new triangulation from its reference
        let reference = am.clone();
        let n = am.vertex_count as usize;
        assert!(delete_vertices(&mut m, 0, &(0..n).collect::<Vec<_>>()));
        assert!(!remesh(&mut m.clone(), 0, &[0.0, 0.0, 1.0, 0.0, 0.0, 1.0], &[0, 1, 2]));
        assert!(remesh_from(
            &mut m,
            0,
            Some(&reference),
            &[0.0, 0.0, 1.0, 0.0, 0.0, 1.0],
            &[0, 1, 2]
        ));
        assert_eq!(m.art_meshes[0].vertex_count, 3);
    }

    #[test]
    fn a_mesh_erased_to_nothing_grows_again_from_its_reference() {
        let mut m = tri_mesh();
        let reference = m.art_meshes[0].clone();
        assert!(delete_vertices(&mut m, 0, &[0, 1, 2]));
        assert_eq!(m.art_meshes[0].vertex_count, 0);
        // the runtime takes an empty mesh
        let mut p = crate::Puppet::new(m.clone());
        p.update();
        // one point at the reference's middle, on both keyforms
        let third = 1.0 / 3.0;
        let v = add_vertex_from(&mut m, 0, &reference, [0, 1, 2], [third; 3], None).unwrap();
        assert_eq!(v, 0);
        let am = &m.art_meshes[0];
        assert_eq!(am.positions.len(), 4, "a point on each of the two keyforms");
        assert!((am.positions[0] - 2.0 / 3.0).abs() < 1e-5);
        assert!((am.positions[2] - (1.0 + 2.0 / 3.0)).abs() < 1e-5, "key 1 is shifted +1");
        // and with one point a second still goes in
        add_vertex_from(&mut m, 0, &reference, [0, 1, 2], [1.0, 0.0, 0.0], None).unwrap();
        assert_eq!(m.art_meshes[0].vertex_count, 2);
        assert_eq!(m.art_meshes[0].positions.len(), 8);
    }

    #[test]
    fn auto_connect_keeps_every_face_that_holds_art() {
        // an imported rectangle over a strip of art in its lower right: the
        // lower left face's centre is on empty texture, but the strip
        // reaches into it, so it must stay
        let mut m = test_model();
        if let Some(am) = m.art_meshes.get_mut(0) {
            am.vertex_count = 4;
            am.uvs = vec![0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 1.0, 1.0];
            am.positions = am.uvs.clone();
            am.indices.clear();
        }
        let strip = |u: f32, v: f32| (0.6..0.95).contains(&u) && v > 0.3;
        assert_eq!(auto_connect(&mut m, 0, true, &[], Some(&strip)), 2);
        // and faces over nothing at all go
        let empty = |_: f32, _: f32| false;
        assert_eq!(auto_connect(&mut m, 0, true, &[], Some(&empty)), 0);
    }

    #[test]
    fn auto_connect_keeps_inside_a_drawn_perimeter() {
        // a U of eight points, its outline drawn as lines: the notch
        // (x 1..2, y 0..2) is outside it and must stay open
        let mut m = test_model();
        let u = [(0.0, 0.0), (0.0, 3.0), (3.0, 3.0), (3.0, 0.0)];
        let notch = [(2.0, 0.0), (2.0, 2.0), (1.0, 2.0), (1.0, 0.0)];
        let pts: Vec<(f32, f32)> =
            u.iter().chain(&notch).map(|&(x, y)| (x / 3.0, y / 3.0)).collect();
        if let Some(am) = m.art_meshes.get_mut(0) {
            am.vertex_count = 8;
            am.uvs = pts.iter().flat_map(|&(x, y)| [x, y]).collect();
            am.positions = am.uvs.clone();
            am.indices.clear();
        }
        let lines: Vec<(u16, u16)> = (0..8u16).map(|i| (i, (i + 1) % 8)).collect();
        let solid = |_: f32, _: f32| true;
        let added = auto_connect(&mut m, 0, true, &lines, Some(&solid));
        assert_eq!(added, 6, "an eight-point outline holds six faces");
        let am = &m.art_meshes[0];
        for t in am.indices.chunks(3) {
            let c = t.iter().fold((0.0, 0.0), |a, &v| {
                (a.0 + pts[v as usize].0 / 3.0, a.1 + pts[v as usize].1 / 3.0)
            });
            let in_notch = c.0 > 1.0 / 3.0 && c.0 < 2.0 / 3.0 && c.1 < 2.0 / 3.0;
            assert!(!in_notch, "a face fills the notch at {c:?}");
        }
        // with protection off the hull fills, notch and all
        let added = auto_connect(&mut m, 0, true, &lines, None);
        assert!(added > 6);
    }

    #[test]
    fn subdivide_keeps_shape_and_old_indices() {
        let mut m = tri_mesh();
        assert!(subdivide4(&mut m, 0));
        let am = m.art_meshes.first().unwrap();
        assert_eq!(am.vertex_count, 6);
        assert_eq!(am.indices.len(), 12);
        // the midpoint of corners 0 and 1 on key 1: ((1,0)+(3,0))/2 = (2,0)
        let stride = 12;
        let mids: Vec<(f32, f32)> = (3..6)
            .map(|i| (am.positions[stride + i * 2], am.positions[stride + i * 2 + 1]))
            .collect();
        assert!(mids.iter().any(|&(x, y)| (x - 2.0).abs() < 1e-6 && y.abs() < 1e-6));
    }

    #[test]
    fn glue_binds_overlaps_and_brushes_weights() {
        let mut m = tri_mesh();
        let mut second = m.art_meshes[0].clone();
        second.id = "M2".into();
        m.art_meshes.push(second);
        // world positions: mesh B sits on A except its last point
        let wa = [0.0, 0.0, 2.0, 0.0, 0.0, 2.0];
        let wb = [0.001, 0.0, 2.0, 0.001, 5.0, 5.0];
        let (gi, added) = bind_glue(&mut m, 0, 1, &wa, &wb, 0.01).unwrap();
        assert_eq!(added, 2);
        assert_eq!(m.glues[gi].weights, vec![0.5, 0.5, 0.5, 0.5]);
        // binding again finds nothing new
        assert!(bind_glue(&mut m, 0, 1, &wa, &wb, 0.01).is_none());
        // favour A on pair 0: A pulls less, B more, total stays 1
        assert!(glue_brush(&mut m, gi, &[(0, 0.2)], GlueBrush::Balance { favor_a: true }));
        let w = &m.glues[gi].weights;
        assert!((w[0] - 0.3).abs() < 1e-6 && (w[1] - 0.7).abs() < 1e-6);
        // only-B separates, re-merge restores the total
        glue_brush(&mut m, gi, &[(1, 0.4)], GlueBrush::OnlyB);
        assert!((m.glues[gi].weights[3] - 0.1).abs() < 1e-6);
        glue_brush(&mut m, gi, &[(1, 1.0)], GlueBrush::Remerge);
        let w = &m.glues[gi].weights;
        assert!((w[2] + w[3] - 1.0).abs() < 1e-5);
        assert!(set_glue_compatibility(&mut m, gi, 0.25));
        assert!(unbind_glue(&mut m, gi));
        assert!(m.glues.is_empty());
    }

    #[test]
    fn wrapping_deformers_keeps_the_pose() {
        // mesh under a rotation, then a warp around the rotation, then a
        // rotation around that warp: the mesh must not move at any step
        let mut m = test_model();
        let at = |m: &Model| (0..=4).map(|i| x_at(m, i as f32 / 4.0, 0.0)).collect::<Vec<f32>>();
        let before = at(&m);
        let r = create_deformer(
            &mut m,
            NewDeformer::Rotation { origin: Some((0.3, 0.1)) },
            &[Child::Mesh(0)],
            None,
            (0.0, 0.0, 1.0, 1.0),
            None,
            "R".into(),
        )
        .unwrap();
        assert!((at(&m)[0] - before[0]).abs() < 1e-3, "rotation step: {:?}", at(&m));
        let w = create_deformer(
            &mut m,
            NewDeformer::Warp { rows: 2, cols: 2 },
            &[Child::Deformer(r)],
            None,
            (0.0, 0.0, 1.0, 1.0),
            None,
            "W".into(),
        )
        .unwrap();
        // the warp went in before the rotation, which shifted up one
        let r = r + 1;
        assert_eq!(w, 0);
        assert_eq!(m.deformers[r].parent_deformer, Some(w as u32));
        assert!((at(&m)[0] - before[0]).abs() < 1e-3, "warp step: {:?}", at(&m));
        let r2 = create_deformer(
            &mut m,
            NewDeformer::Rotation { origin: None },
            &[Child::Deformer(w)],
            None,
            (0.0, 0.0, 1.0, 1.0),
            None,
            "R2".into(),
        )
        .unwrap();
        assert_eq!(r2, 0);
        let w = w + 1;
        assert_eq!(m.deformers[w].parent_deformer, Some(r2 as u32));
        let after = at(&m);
        for (x, y) in before.iter().zip(&after) {
            assert!((x - y).abs() < 1e-3, "{before:?} vs {after:?}");
        }
        // an empty one under a chosen parent holds nothing
        let e = create_deformer(
            &mut m,
            NewDeformer::Warp { rows: 1, cols: 1 },
            &[],
            Some(r2 as u32),
            (-0.5, -0.5, 0.5, 0.5),
            None,
            "E".into(),
        )
        .unwrap();
        assert_eq!(m.deformers[e].parent_deformer, Some(r2 as u32));
    }

    #[test]
    fn deleting_deformers_keeps_the_pose() {
        // the same nesting as above, then take it apart in an awkward order:
        // the middle warp, the outer rotation, the inner rotation
        let mut m = test_model();
        let at = |m: &Model| (0..=4).map(|i| x_at(m, i as f32 / 4.0, 0.0)).collect::<Vec<f32>>();
        let before = at(&m);
        let mk = |m: &mut Model, kind, child| {
            create_deformer(m, kind, &[child], None, (0.0, 0.0, 1.0, 1.0), None, "D".into())
                .unwrap()
        };
        let r = mk(&mut m, NewDeformer::Rotation { origin: Some((0.3, 0.1)) }, Child::Mesh(0));
        let w = mk(&mut m, NewDeformer::Warp { rows: 2, cols: 2 }, Child::Deformer(r));
        let r2 = mk(&mut m, NewDeformer::Rotation { origin: None }, Child::Deformer(w));
        assert_eq!(m.deformers.len(), 3);
        let close = |m: &Model, step: &str| {
            for (x, y) in before.iter().zip(&at(m)) {
                assert!((x - y).abs() < 2e-3, "{step}: {before:?} vs {:?}", at(m));
            }
        };
        // order is r2, w, r; the warp sits in the middle
        let w = m.deformers.iter().position(|d| matches!(d.kind, DeformerKind::Warp(_))).unwrap();
        assert!(delete_deformer(&mut m, w));
        close(&m, "warp deleted");
        assert_eq!(m.deformers.len(), 2);
        // the inner rotation now hangs off the outer one
        assert_eq!(m.deformers[1].parent_deformer, Some(0));
        let _ = r2;
        assert!(delete_deformer(&mut m, 0));
        close(&m, "outer rotation deleted");
        assert_eq!(m.deformers[0].parent_deformer, None);
        assert!(delete_deformer(&mut m, 0));
        close(&m, "last rotation deleted");
        assert!(m.deformers.is_empty());
        assert_eq!(m.art_meshes[0].parent_deformer, None);
        assert!(!delete_deformer(&mut m, 0));
    }

    #[test]
    fn pivot_moves_without_moving_the_mesh() {
        // a rotation that turns 0 -> 40 degrees over parameter B; dragging
        // its pivot must leave the mesh where it is in every pose
        let mut m = test_model();
        let r = create_deformer(
            &mut m,
            NewDeformer::Rotation { origin: Some((0.3, 0.1)) },
            &[Child::Mesh(0)],
            None,
            (0.0, 0.0, 1.0, 1.0),
            None,
            "R".into(),
        )
        .unwrap();
        let DeformerKind::Rotation(rot) = &mut m.deformers[r].kind else { panic!() };
        rot.angles = vec![0.0, 40.0];
        rot.origins_x = vec![0.3, 0.3];
        rot.origins_y = vec![0.1, 0.1];
        rot.scales = vec![1.0, 1.0];
        rot.reflect_x = vec![false, false];
        rot.reflect_y = vec![false, false];
        rot.opacities = vec![1.0, 1.0];
        m.deformers[r].binding = Binding {
            curves: Vec::new(),
            axes: vec![BindingAxis { parameter: 1, keys: vec![0.0, 1.0] }],
        };
        let xy = |m: &Model, a: f32, b: f32| {
            let mut p = Puppet::new(m.clone());
            p.set_value(0, a);
            p.set_value(1, b);
            p.update();
            p.drawables().next().map(|d| (d.positions[0], d.positions[1])).unwrap()
        };
        let poses = [(0.0, 0.0), (1.0, 0.0), (0.0, 1.0), (1.0, 1.0), (0.5, 0.5)];
        let before: Vec<_> = poses.iter().map(|&(a, b)| xy(&m, a, b)).collect();
        // drag the pivot by D at keyform 0 (no turn, root parent): the child
        // offset is -D and keyform k's origin moves by Rot(angle_k) D
        let d = (0.2f32, -0.1f32);
        let deltas: Vec<(f32, f32)> = [0.0f32, 40.0]
            .iter()
            .map(|deg| {
                let (s, c) = deg.to_radians().sin_cos();
                (c * d.0 - s * d.1, s * d.0 + c * d.1)
            })
            .collect();
        assert!(move_rotation_pivot(&mut m, r, &deltas, (-d.0, -d.1)));
        let DeformerKind::Rotation(rot) = &m.deformers[r].kind else { panic!() };
        assert!((rot.origins_x[0] - 0.5).abs() < 1e-6);
        for (p, (x, y)) in poses.iter().zip(&before) {
            let (nx, ny) = xy(&m, p.0, p.1);
            // exact on the keyforms; between them the origin blends along a
            // chord while the angle sweeps an arc, so it may stray by up to
            // |D| (1 - cos(half the turn)), as in any keyform rig
            let on_key = p.1 == 0.0 || p.1 == 1.0;
            let tol = if on_key { 1e-4 } else { 0.25 * (1.0 - 20f32.to_radians().cos()) + 1e-4 };
            let err = ((nx - x).powi(2) + (ny - y).powi(2)).sqrt();
            assert!(err < tol, "{p:?}: ({x},{y}) -> ({nx},{ny}), off by {err}");
        }
        assert!(!move_rotation_pivot(&mut m, r, &[(0.0, 0.0)], (0.0, 0.0)));
    }

    /// The mesh vertex on the canvas at the default pose.
    fn mesh_xy(m: &Model) -> (f32, f32) {
        let mut p = Puppet::new(m.clone());
        p.update();
        p.drawables().next().map(|d| (d.positions[0], d.positions[1])).unwrap()
    }

    /// A mesh under rotation "inner" (30 deg) under rotation "outer"
    /// (25 deg, scale 1.5), plus a free warp at the end of the list.
    fn turned_rig() -> Model {
        let mut m = test_model();
        let mk = |m: &mut Model, kind, kids: &[Child], id: &str| {
            create_deformer(m, kind, kids, None, (0.0, 0.0, 1.0, 1.0), None, id.into()).unwrap()
        };
        let inner = mk(
            &mut m,
            NewDeformer::Rotation { origin: Some((0.4, 0.2)) },
            &[Child::Mesh(0)],
            "inner",
        );
        let outer = mk(
            &mut m,
            NewDeformer::Rotation { origin: Some((0.1, -0.3)) },
            &[Child::Deformer(inner)],
            "outer",
        );
        mk(&mut m, NewDeformer::Warp { rows: 2, cols: 2 }, &[], "free");
        let turn = |m: &mut Model, id: &str, deg: f32, scale: f32| {
            let d = m.deformers.iter_mut().find(|d| d.id == id).unwrap();
            let DeformerKind::Rotation(r) = &mut d.kind else { panic!() };
            r.base_angle = deg;
            r.scales = vec![scale];
        };
        turn(&mut m, "inner", 30.0, 1.0);
        turn(&mut m, "outer", 25.0, 1.5);
        let _ = outer;
        m
    }

    fn idx(m: &Model, id: &str) -> usize {
        m.deformers.iter().position(|d| d.id == id).unwrap()
    }

    fn parents_first(m: &Model) -> bool {
        m.deformers
            .iter()
            .enumerate()
            .all(|(i, d)| d.parent_deformer.is_none_or(|p| (p as usize) < i))
    }

    fn same(a: (f32, f32), b: (f32, f32), step: &str) {
        assert!((a.0 - b.0).abs() < 1e-3 && (a.1 - b.1).abs() < 1e-3, "{step}: {a:?} -> {b:?}");
    }

    #[test]
    fn reparenting_keeps_the_mesh_still() {
        let m0 = turned_rig();
        let at = mesh_xy(&m0);
        // the turned inner rotation out to the top level
        let mut m = m0.clone();
        let inner = idx(&m, "inner");
        set_parent(&mut m, Child::Deformer(inner), None).unwrap();
        same(at, mesh_xy(&m), "inner to top");
        // the mesh itself out of both rotations
        let mut m = m0.clone();
        set_parent(&mut m, Child::Mesh(0), None).unwrap();
        same(at, mesh_xy(&m), "mesh to top");
        assert_eq!(m.art_meshes[0].parent_deformer, None);
        // under the warp that sits last: the list re-sorts, parents first
        let mut m = m0.clone();
        let (inner, free) = (idx(&m, "inner"), idx(&m, "free"));
        assert!(free > inner);
        let moved = set_parent(&mut m, Child::Deformer(inner), Some(free)).unwrap();
        assert!(parents_first(&m));
        assert_eq!(moved, Child::Deformer(idx(&m, "inner")));
        assert_eq!(m.deformers[idx(&m, "inner")].parent_deformer, Some(idx(&m, "free") as u32));
        same(at, mesh_xy(&m), "inner under the warp");
        // never under itself or its own child
        let mut m = m0.clone();
        let (inner, outer) = (idx(&m, "inner"), idx(&m, "outer"));
        assert!(set_parent(&mut m, Child::Deformer(outer), Some(inner)).is_none());
        assert!(set_parent(&mut m, Child::Deformer(outer), Some(outer)).is_none());
    }

    #[test]
    fn deleting_a_turned_parent_keeps_the_mesh_still() {
        let mut m = turned_rig();
        let at = mesh_xy(&m);
        let outer = idx(&m, "outer");
        assert!(delete_deformer(&mut m, outer));
        same(at, mesh_xy(&m), "outer deleted");
        assert!(parents_first(&m));
    }

    #[test]
    fn sliding_a_vertex_moves_it_and_its_uv() {
        let mut m = test_model();
        assert!(slide_vertex(&mut m, 0, 0, 0.5, 0.25, (0.3, 0.7)));
        // every keyform moved, the texture coordinate replaced
        assert_eq!(m.art_meshes[0].positions, vec![0.5, 0.25, 2.5, 0.25]);
        assert_eq!(m.art_meshes[0].uvs, vec![0.3, 0.7]);
        assert!(!slide_vertex(&mut m, 0, 9, 0.1, 0.1, (0.0, 0.0)));
        assert!(!slide_vertex(&mut m, 0, 0, 0.1, 0.1, (f32::NAN, 0.0)));
    }

    #[test]
    fn a_deformer_over_meshes_in_different_warps_wraps_their_warps() {
        let mut m = test_model();
        let base = m.art_meshes[0].clone();
        m.art_meshes = (0..2).map(|i| ArtMesh { id: format!("M{i}"), ..base.clone() }).collect();
        let rest = |m: &Model| {
            let mut p = Puppet::new(m.clone());
            p.update();
            p.drawables().map(|d| d.positions.to_vec()).collect::<Vec<_>>()
        };
        let warp = NewDeformer::Warp { rows: 2, cols: 2 };
        let ext = (0.0, 0.0, 1.0, 1.0);
        create_deformer(
            &mut m,
            warp,
            &[Child::Mesh(0), Child::Mesh(1)],
            None,
            ext,
            None,
            "Head".into(),
        )
        .unwrap();
        let head = m.deformers.iter().position(|d| d.id == "Head").unwrap();
        create_deformer(
            &mut m,
            warp,
            &[Child::Mesh(0)],
            Some(head as u32),
            ext,
            None,
            "HairA".into(),
        )
        .unwrap();
        create_deformer(
            &mut m,
            warp,
            &[Child::Mesh(1)],
            Some(head as u32),
            ext,
            None,
            "HairB".into(),
        )
        .unwrap();
        let before = rest(&m);
        // the two meshes now have different parents: a toggle over both
        create_deformer(
            &mut m,
            warp,
            &[Child::Mesh(0), Child::Mesh(1)],
            None,
            ext,
            None,
            "Toggle".into(),
        )
        .expect("wraps the two hair warps");
        let id = |n: &str| m.deformers.iter().position(|d| d.id == n).unwrap() as u32;
        let (head, toggle) = (id("Head"), id("Toggle"));
        assert_eq!(m.deformers[toggle as usize].parent_deformer, Some(head));
        assert_eq!(m.deformers[id("HairA") as usize].parent_deformer, Some(toggle));
        assert_eq!(m.deformers[id("HairB") as usize].parent_deformer, Some(toggle));
        // the meshes stay in their own warps and do not move
        assert_eq!(m.art_meshes[0].parent_deformer, Some(id("HairA")));
        let after = rest(&m);
        for (a, b) in before.iter().flatten().zip(after.iter().flatten()) {
            assert!((a - b).abs() < 1e-4, "{a} vs {b}");
        }
    }

    #[test]
    fn pasted_points_merge_place_and_join() {
        let mut m = test_model();
        if let Some(am) = m.art_meshes.get_mut(0) {
            // a unit square (two faces), two keyforms: the second moved +1 in x
            am.vertex_count = 4;
            am.uvs = vec![0.0, 0.0, 1.0, 0.0, 1.0, 1.0, 0.0, 1.0];
            am.indices = vec![0, 1, 2, 0, 2, 3];
            am.positions = vec![
                0.0, 0.0, 1.0, 0.0, 1.0, 1.0, 0.0, 1.0, 1.0, 0.0, 2.0, 0.0, 2.0, 1.0, 1.0, 1.0,
            ];
        }
        let centre = PastedPoint::At([0, 1, 2], [0.5, 0.0, 0.5]);
        let pts = [PastedPoint::Existing(0), PastedPoint::Existing(1), centre];
        let got = paste_points(&mut m, 0, &pts, &[[0, 1, 2], [0, 0, 1]]).unwrap();
        assert_eq!(got, vec![0, 1, 4]);
        let am = &m.art_meshes[0];
        assert_eq!(am.vertex_count, 5);
        // the new point sits at the middle on both keyforms, uv included
        assert_eq!(&am.uvs[8..10], &[0.5, 0.5]);
        assert_eq!(&am.positions[8..10], &[0.5, 0.5]);
        assert_eq!(&am.positions[18..20], &[1.5, 0.5]);
        // one face added, the collapsed one skipped
        assert_eq!(am.indices.len(), 9);
        // pasting the same again adds the point but not a repeated face
        let again = paste_points(
            &mut m,
            0,
            &[PastedPoint::Existing(0), PastedPoint::Existing(1), PastedPoint::Existing(4)],
            &[[0, 1, 2]],
        )
        .unwrap();
        assert_eq!(again, vec![0, 1, 4]);
        assert_eq!(m.art_meshes[0].indices.len(), 9);
        assert!(paste_points(&mut m, 0, &[PastedPoint::Existing(99)], &[]).is_none());
    }

    #[test]
    fn a_used_parameter_unbinds_safely_then_deletes() {
        let mut m = test_model();
        // mesh 0 is keyed on parameter A (x from 0 to 2)
        assert_eq!(param_users(&m, 0), (1, 0));
        assert!(!delete_parameter(&mut m, 0), "still used");
        assert!(unbind_parameter_everywhere(&mut m, 0));
        assert_eq!(param_users(&m, 0), (0, 0));
        // the mesh kept its form at A's default (x = 0)
        assert!(m.art_meshes[0].binding.axes.is_empty());
        assert_eq!(&m.art_meshes[0].positions[..2], &[0.0, 0.0]);
        assert!(delete_parameter(&mut m, 0));
        assert_eq!(m.parameters.len(), 1);
    }

    #[test]
    fn deleting_meshes_fixes_every_reference() {
        use ayatsuri_format::model::{DrawGroup, DrawItem, Glue};
        let mut m = test_model();
        let base = m.art_meshes[0].clone();
        m.art_meshes = (0..4).map(|i| ArtMesh { id: format!("M{i}"), ..base.clone() }).collect();
        // M3 clips by M1 (goes) and M2 (stays)
        m.art_meshes[3].masks = vec![1, 2];
        let glue = |a, b| Glue {
            id: format!("G{a}{b}"),
            art_mesh_a: a,
            art_mesh_b: b,
            binding: Binding::default(),
            intensities: vec![1.0],
            weights: vec![],
            indices: vec![],
        };
        m.glues = vec![glue(0, 1), glue(2, 3)];
        let mesh = |object| DrawItem { part: false, object, child_group: None };
        // root: M0 and a part holding group 1 (M1, M2, M3)
        m.draw_groups = vec![
            DrawGroup {
                total: 4,
                min_order: 500,
                max_order: 500,
                items: vec![mesh(0), DrawItem { part: true, object: 0, child_group: Some(1) }],
            },
            DrawGroup {
                total: 3,
                min_order: 500,
                max_order: 500,
                items: vec![mesh(1), mesh(2), mesh(3)],
            },
        ];
        assert!(delete_art_meshes(&mut m, &[1, 1, 9]));
        let ids: Vec<&str> = m.art_meshes.iter().map(|a| a.id.as_str()).collect();
        assert_eq!(ids, ["M0", "M2", "M3"]);
        assert_eq!(m.art_meshes[2].masks, vec![1]);
        assert_eq!(m.glues.len(), 1);
        assert_eq!((m.glues[0].art_mesh_a, m.glues[0].art_mesh_b), (1, 2));
        assert_eq!(m.draw_groups[1].total, 2);
        assert_eq!(m.draw_groups[0].total, 3);
        let objects: Vec<u32> = m.draw_groups[1].items.iter().map(|i| i.object).collect();
        assert_eq!(objects, vec![1, 2]);
        // the runtime still draws all three
        let mut p = Puppet::new(m.clone());
        p.update();
        assert_eq!(p.drawables().count(), 3);
        assert!(!delete_art_meshes(&mut m, &[7]));
    }

    #[test]
    fn a_pasted_mesh_draws_beside_its_original_and_a_cut_keeps_its_id() {
        use ayatsuri_format::model::{DrawGroup, DrawItem};
        let mut m = test_model();
        let base = m.art_meshes[0].clone();
        m.art_meshes = (0..3).map(|i| ArtMesh { id: format!("M{i}"), ..base.clone() }).collect();
        let mesh = |object| DrawItem { part: false, object, child_group: None };
        // root: M0 and a part holding group 1 (M1, M2)
        m.draw_groups = vec![
            DrawGroup {
                total: 3,
                min_order: 500,
                max_order: 500,
                items: vec![mesh(0), DrawItem { part: true, object: 0, child_group: Some(1) }],
            },
            DrawGroup { total: 2, min_order: 500, max_order: 500, items: vec![mesh(1), mesh(2)] },
        ];
        // a copy of M1: a new id, right after M1 in M1's group, totals up the tree
        let copy = m.art_meshes[1].clone();
        let i = insert_art_mesh(&mut m, copy.clone(), None, Some("M1"));
        assert_eq!(i, 3);
        assert_eq!(m.art_meshes[3].id, "M1_copy");
        let objects: Vec<u32> = m.draw_groups[1].items.iter().map(|it| it.object).collect();
        assert_eq!(objects, vec![1, 3, 2]);
        assert_eq!((m.draw_groups[1].total, m.draw_groups[0].total), (3, 4));
        assert_eq!(insert_art_mesh(&mut m, copy, None, Some("M1")), 4);
        assert_eq!(m.art_meshes[4].id, "M1_copy2");
        // cut M2, paste it back: the id is free again, so it keeps it
        let cut = m.art_meshes[2].clone();
        assert!(delete_art_meshes(&mut m, &[2]));
        let i = insert_art_mesh(&mut m, cut, None, Some("M2"));
        assert_eq!(m.art_meshes[i].id, "M2");
        let mut p = Puppet::new(m.clone());
        p.update();
        assert_eq!(p.drawables().count(), 5);
    }

    #[test]
    fn masks_drop_self_repeats_and_unknown_meshes() {
        let mut m = test_model();
        let base = m.art_meshes[0].clone();
        m.art_meshes = (0..3).map(|i| ArtMesh { id: format!("M{i}"), ..base.clone() }).collect();
        assert!(set_masks(&mut m, 0, &[2, 0, 2, 9, 1]));
        assert_eq!(m.art_meshes[0].masks, vec![2, 1]);
        assert!(!set_masks(&mut m, 0, &[2, 1]));
        assert!(set_masks(&mut m, 0, &[]));
        assert!(m.art_meshes[0].masks.is_empty());
    }

    #[test]
    fn opacity_is_set_on_one_keyform() {
        let mut m = test_model();
        assert!(set_opacity(&mut m, Target::ArtMesh(0), 1, 0.25));
        assert_eq!(m.art_meshes[0].opacities, vec![1.0, 0.25]);
        assert!(!set_opacity(&mut m, Target::ArtMesh(0), 1, 0.25));
        // out of range clamps: 7 is the 1.0 already there, -3 becomes 0
        assert!(!set_opacity(&mut m, Target::ArtMesh(0), 0, 7.0));
        assert!(set_opacity(&mut m, Target::ArtMesh(0), 0, -3.0));
        assert_eq!(m.art_meshes[0].opacities[0], 0.0);
        assert!(!set_opacity(&mut m, Target::ArtMesh(0), 5, 0.5));
        assert!(!set_opacity(&mut m, Target::Part(0), 0, 0.5));
        // halfway between the keys the runtime blends it
        let mut p = Puppet::new(m);
        p.set_value(0, 0.5);
        p.update();
        let o = p.drawables().next().map(|d| d.opacity).unwrap_or(f32::NAN);
        assert!((o - 0.125).abs() < 1e-4, "{o}");
    }

    #[test]
    fn deformer_ids_stay_unique() {
        let mut m = test_model();
        let a = create_deformer(
            &mut m,
            NewDeformer::Warp { rows: 1, cols: 1 },
            &[],
            None,
            (0.0, 0.0, 1.0, 1.0),
            None,
            "A".into(),
        )
        .unwrap();
        let b = create_deformer(
            &mut m,
            NewDeformer::Warp { rows: 1, cols: 1 },
            &[],
            None,
            (0.0, 0.0, 1.0, 1.0),
            None,
            "B".into(),
        )
        .unwrap();
        assert!(!rename_deformer(&mut m, b, "A"));
        assert!(!rename_deformer(&mut m, b, "  "));
        assert!(rename_deformer(&mut m, a, " Face "));
        assert_eq!(m.deformers[a].id, "Face");
    }

    #[test]
    fn warp_points_move_in_one_keyform() {
        let mut m = test_model();
        let di = create_warp_deformer(&mut m, &[0], 1, 1, "W".into()).unwrap();
        // give the warp two keyforms on parameter A
        assert!(add_keys(&mut m, Target::Deformer(di), 0, &[0.0, 1.0]));
        assert!(move_warp_points(&mut m, di, Some(1), &[(0, 0.5, 0.0)]));
        let DeformerKind::Warp(w) = &m.deformers[di].kind else { panic!() };
        let stride = w.grid_points() * 2;
        assert!((w.positions[stride] - w.positions[0] - 0.5).abs() < 1e-6);
        // None moves every keyform
        assert!(move_warp_points(&mut m, di, None, &[(1, 0.0, 0.25)]));
    }

    #[test]
    fn warp_division_changes_and_keeps_the_shape() {
        let mut m = test_model();
        let di = create_warp_deformer(&mut m, &[0], 2, 2, "W".into()).unwrap();
        assert!(add_keys(&mut m, Target::Deformer(di), 0, &[0.0, 1.0]));
        // bend the middle point in one keyform only
        assert!(move_warp_points(&mut m, di, Some(1), &[(4, 0.3, -0.2)]));
        let DeformerKind::Warp(before) = m.deformers[di].kind.clone() else { panic!() };
        assert!(set_warp_division(&mut m, di, 4, 4));
        let DeformerKind::Warp(w) = &m.deformers[di].kind else { panic!() };
        assert_eq!((w.cols, w.rows), (4, 4));
        assert_eq!(w.positions.len(), 2 * 25 * 2);
        // every old point is still there, at twice the index
        for k in 0..2 {
            for r in 0..=2 {
                for c in 0..=2 {
                    let old = (k * 9 + r * 3 + c) * 2;
                    let new = (k * 25 + r * 2 * 5 + c * 2) * 2;
                    assert!((w.positions[new] - before.positions[old]).abs() < 1e-5);
                    assert!((w.positions[new + 1] - before.positions[old + 1]).abs() < 1e-5);
                }
            }
        }
        // and back again gives the same grid
        assert!(set_warp_division(&mut m, di, 2, 2));
        let DeformerKind::Warp(w) = &m.deformers[di].kind else { panic!() };
        for (a, b) in w.positions.iter().zip(&before.positions) {
            assert!((a - b).abs() < 1e-5);
        }
        // the same division, out of range, or not a warp: refused
        assert!(!set_warp_division(&mut m, di, 2, 2));
        assert!(!set_warp_division(&mut m, di, 0, 3));
        assert!(!set_warp_division(&mut m, di, 65, 3));
        assert!(!set_warp_division(&mut m, 99, 3, 3));
    }

    #[test]
    fn vertices_move_in_one_keyform() {
        let mut m = test_model();
        // two keyforms: x = 0 and x = 2
        assert!(move_vertices_in(&mut m, 0, Some(1), &[(0, 0.5, 0.0)]));
        assert_eq!(m.art_meshes[0].positions, vec![0.0, 0.0, 2.5, 0.0]);
        assert!(!move_vertices_in(&mut m, 0, Some(9), &[(0, 1.0, 0.0)]));
    }

    #[test]
    fn flip_mirrors_across_the_centre() {
        let mut m = test_model();
        // keyform xs are 0 and 2: centre 1, so they swap
        assert!(flip_object(&mut m, Target::ArtMesh(0), true));
        assert_eq!(m.art_meshes[0].positions, vec![2.0, 0.0, 0.0, 0.0]);
        assert!(!flip_object(&mut m, Target::Part(0), true));
    }
}
