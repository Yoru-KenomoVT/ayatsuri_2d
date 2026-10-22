//! The desktop app: a thin eframe shell around `ayatsuri-editor`.

#![forbid(unsafe_code)]
// a windowed program on Windows: no console window behind it (release
// builds only, so debug builds keep their log output)
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use ayatsuri_editor::EditorUi;

struct App {
    editor: EditorUi,
    trace: Option<RepaintTrace>,
}

/// AYATSURI_TRACE_REPAINT=1: once a second, print to stderr how many frames
/// ran and which lines asked for them, to find what keeps an idle editor
/// drawing.
struct RepaintTrace {
    since: std::time::Instant,
    frames: u32,
    /// Time spent building each frame (the editor's own work, before the
    /// GPU draws it): the total and the slowest this second.
    build: std::time::Duration,
    slowest: std::time::Duration,
    /// Start to start of frames (the whole frame: build, egui's paint, the
    /// GPU and the wait for the screen), and eframe's CPU time per frame
    /// (build plus tessellation), for where the rest of a frame goes.
    last_start: Option<std::time::Instant>,
    gaps: std::time::Duration,
    gap_max: std::time::Duration,
    gap_n: u32,
    cpu: f32,
    causes: std::collections::BTreeMap<(&'static str, u32), u32>,
}

impl RepaintTrace {
    fn frame(
        &mut self,
        ctx: &egui::Context,
        began: std::time::Instant,
        took: std::time::Duration,
        cpu: Option<f32>,
    ) {
        if let Some(prev) = self.last_start {
            let gap = began.duration_since(prev);
            // a gap over 100 ms is the editor resting, not a frame
            if gap.as_millis() < 100 {
                self.gaps += gap;
                self.gap_max = self.gap_max.max(gap);
                self.gap_n += 1;
            }
        }
        self.last_start = Some(began);
        self.cpu += cpu.unwrap_or(0.0);
        self.frames += 1;
        self.build += took;
        self.slowest = self.slowest.max(took);
        for c in ctx.repaint_causes() {
            *self.causes.entry((c.file, c.line)).or_default() += 1;
        }
        if self.since.elapsed().as_secs_f32() >= 1.0 {
            let mut top: Vec<_> = self.causes.iter().collect();
            top.sort_by(|a, b| b.1.cmp(a.1));
            let list: Vec<String> =
                top.iter().take(6).map(|((f, l), n)| format!("{f}:{l} x{n}")).collect();
            let n = self.frames.max(1) as f32;
            let avg = self.build.as_secs_f32() * 1000.0 / n;
            let frame = self.gaps.as_secs_f32() * 1000.0 / self.gap_n.max(1) as f32;
            eprintln!(
                "repaint: {} frames/s  frame avg {frame:.1} ms (max {:.1})  build avg {avg:.1} ms, slowest {:.1} ms  cpu+paint avg {:.1} ms  {}",
                self.frames,
                self.gap_max.as_secs_f32() * 1000.0,
                self.slowest.as_secs_f32() * 1000.0,
                self.cpu * 1000.0 / n,
                list.join("  ")
            );
            let last = self.last_start;
            *self = RepaintTrace::default();
            self.last_start = last;
        }
    }
}

impl Default for RepaintTrace {
    fn default() -> Self {
        Self {
            since: std::time::Instant::now(),
            frames: 0,
            build: std::time::Duration::ZERO,
            slowest: std::time::Duration::ZERO,
            last_start: None,
            gaps: std::time::Duration::ZERO,
            gap_max: std::time::Duration::ZERO,
            gap_n: 0,
            cpu: 0.0,
            causes: Default::default(),
        }
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, eframe_frame: &mut eframe::Frame) {
        // a bug in one frame must never take the whole editor down: the
        // panic hook has already logged it with a backtrace, the editor
        // rescues the work and shows what happened
        let began = std::time::Instant::now();
        let frame = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            self.editor.ui(ui);
        }));
        if frame.is_err() {
            self.editor.on_crash();
            ui.ctx().request_repaint();
        }
        self.editor.apply_gpu_drops(eframe_frame.wgpu_render_state());
        if let Some(t) = &mut self.trace {
            t.frame(ui.ctx(), began, began.elapsed(), eframe_frame.info().cpu_usage);
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

/// The app icon for the window and taskbar, from the packaged icon set.
fn window_icon() -> Option<std::sync::Arc<egui::IconData>> {
    let bytes = include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../assets/app-icon/ayatsuri2d-256.png"
    ));
    let img = image::load_from_memory(bytes).ok()?.into_rgba8();
    let (width, height) = img.dimensions();
    Some(std::sync::Arc::new(egui::IconData { rgba: img.into_raw(), width, height }))
}

fn main() -> eframe::Result {
    ayatsuri_editor::logging::init(log::LevelFilter::Info);
    let options = eframe::NativeOptions {
        viewport: {
            let v = egui::ViewportBuilder::default()
                .with_title("Ayatsuri2D")
                .with_inner_size([1280.0, 800.0]);
            match window_icon() {
                Some(icon) => v.with_icon(icon),
                None => v,
            }
        },
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
                    rs.device.on_uncaptured_error(std::sync::Arc::new(|e: eframe::wgpu::Error| {
                        let oom = matches!(e, eframe::wgpu::Error::OutOfMemory { .. });
                        ayatsuri_editor::gpu_error(oom, &e.to_string());
                    }));
                    // a sample count the device lacks is a fatal error, so
                    // the saved setting steps down to one it supports
                    let wanted = ayatsuri_editor::startup_msaa();
                    let samples =
                        ayatsuri_render::supported_samples(&rs.adapter, &rs.device, wanted);
                    if samples != wanted {
                        log::warn!(
                            "{wanted}x anti-aliasing is not supported on this GPU; using {samples}x"
                        );
                    }
                    let renderer = ayatsuri_render::GpuRenderer::with_samples(
                        &rs.device,
                        &rs.queue,
                        rs.target_format,
                        samples,
                    );
                    rs.renderer.write().callback_resources.insert(renderer);
                    editor.set_gpu_available(true);
                }
                if let Some(path) = open {
                    editor.open_on_start(path);
                }
                Ok(Box::new(App {
                    editor,
                    trace: std::env::var_os("AYATSURI_TRACE_REPAINT")
                        .map(|_| RepaintTrace::default()),
                }))
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
