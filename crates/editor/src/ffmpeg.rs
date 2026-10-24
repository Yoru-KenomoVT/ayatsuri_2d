// ffmpeg for video export, without the user installing it or touching
// PATH: at start, when none is found, a small window says MP4, WebM and MOV
// need it and offers to fetch it (ayatsuri_update::ffmpeg does the download,
// the checksum and where it goes). The Render window offers the same.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, TryRecvError, channel};

use egui::Ui;

use crate::i18n::tr;

/// Said wherever the download is offered.
pub(crate) const NO_ADMIN: &str =
    "Downloaded for Ayatsuri2D only: no admin rights, nothing added to PATH";

/// A download running on its own thread.
pub(crate) struct Download {
    progress: Arc<ayatsuri_update::Progress>,
    bar: crate::windows::ByteBar,
    end: Receiver<Result<PathBuf, String>>,
}

impl Download {
    /// Start fetching ffmpeg into the data folder (`%LOCALAPPDATA%` on Windows).
    pub(crate) fn start() -> Result<Self, String> {
        let dir = ayatsuri_update::ffmpeg::user_dir().ok_or("there is no data folder")?;
        let progress = Arc::new(ayatsuri_update::Progress::default());
        let (tx, end) = channel();
        let counted = progress.clone();
        log::info!("ffmpeg: downloading into {}", dir.display());
        std::thread::Builder::new()
            .name("ffmpeg-get".into())
            .spawn(move || {
                let _ = tx.send(ayatsuri_update::ffmpeg::fetch(&dir, &counted));
            })
            .map_err(|e| e.to_string())?;
        Ok(Self { progress, bar: Default::default(), end })
    }

    /// The outcome once it has one.
    pub(crate) fn poll(&self) -> Option<Result<PathBuf, String>> {
        match self.end.try_recv() {
            Ok(r) => Some(r),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => Some(Err(tr("the download stopped").to_string())),
        }
    }

    /// The bar, as the render queue draws one.
    pub(crate) fn progress(&mut self, ui: &mut Ui) {
        self.bar.show(ui, self.progress.get());
    }
}

/// The start-up offer.
#[derive(Default)]
pub(crate) struct Prompt {
    /// The look for an ffmpeg, off the UI thread (it starts processes).
    check: Option<Receiver<bool>>,
    open: bool,
    get: Option<Download>,
    failed: Option<String>,
    /// Installed from here: the window says so until closed.
    ready: bool,
}

impl Prompt {
    /// Look for ffmpeg in the background, unless the user said not to ask.
    pub(crate) fn new(ask: bool) -> Self {
        if !ask || !ayatsuri_update::ffmpeg::can_fetch() {
            return Self::default();
        }
        let (tx, rx) = channel();
        let spawned = std::thread::Builder::new().name("ffmpeg-look".into()).spawn(move || {
            let _ = tx.send(ayatsuri_update::ffmpeg::program().is_some());
        });
        Self { check: spawned.ok().map(|_| rx), ..Self::default() }
    }

    /// The window, when there is no ffmpeg. `never` is set when the user
    /// says not to ask again.
    pub(crate) fn show(&mut self, ctx: &egui::Context, never: &mut bool) {
        if let Some(rx) = &self.check {
            match rx.try_recv() {
                Ok(found) => {
                    self.check = None;
                    self.open = !found;
                    if !found {
                        log::info!("ffmpeg: none found; offering the download");
                    }
                }
                Err(TryRecvError::Empty) => {
                    ctx.request_repaint_after(std::time::Duration::from_millis(250));
                }
                Err(TryRecvError::Disconnected) => self.check = None,
            }
        }
        if !self.open {
            return;
        }
        if let Some(r) = self.get.as_ref().and_then(Download::poll) {
            self.get = None;
            match r {
                Ok(path) => {
                    log::info!("ffmpeg: ready at {}", path.display());
                    self.ready = true;
                }
                Err(e) => {
                    log::warn!("ffmpeg: could not get it: {e}");
                    self.failed = Some(e);
                }
            }
        }
        let mut close = false;
        crate::windows::mark_shown(ctx);
        egui::Window::new(tr("Video export needs ffmpeg"))
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .default_width(380.0)
            .show(ctx, |ui| {
                ui.set_max_width(380.0);
                if self.ready {
                    ui.label(tr("ffmpeg is installed. MP4, WebM and MOV are ready in the Render window."));
                    ui.add_space(8.0);
                    if ui.button(tr("Close")).clicked() {
                        close = true;
                    }
                    return;
                }
                ui.label(tr(
                    "Ayatsuri2D uses ffmpeg to export MP4, WebM and MOV video, and it is not installed. GIF and PNG work without it.",
                ));
                ui.add_space(4.0);
                ui.weak(tr(
                    "Install it here: it is downloaded for Ayatsuri2D only, with no admin rights and no PATH setup.",
                ));
                if let Some(e) = &self.failed {
                    ui.add_space(4.0);
                    ui.colored_label(crate::theme::pal().rotation_red, e);
                }
                ui.add_space(8.0);
                if let Some(get) = &mut self.get {
                    get.progress(ui);
                    return;
                }
                ui.horizontal(|ui| {
                    let label = if self.failed.is_some() { "Try again" } else { "Install ffmpeg" };
                    if ui.button(tr(label)).on_hover_text(tr(crate::ffmpeg::NO_ADMIN)).clicked() {
                        self.failed = None;
                        match Download::start() {
                            Ok(get) => self.get = Some(get),
                            Err(e) => self.failed = Some(e),
                        }
                    }
                    if ui.button(tr("Not now")).clicked() {
                        log::info!("ffmpeg: download offer put off");
                        close = true;
                    }
                    if ui.button(tr("Don't ask again")).clicked() {
                        log::info!("ffmpeg: download offer turned off");
                        *never = true;
                        close = true;
                    }
                });
            });
        if close {
            self.open = false;
        }
    }
}
