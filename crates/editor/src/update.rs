// The editor's side of updates: what the launcher last learned, a check run
// from About, the dashboard banner's state, and "Update now", which fetches
// the release for this kind of install, checks it against SHA256SUMS.txt and
// puts it in place. Nothing happens without the user asking.

use std::sync::mpsc::{Receiver, channel};
use std::time::{Duration, Instant};

pub(crate) use ayatsuri_update::Latest;
use ayatsuri_update::install::{self, Install, Installed};

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
    /// How this copy was installed, worked out once when first needed.
    how: Option<Install>,
    /// A running "Update now", and how far its download has come.
    installing: Option<Receiver<Result<Installed, String>>>,
    progress: std::sync::Arc<ayatsuri_update::Progress>,
    bar: crate::windows::ByteBar,
    /// What the last "Update now" came to.
    pub(crate) installed: Option<Result<Installed, String>>,
    /// Restart was pressed: start the new version as the editor exits.
    restart: bool,
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
            Ok(_) => {
                log::info!("update check: asking GitHub for the newest release");
                self.pending = Some(rx);
            }
            Err(e) => {
                log::warn!("update check: could not start: {e}");
                self.status = Some(Err(e.to_string()));
            }
        }
    }

    /// How this copy was installed.
    pub(crate) fn how(&mut self) -> &Install {
        self.how.get_or_insert_with(install::detect)
    }

    pub(crate) fn installing(&self) -> bool {
        self.installing.is_some()
    }

    /// "Update now": fetch, check and put the newest release in place, off
    /// the UI thread.
    pub(crate) fn start_install(&mut self) {
        if self.installing.is_some() {
            return;
        }
        let how = self.how().clone();
        let (tx, rx) = channel();
        self.progress = Default::default();
        self.bar = Default::default();
        let counted = self.progress.clone();
        let spawned = std::thread::Builder::new().name("update-install".into()).spawn(move || {
            let answer = ayatsuri_update::fetch_with_assets(&current())
                .and_then(|(_, assets)| install::install_counted(&how, &assets, &counted));
            let _ = tx.send(answer);
        });
        match spawned {
            Ok(_) => {
                log::info!("update: downloading the newest release");
                self.installed = None;
                self.installing = Some(rx);
            }
            Err(e) => {
                log::warn!("update: could not start the download: {e}");
                self.installed = Some(Err(e.to_string()));
            }
        }
    }

    /// Once a frame: pick up a finished check, or the launcher's answer.
    pub(crate) fn poll(&mut self) {
        if let Some(rx) = &self.installing
            && let Ok(answer) = rx.try_recv()
        {
            self.installing = None;
            match &answer {
                Ok(done) => log::info!("update: {done:?}"),
                Err(e) => log::warn!("update failed: {e}"),
            }
            self.installed = Some(answer);
        }
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

/// The buttons for a newer release, shared by the home banner and About:
/// Update now where this copy can update itself, the release page where it
/// can't, and Restart once the new version is in place: it closes the
/// window, and `finish` starts the new version on the way out.
pub(crate) fn actions(ui: &mut egui::Ui, state: &mut UpdateState, latest: &Latest) {
    use crate::i18n::tr;
    if state.installing() {
        ui.vertical(|ui| {
            ui.set_max_width(320.0);
            ui.label(tr("Downloading the update…"));
            state.bar.show(ui, state.progress.get());
        });
        return;
    }
    match &state.installed {
        Some(Ok(_)) => {
            if ui
                .button(tr("Restart now"))
                .on_hover_text(tr("Close Ayatsuri2D and start the new version"))
                .clicked()
            {
                state.restart = true;
                ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                return;
            }
            ui.label(tr("The update is ready."));
            return;
        }
        Some(Err(e)) => {
            ui.label(
                egui::RichText::new(tr("The update failed."))
                    .color(crate::theme::pal().rotation_red),
            )
            .on_hover_text(e.as_str());
        }
        None => {}
    }
    let how = state.how().clone();
    if how.can_update() {
        if ui
            .button(tr("Update now"))
            .on_hover_text(tr("Download, check and install the new version"))
            .clicked()
        {
            state.start_install();
        }
        if ui.button(tr("Release page")).clicked() {
            ui.ctx().open_url(egui::OpenUrl::new_tab(latest.url.clone()));
        }
    } else {
        let why = match how {
            Install::Elsewhere { why } => why,
            _ => "",
        };
        if ui.button(tr("Download")).on_hover_text(why).clicked() {
            ui.ctx().open_url(egui::OpenUrl::new_tab(latest.url.clone()));
        }
    }
}

/// Called as the app closes: after Restart (which closes the window, asking
/// about unsaved work as usual), start the new version, or run its installer
/// so it can replace this one.
pub(crate) fn finish(state: &UpdateState) {
    if !state.restart {
        return;
    }
    let result = match &state.installed {
        Some(Ok(Installed::Replaced)) => install::start_launcher(),
        Some(Ok(Installed::Installer { path, sha256 })) => install::run_installer(path, sha256),
        _ => return,
    };
    if let Err(e) = result {
        log::error!("update: {e}");
    }
}

/// "What's new in X": the release's notes, folded away until asked for.
pub(crate) fn notes(ui: &mut egui::Ui, latest: &Latest) {
    if latest.notes.trim().is_empty() {
        return;
    }
    egui::CollapsingHeader::new(crate::i18n::trf(
        "What's new in {version}",
        &[("version", &latest.version)],
    ))
    .id_salt(("update-notes", latest.version.as_str()))
    .show(ui, |ui| {
        egui::ScrollArea::vertical()
            .max_height(280.0)
            .auto_shrink([false, true])
            .show(ui, |ui| markdown(ui, &latest.notes));
    });
}

/// Changelog Markdown drawn as headings, bullets and paragraphs.
pub(crate) fn markdown(ui: &mut egui::Ui, md: &str) {
    for block in blocks(md) {
        match block {
            Block::Heading(text) => {
                ui.add_space(4.0);
                ui.label(egui::RichText::new(text).strong());
            }
            Block::Bullet(text) => {
                ui.horizontal_wrapped(|ui| {
                    ui.label("•");
                    ui.label(text);
                });
            }
            Block::Text(text) => {
                ui.label(text);
            }
        }
    }
}

#[derive(Debug, PartialEq)]
enum Block {
    Heading(String),
    Bullet(String),
    Text(String),
}

/// The changelog's Markdown as headings, bullets and paragraphs: wrapped
/// lines joined, `**bold**`, `code` and [links](url) reduced to their text,
/// tables left out (the release page has them).
fn blocks(md: &str) -> Vec<Block> {
    let mut out: Vec<Block> = Vec::new();
    let mut open = false;
    for line in md.lines() {
        let t = line.trim();
        if t.is_empty() || t.starts_with('|') {
            open = false;
            continue;
        }
        if let Some(h) = t.strip_prefix("### ").or_else(|| t.strip_prefix("## ")) {
            out.push(Block::Heading(plain(h)));
            open = false;
        } else if let Some(b) = t.strip_prefix("- ").or_else(|| t.strip_prefix("* ")) {
            out.push(Block::Bullet(plain(b)));
            open = true;
        } else if open && let Some(Block::Bullet(s) | Block::Text(s)) = out.last_mut() {
            s.push(' ');
            s.push_str(&plain(t));
        } else {
            out.push(Block::Text(plain(t)));
            open = true;
        }
    }
    out
}

fn plain(s: &str) -> String {
    let s = s.replace("**", "").replace('`', "");
    let mut out = String::with_capacity(s.len());
    let mut rest = s.as_str();
    while let Some(open) = rest.find('[') {
        let (before, after) = rest.split_at(open);
        out.push_str(before);
        match after.find("](").and_then(|m| after.get(m..)?.find(')').map(|c| (m, m + c))) {
            Some((mid, close)) => {
                out.push_str(after.get(1..mid).unwrap_or(""));
                rest = after.get(close + 1..).unwrap_or("");
            }
            None => {
                out.push_str(after);
                rest = "";
            }
        }
    }
    out.push_str(rest);
    out.replace("\\*", "*")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn changelog_markdown_reads_as_blocks() {
        let md = "### Added\n\n- **Search** in the [Parts](https://x) palette,\n  and more.\n\n| a | b |\n|---|---|\nText here\nwraps on.";
        assert_eq!(
            blocks(md),
            vec![
                Block::Heading("Added".into()),
                Block::Bullet("Search in the Parts palette, and more.".into()),
                Block::Text("Text here wraps on.".into()),
            ]
        );
    }
}
