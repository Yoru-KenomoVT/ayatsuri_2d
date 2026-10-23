//! The splash launcher, laid out like Adobe's: the product name large on the
//! left over a living background, a status line and a hairline progress bar
//! along the bottom. The background is the Alterion site's particle network
//! (points drifting in 3D seen through a perspective camera, near pairs
//! joined by faint lines, the pointer pushing them aside) in the primary
//! colour set in Settings.
//!
//! The editor starts straight away, in parallel. Its first drawn frame
//! writes a ready file; the splash then fills its bar, fades and closes. A
//! timeout means it can never hang around.

#![forbid(unsafe_code)]
// a windowed program on Windows: no console window behind it (release
// builds only, so debug builds keep their log output)
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::path::PathBuf;
use std::time::Instant;

use egui::{Color32, Pos2, Rect, Stroke, pos2, vec2};

const W: f32 = 640.0;
const H: f32 = 380.0;
/// Seconds the splash shows at least.
const MIN_SHOW: f32 = 7.0;
/// Seconds the splash lingers once the editor is up.
const AFTER_READY: f32 = 3.0;
/// Seconds before giving up on the ready signal and closing anyway.
const TIMEOUT: f32 = 20.0;
const FADE: f32 = 0.35;

// the site's numbers, in world units; the camera sits at z = 7
const COUNT: usize = 130;
const SPREAD: f32 = 7.0;
const LINK: f32 = 2.4;
const REPEL_RADIUS: f32 = 1.6;
const REPEL: f32 = 0.014;
const MAX_VEL: f32 = 0.04;
const CAMERA_Z: f32 = 7.0;
const FOV_Y: f32 = 55.0;

struct Particle {
    p: [f32; 3],
    drift: [f32; 3],
    push: [f32; 2],
}

struct Launcher {
    started: Instant,
    last: Instant,
    logo: Option<egui::TextureHandle>,
    /// The primary colour from Settings, for the bar and the links.
    accent: Color32,
    /// Settings can keep the splash without the particles.
    particles: bool,
    field: Vec<Particle>,
    ready_file: PathBuf,
    spawn_error: Option<String>,
    /// When the editor said it was up, in seconds since start.
    ready_at: Option<f32>,
    closing_at: Option<f32>,
}

// a small deterministic hash so the field looks random without a dep
fn fract(seed: f32) -> f32 {
    let x = (seed * 12.9898).sin() * 43758.547;
    x - x.floor()
}

/// The launcher's lines go to the end of the editor's log file, so a
/// failed start is in the same place as everything else.
struct LauncherLog(std::sync::Mutex<Option<std::fs::File>>);

impl log::Log for LauncherLog {
    fn enabled(&self, meta: &log::Metadata<'_>) -> bool {
        meta.target().starts_with("ayatsuri") && meta.level() <= log::Level::Info
    }

    fn log(&self, record: &log::Record<'_>) {
        use std::io::Write as _;
        if !self.enabled(record.metadata()) {
            return;
        }
        let secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        let (h, m, s) = ((secs / 3600) % 24, (secs / 60) % 60, secs % 60);
        let line = format!("{h:02}:{m:02}:{s:02} {:5} launcher: {}", record.level(), record.args());
        eprintln!("{line}");
        if let Ok(mut f) = self.0.lock()
            && let Some(f) = f.as_mut()
        {
            let _ = writeln!(f, "{line}");
        }
    }

    fn flush(&self) {}
}

fn init_log() {
    let file = ayatsuri_update::state_dir().and_then(|dir| {
        std::fs::create_dir_all(&dir).ok()?;
        std::fs::OpenOptions::new().create(true).append(true).open(dir.join("ayatsuri.log")).ok()
    });
    let logger = Box::leak(Box::new(LauncherLog(std::sync::Mutex::new(file))));
    if log::set_logger(logger).is_ok() {
        log::set_max_level(log::LevelFilter::Info);
    }
}

fn start_editor(open_arg: Option<String>, ready_file: Option<&PathBuf>) -> Result<(), String> {
    // prefer the sibling binary, fall back to PATH
    let sibling = std::env::current_exe()
        .ok()
        // `ayatsuri.exe` on Windows
        .and_then(|p| {
            p.parent().map(|d| d.join(format!("ayatsuri{}", std::env::consts::EXE_SUFFIX)))
        })
        .filter(|p| p.exists());
    let mut cmd = match sibling {
        Some(p) => std::process::Command::new(p),
        None => std::process::Command::new("ayatsuri"),
    };
    if let Some(arg) = open_arg {
        cmd.arg(arg);
    }
    if let Some(ready) = ready_file {
        cmd.env("AYATSURI_READY_FILE", ready);
    }
    let program = cmd.get_program().to_string_lossy().to_string();
    cmd.spawn().map_err(|e| e.to_string())?;
    log::info!("launcher {}: started {program}", env!("CARGO_PKG_VERSION"));
    Ok(())
}

impl Launcher {
    fn new(ctx: &egui::Context, open_arg: Option<String>, prefs: &serde_json::Value) -> Self {
        let ready_file =
            std::env::temp_dir().join(format!("ayatsuri-ready-{}", std::process::id()));
        let _ = std::fs::remove_file(&ready_file);
        let spawn_error = start_editor(open_arg, Some(&ready_file)).err();
        if let Some(e) = &spawn_error {
            log::error!("could not start ayatsuri: {e}");
        }
        let field = (0..COUNT)
            .map(|i| {
                let f = i as f32;
                Particle {
                    p: [
                        (fract(f + 0.13) - 0.5) * SPREAD * 2.0,
                        (fract(f + 7.71) - 0.5) * SPREAD * 2.0,
                        (fract(f + 3.37) - 0.5) * SPREAD,
                    ],
                    drift: [
                        (fract(f + 1.9) - 0.5) * 0.02,
                        (fract(f + 5.2) - 0.5) * 0.02,
                        (fract(f + 8.4) - 0.5) * 0.008,
                    ],
                    push: [0.0, 0.0],
                }
            })
            .collect();
        let particles = !pref_on(prefs, "splash_particles_off");
        // particles off in Settings: an empty field draws nothing
        let field = if particles { field } else { Vec::new() };
        Self {
            started: Instant::now(),
            last: Instant::now(),
            logo: load_logo(ctx),
            accent: primary_colour(prefs),
            particles,
            field,
            ready_file,
            spawn_error,
            ready_at: None,
            closing_at: None,
        }
    }

    /// Move the field one step; `steps` is elapsed time in 60 Hz frames, so
    /// the motion matches the site whatever the refresh rate.
    fn step(&mut self, steps: f32, mouse: Option<[f32; 2]>) {
        for pt in &mut self.field {
            let [px, py, _] = pt.p;
            if let Some([mx, my]) = mouse {
                let (dx, dy) = (px - mx, py - my);
                let d = (dx * dx + dy * dy).sqrt();
                if d < REPEL_RADIUS && d > 0.01 {
                    let force = (1.0 - d / REPEL_RADIUS) * REPEL * steps;
                    pt.push[0] += dx / d * force;
                    pt.push[1] += dy / d * force;
                }
            }
            let damp = 0.92f32.powf(steps);
            pt.push[0] *= damp;
            pt.push[1] *= damp;
            let v = [pt.drift[0] + pt.push[0], pt.drift[1] + pt.push[1], pt.drift[2]];
            let speed = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
            let k = if speed > MAX_VEL { MAX_VEL / speed } else { 1.0 } * steps;
            for (axis, (c, dv)) in pt.p.iter_mut().zip(v).enumerate() {
                *c += dv * k;
                let lim = if axis == 2 { SPREAD * 0.5 } else { SPREAD };
                if *c > lim {
                    *c = -lim;
                } else if *c < -lim {
                    *c = lim;
                }
            }
        }
    }
}

/// The editor's saved preferences, or null when missing or unreadable.
fn editor_prefs() -> serde_json::Value {
    ayatsuri_update::config_dir()
        .map(|d| d.join("preferences.json"))
        .and_then(|p| std::fs::read(p).ok())
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

fn pref_on(prefs: &serde_json::Value, key: &str) -> bool {
    prefs.get(key).and_then(|v| v.as_bool()).unwrap_or(false)
}

/// The primary colour chosen in the editor's Settings, else the logo
/// orange. Read loosely so a newer or broken preferences file never stops
/// the splash.
fn primary_colour(prefs: &serde_json::Value) -> Color32 {
    let fallback = Color32::from_rgb(0xF0, 0x8E, 0x1E);
    let channel =
        |i: usize| prefs.get("accent")?.get(i)?.as_u64().and_then(|v| u8::try_from(v).ok());
    let alpha =
        prefs.get("accent_alpha").and_then(|v| v.as_u64()).and_then(|v| u8::try_from(v).ok());
    match (channel(0), channel(1), channel(2)) {
        (Some(r), Some(g), Some(b)) => {
            Color32::from_rgba_unmultiplied(r, g, b, alpha.unwrap_or(255))
        }
        _ => fallback,
    }
}

impl eframe::App for Launcher {
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        [0.0, 0.0, 0.0, 0.0]
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let rect = ctx.content_rect();
        let t = self.started.elapsed().as_secs_f32();
        let steps = (self.last.elapsed().as_secs_f32() * 60.0).clamp(0.0, 4.0);
        self.last = Instant::now();

        // has the editor finished loading (it waits windowless for us)?
        if self.ready_at.is_none() && self.ready_file.exists() {
            self.ready_at = Some(t);
            let _ = std::fs::remove_file(&self.ready_file);
        }
        let skip = ui.input(|i| i.pointer.any_click());
        let close_at = self.ready_at.map(|r| (r + AFTER_READY).max(MIN_SHOW));
        let done = close_at.is_some_and(|c| t >= c);
        if self.closing_at.is_none() && self.spawn_error.is_none() && (done || skip || t > TIMEOUT)
        {
            self.closing_at = Some(t);
            // let the editor open its window now, it maps as we fade
            self.release_editor();
        }
        let alpha = match self.closing_at {
            Some(c) => (1.0 - (t - c) / FADE).clamp(0.0, 1.0),
            None => (t / 0.35).clamp(0.0, 1.0),
        };
        if self.closing_at.is_some() && alpha <= 0.0 {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        if self.spawn_error.is_some() && skip {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        let a = |c: Color32| c.gamma_multiply(alpha);

        let accent = self.accent;
        let p = ui.painter();

        // a flat card; the particles are the only decoration
        p.rect_filled(rect, 12.0, a(Color32::from_rgb(0x0f, 0x0e, 0x12)));
        let card = p.with_clip_rect(rect.shrink(1.0));

        // the particle field through a perspective camera
        let focal = (rect.height() * 0.5) / (FOV_Y.to_radians() * 0.5).tan();
        let c = rect.center();
        let mouse = ui.input(|i| i.pointer.hover_pos()).filter(|m| rect.contains(*m)).map(|m| {
            // the pointer on the z = 0 plane
            [(m.x - c.x) * CAMERA_Z / focal, -(m.y - c.y) * CAMERA_Z / focal]
        });
        if self.particles {
            self.step(steps, mouse);
        }
        let project = |q: &[f32; 3]| -> (Pos2, f32) {
            let depth = (CAMERA_Z - q[2]).max(0.5);
            (pos2(c.x + q[0] * focal / depth, c.y - q[1] * focal / depth), depth)
        };
        let shown: Vec<(Pos2, f32)> = self.field.iter().map(|pt| project(&pt.p)).collect();
        for (i, pa) in self.field.iter().enumerate() {
            for (j, pb) in self.field.iter().enumerate().skip(i + 1) {
                let d = ((pa.p[0] - pb.p[0]).powi(2)
                    + (pa.p[1] - pb.p[1]).powi(2)
                    + (pa.p[2] - pb.p[2]).powi(2))
                .sqrt();
                if d < LINK
                    && let (Some(&(sa, _)), Some(&(sb, _))) = (shown.get(i), shown.get(j))
                {
                    let fade = 0.20 * (1.0 - d / LINK);
                    card.line_segment([sa, sb], Stroke::new(0.8, a(accent.gamma_multiply(fade))));
                }
            }
        }
        for &(sp, depth) in &shown {
            let near = (1.0 - (depth - 3.5) / 7.0).clamp(0.0, 1.0);
            let r = 0.8 + 1.6 * near;
            card.circle_filled(
                sp,
                r,
                a(Color32::from_rgb(0xF6, 0xE7, 0xD6).gamma_multiply(0.35 + 0.55 * near)),
            );
        }

        // the product, left aligned like Adobe's splashes
        let left = rect.min.x + 40.0;
        let mut y = rect.min.y + 96.0;
        // the wordmark is the title; plain text only if it failed to load
        match &self.logo {
            Some(tex) => {
                let size = tex.size_vec2();
                let h = 68.0;
                let w = if size.y > 0.0 { h * size.x / size.y } else { h };
                card.image(
                    tex.id(),
                    Rect::from_min_size(pos2(left, y), vec2(w.min(rect.width() * 0.7), h)),
                    Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                    a(Color32::WHITE),
                );
                y += h + 14.0;
            }
            None => {
                card.text(
                    pos2(left - 2.0, y),
                    egui::Align2::LEFT_TOP,
                    "Ayatsuri2D",
                    egui::FontId::proportional(44.0),
                    a(Color32::from_gray(0xf2)),
                );
                y += 58.0;
            }
        }
        card.text(
            pos2(left + 2.0, y),
            egui::Align2::LEFT_TOP,
            "Open 2D puppet studio",
            egui::FontId::proportional(14.0),
            a(Color32::from_gray(0xa8)),
        );

        // the bar eases toward 90% while waiting, then once ready glides
        // from wherever it was to full, landing just as the splash closes
        let ease = |at: f32| 0.9 * (1.0 - (-at / 2.5).exp());
        let prog = match (self.ready_at, close_at) {
            (Some(r), Some(c)) => {
                let from = ease(r);
                let k = ((t - r) / (c - r).max(0.01)).clamp(0.0, 1.0);
                let k = k * k * (3.0 - 2.0 * k);
                from + (1.0 - from) * k
            }
            _ => ease(t),
        };
        // status, honest about what is happening
        let status = match &self.spawn_error {
            Some(e) => format!("Could not start the editor: {e}. Click to close."),
            None => match prog {
                p if p < 0.12 => "Starting up…",
                _ if self.ready_at.is_none() && t > 4.0 => "Still loading, nearly there…",
                _ if self.ready_at.is_none() => "Loading preferences and fonts…",
                p if p < 0.6 => "Preparing the workspace…",
                p if p < 0.92 => "Starting the renderer…",
                _ => "Ready",
            }
            .to_string(),
        };
        let bottom = rect.max.y - 24.0;
        card.text(
            pos2(left, bottom - 14.0),
            egui::Align2::LEFT_BOTTOM,
            status,
            egui::FontId::proportional(11.5),
            a(Color32::from_gray(0xb8)),
        );
        card.text(
            pos2(rect.max.x - 40.0, bottom - 14.0),
            egui::Align2::RIGHT_BOTTOM,
            format!("Version {}  ·  Yoru_KenomoVT", env!("CARGO_PKG_VERSION")),
            egui::FontId::proportional(10.5),
            a(Color32::from_gray(0x80)),
        );

        let track = Rect::from_min_size(pos2(left, bottom - 4.0), vec2(rect.width() - 80.0, 2.0));
        card.rect_filled(track, 1.0, a(Color32::from_gray(0x2a)));
        card.rect_filled(
            Rect::from_min_size(track.min, vec2(track.width() * prog, 2.0)),
            1.0,
            a(accent),
        );

        ctx.request_repaint();
    }
}

impl Launcher {
    fn go_file(&self) -> PathBuf {
        let mut go = self.ready_file.clone().into_os_string();
        go.push(".go");
        PathBuf::from(go)
    }

    fn release_editor(&self) {
        if let Err(e) = std::fs::write(self.go_file(), b"go") {
            // the editor stops waiting on its own after 30 seconds
            log::warn!("could not tell the editor to show its window: {e}");
        }
    }
}

impl Drop for Launcher {
    fn drop(&mut self) {
        // however we close, the editor must never be left waiting
        self.release_editor();
        let _ = std::fs::remove_file(&self.ready_file);
    }
}

fn load_logo(ctx: &egui::Context) -> Option<egui::TextureHandle> {
    let bytes = include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/logo-dark.png"));
    let img = image::load_from_memory(bytes).ok()?.to_rgba8();
    let size = [img.width() as usize, img.height() as usize];
    let color = egui::ColorImage::from_rgba_unmultiplied(size, &img.into_raw());
    Some(ctx.load_texture("logo", color, egui::TextureOptions::LINEAR))
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

/// `--get-ffmpeg [dir]`, for the installers: fetch ffmpeg for video export
/// into `dir` (the preferences folder when left out) with no window, and
/// exit 0 when it is in place. A failure is logged and is not fatal to the
/// install: the editor offers the download again from its Render window.
fn get_ffmpeg(dir: Option<String>) -> ! {
    let dir = dir.map(std::path::PathBuf::from).or_else(ayatsuri_update::ffmpeg::user_dir);
    let Some(dir) = dir else {
        log::error!("--get-ffmpeg: no folder to put ffmpeg in");
        std::process::exit(2);
    };
    match ayatsuri_update::ffmpeg::fetch(&dir, &Default::default()) {
        Ok(path) => {
            println!("ffmpeg is ready: {}", path.display());
            std::process::exit(0);
        }
        Err(e) => {
            log::error!("--get-ffmpeg: {e}");
            eprintln!("could not get ffmpeg: {e}");
            std::process::exit(1);
        }
    }
}

fn main() -> eframe::Result {
    init_log();
    let mut args = std::env::args().skip(1);
    if args.next().as_deref() == Some("--get-ffmpeg") {
        get_ffmpeg(args.next());
    }
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Ayatsuri2D")
            .with_inner_size([W, H])
            .with_min_inner_size([W, H])
            .with_max_inner_size([W, H])
            .with_resizable(false)
            .with_decorations(false)
            .with_transparent(true)
            .with_always_on_top()
            .with_icon(window_icon().unwrap_or_default()),
        ..Default::default()
    };
    // the version an update replaced waits beside us as .old until now
    ayatsuri_update::install::sweep_previous();
    let open_arg = std::env::args().nth(1);
    let prefs = editor_prefs();
    // splash turned off in Settings: just start the editor, no window
    if pref_on(&prefs, "splash_off") {
        if let Err(e) = start_editor(open_arg, None) {
            log::error!("could not start ayatsuri: {e}");
        }
        return Ok(());
    }
    // ask GitHub for a newer release while the splash shows; the editor
    // finds the answer next to its preferences. Only ever reports.
    if !pref_on(&prefs, "update_check_off") {
        let spawned = std::thread::Builder::new().name("update-check".into()).spawn(|| {
            match ayatsuri_update::fetch(env!("CARGO_PKG_VERSION")) {
                Ok(latest) => {
                    log::info!("update check: newest release is {}", latest.version);
                    ayatsuri_update::remember(&latest);
                }
                Err(e) => log::debug!("update check: {e}"),
            }
        });
        if let Err(e) = spawned {
            log::warn!("update check: could not start: {e}");
        }
    }
    eframe::run_native(
        "ayatsuri2d-launcher",
        options,
        Box::new(move |cc| Ok(Box::new(Launcher::new(&cc.egui_ctx, open_arg, &prefs)))),
    )
}
