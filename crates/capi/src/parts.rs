//! Part opacities as arrays: the host's own fade of a part, multiplied
//! over whatever the model's own keyforms give it. Through the inbox and
//! the published frame, like parameters: any thread.

use crate::error::{Status, guard, set_error};
use crate::handle::{Model, input, lock, model, output};

/// Every part's host opacity after the last update, by index, into
/// `values` (`ay2d_part_count` floats, 0 to 1; 1 when never set).
#[unsafe(no_mangle)]
pub extern "C" fn ay2d_get_part_opacities(m: *const Model, values: *mut f32, n: usize) -> Status {
    guard(Status::Internal, || {
        let model = match model(m) {
            Ok(m) => m,
            Err(s) => return s,
        };
        let frame = model.front();
        let count = frame.parts.len();
        if n < count {
            set_error(format!("{n} given, the model has {count} parts"));
            return Status::TooSmall;
        }
        let out = match output(values, count) {
            Ok(o) => o,
            Err(s) => return s,
        };
        out.copy_from_slice(&frame.parts);
        Status::Ok
    })
}

/// Set `n` parts' host opacities at once: `indices[i]` takes `values[i]`,
/// clamped to 0 to 1. An index past the model fails the whole call before
/// anything changes. Takes effect at the next `ay2d_update`.
#[unsafe(no_mangle)]
pub extern "C" fn ay2d_set_part_opacities(
    m: *mut Model,
    indices: *const u32,
    values: *const f32,
    n: usize,
) -> Status {
    guard(Status::Internal, || {
        let model = match model(m) {
            Ok(m) => m,
            Err(s) => return s,
        };
        let (idx, vals) = match (input(indices, n), input(values, n)) {
            (Ok(i), Ok(v)) => (i, v),
            (Err(s), _) | (_, Err(s)) => return s,
        };
        let count = model.facts.part_ids.len();
        if let Some(bad) = idx.iter().find(|&&i| i as usize >= count) {
            set_error(format!("part index {bad} is past the model's {count}"));
            return Status::Range;
        }
        let mut inbox = lock(&model.inbox);
        for (&i, &v) in idx.iter().zip(vals) {
            if let Some(slot) = inbox.parts.get_mut(i as usize) {
                *slot = Some(if v.is_finite() { v.clamp(0.0, 1.0) } else { 1.0 });
            }
        }
        Status::Ok
    })
}
