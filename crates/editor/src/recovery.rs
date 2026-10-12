// Unsaved work kept across a crash. Autosaves and crash copies live in
// one folder; a clean exit, a save or closing the model removes that
// model's files, so whatever is still there at startup was left behind by
// a crash or a kill, and the Recover Unsaved Work dialog offers it.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

/// Files touched this recently may belong to another editor window that
/// is still running (autosave runs every two minutes), so they wait.
const LIVE: Duration = Duration::from_secs(150);

pub(crate) fn dir() -> Option<PathBuf> {
    Some(crate::logging::log_path()?.parent()?.join("recovery"))
}

/// One file the dialog can offer.
pub(crate) struct Found {
    pub(crate) path: PathBuf,
    /// The model's name, without the autosave or timestamp suffix.
    pub(crate) name: String,
    pub(crate) age: Duration,
    pub(crate) bytes: u64,
}

/// Everything recoverable, newest first. `always` is the copy a crash
/// relaunch was handed, offered however fresh it is.
pub(crate) fn scan(always: Option<&Path>) -> Vec<Found> {
    let Some(dir) = dir() else { return Vec::new() };
    let Ok(entries) = std::fs::read_dir(&dir) else { return Vec::new() };
    let now = SystemTime::now();
    let mut out: Vec<Found> = entries
        .flatten()
        .filter_map(|e| {
            let path = e.path();
            if path.extension().is_none_or(|x| x != "aom") {
                return None;
            }
            let meta = e.metadata().ok()?;
            let age =
                meta.modified().ok().and_then(|m| now.duration_since(m).ok()).unwrap_or_default();
            if age < LIVE && always != Some(path.as_path()) {
                return None;
            }
            Some(Found { name: display_name(&path), age, bytes: meta.len(), path })
        })
        .collect();
    out.sort_by_key(|f| f.age);
    out
}

/// `ariu-autosave.aom` and `ariu-1791751593.aom` both read as "ariu".
pub(crate) fn display_name(path: &Path) -> String {
    let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    match stem.rsplit_once('-') {
        Some((name, tail)) if tail == "autosave" || tail.chars().all(|c| c.is_ascii_digit()) => {
            name.to_string()
        }
        _ => stem,
    }
}

/// "3 minutes ago", "yesterday" and so on.
pub(crate) fn ago(age: Duration) -> String {
    let m = age.as_secs() / 60;
    match m {
        0 => "just now".into(),
        1 => "a minute ago".into(),
        2..=59 => format!("{m} minutes ago"),
        60..=119 => "an hour ago".into(),
        120..=1439 => format!("{} hours ago", m / 60),
        1440..=2879 => "yesterday".into(),
        _ => format!("{} days ago", m / 1440),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_drop_the_suffix() {
        assert_eq!(display_name(Path::new("/r/ariu-autosave.aom")), "ariu");
        assert_eq!(display_name(Path::new("/r/ariu-1791751593.aom")), "ariu");
        assert_eq!(display_name(Path::new("/r/my-model-autosave.aom")), "my-model");
        assert_eq!(display_name(Path::new("/r/plain.aom")), "plain");
    }

    #[test]
    fn ages_read_naturally() {
        assert_eq!(ago(Duration::from_secs(30)), "just now");
        assert_eq!(ago(Duration::from_secs(5 * 60)), "5 minutes ago");
        assert_eq!(ago(Duration::from_secs(3 * 3600)), "3 hours ago");
        assert_eq!(ago(Duration::from_secs(30 * 3600)), "yesterday");
    }
}
