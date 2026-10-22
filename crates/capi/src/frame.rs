//! What an update publishes. Two frames take turns: an update fills the
//! back one from the puppet and flips it to the front, so a render thread
//! reads frame N while the next update writes N + 1.
//!
//! ```text
//! update N + 1 ──► back ─┐ flip ┌─► front ◄── drawables, parameters,
//!                        └──────┘             changes, events (frame N)
//! ```
//!
//! A view taken from the front frame stays valid while the next update
//! runs and until the one after it starts writing that frame again.

use std::sync::atomic::Ordering;

use crate::error::{Status, guard, set_error};
use crate::handle::{Core, Model, lock, model, output};

/// The mesh's vertex positions moved.
pub const AY2D_CHANGED_POSITIONS: u32 = 1;
/// The mesh came into or went out of the drawables.
pub const AY2D_CHANGED_VISIBILITY: u32 = 2;
/// The mesh's opacity changed.
pub const AY2D_CHANGED_OPACITY: u32 = 4;
/// The mesh's place in the draw order changed.
pub const AY2D_CHANGED_ORDER: u32 = 8;

/// An event as a frame keeps it; the name is the address of a string the
/// model owns for its whole life.
#[derive(Clone, Copy)]
pub(crate) struct EventRec {
    pub motion: u32,
    pub time: f32,
    pub name: usize,
}

/// One published result.
pub(crate) struct Frame {
    /// Every mesh's positions at its fixed offset; sized once at load, so
    /// pointers into it never move.
    pub positions: Vec<f32>,
    /// The visible meshes back to front, with their opacity.
    pub drawn: Vec<(u32, f32)>,
    pub params: Vec<f32>,
    pub parts: Vec<f32>,
    pub changes: Vec<u32>,
    pub events: Vec<EventRec>,
    /// Per mesh, for telling what changed: drawn, opacity, place in order.
    visible: Vec<bool>,
    opacity: Vec<f32>,
    place: Vec<u32>,
    /// Holds a real result (the very first frame compares against nothing).
    primed: bool,
}

impl Frame {
    pub(crate) fn new(position_floats: usize) -> Self {
        Self {
            positions: vec![0.0; position_floats],
            drawn: Vec::new(),
            params: Vec::new(),
            parts: Vec::new(),
            changes: Vec::new(),
            events: Vec::new(),
            visible: Vec::new(),
            opacity: Vec::new(),
            place: Vec::new(),
            primed: false,
        }
    }

    /// A mesh's positions in this frame.
    pub(crate) fn mesh(&self, offset: usize, len: usize) -> &[f32] {
        self.positions.get(offset..offset.saturating_add(len)).unwrap_or(&[])
    }
}

/// Fill the back frame from the core's puppet, work out what changed
/// against the front, and flip. The caller holds the core.
pub(crate) fn publish(model: &Model, core: &Core) {
    let f = &model.facts;
    let front_at = model.front.load(Ordering::Acquire);
    let [a, b] = &model.frames;
    let (back_m, front_m) = if front_at == 0 { (b, a) } else { (a, b) };
    let mut back = lock(back_m);
    let back = &mut *back;
    let n = f.meshes.len();
    for (i, mesh) in f.meshes.iter().enumerate() {
        let src = core.puppet.mesh_geometry(i).map(|g| g.positions).unwrap_or(&[]);
        if let Some(dst) = back.positions.get_mut(mesh.offset..mesh.offset.saturating_add(mesh.len))
        {
            let k = dst.len().min(src.len());
            if let (Some(d), Some(s)) = (dst.get_mut(..k), src.get(..k)) {
                d.copy_from_slice(s);
            }
        }
    }
    back.drawn.clear();
    back.drawn.extend(core.puppet.drawables().map(|d| (d.mesh as u32, d.opacity)));
    back.params.clear();
    back.params.extend((0..f.param_ids.len()).map(|i| core.puppet.value(i)));
    back.parts.clear();
    back.parts.extend((0..f.part_ids.len()).map(|i| core.puppet.part_user_opacity(i)));
    back.events.clear();
    back.events.extend(core.player.fired().iter().map(|e| EventRec {
        motion: e.motion,
        time: e.time,
        name: e.name as usize,
    }));
    back.visible.clear();
    back.visible.resize(n, false);
    back.opacity.clear();
    back.opacity.resize(n, 0.0);
    back.place.clear();
    back.place.resize(n, u32::MAX);
    for (at, &(mesh, op)) in back.drawn.iter().enumerate() {
        let i = mesh as usize;
        if let (Some(v), Some(o), Some(p)) =
            (back.visible.get_mut(i), back.opacity.get_mut(i), back.place.get_mut(i))
        {
            *v = true;
            *o = op;
            *p = at as u32;
        }
    }
    back.changes.clear();
    back.changes.resize(n, 0);
    {
        // the front is only read here, and a reader holds one frame at a
        // time, so taking it after the back cannot deadlock
        let front = lock(front_m);
        for (i, mesh) in f.meshes.iter().enumerate() {
            let mut bits = 0;
            if !front.primed {
                bits = AY2D_CHANGED_POSITIONS
                    | AY2D_CHANGED_VISIBILITY
                    | AY2D_CHANGED_OPACITY
                    | AY2D_CHANGED_ORDER;
            } else {
                if back.mesh(mesh.offset, mesh.len) != front.mesh(mesh.offset, mesh.len) {
                    bits |= AY2D_CHANGED_POSITIONS;
                }
                if back.visible.get(i) != front.visible.get(i) {
                    bits |= AY2D_CHANGED_VISIBILITY;
                }
                if back.opacity.get(i) != front.opacity.get(i) {
                    bits |= AY2D_CHANGED_OPACITY;
                }
                if back.place.get(i) != front.place.get(i) {
                    bits |= AY2D_CHANGED_ORDER;
                }
            }
            if let Some(slot) = back.changes.get_mut(i) {
                *slot = bits;
            }
        }
    }
    back.primed = true;
    model.front.store(if front_at == 0 { 1 } else { 0 }, Ordering::Release);
}

/// What changed for every mesh in the last update, as `AY2D_CHANGED_*`
/// bits by mesh index, into `out` (`ay2d_mesh_count` entries). A host
/// streaming positions to the GPU uploads the meshes with the positions
/// bit and leaves the rest. After load every bit is set. Any thread.
#[unsafe(no_mangle)]
pub extern "C" fn ay2d_mesh_changes(m: *const Model, out: *mut u32, n: usize) -> Status {
    guard(Status::Internal, || {
        let model = match model(m) {
            Ok(m) => m,
            Err(s) => return s,
        };
        let frame = model.front();
        let count = frame.changes.len();
        if n < count {
            set_error(format!("{n} given, the model has {count} meshes"));
            return Status::TooSmall;
        }
        let out = match output(out, count) {
            Ok(o) => o,
            Err(s) => return s,
        };
        out.copy_from_slice(&frame.changes);
        Status::Ok
    })
}
