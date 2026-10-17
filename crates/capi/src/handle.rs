//! The opaque model handle: load, free, and what a host asks about a model
//! before it drives it. Ids are handed out as NUL-terminated strings that
//! live as long as the model, so a binding reads them once.

use std::collections::HashMap;
use std::ffi::{CStr, CString, c_char};

use ayatsuri_core::Puppet;
use ayatsuri_format::{Animation, Composition};

use crate::error::{Status, guard, set_error};

/// A loaded model: the puppet and what the ABI keeps beside it.
pub struct Model {
    pub(crate) puppet: Puppet,
    /// The texture pages as stored in the file, PNG, by texture index.
    pub(crate) textures: Vec<Vec<u8>>,
    pub(crate) param_ids: Vec<CString>,
    pub(crate) part_ids: Vec<CString>,
    pub(crate) mesh_ids: Vec<CString>,
    /// Physics runs on update while this is set; on by default when the
    /// model has a rig.
    pub(crate) physics_on: bool,
    /// The file's scenes and compositions, for the motion player.
    pub(crate) scenes: Vec<Animation>,
    pub(crate) comps: Vec<Composition>,
    pub(crate) param_index: HashMap<String, usize>,
    pub(crate) part_index: HashMap<String, usize>,
    pub(crate) player: crate::motions::Player,
    pub(crate) exprs: crate::expressions::Expressions,
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

// ---- the boundary -------------------------------------------------------
//
// Every raw pointer the host hands over is turned into a reference, a slice
// or a string here and nowhere else; the other modules are safe Rust over
// these six helpers. The contracts are the ABI's (ayatsuri.h): a handle is
// one `ay2d_load` returned and `ay2d_free` has not taken back, an array
// pointer covers its stated length, a string ends in NUL. A test keeps the
// `unsafe` keyword out of every other file.

/// The model behind a handle, or `Null` when there is none. The host
/// promises `m` is null or a live handle from `ay2d_load`.
pub(crate) fn model<'a>(m: *const Model) -> Result<&'a Model, Status> {
    if m.is_null() {
        set_error("model handle is null");
        return Err(Status::Null);
    }
    // SAFETY: the ABI's contract; a non-null handle from ay2d_load is a
    // live Box<Model> until ay2d_free
    Ok(unsafe { &*m })
}

/// As [`model`]; the host promises no other reference is alive, which the
/// one-thread-per-model rule gives.
pub(crate) fn model_mut<'a>(m: *mut Model) -> Result<&'a mut Model, Status> {
    if m.is_null() {
        set_error("model handle is null");
        return Err(Status::Null);
    }
    // SAFETY: the ABI's contract, as for `model`
    Ok(unsafe { &mut *m })
}

/// A slice over a host array; the host promises `ptr` covers `n` readable
/// elements (a null pointer with `n` of zero is an empty array).
pub(crate) fn input<'a, T>(ptr: *const T, n: usize) -> Result<&'a [T], Status> {
    if n == 0 {
        return Ok(&[]);
    }
    if ptr.is_null() {
        set_error("array pointer is null");
        return Err(Status::Null);
    }
    // SAFETY: the ABI's contract; n readable elements at ptr
    Ok(unsafe { std::slice::from_raw_parts(ptr, n) })
}

/// A writable slice over a host array; the host promises `ptr` covers `n`
/// writable elements.
pub(crate) fn output<'a, T>(ptr: *mut T, n: usize) -> Result<&'a mut [T], Status> {
    if n == 0 {
        return Ok(&mut []);
    }
    if ptr.is_null() {
        set_error("array pointer is null");
        return Err(Status::Null);
    }
    // SAFETY: the ABI's contract; n writable elements at ptr
    Ok(unsafe { std::slice::from_raw_parts_mut(ptr, n) })
}

/// A host string; the host promises it ends in NUL.
pub(crate) fn cstr_in<'a>(ptr: *const c_char) -> Result<&'a CStr, Status> {
    if ptr.is_null() {
        set_error("string pointer is null");
        return Err(Status::Null);
    }
    // SAFETY: the ABI's contract; a NUL-terminated string at ptr
    Ok(unsafe { CStr::from_ptr(ptr) })
}

/// Write one value to a host slot; the host promises `out` points at one
/// writable `T`.
pub(crate) fn put<T>(out: *mut T, value: T) -> Status {
    if out.is_null() {
        set_error("output pointer is null");
        return Status::Null;
    }
    // SAFETY: the ABI's contract; one writable T at out
    unsafe { *out = value };
    Status::Ok
}

/// Load a model from the bytes (`len` of them at `bytes`) of an `.aom`
/// file, or of a `.moc3`. The bytes are read, not kept: the host may free
/// them as soon as this returns. Null when the bytes are not a model;
/// `ay2d_last_error` says why. The model is evaluated once at its
/// defaults, so drawables are ready at once.
///
/// An `.aom` carries everything: textures, motions, expressions, physics,
/// names. A `.moc3` is the rig alone; the host keeps the bundle's textures
/// (`ay2d_texture_count` still says how many pages the rig draws from) and
/// adds the bundle's JSON files with `ay2d_add_physics3`,
/// `ay2d_add_motion3` and `ay2d_add_expression3`.
#[unsafe(no_mangle)]
pub extern "C" fn ay2d_load(bytes: *const u8, len: usize) -> *mut Model {
    guard(std::ptr::null_mut(), || {
        let Ok(bytes) = input(bytes, len) else { return std::ptr::null_mut() };
        let payload = if bytes.starts_with(b"MOC3") {
            match ayatsuri_import::from_moc3(bytes) {
                Ok(model) => {
                    // the rig says how many pages it draws from
                    let pages =
                        model.art_meshes.iter().map(|a| a.texture as usize + 1).max().unwrap_or(0);
                    ayatsuri_format::payload::AomPayload {
                        model,
                        textures: vec![serde_bytes::ByteBuf::new(); pages],
                        ..Default::default()
                    }
                }
                Err(e) => {
                    set_error(format!("not a model: {e}"));
                    return std::ptr::null_mut();
                }
            }
        } else {
            match ayatsuri_format::read(bytes) {
                Ok(p) => p,
                Err(e) => {
                    set_error(format!("not a model: {e}"));
                    return std::ptr::null_mut();
                }
            }
        };
        let m = &payload.model;
        let param_ids = cstrings(m.parameters.iter().map(|p| p.id.as_str()));
        let part_ids = cstrings(m.parts.iter().map(|p| p.id.as_str()));
        let mesh_ids = cstrings(m.art_meshes.iter().map(|a| a.id.as_str()));
        let param_index: HashMap<String, usize> =
            m.parameters.iter().enumerate().map(|(i, p)| (p.id.clone(), i)).collect();
        let part_index: HashMap<String, usize> =
            m.parts.iter().enumerate().map(|(i, p)| (p.id.clone(), i)).collect();
        let player = crate::motions::Player::new(
            &payload.animations,
            &payload.compositions,
            &param_index,
            &part_index,
        );
        let exprs = crate::expressions::Expressions::new(&payload.expressions, &param_index);
        let mut puppet = Puppet::new(payload.model);
        puppet.update();
        let physics_on = puppet.has_physics();
        let mut model = Model {
            puppet,
            textures: payload.textures.into_iter().map(|b| b.into_vec()).collect(),
            param_ids,
            part_ids,
            mesh_ids,
            physics_on,
            scenes: payload.animations,
            comps: payload.compositions,
            param_index,
            part_index,
            player,
            exprs,
            order: Vec::new(),
        };
        model.refresh_order();
        Box::into_raw(Box::new(model))
    })
}

impl Model {
    /// What follows every evaluation: the draw order, and later the
    /// change flags.
    pub(crate) fn after_update(&mut self) {
        self.refresh_order();
    }

    /// The drawables' mesh indices after an update, in draw order.
    pub(crate) fn refresh_order(&mut self) {
        self.order.clear();
        self.order.extend(self.puppet.drawables().map(|d| d.mesh));
    }
}

/// Free a model, once. Null is fine. Every pointer the model handed out
/// (ids, drawables, textures) dies with it.
#[unsafe(no_mangle)]
// the one entry point that consumes its pointer; the contract is the ABI's
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn ay2d_free(m: *mut Model) {
    if !m.is_null() {
        // SAFETY: the ABI's contract; the handle came from Box::into_raw in
        // ay2d_load and is freed once
        drop(unsafe { Box::from_raw(m) });
    }
}

/// How many parameters the model has.
///
/// `m` is a live handle.
#[unsafe(no_mangle)]
pub extern "C" fn ay2d_parameter_count(m: *const Model) -> u32 {
    guard(0, || model(m).map_or(0, |m| m.param_ids.len() as u32))
}

/// How many parts the model has.
///
/// `m` is a live handle.
#[unsafe(no_mangle)]
pub extern "C" fn ay2d_part_count(m: *const Model) -> u32 {
    guard(0, || model(m).map_or(0, |m| m.part_ids.len() as u32))
}

/// How many meshes the model has; a drawable's `mesh` is an index into
/// them, and so is every mesh-wise call.
///
/// `m` is a live handle.
#[unsafe(no_mangle)]
pub extern "C" fn ay2d_mesh_count(m: *const Model) -> u32 {
    guard(0, || model(m).map_or(0, |m| m.mesh_ids.len() as u32))
}

/// How many texture pages the model carries.
///
/// `m` is a live handle.
#[unsafe(no_mangle)]
pub extern "C" fn ay2d_texture_count(m: *const Model) -> u32 {
    guard(0, || model(m).map_or(0, |m| m.textures.len() as u32))
}

/// The canvas: size in pixels and the model-unit to pixel mapping.
///
/// `m` is a live handle; `out` points at one writable canvas.
#[unsafe(no_mangle)]
pub extern "C" fn ay2d_canvas(m: *const Model, out: *mut Canvas) -> Status {
    guard(Status::Internal, || {
        let model = match model(m) {
            Ok(m) => m,
            Err(s) => return s,
        };
        let c = model.puppet.model().canvas;
        put(
            out,
            Canvas {
                width: c.width,
                height: c.height,
                pixels_per_unit: c.pixels_per_unit,
                origin_x: c.origin_x,
                origin_y: c.origin_y,
            },
        )
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
/// `m` is a live handle.
#[unsafe(no_mangle)]
pub extern "C" fn ay2d_parameter_id(m: *const Model, i: u32) -> *const c_char {
    guard(std::ptr::null(), || model(m).map_or(std::ptr::null(), |m| id_at(&m.param_ids, i)))
}

/// A part's id, as [`ay2d_parameter_id`].
///
/// `m` is a live handle.
#[unsafe(no_mangle)]
pub extern "C" fn ay2d_part_id(m: *const Model, i: u32) -> *const c_char {
    guard(std::ptr::null(), || model(m).map_or(std::ptr::null(), |m| id_at(&m.part_ids, i)))
}

/// A mesh's id, as [`ay2d_parameter_id`].
///
/// `m` is a live handle.
#[unsafe(no_mangle)]
pub extern "C" fn ay2d_mesh_id(m: *const Model, i: u32) -> *const c_char {
    guard(std::ptr::null(), || model(m).map_or(std::ptr::null(), |m| id_at(&m.mesh_ids, i)))
}

/// The index of the parameter with this id, or -1 when the model has none.
/// Resolve ids once at load; drive by index after.
///
/// `m` is a live handle; `id` is a NUL-terminated string.
#[unsafe(no_mangle)]
pub extern "C" fn ay2d_parameter_index(m: *const Model, id: *const c_char) -> i32 {
    guard(-1, || {
        let Ok(model) = model(m) else { return -1 };
        let Ok(want) = cstr_in(id) else { return -1 };
        model.param_ids.iter().position(|p| p.as_c_str() == want).map_or(-1, |i| i as i32)
    })
}
