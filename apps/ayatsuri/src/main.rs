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
}
