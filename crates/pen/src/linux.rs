// Linux: read the pen tablet's input device. sysfs says which devices are
// pens (an absolute pressure axis and a pen tool button) before anything is
// opened, so keyboards and mice are never touched. Each pen gets a reader
// thread; a scan every few seconds picks up a tablet plugged in later.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use evdev::{AbsoluteAxisCode, Device, EventSummary, KeyCode};

use crate::{Status, normalize, set_status, store_down, store_eraser, store_pressure};

type Wake = Arc<dyn Fn() + Send + Sync>;

/// ABS_PRESSURE and BTN_TOOL_PEN, as numbered by the kernel.
const ABS_PRESSURE: usize = 0x18;
const BTN_TOOL_PEN: usize = 0x140;

const RESCAN: Duration = Duration::from_secs(3);

pub(crate) fn start(wake: Wake) {
    let spawned = std::thread::Builder::new().name("pen-scan".into()).spawn(move || {
        // devices being read, so a scan opens each pen once
        let reading: Arc<Mutex<HashSet<PathBuf>>> = Arc::default();
        let mut warned: HashSet<PathBuf> = HashSet::new();
        loop {
            scan(&reading, &mut warned, &wake);
            std::thread::sleep(RESCAN);
        }
    });
    if let Err(e) = spawned {
        log::warn!("pen tablet: could not start the reader: {e}");
    }
}

fn scan(reading: &Arc<Mutex<HashSet<PathBuf>>>, warned: &mut HashSet<PathBuf>, wake: &Wake) {
    let mut names = Vec::new();
    let mut denied = Vec::new();
    for (dev, name) in pens(Path::new("/sys/class/input")) {
        let busy = reading.lock().map(|r| r.contains(&dev)).unwrap_or(true);
        if busy {
            names.push(name);
            continue;
        }
        match Device::open(&dev) {
            Ok(device) => {
                log::info!("pen tablet: reading pressure from {name} ({})", dev.display());
                if let Ok(mut r) = reading.lock() {
                    r.insert(dev.clone());
                }
                names.push(name.clone());
                let (reading, wake) = (Arc::clone(reading), Arc::clone(wake));
                let spawned =
                    std::thread::Builder::new().name("pen-read".into()).spawn(move || {
                        read(device, &wake);
                        if let Ok(mut r) = reading.lock() {
                            r.remove(&dev);
                        }
                        log::info!("pen tablet: stopped reading {}", dev.display());
                    });
                if let Err(e) = spawned {
                    log::warn!("pen tablet: could not start reading {name}: {e}");
                }
            }
            Err(e) => {
                if warned.insert(dev.clone()) {
                    log::warn!(
                        "pen tablet: {name} ({}) cannot be read: {e}; joining the input group usually fixes it",
                        dev.display()
                    );
                }
                denied.push(format!("{name} ({})", dev.display()));
            }
        }
    }
    set_status(if !names.is_empty() {
        Status::Reading(names)
    } else if !denied.is_empty() {
        Status::NoAccess(denied)
    } else {
        Status::NoTablet
    });
}

/// Read one pen until it goes away.
fn read(mut device: Device, wake: &Wake) {
    let (min, max) = device
        .get_absinfo()
        .ok()
        .and_then(|mut axes| axes.find(|(c, _)| *c == AbsoluteAxisCode::ABS_PRESSURE))
        .map(|(_, info)| (info.minimum(), info.maximum()))
        .unwrap_or((0, 0));
    loop {
        let events = match device.fetch_events() {
            Ok(events) => events,
            Err(_) => return,
        };
        let mut changed = false;
        for ev in events {
            match ev.destructure() {
                EventSummary::AbsoluteAxis(_, AbsoluteAxisCode::ABS_PRESSURE, v) => {
                    store_pressure(normalize(v, min, max));
                    changed = true;
                }
                EventSummary::Key(_, KeyCode::BTN_TOUCH, v) => {
                    store_down(v != 0);
                    changed = true;
                }
                EventSummary::Key(_, KeyCode::BTN_TOOL_RUBBER, v) => store_eraser(v != 0),
                _ => {}
            }
        }
        if changed {
            wake();
        }
    }
}

/// The pen devices under `sysfs` (normally /sys/class/input): their
/// /dev/input path and name.
fn pens(sysfs: &Path) -> Vec<(PathBuf, String)> {
    let Ok(entries) = std::fs::read_dir(sysfs) else { return Vec::new() };
    let mut out: Vec<(PathBuf, String)> = entries
        .flatten()
        .filter_map(|e| {
            let node = e.file_name().to_string_lossy().into_owned();
            if !node.starts_with("event") {
                return None;
            }
            let caps = e.path().join("device/capabilities");
            let abs = std::fs::read_to_string(caps.join("abs")).ok()?;
            let key = std::fs::read_to_string(caps.join("key")).ok()?;
            if !has_bit(&abs, ABS_PRESSURE) || !has_bit(&key, BTN_TOOL_PEN) {
                return None;
            }
            let name = std::fs::read_to_string(e.path().join("device/name"))
                .map(|n| n.trim().to_string())
                .unwrap_or_else(|_| node.clone());
            Some((Path::new("/dev/input").join(&node), name))
        })
        .collect();
    out.sort();
    out
}

/// Whether a sysfs capability mask has `bit` set: hex words of the
/// kernel's long, most significant first.
fn has_bit(mask: &str, bit: usize) -> bool {
    let word_bits = usize::BITS as usize;
    let words: Vec<&str> = mask.split_whitespace().collect();
    let Some(at) = words.len().checked_sub(1 + bit / word_bits) else { return false };
    words
        .get(at)
        .and_then(|w| u64::from_str_radix(w, 16).ok())
        .is_some_and(|w| (w >> (bit % word_bits)) & 1 == 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    // masks as an XP-Pen tablet, its keyboard and a touchpad list them
    const PEN_ABS: &str = "d000003";
    const PEN_KEY: &str = "c03 0 0 0 0 0";
    const KEYBOARD_KEY: &str = "1000000000007 ff9f207ac14057ff febeffdfffefffff ffffffffffff";
    const TOUCHPAD_ABS: &str = "260800000000003";
    const TOUCHPAD_KEY: &str = "e520 10000 0 0 0 0";

    #[test]
    fn capability_masks_read_bit_by_bit() {
        assert!(has_bit(PEN_ABS, ABS_PRESSURE));
        assert!(has_bit(PEN_ABS, 0), "ABS_X");
        assert!(!has_bit(PEN_ABS, 2));
        assert!(has_bit(PEN_KEY, BTN_TOOL_PEN));
        assert!(has_bit(PEN_KEY, 0x14a), "BTN_TOUCH");
        assert!(!has_bit(KEYBOARD_KEY, BTN_TOOL_PEN), "a keyboard is never a pen");
        assert!(!has_bit(TOUCHPAD_KEY, BTN_TOOL_PEN), "a touchpad is never a pen");
        assert!(!has_bit("", 3));
        assert!(!has_bit("zz", 3));
        let _ = TOUCHPAD_ABS;
    }

    #[test]
    fn only_pens_are_listed() {
        let root = std::env::temp_dir().join(format!("ayatsuri-pen-test-{}", std::process::id()));
        let dev = |node: &str, name: &str, abs: &str, key: &str| {
            let caps = root.join(node).join("device/capabilities");
            std::fs::create_dir_all(&caps).unwrap();
            std::fs::write(caps.join("abs"), abs).unwrap();
            std::fs::write(caps.join("key"), key).unwrap();
            std::fs::write(root.join(node).join("device/name"), format!("{name}\n")).unwrap();
        };
        dev("event9", "11 inch PenTablet", PEN_ABS, PEN_KEY);
        dev("event8", "11 inch PenTablet Keyboard", "0", KEYBOARD_KEY);
        dev("event3", "Touchpad", TOUCHPAD_ABS, TOUCHPAD_KEY);
        std::fs::create_dir_all(root.join("input4")).unwrap();
        let found = pens(&root);
        std::fs::remove_dir_all(&root).unwrap();
        assert_eq!(
            found,
            vec![(PathBuf::from("/dev/input/event9"), "11 inch PenTablet".to_string())]
        );
    }
}
