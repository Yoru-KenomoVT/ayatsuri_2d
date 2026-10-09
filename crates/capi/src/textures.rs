//! The texture pages, as the file stores them: PNG bytes a host decodes
//! with whatever it has. Decoding is the one thing this ABI leaves to the
//! host, because every engine already owns an image loader.

use crate::error::{Status, guard, set_error};
use crate::handle::{Model, model};

/// Texture page `i` as encoded bytes (PNG), alive as long as the model.
/// A drawable's `texture` is an index into these.
///
/// # Safety
/// `m` is a live handle; `bytes` and `len` each point at one writable slot.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ay2d_texture(
    m: *const Model,
    i: u32,
    bytes: *mut *const u8,
    len: *mut usize,
) -> Status {
    guard(Status::Internal, || {
        let model = match unsafe { model(m) } {
            Ok(m) => m,
            Err(s) => return s,
        };
        if bytes.is_null() || len.is_null() {
            set_error("output pointer is null");
            return Status::Null;
        }
        let Some(png) = model.textures.get(i as usize) else {
            set_error(format!("texture {i} is past the model's {}", model.textures.len()));
            return Status::Range;
        };
        // SAFETY: the host promises one writable slot each
        unsafe {
            *bytes = png.as_ptr();
            *len = png.len();
        }
        Status::Ok
    })
}
