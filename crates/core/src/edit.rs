// The first real editing operations: adding and removing parameter keys on
// an object's binding. Keyform arrays are cartesian products (first axis
// fastest), so every change resamples the product: inserted keys take the
// interpolated shape, so the puppet looks identical until someone moves it.

use ayatsuri_format::model::{Binding, BindingAxis, DeformerKind, Model};

/// Which rig object an edit touches.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Target {
    Part(usize),
    Deformer(usize),
    ArtMesh(usize),
}

/// Where each key of a changed axis takes its data from.
#[derive(Clone, Copy)]
enum KeySrc {
    Copy(usize),
    Lerp(usize, usize, f32),
}

enum Channel<'a> {
    F32 { elems: usize, data: &'a mut Vec<f32> },
    Bool { data: &'a mut Vec<bool> },
}

fn axis_lens(binding: &Binding) -> Vec<usize> {
    binding.axes.iter().map(|a| a.keys.len().max(1)).collect()
}

fn product(lens: &[usize]) -> usize {
    lens.iter().product::<usize>().max(1)
}

// Walk every keyform of the new product and pull its data from the old one
// through `map` on the changed axis.
fn remap(old_lens: &[usize], axis: usize, map: &[KeySrc], channel: &mut Channel<'_>) {
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

fn with_object<R>(
    model: &mut Model,
    target: Target,
    f: impl FnOnce(&mut Binding, Vec<Channel<'_>>) -> R,
) -> Option<R> {
    match target {
        Target::Part(i) => {
            let p = model.parts.get_mut(i)?;
            let channels = vec![Channel::F32 { elems: 1, data: &mut p.draw_orders }];
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

/// Add keys at `values` to `target`'s binding on `param`. Objects not yet
/// bound on the parameter gain a new axis; existing shapes are preserved
/// exactly (new keys take the interpolated form). Returns true on change.
pub fn add_keys(model: &mut Model, target: Target, param: usize, values: &[f32]) -> bool {
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
                        map.push(KeySrc::Lerp(pos - 1, pos, t));
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
}

/// Shift one vertex by (dx, dy) in the mesh's parent space, across every
/// keyform. Blending is affine, so the evaluated vertex moves by exactly the
/// same amount whatever the current pose.
pub fn move_vertex(model: &mut Model, mesh: usize, vertex: usize, dx: f32, dy: f32) -> bool {
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
}

/// Shift several vertices at once, each by its own parent-space offset.
/// The deform path and brush tools land their falloff-weighted drags here.
pub fn move_vertices(model: &mut Model, mesh: usize, moves: &[(usize, f32, f32)]) -> bool {
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
}

/// Split the triangle `tri` of a mesh at the barycentric point (w0, w1, w2).
/// The new vertex is appended, so nothing else moves; its uv and every
/// keyform position are the same weighted mix of the triangle's corners,
/// which keeps all existing deformation intact.
pub fn add_vertex(model: &mut Model, mesh: usize, tri: usize, w: [f32; 3]) -> Option<usize> {
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
}

/// Add a vertex anywhere, placed by barycentric weights against `tri`.
/// Weights may be negative: outside the mesh the triangle's affine map is
/// extrapolated, so uv and every keyform stay consistent. With `connect`
/// the new vertex closes a triangle onto that edge; without it the vertex
/// starts loose, ready for the edge tool.
pub fn add_free_vertex(
    model: &mut Model,
    mesh: usize,
    tri: usize,
    w: [f32; 3],
    connect: Option<(u16, u16)>,
) -> Option<usize> {
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
    let Some(m) = model.art_meshes.get_mut(mesh) else { return false };
    let n = m.vertex_count as usize;
    if vertex >= n || n <= 3 {
        return false;
    }
    let v16 = vertex as u16;

    let mut indices = Vec::with_capacity(m.indices.len());
    let (tris, _) = m.indices.as_chunks::<3>();
    for t in tris {
        if t.contains(&v16) {
            continue;
        }
        for &i in t {
            indices.push(if i > v16 { i - 1 } else { i });
        }
    }
    m.indices = indices;

    let stride = n * 2;
    let keyforms = m.positions.len().checked_div(stride).unwrap_or(0);
    let mut positions = Vec::with_capacity(keyforms * (stride - 2));
    for k in 0..keyforms {
        let base = k * stride;
        for vi in 0..n {
            if vi == vertex {
                continue;
            }
            positions.push(m.positions.get(base + vi * 2).copied().unwrap_or(0.0));
            positions.push(m.positions.get(base + vi * 2 + 1).copied().unwrap_or(0.0));
        }
    }
    m.positions = positions;
    m.uvs.remove(vertex * 2 + 1);
    m.uvs.remove(vertex * 2);
    m.vertex_count -= 1;

    let mesh32 = mesh as u32;
    for g in &mut model.glues {
        if g.art_mesh_a != mesh32 && g.art_mesh_b != mesh32 {
            continue;
        }
        // pairs are (a, b) interleaved in weights/indices
        let affected = |slot: usize| {
            (slot.is_multiple_of(2) && g.art_mesh_a == mesh32)
                || (!slot.is_multiple_of(2) && g.art_mesh_b == mesh32)
        };
        let (pairs, _) = g.indices.as_chunks::<2>();
        let mut keep: Vec<bool> = Vec::with_capacity(pairs.len());
        for pair in pairs {
            let hit = pair.iter().enumerate().any(|(s, &i)| affected(s) && i == v16);
            keep.push(!hit);
        }
        let mut wi = 0usize;
        let mut new_w = Vec::with_capacity(g.weights.len());
        let mut new_i = Vec::with_capacity(g.indices.len());
        for (pi, pair) in pairs.iter().enumerate() {
            if keep.get(pi).copied().unwrap_or(false) {
                for (slot, &i) in pair.iter().enumerate() {
                    new_i.push(if affected(slot) && i > v16 { i - 1 } else { i });
                    new_w.push(g.weights.get(wi + slot).copied().unwrap_or(0.0));
                }
            }
            wi += 2;
        }
        g.weights = new_w;
        g.indices = new_i;
    }
    true
}

/// Swap in a whole new triangulation. Every new vertex samples the old mesh
/// in uv space: barycentric inside whichever old triangle holds it, nearest
/// old vertex otherwise, so every keyform carries over. Glue on this mesh
/// cannot survive a retriangulation and is cleared.
pub fn remesh(model: &mut Model, mesh: usize, uvs: &[f32], indices: &[u16]) -> bool {
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
        }
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
    let used = model.parts.iter().any(|p| uses(&p.binding))
        || model.deformers.iter().any(|d| uses(&d.binding))
        || model.art_meshes.iter().any(|m| uses(&m.binding))
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
    }
    for g in &mut model.glues {
        shift(&mut g.binding);
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
    use ayatsuri_format::model::{ArtMesh, Canvas, Parameter};

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
                },
                Parameter {
                    id: "B".into(),
                    min: 0.0,
                    max: 1.0,
                    default: 0.0,
                    repeat: false,
                    decimals: 3,
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
                vertex_count: 1,
                uvs: vec![0.0, 0.0],
                indices: vec![0, 0, 0],
                masks: vec![],
                binding: Binding { axes: vec![BindingAxis { parameter: 0, keys: vec![0.0, 1.0] }] },
                opacities: vec![1.0, 1.0],
                draw_orders: vec![500.0, 500.0],
                // the single vertex slides from x=0 to x=2
                positions: vec![0.0, 0.0, 2.0, 0.0],
            }],
            glues: vec![],
            draw_groups: vec![],
            physics: None,
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
        let vi = add_free_vertex(&mut m, 0, 0, [0.5, 0.75, -0.25], Some((0, 1)));
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
        add_free_vertex(&mut m, 0, 0, [-0.5, 0.75, 0.75], None).unwrap();
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
}
