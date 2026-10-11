//! The C ABI. Coarse by design: arrays in, arrays out, never a call per
//! parameter (docs/integration.md).
//!
//! Every function here is callable from C, holds model state behind an opaque
//! handle, and never allocates per frame.

/// The ABI version a host checks before anything else. Bumped only on a
/// breaking change to the header.
#[unsafe(no_mangle)]
pub extern "C" fn ay2d_abi_version() -> u32 {
    1
}
