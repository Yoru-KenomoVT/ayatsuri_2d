//! Skinning, as Cubism Editor does it: a long, thin mesh (a strand of hair,
//! a ribbon, a tail) gets a chain of rotation deformers along a path from
//! root to tip, one at each path point but the tip, each the child of the
//! one before. The mesh is cut into one piece per deformer, midway between
//! them, the pieces are glued at the cuts so they bend as one, and every
//! deformer gets a parameter of its own. The original mesh is hidden and
//! kept. Everything it makes is an ordinary deformer, mesh, glue and
//! parameter, so a skinned model exports to moc3 like any other.

use ayatsuri_format::model::{DeformerKind, DrawItem, Model};

use crate::edit::{self, Child, Target};

/// What skinning made, by index into the model.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Skin {
    /// The pieces, root first.
    pub pieces: Vec<usize>,
    /// The rotation deformers, root first.
    pub deformers: Vec<usize>,
    /// One parameter per deformer, root first.
    pub params: Vec<usize>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkinError {
    /// No such mesh.
    NoMesh,
    /// The path needs a root and a tip at least.
    TooFewPoints,
    /// A piece came out with no triangles: the path points are closer
    /// together than the mesh's triangles. Space them out or add points.
    EmptyPiece,
    /// One of the deformers given is not a rotation deformer.
    NotRotation,
}

/// Skin `mesh` along `path`, root to tip, in model units at the rest pose:
/// a new rotation deformer at each point but the tip, chained. Each
/// parameter runs from `-range` to `range` degrees and is named
/// `<id_base>1`, `<id_base>2`, ... (an id already taken gets a number on).
pub fn skin(
    model: &mut Model,
    mesh: usize,
    path: &[[f32; 2]],
    range: f32,
    id_base: &str,
) -> Result<Skin, SkinError> {
    let orig = model.art_meshes.get(mesh).cloned().ok_or(SkinError::NoMesh)?;
    let mut joints: Vec<[f32; 2]> = Vec::with_capacity(path.len());
    for &p in path {
        if joints.last().is_none_or(|q| dist(*q, p) > 1e-6) {
            joints.push(p);
        }
    }
    if joints.len() < 2 {
        return Err(SkinError::TooFewPoints);
    }
    let n_def = joints.len() - 1;
    let pieces = cut(model, mesh, &joints, n_def)?;

    // a rotation deformer at each joint but the tip, holding its piece,
    // then chained root to tip
    let rest = edit::rest_puppet(model);
    let mut ids = Vec::with_capacity(n_def);
    for (i, &piece) in pieces.iter().enumerate() {
        let joint = joints.get(i).copied().unwrap_or([0.0; 2]);
        let origin =
            edit::to_local(&rest, orig.parent_deformer.map(|p| p as usize), (joint[0], joint[1]));
        let id = unique_deformer_id(model, &format!("Rotation_{}_Skin{}", orig.id, i + 1));
        edit::create_rotation_deformer(model, &[piece], Some(origin), id.clone())
            .ok_or(SkinError::NoMesh)?;
        ids.push(id);
    }
    for w in ids.windows(2) {
        let (Some(parent), Some(child)) = (w.first(), w.get(1)) else { continue };
        if let (Some(p), Some(c)) = (index_of(model, parent), index_of(model, child)) {
            edit::set_parent(model, Child::Deformer(c), Some(p));
        }
    }
    let params = key_angles(model, &ids, range, id_base)?;
    glue_chain(model, &pieces);
    let deformers = ids.iter().filter_map(|id| index_of(model, id)).collect();
    Ok(Skin { pieces, deformers, params })
}

/// Skin `mesh` onto rotation deformers already placed along it, root to
/// tip (Cubism's Modeling > Skinning > Skinning): the mesh is cut midway
/// between their pivots, each piece goes under its deformer without moving,
/// and the pieces are glued at the cuts. No parameters are made; see
/// [`rotation_parameters`].
pub fn skin_with_rotations(
    model: &mut Model,
    mesh: usize,
    deformers: &[usize],
) -> Result<Skin, SkinError> {
    if deformers.len() < 2 {
        return Err(SkinError::TooFewPoints);
    }
    let rest = edit::rest_puppet(model);
    let mut ids = Vec::with_capacity(deformers.len());
    let mut joints = Vec::with_capacity(deformers.len());
    for &d in deformers {
        let def = model.deformers.get(d).ok_or(SkinError::NotRotation)?;
        let r = rest.rotation_state(d).ok_or(SkinError::NotRotation)?;
        ids.push(def.id.clone());
        joints.push([r.origin.x, r.origin.y]);
    }
    let pieces = cut(model, mesh, &joints, joints.len())?;
    for (&piece, id) in pieces.iter().zip(&ids) {
        if let Some(d) = index_of(model, id) {
            edit::set_parent(model, Child::Mesh(piece), Some(d));
        }
    }
    glue_chain(model, &pieces);
    let deformers = ids.iter().filter_map(|id| index_of(model, id)).collect();
    Ok(Skin { pieces, deformers, params: Vec::new() })
}

/// A parameter for each rotation deformer, keyed at `-range`, 0 and `range`
/// to its angle (Cubism's Generate Parameters for Rotation Deformer), named
/// `<id_base>1`, `<id_base>2`, ... in the order given.
pub fn rotation_parameters(
    model: &mut Model,
    deformers: &[usize],
    range: f32,
    id_base: &str,
) -> Result<Vec<usize>, SkinError> {
    let mut ids = Vec::with_capacity(deformers.len());
    for &d in deformers {
        let def = model.deformers.get(d).ok_or(SkinError::NotRotation)?;
        if !matches!(def.kind, DeformerKind::Rotation(_)) {
            return Err(SkinError::NotRotation);
        }
        ids.push(def.id.clone());
    }
    key_angles(model, &ids, range, id_base)
}

fn index_of(model: &Model, id: &str) -> Option<usize> {
    model.deformers.iter().position(|d| d.id == id)
}

/// Cut `mesh` into `n` pieces along `joints` (the first `n` of them are
/// what the pieces gather round), midway between neighbours; the pieces
/// take the original's place in the draw order and in other meshes' masks,
/// and the original is hidden.
fn cut(
    model: &mut Model,
    mesh: usize,
    joints: &[[f32; 2]],
    n: usize,
) -> Result<Vec<usize>, SkinError> {
    let orig = model.art_meshes.get(mesh).cloned().ok_or(SkinError::NoMesh)?;
    let mut along = vec![0.0f32];
    for w in joints.windows(2) {
        let (Some(a), Some(b)) = (w.first(), w.get(1)) else { continue };
        let last = along.last().copied().unwrap_or(0.0);
        along.push(last + dist(*a, *b));
    }
    let cuts: Vec<f32> = (0..n.saturating_sub(1))
        .map(|i| {
            let a = along.get(i).copied().unwrap_or(0.0);
            let b = along.get(i + 1).copied().unwrap_or(0.0);
            (a + b) * 0.5
        })
        .collect();

    // every triangle goes to the piece its centre falls in
    let rest = edit::rest_puppet(model);
    let world: Vec<[f32; 2]> = rest
        .mesh_geometry(mesh)
        .map(|g| g.positions.as_chunks::<2>().0.to_vec())
        .ok_or(SkinError::NoMesh)?;
    let mut tris: Vec<Vec<[u16; 3]>> = vec![Vec::new(); n];
    for t in orig.indices.as_chunks::<3>().0 {
        let at = |i: u16| world.get(i as usize).copied().unwrap_or([0.0; 2]);
        let (a, b, c) = (at(t[0]), at(t[1]), at(t[2]));
        let centre = [(a[0] + b[0] + c[0]) / 3.0, (a[1] + b[1] + c[1]) / 3.0];
        let s = position_along(joints, &along, centre);
        let piece = cuts.iter().filter(|&&c| s >= c).count();
        if let Some(list) = tris.get_mut(piece) {
            list.push(*t);
        }
    }
    if tris.iter().any(Vec::is_empty) {
        return Err(SkinError::EmptyPiece);
    }

    // the pieces: the original's art, rig and settings, cut down to their
    // triangles; a vertex on a cut goes into both pieces it borders
    let keyforms = orig.opacities.len().max(1);
    let vc = orig.vertex_count as usize;
    let mut pieces = Vec::with_capacity(n);
    for (pi, list) in tris.iter().enumerate() {
        let mut used: Vec<u16> = list.iter().flatten().copied().collect();
        used.sort_unstable();
        used.dedup();
        let local = |v: u16| used.binary_search(&v).unwrap_or(0) as u16;
        let mut piece = orig.clone();
        piece.id = unique_mesh_id(model, &format!("{}_Skin{}", orig.id, pi + 1));
        piece.visible = true;
        piece.vertex_count = used.len() as u32;
        piece.indices = list.iter().flat_map(|t| t.map(local)).collect();
        piece.uvs = used
            .iter()
            .flat_map(|&v| {
                let at = v as usize * 2;
                [
                    orig.uvs.get(at).copied().unwrap_or(0.0),
                    orig.uvs.get(at + 1).copied().unwrap_or(0.0),
                ]
            })
            .collect();
        piece.positions = (0..keyforms)
            .flat_map(|k| {
                let orig = &orig;
                used.iter().flat_map(move |&v| {
                    let at = (k * vc + v as usize) * 2;
                    [
                        orig.positions.get(at).copied().unwrap_or(0.0),
                        orig.positions.get(at + 1).copied().unwrap_or(0.0),
                    ]
                })
            })
            .collect();
        model.art_meshes.push(piece);
        pieces.push(model.art_meshes.len() - 1);
    }
    place_in_draw_order(model, mesh, &pieces);
    // what the original clipped, its pieces clip now
    for m in &mut model.art_meshes {
        if m.masks.contains(&(mesh as u32)) {
            for &p in &pieces {
                m.masks.push(p as u32);
            }
        }
    }
    if let Some(m) = model.art_meshes.get_mut(mesh) {
        m.visible = false;
    }
    Ok(pieces)
}

/// Glue each cut: the shared vertices sit on top of each other.
fn glue_chain(model: &mut Model, pieces: &[usize]) {
    let rest = edit::rest_puppet(model);
    let geo = |p: usize| rest.mesh_geometry(p).map(|g| g.positions.to_vec()).unwrap_or_default();
    let worlds: Vec<Vec<f32>> = pieces.iter().map(|&p| geo(p)).collect();
    for (w, g) in pieces.windows(2).zip(worlds.windows(2)) {
        let (Some(&a), Some(&b), Some(wa), Some(wb)) = (w.first(), w.get(1), g.first(), g.get(1))
        else {
            continue;
        };
        edit::bind_glue(model, a, b, wa, wb, 1e-3);
    }
}

/// A parameter per deformer (by id), keyed at -range, 0 and range to its
/// angle.
fn key_angles(
    model: &mut Model,
    ids: &[String],
    range: f32,
    id_base: &str,
) -> Result<Vec<usize>, SkinError> {
    let range = range.abs().max(1.0);
    let mut params = Vec::with_capacity(ids.len());
    for (i, id) in ids.iter().enumerate() {
        let pid = unique_param_id(model, &format!("{id_base}{}", i + 1));
        let p = edit::create_parameter(model, pid, -range, 0.0, range).ok_or(SkinError::NoMesh)?;
        params.push(p);
        let Some(d) = index_of(model, id) else { continue };
        edit::add_keys(model, Target::Deformer(d), p, &[-range, 0.0, range]);
        if let Some(def) = model.deformers.get_mut(d)
            && let DeformerKind::Rotation(r) = &mut def.kind
            && let Some(axis) = def.binding.axes.iter().find(|a| a.parameter as usize == p)
        {
            for (k, key) in axis.keys.iter().enumerate() {
                if let Some(a) = r.angles.get_mut(k) {
                    *a = *key;
                }
            }
        }
    }
    Ok(params)
}

/// Skin `mesh` along path points pinned to its triangles (a triangle and
/// barycentric weights each, root to tip), as the editor's deform paths
/// keep them; the points are read off the mesh's rest shape, whatever pose
/// the editor shows.
pub fn skin_from_pins(
    model: &mut Model,
    mesh: usize,
    pins: &[(usize, [f32; 3])],
    range: f32,
    id_base: &str,
) -> Result<Skin, SkinError> {
    let rest = edit::rest_puppet(model);
    let geo = rest.mesh_geometry(mesh).ok_or(SkinError::NoMesh)?;
    let pts = geo.positions.as_chunks::<2>().0;
    let tris = geo.indices.as_chunks::<3>().0;
    let path: Vec<[f32; 2]> = pins
        .iter()
        .filter_map(|&(tri, w)| {
            let t = tris.get(tri)?;
            let (a, b, c) =
                (pts.get(t[0] as usize)?, pts.get(t[1] as usize)?, pts.get(t[2] as usize)?);
            Some([a[0] * w[0] + b[0] * w[1] + c[0] * w[2], a[1] * w[0] + b[1] * w[1] + c[1] * w[2]])
        })
        .collect();
    skin(model, mesh, &path, range, id_base)
}

fn dist(a: [f32; 2], b: [f32; 2]) -> f32 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2)).sqrt()
}

/// How far along the path the point nearest `p` is, past either end
/// counted as before the root or after the tip.
fn position_along(joints: &[[f32; 2]], along: &[f32], p: [f32; 2]) -> f32 {
    let mut best = (f32::MAX, 0.0f32);
    let last = joints.len().saturating_sub(2);
    for (i, w) in joints.windows(2).enumerate() {
        let (Some(a), Some(b)) = (w.first(), w.get(1)) else { continue };
        let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
        let len2 = (dx * dx + dy * dy).max(1e-12);
        let mut t = ((p[0] - a[0]) * dx + (p[1] - a[1]) * dy) / len2;
        // the first segment reaches back past the root, the last past the tip
        if i > 0 {
            t = t.max(0.0);
        }
        if i < last {
            t = t.min(1.0);
        }
        let q = [a[0] + dx * t, a[1] + dy * t];
        let d = dist(p, q);
        if d < best.0 {
            best = (d, along.get(i).copied().unwrap_or(0.0) + t * len2.sqrt());
        }
    }
    best.1
}

/// The pieces join the original's draw-order group, right after it.
fn place_in_draw_order(model: &mut Model, mesh: usize, pieces: &[usize]) {
    let found = model.draw_groups.iter().enumerate().find_map(|(gi, g)| {
        g.items.iter().position(|it| !it.part && it.object as usize == mesh).map(|at| (gi, at))
    });
    let Some((gi, at)) = found else { return };
    if let Some(g) = model.draw_groups.get_mut(gi) {
        for (k, &p) in pieces.iter().enumerate() {
            g.items
                .insert(at + 1 + k, DrawItem { part: false, object: p as u32, child_group: None });
        }
    }
    // the group and every group holding it count its objects
    let mut cur = Some(gi);
    for _ in 0..model.draw_groups.len() {
        let Some(c) = cur else { break };
        if let Some(g) = model.draw_groups.get_mut(c) {
            g.total += pieces.len() as u32;
        }
        cur = model
            .draw_groups
            .iter()
            .position(|g| g.items.iter().any(|it| it.child_group == Some(c as u32)));
    }
}

fn unique(taken: impl Fn(&str) -> bool, base: &str) -> String {
    if !taken(base) {
        return base.to_string();
    }
    (2..).map(|n| format!("{base}_{n}")).find(|c| !taken(c)).unwrap_or_else(|| base.to_string())
}

fn unique_mesh_id(model: &Model, base: &str) -> String {
    unique(|c| model.art_meshes.iter().any(|m| m.id == c), base)
}

fn unique_deformer_id(model: &Model, base: &str) -> String {
    unique(|c| model.deformers.iter().any(|d| d.id == c), base)
}

fn unique_param_id(model: &Model, base: &str) -> String {
    unique(|c| model.parameters.iter().any(|p| p.id == c), base)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ayatsuri_format::model::{ArtMesh, Binding, DrawGroup};

    /// A 1 x 6 strip of quads along x, as a hair strand would be.
    fn strip() -> Model {
        let mut m = Model::new();
        let cols = 6usize;
        let mut positions = Vec::new();
        let mut uvs = Vec::new();
        for c in 0..=cols {
            for r in 0..2 {
                positions.extend_from_slice(&[c as f32, r as f32 * 0.5]);
                uvs.extend_from_slice(&[c as f32 / cols as f32, r as f32]);
            }
        }
        let mut indices = Vec::new();
        for c in 0..cols as u16 {
            let (a, b, cc, d) = (c * 2, c * 2 + 1, c * 2 + 2, c * 2 + 3);
            indices.extend_from_slice(&[a, b, cc, b, d, cc]);
        }
        m.art_meshes.push(ArtMesh {
            id: "Hair".into(),
            parent_part: None,
            parent_deformer: None,
            visible: true,
            enabled: true,
            texture: 0,
            flags: 0,
            blend: Default::default(),
            vertex_count: ((cols + 1) * 2) as u32,
            uvs,
            indices,
            masks: vec![],
            binding: Binding::default(),
            opacities: vec![1.0],
            draw_orders: vec![500.0],
            positions,
            blend_shapes: Vec::new(),
        });
        m.draw_groups.push(DrawGroup {
            total: 1,
            min_order: 500,
            max_order: 500,
            items: vec![DrawItem { part: false, object: 0, child_group: None }],
        });
        m
    }

    #[test]
    fn a_strip_skins_into_glued_pieces_on_a_chain() {
        let mut m = strip();
        let s = skin(
            &mut m,
            0,
            &[[0.0, 0.25], [2.0, 0.25], [4.0, 0.25], [6.0, 0.25]],
            45.0,
            "ParamHair",
        )
        .unwrap();
        assert_eq!(s.pieces.len(), 3);
        assert_eq!(s.deformers.len(), 3);
        assert_eq!(s.params.len(), 3);
        assert!(!m.art_meshes[0].visible, "the original is hidden");
        // a chain: each deformer the child of the one before
        for w in s.deformers.windows(2) {
            assert_eq!(m.deformers[w[1]].parent_deformer, Some(w[0] as u32));
        }
        // every piece under its own deformer, glued to the next
        for (p, d) in s.pieces.iter().zip(&s.deformers) {
            assert_eq!(m.art_meshes[*p].parent_deformer, Some(*d as u32));
        }
        assert_eq!(m.glues.len(), 2);
        assert!(m.glues.iter().all(|g| !g.indices.is_empty()));
        // every triangle went somewhere, and the pieces draw where it did
        let tris: usize = s.pieces.iter().map(|&p| m.art_meshes[p].indices.len() / 3).sum();
        assert_eq!(tris, m.art_meshes[0].indices.len() / 3);
        assert_eq!(m.draw_groups[0].total, 4);
        assert_eq!(m.parameters[s.params[0]].min, -45.0);
    }

    #[test]
    fn nothing_moves_at_rest() {
        let mut m = strip();
        let before = edit::rest_puppet(&m).mesh_geometry(0).unwrap().positions.to_vec();
        let s =
            skin(&mut m, 0, &[[0.0, 0.25], [3.0, 0.25], [6.0, 0.25]], 30.0, "ParamHair").unwrap();
        let rest = edit::rest_puppet(&m);
        // every piece vertex sits where the original's did
        for &p in &s.pieces {
            for q in rest.mesh_geometry(p).unwrap().positions.as_chunks::<2>().0 {
                let hit = before
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .any(|o| (o[0] - q[0]).abs() < 1e-3 && (o[1] - q[1]).abs() < 1e-3);
                assert!(hit, "{q:?} moved");
            }
        }
    }

    #[test]
    fn turning_a_parameter_bends_the_tip() {
        let mut m = strip();
        let s =
            skin(&mut m, 0, &[[0.0, 0.25], [3.0, 0.25], [6.0, 0.25]], 30.0, "ParamHair").unwrap();
        let mut puppet = crate::Puppet::new(m.clone());
        puppet.set_value(s.params[0], 30.0);
        puppet.update();
        let tip = s.pieces[1];
        let moved = puppet.mesh_geometry(tip).unwrap().positions.to_vec();
        let rest = edit::rest_puppet(&m).mesh_geometry(tip).unwrap().positions.to_vec();
        let far = moved
            .chunks(2)
            .zip(rest.chunks(2))
            .map(|(a, b)| (a[1] - b[1]).abs())
            .fold(0.0, f32::max);
        assert!(far > 0.5, "the tip swings: {far}");
    }

    /// Two rotation deformers placed by hand along the strip, root to tip.
    fn with_rotations() -> Model {
        use ayatsuri_format::model::{Deformer, RotationData};
        let mut m = strip();
        for (i, x) in [0.0f32, 3.0].into_iter().enumerate() {
            m.deformers.push(Deformer {
                id: format!("Rot{i}"),
                parent_part: None,
                parent_deformer: (i > 0).then_some(0),
                visible: true,
                enabled: true,
                binding: Binding::default(),
                kind: DeformerKind::Rotation(RotationData {
                    base_angle: 0.0,
                    opacities: vec![1.0],
                    angles: vec![0.0],
                    // the second sits under the first, in its space
                    origins_x: vec![if i == 0 { x } else { x - 0.0 }],
                    origins_y: vec![if i == 0 { 0.25 } else { 0.0 }],
                    scales: vec![1.0],
                    reflect_x: vec![false],
                    reflect_y: vec![false],
                }),
            });
        }
        m
    }

    #[test]
    fn a_mesh_skins_onto_rotation_deformers_placed_by_hand() {
        let mut m = with_rotations();
        let before = edit::rest_puppet(&m).mesh_geometry(0).unwrap().positions.to_vec();
        let s = skin_with_rotations(&mut m, 0, &[0, 1]).unwrap();
        assert_eq!(s.pieces.len(), 2);
        assert!(s.params.is_empty());
        assert!(!m.art_meshes[0].visible);
        assert_eq!(m.glues.len(), 1);
        for (p, d) in s.pieces.iter().zip(&s.deformers) {
            assert_eq!(m.art_meshes[*p].parent_deformer, Some(*d as u32));
        }
        // nothing moved
        let rest = edit::rest_puppet(&m);
        for &p in &s.pieces {
            for q in rest.mesh_geometry(p).unwrap().positions.as_chunks::<2>().0 {
                let hit = before
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .any(|o| (o[0] - q[0]).abs() < 1e-3 && (o[1] - q[1]).abs() < 1e-3);
                assert!(hit, "{q:?} moved");
            }
        }
        // then a parameter each, keyed to the angle
        let params = rotation_parameters(&mut m, &s.deformers, 30.0, "ParamHairRot").unwrap();
        assert_eq!(params.len(), 2);
        assert_eq!(m.parameters[params[0]].min, -30.0);
        let mut puppet = crate::Puppet::new(m.clone());
        puppet.set_value(params[1], 30.0);
        puppet.update();
        let tip = puppet.mesh_geometry(s.pieces[1]).unwrap().positions.to_vec();
        let far = tip
            .chunks(2)
            .zip(before.chunks(2))
            .map(|(a, b)| (a[1] - b[1]).abs())
            .fold(0.0, f32::max);
        assert!(far > 0.3, "the tip swings: {far}");
    }

    #[test]
    fn skinning_onto_a_warp_is_refused() {
        let mut m = with_rotations();
        assert_eq!(rotation_parameters(&mut m, &[5], 30.0, "P"), Err(SkinError::NotRotation));
        assert_eq!(skin_with_rotations(&mut m, 0, &[0]), Err(SkinError::TooFewPoints));
    }

    #[test]
    fn points_too_close_for_the_mesh_are_refused() {
        let mut m = strip();
        let r = skin(&mut m, 0, &[[0.0, 0.25], [0.05, 0.25], [0.1, 0.25], [6.0, 0.25]], 45.0, "P");
        assert_eq!(r, Err(SkinError::EmptyPiece));
        assert_eq!(skin(&mut strip(), 0, &[[0.0, 0.0]], 45.0, "P"), Err(SkinError::TooFewPoints));
    }
}
