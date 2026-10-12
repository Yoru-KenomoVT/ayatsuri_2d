//! Deformation, physics, masking, animation and draw order.
//!
//! The rules this crate lives by (docs/goals.md):
//! - no I/O and no renderer: it consumes `ayatsuri-format` types and produces
//!   vertex positions and draw lists;
//! - allocation happens at load, the frame loop allocates nothing;
//! - never depends on `openmoc3` (docs/architecture.md, dependency rules).

#![forbid(unsafe_code)]

pub mod anim;
pub mod automesh;
pub mod blend;
pub mod deform;
pub mod edit;
pub mod physics;

use ayatsuri_format::model::{DeformerKind, Model};
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
    /// The pendulum rig's live state, when the model carries one.
    physics: Option<physics::Physics>,
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
        let mut mask_source = vec![false; n_meshes];
        for m in &model.art_meshes {
            for &src in &m.masks {
                if let Some(f) = mask_source.get_mut(src as usize) {
                    *f = true;
                }
            }
        }

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
            dog_first: vec![-1; max_level],
            dog_last: vec![-1; max_level],
            dog_next: vec![-1; max_items],
            dog_cursor: vec![0; model.draw_groups.len()],
            dog_order: vec![500.0; max_items],
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
            let mut opacity = self.part_user_opacity.get(i).copied().unwrap_or(1.0);
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
                continue;
            }

            let local = self.def_local.get(di).copied().unwrap_or(0) as usize;
            let parent = d.parent_deformer.map(|p| p as usize);
            let parent_opacity =
                parent.map_or(1.0, |p| self.def_opacity.get(p).copied().unwrap_or(1.0));

            match &d.kind {
                DeformerKind::Warp(w) => {
                    let len = self.warp_grid_len.get(local).copied().unwrap_or(0);
                    Self::blend_positions(&corners, &w.positions, len, &mut scratch);
                    if let Some(p) = parent
                        && let Some(io) = scratch.get_mut(..len)
                    {
                        self.apply_deformer(p, io);
                    }
                    let off = self.warp_grid_off.get(local).copied().unwrap_or(0);
                    if let (Some(dst), Some(src)) =
                        (self.warp_grid.get_mut(off..off.saturating_add(len)), scratch.get(..len))
                    {
                        dst.copy_from_slice(src);
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
            if !enabled {
                if let Some(o) = self.mesh_opacity.get_mut(mi) {
                    *o = 0.0;
                }
                // a hidden mesh that clips others still needs geometry; its
                // own binding decides whether there is anything to blend
                let needed =
                    self.mask_source.get(mi).copied().unwrap_or(false) && !corners.is_empty();
                if !needed {
                    continue;
                }
                let len = (m.vertex_count as usize).saturating_mul(2);
                let off = self.mesh_pos_off.get(mi).copied().unwrap_or(0);
                if let Some(out) = mesh_pos.get_mut(off..off.saturating_add(len)) {
                    Self::blend_positions(&corners, &m.positions, len, out);
                    if let Some(pd) = m.parent_deformer {
                        self.apply_deformer(pd as usize, out);
                    }
                }
                continue;
            }

            let len = (m.vertex_count as usize).saturating_mul(2);
            let off = self.mesh_pos_off.get(mi).copied().unwrap_or(0);
            if let Some(out) = mesh_pos.get_mut(off..off.saturating_add(len)) {
                Self::blend_positions(&corners, &m.positions, len, out);
                if let Some(pd) = m.parent_deformer {
                    self.apply_deformer(pd as usize, out);
                }
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
        if self.model.draw_groups.is_empty() {
            let orders = &self.mesh_draw_order;
            self.render_order.sort_unstable_by(|&a, &b| {
                let oa = orders.get(a as usize).copied().unwrap_or(500.0);
                let ob = orders.get(b as usize).copied().unwrap_or(500.0);
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
                        if let Some(cg) = item.child_group
                            && let Some(child_total) =
                                self.model.draw_groups.get(cg as usize).map(|g| g.total)
                        {
                            if let Some(c) = self.dog_cursor.get_mut(cg as usize) {
                                *c = pos;
                            }
                            pos = pos.saturating_add(child_total);
                        }
                    } else {
                        if let Some(slot) = self.render_order.get_mut(pos as usize) {
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
    }

    /// Map a point through one deformer's current state; editors use this
    /// to probe local bases when converting screen drags to parent space.
    pub fn deformer_point(&self, deformer: usize, x: f32, y: f32) -> (f32, f32) {
        let p = self.transform_point(deformer, v2(x, y));
        (p.x, p.y)
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
                opacity: self.mesh_opacity.get(mi).copied().unwrap_or(0.0),
                positions: self.mesh_pos.get(off..off.saturating_add(len)).unwrap_or(&[]),
                uvs: &m.uvs,
                indices: &m.indices,
                masks: &m.masks,
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
pub struct Drawable<'a> {
    pub mesh: usize,
    pub texture: u32,
    pub flags: u8,
    pub opacity: f32,
    /// xy pairs, `vertex_count * 2`.
    pub positions: &'a [f32],
    pub uvs: &'a [f32],
    pub indices: &'a [u16],
    /// Meshes whose texture alpha clips this one.
    pub masks: &'a [u32],
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
    use ayatsuri_format::model::{ArtMesh, Binding, BindingAxis, Canvas, Parameter};

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
                vertex_count: 3,
                uvs: vec![0.0; 6],
                indices: vec![0, 1, 2],
                masks: vec![],
                binding: Binding { axes: vec![BindingAxis { parameter: 0, keys: vec![0.0, 1.0] }] },
                opacities: vec![1.0, 0.5],
                draw_orders: vec![500.0, 500.0],
                positions: vec![
                    0.0, 0.0, 1.0, 0.0, 0.0, 1.0, // keyform 0
                    1.0, 0.0, 2.0, 0.0, 1.0, 1.0, // keyform 1
                ],
            }],
            glues: vec![],
            draw_groups: vec![],
            physics: None,
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
}
