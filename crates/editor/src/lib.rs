//! The editor frontend: panels, viewport, parameters, timeline.
//!
//! This crate owns the egui UI and nothing else. Deformation and physics live
//! in `ayatsuri-core`; this is how content gets into the runtime, not the
//! runtime itself.

#![forbid(unsafe_code)]

pub mod dock;
mod gpu_view;
pub mod logging;
mod panels;
mod prefs;
mod session;
pub mod theme;
mod thumb;

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
