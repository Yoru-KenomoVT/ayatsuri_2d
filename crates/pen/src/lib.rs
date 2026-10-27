//! Pen tablet pressure that the window layer (winit 0.30) does not pass on.
//!
//! - Windows: nothing here. Its pens arrive as touches with a force, which
//!   the editor reads from egui's events.
//! - Linux: the tablet's own input device is read directly, in safe Rust,
//!   on Wayland and X11 alike. The compositor keeps driving the pointer (the
//!   cursor, clicks and menus stay as they are); this only adds the
//!   pressure. Only pen devices are opened (pressure plus a pen tool, picked
//!   from what the kernel lists in sysfs), never keyboards or mice.
//! - macOS: a local event monitor reads the pressure AppKit puts on tablet
//!   events. It is the one place in the crate that needs unsafe.
//!
//! Readings land in atomics; the editor asks for the latest each frame.

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;

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

/// Start reading the pen, once; later calls do nothing. `wake` is called
/// from the reader whenever the pen changes, to ask for a frame. On macOS
/// call it from the main thread (the editor's frame runs there).
pub fn start(wake: impl Fn() + Send + Sync + 'static) {
    if STARTED.set(()).is_err() {
        return;
    }
    #[cfg(target_os = "linux")]
    linux::start(std::sync::Arc::new(wake));
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
