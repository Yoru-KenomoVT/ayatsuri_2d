//! The frame: one update takes what the inbox holds, runs motions,
//! expressions and the rig, evaluates the model and publishes the result.
//! Physics is a host switch; wind is a host input.

use std::sync::atomic::Ordering;

use crate::error::{Status, guard, set_error};
use crate::handle::{Model, core, lock, model};

/// Advance the model by `dt` seconds and evaluate it: values set since the
/// last update come in, playing motions and the shown expression write
/// over the pose, physics steps (when the model has a rig and it is on),
/// deformers and meshes move, draw order sorts, and the result is
/// published for every reader. `dt` of zero evaluates without moving
/// motions or physics. One thread at a time drives a model: a second
/// thread calling while this runs gets `BUSY`.
#[unsafe(no_mangle)]
pub extern "C" fn ay2d_update(m: *mut Model, dt: f32) -> Status {
    guard(Status::Internal, || {
        let model = match model(m) {
            Ok(m) => m,
            Err(s) => return s,
        };
        if !dt.is_finite() || dt < 0.0 {
            set_error(format!("dt {dt} is not a span of time"));
            return Status::Range;
        }
        let mut g = match core(model) {
            Ok(g) => g,
            Err(s) => return s,
        };
        let c = &mut *g;
        {
            // the host's values become the base pose
            let mut inbox = lock(&model.inbox);
            for (slot, b) in inbox.params.iter_mut().zip(c.base.iter_mut()) {
                if let Some(v) = slot.take() {
                    *b = v;
                }
            }
            for (slot, b) in inbox.parts.iter_mut().zip(c.base_parts.iter_mut()) {
                if let Some(v) = slot.take() {
                    *b = v;
                }
            }
        }
        // every update starts from the base pose, so a cleared expression or
        // a stopped motion leaves nothing behind and an Add never piles up
        for (i, &v) in c.base.iter().enumerate() {
            c.puppet.set_value(i, v);
        }
        for (i, &v) in c.base_parts.iter().enumerate() {
            c.puppet.set_part_opacity(i, v);
        }
        c.player.advance(dt);
        c.exprs.advance(dt);
        c.player.apply(&mut c.puppet, &c.scenes, &c.comps, &c.param_index, &c.part_index);
        c.exprs.apply(&mut c.puppet);
        if c.physics_on && dt > 0.0 && c.puppet.has_physics() {
            c.puppet.step_physics(dt);
        }
        c.puppet.update();
        crate::frame::publish(model, c);
        Status::Ok
    })
}

/// 1 when the model carries a pendulum rig, else 0. Any thread.
#[unsafe(no_mangle)]
pub extern "C" fn ay2d_has_physics(m: *const Model) -> i32 {
    guard(0, || model(m).map_or(0, |m| i32::from(m.has_physics.load(Ordering::Acquire))))
}

/// Run (1) or hold (0) the rig on update. On by default when the model
/// has one; off, the physics-driven parameters keep their set values.
#[unsafe(no_mangle)]
pub extern "C" fn ay2d_set_physics(m: *mut Model, on: i32) -> Status {
    guard(Status::Internal, || {
        let model = match model(m) {
            Ok(m) => m,
            Err(s) => return s,
        };
        match core(model) {
            Ok(mut c) => {
                c.physics_on = on != 0;
                Status::Ok
            }
            Err(s) => s,
        }
    })
}

/// Wind on the rig, in the same units gravity has (down is 0, -1).
#[unsafe(no_mangle)]
pub extern "C" fn ay2d_set_wind(m: *mut Model, x: f32, y: f32) -> Status {
    guard(Status::Internal, || {
        let model = match model(m) {
            Ok(m) => m,
            Err(s) => return s,
        };
        if !(x.is_finite() && y.is_finite()) {
            set_error("wind is not finite");
            return Status::Range;
        }
        match core(model) {
            Ok(mut c) => {
                c.puppet.set_physics_wind(x, y);
                Status::Ok
            }
            Err(s) => s,
        }
    })
}

/// Put every pendulum back at rest, as after a scene cut.
#[unsafe(no_mangle)]
pub extern "C" fn ay2d_reset_physics(m: *mut Model) -> Status {
    guard(Status::Internal, || {
        let model = match model(m) {
            Ok(m) => m,
            Err(s) => return s,
        };
        match core(model) {
            Ok(mut c) => {
                c.puppet.reset_physics();
                Status::Ok
            }
            Err(s) => s,
        }
    })
}
