//! The frame: one update evaluates the model, stepping the pendulum rig
//! first when it is on. Physics is a host switch; wind is a host input.

use crate::error::{Status, guard, set_error};
use crate::handle::{Model, model, model_mut};

/// Advance the model by `dt` seconds and evaluate it: playing motions and
/// the shown expression write their parameters over the pose, physics
/// steps (when the model has a rig and it is on), parameters resolve,
/// deformers and meshes move, draw order sorts. After this
/// `ay2d_drawables` and `ay2d_motion_events` are current. `dt` of zero
/// evaluates without moving motions or physics.
///
/// `m` is a live handle.
#[unsafe(no_mangle)]
pub extern "C" fn ay2d_update(m: *mut Model, dt: f32) -> Status {
    guard(Status::Internal, || {
        let model = match model_mut(m) {
            Ok(m) => m,
            Err(s) => return s,
        };
        if !dt.is_finite() || dt < 0.0 {
            set_error(format!("dt {dt} is not a span of time"));
            return Status::Range;
        }
        model.player.advance(dt);
        model.exprs.advance(dt);
        model.player.apply(
            &mut model.puppet,
            &model.scenes,
            &model.comps,
            &model.param_index,
            &model.part_index,
        );
        model.exprs.apply(&mut model.puppet);
        if model.physics_on && dt > 0.0 && model.puppet.has_physics() {
            model.puppet.step_physics(dt);
        }
        model.puppet.update();
        model.after_update();
        Status::Ok
    })
}

/// 1 when the model carries a pendulum rig, else 0.
///
/// `m` is a live handle.
#[unsafe(no_mangle)]
pub extern "C" fn ay2d_has_physics(m: *const Model) -> i32 {
    guard(0, || model(m).map_or(0, |m| i32::from(m.puppet.has_physics())))
}

/// Run (1) or hold (0) the rig on update. On by default when the model
/// has one; off, the physics-driven parameters keep their set values.
///
/// `m` is a live handle.
#[unsafe(no_mangle)]
pub extern "C" fn ay2d_set_physics(m: *mut Model, on: i32) -> Status {
    guard(Status::Internal, || match model_mut(m) {
        Ok(model) => {
            model.physics_on = on != 0;
            Status::Ok
        }
        Err(s) => s,
    })
}

/// Wind on the rig, in the same units gravity has (down is 0, -1).
///
/// `m` is a live handle.
#[unsafe(no_mangle)]
pub extern "C" fn ay2d_set_wind(m: *mut Model, x: f32, y: f32) -> Status {
    guard(Status::Internal, || match model_mut(m) {
        Ok(model) => {
            if !(x.is_finite() && y.is_finite()) {
                set_error("wind is not finite");
                return Status::Range;
            }
            model.puppet.set_physics_wind(x, y);
            Status::Ok
        }
        Err(s) => s,
    })
}

/// Put every pendulum back at rest, as after a scene cut.
///
/// `m` is a live handle.
#[unsafe(no_mangle)]
pub extern "C" fn ay2d_reset_physics(m: *mut Model) -> Status {
    guard(Status::Internal, || match model_mut(m) {
        Ok(model) => {
            model.puppet.reset_physics();
            Status::Ok
        }
        Err(s) => s,
    })
}
