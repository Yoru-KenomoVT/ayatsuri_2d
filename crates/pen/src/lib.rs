//! Pen tablet pressure that the window layer (winit 0.30) does not pass on.
//!
//! - Windows: nothing here. Its pens arrive as touches with a force, which
//!   the editor reads from egui's events.
//! - Linux: the tablet's own input device is read directly, in safe Rust,
//!   on Wayland and X11 alike. The compositor keeps driving the pointer (the
//!   cursor, clicks and menus stay as they are); this only adds the
//!   pressure. Only pen devices are opened (pressure plus a pen tool, picked
//!   from what the kernel lists in sysfs), never keyboards or mice.
//! - Linux on Wayland, in addition: the pen is taken through the tablet
//!   protocol on the window's own connection (`attach`), because
//!   compositors' fallback of passing a pen on as a pointer can be broken
//!   (Hyprland 0.56 drops its clicks). Its events come back as pointer
//!   events for the editor to hand to egui (`drain_pointer`).
//! - macOS: a local event monitor reads the pressure AppKit puts on tablet
//!   events.
//!
//! Attaching to the Wayland connection and adding the macOS monitor are
//! the only unsafe calls, each in its own module.
//!
//! Readings land in atomics; the editor asks for the latest each frame.

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "linux")]
mod wayland;

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Mutex, OnceLock};

/// The pen as last read.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Reading {
    /// 0 to 1.
    pub pressure: f32,
    /// The tip (or eraser) touches the tablet.
    pub down: bool,
    /// The eraser end is the one in use.
    pub eraser: bool,
}

/// What the reader found, for the settings page.
#[derive(Clone, Debug, PartialEq, Default)]
pub enum Status {
    #[default]
    NotStarted,
    /// This platform passes pressure on by itself (Windows).
    NotNeeded,
    /// No pen tablet found yet (Linux looks again every few seconds).
    NoTablet,
    /// Reading these tablets.
    Reading(Vec<String>),
    /// Pen tablets found that this user may not read (Linux: the device
    /// files, usually fixed by joining the `input` group).
    NoAccess(Vec<String>),
}

static PRESSURE: AtomicU32 = AtomicU32::new(0);
static DOWN: AtomicBool = AtomicBool::new(false);
static ERASER: AtomicBool = AtomicBool::new(false);
static SEEN: AtomicBool = AtomicBool::new(false);
static STATUS: Mutex<Status> = Mutex::new(Status::NotStarted);
static STARTED: OnceLock<()> = OnceLock::new();
type Wake = std::sync::Arc<dyn Fn() + Send + Sync>;
static WAKE: OnceLock<Wake> = OnceLock::new();
static POINTER: Mutex<Vec<PenPointer>> = Mutex::new(Vec::new());
static ATTACHED: AtomicBool = AtomicBool::new(false);

/// A pen taken through the Wayland tablet protocol, as pointer events.
/// Positions are window pixels (logical, before the app's own zoom).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PenPointer {
    Move {
        x: f32,
        y: f32,
    },
    Button {
        button: PenButton,
        pressed: bool,
    },
    /// The pen left the window (or the tablet).
    Leave,
}

/// The tip, and the barrel buttons.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PenButton {
    Primary,
    Secondary,
    Middle,
}

/// Start reading the pen, once; later calls do nothing. `wake` is called
/// from the reader whenever the pen changes, to ask for a frame. On macOS
/// call it from the main thread (the editor's frame runs there).
pub fn start(wake: impl Fn() + Send + Sync + 'static) {
    if STARTED.set(()).is_err() {
        return;
    }
    let wake: Wake = std::sync::Arc::new(wake);
    let _ = WAKE.set(wake.clone());
    #[cfg(target_os = "linux")]
    linux::start(wake);
    #[cfg(target_os = "macos")]
    {
        macos::start();
        drop(wake);
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        drop(wake);
        set_status(Status::NotNeeded);
    }
}

/// Take the pen through the window's Wayland connection when the window is
/// on Wayland (see the crate notes); does nothing elsewhere, or twice.
/// Call it once the window exists, with the window (or its frame).
pub fn attach(window: &impl raw_window_handle::HasDisplayHandle) {
    if ATTACHED.swap(true, Ordering::AcqRel) {
        return;
    }
    let Ok(handle) = window.display_handle() else { return };
    match handle.as_raw() {
        #[cfg(target_os = "linux")]
        raw_window_handle::RawDisplayHandle::Wayland(w) => {
            wayland::attach(w.display);
        }
        _ => {}
    }
}

/// Stop taking the pen through Wayland, before the window closes.
pub fn stop() {
    #[cfg(target_os = "linux")]
    wayland::stop();
}

/// The pen's pointer events since the last call (Wayland only).
pub fn drain_pointer() -> Vec<PenPointer> {
    POINTER.lock().map(|mut p| std::mem::take(&mut *p)).unwrap_or_default()
}

#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn push_pointer(events: Vec<PenPointer>) {
    if let Ok(mut p) = POINTER.lock() {
        // a frame nobody took (no frames drawn for a while) is dropped
        // rather than piling up
        if p.len() > 4096 {
            p.clear();
        }
        p.extend(events);
    }
}

#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn wake() {
    if let Some(w) = WAKE.get() {
        w();
    }
}

/// The latest reading, None until a pen has reported pressure.
pub fn reading() -> Option<Reading> {
    SEEN.load(Ordering::Acquire).then(|| Reading {
        pressure: f32::from_bits(PRESSURE.load(Ordering::Acquire)),
        down: DOWN.load(Ordering::Acquire),
        eraser: ERASER.load(Ordering::Acquire),
    })
}

/// What the reader found.
pub fn status() -> Status {
    STATUS.lock().map(|s| s.clone()).unwrap_or_default()
}

fn set_status(s: Status) {
    if let Ok(mut now) = STATUS.lock() {
        *now = s;
    }
}

// Windows passes pressure on itself and has no reader here
#[cfg_attr(not(any(target_os = "linux", target_os = "macos")), allow(dead_code))]
fn store_pressure(p: f32) {
    PRESSURE.store(p.clamp(0.0, 1.0).to_bits(), Ordering::Release);
    SEEN.store(true, Ordering::Release);
}

// Windows passes pressure on itself and has no reader here
#[cfg_attr(not(any(target_os = "linux", target_os = "macos")), allow(dead_code))]
fn store_down(down: bool) {
    DOWN.store(down, Ordering::Release);
}

// only the Linux reader can tell the eraser end apart
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn store_eraser(eraser: bool) {
    ERASER.store(eraser, Ordering::Release);
}

/// A raw axis value between its device's limits, as 0 to 1.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn normalize(v: i32, min: i32, max: i32) -> f32 {
    if max <= min {
        return 0.0;
    }
    ((v - min) as f32 / (max - min) as f32).clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values_land_between_the_device_limits() {
        assert_eq!(normalize(0, 0, 8191), 0.0);
        assert_eq!(normalize(8191, 0, 8191), 1.0);
        assert!((normalize(4096, 0, 8192) - 0.5).abs() < 1e-6);
        assert_eq!(normalize(-5, 0, 100), 0.0);
        assert_eq!(normalize(5, 10, 10), 0.0, "a broken range reads as no pressure");
    }
}
