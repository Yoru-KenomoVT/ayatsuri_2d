//! Parameters as arrays. A host resolves ids to indices once, then every
//! frame is one call with the indices and values it has.

use crate::error::{Status, guard, set_error};
use crate::handle::{Model, input, model, model_mut, output};

/// Every parameter's minimum, maximum and default, by index. Each array
/// holds `ay2d_parameter_count` floats; any may be null to skip it.
///
/// # Safety
/// `m` is a live handle; each non-null array holds `n` writable floats.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ay2d_parameter_ranges(
    m: *const Model,
    min: *mut f32,
    max: *mut f32,
    default: *mut f32,
    n: usize,
) -> Status {
    guard(Status::Internal, || {
        let model = match unsafe { model(m) } {
            Ok(m) => m,
            Err(s) => return s,
        };
        let params = &model.puppet.model().parameters;
        if n < params.len() {
            set_error(format!("{n} given, the model has {} parameters", params.len()));
            return Status::TooSmall;
        }
        for (dst, pick) in [
            (min, (|p: &ayatsuri_format::model::Parameter| p.min) as fn(_) -> f32),
            (max, |p| p.max),
            (default, |p| p.default),
        ] {
            if dst.is_null() {
                continue;
            }
            let out = match unsafe { output(dst, params.len()) } {
                Ok(o) => o,
                Err(s) => return s,
            };
            for (slot, p) in out.iter_mut().zip(params) {
                *slot = pick(p);
            }
        }
        Status::Ok
    })
}

/// Every parameter's current value, by index, into `values`
/// (`ay2d_parameter_count` floats).
///
/// # Safety
/// `m` is a live handle; `values` holds `n` writable floats.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ay2d_get_parameters(
    m: *const Model,
    values: *mut f32,
    n: usize,
) -> Status {
    guard(Status::Internal, || {
        let model = match unsafe { model(m) } {
            Ok(m) => m,
            Err(s) => return s,
        };
        let count = model.param_ids.len();
        if n < count {
            set_error(format!("{n} given, the model has {count} parameters"));
            return Status::TooSmall;
        }
        let out = match unsafe { output(values, count) } {
            Ok(o) => o,
            Err(s) => return s,
        };
        for (i, slot) in out.iter_mut().enumerate() {
            *slot = model.puppet.value(i);
        }
        Status::Ok
    })
}

/// Set `n` parameters at once: `indices[i]` takes `values[i]`. Values are
/// clamped to the parameter's range. An index past the model fails the
/// whole call before anything changes. Nothing is evaluated until
/// `ay2d_update`.
///
/// # Safety
/// `m` is a live handle; `indices` and `values` hold `n` readable items.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ay2d_set_parameters(
    m: *mut Model,
    indices: *const u32,
    values: *const f32,
    n: usize,
) -> Status {
    guard(Status::Internal, || {
        let model = match unsafe { model_mut(m) } {
            Ok(m) => m,
            Err(s) => return s,
        };
        let (idx, vals) = match (unsafe { input(indices, n) }, unsafe { input(values, n) }) {
            (Ok(i), Ok(v)) => (i, v),
            (Err(s), _) | (_, Err(s)) => return s,
        };
        let count = model.param_ids.len();
        if let Some(bad) = idx.iter().find(|&&i| i as usize >= count) {
            set_error(format!("parameter index {bad} is past the model's {count}"));
            return Status::Range;
        }
        for (&i, &v) in idx.iter().zip(vals) {
            model.puppet.set_value(i as usize, v);
        }
        Status::Ok
    })
}

/// Set every parameter from one array of `ay2d_parameter_count` values,
/// by index. The usual frame call for a host that keeps the whole pose.
///
/// # Safety
/// `m` is a live handle; `values` holds `n` readable floats.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ay2d_set_all_parameters(
    m: *mut Model,
    values: *const f32,
    n: usize,
) -> Status {
    guard(Status::Internal, || {
        let model = match unsafe { model_mut(m) } {
            Ok(m) => m,
            Err(s) => return s,
        };
        let count = model.param_ids.len();
        if n < count {
            set_error(format!("{n} given, the model has {count} parameters"));
            return Status::TooSmall;
        }
        let vals = match unsafe { input(values, count) } {
            Ok(v) => v,
            Err(s) => return s,
        };
        for (i, &v) in vals.iter().enumerate() {
            model.puppet.set_value(i, v);
        }
        Status::Ok
    })
}
