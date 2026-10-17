//! The rest of a Cubism bundle, for a model loaded from a `.moc3`: the
//! physics, motion and expression JSON files the host has beside it, each
//! handed over as bytes. An `.aom` carries all of this already.

use std::ffi::c_char;

use ayatsuri_core::Puppet;

use crate::error::{Status, guard, set_error};
use crate::handle::{Model, cstr_in, input, model_mut};

/// Give the model a pendulum rig from a `physics3.json` (`len` bytes at
/// `json`), replacing any it has. Parameter and part values stay as they
/// are; physics comes on.
#[unsafe(no_mangle)]
pub extern "C" fn ay2d_add_physics3(m: *mut Model, json: *const u8, len: usize) -> Status {
    guard(Status::Internal, || {
        let model = match model_mut(m) {
            Ok(m) => m,
            Err(s) => return s,
        };
        let bytes = match input(json, len) {
            Ok(b) => b,
            Err(s) => return s,
        };
        let rig = match ayatsuri_import::from_physics3(bytes) {
            Ok(r) => r,
            Err(e) => {
                set_error(format!("physics3: {e}"));
                return Status::BadModel;
            }
        };
        // the rig lives inside the puppet from construction: rebuild it
        // around the same model and carry the pose across
        let n = model.param_ids.len();
        let values: Vec<f32> = (0..n).map(|i| model.puppet.value(i)).collect();
        let parts: Vec<f32> =
            (0..model.part_ids.len()).map(|i| model.puppet.part_user_opacity(i)).collect();
        let mut rigged = model.puppet.model().clone();
        rigged.physics = Some(rig);
        let mut puppet = Puppet::new(rigged);
        for (i, v) in values.into_iter().enumerate() {
            puppet.set_value(i, v);
        }
        for (i, v) in parts.into_iter().enumerate() {
            puppet.set_part_opacity(i, v);
        }
        puppet.update();
        model.puppet = puppet;
        model.physics_on = true;
        model.after_update();
        Status::Ok
    })
}

/// Add a motion from a `motion3.json` under `name`; returns its index for
/// `ay2d_motion_play`, or -1 with a reason. Curves on parameters or parts
/// the model lacks are kept but drive nothing.
#[unsafe(no_mangle)]
pub extern "C" fn ay2d_add_motion3(
    m: *mut Model,
    name: *const c_char,
    json: *const u8,
    len: usize,
) -> i32 {
    guard(-1, || {
        let Ok(model) = model_mut(m) else { return -1 };
        let Ok(name) = cstr_in(name) else { return -1 };
        let Ok(bytes) = input(json, len) else { return -1 };
        let anim = match ayatsuri_import::from_motion3(bytes, &name.to_string_lossy()) {
            Ok(a) => a,
            Err(e) => {
                set_error(format!("motion3: {e}"));
                return -1;
            }
        };
        let index = model.scenes.len();
        let clip = model.player.add_scene(index, &anim, &model.param_index, &model.part_index);
        model.scenes.push(anim);
        clip as i32
    })
}

/// Add an expression from an `exp3.json` under `name`; returns its index
/// for `ay2d_expression_set`, or -1 with a reason.
#[unsafe(no_mangle)]
pub extern "C" fn ay2d_add_expression3(
    m: *mut Model,
    name: *const c_char,
    json: *const u8,
    len: usize,
) -> i32 {
    guard(-1, || {
        let Ok(model) = model_mut(m) else { return -1 };
        let Ok(name) = cstr_in(name) else { return -1 };
        let Ok(bytes) = input(json, len) else { return -1 };
        let e = match ayatsuri_import::from_exp3(bytes, &name.to_string_lossy()) {
            Ok(e) => e,
            Err(err) => {
                set_error(format!("exp3: {err}"));
                return -1;
            }
        };
        model.exprs.add(&e, &model.param_index) as i32
    })
}
