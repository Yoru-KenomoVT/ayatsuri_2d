//! What moved in the last update, per mesh, so a host that keeps geometry
//! on the GPU uploads only what changed. Worked out by comparing against
//! the previous frame after every evaluation; the first frame after load
//! reports everything as changed.

use crate::error::{Status, guard, set_error};
use crate::handle::{Model, model, output};

/// The mesh's vertex positions moved.
pub const AY2D_CHANGED_POSITIONS: u32 = 1;
/// The mesh came into or went out of the drawables.
pub const AY2D_CHANGED_VISIBILITY: u32 = 2;
/// The mesh's opacity changed.
pub const AY2D_CHANGED_OPACITY: u32 = 4;
/// The mesh's place in the draw order changed.
pub const AY2D_CHANGED_ORDER: u32 = 8;

/// The previous frame, kept to tell what moved.
#[derive(Default)]
pub(crate) struct Changes {
    positions: Vec<Vec<f32>>,
    visible: Vec<bool>,
    opacity: Vec<f32>,
    /// Each mesh's place in the draw order, or u32::MAX when not drawn.
    place: Vec<u32>,
    pub flags: Vec<u32>,
    primed: bool,
}

impl Changes {
    /// Compare the puppet's state with the frame before and remember it.
    pub(crate) fn after_update(&mut self, puppet: &ayatsuri_core::Puppet, order: &[usize]) {
        let n = puppet.model().art_meshes.len();
        if self.positions.len() != n {
            self.positions = vec![Vec::new(); n];
            self.visible = vec![false; n];
            self.opacity = vec![0.0; n];
            self.place = vec![u32::MAX; n];
            self.flags = vec![0; n];
            self.primed = false;
        }
        let mut place = vec![u32::MAX; n];
        let mut visible = vec![false; n];
        let mut opacity = vec![0.0f32; n];
        for (at, &mesh) in order.iter().enumerate() {
            if let Some(p) = place.get_mut(mesh) {
                *p = at as u32;
            }
        }
        for d in puppet.drawables() {
            if let (Some(v), Some(o)) = (visible.get_mut(d.mesh), opacity.get_mut(d.mesh)) {
                *v = true;
                *o = d.opacity;
            }
        }
        for mesh in 0..n {
            let mut f = 0;
            let now = puppet.mesh_geometry(mesh).map(|g| g.positions).unwrap_or(&[]);
            let was = self.positions.get(mesh).map(Vec::as_slice).unwrap_or(&[]);
            if !self.primed || now.len() != was.len() || now.iter().zip(was).any(|(a, b)| a != b) {
                f |= AY2D_CHANGED_POSITIONS;
                if let Some(slot) = self.positions.get_mut(mesh) {
                    slot.clear();
                    slot.extend_from_slice(now);
                }
            }
            if !self.primed || self.visible.get(mesh) != visible.get(mesh) {
                f |= AY2D_CHANGED_VISIBILITY;
            }
            if !self.primed || self.opacity.get(mesh) != opacity.get(mesh) {
                f |= AY2D_CHANGED_OPACITY;
            }
            if !self.primed || self.place.get(mesh) != place.get(mesh) {
                f |= AY2D_CHANGED_ORDER;
            }
            if let Some(slot) = self.flags.get_mut(mesh) {
                *slot = f;
            }
        }
        self.visible = visible;
        self.opacity = opacity;
        self.place = place;
        self.primed = true;
    }
}

/// What changed for every mesh in the last update, as `AY2D_CHANGED_*`
/// bits by mesh index, into `out` (`ay2d_mesh_count` entries). A host
/// streaming positions to the GPU uploads the meshes with the positions
/// bit and leaves the rest. After load every bit is set.
#[unsafe(no_mangle)]
pub extern "C" fn ay2d_mesh_changes(m: *const Model, out: *mut u32, n: usize) -> Status {
    guard(Status::Internal, || {
        let model = match model(m) {
            Ok(m) => m,
            Err(s) => return s,
        };
        let count = model.changes.flags.len();
        if n < count {
            set_error(format!("{n} given, the model has {count} meshes"));
            return Status::TooSmall;
        }
        let out = match output(out, count) {
            Ok(o) => o,
            Err(s) => return s,
        };
        out.copy_from_slice(&model.changes.flags);
        Status::Ok
    })
}
