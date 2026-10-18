//! The C ABI. Coarse by design: arrays in, arrays out, never a call per
//! parameter (docs/integration.md). `include/ayatsuri.h` is the contract;
//! every function here is callable from C, keeps model state behind an
//! opaque handle, and never allocates per frame.
//!
//! Panics never cross the boundary: every entry point runs under a catch,
//! reports through `ay2d_last_error` and returns a status or a null.

mod bundle;
mod draw;
mod error;
mod expressions;
#[cfg(any(test, feature = "fixture"))]
pub mod fixture;
mod frame;
mod handle;
mod info;
mod motions;
mod params;
mod parts;
mod physics;
mod textures;

use std::ffi::c_char;

pub use bundle::{ay2d_add_expression3, ay2d_add_motion3, ay2d_add_physics3};
pub use draw::{
    AY2D_FLAG_ADDITIVE, AY2D_FLAG_DOUBLE_SIDED, AY2D_FLAG_INVERTED_MASK, AY2D_FLAG_MULTIPLICATIVE,
    Drawable, ay2d_drawables, ay2d_mesh_geometry,
};
pub use error::{Status, ay2d_last_error};
pub use expressions::{
    ay2d_expression_clear, ay2d_expression_count, ay2d_expression_id, ay2d_expression_index,
    ay2d_expression_set, ay2d_expression_shown,
};
pub use frame::{
    AY2D_CHANGED_OPACITY, AY2D_CHANGED_ORDER, AY2D_CHANGED_POSITIONS, AY2D_CHANGED_VISIBILITY,
    ay2d_mesh_changes,
};
pub use handle::{
    Canvas, Model, ay2d_canvas, ay2d_free, ay2d_load, ay2d_mesh_count, ay2d_mesh_id,
    ay2d_parameter_count, ay2d_parameter_id, ay2d_parameter_index, ay2d_part_count, ay2d_part_id,
    ay2d_texture_count,
};
pub use info::{
    ay2d_mesh_name, ay2d_mesh_part, ay2d_model_group, ay2d_model_group_count,
    ay2d_model_group_name, ay2d_parameter_group, ay2d_parameter_group_count,
    ay2d_parameter_group_name, ay2d_parameter_keys, ay2d_parameter_name, ay2d_parameter_repeat,
    ay2d_part_name, ay2d_part_parent,
};
pub use motions::{
    Event, MotionInfo, ay2d_motion_count, ay2d_motion_events, ay2d_motion_finished, ay2d_motion_id,
    ay2d_motion_index, ay2d_motion_info, ay2d_motion_play, ay2d_motion_playing, ay2d_motion_seek,
    ay2d_motion_set_speed, ay2d_motion_stop, ay2d_motion_time,
};
pub use params::{
    ay2d_get_parameters, ay2d_parameter_ranges, ay2d_set_all_parameters, ay2d_set_parameters,
};
pub use parts::{ay2d_get_part_opacities, ay2d_set_part_opacities};
pub use physics::{
    ay2d_has_physics, ay2d_reset_physics, ay2d_set_physics, ay2d_set_wind, ay2d_update,
};
pub use textures::ay2d_texture;

/// The ABI version this header describes. Bumped only on a breaking
/// change to it; a host compares it with `ay2d_abi_version()`.
pub const AY2D_ABI_VERSION: u32 = 1;

/// The ABI version the library was built with, for a host to check
/// against `AY2D_ABI_VERSION` before anything else.
#[unsafe(no_mangle)]
pub extern "C" fn ay2d_abi_version() -> u32 {
    AY2D_ABI_VERSION
}

/// The library's own version, as a NUL-terminated string that lives as
/// long as the library.
#[unsafe(no_mangle)]
pub extern "C" fn ay2d_version() -> *const c_char {
    concat!(env!("CARGO_PKG_VERSION"), "\0").as_ptr().cast()
}

#[cfg(test)]
mod tests;
