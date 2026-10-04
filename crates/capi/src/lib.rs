//! The C ABI. Coarse by design: arrays in, arrays out, never a call per
//! parameter (docs/integration.md). `include/ayatsuri.h` is the contract;
//! every function here is callable from C, keeps model state behind an
//! opaque handle, and never allocates per frame.
//!
//! Panics never cross the boundary: every entry point runs under a catch,
//! reports through `ay2d_last_error` and returns a status or a null.

mod error;
mod handle;
mod params;

use std::ffi::c_char;

pub use error::{Status, ay2d_last_error};
pub use handle::{
    Canvas, Model, ay2d_canvas, ay2d_free, ay2d_load, ay2d_mesh_count, ay2d_mesh_id,
    ay2d_parameter_count, ay2d_parameter_id, ay2d_parameter_index, ay2d_part_count, ay2d_part_id,
    ay2d_texture_count,
};
pub use params::{
    ay2d_get_parameters, ay2d_parameter_ranges, ay2d_set_all_parameters, ay2d_set_parameters,
};

/// The ABI version a host checks before anything else. Bumped only on a
/// breaking change to the header.
#[unsafe(no_mangle)]
pub extern "C" fn ay2d_abi_version() -> u32 {
    1
}

/// The library's own version, as a NUL-terminated string that lives as
/// long as the library.
#[unsafe(no_mangle)]
pub extern "C" fn ay2d_version() -> *const c_char {
    concat!(env!("CARGO_PKG_VERSION"), "\0").as_ptr().cast()
}
