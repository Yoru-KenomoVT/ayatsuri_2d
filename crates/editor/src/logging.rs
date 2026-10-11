// The editor's logger: everything goes to an in-memory ring the Log panel
// shows, and to a file under the state dir. Dependency crates are capped at
// warn so the log stays about ayatsuri.

use std::collections::VecDeque;
use std::io::Write as _;
use std::path::PathBuf;
use std::sync::Mutex;

use log::{Level, LevelFilter, Log, Metadata, Record};

const RING_CAP: usize = 2000;
const ROTATE_BYTES: u64 = 5 * 1024 * 1024;

static BUFFER: Mutex<VecDeque<(Level, String)>> = Mutex::new(VecDeque::new());
static FILE: Mutex<Option<std::fs::File>> = Mutex::new(None);

struct EditorLogger;
static LOGGER: EditorLogger = EditorLogger;

fn our_crate(target: &str) -> bool {
    target.starts_with("ayatsuri") || target.starts_with("openmoc3")
}

fn stamp() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let (h, m, s) = ((secs / 3600) % 24, (secs / 60) % 60, secs % 60);
    format!("{h:02}:{m:02}:{s:02}")
}

impl Log for EditorLogger {
    fn enabled(&self, meta: &Metadata<'_>) -> bool {
        if our_crate(meta.target()) {
            meta.level() <= log::max_level()
        } else {
            meta.level() <= Level::Warn
        }
    }

    fn log(&self, record: &Record<'_>) {
        if !self.enabled(record.metadata()) {
            return;
        }
        let line = format!(
            "{} {:5} {}: {}",
            stamp(),
            record.level(),
            record.target().split("::").next().unwrap_or(""),
            record.args()
        );
        if let Ok(mut buf) = BUFFER.lock() {
            if buf.len() >= RING_CAP {
                buf.pop_front();
            }
            buf.push_back((record.level(), line.clone()));
        }
        if let Ok(mut file) = FILE.lock()
            && let Some(f) = file.as_mut()
        {
            let _ = writeln!(f, "{line}");
        }
    }

    fn flush(&self) {
        if let Ok(mut file) = FILE.lock()
            && let Some(f) = file.as_mut()
        {
            let _ = f.flush();
        }
    }
}

pub fn log_path() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/state")))?;
    Some(base.join("ayatsuri2d").join("ayatsuri.log"))
}

pub fn init(level: LevelFilter) {
    if let Some(path) = log_path() {
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        // one rotation keeps the file bounded without losing the last run
        if std::fs::metadata(&path).map(|m| m.len() > ROTATE_BYTES).unwrap_or(false) {
            let _ = std::fs::rename(&path, path.with_extension("log.old"));
        }
        if let Ok(f) = std::fs::OpenOptions::new().create(true).append(true).open(&path)
            && let Ok(mut slot) = FILE.lock()
        {
            *slot = Some(f);
        }
    }
    if log::set_logger(&LOGGER).is_ok() {
        log::set_max_level(level);
        log::info!("ayatsuri {} started, log level {level}", env!("CARGO_PKG_VERSION"));
    }
}

pub fn set_level(level: LevelFilter) {
    if log::max_level() != level {
        log::set_max_level(level);
        log::info!("log level set to {level}");
    }
}

/// Snapshot of the ring at or above `show`, newest last.
pub fn lines(show: Level) -> Vec<String> {
    BUFFER
        .lock()
        .map(|b| b.iter().filter(|(l, _)| *l <= show).map(|(_, s)| s.clone()).collect())
        .unwrap_or_default()
}

pub fn level_from_str(s: &str) -> LevelFilter {
    match s {
        "error" => LevelFilter::Error,
        "warn" => LevelFilter::Warn,
        "debug" => LevelFilter::Debug,
        "trace" => LevelFilter::Trace,
        _ => LevelFilter::Info,
    }
}
