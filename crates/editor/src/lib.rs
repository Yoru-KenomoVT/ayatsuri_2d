//! The editor frontend: panels, viewport, parameters, timeline.
//!
//! This crate owns the egui UI and nothing else. Deformation and physics live
//! in `ayatsuri-core`; this is how content gets into the runtime, not the
//! runtime itself.

#![forbid(unsafe_code)]

mod api;
mod discord;
pub mod dock;
mod export;
mod gpu_view;
pub mod logging;
mod panels;
mod point_edit;
mod prefs;
mod recovery;
mod session;
pub mod theme;
mod thumb;
mod update;
mod windows;

pub use panels::EditorUi;

/// The MSAA sample count saved in preferences, for the app shell to hand
/// the renderer at startup. 0 in the file means the default.
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
