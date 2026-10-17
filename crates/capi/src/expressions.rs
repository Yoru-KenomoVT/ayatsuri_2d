//! Expressions: the model's exp3 sets, one shown at a time, faded in over
//! the pose after motions have had their say and before physics.

use std::ffi::{CString, c_char};

use ayatsuri_core::Puppet;
use ayatsuri_format::{Expression, ExpressionBlend};

use crate::error::{Status, guard, set_error};
use crate::handle::{Model, cstr_in, model, model_mut};
use crate::motions::cstring;

const DEFAULT_FADE: f32 = 1.0;

pub(crate) struct Set {
    pub id: CString,
    /// Parameter index, value and blend, for the parameters the model has.
    pub params: Vec<(usize, f32, ExpressionBlend)>,
    pub fade_in: Option<f32>,
    pub fade_out: Option<f32>,
}

struct Shown {
    set: usize,
    fade_in: f32,
    elapsed: f32,
    out: Option<(f32, f32, f32)>,
}

impl Shown {
    fn weight(&self) -> f32 {
        let w_in = if self.fade_in <= 0.0 { 1.0 } else { (self.elapsed / self.fade_in).min(1.0) };
        match self.out {
            Some((from, dur, gone)) => {
                if dur <= 0.0 {
                    0.0
                } else {
                    from * (1.0 - gone / dur).max(0.0)
                }
            }
            None => w_in,
        }
    }
}

#[derive(Default)]
pub(crate) struct Expressions {
    pub sets: Vec<Set>,
    shown: Vec<Shown>,
}

impl Expressions {
    pub(crate) fn new(
        exprs: &[Expression],
        params: &std::collections::HashMap<String, usize>,
    ) -> Self {
        let mut out = Self::default();
        for e in exprs {
            out.add(e, params);
        }
        out
    }

    /// An expression added after load (an exp3 file); returns its index.
    pub(crate) fn add(
        &mut self,
        e: &Expression,
        params: &std::collections::HashMap<String, usize>,
    ) -> usize {
        self.sets.push(Set {
            id: cstring(&e.name),
            params: e
                .params
                .iter()
                .filter_map(|p| params.get(&p.id).map(|&i| (i, p.value, p.blend)))
                .collect(),
            fade_in: e.fade_in,
            fade_out: e.fade_out,
        });
        self.sets.len() - 1
    }

    fn set(&mut self, set: usize, fade_in: f32) {
        let authored = self.sets.get(set).and_then(|s| s.fade_in);
        let fade_in = if fade_in >= 0.0 { fade_in } else { authored.unwrap_or(DEFAULT_FADE) };
        self.clear(-1.0);
        self.shown.push(Shown { set, fade_in, elapsed: 0.0, out: None });
    }

    fn clear(&mut self, fade_out: f32) {
        for s in &mut self.shown {
            if s.out.is_some() {
                continue;
            }
            let authored = self.sets.get(s.set).and_then(|e| e.fade_out);
            let dur = if fade_out >= 0.0 { fade_out } else { authored.unwrap_or(DEFAULT_FADE) };
            s.out = Some((s.weight(), dur, 0.0));
        }
    }

    fn current(&self) -> Option<usize> {
        self.shown.iter().rev().find(|s| s.out.is_none()).map(|s| s.set)
    }

    pub(crate) fn advance(&mut self, dt: f32) {
        for s in &mut self.shown {
            s.elapsed += dt;
            if let Some((_, _, gone)) = &mut s.out {
                *gone += dt;
            }
        }
        self.shown.retain(|s| s.out.is_none_or(|(_, dur, gone)| dur > 0.0 && gone < dur));
    }

    pub(crate) fn apply(&self, puppet: &mut Puppet) {
        for s in &self.shown {
            let w = s.weight();
            if w <= 0.0 {
                continue;
            }
            let Some(set) = self.sets.get(s.set) else { continue };
            for &(i, value, blend) in &set.params {
                let base = puppet.value(i);
                let v = match blend {
                    ExpressionBlend::Add => base + value,
                    ExpressionBlend::Multiply => base * value,
                    ExpressionBlend::Overwrite => value,
                };
                puppet.set_value(i, base + (v - base) * w);
            }
        }
    }
}

/// How many expressions the model carries.
///
/// `m` is a live handle.
#[unsafe(no_mangle)]
pub extern "C" fn ay2d_expression_count(m: *const Model) -> u32 {
    guard(0, || model(m).map_or(0, |m| m.exprs.sets.len() as u32))
}

/// An expression's name, NUL-terminated, alive as long as the model.
///
/// `m` is a live handle.
#[unsafe(no_mangle)]
pub extern "C" fn ay2d_expression_id(m: *const Model, i: u32) -> *const c_char {
    guard(std::ptr::null(), || {
        let Ok(model) = model(m) else { return std::ptr::null() };
        match model.exprs.sets.get(i as usize) {
            Some(s) => s.id.as_ptr(),
            None => {
                set_error(format!("expression {i} is past the model's {}", model.exprs.sets.len()));
                std::ptr::null()
            }
        }
    })
}

/// The index of the expression with this name, or -1.
///
/// `m` is a live handle; `name` is a NUL-terminated string.
#[unsafe(no_mangle)]
pub extern "C" fn ay2d_expression_index(m: *const Model, name: *const c_char) -> i32 {
    guard(-1, || {
        let Ok(model) = model(m) else { return -1 };
        let Ok(want) = cstr_in(name) else { return -1 };
        model.exprs.sets.iter().position(|s| s.id.as_c_str() == want).map_or(-1, |i| i as i32)
    })
}

/// Show expression `i`: the one shown before fades out over its own fade
/// while this one fades in over `fade_in` seconds (below zero: its
/// authored fade or the spec's second; zero: at once).
///
/// `m` is a live handle.
#[unsafe(no_mangle)]
pub extern "C" fn ay2d_expression_set(m: *mut Model, i: u32, fade_in: f32) -> Status {
    guard(Status::Internal, || {
        let model = match model_mut(m) {
            Ok(m) => m,
            Err(s) => return s,
        };
        if i as usize >= model.exprs.sets.len() {
            set_error(format!("expression {i} is past the model's {}", model.exprs.sets.len()));
            return Status::Range;
        }
        if !fade_in.is_finite() {
            set_error("fade is not finite");
            return Status::Range;
        }
        model.exprs.set(i as usize, fade_in);
        Status::Ok
    })
}

/// Fade the shown expression out over `fade_out` seconds (below zero: its
/// own fade or the spec's second; zero: at once).
///
/// `m` is a live handle.
#[unsafe(no_mangle)]
pub extern "C" fn ay2d_expression_clear(m: *mut Model, fade_out: f32) -> Status {
    guard(Status::Internal, || {
        let model = match model_mut(m) {
            Ok(m) => m,
            Err(s) => return s,
        };
        if !fade_out.is_finite() {
            set_error("fade is not finite");
            return Status::Range;
        }
        model.exprs.clear(fade_out);
        Status::Ok
    })
}

/// The expression shown (the last one set and not cleared), or -1.
///
/// `m` is a live handle.
#[unsafe(no_mangle)]
pub extern "C" fn ay2d_expression_shown(m: *const Model) -> i32 {
    guard(-1, || model(m).map_or(-1, |m| m.exprs.current().map_or(-1, |i| i as i32)))
}
