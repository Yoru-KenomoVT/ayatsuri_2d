//! Is there a newer Ayatsuri2D? One request to GitHub's public releases list
//! (betas included, drafts never), compared with the running version. It
//! only reports: nothing is downloaded or installed, and nothing about the
//! user is sent beyond the request itself.
//!
//! The launcher checks at start and leaves the answer in `update.json` next
//! to the editor's preferences; the editor reads it for its home screen and
//! can check again from About.

#![forbid(unsafe_code)]

use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

pub mod ffmpeg;
pub mod install;

/// How far a download has come, for a progress bar on another thread:
/// bytes so far, and the size once the server has said it (0 until then).
#[derive(Debug, Default)]
pub struct Progress {
    pub done: std::sync::atomic::AtomicU64,
    pub total: std::sync::atomic::AtomicU64,
}

impl Progress {
    /// Bytes so far and the size (0 while unknown).
    pub fn get(&self) -> (u64, u64) {
        use std::sync::atomic::Ordering::Relaxed;
        (self.done.load(Relaxed), self.total.load(Relaxed))
    }
}

/// Read a response body to the end, counting into `progress`, refusing
/// more than `limit` bytes.
pub(crate) fn read_counted(
    resp: &mut ureq::http::Response<ureq::Body>,
    limit: u64,
    progress: &Progress,
) -> Result<Vec<u8>, String> {
    use std::io::Read;
    use std::sync::atomic::Ordering::Relaxed;
    if let Some(len) = resp
        .headers()
        .get("content-length")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<u64>().ok())
    {
        progress.total.store(len, Relaxed);
    }
    let mut reader = resp.body_mut().with_config().limit(limit).reader();
    let mut bytes = Vec::new();
    let mut buf = vec![0u8; 256 * 1024];
    loop {
        let n = reader.read(&mut buf).map_err(|e| format!("the download did not finish: {e}"))?;
        if n == 0 {
            return Ok(bytes);
        }
        bytes.extend_from_slice(buf.get(..n).unwrap_or_default());
        progress.done.store(bytes.len() as u64, Relaxed);
    }
}

/// The releases list: newest first, pre-releases included.
pub const RELEASES_API: &str =
    "https://api.github.com/repos/Yoru-KenomoVT/ayatsuri_2d/releases?per_page=20";
/// Where people download from.
pub const RELEASES_PAGE: &str = "https://github.com/Yoru-KenomoVT/ayatsuri_2d/releases";

/// The newest release GitHub knows of.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Latest {
    /// "0.5.5", without the tag's leading v.
    pub version: String,
    /// The release's own page.
    pub url: String,
    /// When this was learned, in seconds since 1970.
    pub checked_at: u64,
    #[serde(default)]
    pub notes: String,
}

impl Latest {
    /// Whether this is newer than `current`.
    pub fn newer_than(&self, current: &str) -> bool {
        is_newer(&self.version, current)
    }
}

/// `a` > `b` for versions like 0.5.4 or 0.5.4-beta: numbers compared part by
/// part; on a tie a plain version beats one with a suffix.
pub fn is_newer(a: &str, b: &str) -> bool {
    let split = |v: &str| {
        let v = v.trim().trim_start_matches('v');
        let (num, pre) = match v.split_once('-') {
            Some((n, p)) => (n.to_string(), Some(p.to_string())),
            None => (v.to_string(), None),
        };
        let parts: Vec<u64> = num.split('.').map(|p| p.parse().unwrap_or(0)).collect();
        (parts, pre)
    };
    let ((na, pa), (nb, pb)) = (split(a), split(b));
    let len = na.len().max(nb.len());
    for i in 0..len {
        let (x, y) = (na.get(i).copied().unwrap_or(0), nb.get(i).copied().unwrap_or(0));
        if x != y {
            return x > y;
        }
    }
    matches!((pa, pb), (None, Some(_)))
}

#[derive(Deserialize)]
struct Release {
    tag_name: String,
    html_url: String,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    assets: Vec<install::Asset>,
    #[serde(default)]
    body: Option<String>,
}

/// The newest published release in a releases-list response, with the
/// files attached to it.
pub fn newest_with_assets(releases_json: &str) -> Option<(Latest, Vec<install::Asset>)> {
    let list: Vec<Release> = serde_json::from_str(releases_json).ok()?;
    let mut best: Option<Release> = None;
    for r in list.into_iter().filter(|r| !r.draft) {
        if best.as_ref().is_none_or(|b| is_newer(&r.tag_name, &b.tag_name)) {
            best = Some(r);
        }
    }
    best.map(|r| {
        let latest = Latest {
            version: r.tag_name.trim_start_matches('v').to_string(),
            url: r.html_url,
            checked_at: now(),
            notes: r.body.unwrap_or_default(),
        };
        (latest, r.assets)
    })
}

/// The newest published release in a releases-list response.
pub fn newest(releases_json: &str) -> Option<Latest> {
    newest_with_assets(releases_json).map(|(l, _)| l)
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

/// Ask GitHub. Blocks for at most ten seconds; run it off the UI thread.
/// `current` goes in the User-Agent, as GitHub asks every client to send one.
pub fn fetch(current: &str) -> Result<Latest, String> {
    fetch_with_assets(current).map(|(l, _)| l)
}

/// `fetch`, with the newest release's files for an update to choose from.
pub fn fetch_with_assets(current: &str) -> Result<(Latest, Vec<install::Asset>), String> {
    let agent: ureq::Agent =
        ureq::Agent::config_builder().timeout_global(Some(Duration::from_secs(10))).build().into();
    let body = agent
        .get(RELEASES_API)
        .header("User-Agent", &format!("Ayatsuri2D/{current}"))
        .header("Accept", "application/vnd.github+json")
        .call()
        .map_err(|e| e.to_string())?
        .body_mut()
        .read_to_string()
        .map_err(|e| e.to_string())?;
    newest_with_assets(&body).ok_or_else(|| "no published release found".to_string())
}

/// The folder the editor keeps its preferences in.
pub fn config_dir() -> Option<PathBuf> {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
        .or_else(|| std::env::var_os("APPDATA").map(PathBuf::from))
        .map(|b| b.join("ayatsuri2d"))
}

fn cache_path() -> Option<PathBuf> {
    config_dir().map(|d| d.join("update.json"))
}

/// The last answer saved, if any.
pub fn cached() -> Option<Latest> {
    let bytes = std::fs::read(cache_path()?).ok()?;
    serde_json::from_slice(&bytes).ok()
}

/// Save an answer for the editor to find.
pub fn remember(latest: &Latest) {
    let Some(path) = cache_path() else { return };
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Ok(bytes) = serde_json::to_vec_pretty(latest) {
        let _ = std::fs::write(path, bytes);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_compare_part_by_part() {
        assert!(is_newer("0.5.10", "0.5.9"));
        assert!(is_newer("v0.6.0", "0.5.99"));
        assert!(!is_newer("0.5.4", "0.5.4"));
        assert!(!is_newer("0.5.3", "0.5.4"));
        assert!(is_newer("0.5.4", "0.5.4-beta"));
        assert!(!is_newer("0.5.4-beta", "0.5.4"));
    }

    #[test]
    fn newest_skips_drafts_and_reads_prereleases() {
        let json = r#"[
            {"tag_name":"v0.6.0","html_url":"d","draft":true},
            {"tag_name":"v0.5.4","html_url":"a","draft":false,"prerelease":true},
            {"tag_name":"v0.5.10","html_url":"b","draft":false,"prerelease":true},
            {"tag_name":"v0.5.9","html_url":"c","draft":false}
        ]"#;
        let l = newest(json).unwrap();
        assert_eq!((l.version.as_str(), l.url.as_str()), ("0.5.10", "b"));
        assert!(l.newer_than("0.5.4"));
        assert!(newest("not json").is_none());
    }
}

#[cfg(test)]
mod live {
    /// The real request; run with `cargo test -p ayatsuri-update -- --ignored`.
    #[test]
    #[ignore]
    fn asks_github() {
        let latest = super::fetch("0.0.0").unwrap();
        assert!(latest.newer_than("0.0.0"), "{latest:?}");
        println!("{latest:?}");
    }
}
