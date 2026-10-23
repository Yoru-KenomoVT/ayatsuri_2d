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
/// The most recent crash, for the app's crash dialog to pick up.
static LAST_CRASH: Mutex<Option<Crash>> = Mutex::new(None);

/// What the crash dialog shows.
#[derive(Clone)]
pub struct Crash {
    pub message: String,
    pub location: String,
}

/// The file always records at least debug, whatever the Log panel shows,
/// so a bug report never lacks the lead-up.
const FILE_FLOOR: LevelFilter = LevelFilter::Debug;

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
    Some(ayatsuri_update::state_dir()?.join("ayatsuri.log"))
}

pub fn init(level: LevelFilter) {
    // the logger is not up yet, so a failure here is told once it is
    let mut problem = None;
    if let Some(path) = log_path() {
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        // one rotation keeps the file bounded without losing the last run
        if std::fs::metadata(&path).map(|m| m.len() > ROTATE_BYTES).unwrap_or(false)
            && let Err(e) = std::fs::rename(&path, path.with_extension("log.old"))
        {
            problem = Some(format!("could not rotate {}: {e}", path.display()));
        }
        match std::fs::OpenOptions::new().create(true).append(true).open(&path) {
            Ok(f) => {
                if let Ok(mut slot) = FILE.lock() {
                    *slot = Some(f);
                }
            }
            Err(e) => {
                problem = Some(format!(
                    "could not open the log file {}: {e}; logging to the Log panel only",
                    path.display()
                ));
            }
        }
    }
    if log::set_logger(&LOGGER).is_ok() {
        log::set_max_level(level.max(FILE_FLOOR));
        log::info!(
            "ayatsuri {} started on {} {}, log level {}",
            env!("CARGO_PKG_VERSION"),
            std::env::consts::OS,
            std::env::consts::ARCH,
            level.max(FILE_FLOOR)
        );
        if let Some(problem) = problem {
            log::warn!("{problem}");
        }
    }
    // a crash should never be silent: whatever panics, from our code or a
    // dependency, lands in the log with where it happened and how we got
    // there, before the default handler prints and the process goes down
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let msg = info
            .payload()
            .downcast_ref::<&str>()
            .map(|s| (*s).to_string())
            .or_else(|| info.payload().downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "non-text panic".into());
        let at = info
            .location()
            .map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column()))
            .unwrap_or_default();
        log::error!("CRASH at {at}: {msg}\n{}", std::backtrace::Backtrace::force_capture());
        if let Ok(mut slot) = LAST_CRASH.lock() {
            *slot = Some(Crash { message: msg, location: at });
        }
        previous(info);
    }));
}

pub fn set_level(level: LevelFilter) {
    let level = level.max(FILE_FLOOR);
    if log::max_level() != level {
        log::set_max_level(level);
        log::info!("log level set to {level}");
    }
}

/// Whether a crash is waiting to be shown.
pub fn has_crash() -> bool {
    LAST_CRASH.lock().map(|s| s.is_some()).unwrap_or(false)
}

/// Hand the latest crash to whoever shows it, once.
pub fn take_crash() -> Option<Crash> {
    LAST_CRASH.lock().ok().and_then(|mut s| s.take())
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
