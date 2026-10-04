//! The opaque model handle: load, free, and what a host asks about a model
//! before it drives it. Ids are handed out as NUL-terminated strings that
//! live as long as the model, so a binding reads them once.

use std::ffi::{CString, c_char};

use ayatsuri_core::Puppet;

use crate::error::{Status, guard, set_error};

/// A loaded model: the puppet and what the ABI keeps beside it.
pub struct Model {
    pub(crate) puppet: Puppet,
    /// The texture pages as stored in the file, PNG, by texture index.
    pub(crate) textures: Vec<Vec<u8>>,
    pub(crate) param_ids: Vec<CString>,
    pub(crate) part_ids: Vec<CString>,
    pub(crate) mesh_ids: Vec<CString>,
    /// Where each visible drawable's data sits after the last update, in
    /// draw order, so `ay2d_drawables` hands out views without a sort.
    pub(crate) order: Vec<usize>,
}

/// The canvas the model is drawn on: pixels, and how model units map
/// onto them (`px = unit * pixels_per_unit + origin`).
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct Canvas {
    pub width: f32,
    pub height: f32,
    pub pixels_per_unit: f32,
    pub origin_x: f32,
    pub origin_y: f32,
}

fn cstrings<'a>(ids: impl Iterator<Item = &'a str>) -> Vec<CString> {
    ids.map(|id| CString::new(id.replace('\0', "")).unwrap_or_default()).collect()
}

/// The model behind a handle, or a status when there is none.
///
/// # Safety
/// `m` is null or a handle `ay2d_load` returned and `ay2d_free` has not
/// taken back.
pub(crate) unsafe fn model<'a>(m: *const Model) -> Result<&'a Model, Status> {
    if m.is_null() {
        set_error("model handle is null");
        return Err(Status::Null);
    }
    // SAFETY: a non-null handle from ay2d_load is a live Box<Model>
    Ok(unsafe { &*m })
}

/// # Safety
/// As [`model`], and no other reference to the model is alive.
pub(crate) unsafe fn model_mut<'a>(m: *mut Model) -> Result<&'a mut Model, Status> {
    if m.is_null() {
        set_error("model handle is null");
        return Err(Status::Null);
    }
    // SAFETY: a non-null handle from ay2d_load is a live Box<Model>
    Ok(unsafe { &mut *m })
}

/// A slice over a host array, or a status when the pointer is null while
/// the length is not.
///
/// # Safety
/// `ptr` points at `n` readable `T`s, or `n` is zero.
pub(crate) unsafe fn input<'a, T>(ptr: *const T, n: usize) -> Result<&'a [T], Status> {
    if n == 0 {
        return Ok(&[]);
    }
    if ptr.is_null() {
        set_error("array pointer is null");
        return Err(Status::Null);
    }
    // SAFETY: the host promises n readable elements
    Ok(unsafe { std::slice::from_raw_parts(ptr, n) })
}

/// # Safety
/// `ptr` points at `n` writable `T`s, or `n` is zero.
pub(crate) unsafe fn output<'a, T>(ptr: *mut T, n: usize) -> Result<&'a mut [T], Status> {
    if n == 0 {
        return Ok(&mut []);
    }
    if ptr.is_null() {
        set_error("array pointer is null");
        return Err(Status::Null);
    }
    // SAFETY: the host promises n writable elements
    Ok(unsafe { std::slice::from_raw_parts_mut(ptr, n) })
}

/// Load a model from the bytes of an `.aom` file. The bytes are read, not
/// kept: the host may free them as soon as this returns. Null when the
/// bytes are not a model; `ay2d_last_error` says why. The model is
/// evaluated once at its defaults, so drawables are ready at once.
///
/// # Safety
/// `bytes` points at `len` readable bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ay2d_load(bytes: *const u8, len: usize) -> *mut Model {
    guard(std::ptr::null_mut(), || {
        let Ok(bytes) = (unsafe { input(bytes, len) }) else { return std::ptr::null_mut() };
        let payload = match ayatsuri_format::read(bytes) {
            Ok(p) => p,
            Err(e) => {
                set_error(format!("not a model: {e}"));
                return std::ptr::null_mut();
            }
        };
        let m = &payload.model;
        let param_ids = cstrings(m.parameters.iter().map(|p| p.id.as_str()));
        let part_ids = cstrings(m.parts.iter().map(|p| p.id.as_str()));
        let mesh_ids = cstrings(m.art_meshes.iter().map(|a| a.id.as_str()));
        let mut puppet = Puppet::new(payload.model);
        puppet.update();
        let mut model = Model {
            puppet,
            textures: payload.textures.into_iter().map(|b| b.into_vec()).collect(),
            param_ids,
            part_ids,
            mesh_ids,
            order: Vec::new(),
        };
        model.refresh_order();
        Box::into_raw(Box::new(model))
    })
}

impl Model {
    /// The drawables' mesh indices after an update, in draw order.
    pub(crate) fn refresh_order(&mut self) {
        self.order.clear();
        self.order.extend(self.puppet.drawables().map(|d| d.mesh));
    }
}

/// Free a model. Null is fine. Every pointer the model handed out (ids,
/// drawables, textures) dies with it.
///
/// # Safety
/// `m` is null or a handle `ay2d_load` returned, freed once.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ay2d_free(m: *mut Model) {
    if !m.is_null() {
        // SAFETY: the handle came from Box::into_raw in ay2d_load
        drop(unsafe { Box::from_raw(m) });
    }
}

/// How many parameters the model has.
///
/// # Safety
/// `m` is a live handle.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ay2d_parameter_count(m: *const Model) -> u32 {
    guard(0, || unsafe { model(m) }.map_or(0, |m| m.param_ids.len() as u32))
}

/// How many parts the model has.
///
/// # Safety
/// `m` is a live handle.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ay2d_part_count(m: *const Model) -> u32 {
    guard(0, || unsafe { model(m) }.map_or(0, |m| m.part_ids.len() as u32))
}

/// How many meshes the model has; a drawable's `mesh` is an index into
/// them, and so is every mesh-wise call.
///
/// # Safety
/// `m` is a live handle.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ay2d_mesh_count(m: *const Model) -> u32 {
    guard(0, || unsafe { model(m) }.map_or(0, |m| m.mesh_ids.len() as u32))
}

/// How many texture pages the model carries.
///
/// # Safety
/// `m` is a live handle.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ay2d_texture_count(m: *const Model) -> u32 {
    guard(0, || unsafe { model(m) }.map_or(0, |m| m.textures.len() as u32))
}

/// The canvas: size in pixels and the model-unit to pixel mapping.
///
/// # Safety
/// `m` is a live handle; `out` points at one writable canvas.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ay2d_canvas(m: *const Model, out: *mut Canvas) -> Status {
    guard(Status::Internal, || {
        let model = match unsafe { model(m) } {
            Ok(m) => m,
            Err(s) => return s,
        };
        if out.is_null() {
            set_error("canvas pointer is null");
            return Status::Null;
        }
        let c = model.puppet.model().canvas;
        // SAFETY: the host promises one writable Canvas
        unsafe {
            *out = Canvas {
                width: c.width,
                height: c.height,
                pixels_per_unit: c.pixels_per_unit,
                origin_x: c.origin_x,
                origin_y: c.origin_y,
            };
        }
        Status::Ok
    })
}

fn id_at(ids: &[CString], i: u32) -> *const c_char {
    match ids.get(i as usize) {
        Some(s) => s.as_ptr(),
        None => {
            set_error(format!("index {i} is past the model's {}", ids.len()));
            std::ptr::null()
        }
    }
}

/// A parameter's id (`ParamAngleX`...), NUL-terminated, alive as long as
/// the model. Null past the count.
///
/// # Safety
/// `m` is a live handle.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ay2d_parameter_id(m: *const Model, i: u32) -> *const c_char {
    guard(std::ptr::null(), || {
        unsafe { model(m) }.map_or(std::ptr::null(), |m| id_at(&m.param_ids, i))
    })
}

/// A part's id, as [`ay2d_parameter_id`].
///
/// # Safety
/// `m` is a live handle.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ay2d_part_id(m: *const Model, i: u32) -> *const c_char {
    guard(std::ptr::null(), || {
        unsafe { model(m) }.map_or(std::ptr::null(), |m| id_at(&m.part_ids, i))
    })
}

/// A mesh's id, as [`ay2d_parameter_id`].
///
/// # Safety
/// `m` is a live handle.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ay2d_mesh_id(m: *const Model, i: u32) -> *const c_char {
    guard(std::ptr::null(), || {
        unsafe { model(m) }.map_or(std::ptr::null(), |m| id_at(&m.mesh_ids, i))
    })
}

/// The index of the parameter with this id, or -1 when the model has none.
/// Resolve ids once at load; drive by index after.
///
/// # Safety
/// `m` is a live handle; `id` is a NUL-terminated string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ay2d_parameter_index(m: *const Model, id: *const c_char) -> i32 {
    guard(-1, || {
        let Ok(model) = (unsafe { model(m) }) else { return -1 };
        if id.is_null() {
            set_error("id is null");
            return -1;
        }
        // SAFETY: the host promises a NUL-terminated string
        let want = unsafe { std::ffi::CStr::from_ptr(id) };
        model.param_ids.iter().position(|p| p.as_c_str() == want).map_or(-1, |i| i as i32)
    })
}
