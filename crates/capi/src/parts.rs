//! Part opacities as arrays: the host's own fade of a part, multiplied
//! over whatever the model's own keyforms give it.

use crate::error::{Status, guard, set_error};
use crate::handle::{Model, input, model, model_mut, output};

/// Every part's host opacity, by index, into `values`
/// (`ay2d_part_count` floats, 0 to 1; 1 when never set).
///
/// `m` is a live handle; `values` holds `n` writable floats.
#[unsafe(no_mangle)]
pub extern "C" fn ay2d_get_part_opacities(m: *const Model, values: *mut f32, n: usize) -> Status {
    guard(Status::Internal, || {
        let model = match model(m) {
            Ok(m) => m,
            Err(s) => return s,
        };
        let count = model.part_ids.len();
        if n < count {
            set_error(format!("{n} given, the model has {count} parts"));
            return Status::TooSmall;
        }
        let out = match output(values, count) {
            Ok(o) => o,
            Err(s) => return s,
        };
        for (i, slot) in out.iter_mut().enumerate() {
            *slot = model.puppet.part_user_opacity(i);
        }
        Status::Ok
    })
}

/// Set `n` parts' host opacities at once: `indices[i]` takes `values[i]`,
/// clamped to 0 to 1. An index past the model fails the whole call before
/// anything changes. Nothing is evaluated until `ay2d_update`.
///
/// `m` is a live handle; `indices` and `values` hold `n` readable items.
#[unsafe(no_mangle)]
pub extern "C" fn ay2d_set_part_opacities(
    m: *mut Model,
    indices: *const u32,
    values: *const f32,
    n: usize,
) -> Status {
    guard(Status::Internal, || {
        let model = match model_mut(m) {
            Ok(m) => m,
            Err(s) => return s,
        };
        let (idx, vals) = match (input(indices, n), input(values, n)) {
            (Ok(i), Ok(v)) => (i, v),
            (Err(s), _) | (_, Err(s)) => return s,
        };
        let count = model.part_ids.len();
        if let Some(bad) = idx.iter().find(|&&i| i as usize >= count) {
            set_error(format!("part index {bad} is past the model's {count}"));
            return Status::Range;
        }
        for (&i, &v) in idx.iter().zip(vals) {
            model.puppet.set_part_opacity(i as usize, v.clamp(0.0, 1.0));
        }
        Status::Ok
    })
}
