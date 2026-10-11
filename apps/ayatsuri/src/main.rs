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
    eframe::run_native(
        "ayatsuri2d",
        options,
        Box::new(|cc| {
            ayatsuri_editor::theme::install(&cc.egui_ctx);
            let mut editor = EditorUi::new();
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
