//! Parameters as arrays. A host resolves ids to indices once, then every
//! frame is one call with the indices and values it has. Setting goes into
//! the model's inbox and reading comes from the last published frame, so
//! both work from any thread, while an update runs on another.

use crate::error::{Status, guard, set_error};
use crate::handle::{Model, input, lock, model, output};

/// Every parameter's minimum, maximum and default, by index. Each array
/// holds `ay2d_parameter_count` floats; any may be null to skip it.
#[unsafe(no_mangle)]
pub extern "C" fn ay2d_parameter_ranges(
    m: *const Model,
    min: *mut f32,
    max: *mut f32,
    default: *mut f32,
    n: usize,
) -> Status {
    guard(Status::Internal, || {
        let model = match model(m) {
            Ok(m) => m,
            Err(s) => return s,
        };
        let ranges = &model.facts.ranges;
        if n < ranges.len() {
            set_error(format!("{n} given, the model has {} parameters", ranges.len()));
            return Status::TooSmall;
        }
        for (dst, pick) in [
            (min, (|r: &(f32, f32, f32)| r.0) as fn(&(f32, f32, f32)) -> f32),
            (max, |r| r.1),
            (default, |r| r.2),
        ] {
            if dst.is_null() {
                continue;
            }
            let out = match output(dst, ranges.len()) {
                Ok(o) => o,
                Err(s) => return s,
            };
            for (slot, r) in out.iter_mut().zip(ranges) {
                *slot = pick(r);
            }
        }
        Status::Ok
    })
}

/// Every parameter's value after the last update, by index, into `values`
/// (`ay2d_parameter_count` floats): what motions, expressions and physics
/// made of the pose. A value set since then shows after the next update.
#[unsafe(no_mangle)]
pub extern "C" fn ay2d_get_parameters(m: *const Model, values: *mut f32, n: usize) -> Status {
    guard(Status::Internal, || {
        let model = match model(m) {
            Ok(m) => m,
            Err(s) => return s,
        };
        let frame = model.front();
        let count = frame.params.len();
        if n < count {
            set_error(format!("{n} given, the model has {count} parameters"));
            return Status::TooSmall;
        }
        let out = match output(values, count) {
            Ok(o) => o,
            Err(s) => return s,
        };
        out.copy_from_slice(&frame.params);
        Status::Ok
    })
}

/// A value held to its parameter's range; a NaN lands on the default.
fn clamped(model: &Model, i: usize, v: f32) -> f32 {
    match model.facts.ranges.get(i) {
        Some(&(lo, hi, _)) if v.is_finite() => v.clamp(lo.min(hi), hi.max(lo)),
        Some(&(_, _, def)) => def,
        None => v,
    }
}

/// Set `n` parameters at once: `indices[i]` takes `values[i]`. Values are
/// clamped to the parameter's range. An index past the model fails the
/// whole call before anything changes. The values take effect at the next
/// `ay2d_update`. Any thread, while an update runs on another.
#[unsafe(no_mangle)]
pub extern "C" fn ay2d_set_parameters(
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
        let count = model.facts.param_ids.len();
        if let Some(bad) = idx.iter().find(|&&i| i as usize >= count) {
            set_error(format!("parameter index {bad} is past the model's {count}"));
            return Status::Range;
        }
        let mut inbox = lock(&model.inbox);
        for (&i, &v) in idx.iter().zip(vals) {
            if let Some(slot) = inbox.params.get_mut(i as usize) {
                *slot = Some(clamped(model, i as usize, v));
            }
        }
        Status::Ok
    })
}

/// Set every parameter from one array of `ay2d_parameter_count` values,
/// by index. The usual frame call for a host that keeps the whole pose.
/// Takes effect at the next update; any thread.
#[unsafe(no_mangle)]
pub extern "C" fn ay2d_set_all_parameters(m: *mut Model, values: *const f32, n: usize) -> Status {
    guard(Status::Internal, || {
        let model = match model(m) {
            Ok(m) => m,
            Err(s) => return s,
        };
        let count = model.facts.param_ids.len();
        if n < count {
            set_error(format!("{n} given, the model has {count} parameters"));
            return Status::TooSmall;
        }
        let vals = match input(values, count) {
            Ok(v) => v,
            Err(s) => return s,
        };
        let mut inbox = lock(&model.inbox);
        for (i, (slot, &v)) in inbox.params.iter_mut().zip(vals).enumerate() {
            *slot = Some(clamped(model, i, v));
        }
        Status::Ok
    })
}
