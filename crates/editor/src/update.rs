// The editor's side of update checking: what the launcher last learned, a
// check run from About, and the dashboard banner's state. Only ever reports;
// downloading is the user's call, from the release page.

use std::sync::mpsc::{Receiver, channel};
use std::time::{Duration, Instant};

pub(crate) use ayatsuri_update::Latest;

/// The version updates are compared against: the real one, unless
/// AYATSURI_PRETEND_VERSION says otherwise (for trying the update banner).
pub(crate) fn current() -> String {
    std::env::var("AYATSURI_PRETEND_VERSION")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .unwrap_or_else(|| env!("CARGO_PKG_VERSION").to_string())
}

/// A saved answer older than this is checked again when the editor starts
/// on its own (the launcher checks every time it runs).
const STALE: u64 = 12 * 60 * 60;

#[derive(Default)]
pub(crate) struct UpdateState {
    /// The newest release known, from the launcher or a check here.
    pub(crate) latest: Option<Latest>,
    pending: Option<Receiver<Result<Latest, String>>>,
    /// The last check's outcome for About, None before any check here.
    pub(crate) status: Option<Result<(), String>>,
    /// The launcher may finish its check just after the editor starts, so
    /// its answer is looked for again for a little while.
    started: Option<Instant>,
    last_read: Option<Instant>,
}

impl UpdateState {
    pub(crate) fn new(check_at_start: bool) -> Self {
        let latest = ayatsuri_update::cached();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        let stale = latest.as_ref().is_none_or(|l| now.saturating_sub(l.checked_at) > STALE);
        let mut state = Self { latest, started: Some(Instant::now()), ..Self::default() };
        if check_at_start && stale {
            state.start();
        }
        state
    }

    pub(crate) fn checking(&self) -> bool {
        self.pending.is_some()
    }

    /// Ask GitHub now, off the UI thread.
    pub(crate) fn start(&mut self) {
        if self.pending.is_some() {
            return;
        }
        let (tx, rx) = channel();
        let spawned = std::thread::Builder::new().name("update-check".into()).spawn(move || {
            let answer = ayatsuri_update::fetch(&current());
            if let Ok(latest) = &answer {
                ayatsuri_update::remember(latest);
            }
            let _ = tx.send(answer);
        });
        match spawned {
            Ok(_) => self.pending = Some(rx),
            Err(e) => self.status = Some(Err(e.to_string())),
        }
    }

    /// Once a frame: pick up a finished check, or the launcher's answer.
    pub(crate) fn poll(&mut self) {
        if let Some(rx) = &self.pending
            && let Ok(answer) = rx.try_recv()
        {
            self.pending = None;
            match answer {
                Ok(latest) => {
                    log::info!("update check: newest release is {}", latest.version);
                    self.latest = Some(latest);
                    self.status = Some(Ok(()));
                }
                Err(e) => {
                    log::warn!("update check failed: {e}");
                    self.status = Some(Err(e));
                }
            }
        }
        let young = self.started.is_some_and(|t| t.elapsed() < Duration::from_secs(30));
        let due = self.last_read.is_none_or(|t| t.elapsed() > Duration::from_secs(2));
        if young && due && self.pending.is_none() {
            self.last_read = Some(Instant::now());
            if let Some(cached) = ayatsuri_update::cached()
                && self.latest.as_ref().is_none_or(|l| cached.checked_at > l.checked_at)
            {
                self.latest = Some(cached);
            }
        }
    }

    /// The newer release to tell the user about, if any.
    pub(crate) fn available(&self) -> Option<&Latest> {
        let current = current();
        self.latest.as_ref().filter(|l| l.newer_than(&current))
    }
}
