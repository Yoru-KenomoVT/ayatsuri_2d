//! Facts about the model a UI wants and a frame does not: display names,
//! the keys a parameter was rigged at, who sits under whom, the palette's
//! folders and the standard groups (eye blink, lip sync). Read once.

use std::ffi::{CString, c_char};

use ayatsuri_format::payload::DisplayInfo;

use crate::error::{guard, set_error};
use crate::handle::{Model, model, output};

/// What the file says about names and groups, resolved to indices and
/// NUL-terminated strings at load.
#[derive(Default)]
pub(crate) struct Info {
    pub param_names: Vec<CString>,
    pub part_names: Vec<CString>,
    pub mesh_names: Vec<CString>,
    /// Palette folders: name and member parameter indices.
    pub groups: Vec<(CString, Vec<u32>)>,
    /// The model3 groups (EyeBlink, LipSync...): name and parameter indices.
    pub model_groups: Vec<(CString, Vec<u32>)>,
    /// Every key value a parameter is rigged at, sorted, by parameter.
    pub keys: Vec<Vec<f32>>,
}

fn cstring(s: &str) -> CString {
    CString::new(s.replace('\0', "")).unwrap_or_default()
}

impl Info {
    pub(crate) fn new(
        model: &ayatsuri_format::Model,
        display: Option<&DisplayInfo>,
        param_index: &std::collections::HashMap<String, usize>,
    ) -> Self {
        // names fall back to ids where the file has none
        let names = |given: Option<&Vec<String>>, ids: Vec<&str>| -> Vec<CString> {
            ids.iter()
                .enumerate()
                .map(|(i, id)| {
                    given
                        .and_then(|g| g.get(i))
                        .filter(|n| !n.is_empty())
                        .map_or_else(|| cstring(id), |n| cstring(n))
                })
                .collect()
        };
        let param_names = names(
            display.map(|d| &d.param_names),
            model.parameters.iter().map(|p| p.id.as_str()).collect(),
        );
        let part_names = names(
            display.map(|d| &d.part_names),
            model.parts.iter().map(|p| p.id.as_str()).collect(),
        );
        let mesh_names = names(
            display.map(|d| &d.mesh_names),
            model.art_meshes.iter().map(|a| a.id.as_str()).collect(),
        );
        let n = model.parameters.len() as u32;
        let groups = display
            .map(|d| {
                d.groups
                    .iter()
                    .map(|(name, members)| {
                        (cstring(name), members.iter().copied().filter(|&m| m < n).collect())
                    })
                    .collect()
            })
            .unwrap_or_default();
        let model_groups = display
            .and_then(|d| d.model_groups.as_ref())
            .map(|gs| {
                gs.iter()
                    .map(|(_, name, ids)| {
                        let members = ids
                            .iter()
                            .filter_map(|id| param_index.get(id).map(|&i| i as u32))
                            .collect();
                        (cstring(name), members)
                    })
                    .collect()
            })
            .unwrap_or_default();
        // the keys: every binding axis of every object, per parameter
        let mut keys: Vec<Vec<f32>> = vec![Vec::new(); model.parameters.len()];
        let bindings = model
            .art_meshes
            .iter()
            .map(|a| &a.binding)
            .chain(model.deformers.iter().map(|d| &d.binding))
            .chain(model.parts.iter().map(|p| &p.binding));
        for b in bindings {
            for axis in &b.axes {
                if let Some(slot) = keys.get_mut(axis.parameter as usize) {
                    slot.extend(axis.keys.iter().copied().filter(|k| k.is_finite()));
                }
            }
        }
        for slot in &mut keys {
            slot.sort_by(f32::total_cmp);
            slot.dedup_by(|a, b| (*a - *b).abs() < 1e-5);
        }
        Self { param_names, part_names, mesh_names, groups, model_groups, keys }
    }
}

fn name_at(list: &[CString], i: u32, what: &str) -> *const c_char {
    match list.get(i as usize) {
        Some(s) => s.as_ptr(),
        None => {
            set_error(format!("{what} {i} is past the model's {}", list.len()));
            std::ptr::null()
        }
    }
}

/// A parameter's display name ("Angle X"), the id when the file names
/// none. NUL-terminated, alive as long as the model.
#[unsafe(no_mangle)]
pub extern "C" fn ay2d_parameter_name(m: *const Model, i: u32) -> *const c_char {
    guard(std::ptr::null(), || {
        model(m).map_or(std::ptr::null(), |m| name_at(&m.info.param_names, i, "parameter"))
    })
}

/// A part's display name, as [`ay2d_parameter_name`].
#[unsafe(no_mangle)]
pub extern "C" fn ay2d_part_name(m: *const Model, i: u32) -> *const c_char {
    guard(std::ptr::null(), || {
        model(m).map_or(std::ptr::null(), |m| name_at(&m.info.part_names, i, "part"))
    })
}

/// A mesh's display name, as [`ay2d_parameter_name`].
#[unsafe(no_mangle)]
pub extern "C" fn ay2d_mesh_name(m: *const Model, i: u32) -> *const c_char {
    guard(std::ptr::null(), || {
        model(m).map_or(std::ptr::null(), |m| name_at(&m.info.mesh_names, i, "mesh"))
    })
}

/// 1 when parameter `i` wraps around its range instead of clamping
/// (turn-around heads).
#[unsafe(no_mangle)]
pub extern "C" fn ay2d_parameter_repeat(m: *const Model, i: u32) -> i32 {
    guard(0, || {
        model(m).map_or(0, |m| {
            m.puppet.model().parameters.get(i as usize).map_or(0, |p| i32::from(p.repeat))
        })
    })
}

/// The values parameter `i` was rigged at (the keyform positions), sorted,
/// into `out`. Returns how many there are; with `n` smaller only the first
/// `n` are written. A UI shows these as the dots on a slider.
#[unsafe(no_mangle)]
pub extern "C" fn ay2d_parameter_keys(m: *const Model, i: u32, out: *mut f32, n: usize) -> u32 {
    guard(0, || {
        let Ok(model) = model(m) else { return 0 };
        let Some(keys) = model.info.keys.get(i as usize) else {
            set_error(format!("parameter {i} is past the model's {}", model.info.keys.len()));
            return 0;
        };
        let Ok(out) = output(out, n) else { return 0 };
        for (slot, k) in out.iter_mut().zip(keys) {
            *slot = *k;
        }
        keys.len() as u32
    })
}

/// The part that holds part `i`, or -1 at the top.
#[unsafe(no_mangle)]
pub extern "C" fn ay2d_part_parent(m: *const Model, i: u32) -> i32 {
    guard(-1, || {
        model(m).map_or(-1, |m| {
            m.puppet
                .model()
                .parts
                .get(i as usize)
                .map_or(-1, |p| p.parent_part.map_or(-1, |x| x as i32))
        })
    })
}

/// The part that holds mesh `i`, or -1 when it sits at the top.
#[unsafe(no_mangle)]
pub extern "C" fn ay2d_mesh_part(m: *const Model, i: u32) -> i32 {
    guard(-1, || {
        model(m).map_or(-1, |m| {
            m.puppet
                .model()
                .art_meshes
                .get(i as usize)
                .map_or(-1, |a| a.parent_part.map_or(-1, |x| x as i32))
        })
    })
}

/// How many parameter folders the file's palette has.
#[unsafe(no_mangle)]
pub extern "C" fn ay2d_parameter_group_count(m: *const Model) -> u32 {
    guard(0, || model(m).map_or(0, |m| m.info.groups.len() as u32))
}

/// A folder's name, NUL-terminated, alive as long as the model.
#[unsafe(no_mangle)]
pub extern "C" fn ay2d_parameter_group_name(m: *const Model, g: u32) -> *const c_char {
    guard(std::ptr::null(), || {
        let Ok(model) = model(m) else { return std::ptr::null() };
        match model.info.groups.get(g as usize) {
            Some((name, _)) => name.as_ptr(),
            None => {
                set_error(format!("group {g} is past the model's {}", model.info.groups.len()));
                std::ptr::null()
            }
        }
    })
}

/// The parameters in folder `g`, by index, into `out`. Returns how many
/// there are; with `n` smaller only the first `n` are written.
#[unsafe(no_mangle)]
pub extern "C" fn ay2d_parameter_group(m: *const Model, g: u32, out: *mut u32, n: usize) -> u32 {
    guard(0, || {
        let Ok(model) = model(m) else { return 0 };
        let Some((_, members)) = model.info.groups.get(g as usize) else {
            set_error(format!("group {g} is past the model's {}", model.info.groups.len()));
            return 0;
        };
        let Ok(out) = output(out, n) else { return 0 };
        for (slot, i) in out.iter_mut().zip(members) {
            *slot = *i;
        }
        members.len() as u32
    })
}

/// How many standard groups (EyeBlink, LipSync...) the model names.
#[unsafe(no_mangle)]
pub extern "C" fn ay2d_model_group_count(m: *const Model) -> u32 {
    guard(0, || model(m).map_or(0, |m| m.info.model_groups.len() as u32))
}

/// A standard group's name ("EyeBlink", "LipSync"), NUL-terminated, alive
/// as long as the model.
#[unsafe(no_mangle)]
pub extern "C" fn ay2d_model_group_name(m: *const Model, g: u32) -> *const c_char {
    guard(std::ptr::null(), || {
        let Ok(model) = model(m) else { return std::ptr::null() };
        match model.info.model_groups.get(g as usize) {
            Some((name, _)) => name.as_ptr(),
            None => {
                set_error(format!(
                    "group {g} is past the model's {}",
                    model.info.model_groups.len()
                ));
                std::ptr::null()
            }
        }
    })
}

/// The parameters in standard group `g`, by index, into `out`: the ones a
/// blink or a lip sync should drive. Returns how many there are.
#[unsafe(no_mangle)]
pub extern "C" fn ay2d_model_group(m: *const Model, g: u32, out: *mut u32, n: usize) -> u32 {
    guard(0, || {
        let Ok(model) = model(m) else { return 0 };
        let Some((_, members)) = model.info.model_groups.get(g as usize) else {
            set_error(format!("group {g} is past the model's {}", model.info.model_groups.len()));
            return 0;
        };
        let Ok(out) = output(out, n) else { return 0 };
        for (slot, i) in out.iter_mut().zip(members) {
            *slot = *i;
        }
        members.len() as u32
    })
}
