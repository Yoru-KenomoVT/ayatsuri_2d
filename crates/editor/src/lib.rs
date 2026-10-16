//! The editor frontend: panels, viewport, parameters, timeline.
//!
//! This crate owns the egui UI and nothing else. Deformation and physics live
//! in `ayatsuri-core`; this is how content gets into the runtime, not the
//! runtime itself.

#![forbid(unsafe_code)]

mod api;
mod atlas;
mod discord;
pub mod dock;
mod export;
mod ffmpeg;
mod gpu_view;
pub(crate) mod i18n;
pub mod logging;
mod mesh_clip;
mod panels;
mod point_edit;
mod prefs;
mod recovery;
mod residency;
mod session;
pub mod theme;
mod thumb;
pub(crate) mod timelapse;
mod update;
mod windows;

pub use panels::EditorUi;

/// The MSAA sample count saved in preferences, for the app shell to hand
/// the renderer at startup. 0 in the file means the default.
static GPU_OUT_OF_MEMORY: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
static GPU_ERRORS_LOGGED: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// The GPU's error handler: log (the first few), never panic, remember an
/// out-of-memory so the editor can show it. `out_of_memory` says which kind.
pub fn gpu_error(out_of_memory: bool, message: &str) {
    use std::sync::atomic::Ordering;
    if out_of_memory {
        GPU_OUT_OF_MEMORY.store(true, Ordering::Release);
    }
    if GPU_ERRORS_LOGGED.fetch_add(1, Ordering::AcqRel) < 20 {
        log::error!("GPU error (the editor keeps going): {message}");
    }
}

/// Whether the GPU has run out of memory since the last `gpu_memory_ok`.
pub fn gpu_out_of_memory() -> bool {
    GPU_OUT_OF_MEMORY.load(std::sync::atomic::Ordering::Acquire)
}

/// A new model is on the GPU: forget the last out-of-memory.
pub fn gpu_memory_ok() {
    GPU_OUT_OF_MEMORY.store(false, std::sync::atomic::Ordering::Release);
}

pub fn startup_msaa() -> u32 {
    let p = prefs::Prefs::load();
    match p.msaa {
        1 | 2 | 4 | 8 => p.msaa,
        _ => 4,
    }
}

static LAST_RECOVERY: std::sync::Mutex<Option<std::path::PathBuf>> = std::sync::Mutex::new(None);

/// The newest recovery copy written after a crash, for the relaunch.
pub(crate) fn remember_recovery(path: std::path::PathBuf) {
    if let Ok(mut g) = LAST_RECOVERY.lock() {
        *g = Some(path);
    }
}

/// The recovery copy to reopen if the app has to restart itself.
pub fn last_recovery() -> Option<std::path::PathBuf> {
    LAST_RECOVERY.lock().ok().and_then(|g| g.clone())
}

static WRITTEN: std::sync::Mutex<Vec<std::path::PathBuf>> = std::sync::Mutex::new(Vec::new());

/// Crash copies this run wrote; a clean exit removes them, since the work
/// went on and was saved or discarded after.
pub(crate) fn note_crash_copy(path: std::path::PathBuf) {
    if let Ok(mut g) = WRITTEN.lock() {
        g.push(path);
    }
}

pub(crate) fn take_crash_copies() -> Vec<std::path::PathBuf> {
    WRITTEN.lock().map(|mut g| std::mem::take(&mut *g)).unwrap_or_default()
}
