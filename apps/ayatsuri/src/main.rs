//! The desktop app: a thin eframe shell around `ayatsuri-editor`.

#![forbid(unsafe_code)]

use ayatsuri_editor::EditorUi;

struct App {
    editor: EditorUi,
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        // a bug in one frame must never take the whole editor down: the
        // panic hook has already logged it with a backtrace, the editor
        // rescues the work and shows what happened
        let frame = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            self.editor.ui(ui);
        }));
        if frame.is_err() {
            self.editor.on_crash();
            ui.ctx().request_repaint();
        }
    }
}

impl Drop for App {
    fn drop(&mut self) {
        // dropped while unwinding means eframe itself panicked past the
        // per-frame guard: keep the work for the relaunch
        if std::thread::panicking() {
            self.editor.emergency_save();
        } else {
            self.editor.clean_exit();
        }
    }
}

/// eframe went down with a panic: log it and start the editor again on the
/// recovery copy, at most twice in a row so a bad file cannot loop.
fn relaunch_after_crash() {
    let restarts: u32 =
        std::env::var("AYATSURI_RESTARTS").ok().and_then(|v| v.parse().ok()).unwrap_or(0);
    if restarts >= 2 {
        log::error!("crashed again after {restarts} restarts; not restarting");
        return;
    }
    let Ok(exe) = std::env::current_exe() else { return };
    let mut cmd = std::process::Command::new(exe);
    cmd.env("AYATSURI_RESTARTS", (restarts + 1).to_string());
    // the relaunch asks before recovering, through the startup dialog
    if let Some(p) = ayatsuri_editor::last_recovery() {
        cmd.env("AYATSURI_RECOVER", p);
    }
    match cmd.spawn() {
        Ok(_) => log::error!("the editor crashed and was restarted"),
        Err(e) => log::error!("could not restart after the crash: {e}"),
    }
}

fn main() -> eframe::Result {
    ayatsuri_editor::logging::init(log::LevelFilter::Info);
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Ayatsuri2D")
            .with_inner_size([1280.0, 800.0]),
        ..Default::default()
    };
    // `ayatsuri model.moc3` opens straight onto a model
    let open = std::env::args().nth(1).map(std::path::PathBuf::from);
    // started from the launcher: load everything windowless first, tell the
    // splash we are ready, and only open the window once it has gone, so
    // the dashboard never covers the splash
    let mut editor = Some(EditorUi::new());
    if let Some(ready) = std::env::var_os("AYATSURI_READY_FILE").map(std::path::PathBuf::from) {
        let _ = std::fs::write(&ready, b"ready");
        let mut go = ready.into_os_string();
        go.push(".go");
        let go = std::path::PathBuf::from(go);
        let started = std::time::Instant::now();
        while !go.exists() && started.elapsed() < std::time::Duration::from_secs(30) {
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        let _ = std::fs::remove_file(&go);
    }
    // every frame is already guarded; this catches eframe itself
    let run = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
        eframe::run_native(
            "ayatsuri2d",
            options,
            Box::new(move |cc| {
                ayatsuri_editor::theme::install(&cc.egui_ctx);
                let mut editor = editor.take().unwrap_or_default();
                if let Some(rs) = &cc.wgpu_render_state {
                    let renderer = ayatsuri_render::GpuRenderer::with_samples(
                        &rs.device,
                        &rs.queue,
                        rs.target_format,
                        ayatsuri_editor::startup_msaa(),
                    );
                    rs.renderer.write().callback_resources.insert(renderer);
                    editor.set_gpu_available(true);
                }
                if let Some(path) = open {
                    editor.open_on_start(path);
                }
                Ok(Box::new(App { editor }))
            }),
        )
    }));
    match run {
        Ok(result) => result,
        Err(_) => {
            relaunch_after_crash();
            Ok(())
        }
    }
}
