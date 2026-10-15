// File > Export Animation and Export Timelapse: the Render window. Presets
// down the left, the settings for the one picked in the middle, a preview
// of the frame and the render queue on the right, the way video editors
// lay their delivery page out. Frames are drawn offscreen on a worker
// thread with the same renderer the thumbnails use. GIF and PNG sequences
// are written here; MP4, WebM and MOV go to ffmpeg as raw frames over a
// pipe, so nothing patent-encumbered ships and no temporary frames land
// on disk.

use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender, channel};

use ayatsuri_format::model::Model;
use ayatsuri_format::{Animation, CurveTarget, Expression, ExpressionBlend};
use egui::Ui;

use crate::i18n::{tr, trf};
use crate::panels::EditorUi;

/// What the frames come from.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Source {
    Animation,
    Timelapse,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Preset {
    Gif,
    PngSeq,
    Mp4,
    Webm,
    Mov,
}

impl Preset {
    const ALL: [Preset; 5] = [Preset::Gif, Preset::PngSeq, Preset::Mp4, Preset::Webm, Preset::Mov];

    fn label(self) -> &'static str {
        match self {
            Preset::Gif => "GIF",
            Preset::PngSeq => "PNG sequence",
            Preset::Mp4 => "MP4 (H.264)",
            Preset::Webm => "WebM (VP9)",
            Preset::Mov => "MOV (lossless)",
        }
    }

    fn blurb(self) -> &'static str {
        match self {
            Preset::Gif => "Loops anywhere. Up to 256 colours, no half transparency.",
            Preset::PngSeq => "One PNG per frame with full transparency, for compositing.",
            Preset::Mp4 => "Plays everywhere. No transparency.",
            Preset::Webm => "Small, with transparency where the player supports it.",
            Preset::Mov => "Every pixel kept, with transparency; large files.",
        }
    }

    fn ffmpeg(self) -> bool {
        matches!(self, Preset::Mp4 | Preset::Webm | Preset::Mov)
    }

    fn alpha(self) -> bool {
        matches!(self, Preset::PngSeq | Preset::Webm | Preset::Mov)
    }

    fn ext(self) -> &'static str {
        match self {
            Preset::Gif => "gif",
            Preset::PngSeq => "",
            Preset::Mp4 => "mp4",
            Preset::Webm => "webm",
            Preset::Mov => "mov",
        }
    }
}

/// Preview frames are drawn this wide; the panel scales them to fit.
const PREVIEW_W: usize = 560;

enum Progress {
    Frames(usize, usize),
    Done(Result<String, String>),
}

/// Everything a worker needs, cloned out of the session so the editor
/// stays free while it renders.
struct Spec {
    source: Source,
    preset: Preset,
    fps: u32,
    width: u32,
    transparent: bool,
    work_area: bool,
    hold: f32,
    physics: bool,
    folder: PathBuf,
    name: String,
    model: Model,
    pngs: Vec<Vec<u8>>,
    anim: Option<Animation>,
    expression: Option<Expression>,
    timelapse: Option<PathBuf>,
    /// The ffmpeg to run for MP4, WebM and MOV.
    ffmpeg: PathBuf,
}

enum State {
    Queued(Box<Spec>),
    Running,
    Done(String),
    Failed(String),
}

struct Job {
    label: String,
    done: usize,
    total: usize,
    state: State,
}

pub struct RenderWin {
    pub source: Source,
    preset: Preset,
    scene: usize,
    expression: Option<usize>,
    fps: u32,
    width: u32,
    work_area: bool,
    transparent: bool,
    hold: f32,
    folder: Option<PathBuf>,
    /// The ffmpeg that works here, if any, and a download of one under way.
    ffmpeg: Option<PathBuf>,
    ffmpeg_get: Option<crate::ffmpeg::Download>,
    ffmpeg_failed: Option<String>,
    preview: Option<egui::TextureHandle>,
    /// The preview player: a puppet posed per frame for a scene, the
    /// timelapse steps for a timelapse.
    playing: bool,
    time: f32,
    step: usize,
    last_tick: Option<std::time::Instant>,
    shown: Option<(Source, usize, u32)>,
    puppet: Option<ayatsuri_core::Puppet>,
    player: Option<crate::timelapse::Player>,
    queue: Vec<Job>,
    rx: Option<Receiver<Progress>>,
}

impl RenderWin {
    pub fn set_folder(&mut self, dir: PathBuf) {
        self.folder = Some(dir);
    }

    pub fn new(source: Source, scene_fps: f32, canvas_width: f32, folder: Option<PathBuf>) -> Self {
        let ffmpeg = ayatsuri_update::ffmpeg::program();
        RenderWin {
            source,
            preset: if ffmpeg.is_some() { Preset::Mp4 } else { Preset::Gif },
            scene: 0,
            expression: None,
            fps: if source == Source::Timelapse {
                30
            } else {
                scene_fps.round().clamp(1.0, 240.0) as u32
            },
            width: (canvas_width.max(1.0) as u32).min(1920),
            work_area: true,
            transparent: false,
            hold: 2.0,
            folder,
            ffmpeg,
            ffmpeg_get: None,
            ffmpeg_failed: None,
            preview: None,
            playing: false,
            time: 0.0,
            step: 0,
            last_tick: None,
            shown: None,
            puppet: None,
            player: None,
            queue: Vec::new(),
            rx: None,
        }
    }

    /// Draw the preview frame for the current time or step.
    fn refresh_preview(&mut self, ctx: &egui::Context, s: &crate::session::Session) {
        let frame = match self.source {
            Source::Animation => {
                let puppet = self
                    .puppet
                    .get_or_insert_with(|| ayatsuri_core::Puppet::new(s.puppet.model().clone()));
                if let Some(anim) = s.animations.get(self.scene) {
                    pose(
                        puppet,
                        anim,
                        self.time,
                        self.expression.and_then(|i| s.expressions.get(i)),
                    );
                }
                puppet.update();
                crate::thumb::render(puppet, &s.preview_tex, PREVIEW_W)
            }
            Source::Timelapse => {
                if self.player.is_none()
                    && let Some(p) = &s.aom_path
                {
                    self.player =
                        crate::timelapse::Player::open(&crate::timelapse::Recorder::sidecar(p))
                            .ok();
                }
                let Some(player) = &mut self.player else { return };
                let Some(model) = player.model(self.step) else { return };
                let mut puppet = ayatsuri_core::Puppet::new(model);
                puppet.update();
                crate::thumb::render(&puppet, &s.preview_tex, PREVIEW_W)
            }
        };
        let (w, h, px) = frame;
        let img = egui::ColorImage::from_rgba_unmultiplied([w, h], &px);
        match &mut self.preview {
            Some(t) => t.set(img, egui::TextureOptions::LINEAR),
            None => {
                self.preview =
                    Some(ctx.load_texture("render-preview", img, egui::TextureOptions::LINEAR))
            }
        }
    }
}

/// Decode the texture pages at the detail the export width needs.
fn textures(pngs: &[Vec<u8>], canvas_w: f32, width: u32) -> Vec<ayatsuri_render::TexData> {
    let k = (width as f32 / canvas_w.max(1.0) * 1.25).min(1.0);
    pngs.iter()
        .map(|png| match image::load_from_memory(png) {
            Ok(img) => {
                let img = if k < 0.9 {
                    let (w, h) = (
                        ((img.width() as f32 * k) as u32).max(1),
                        ((img.height() as f32 * k) as u32).max(1),
                    );
                    img.resize_exact(w, h, image::imageops::FilterType::Triangle)
                } else {
                    img
                };
                let img = img.into_rgba8();
                ayatsuri_render::TexData {
                    width: img.width(),
                    height: img.height(),
                    rgba: img.into_raw(),
                }
            }
            Err(_) => ayatsuri_render::TexData { width: 1, height: 1, rgba: vec![0; 4] },
        })
        .collect()
}

/// Onto white, for formats without transparency.
fn flatten(px: &mut [u8]) {
    for p in px.as_chunks_mut::<4>().0 {
        let a = p[3] as u32;
        for c in p.iter_mut().take(3) {
            *c = ((*c as u32 * a + 255 * (255 - a)) / 255) as u8;
        }
        p[3] = 255;
    }
}

/// Where the frames go: a folder of PNGs, a GIF, or ffmpeg's stdin.
enum Sink {
    Png(PathBuf, usize),
    Gif(image::codecs::gif::GifEncoder<std::io::BufWriter<std::fs::File>>, image::Delay),
    Pipe(std::process::Child, std::io::BufWriter<std::process::ChildStdin>),
}

/// Under the formats when there is no ffmpeg: a button that fetches one
/// into the preferences folder (no admin, no PATH), with its progress.
fn ffmpeg_offer(ui: &mut Ui, win: &mut RenderWin) {
    if let Some(r) = win.ffmpeg_get.as_ref().and_then(crate::ffmpeg::Download::poll) {
        win.ffmpeg_get = None;
        match r {
            Ok(path) => {
                log::info!("render: ffmpeg is ready at {}", path.display());
                win.ffmpeg = Some(path);
                return;
            }
            Err(e) => {
                log::warn!("render: could not get ffmpeg: {e}");
                win.ffmpeg_failed = Some(e);
            }
        }
    }
    if let Some(get) = &mut win.ffmpeg_get {
        get.progress(ui);
        return;
    }
    ui.weak(tr("MP4, WebM and MOV need ffmpeg. GIF and PNG work without it."));
    if let Some(e) = &win.ffmpeg_failed {
        ui.colored_label(crate::theme::pal().rotation_red, e);
    }
    if !ayatsuri_update::ffmpeg::can_fetch() {
        ui.weak(tr("Install ffmpeg and put it on PATH, then open this window again."));
        return;
    }
    if ui.button(tr("Install ffmpeg")).on_hover_text(tr(crate::ffmpeg::NO_ADMIN)).clicked() {
        win.ffmpeg_failed = None;
        match crate::ffmpeg::Download::start() {
            Ok(get) => win.ffmpeg_get = Some(get),
            Err(e) => win.ffmpeg_failed = Some(e),
        }
    }
}

impl Sink {
    fn open(spec: &Spec, w: u32, h: u32) -> Result<(Self, PathBuf), String> {
        let out = spec.folder.join(format!("{}.{}", spec.name, spec.preset.ext()));
        match spec.preset {
            Preset::PngSeq => {
                let dir = spec.folder.join(&spec.name);
                std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
                Ok((Sink::Png(dir.clone(), 0), dir))
            }
            Preset::Gif => {
                let file = std::fs::File::create(&out).map_err(|e| e.to_string())?;
                let mut gif = image::codecs::gif::GifEncoder::new_with_speed(
                    std::io::BufWriter::new(file),
                    10,
                );
                gif.set_repeat(image::codecs::gif::Repeat::Infinite).map_err(|e| e.to_string())?;
                Ok((Sink::Gif(gif, image::Delay::from_numer_denom_ms(1000, spec.fps.max(1))), out))
            }
            Preset::Mp4 | Preset::Webm | Preset::Mov => {
                let mut cmd = std::process::Command::new(&spec.ffmpeg);
                cmd.args(["-y", "-f", "rawvideo", "-pix_fmt", "rgba"])
                    .arg("-s")
                    .arg(format!("{w}x{h}"))
                    .arg("-r")
                    .arg(spec.fps.to_string())
                    .args(["-i", "-"]);
                match spec.preset {
                    Preset::Mp4 => {
                        cmd.args([
                            "-vf",
                            "pad=ceil(iw/2)*2:ceil(ih/2)*2",
                            "-c:v",
                            "libx264",
                            "-pix_fmt",
                            "yuv420p",
                            "-crf",
                            "18",
                        ]);
                    }
                    Preset::Webm => {
                        let pix = if spec.transparent { "yuva420p" } else { "yuv420p" };
                        cmd.args([
                            "-c:v",
                            "libvpx-vp9",
                            "-pix_fmt",
                            pix,
                            "-b:v",
                            "0",
                            "-crf",
                            "30",
                        ]);
                    }
                    _ => {
                        cmd.args(["-c:v", "png"]);
                    }
                }
                cmd.arg(&out)
                    .stdin(std::process::Stdio::piped())
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null());
                let mut child = cmd.spawn().map_err(|e| format!("ffmpeg: {e}"))?;
                let stdin = child.stdin.take().ok_or("ffmpeg: no stdin")?;
                Ok((Sink::Pipe(child, std::io::BufWriter::with_capacity(1 << 20, stdin)), out))
            }
        }
    }

    fn frame(&mut self, w: u32, h: u32, px: Vec<u8>) -> Result<(), String> {
        match self {
            Sink::Png(dir, n) => {
                *n += 1;
                let img = image::RgbaImage::from_raw(w, h, px).ok_or("bad frame")?;
                img.save(dir.join(format!("frame_{n:05}.png"))).map_err(|e| e.to_string())
            }
            Sink::Gif(gif, delay) => {
                let img = image::RgbaImage::from_raw(w, h, px).ok_or("bad frame")?;
                gif.encode_frame(image::Frame::from_parts(img, 0, 0, *delay))
                    .map_err(|e| e.to_string())
            }
            Sink::Pipe(_, stdin) => {
                use std::io::Write;
                stdin.write_all(&px).map_err(|e| format!("ffmpeg: {e}"))
            }
        }
    }

    fn finish(self) -> Result<(), String> {
        match self {
            Sink::Png(..) => Ok(()),
            Sink::Gif(gif, _) => {
                drop(gif);
                Ok(())
            }
            Sink::Pipe(mut child, stdin) => {
                drop(stdin);
                let st = child.wait().map_err(|e| format!("ffmpeg: {e}"))?;
                if st.success() { Ok(()) } else { Err("ffmpeg could not encode the video".into()) }
            }
        }
    }
}

/// Apply a scene at time `t`, then the expression, onto the puppet.
fn pose(
    puppet: &mut ayatsuri_core::Puppet,
    anim: &Animation,
    t: f32,
    expression: Option<&Expression>,
) {
    for c in &anim.curves {
        let Some(v) = ayatsuri_core::anim::sample(anim, c, t) else { continue };
        match c.target {
            CurveTarget::Parameter => {
                if let Some(pi) = puppet.model().parameters.iter().position(|p| p.id == c.id) {
                    puppet.set_value(pi, v);
                }
            }
            CurveTarget::PartOpacity => {
                if let Some(i) = puppet.model().parts.iter().position(|p| p.id == c.id) {
                    puppet.set_part_opacity(i, v);
                }
            }
            CurveTarget::Model => {}
        }
    }
    if let Some(e) = expression {
        for ep in &e.params {
            let Some(pi) = puppet.model().parameters.iter().position(|p| p.id == ep.id) else {
                continue;
            };
            let now = puppet.value(pi);
            puppet.set_value(
                pi,
                match ep.blend {
                    ExpressionBlend::Add => now + ep.value,
                    ExpressionBlend::Multiply => now * ep.value,
                    ExpressionBlend::Overwrite => ep.value,
                },
            );
        }
    }
}

fn run(spec: Spec, tx: &Sender<Progress>) -> Result<String, String> {
    let canvas_w = spec.model.canvas.width.max(1.0);
    let tex = textures(&spec.pngs, canvas_w, spec.width);
    let width = spec.width.clamp(16, 8192) as usize;
    let mut sink: Option<Sink> = None;
    let mut out = PathBuf::new();
    let mut written = 0usize;
    let mut push = |w: usize, h: usize, mut px: Vec<u8>, total: usize| -> Result<(), String> {
        if !spec.transparent || !spec.preset.alpha() {
            flatten(&mut px);
        }
        if sink.is_none() {
            let (s, o) = Sink::open(&spec, w as u32, h as u32)?;
            sink = Some(s);
            out = o;
        }
        if let Some(s) = &mut sink {
            s.frame(w as u32, h as u32, px)?;
        }
        written += 1;
        let _ = tx.send(Progress::Frames(written, total));
        Ok(())
    };
    match spec.source {
        Source::Animation => {
            let anim = spec.anim.as_ref().ok_or("no scene to render")?;
            let (start, end) = match (spec.work_area, anim.work_area) {
                (true, Some((a, b))) if b > a => (a, b),
                _ => (0.0, anim.duration),
            };
            let fps = spec.fps.max(1) as f32;
            let frames = ((end - start) * fps).round().max(0.0) as usize + 1;
            let mut puppet = ayatsuri_core::Puppet::new(spec.model.clone());
            let mut physics =
                if spec.physics { ayatsuri_core::physics::Physics::new(&spec.model) } else { None };
            let meta: Vec<(f32, f32, f32)> =
                spec.model.parameters.iter().map(|p| (p.min, p.max, p.default)).collect();
            for i in 0..frames {
                let t = start + i as f32 / fps;
                pose(&mut puppet, anim, t, spec.expression.as_ref());
                if let Some(ph) = &mut physics {
                    let mut values: Vec<f32> = (0..meta.len()).map(|k| puppet.value(k)).collect();
                    ph.step(1.0 / fps, &mut values, |k| {
                        meta.get(k).copied().unwrap_or((0.0, 1.0, 0.0))
                    });
                    for (k, v) in values.iter().enumerate() {
                        puppet.set_value(k, *v);
                    }
                }
                puppet.update();
                let (w, h, px) = crate::thumb::render(&puppet, &tex, width);
                push(w, h, px, frames)?;
            }
        }
        Source::Timelapse => {
            let path = spec.timelapse.as_ref().ok_or("this project has no timelapse yet")?;
            let mut player = crate::timelapse::Player::open(path)?;
            let n = player.steps.len();
            if n == 0 {
                return Err("the timelapse is empty".into());
            }
            let stride = n.div_ceil(600).max(1);
            let mut picks: Vec<usize> =
                (0..n).step_by(stride).chain(std::iter::once(n - 1)).collect();
            picks.dedup();
            let hold = (spec.hold.max(0.0) * spec.fps.max(1) as f32).round() as usize;
            let total = picks.len() + hold;
            let mut last = None;
            for &i in &picks {
                let model = player.model(i).ok_or("a step could not be read")?;
                let mut puppet = ayatsuri_core::Puppet::new(model);
                puppet.update();
                let (w, h, px) = crate::thumb::render(&puppet, &tex, width);
                last = Some((w, h, px.clone()));
                push(w, h, px, total)?;
            }
            if let Some((w, h, px)) = last {
                for _ in 0..hold {
                    push(w, h, px.clone(), total)?;
                }
            }
        }
    }
    match sink {
        Some(s) => s.finish()?,
        None => return Err("nothing to render".into()),
    }
    Ok(trf("{frames} frames to {path}", &[("frames", &written), ("path", &out.display())]))
}

impl EditorUi {
    /// Open the Render window on a source, keeping the last settings.
    pub(crate) fn open_render(&mut self, ctx: &egui::Context, source: Source) {
        let Some(s) = &self.session else { return };
        let fps = s.current_anim.and_then(|i| s.animations.get(i)).map_or(30.0, |a| a.fps);
        let folder = s.aom_path.as_ref().and_then(|p| p.parent().map(|d| d.to_path_buf()));
        let mut win = self
            .render_win
            .take()
            .unwrap_or_else(|| RenderWin::new(source, fps, s.puppet.model().canvas.width, folder));
        win.source = source;
        if let Some(i) = s.current_anim {
            win.scene = i;
        }
        let _ = ctx;
        win.puppet = None;
        win.player = None;
        win.shown = None;
        win.playing = false;
        self.render_win = Some(win);
    }

    pub(crate) fn render_window(&mut self, ui: &mut Ui) {
        let Some(win) = &mut self.render_win else { return };
        // progress from the running job, and the next one once it is done
        if let Some(rx) = &win.rx {
            let mut finished = false;
            while let Ok(p) = rx.try_recv() {
                match p {
                    Progress::Frames(a, b) => {
                        if let Some(j) =
                            win.queue.iter_mut().find(|j| matches!(j.state, State::Running))
                        {
                            j.done = a;
                            j.total = b;
                        }
                    }
                    Progress::Done(r) => {
                        if let Some(j) =
                            win.queue.iter_mut().find(|j| matches!(j.state, State::Running))
                        {
                            j.state = match r {
                                Ok(m) => State::Done(m),
                                Err(e) => State::Failed(e),
                            };
                        }
                        finished = true;
                    }
                }
            }
            if finished {
                win.rx = None;
            }
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(100));
        }
        if win.rx.is_none()
            && let Some(j) = win.queue.iter_mut().find(|j| matches!(j.state, State::Queued(_)))
            && let State::Queued(spec) = std::mem::replace(&mut j.state, State::Running)
        {
            let (tx, rx) = channel();
            win.rx = Some(rx);
            let spawned = std::thread::Builder::new().name("render".into()).spawn(move || {
                let r = run(*spec, &tx);
                let _ = tx.send(Progress::Done(r));
            });
            if spawned.is_err() {
                j.state = State::Failed("could not start the render thread".into());
                win.rx = None;
            }
        }

        let Some(s) = &self.session else { return };
        // advance the preview while it plays, at the scene's own clock
        let now = std::time::Instant::now();
        let dt = win.last_tick.map_or(0.0, |t| now.duration_since(t).as_secs_f32()).min(0.1);
        win.last_tick = Some(now);
        if win.playing {
            match win.source {
                Source::Animation => {
                    if let Some(a) = s.animations.get(win.scene) {
                        win.time += dt;
                        if win.time > a.duration {
                            win.time = if a.duration > 0.0 { win.time % a.duration } else { 0.0 };
                        }
                    }
                }
                Source::Timelapse => {
                    let n = win.player.as_ref().map_or(0, |p| p.steps.len());
                    let per = (n as f32 / 600.0).max(1.0);
                    win.step += (dt * win.fps as f32 * per).max(1.0) as usize;
                    if n > 0 && win.step >= n {
                        win.step = 0;
                    }
                }
            }
            win.shown = None;
            ui.ctx().request_repaint();
        }
        let key = (win.source, win.scene, ((win.time * 1000.0) as u32) ^ ((win.step as u32) << 8));
        if win.shown != Some(key) {
            win.refresh_preview(ui.ctx(), s);
            win.shown = Some(key);
        }
        let mut open = true;
        let mut enqueue = false;
        let mut pick_folder = false;
        let has_timelapse =
            s.aom_path.as_ref().is_some_and(|p| crate::timelapse::Recorder::sidecar(p).exists());
        egui::Window::new(tr("Render")).open(&mut open).default_size([1040.0, 600.0]).show(ui.ctx(), |ui| {
            ui.horizontal(|ui| {
                ui.selectable_value(&mut win.source, Source::Animation, tr("Animation"));
                ui.add_enabled_ui(has_timelapse, |ui| {
                    ui.selectable_value(&mut win.source, Source::Timelapse, tr("Timelapse"))
                        .on_disabled_hover_text(tr("Save the project with Record timelapse on to get one"));
                });
            });
            ui.separator();
            ui.horizontal_top(|ui| {
                // presets
                ui.vertical(|ui| {
                    ui.set_width(190.0);
                    ui.strong(tr("Format"));
                    for p in Preset::ALL {
                        let usable = !p.ffmpeg() || win.ffmpeg.is_some();
                        let resp = ui.add_enabled(
                            usable,
                            egui::Button::selectable(win.preset == p, tr(p.label())),
                        );
                        if resp.clicked() {
                            win.preset = p;
                        }
                        if !usable {
                            resp.on_disabled_hover_text(tr("Needs ffmpeg"));
                        }
                    }
                    if win.ffmpeg.is_none() {
                        ui.add_space(6.0);
                        ffmpeg_offer(ui, win);
                    }
                });
                ui.separator();
                // settings
                ui.vertical(|ui| {
                    ui.set_width(330.0);
                    ui.strong(tr(win.preset.label()));
                    ui.weak(tr(win.preset.blurb()));
                    ui.add_space(4.0);
                    crate::windows::form(ui, "render-settings", |ui| {
                        match win.source {
                            Source::Animation => {
                                ui.label(tr("Scene"));
                                let name = s.animations.get(win.scene).map(|a| a.name.clone()).unwrap_or_default();
                                egui::ComboBox::from_id_salt("render-scene").selected_text(name).show_ui(ui, |ui| {
                                    for (i, a) in s.animations.iter().enumerate() {
                                        ui.selectable_value(&mut win.scene, i, &a.name);
                                    }
                                });
                                ui.end_row();
                                ui.label(tr("Expression"));
                                let ename = win.expression.and_then(|i| s.expressions.get(i)).map_or_else(|| tr("None").to_string(), |e| e.name.clone());
                                egui::ComboBox::from_id_salt("render-expr").selected_text(ename).show_ui(ui, |ui| {
                                    ui.selectable_value(&mut win.expression, None, tr("None"));
                                    for (i, e) in s.expressions.iter().enumerate() {
                                        ui.selectable_value(&mut win.expression, Some(i), &e.name);
                                    }
                                });
                                ui.end_row();
                                ui.label(tr("Range"));
                                let has_work = s.animations.get(win.scene).is_some_and(|a| a.work_area.is_some());
                                ui.add_enabled(has_work, egui::Checkbox::new(&mut win.work_area, tr("Work area only")));
                                ui.end_row();
                            }
                            Source::Timelapse => {
                                ui.label(tr("Hold on the finished model"));
                                ui.add(egui::DragValue::new(&mut win.hold).range(0.0..=30.0).suffix(" s").speed(0.1));
                                ui.end_row();
                            }
                        }
                        ui.label(tr("Frame rate"));
                        ui.add(egui::DragValue::new(&mut win.fps).range(1..=120).suffix(" fps"));
                        ui.end_row();
                        ui.label(tr("Width"));
                        ui.horizontal(|ui| {
                            ui.add(egui::DragValue::new(&mut win.width).range(16..=8192).suffix(" px"));
                            let c = s.puppet.model().canvas;
                            let h = (win.width as f32 * c.height / c.width.max(1.0)).round() as u32;
                            ui.weak(trf("× {height} px", &[("height", &h)]));
                        });
                        ui.end_row();
                        ui.label(tr("Background"));
                        ui.add_enabled(win.preset.alpha(), egui::Checkbox::new(&mut win.transparent, tr("Transparent")));
                        ui.end_row();
                        ui.label(tr("Folder"));
                        ui.horizontal(|ui| {
                            let shown = win.folder.as_ref().map(|f| f.display().to_string()).unwrap_or_else(|| tr("Choose…").to_string());
                            if ui.button(egui::RichText::new(shown).small()).clicked() {
                                pick_folder = true;
                            }
                        });
                        ui.end_row();
                    });
                    ui.add_space(8.0);
                    let ready = win.folder.is_some() && (win.source == Source::Timelapse || !s.animations.is_empty());
                    if ui.add_enabled(ready, egui::Button::new(egui::RichText::new(tr("Add to queue")).strong())).clicked() {
                        enqueue = true;
                    }
                });
                ui.separator();
                // preview and queue
                ui.vertical(|ui| {
                    ui.set_min_width(420.0);
                    ui.strong(tr("Preview"));
                    // the frame keeps the canvas's shape; the image sits
                    // centred on dark or white, as the export will
                    let c = s.puppet.model().canvas;
                    let aspect = (c.height / c.width.max(1.0)).max(0.1);
                    let fw = 420.0_f32;
                    let fh = (fw * aspect).clamp(160.0, 380.0);
                    let fill = if win.transparent && win.preset.alpha() {
                        egui::Color32::from_gray(44)
                    } else {
                        egui::Color32::WHITE
                    };
                    let (frame_rect, _) = ui.allocate_exact_size(egui::vec2(fw, fh), egui::Sense::hover());
                    let painter = ui.painter_at(frame_rect);
                    painter.rect_filled(frame_rect, 6.0, fill);
                    if let Some(t) = &win.preview {
                        let size = t.size_vec2();
                        let k = ((fw - 16.0) / size.x).min((fh - 16.0) / size.y).min(1.0);
                        let shown = size * k;
                        let at = egui::Rect::from_center_size(frame_rect.center(), shown);
                        painter.image(t.id(), at, egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), egui::Color32::WHITE);
                    }
                    painter.rect_stroke(frame_rect, 6.0, egui::Stroke::new(1.0, crate::theme::pal().text_faint.gamma_multiply(0.5)), egui::StrokeKind::Inside);
                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        let icon = if win.playing { egui_phosphor::regular::PAUSE } else { egui_phosphor::regular::PLAY };
                        if ui.add_sized([30.0, 24.0], egui::Button::new(egui::RichText::new(icon).size(15.0))).clicked() {
                            win.playing = !win.playing;
                            win.last_tick = None;
                        }
                        let slider_w = (fw - 30.0 - 72.0).max(80.0);
                        ui.spacing_mut().slider_width = slider_w;
                        match win.source {
                            Source::Animation => {
                                let end = s.animations.get(win.scene).map_or(0.0, |a| a.duration).max(0.0);
                                if ui.add(egui::Slider::new(&mut win.time, 0.0..=end).show_value(false)).changed() {
                                    win.playing = false;
                                }
                                ui.weak(egui::RichText::new(format!("{:6.2}s", win.time)).monospace());
                            }
                            Source::Timelapse => {
                                let n = win.player.as_ref().map_or(0, |p| p.steps.len()).saturating_sub(1);
                                if ui.add(egui::Slider::new(&mut win.step, 0..=n).show_value(false)).changed() {
                                    win.playing = false;
                                }
                                ui.weak(egui::RichText::new(format!("{}/{}", win.step, n)).monospace());
                            }
                        }
                    });
                    ui.add_space(8.0);
                    ui.strong(tr("Queue"));
                    if win.queue.is_empty() {
                        ui.weak(tr("Nothing queued yet."));
                    }
                    let mut remove = None;
                    for (i, j) in win.queue.iter().enumerate() {
                        ui.horizontal(|ui| {
                            ui.label(&j.label);
                            match &j.state {
                                State::Queued(_) => {
                                    ui.weak(tr("queued"));
                                    if ui.small_button("×").clicked() {
                                        remove = Some(i);
                                    }
                                }
                                State::Running => {
                                    let f = if j.total > 0 { j.done as f32 / j.total as f32 } else { 0.0 };
                                    ui.add(egui::ProgressBar::new(f).desired_width(110.0).text(format!("{}/{}", j.done, j.total)));
                                }
                                State::Done(m) => {
                                    ui.colored_label(crate::theme::pal().warp_green, egui_phosphor::regular::CHECK).on_hover_text(m);
                                    if ui.small_button("×").clicked() {
                                        remove = Some(i);
                                    }
                                }
                                State::Failed(e) => {
                                    ui.colored_label(crate::theme::pal().accent, egui_phosphor::regular::WARNING).on_hover_text(e);
                                    if ui.small_button("×").clicked() {
                                        remove = Some(i);
                                    }
                                }
                            }
                        });
                    }
                    if let Some(i) = remove {
                        win.queue.remove(i);
                    }
                });
            });
        });
        if enqueue && let Some(folder) = win.folder.clone() {
            let anim = s.animations.get(win.scene).cloned();
            let stem = match (win.source, &anim) {
                (Source::Timelapse, _) => format!("{}-timelapse", s.name),
                (_, Some(a)) => format!("{}-{}", s.name, a.name),
                _ => s.name.clone(),
            };
            let label = format!("{} · {}", stem, win.preset.label());
            let spec = Spec {
                source: win.source,
                preset: win.preset,
                fps: win.fps,
                width: win.width,
                transparent: win.transparent && win.preset.alpha(),
                work_area: win.work_area,
                hold: win.hold,
                physics: self.physics_on,
                folder,
                name: stem,
                model: s.puppet.model().clone(),
                pngs: s.texture_pngs.clone(),
                anim,
                expression: win.expression.and_then(|i| s.expressions.get(i).cloned()),
                timelapse: s.aom_path.as_ref().map(|p| crate::timelapse::Recorder::sidecar(p)),
                ffmpeg: win.ffmpeg.clone().unwrap_or_else(|| PathBuf::from("ffmpeg")),
            };
            win.queue.push(Job { label, done: 0, total: 0, state: State::Queued(Box::new(spec)) });
        }
        if pick_folder {
            self.pick_async(crate::panels::PickFor::RenderFolder, || {
                rfd::FileDialog::new().set_title(tr("Render into which folder?")).pick_folder()
            });
        }
        if !open {
            // a job already running finishes on its own thread; its files
            // land in the folder either way
            self.render_win = None;
        }
    }
}
