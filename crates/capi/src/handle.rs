//! The opaque model handle: load, free, and what a host asks about a model
//! before it drives it. Ids are handed out as NUL-terminated strings that
//! live as long as the model, so a binding reads them once.
//!
//! A model is four pieces, each with its own rule for threads:
//!
//! ```text
//! facts    read-only after load: ids, names, ranges, textures, the
//!          meshes' uvs and triangles                         any thread
//! inbox    parameter and part values a host sets             any thread
//! frames   two published results of update; update writes
//!          one while hosts read the other                    any thread
//! core     the puppet, motions, expressions, physics         one caller;
//!                                                            BUSY otherwise
//! ```

use std::collections::HashMap;
use std::ffi::{CStr, CString, c_char};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Mutex, MutexGuard, PoisonError, TryLockError};

use ayatsuri_core::Puppet;
use ayatsuri_format::{Animation, Composition};

use crate::error::{Status, guard, set_error};
use crate::frame::Frame;

/// A loaded model.
pub struct Model {
    pub(crate) facts: Facts,
    pub(crate) core: Mutex<Core>,
    pub(crate) inbox: Mutex<Inbox>,
    pub(crate) frames: [Mutex<Frame>; 2],
    /// Which of `frames` hosts read; update writes the other and flips it.
    pub(crate) front: AtomicUsize,
    /// Mirrors the core's rig, so asking never waits on an update.
    pub(crate) has_physics: AtomicBool,
}

/// What a mesh draws with that never changes after load.
pub(crate) struct MeshStatic {
    pub uvs: Vec<f32>,
    pub indices: Vec<u16>,
    pub masks: Vec<u32>,
    pub texture: u32,
    pub flags: u32,
    /// Where its positions sit in a frame, and how many floats.
    pub offset: usize,
    pub len: usize,
}

/// Everything fixed at load.
pub(crate) struct Facts {
    /// The texture pages as stored in the file, PNG, by texture index.
    pub textures: Vec<Vec<u8>>,
    pub param_ids: Vec<CString>,
    pub part_ids: Vec<CString>,
    pub mesh_ids: Vec<CString>,
    pub info: crate::info::Info,
    pub canvas: Canvas,
    /// Minimum, maximum and default, by parameter.
    pub ranges: Vec<(f32, f32, f32)>,
    pub repeat: Vec<bool>,
    pub part_parents: Vec<i32>,
    pub mesh_parts: Vec<i32>,
    pub meshes: Vec<MeshStatic>,
}

/// What an update drives; one caller at a time.
pub(crate) struct Core {
    pub puppet: Puppet,
    /// Physics runs on update while this is set; on by default when the
    /// model has a rig.
    pub physics_on: bool,
    /// The file's scenes and compositions, for the motion player.
    pub scenes: Vec<Animation>,
    pub comps: Vec<Composition>,
    pub param_index: HashMap<String, usize>,
    pub part_index: HashMap<String, usize>,
    pub player: crate::motions::Player,
    pub exprs: crate::expressions::Expressions,
}

/// Values a host set since the last update, newest wins.
pub(crate) struct Inbox {
    pub params: Vec<Option<f32>>,
    pub parts: Vec<Option<f32>>,
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

/// A mutex taken whatever a panic elsewhere left in it: every call runs
/// under the panic guard, so a poisoned lock only means an earlier call
/// failed with `INTERNAL`.
pub(crate) fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The core, or `Busy` when another thread is in a call that drives it.
pub(crate) fn core(model: &Model) -> Result<MutexGuard<'_, Core>, Status> {
    match model.core.try_lock() {
        Ok(g) => Ok(g),
        Err(TryLockError::Poisoned(p)) => Ok(p.into_inner()),
        Err(TryLockError::WouldBlock) => {
            set_error("busy: another thread is in a call that drives this model");
            Err(Status::Busy)
        }
    }
}

impl Model {
    /// The frame hosts read: the last one an update published.
    pub(crate) fn front(&self) -> MutexGuard<'_, Frame> {
        let [a, b] = &self.frames;
        lock(if self.front.load(Ordering::Acquire) == 0 { a } else { b })
    }
}

fn cstrings<'a>(ids: impl Iterator<Item = &'a str>) -> Vec<CString> {
    ids.map(|id| CString::new(id.replace('\0', "")).unwrap_or_default()).collect()
}

// ---- the boundary -------------------------------------------------------
//
// Every raw pointer the host hands over is turned into a reference, a slice
// or a string here and nowhere else; the other modules are safe Rust over
// these five helpers. The contracts are the ABI's (ayatsuri.h): a handle is
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
        let param_index: HashMap<String, usize> =
            m.parameters.iter().enumerate().map(|(i, p)| (p.id.clone(), i)).collect();
        let part_index: HashMap<String, usize> =
            m.parts.iter().enumerate().map(|(i, p)| (p.id.clone(), i)).collect();
        let mut offset = 0;
        let meshes: Vec<MeshStatic> = m
            .art_meshes
            .iter()
            .map(|a| {
                let len = (a.vertex_count as usize).saturating_mul(2);
                let s = MeshStatic {
                    uvs: a.uvs.clone(),
                    indices: a.indices.clone(),
                    masks: a.masks.clone(),
                    texture: a.texture,
                    flags: u32::from(a.flags),
                    offset,
                    len,
                };
                offset += len;
                s
            })
            .collect();
        let c = m.canvas;
        let facts = Facts {
            textures: Vec::new(),
            param_ids: cstrings(m.parameters.iter().map(|p| p.id.as_str())),
            part_ids: cstrings(m.parts.iter().map(|p| p.id.as_str())),
            mesh_ids: cstrings(m.art_meshes.iter().map(|a| a.id.as_str())),
            info: crate::info::Info::new(m, payload.display.as_ref(), &param_index),
            canvas: Canvas {
                width: c.width,
                height: c.height,
                pixels_per_unit: c.pixels_per_unit,
                origin_x: c.origin_x,
                origin_y: c.origin_y,
            },
            ranges: m.parameters.iter().map(|p| (p.min, p.max, p.default)).collect(),
            repeat: m.parameters.iter().map(|p| p.repeat).collect(),
            part_parents: m.parts.iter().map(|p| p.parent_part.map_or(-1, |x| x as i32)).collect(),
            mesh_parts: m
                .art_meshes
                .iter()
                .map(|a| a.parent_part.map_or(-1, |x| x as i32))
                .collect(),
            meshes,
        };
        let player = crate::motions::Player::new(
            &payload.animations,
            &payload.compositions,
            &param_index,
            &part_index,
        );
        let exprs = crate::expressions::Expressions::new(&payload.expressions, &param_index);
        let inbox =
            Inbox { params: vec![None; m.parameters.len()], parts: vec![None; m.parts.len()] };
        let mut puppet = Puppet::new(payload.model);
        puppet.update();
        let physics_on = puppet.has_physics();
        let model = Model {
            facts: Facts {
                textures: payload.textures.into_iter().map(|b| b.into_vec()).collect(),
                ..facts
            },
            has_physics: AtomicBool::new(physics_on),
            core: Mutex::new(Core {
                puppet,
                physics_on,
                scenes: payload.animations,
                comps: payload.compositions,
                param_index,
                part_index,
                player,
                exprs,
            }),
            inbox: Mutex::new(inbox),
            frames: [Mutex::new(Frame::new(offset)), Mutex::new(Frame::new(offset))],
            front: AtomicUsize::new(0),
        };
        {
            let c = lock(&model.core);
            crate::frame::publish(&model, &c);
        }
        Box::into_raw(Box::new(model))
    })
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
    guard(0, || model(m).map_or(0, |m| m.facts.param_ids.len() as u32))
}

/// How many parts the model has.
///
/// `m` is a live handle.
#[unsafe(no_mangle)]
pub extern "C" fn ay2d_part_count(m: *const Model) -> u32 {
    guard(0, || model(m).map_or(0, |m| m.facts.part_ids.len() as u32))
}

/// How many meshes the model has; a drawable's `mesh` is an index into
/// them, and so is every mesh-wise call.
///
/// `m` is a live handle.
#[unsafe(no_mangle)]
pub extern "C" fn ay2d_mesh_count(m: *const Model) -> u32 {
    guard(0, || model(m).map_or(0, |m| m.facts.mesh_ids.len() as u32))
}

/// How many texture pages the model carries.
///
/// `m` is a live handle.
#[unsafe(no_mangle)]
pub extern "C" fn ay2d_texture_count(m: *const Model) -> u32 {
    guard(0, || model(m).map_or(0, |m| m.facts.textures.len() as u32))
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
        put(out, model.facts.canvas)
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
    guard(std::ptr::null(), || model(m).map_or(std::ptr::null(), |m| id_at(&m.facts.param_ids, i)))
}

/// A part's id, as [`ay2d_parameter_id`].
///
/// `m` is a live handle.
#[unsafe(no_mangle)]
pub extern "C" fn ay2d_part_id(m: *const Model, i: u32) -> *const c_char {
    guard(std::ptr::null(), || model(m).map_or(std::ptr::null(), |m| id_at(&m.facts.part_ids, i)))
}

/// A mesh's id, as [`ay2d_parameter_id`].
///
/// `m` is a live handle.
#[unsafe(no_mangle)]
pub extern "C" fn ay2d_mesh_id(m: *const Model, i: u32) -> *const c_char {
    guard(std::ptr::null(), || model(m).map_or(std::ptr::null(), |m| id_at(&m.facts.mesh_ids, i)))
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
        model.facts.param_ids.iter().position(|p| p.as_c_str() == want).map_or(-1, |i| i as i32)
    })
}

// ---- WebAssembly only ------------------------------------------------
//
// A JavaScript host can only hand arrays to the library through the
// module's own memory, so it needs a way to get some. Not part of the C
// header: a native host uses its own allocator.

/// `len` bytes of the module's memory for a JavaScript host to fill, or
/// null. Give them back with `ay2d_wasm_free`.
/// cbindgen:ignore
#[cfg(target_family = "wasm")]
#[unsafe(no_mangle)]
pub extern "C" fn ay2d_wasm_alloc(len: usize) -> *mut u8 {
    let mut v: Vec<u8> = Vec::with_capacity(len.max(1));
    let p = v.as_mut_ptr();
    std::mem::forget(v);
    p
}

/// Give back what `ay2d_wasm_alloc` handed out, with the same `len`.
/// cbindgen:ignore
#[cfg(target_family = "wasm")]
#[unsafe(no_mangle)]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn ay2d_wasm_free(p: *mut u8, len: usize) {
    if !p.is_null() {
        // SAFETY: p came from ay2d_wasm_alloc with this capacity
        drop(unsafe { Vec::from_raw_parts(p, 0, len.max(1)) });
    }
}
