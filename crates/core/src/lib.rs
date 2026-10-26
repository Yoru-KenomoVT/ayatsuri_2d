//! Deformation, physics, masking, animation and draw order.
//!
//! The rules this crate lives by (docs/goals.md):
//! - no I/O and no renderer: it consumes `ayatsuri-format` types and produces
//!   vertex positions and draw lists;
//! - allocation happens at load, the frame loop allocates nothing;
//! - never depends on `openmoc3` (docs/architecture.md, dependency rules).

#![forbid(unsafe_code)]

pub mod anim;
pub mod artpath;
pub mod automesh;
pub mod bezier;
pub mod blend;
pub mod blendshape;
pub mod clip;
pub mod comp;
pub mod compose;
pub mod deform;
pub mod draworder;
pub mod edit;
pub mod keyform;
pub mod palette;
pub mod physics;
pub mod skin;

use ayatsuri_format::model::{ArtMesh, Blend, BlendShape, DeformerKind, Model, Parameter};
use deform::{Rotation, Vec2, rotation_transform, signed_angle, v2, warp_transform};

/// A loaded puppet: the model plus every buffer the frame loop needs,
/// allocated once here.
pub struct Puppet {
    model: Model,
    /// Current parameter values, resolved (clamped or wrapped).
    values: Vec<f32>,
    /// Per parameter: (snap epsilon, interpolation epsilon), 10^-decimals.
    eps: Vec<(f32, f32)>,
    /// Per part: user opacity input, propagated opacity, enabled.
    part_user_opacity: Vec<f32>,
    part_opacity: Vec<f32>,
    part_enabled: Vec<bool>,
    part_draw_order: Vec<f32>,
    /// Per part: an offscreen part's layer opacity this frame (1.0 for the
    /// rest).
    layer_opacity: Vec<f32>,
    /// Per mesh: the offscreen parts it draws inside, outermost first.
    mesh_layers: Vec<Vec<u32>>,
    /// Per mesh: the mask sets it inherits past its own masks (see `clip`).
    mesh_clip_and: Vec<Vec<Vec<u32>>>,
    /// The deepest offscreen nesting any mesh has (0: no layers at all).
    layer_depth: usize,
    /// Per part: its lowest mesh, the tie order of an offscreen part's item.
    part_tie: Vec<u32>,
    /// Per draw order group: its total, counted the runtime's way (see
    /// `draworder`), offscreen layer slots included.
    group_total: Vec<u32>,
    // grouped walk workspace, sized at load: the render slots (meshes and
    // offscreen layers) and which meshes found one
    slot_buf: Vec<u32>,
    mesh_seen: Vec<bool>,
    /// Per deformer: chain state.
    def_enabled: Vec<bool>,
    def_user_visible: Vec<bool>,
    def_opacity: Vec<f32>,
    def_scale: Vec<f32>,
    /// Which warp/rotation each deformer is, locally.
    def_local: Vec<u32>,
    /// Blended-then-transformed warp grids, one flat buffer.
    warp_grid: Vec<f32>,
    warp_grid_off: Vec<usize>,
    warp_grid_len: Vec<usize>,
    /// Resolved rotation states.
    rot: Vec<Rotation>,
    /// Blended-then-transformed mesh vertices, one flat buffer.
    mesh_pos: Vec<f32>,
    mesh_pos_off: Vec<usize>,
    mesh_opacity: Vec<f32>,
    mesh_draw_order: Vec<f32>,
    mesh_visible: Vec<bool>,
    mesh_user_visible: Vec<bool>,
    /// Mesh indices, sorted back to front.
    render_order: Vec<u32>,
    /// Meshes other meshes clip by; they keep evaluating while hidden.
    mask_source: Vec<bool>,
    // draw-order-group sort workspace, sized at load
    dog_first: Vec<i32>,
    dog_last: Vec<i32>,
    dog_next: Vec<i32>,
    dog_cursor: Vec<u32>,
    dog_order: Vec<f32>,
    /// Scratch, reused every frame.
    corners: Vec<blend::Corner>,
    scratch: Vec<f32>,
    /// A mesh's keyform weights at the default pose, for its blend shapes'
    /// base form.
    shape_corners: Vec<blend::Corner>,
    /// The pendulum rig's live state, when the model carries one.
    physics: Option<physics::Physics>,
    /// Work skipped when nothing changed. Each deformer and mesh remembers
    /// the keyform weights its geometry was last made from; when they match
    /// this frame's bit for bit, its parent did not move and the cache is
    /// valid, last frame's geometry stands. Glued meshes are always redone
    /// (glue moves their points after they are made), and any in-place edit
    /// of the model's keyforms invalidates everything.
    cache: Cache,
}

fn shape_value(values: &[f32], shape: &BlendShape) -> f32 {
    values.get(shape.parameter as usize).copied().unwrap_or(0.0)
}

impl Cache {
    /// The mesh was last made at these blend shape values, bit for bit.
    fn mesh_shapes_same(&self, mi: usize, shapes: &[BlendShape], values: &[f32]) -> bool {
        self.mesh_shapes.get(mi).is_some_and(|seen| {
            seen.len() == shapes.len()
                && seen
                    .iter()
                    .zip(shapes)
                    .all(|(v, s)| v.to_bits() == shape_value(values, s).to_bits())
        })
    }

    fn remember_mesh(
        &mut self,
        mi: usize,
        corners: &[blend::Corner],
        shapes: &[BlendShape],
        values: &[f32],
    ) {
        if let Some(c) = self.mesh_corners.get_mut(mi) {
            c.clear();
            c.extend_from_slice(corners);
        }
        if let Some(seen) = self.mesh_shapes.get_mut(mi) {
            seen.clear();
            seen.extend(shapes.iter().map(|s| shape_value(values, s)));
        }
        if let Some(m) = self.mesh_made.get_mut(mi) {
            *m = true;
        }
    }
}

#[derive(Default)]
struct Cache {
    /// False after an in-place keyform edit, until the next full update.
    valid: bool,
    def_corners: Vec<Vec<blend::Corner>>,
    /// Made from `def_corners` and still standing.
    def_made: Vec<bool>,
    /// Changed this frame: children redo theirs.
    def_moved: Vec<bool>,
    mesh_corners: Vec<Vec<blend::Corner>>,
    /// The blend shape parameter values each mesh was made at.
    mesh_shapes: Vec<Vec<f32>>,
    mesh_made: Vec<bool>,
    /// Held by glue: always redone.
    mesh_glued: Vec<bool>,
}

impl Puppet {
    pub fn new(model: Model) -> Self {
        let values: Vec<f32> = model.parameters.iter().map(|p| p.default).collect();
        let eps = model
            .parameters
            .iter()
            .map(|p| {
                let snap = 0.1f32.powi(p.decimals.clamp(0, 10));
                (snap, snap * 1.5)
            })
            .collect();

        let n_parts = model.parts.len();
        let n_defs = model.deformers.len();
        let n_meshes = model.art_meshes.len();

        let mut def_local = Vec::with_capacity(n_defs);
        let mut warp_grid_off = Vec::new();
        let mut warp_grid_len = Vec::new();
        let mut rot_count = 0u32;
        let mut warp_count = 0u32;
        let mut grid_total = 0usize;
        for d in &model.deformers {
            match &d.kind {
                DeformerKind::Warp(w) => {
                    def_local.push(warp_count);
                    let len = w.grid_points().saturating_mul(2);
                    warp_grid_off.push(grid_total);
                    warp_grid_len.push(len);
                    grid_total = grid_total.saturating_add(len);
                    warp_count += 1;
                }
                DeformerKind::Rotation(_) => {
                    def_local.push(rot_count);
                    rot_count += 1;
                }
            }
        }

        let mut mesh_pos_off = Vec::with_capacity(n_meshes);
        let mut pos_total = 0usize;
        for m in &model.art_meshes {
            mesh_pos_off.push(pos_total);
            pos_total = pos_total.saturating_add((m.vertex_count as usize).saturating_mul(2));
        }
        let mesh_clip_and: Vec<Vec<Vec<u32>>> = (0..n_meshes)
            .map(|mi| clip::chain_of(&model, mi).0.into_iter().skip(1).collect())
            .collect();
        let mut mask_source = vec![false; n_meshes];
        for &src in mesh_clip_and.iter().flatten().flatten() {
            if let Some(f) = mask_source.get_mut(src as usize) {
                *f = true;
            }
        }
        let layer_masks =
            model.parts.iter().filter_map(|p| p.offscreen.as_ref()).flat_map(|o| &o.masks);
        for &src in model.art_meshes.iter().flat_map(|m| &m.masks).chain(layer_masks) {
            if let Some(f) = mask_source.get_mut(src as usize) {
                *f = true;
            }
        }
        // each mesh's offscreen parts, walking up (bounded, so a parent loop
        // in a broken file cannot spin)
        let mesh_layers: Vec<Vec<u32>> = model
            .art_meshes
            .iter()
            .map(|m| {
                let mut chain = Vec::new();
                let mut at = m.parent_part;
                for _ in 0..n_parts {
                    let Some(pi) = at else { break };
                    let Some(p) = model.parts.get(pi as usize) else { break };
                    if p.offscreen.is_some() {
                        chain.push(pi);
                    }
                    at = p.parent_part;
                }
                chain.reverse();
                chain
            })
            .collect();
        let layer_depth = mesh_layers.iter().map(Vec::len).max().unwrap_or(0);

        let max_corners = model
            .parts
            .iter()
            .map(|p| &p.binding)
            .chain(model.deformers.iter().map(|d| &d.binding))
            .chain(model.art_meshes.iter().map(|m| &m.binding))
            .chain(model.glues.iter().map(|g| &g.binding))
            .map(|b| 1usize << b.axes.len().min(16))
            .max()
            .unwrap_or(1);
        let max_grid = warp_grid_len.iter().copied().max().unwrap_or(0);
        let max_mesh =
            model.art_meshes.iter().map(|m| (m.vertex_count as usize) * 2).max().unwrap_or(0);

        let max_level = model
            .draw_groups
            .iter()
            .map(|g| (g.max_order.saturating_sub(g.min_order).max(0) as usize).saturating_add(1))
            .max()
            .unwrap_or(1);
        let max_items = model.draw_groups.iter().map(|g| g.items.len()).max().unwrap_or(1);

        Self {
            values,
            eps,
            part_user_opacity: vec![1.0; n_parts],
            part_opacity: vec![1.0; n_parts],
            part_enabled: vec![true; n_parts],
            part_draw_order: vec![500.0; n_parts],
            layer_opacity: vec![1.0; n_parts],
            mesh_layers,
            mesh_clip_and,
            layer_depth,
            part_tie: draworder::part_first_mesh(&model),
            group_total: draworder::group_totals(&model),
            slot_buf: vec![
                u32::MAX;
                n_meshes + model.parts.iter().filter(|p| p.offscreen.is_some()).count()
            ],
            mesh_seen: vec![false; n_meshes],
            dog_first: vec![-1; max_level],
            dog_last: vec![-1; max_level],
            dog_next: vec![-1; max_items],
            dog_cursor: vec![0; model.draw_groups.len()],
            dog_order: vec![500.0; max_items],
            cache: Cache {
                valid: false,
                def_corners: vec![Vec::new(); n_defs],
                def_made: vec![false; n_defs],
                def_moved: vec![true; n_defs],
                mesh_corners: vec![Vec::new(); n_meshes],
                mesh_shapes: model
                    .art_meshes
                    .iter()
                    .map(|m| Vec::with_capacity(m.blend_shapes.len()))
                    .collect(),
                mesh_made: vec![false; n_meshes],
                mesh_glued: {
                    let mut g = vec![false; n_meshes];
                    for glue in &model.glues {
                        for m in [glue.art_mesh_a, glue.art_mesh_b] {
                            if let Some(x) = g.get_mut(m as usize) {
                                *x = true;
                            }
                        }
                    }
                    g
                },
            },
            def_enabled: vec![true; n_defs],
            def_user_visible: vec![true; n_defs],
            def_opacity: vec![1.0; n_defs],
            def_scale: vec![1.0; n_defs],
            def_local,
            warp_grid: vec![0.0; grid_total],
            warp_grid_off,
            warp_grid_len,
            rot: vec![Rotation::default(); rot_count as usize],
            mesh_pos: vec![0.0; pos_total],
            mesh_pos_off,
            mesh_opacity: vec![0.0; n_meshes],
            mesh_draw_order: vec![500.0; n_meshes],
            mesh_visible: vec![false; n_meshes],
            mesh_user_visible: vec![true; n_meshes],
            render_order: (0..n_meshes as u32).collect(),
            mask_source,
            corners: Vec::with_capacity(max_corners),
            scratch: vec![0.0; max_grid.max(max_mesh)],
            shape_corners: Vec::with_capacity(max_corners),
            physics: physics::Physics::new(&model),
            model,
        }
    }

    pub fn has_physics(&self) -> bool {
        self.physics.is_some()
    }

    /// Advance the pendulum rig by `dt` seconds and write its outputs into
    /// the parameter values. Returns true when anything moved; the caller
    /// follows up with `update()` as usual.
    pub fn step_physics(&mut self, dt: f32) -> bool {
        let Some(mut ph) = self.physics.take() else { return false };
        let parameters = &self.model.parameters;
        let moved = ph.step(dt, &mut self.values, |p| {
            parameters.get(p).map(|q| (q.min, q.max, q.default)).unwrap_or((0.0, 1.0, 0.0))
        });
        self.physics = Some(ph);
        moved
    }

    /// Keep physics off one parameter while the user drags its slider, so a
    /// physics-driven parameter can still be moved by hand; empty lets go.
    pub fn hold_physics_params(&mut self, params: &[usize]) {
        if let Some(ph) = &mut self.physics {
            ph.hold(params);
        }
    }

    /// Settle every strand back to rest.
    pub fn reset_physics(&mut self) {
        if let Some(ph) = &mut self.physics {
            ph.reset();
        }
    }

    pub fn set_physics_wind(&mut self, x: f32, y: f32) {
        if let Some(ph) = &mut self.physics {
            ph.set_wind(x, y);
        }
    }

    pub fn physics_wind(&self) -> (f32, f32) {
        self.physics.as_ref().map(|p| p.wind()).unwrap_or((0.0, 0.0))
    }

    /// The live pendulum chain of one rig group, for the physics dialog.
    pub fn physics_group_nodes(&self, rig_group: usize) -> Option<Vec<(f32, f32)>> {
        self.physics.as_ref().and_then(|p| p.group_nodes(rig_group))
    }

    /// How far one rig group's frame is tilted by its angle inputs, in
    /// degrees: the physics dialog's rotating cross.
    pub fn physics_group_tilt(&self, rig_group: usize) -> Option<f32> {
        self.physics.as_ref().and_then(|p| p.group_tilt(rig_group))
    }

    /// The physics dialog's per-group Valid checkbox.
    pub fn set_physics_group_enabled(&mut self, rig_group: usize, on: bool) {
        if let Some(ph) = &mut self.physics {
            ph.set_group_enabled(rig_group, on);
        }
    }

    pub fn physics_group_enabled(&self, rig_group: usize) -> bool {
        self.physics.as_ref().is_none_or(|p| p.group_enabled(rig_group))
    }

    /// Output Maximum (%) per output of one rig group, dialog fodder.
    pub fn physics_output_peaks(&self, rig_group: usize) -> Option<Vec<(usize, f32)>> {
        self.physics.as_ref().and_then(|p| p.output_peaks(rig_group))
    }

    pub fn reset_physics_output_peaks(&mut self) {
        if let Some(ph) = &mut self.physics {
            ph.reset_output_peaks();
        }
    }

    /// The evaluated draw order of one mesh at the current pose.
    pub fn mesh_draw_order_value(&self, mesh: usize) -> f32 {
        self.mesh_draw_order.get(mesh).copied().unwrap_or(500.0)
    }

    pub fn model(&self) -> &Model {
        &self.model
    }

    /// One keyform's stored vertex positions of a mesh, editable in place
    /// for a live drag preview. Only the numbers change, never the shape of
    /// the data, so nothing derived from the model needs rebuilding; the
    /// next `update` shows it. Callers restore and commit through an edit.
    pub fn mesh_keyform_mut(&mut self, mesh: usize, keyform: usize) -> Option<&mut [f32]> {
        self.cache.valid = false;
        keyform::mesh_form_mut(self.model.art_meshes.get_mut(mesh)?, keyform)
    }

    /// One keyform's opacity of an art mesh or deformer, editable in place
    /// for a live slider preview, the same way.
    pub fn opacity_mut(&mut self, target: edit::Target, keyform: usize) -> Option<&mut f32> {
        self.cache.valid = false;
        match target {
            edit::Target::ArtMesh(i) => {
                self.model.art_meshes.get_mut(i)?.opacities.get_mut(keyform)
            }
            edit::Target::Deformer(i) => match &mut self.model.deformers.get_mut(i)?.kind {
                DeformerKind::Warp(w) => w.opacities.get_mut(keyform),
                DeformerKind::Rotation(r) => r.opacities.get_mut(keyform),
            },
            edit::Target::Part(i) => {
                self.model.parts.get_mut(i)?.offscreen.as_mut()?.opacities.get_mut(keyform)
            }
        }
    }

    /// A rotation deformer's keyform angles, editable in place for a live
    /// turn preview, the same way.
    pub fn rotation_angles_mut(&mut self, deformer: usize) -> Option<&mut [f32]> {
        self.cache.valid = false;
        let d = self.model.deformers.get_mut(deformer)?;
        let DeformerKind::Rotation(r) = &mut d.kind else { return None };
        Some(r.angles.as_mut_slice())
    }

    /// One keyform's warp grid, editable in place the same way.
    pub fn warp_keyform_mut(&mut self, deformer: usize, keyform: usize) -> Option<&mut [f32]> {
        self.cache.valid = false;
        let d = self.model.deformers.get_mut(deformer)?;
        let DeformerKind::Warp(w) = &mut d.kind else { return None };
        let points = (w.rows as usize + 1).checked_mul(w.cols as usize + 1)?;
        let stride = points.checked_mul(2)?;
        let start = keyform.checked_mul(stride)?;
        w.positions.get_mut(start..start.checked_add(stride)?)
    }

    pub fn value(&self, parameter: usize) -> f32 {
        self.values.get(parameter).copied().unwrap_or(0.0)
    }

    pub fn set_value(&mut self, parameter: usize, value: f32) {
        if let Some(v) = self.values.get_mut(parameter) {
            *v = value;
        }
    }

    /// Editor show/hide for one mesh, independent of the rig's own flags.
    pub fn set_mesh_visible(&mut self, mesh: usize, visible: bool) {
        if let Some(v) = self.mesh_user_visible.get_mut(mesh) {
            *v = visible;
        }
    }

    /// Editor show/hide for a deformer; its whole subtree goes with it.
    pub fn set_deformer_visible(&mut self, deformer: usize, visible: bool) {
        if let Some(v) = self.def_user_visible.get_mut(deformer) {
            *v = visible;
        }
    }

    /// The opacity a part was given with `set_part_opacity` (1.0 unless set).
    pub fn part_user_opacity(&self, part: usize) -> f32 {
        self.part_user_opacity.get(part).copied().unwrap_or(1.0)
    }

    pub fn set_part_opacity(&mut self, part: usize, opacity: f32) {
        if let Some(o) = self.part_user_opacity.get_mut(part) {
            *o = opacity.clamp(0.0, 1.0);
        }
    }

    /// Evaluate the whole puppet at the current parameter values.
    /// Allocates nothing.
    pub fn update(&mut self) {
        self.resolve_params();
        self.update_parts();
        self.update_deformers();
        self.update_meshes();
        self.apply_glues();
        self.sort_render_order();
        self.cache.valid = true;
    }

    /// Forget what the last update made, so the next one recomputes every
    /// deformer and mesh. Never needed for correctness; tests use it to
    /// check skipped work against a full pass.
    pub fn invalidate_cache(&mut self) {
        self.cache.valid = false;
    }

    fn resolve_params(&mut self) {
        for (i, p) in self.model.parameters.iter().enumerate() {
            let Some(v) = self.values.get_mut(i) else { continue };
            if p.repeat {
                let len = p.max - p.min;
                if len > 0.0 {
                    let normalized = (*v - p.min) / len;
                    *v = (normalized - normalized.floor()) * len + p.min;
                }
            } else {
                *v = v.clamp(p.min, p.max);
            }
        }
    }

    fn update_parts(&mut self) {
        let mut corners = core::mem::take(&mut self.corners);
        // parents come first in part order in every model seen; a child whose
        // parent follows it keeps the parent's previous-frame state, which
        // settles in one frame and never reads out of bounds
        for (i, part) in self.model.parts.iter().enumerate() {
            let values = &self.values;
            let eps = &self.eps;
            let in_range = blend::corners(
                &part.binding,
                |p| values.get(p).copied().unwrap_or(0.0),
                |p| eps.get(p).copied().unwrap_or((0.001, 0.0015)),
                &mut corners,
            );
            let order =
                if in_range { Self::blend_scalar(&corners, &part.draw_orders) } else { 500.0 };
            if let Some(d) = self.part_draw_order.get_mut(i) {
                *d = order;
            }
            let mut enabled = part.visible && part.enabled;
            let own = self.part_user_opacity.get(i).copied().unwrap_or(1.0);
            // an offscreen part's opacity applies to its layer as a whole,
            // so overlapping children show no seams; its children only get
            // what it inherits
            let (mut opacity, layer) = match &part.offscreen {
                Some(o) => {
                    let keyed =
                        if in_range { Self::blend_scalar(&corners, &o.opacities) } else { 1.0 };
                    (1.0, own * keyed.clamp(0.0, 1.0))
                }
                None => (own, 1.0),
            };
            if let Some(l) = self.layer_opacity.get_mut(i) {
                *l = layer;
            }
            if let Some(parent) = part.parent_part {
                enabled =
                    enabled && self.part_enabled.get(parent as usize).copied().unwrap_or(true);
                opacity *= self.part_opacity.get(parent as usize).copied().unwrap_or(1.0);
            }
            if let Some(e) = self.part_enabled.get_mut(i) {
                *e = enabled;
            }
            if let Some(o) = self.part_opacity.get_mut(i) {
                *o = opacity;
            }
        }
        self.corners = corners;
    }

    /// Blend one scalar channel over the current corners.
    fn blend_scalar(corners: &[blend::Corner], pool: &[f32]) -> f32 {
        corners.iter().map(|&(k, w)| pool.get(k).copied().unwrap_or(0.0) * w).sum()
    }

    /// Blend a per-keyform position block of `len` floats into `out`.
    fn blend_positions(corners: &[blend::Corner], pool: &[f32], len: usize, out: &mut [f32]) {
        for o in out.iter_mut().take(len) {
            *o = 0.0;
        }
        for &(k, w) in corners {
            let base = k.saturating_mul(len);
            let Some(src) = pool.get(base..base.saturating_add(len)) else { continue };
            for (o, s) in out.iter_mut().zip(src) {
                *o += s * w;
            }
        }
    }

    /// Add a mesh's blend shapes onto `out`, its blend at the current
    /// values: each shape's weighted difference from the base form, the
    /// mesh's blend at every parameter's default, made in `base`. A mesh
    /// whose binding does not cover the defaults has no base form and so no
    /// blend shapes (Cubism requires the defaults keyed).
    fn add_blend_shapes(
        params: &[Parameter],
        m: &ArtMesh,
        values: &[f32],
        eps: &[(f32, f32)],
        corners: &mut Vec<blend::Corner>,
        base: &mut [f32],
        out: &mut [f32],
    ) {
        if m.blend_shapes.is_empty() {
            return;
        }
        let len = out.len();
        let Some(base) = base.get_mut(..len) else { return };
        let at_default = blend::corners(
            &m.binding,
            |p| params.get(p).map_or(0.0, |q| q.default),
            |p| eps.get(p).copied().unwrap_or((0.001, 0.0015)),
            corners,
        );
        if !at_default {
            return;
        }
        Self::blend_positions(corners, &m.positions, len, base);
        for shape in &m.blend_shapes {
            for (k, w) in blendshape::weights(shape_value(values, shape), &shape.keys) {
                if w == 0.0 {
                    continue;
                }
                let start = k.saturating_mul(len);
                let Some(form) = shape.positions.get(start..start.saturating_add(len)) else {
                    continue;
                };
                blendshape::add(out, form, base, w);
            }
        }
    }

    /// Map points through a deformer's CURRENT state (ancestors already
    /// resolved this frame, which index order guarantees).
    fn apply_deformer(&self, di: usize, io: &mut [f32]) {
        let Some(d) = self.model.deformers.get(di) else { return };
        let local = self.def_local.get(di).copied().unwrap_or(0) as usize;
        match &d.kind {
            DeformerKind::Warp(w) => {
                let off = self.warp_grid_off.get(local).copied().unwrap_or(0);
                let len = self.warp_grid_len.get(local).copied().unwrap_or(0);
                let grid = self.warp_grid.get(off..off.saturating_add(len)).unwrap_or(&[]);
                warp_transform(grid, w.rows as usize, w.cols as usize, w.quad, io);
            }
            DeformerKind::Rotation(_) => {
                let r = self.rot.get(local).copied().unwrap_or_default();
                rotation_transform(&r, io);
            }
        }
    }

    fn transform_point(&self, di: usize, p: Vec2) -> Vec2 {
        let mut io = [p.x, p.y];
        self.apply_deformer(di, &mut io);
        v2(io[0], io[1])
    }

    fn update_deformers(&mut self) {
        let mut corners = core::mem::take(&mut self.corners);
        let mut scratch = core::mem::take(&mut self.scratch);

        for di in 0..self.model.deformers.len() {
            let Some(d) = self.model.deformers.get(di) else { break };
            let mut enabled =
                d.visible && d.enabled && self.def_user_visible.get(di).copied().unwrap_or(true);
            if let Some(pp) = d.parent_part {
                enabled = enabled && self.part_enabled.get(pp as usize).copied().unwrap_or(true);
            }
            if let Some(pd) = d.parent_deformer {
                enabled = enabled && self.def_enabled.get(pd as usize).copied().unwrap_or(true);
            }
            if enabled {
                let values = &self.values;
                let eps = &self.eps;
                enabled = blend::corners(
                    &d.binding,
                    |p| values.get(p).copied().unwrap_or(0.0),
                    |p| eps.get(p).copied().unwrap_or((0.001, 0.0015)),
                    &mut corners,
                );
            }
            if let Some(e) = self.def_enabled.get_mut(di) {
                *e = enabled;
            }
            if !enabled {
                if let Some(o) = self.def_opacity.get_mut(di) {
                    *o = 0.0;
                }
                if let Some(m) = self.cache.def_made.get_mut(di) {
                    *m = false;
                }
                if let Some(m) = self.cache.def_moved.get_mut(di) {
                    *m = true;
                }
                continue;
            }

            let local = self.def_local.get(di).copied().unwrap_or(0) as usize;
            let parent = d.parent_deformer.map(|p| p as usize);
            let parent_opacity =
                parent.map_or(1.0, |p| self.def_opacity.get(p).copied().unwrap_or(1.0));
            // the same weights over the same parent: last frame's geometry stands
            let parent_moved =
                parent.is_some_and(|p| self.cache.def_moved.get(p).copied().unwrap_or(true));
            let same = self.cache.valid
                && !parent_moved
                && self.cache.def_made.get(di).copied().unwrap_or(false)
                && self.cache.def_corners.get(di).is_some_and(|c| *c == corners);
            if let Some(m) = self.cache.def_moved.get_mut(di) {
                *m = !same;
            }
            if !same {
                if let Some(c) = self.cache.def_corners.get_mut(di) {
                    c.clone_from(&corners);
                }
                if let Some(m) = self.cache.def_made.get_mut(di) {
                    *m = true;
                }
            }

            match &d.kind {
                DeformerKind::Warp(w) => {
                    if !same {
                        let len = self.warp_grid_len.get(local).copied().unwrap_or(0);
                        Self::blend_positions(&corners, &w.positions, len, &mut scratch);
                        if let Some(p) = parent
                            && let Some(io) = scratch.get_mut(..len)
                        {
                            self.apply_deformer(p, io);
                        }
                        let off = self.warp_grid_off.get(local).copied().unwrap_or(0);
                        if let (Some(dst), Some(src)) = (
                            self.warp_grid.get_mut(off..off.saturating_add(len)),
                            scratch.get(..len),
                        ) {
                            dst.copy_from_slice(src);
                        }
                    }
                    let opacity = Self::blend_scalar(&corners, &w.opacities);
                    if let Some(o) = self.def_opacity.get_mut(di) {
                        *o = opacity * parent_opacity;
                    }
                    let scale =
                        parent.map_or(1.0, |p| self.def_scale.get(p).copied().unwrap_or(1.0));
                    if let Some(s) = self.def_scale.get_mut(di) {
                        *s = scale;
                    }
                }
                DeformerKind::Rotation(r) if same => {
                    // the resolved rotation stands; opacity can still fade
                    let opacity = Self::blend_scalar(&corners, &r.opacities);
                    let scale = self.rot.get(local).map_or(1.0, |r| r.scale);
                    if let Some(s) = self.def_scale.get_mut(di) {
                        *s = scale;
                    }
                    if let Some(o) = self.def_opacity.get_mut(di) {
                        *o = opacity * parent_opacity;
                    }
                }
                DeformerKind::Rotation(r) => {
                    let mut rot = Rotation {
                        origin: v2(
                            Self::blend_scalar(&corners, &r.origins_x),
                            Self::blend_scalar(&corners, &r.origins_y),
                        ),
                        angle: r.base_angle + Self::blend_scalar(&corners, &r.angles),
                        scale: Self::blend_scalar(&corners, &r.scales),
                        reflect_x: corners
                            .first()
                            .and_then(|&(k, _)| r.reflect_x.get(k))
                            .copied()
                            .unwrap_or(false),
                        reflect_y: corners
                            .first()
                            .and_then(|&(k, _)| r.reflect_y.get(k))
                            .copied()
                            .unwrap_or(false),
                    };
                    let opacity = Self::blend_scalar(&corners, &r.opacities);
                    if let Some(p) = parent {
                        // The parent may bend space, so the child's angle has
                        // no closed form: probe how a short vertical step maps
                        // and measure the turn (the verified runtime's method).
                        let parent_is_rotation = matches!(
                            self.model.deformers.get(p).map(|pd| &pd.kind),
                            Some(DeformerKind::Rotation(_))
                        );
                        let dir_delta = if parent_is_rotation { -10.0 } else { -0.1 };
                        let t_origin = self.transform_point(p, rot.origin);
                        let mut direction = v2(0.0, 0.0);
                        let mut probe = 1.0f32;
                        for _ in 0..16 {
                            let tt = self.transform_point(
                                p,
                                v2(rot.origin.x, rot.origin.y + probe * dir_delta),
                            );
                            let dxy = tt.sub2(t_origin);
                            if dxy.x != 0.0 || dxy.y != 0.0 {
                                direction = dxy;
                                break;
                            }
                            let tt = self.transform_point(
                                p,
                                v2(rot.origin.x, rot.origin.y - probe * dir_delta),
                            );
                            let dxy = tt.sub2(t_origin);
                            if dxy.x != 0.0 || dxy.y != 0.0 {
                                direction = v2(-dxy.x, -dxy.y);
                                break;
                            }
                            probe *= 0.1;
                        }
                        let angle_adj = signed_angle(v2(0.0, dir_delta), direction) * -180.0
                            / core::f32::consts::PI;
                        rot.angle += angle_adj;
                        rot.origin = t_origin;
                        rot.scale *= self.def_scale.get(p).copied().unwrap_or(1.0);
                    }
                    if let Some(s) = self.def_scale.get_mut(di) {
                        *s = rot.scale;
                    }
                    if let Some(o) = self.def_opacity.get_mut(di) {
                        *o = opacity * parent_opacity;
                    }
                    if let Some(slot) = self.rot.get_mut(local) {
                        *slot = rot;
                    }
                }
            }
        }

        self.corners = corners;
        self.scratch = scratch;
    }

    fn update_meshes(&mut self) {
        let mut corners = core::mem::take(&mut self.corners);
        let mut mesh_pos = core::mem::take(&mut self.mesh_pos);
        let mut scratch = core::mem::take(&mut self.scratch);
        let mut shape_corners = core::mem::take(&mut self.shape_corners);

        for mi in 0..self.model.art_meshes.len() {
            let Some(m) = self.model.art_meshes.get(mi) else { break };
            let mut enabled =
                m.visible && m.enabled && self.mesh_user_visible.get(mi).copied().unwrap_or(true);
            if let Some(pp) = m.parent_part {
                enabled = enabled && self.part_enabled.get(pp as usize).copied().unwrap_or(true);
            }
            if let Some(pd) = m.parent_deformer {
                enabled = enabled && self.def_enabled.get(pd as usize).copied().unwrap_or(true);
            }
            if enabled {
                let values = &self.values;
                let eps = &self.eps;
                enabled = blend::corners(
                    &m.binding,
                    |p| values.get(p).copied().unwrap_or(0.0),
                    |p| eps.get(p).copied().unwrap_or((0.001, 0.0015)),
                    &mut corners,
                );
            }
            if let Some(v) = self.mesh_visible.get_mut(mi) {
                *v = enabled;
            }
            // the same weights over a parent that did not move: last frame's
            // points stand (never for glued meshes, which glue moves after)
            let parent_moved = m
                .parent_deformer
                .is_some_and(|p| self.cache.def_moved.get(p as usize).copied().unwrap_or(true));
            let same = self.cache.valid
                && !parent_moved
                && !self.cache.mesh_glued.get(mi).copied().unwrap_or(true)
                && self.cache.mesh_made.get(mi).copied().unwrap_or(false)
                && self.cache.mesh_corners.get(mi).is_some_and(|c| *c == corners)
                && self.cache.mesh_shapes_same(mi, &m.blend_shapes, &self.values);
            if !enabled {
                if let Some(o) = self.mesh_opacity.get_mut(mi) {
                    *o = 0.0;
                }
                // a hidden mesh that clips others still needs geometry; its
                // own binding decides whether there is anything to blend
                let needed =
                    self.mask_source.get(mi).copied().unwrap_or(false) && !corners.is_empty();
                if !needed {
                    if let Some(made) = self.cache.mesh_made.get_mut(mi) {
                        *made = false;
                    }
                    continue;
                }
                if !same {
                    let len = (m.vertex_count as usize).saturating_mul(2);
                    let off = self.mesh_pos_off.get(mi).copied().unwrap_or(0);
                    if let Some(out) = mesh_pos.get_mut(off..off.saturating_add(len)) {
                        Self::blend_positions(&corners, &m.positions, len, out);
                        Self::add_blend_shapes(
                            &self.model.parameters,
                            m,
                            &self.values,
                            &self.eps,
                            &mut shape_corners,
                            &mut scratch,
                            out,
                        );
                        if let Some(pd) = m.parent_deformer {
                            self.apply_deformer(pd as usize, out);
                        }
                    }
                    self.cache.remember_mesh(mi, &corners, &m.blend_shapes, &self.values);
                }
                continue;
            }

            if !same {
                let len = (m.vertex_count as usize).saturating_mul(2);
                let off = self.mesh_pos_off.get(mi).copied().unwrap_or(0);
                if let Some(out) = mesh_pos.get_mut(off..off.saturating_add(len)) {
                    Self::blend_positions(&corners, &m.positions, len, out);
                    Self::add_blend_shapes(
                        &self.model.parameters,
                        m,
                        &self.values,
                        &self.eps,
                        &mut shape_corners,
                        &mut scratch,
                        out,
                    );
                    if let Some(pd) = m.parent_deformer {
                        self.apply_deformer(pd as usize, out);
                    }
                }
                self.cache.remember_mesh(mi, &corners, &m.blend_shapes, &self.values);
            }

            let mut opacity = Self::blend_scalar(&corners, &m.opacities);
            if let Some(pd) = m.parent_deformer {
                opacity *= self.def_opacity.get(pd as usize).copied().unwrap_or(1.0);
            }
            if let Some(pp) = m.parent_part {
                opacity *= self.part_opacity.get(pp as usize).copied().unwrap_or(1.0);
            }
            if let Some(o) = self.mesh_opacity.get_mut(mi) {
                *o = opacity.clamp(0.0, 1.0);
            }
            let draw_order = Self::blend_scalar(&corners, &m.draw_orders);
            if let Some(d) = self.mesh_draw_order.get_mut(mi) {
                *d = draw_order;
            }
        }

        self.corners = corners;
        self.mesh_pos = mesh_pos;
        self.scratch = scratch;
        self.shape_corners = shape_corners;
    }

    /// Stitch glued mesh pairs: each paired vertex pulls toward its partner
    /// by the glue's blended intensity times its own weight. Runs after every
    /// mesh is in world space, matching the reference pipeline.
    fn apply_glues(&mut self) {
        let mut corners = core::mem::take(&mut self.corners);
        for g in &self.model.glues {
            let values = &self.values;
            let eps = &self.eps;
            let in_range = blend::corners(
                &g.binding,
                |p| values.get(p).copied().unwrap_or(0.0),
                |p| eps.get(p).copied().unwrap_or((0.001, 0.0015)),
                &mut corners,
            );
            if !in_range {
                continue;
            }
            let intensity = Self::blend_scalar(&corners, &g.intensities);
            if intensity == 0.0 {
                continue;
            }
            let off_a = self.mesh_pos_off.get(g.art_mesh_a as usize).copied().unwrap_or(0);
            let off_b = self.mesh_pos_off.get(g.art_mesh_b as usize).copied().unwrap_or(0);
            let (idx_pairs, _) = g.indices.as_chunks::<2>();
            let (w_pairs, _) = g.weights.as_chunks::<2>();
            for (&[i0, i1], &[w0, w1]) in idx_pairs.iter().zip(w_pairs) {
                let ia = off_a + (i0 as usize) * 2;
                let ib = off_b + (i1 as usize) * 2;
                let (Some(&ax), Some(&ay)) = (self.mesh_pos.get(ia), self.mesh_pos.get(ia + 1))
                else {
                    continue;
                };
                let (Some(&bx), Some(&by)) = (self.mesh_pos.get(ib), self.mesh_pos.get(ib + 1))
                else {
                    continue;
                };
                let (dx, dy) = (bx - ax, by - ay);
                if let Some(v) = self.mesh_pos.get_mut(ia) {
                    *v = ax + dx * intensity * w0;
                }
                if let Some(v) = self.mesh_pos.get_mut(ia + 1) {
                    *v = ay + dy * intensity * w0;
                }
                if let Some(v) = self.mesh_pos.get_mut(ib) {
                    *v = bx - dx * intensity * w1;
                }
                if let Some(v) = self.mesh_pos.get_mut(ib + 1) {
                    *v = by - dy * intensity * w1;
                }
            }
        }
        self.corners = corners;
    }

    fn sort_render_order(&mut self) {
        self.sort_by_draw_order();
        self.group_layers();
    }

    /// In a model without draw order groups of its own, an offscreen part
    /// sorts as one item at its own draw order among its siblings, its
    /// contents together inside it, as a draw order group does (and as the
    /// groups a moc3 export writes for it, `draworder::synthesize`). Ties
    /// keep mesh index order, a part by its lowest mesh.
    fn group_layers(&mut self) {
        if self.layer_depth == 0 || !self.model.draw_groups.is_empty() {
            return;
        }
        let (layers, tie) = (&self.mesh_layers, &self.part_tie);
        let (parts, meshes) = (&self.part_draw_order, &self.mesh_draw_order);
        // a mesh's item at each nesting level: the offscreen part it sits in
        // there, or past its own chain the mesh itself
        let key = |mi: u32, level: usize| match layers.get(mi as usize).and_then(|c| c.get(level)) {
            Some(&p) => (
                parts.get(p as usize).copied().unwrap_or(500.0).round() as i64,
                tie.get(p as usize).copied().unwrap_or(u32::MAX),
            ),
            None => (meshes.get(mi as usize).copied().unwrap_or(500.0).round() as i64, mi),
        };
        let depth = self.layer_depth;
        self.render_order.sort_unstable_by(|&a, &b| {
            (0..=depth)
                .map(|level| key(a, level).cmp(&key(b, level)))
                .find(|o| o.is_ne())
                .unwrap_or(core::cmp::Ordering::Equal)
        });
    }

    fn sort_by_draw_order(&mut self) {
        if self.model.draw_groups.is_empty() {
            // draw orders are whole numbers, as in the grouped walk below:
            // a blend that lands on 500.00003 must still tie with 500
            let orders = &self.mesh_draw_order;
            self.render_order.sort_unstable_by(|&a, &b| {
                let oa = orders.get(a as usize).copied().unwrap_or(500.0).round();
                let ob = orders.get(b as usize).copied().unwrap_or(500.0).round();
                oa.total_cmp(&ob).then(a.cmp(&b))
            });
            return;
        }
        // the group walk the reference runtime does: bucket sort each group
        // by relative draw order, meshes take slots, a part item reserves a
        // run of slots for its child group
        for c in self.dog_cursor.iter_mut() {
            *c = 0;
        }
        self.slot_buf.fill(u32::MAX);
        for gi in 0..self.model.draw_groups.len() {
            let Some(group) = self.model.draw_groups.get(gi) else { break };
            let n = group.items.len();
            let level =
                (group.max_order.saturating_sub(group.min_order).max(0) as usize).saturating_add(1);
            for f in self.dog_first.iter_mut().take(level) {
                *f = -1;
            }
            for l in self.dog_last.iter_mut().take(level) {
                *l = -1;
            }
            for x in self.dog_next.iter_mut().take(n) {
                *x = -1;
            }
            for (j, item) in group.items.iter().enumerate() {
                let (order, on) = if item.part {
                    (
                        self.part_draw_order.get(item.object as usize).copied().unwrap_or(500.0),
                        self.part_enabled.get(item.object as usize).copied().unwrap_or(true),
                    )
                } else {
                    (
                        self.mesh_draw_order.get(item.object as usize).copied().unwrap_or(500.0),
                        self.mesh_visible.get(item.object as usize).copied().unwrap_or(false),
                    )
                };
                let order = if on { order } else { group.min_order as f32 };
                let rel = ((order as i32).saturating_sub(group.min_order))
                    .clamp(0, level as i32 - 1) as usize;
                if let Some(d) = self.dog_order.get_mut(j) {
                    *d = order;
                }
                let j = j as i32;
                match self.dog_last.get(rel).copied() {
                    Some(-1) | None => {
                        if let Some(f) = self.dog_first.get_mut(rel) {
                            *f = j;
                        }
                    }
                    Some(prev) => {
                        if let Some(nx) = self.dog_next.get_mut(prev as usize) {
                            *nx = j;
                        }
                    }
                }
                if let Some(l) = self.dog_last.get_mut(rel) {
                    *l = j;
                }
            }
            let mut pos = self.dog_cursor.get(gi).copied().unwrap_or(0);
            for b in 0..level {
                let mut di = self.dog_first.get(b).copied().unwrap_or(-1);
                while di >= 0 && (di as usize) < n {
                    let Some(item) = group.items.get(di as usize) else { break };
                    if item.part {
                        // an offscreen part's layer takes a slot of its own
                        if self
                            .model
                            .parts
                            .get(item.object as usize)
                            .is_some_and(|p| p.offscreen.is_some())
                        {
                            pos = pos.saturating_add(1);
                        }
                        if let Some(cg) = item.child_group
                            && let Some(&child_total) = self.group_total.get(cg as usize)
                        {
                            if let Some(c) = self.dog_cursor.get_mut(cg as usize) {
                                *c = pos;
                            }
                            pos = pos.saturating_add(child_total);
                        }
                    } else {
                        if let Some(slot) = self.slot_buf.get_mut(pos as usize) {
                            *slot = item.object;
                        }
                        pos = pos.saturating_add(1);
                    }
                    let ni = self.dog_next.get(di as usize).copied().unwrap_or(-1);
                    if ni <= di {
                        break;
                    }
                    di = ni;
                }
            }
        }
        // the slots in order are the render order; layer slots hold no mesh.
        // A mesh no group placed (a malformed file) draws last
        self.mesh_seen.fill(false);
        let mut n = 0;
        for &mi in self.slot_buf.iter().filter(|&&mi| mi != u32::MAX) {
            if let Some(seen) = self.mesh_seen.get_mut(mi as usize)
                && !*seen
            {
                *seen = true;
                if let Some(slot) = self.render_order.get_mut(n) {
                    *slot = mi;
                }
                n += 1;
            }
        }
        for mi in 0..self.mesh_seen.len() {
            if !self.mesh_seen.get(mi).copied().unwrap_or(true)
                && let Some(slot) = self.render_order.get_mut(n)
            {
                *slot = mi as u32;
                n += 1;
            }
        }
    }

    /// Map a point through one deformer's current state; editors use this
    /// to probe local bases when converting screen drags to parent space.
    pub fn deformer_point(&self, deformer: usize, x: f32, y: f32) -> (f32, f32) {
        let p = self.transform_point(deformer, v2(x, y));
        (p.x, p.y)
    }

    /// A mesh's points as drawn now (model units, canvas frame), hidden or
    /// not: what the clipboard keeps of a mesh pasted where its parent
    /// deformer does not exist.
    pub fn mesh_drawn_positions(&self, mesh: usize) -> Option<Vec<f32>> {
        let m = self.model.art_meshes.get(mesh)?;
        let len = (m.vertex_count as usize).saturating_mul(2);
        let off = self.mesh_pos_off.get(mesh).copied()?;
        self.mesh_pos.get(off..off.saturating_add(len)).map(<[f32]>::to_vec)
    }

    /// A mesh's blended keyform positions before its parent deformer runs,
    /// in parent space. Mesh editing probes these through `deformer_point`
    /// to turn a screen drag into a parent-space offset.
    pub fn mesh_local_positions(&self, mesh: usize) -> Option<Vec<f32>> {
        let m = self.model.art_meshes.get(mesh)?;
        let mut corners = Vec::new();
        let ok = blend::corners(
            &m.binding,
            |p| self.values.get(p).copied().unwrap_or(0.0),
            |p| self.eps.get(p).copied().unwrap_or((0.001, 0.0015)),
            &mut corners,
        );
        if !ok {
            return None;
        }
        let len = (m.vertex_count as usize).saturating_mul(2);
        let mut out = vec![0.0; len];
        Self::blend_positions(&corners, &m.positions, len, &mut out);
        let mut base = vec![0.0; len];
        Self::add_blend_shapes(
            &self.model.parameters,
            m,
            &self.values,
            &self.eps,
            &mut corners,
            &mut base,
            &mut out,
        );
        Some(out)
    }

    /// A warp deformer's blended control grid before its parent chain runs,
    /// in parent space; grid editing inverts the parent map through this.
    pub fn warp_local_grid(&self, deformer: usize) -> Option<Vec<f32>> {
        let d = self.model.deformers.get(deformer)?;
        let DeformerKind::Warp(w) = &d.kind else { return None };
        let mut corners = Vec::new();
        let ok = blend::corners(
            &d.binding,
            |p| self.values.get(p).copied().unwrap_or(0.0),
            |p| self.eps.get(p).copied().unwrap_or((0.001, 0.0015)),
            &mut corners,
        );
        if !ok {
            return None;
        }
        let len = w.grid_points() * 2;
        let mut out = vec![0.0; len];
        Self::blend_positions(&corners, &w.positions, len, &mut out);
        Some(out)
    }

    /// A warp deformer's evaluated control grid, for editor overlays.
    pub fn warp_grid(&self, deformer: usize) -> Option<(&[f32], u32, u32)> {
        let d = self.model.deformers.get(deformer)?;
        let DeformerKind::Warp(w) = &d.kind else { return None };
        let local = self.def_local.get(deformer).copied()? as usize;
        let off = self.warp_grid_off.get(local).copied()?;
        let len = self.warp_grid_len.get(local).copied()?;
        Some((self.warp_grid.get(off..off.saturating_add(len))?, w.rows, w.cols))
    }

    /// A rotation deformer's blended origin before the parent chain runs,
    /// in parent space, for origin dragging.
    pub fn rotation_local_origin(&self, deformer: usize) -> Option<(f32, f32)> {
        let d = self.model.deformers.get(deformer)?;
        let DeformerKind::Rotation(r) = &d.kind else { return None };
        let mut corners = Vec::new();
        let ok = blend::corners(
            &d.binding,
            |p| self.values.get(p).copied().unwrap_or(0.0),
            |p| self.eps.get(p).copied().unwrap_or((0.001, 0.0015)),
            &mut corners,
        );
        if !ok {
            return None;
        }
        Some((
            Self::blend_scalar(&corners, &r.origins_x),
            Self::blend_scalar(&corners, &r.origins_y),
        ))
    }

    /// A rotation deformer's resolved state, for editor overlays.
    pub fn rotation_state(&self, deformer: usize) -> Option<deform::Rotation> {
        let d = self.model.deformers.get(deformer)?;
        if !matches!(d.kind, DeformerKind::Rotation(_)) {
            return None;
        }
        let local = self.def_local.get(deformer).copied()? as usize;
        self.rot.get(local).copied()
    }

    /// A mesh's evaluated geometry whether or not it is drawn.
    pub fn mesh_geometry(&self, mesh: usize) -> Option<MeshGeometry<'_>> {
        let m = self.model.art_meshes.get(mesh)?;
        let len = (m.vertex_count as usize).saturating_mul(2);
        let off = self.mesh_pos_off.get(mesh).copied().unwrap_or(0);
        Some(MeshGeometry {
            positions: self.mesh_pos.get(off..off.saturating_add(len)).unwrap_or(&[]),
            uvs: &m.uvs,
            indices: &m.indices,
            texture: m.texture,
        })
    }

    /// The meshes to draw, back to front. Positions are model units in the
    /// canvas frame: screen position is origin + xy * pixels_per_unit, no
    /// axis flip. UVs address the texture top-down, also unflipped.
    /// The offscreen parts a mesh draws inside, outermost first: a renderer
    /// opens a layer for each on the way in and lands it on the way out.
    pub fn layers_of(&self, mesh: usize) -> &[u32] {
        self.mesh_layers.get(mesh).map(Vec::as_slice).unwrap_or_default()
    }

    /// How an offscreen part's layer lands this frame; None for a part that
    /// is not drawn offscreen.
    pub fn layer(&self, part: usize) -> Option<Layer<'_>> {
        let o = self.model.parts.get(part)?.offscreen.as_ref()?;
        Some(Layer {
            part,
            blend: o.blend,
            opacity: self.layer_opacity.get(part).copied().unwrap_or(1.0),
            masks: &o.masks,
            inverted: o.inverted,
        })
    }

    pub fn drawables(&self) -> impl Iterator<Item = Drawable<'_>> {
        self.render_order.iter().filter_map(move |&mi| {
            let mi = mi as usize;
            let m = self.model.art_meshes.get(mi)?;
            if !self.mesh_visible.get(mi).copied().unwrap_or(false) {
                return None;
            }
            let len = (m.vertex_count as usize).saturating_mul(2);
            let off = self.mesh_pos_off.get(mi).copied().unwrap_or(0);
            Some(Drawable {
                mesh: mi,
                texture: m.texture,
                flags: m.flags,
                blend: Blend::of(m.blend, m.flags),
                opacity: self.mesh_opacity.get(mi).copied().unwrap_or(0.0),
                positions: self.mesh_pos.get(off..off.saturating_add(len)).unwrap_or(&[]),
                uvs: &m.uvs,
                indices: &m.indices,
                masks: &m.masks,
                clip_and: self.mesh_clip_and.get(mi).map(Vec::as_slice).unwrap_or_default(),
            })
        })
    }
}

impl Vec2 {
    fn sub2(self, o: Vec2) -> Vec2 {
        v2(self.x - o.x, self.y - o.y)
    }
}

/// One mesh, evaluated and ready to draw.
/// An offscreen part's layer as it lands on what is beneath it.
#[derive(Clone, Copy, Debug)]
pub struct Layer<'a> {
    pub part: usize,
    pub blend: Blend,
    pub opacity: f32,
    /// Meshes whose texture alpha clips the layer.
    pub masks: &'a [u32],
    /// Shown outside the masks instead of inside.
    pub inverted: bool,
}

pub struct Drawable<'a> {
    pub mesh: usize,
    pub texture: u32,
    pub flags: u8,
    /// Colour and alpha blend, from the mesh or its flag bits.
    pub blend: Blend,
    pub opacity: f32,
    /// xy pairs, `vertex_count * 2`.
    pub positions: &'a [f32],
    pub uvs: &'a [f32],
    pub indices: &'a [u16],
    /// Meshes whose texture alpha clips this one.
    pub masks: &'a [u32],
    /// Inherited clips: further mask sets it must also be inside, one per
    /// level (a highlight inside the iris inside the eye white).
    pub clip_and: &'a [Vec<u32>],
}

/// A mesh's current geometry, visible or not; mask rendering needs this.
pub struct MeshGeometry<'a> {
    pub positions: &'a [f32],
    pub uvs: &'a [f32],
    pub indices: &'a [u16],
    pub texture: u32,
}

#[cfg(test)]
mod tests {
    use super::*;
    use ayatsuri_format::model::{Binding, BindingAxis, Canvas};

    fn one_param_model() -> Model {
        // one parameter 0..1, one mesh with two keyforms: a unit-right
        // translation between them
        Model {
            canvas: Canvas { pixels_per_unit: 1.0, ..Canvas::default() },
            parameters: vec![Parameter {
                id: "ParamX".into(),
                min: 0.0,
                max: 1.0,
                default: 0.0,
                repeat: false,
                decimals: 3,
                blend_shape: false,
            }],
            parts: vec![],
            deformers: vec![],
            art_meshes: vec![ArtMesh {
                id: "D_MESH".into(),
                parent_part: None,
                parent_deformer: None,
                visible: true,
                enabled: true,
                texture: 0,
                flags: 0,
                blend: Default::default(),
                vertex_count: 3,
                uvs: vec![0.0; 6],
                indices: vec![0, 1, 2],
                masks: vec![],
                binding: Binding {
                    curves: Vec::new(),
                    axes: vec![BindingAxis { parameter: 0, keys: vec![0.0, 1.0] }],
                },
                opacities: vec![1.0, 0.5],
                draw_orders: vec![500.0, 500.0],
                positions: vec![
                    0.0, 0.0, 1.0, 0.0, 0.0, 1.0, // keyform 0
                    1.0, 0.0, 2.0, 0.0, 1.0, 1.0, // keyform 1
                ],
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

    #[test]
    fn keyforms_interpolate_across_a_parameter() {
        let mut p = Puppet::new(one_param_model());
        p.update();
        let d: Vec<_> = p.drawables().collect();
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].positions[0], 0.0);
        assert_eq!(d[0].opacity, 1.0);

        p.set_value(0, 0.5);
        p.update();
        let d: Vec<_> = p.drawables().collect();
        assert!((d[0].positions[0] - 0.5).abs() < 1e-6);
        assert!((d[0].opacity - 0.75).abs() < 1e-6);

        p.set_value(0, 1.0);
        p.update();
        let d: Vec<_> = p.drawables().collect();
        assert!((d[0].positions[0] - 1.0).abs() < 1e-6);
    }

    #[test]
    fn values_clamp_to_range() {
        let mut p = Puppet::new(one_param_model());
        p.set_value(0, 7.0);
        p.update();
        assert_eq!(p.value(0), 1.0);
    }

    fn smile_model() -> Model {
        // one_param_model plus a blend shape parameter whose key 1.0 lifts
        // vertex 0 by 3 over the base form (keyform 0, ParamX's default)
        let mut model = one_param_model();
        model.parameters.push(Parameter {
            id: "ParamSmile".into(),
            min: 0.0,
            max: 1.0,
            default: 0.0,
            repeat: false,
            decimals: 3,
            blend_shape: true,
        });
        if let Some(m) = model.art_meshes.first_mut() {
            m.blend_shapes.push(BlendShape {
                parameter: 1,
                keys: vec![1.0],
                positions: vec![0.0, 3.0, 1.0, 0.0, 0.0, 1.0],
            });
        }
        model
    }

    #[test]
    fn blend_shapes_add_on_top_of_keyforms() {
        let mut p = Puppet::new(smile_model());
        p.set_value(0, 0.5);
        p.update();
        let d: Vec<_> = p.drawables().collect();
        assert_eq!(&d[0].positions[..2], &[0.5, 0.0]);

        // only the blend shape moves: the cache must not keep last frame's
        p.set_value(1, 0.5);
        p.update();
        let d: Vec<_> = p.drawables().collect();
        assert!((d[0].positions[0] - 0.5).abs() < 1e-6);
        assert!((d[0].positions[1] - 1.5).abs() < 1e-6);
        // the other vertices match the base form, so nothing lands on them
        assert!((d[0].positions[2] - 1.5).abs() < 1e-6);
        assert_eq!(d[0].positions[3], 0.0);

        // the editor's local probe sees the same points
        let local = p.mesh_local_positions(0).unwrap_or_default();
        assert!((local[1] - 1.5).abs() < 1e-6);
    }

    #[test]
    fn an_offscreen_part_keeps_its_meshes_together_and_owns_its_opacity() {
        use ayatsuri_format::model::{Offscreen, Part};
        // three meshes drawn a, x, b; a and b sit in an offscreen part
        let mut model = one_param_model();
        let mesh = model.art_meshes[0].clone();
        model.art_meshes.clear();
        for (id, order, part) in [("a", 100.0, Some(0)), ("x", 200.0, None), ("b", 300.0, Some(0))]
        {
            let mut m = mesh.clone();
            m.id = id.into();
            m.parent_part = part;
            m.draw_orders = vec![order, order];
            model.art_meshes.push(m);
        }
        model.parts.push(Part {
            id: "P".into(),
            parent_part: None,
            visible: true,
            enabled: true,
            binding: Binding::default(),
            draw_orders: vec![500.0],
            offscreen: Some(Offscreen {
                masks: vec![1],
                inverted: true,
                blend: Default::default(),
                opacities: vec![0.5],
            }),
        });
        // the part sorts as one item at its own order (500): after x
        let mut p = Puppet::new(model.clone());
        p.update();
        let order: Vec<usize> = p.drawables().map(|d| d.mesh).collect();
        assert_eq!(order, vec![1, 0, 2]);
        // switched on through the editor, it takes its lowest mesh's order
        // (100) and stays where a was: a, b, then x
        model.parts[0].offscreen = None;
        assert!(edit::set_offscreen(&mut model, 0, true));
        if let Some(o) = &mut model.parts[0].offscreen {
            (o.masks, o.inverted, o.opacities) = (vec![1], true, vec![0.5]);
        }
        let mut p = Puppet::new(model);
        p.set_part_opacity(0, 0.8);
        p.update();
        let order: Vec<usize> = p.drawables().map(|d| d.mesh).collect();
        assert_eq!(order, vec![0, 2, 1]);
        assert_eq!(p.layers_of(0), &[0]);
        assert!(p.layers_of(1).is_empty());
        let layer = p.layer(0).unwrap_or_else(|| unreachable!());
        assert!((layer.opacity - 0.4).abs() < 1e-6);
        assert_eq!((layer.masks, layer.inverted), (&[1u32][..], true));
        // the layer carries the part's opacity, its meshes don't
        assert!(p.drawables().all(|d| (d.opacity - 1.0).abs() < 1e-6));
    }

    #[test]
    fn the_flat_rule_matches_the_groups_an_export_writes() {
        use ayatsuri_format::model::{Offscreen, Part};
        // meshes a, x, b, c at 100, 200, 300, 250: a and b inside offscreen
        // P, c inside offscreen Q nested in P
        let mut model = one_param_model();
        let mesh = model.art_meshes[0].clone();
        model.art_meshes.clear();
        for (order, part) in [(100.0, Some(0)), (200.0, None), (300.0, Some(0)), (250.0, Some(1))] {
            let mut m = mesh.clone();
            m.parent_part = part;
            m.draw_orders = vec![order, order];
            model.art_meshes.push(m);
        }
        let part = |parent, order| Part {
            id: String::new(),
            parent_part: parent,
            visible: true,
            enabled: true,
            binding: Binding::default(),
            draw_orders: vec![order],
            offscreen: Some(Offscreen {
                masks: vec![],
                inverted: false,
                blend: Default::default(),
                opacities: vec![1.0],
            }),
        };
        let order_of = |model: &Model| {
            let mut p = Puppet::new(model.clone());
            p.update();
            p.drawables().map(|d| d.mesh).collect::<Vec<_>>()
        };
        for (p_order, q_order) in [(150.0, 400.0), (500.0, 100.0), (100.0, 300.0)] {
            model.parts = vec![part(None, p_order), part(Some(0), q_order)];
            model.draw_groups.clear();
            let flat = order_of(&model);
            model.draw_groups = draworder::synthesize(&model);
            assert_eq!(order_of(&model), flat, "P at {p_order}, Q at {q_order}");
        }
    }

    #[test]
    fn a_blend_shape_is_made_edited_and_survives_mesh_edits() {
        use edit::Target;
        let mut model = smile_model();
        if let Some(m) = model.art_meshes.first_mut() {
            m.blend_shapes.clear();
        }
        // Add 2 Keyforms: the base key is implied, key 1.0 starts as the base
        assert!(edit::add_keys(&mut model, Target::ArtMesh(0), 1, &[0.0, 1.0]));
        let m = &model.art_meshes[0];
        assert_eq!(m.blend_shapes[0].keys, vec![1.0]);
        assert_eq!(m.blend_shapes[0].positions, m.positions[..6].to_vec());
        assert_eq!(keyform::keys_on(&model, Target::ArtMesh(0), 1), Some(vec![0.0, 1.0]));

        // on key 1.0 the blend keyform is the one edited, wherever ParamX is
        let k = keyform::keyform_at(&model, Target::ArtMesh(0), &[0.3, 1.0]);
        assert_eq!(k, Some(2));
        assert!(edit::move_vertices_in(&mut model, 0, k, &[(0, 0.0, 3.0)]));
        assert_eq!(model.art_meshes[0].blend_shapes[0].positions[1], 3.0);
        // at the base key the normal keyforms are, between keys nothing is
        assert_eq!(keyform::keyform_at(&model, Target::ArtMesh(0), &[1.0, 0.0]), Some(1));
        assert_eq!(keyform::keyform_at(&model, Target::ArtMesh(0), &[1.0, 0.5]), None);

        let mut p = Puppet::new(model.clone());
        p.set_value(1, 0.5);
        p.update();
        assert!((p.drawables().next().map_or(0.0, |d| d.positions[1]) - 1.5).abs() < 1e-6);

        // a new vertex lands in the blend form too, the forms stay aligned
        assert!(edit::add_vertex(&mut model, 0, 0, [1.0 / 3.0; 3]).is_some());
        let m = &model.art_meshes[0];
        assert_eq!(m.positions.len(), 2 * 8);
        assert_eq!(m.blend_shapes[0].positions.len(), 8);
        assert!((m.blend_shapes[0].positions[1] - 3.0).abs() < 1e-6);
        assert!((m.blend_shapes[0].positions[7] - 4.0 / 3.0).abs() < 1e-6);

        // the parameter is in use until the blend shape goes
        assert!(!edit::delete_parameter(&mut model, 1));
        assert!(edit::delete_key(&mut model, Target::ArtMesh(0), 1, 1.0));
        assert!(model.art_meshes[0].blend_shapes.is_empty());
        assert!(edit::delete_parameter(&mut model, 1));
    }

    #[test]
    fn skipped_work_matches_a_full_pass() {
        let positions = |p: &Puppet| -> Vec<u32> {
            p.drawables().flat_map(|d| d.positions.iter().map(|x| x.to_bits())).collect()
        };
        let mut cached = Puppet::new(one_param_model());
        let mut full = Puppet::new(one_param_model());
        let check = |cached: &mut Puppet, full: &mut Puppet| {
            cached.update();
            full.invalidate_cache();
            full.update();
            assert_eq!(positions(cached), positions(full));
        };
        for v in [0.25, 0.25, 0.25, 0.75, 0.75] {
            cached.set_value(0, v);
            full.set_value(0, v);
            check(&mut cached, &mut full);
        }
        // an in-place keyform edit shows on the next update, values held
        for p in [&mut cached, &mut full] {
            if let Some(k) = p.mesh_keyform_mut(0, 1) {
                k[0] = 3.0;
            }
        }
        check(&mut cached, &mut full);
        // hidden then shown again: made afresh
        for on in [false, true] {
            cached.set_mesh_visible(0, on);
            full.set_mesh_visible(0, on);
            check(&mut cached, &mut full);
        }
    }
}
