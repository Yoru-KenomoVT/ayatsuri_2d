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

use ayatsuri_format::model::{Canvas, Model};
use ayatsuri_format::{Animation, CurveTarget, Expression, ExpressionBlend};
use egui::Ui;

use crate::i18n::{tr, trf};
use crate::model_diff::{Change, Counts, Kind, What};
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
    /// Which source the puppet was built for; switching rebuilds it.
    puppet_src: Option<Source>,
    player: Option<crate::timelapse::Player>,
    loop_on: bool,
    /// Timelapse speed: 1 plays the whole thing in twenty seconds.
    speed: f32,
    /// Fractional steps carried between frames at low speeds.
    carry: f32,
    /// Draw what changed at this step over the frame.
    show_changes: bool,
    /// The step shown last and its model, the base of the next diff.
    prev_step: Option<(usize, Model)>,
    changes: Vec<Change>,
    counts: Counts,
    /// One change picked in the list, highlighted alone.
    picked: Option<usize>,
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
            puppet_src: None,
            player: None,
            loop_on: true,
            speed: 1.0,
            carry: 0.0,
            show_changes: true,
            prev_step: None,
            changes: Vec::new(),
            counts: Counts::default(),
            picked: None,
            queue: Vec::new(),
            rx: None,
        }
    }

    /// Draw the preview frame for the current time or step; for a timelapse
    /// also work out what changed since the step shown before.
    fn refresh_preview(&mut self, ctx: &egui::Context, s: &crate::session::Session) {
        if self.puppet_src != Some(self.source) {
            self.puppet = None;
            self.puppet_src = Some(self.source);
        }
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
                // the step before: the one shown last when playing forward,
                // else rebuilt; step 0 is measured against nothing
                let before = match self.prev_step.take() {
                    Some((i, m)) if i < self.step => Some(m),
                    _ if self.step == 0 => Some(Model::new()),
                    _ => player.model(self.step - 1),
                };
                let Some(model) = player.model(self.step) else { return };
                if let Some(b) = &before {
                    self.changes = crate::model_diff::diff(b, &model);
                    self.picked = None;
                }
                self.counts = crate::model_diff::counts(&model);
                self.prev_step = Some((self.step, model.clone()));
                let mut puppet = ayatsuri_core::Puppet::new(model);
                puppet.update();
                let frame = crate::thumb::render(&puppet, &s.preview_tex, PREVIEW_W);
                self.puppet = Some(puppet);
                frame
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
        win.puppet_src = None;
        win.player = None;
        win.prev_step = None;
        win.changes.clear();
        win.picked = None;
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
                            if win.loop_on && a.duration > 0.0 {
                                win.time %= a.duration;
                            } else {
                                win.time = a.duration.max(0.0);
                                win.playing = false;
                            }
                        }
                    }
                }
                Source::Timelapse => {
                    let n = win.player.as_ref().map_or(0, |p| p.steps.len());
                    win.carry += dt * win.speed * (n as f32 / 20.0).max(1.0);
                    let whole = win.carry.floor();
                    win.carry -= whole;
                    win.step += whole as usize;
                    if n > 0 && win.step >= n {
                        if win.loop_on {
                            win.step = 0;
                        } else {
                            win.step = n - 1;
                            win.playing = false;
                        }
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
        egui::Window::new(tr("Render")).open(&mut open).default_size([1290.0, 660.0]).show(ui.ctx(), |ui| {
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
                // the player with what drives the frame beside it, the
                // queue underneath
                ui.vertical(|ui| {
                    ui.horizontal_top(|ui| {
                        win.player_ui(ui, s);
                        ui.add_space(6.0);
                        win.side_panel(ui, s);
                    });
                    ui.add_space(8.0);
                    win.queue_ui(ui);
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

/// The player's width; the side panel takes the rest of the column.
const PLAYER_W: f32 = 540.0;
const SIDE_W: f32 = 240.0;
const STRIP_H: f32 = 22.0;
const BAR_H: f32 = 34.0;
/// Changes listed for one step before the list says "and more".
const MAX_CHANGES: usize = 80;

/// MM:SS:FF at the scene's frame rate.
fn timecode(t: f32, fps: f32) -> String {
    let fps = fps.max(1.0);
    let per = fps.round().max(1.0) as u64;
    let frames = (t.max(0.0) * fps).round() as u64;
    let secs = frames / per;
    format!("{:02}:{:02}:{:02}", secs / 60, secs % 60, frames % per)
}

/// A wall-clock span in the largest two units that matter.
fn span(ms: u64) -> String {
    let s = ms / 1000;
    let (h, m) = (s / 3600, (s / 60) % 60);
    if h > 0 {
        trf("{h} h {m} min", &[("h", &h), ("m", &m)])
    } else if m > 0 {
        trf("{m} min", &[("m", &m)])
    } else {
        trf("{s} s", &[("s", &s)])
    }
}

/// A flat glyph button in the transport bar; `on` lights it in the accent.
fn icon_button(ui: &mut Ui, icon: &str, on: bool, tip: &str) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(26.0, 24.0), egui::Sense::click());
    let p = ui.painter();
    if resp.hovered() {
        p.rect_filled(rect, 4.0, egui::Color32::from_white_alpha(18));
    }
    let col = if on {
        crate::theme::pal().accent
    } else if resp.hovered() {
        egui::Color32::from_gray(240)
    } else {
        egui::Color32::from_gray(185)
    };
    p.text(rect.center(), egui::Align2::CENTER_CENTER, icon, egui::FontId::proportional(15.0), col);
    resp.on_hover_text(tip)
}

/// A small text button in the bar, for the speed.
fn text_button(ui: &mut Ui, text: &str, tip: &str) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(40.0, 24.0), egui::Sense::click());
    let p = ui.painter();
    if resp.hovered() {
        p.rect_filled(rect, 4.0, egui::Color32::from_white_alpha(18));
    }
    let col =
        if resp.hovered() { egui::Color32::from_gray(240) } else { egui::Color32::from_gray(185) };
    p.text(rect.center(), egui::Align2::CENTER_CENTER, text, egui::FontId::monospace(11.5), col);
    resp.on_hover_text(tip)
}

/// A scrubber: the played part in the accent, a knob that grows under the
/// pointer. Returns the new position when clicked or dragged.
fn scrubber(ui: &mut Ui, frac: f32, width: f32) -> Option<f32> {
    let (rect, resp) =
        ui.allocate_exact_size(egui::vec2(width.max(40.0), 24.0), egui::Sense::click_and_drag());
    let p = ui.painter();
    let pal = crate::theme::pal();
    let (x0, x1, y) = (rect.min.x + 6.0, rect.max.x - 6.0, rect.center().y);
    let frac = frac.clamp(0.0, 1.0);
    let kx = x0 + (x1 - x0) * frac;
    p.line_segment(
        [egui::pos2(x0, y), egui::pos2(x1, y)],
        egui::Stroke::new(3.0, egui::Color32::from_gray(58)),
    );
    p.line_segment([egui::pos2(x0, y), egui::pos2(kx, y)], egui::Stroke::new(3.0, pal.accent));
    let hot = resp.hovered() || resp.dragged();
    p.circle_filled(egui::pos2(kx, y), if hot { 7.0 } else { 5.0 }, pal.accent);
    p.circle_stroke(
        egui::pos2(kx, y),
        if hot { 7.0 } else { 5.0 },
        egui::Stroke::new(1.0, egui::Color32::from_gray(20)),
    );
    if (resp.clicked() || resp.dragged())
        && let Some(pos) = resp.interact_pointer_pos()
    {
        return Some(((pos.x - x0) / (x1 - x0).max(1.0)).clamp(0.0, 1.0));
    }
    None
}

impl RenderWin {
    /// The frame's height for a canvas shape at the player's width.
    fn frame_h(c: Canvas) -> f32 {
        let aspect = (c.height / c.width.max(1.0)).max(0.1);
        (PLAYER_W * aspect).clamp(220.0, 400.0)
    }

    /// The player: a title strip, the frame on dark with the output's
    /// background under the model, and a transport bar.
    fn player_ui(&mut self, ui: &mut Ui, s: &crate::session::Session) {
        use egui::{Align2, Color32, FontId, Rect, Sense, pos2, vec2};
        let canvas = self.puppet.as_ref().map_or(s.puppet.model().canvas, |p| p.model().canvas);
        let fh = Self::frame_h(s.puppet.model().canvas);
        let (panel, panel_resp) =
            ui.allocate_exact_size(vec2(PLAYER_W, STRIP_H + fh + BAR_H), Sense::hover());
        let p = ui.painter_at(panel);
        p.rect_filled(panel, 8.0, Color32::from_gray(20));
        // title strip: what plays, at what size and rate
        let strip = Rect::from_min_size(panel.min, vec2(PLAYER_W, STRIP_H));
        let name = match self.source {
            Source::Animation => {
                s.animations.get(self.scene).map(|a| a.name.clone()).unwrap_or_default()
            }
            Source::Timelapse => tr("Timelapse").to_string(),
        };
        let c = s.puppet.model().canvas;
        let out_h = (self.width as f32 * c.height / c.width.max(1.0)).round() as u32;
        p.text(
            pos2(strip.min.x + 10.0, strip.center().y),
            Align2::LEFT_CENTER,
            name,
            FontId::proportional(11.5),
            Color32::from_gray(200),
        );
        p.text(
            pos2(strip.max.x - 10.0, strip.center().y),
            Align2::RIGHT_CENTER,
            format!("{}×{}  {} fps  {}", self.width, out_h, self.fps, tr(self.preset.label())),
            FontId::monospace(10.5),
            Color32::from_gray(120),
        );
        // the frame: the canvas letterboxed, on white or a checker as the
        // export will be, the model over it
        let frame =
            Rect::from_min_size(pos2(panel.min.x, panel.min.y + STRIP_H), vec2(PLAYER_W, fh));
        let transparent = self.transparent && self.preset.alpha();
        let at = match &self.preview {
            Some(t) => {
                let size = t.size_vec2();
                let k = ((frame.width() - 12.0) / size.x).min((frame.height() - 12.0) / size.y);
                Rect::from_center_size(frame.center(), size * k)
            }
            None => frame.shrink(6.0),
        };
        if transparent {
            let cp = p.with_clip_rect(at);
            let cell = 10.0;
            let (nx, ny) = ((at.width() / cell).ceil() as i32, (at.height() / cell).ceil() as i32);
            for j in 0..ny {
                for i in 0..nx {
                    let dark = (i + j) % 2 == 0;
                    let r = Rect::from_min_size(
                        pos2(at.min.x + i as f32 * cell, at.min.y + j as f32 * cell),
                        vec2(cell, cell),
                    );
                    cp.rect_filled(
                        r,
                        0.0,
                        if dark { Color32::from_gray(58) } else { Color32::from_gray(74) },
                    );
                }
            }
        } else {
            p.rect_filled(at, 0.0, Color32::WHITE);
        }
        if let Some(t) = &self.preview {
            p.image(t.id(), at, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
        }
        if self.source == Source::Timelapse && self.show_changes {
            self.overlay(&p.with_clip_rect(at), at, canvas);
        }
        // click the frame to play or pause, as any player does
        let frame_resp = ui.interact(frame, ui.id().with("render-frame"), Sense::click());
        if frame_resp.clicked() {
            self.playing = !self.playing;
            self.last_tick = None;
        }
        // the transport bar
        let bar = Rect::from_min_size(
            pos2(panel.min.x + 6.0, frame.max.y + 5.0),
            vec2(PLAYER_W - 12.0, BAR_H - 10.0),
        );
        let mut b = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(bar)
                .layout(egui::Layout::left_to_right(egui::Align::Center)),
        );
        b.spacing_mut().item_spacing.x = 2.0;
        let n = self.player.as_ref().map_or(0, |pl| pl.steps.len());
        let last = n.saturating_sub(1);
        let anim = s.animations.get(self.scene);
        let (dur, fps) = anim.map_or((0.0, 30.0), |a| (a.duration.max(0.0), a.fps));
        if icon_button(&mut b, egui_phosphor::regular::SKIP_BACK, false, tr("Start")).clicked() {
            self.time = 0.0;
            self.step = 0;
            self.prev_step = None;
        }
        let play =
            if self.playing { egui_phosphor::regular::PAUSE } else { egui_phosphor::regular::PLAY };
        if icon_button(&mut b, play, false, tr("Play or pause (Space)")).clicked() {
            self.playing = !self.playing;
            self.last_tick = None;
        }
        if icon_button(&mut b, egui_phosphor::regular::SKIP_FORWARD, false, tr("End")).clicked() {
            self.time = dur;
            self.step = last;
            self.playing = false;
        }
        b.add_space(4.0);
        let right_w = match self.source {
            Source::Animation => 148.0,
            Source::Timelapse => 232.0,
        };
        let sw = (bar.width() - 84.0 - right_w).max(60.0);
        match self.source {
            Source::Animation => {
                let frac = if dur > 0.0 { self.time / dur } else { 0.0 };
                if let Some(f) = scrubber(&mut b, frac, sw) {
                    self.time = f * dur;
                    self.playing = false;
                }
                b.add_space(6.0);
                b.label(
                    egui::RichText::new(format!(
                        "{} / {}",
                        timecode(self.time, fps),
                        timecode(dur, fps)
                    ))
                    .monospace()
                    .size(11.5)
                    .color(Color32::from_gray(200)),
                );
            }
            Source::Timelapse => {
                let frac = if last > 0 { self.step as f32 / last as f32 } else { 0.0 };
                if let Some(f) = scrubber(&mut b, frac, sw) {
                    let to = (f * last as f32).round() as usize;
                    if to < self.step {
                        self.prev_step = None;
                    }
                    self.step = to;
                    self.playing = false;
                }
                b.add_space(6.0);
                b.label(
                    egui::RichText::new(format!("{} / {}", self.step + usize::from(n > 0), n))
                        .monospace()
                        .size(11.5)
                        .color(Color32::from_gray(200)),
                );
                b.add_space(2.0);
                let speed_txt = if self.speed >= 1.0 {
                    format!("{}×", self.speed as u32)
                } else {
                    format!("{}×", self.speed)
                };
                if text_button(
                    &mut b,
                    &speed_txt,
                    tr("Speed: at 1× the whole timelapse plays in twenty seconds"),
                )
                .clicked()
                {
                    self.speed = match self.speed {
                        x if x < 0.5 => 0.5,
                        x if x < 1.0 => 1.0,
                        x if x < 2.0 => 2.0,
                        x if x < 4.0 => 4.0,
                        x if x < 8.0 => 8.0,
                        _ => 0.25,
                    };
                }
                if icon_button(&mut b, egui_phosphor::regular::EYE, self.show_changes, tr("Highlight what changed at this step: meshes in orange, new ones in green, deformer grids")).clicked() {
                    self.show_changes = !self.show_changes;
                }
            }
        }
        if icon_button(&mut b, egui_phosphor::regular::REPEAT, self.loop_on, tr("Loop")).clicked() {
            self.loop_on = !self.loop_on;
        }
        // keys while the pointer is over the player
        if panel_resp.hovered() || frame_resp.hovered() {
            let (space, left, right, shift) = ui.input(|i| {
                (
                    i.key_pressed(egui::Key::Space),
                    i.key_pressed(egui::Key::ArrowLeft),
                    i.key_pressed(egui::Key::ArrowRight),
                    i.modifiers.shift,
                )
            });
            if space {
                self.playing = !self.playing;
                self.last_tick = None;
            }
            if left || right {
                self.playing = false;
                let dir: f32 = if right { 1.0 } else { -1.0 };
                let by = if shift { 10.0 } else { 1.0 };
                match self.source {
                    Source::Animation => {
                        self.time = (self.time + dir * by / fps.max(1.0)).clamp(0.0, dur);
                    }
                    Source::Timelapse => {
                        let to = (self.step as f32 + dir * by).clamp(0.0, last as f32) as usize;
                        if to < self.step {
                            self.prev_step = None;
                        }
                        self.step = to;
                    }
                }
            }
        }
    }

    /// Draw this step's changed objects over the frame: meshes as
    /// wireframes, warp grids as grids, rotation deformers as their pivot.
    fn overlay(&self, p: &egui::Painter, at: egui::Rect, c: Canvas) {
        use egui::{Stroke, pos2};
        let Some(puppet) = &self.puppet else { return };
        let pal = crate::theme::pal();
        let k = at.width() / c.width.max(1.0);
        let to = |x: f32, y: f32| {
            pos2(
                at.min.x + (c.origin_x + x * c.pixels_per_unit) * k,
                at.min.y + (c.origin_y + y * c.pixels_per_unit) * k,
            )
        };
        for (ci, ch) in self.changes.iter().enumerate() {
            if self.picked.is_some_and(|pk| pk != ci) {
                continue;
            }
            let Some(i) = ch.index else { continue };
            let col = match ch.kind {
                Kind::Added => pal.warp_green,
                Kind::Edited => pal.accent,
                Kind::Removed => pal.rotation_red,
            };
            match ch.what {
                What::Mesh => {
                    let Some(g) = puppet.mesh_geometry(i) else { continue };
                    let (pts, _) = g.positions.as_chunks::<2>();
                    let (tris, _) = g.indices.as_chunks::<3>();
                    let stroke = Stroke::new(1.0, col.gamma_multiply(0.75));
                    for &[a, b, d] in tris {
                        let (Some(pa), Some(pb), Some(pd)) =
                            (pts.get(a as usize), pts.get(b as usize), pts.get(d as usize))
                        else {
                            continue;
                        };
                        let (qa, qb, qd) = (to(pa[0], pa[1]), to(pb[0], pb[1]), to(pd[0], pd[1]));
                        p.line_segment([qa, qb], stroke);
                        p.line_segment([qb, qd], stroke);
                        p.line_segment([qd, qa], stroke);
                    }
                }
                What::Warp => {
                    let Some((grid, rows, cols)) = puppet.warp_grid(i) else { continue };
                    let (pts, _) = grid.as_chunks::<2>();
                    let stride = cols as usize + 1;
                    let stroke = Stroke::new(1.3, col);
                    for r in 0..=rows as usize {
                        let line: Vec<egui::Pos2> = (0..stride)
                            .filter_map(|cc| pts.get(r * stride + cc))
                            .map(|q| to(q[0], q[1]))
                            .collect();
                        p.add(egui::Shape::line(line, stroke));
                    }
                    for cc in 0..stride {
                        let line: Vec<egui::Pos2> = (0..=rows as usize)
                            .filter_map(|r| pts.get(r * stride + cc))
                            .map(|q| to(q[0], q[1]))
                            .collect();
                        p.add(egui::Shape::line(line, stroke));
                    }
                    for q in pts {
                        p.circle_filled(to(q[0], q[1]), 2.0, col);
                    }
                }
                What::Rotation => {
                    let Some(rot) = puppet.rotation_state(i) else { continue };
                    let o = to(rot.origin.x, rot.origin.y);
                    let a = rot.angle.to_radians();
                    let tip = o + egui::vec2(a.sin(), -a.cos()) * 26.0;
                    p.circle_stroke(o, 7.0, Stroke::new(2.0, col));
                    p.circle_filled(o, 2.5, col);
                    p.line_segment([o, tip], Stroke::new(2.0, col));
                    p.circle_filled(tip, 3.0, col);
                }
                _ => {}
            }
        }
    }

    /// Beside the player: the parameters a scene drives, or what a timelapse
    /// step changed.
    fn side_panel(&mut self, ui: &mut Ui, s: &crate::session::Session) {
        let h = STRIP_H + Self::frame_h(s.puppet.model().canvas) + BAR_H;
        ui.allocate_ui_with_layout(
            egui::vec2(SIDE_W, h),
            egui::Layout::top_down(egui::Align::Min),
            |ui| {
                ui.set_min_height(h);
                match self.source {
                    Source::Animation => self.params_panel(ui, s, h),
                    Source::Timelapse => self.changes_panel(ui, h),
                }
            },
        );
    }

    fn params_panel(&self, ui: &mut Ui, s: &crate::session::Session, h: f32) {
        let anim = s.animations.get(self.scene);
        let model = self.puppet.as_ref().map_or(s.puppet.model(), |p| p.model());
        let live: Vec<&str> = anim
            .map(|a| {
                a.curves
                    .iter()
                    .filter(|c| c.target == CurveTarget::Parameter)
                    .map(|c| c.id.as_str())
                    .collect()
            })
            .unwrap_or_default();
        let part_curves: Vec<&str> = anim
            .map(|a| {
                a.curves
                    .iter()
                    .filter(|c| c.target == CurveTarget::PartOpacity)
                    .map(|c| c.id.as_str())
                    .collect()
            })
            .unwrap_or_default();
        ui.horizontal(|ui| {
            ui.strong(tr("Parameters"));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.weak(egui::RichText::new(trf("{n} animated", &[("n", &live.len())])).small());
            });
        });
        // animated first, in the scene's order, then the rest
        let mut order: Vec<usize> = live
            .iter()
            .filter_map(|id| model.parameters.iter().position(|p| p.id == *id))
            .collect();
        order.dedup();
        for (i, p) in model.parameters.iter().enumerate() {
            if !live.contains(&p.id.as_str()) {
                order.push(i);
            }
        }
        egui::ScrollArea::vertical().id_salt("render-params").max_height(h - 24.0).show(ui, |ui| {
            ui.set_min_width(SIDE_W - 14.0);
            for i in order {
                let Some(p) = model.parameters.get(i) else { continue };
                let v = self.puppet.as_ref().map_or(p.default, |pp| pp.value(i));
                value_row(
                    ui,
                    &p.id,
                    v,
                    p.min,
                    p.max,
                    p.default,
                    p.decimals,
                    live.contains(&p.id.as_str()),
                );
            }
            if !part_curves.is_empty() {
                ui.add_space(6.0);
                ui.strong(tr("Part opacity"));
                for id in part_curves {
                    let Some(i) = model.parts.iter().position(|p| p.id == id) else { continue };
                    let v = self.puppet.as_ref().map_or(1.0, |pp| pp.part_user_opacity(i));
                    value_row(ui, id, v, 0.0, 1.0, 1.0, 2, true);
                }
            }
        });
    }

    fn changes_panel(&mut self, ui: &mut Ui, h: f32) {
        let pal = crate::theme::pal();
        let (elapsed, total) = match &self.player {
            Some(pl) => {
                let t0 = pl.steps.first().map_or(0, |x| x.0);
                let now = pl.steps.get(self.step).map_or(t0, |x| x.0);
                let end = pl.steps.last().map_or(t0, |x| x.0);
                (now.saturating_sub(t0), end.saturating_sub(t0))
            }
            None => (0, 0),
        };
        ui.horizontal(|ui| {
            ui.strong(tr("This step"));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.weak(
                    egui::RichText::new(trf(
                        "{at} of {total}",
                        &[("at", &span(elapsed)), ("total", &span(total))],
                    ))
                    .small(),
                )
                .on_hover_text(tr("Working time into the rig at this step"));
            });
        });
        let c = self.counts;
        egui::Grid::new("render-counts").num_columns(4).spacing([6.0, 1.0]).show(ui, |ui| {
            let cell = |ui: &mut Ui, label: &str, n: usize| {
                ui.weak(egui::RichText::new(label).small());
                ui.label(egui::RichText::new(n.to_string()).monospace().small());
            };
            cell(ui, tr("Meshes"), c.meshes);
            cell(ui, tr("Deformers"), c.deformers);
            ui.end_row();
            cell(ui, tr("Parameters"), c.parameters);
            cell(ui, tr("Parts"), c.parts);
            ui.end_row();
            cell(ui, tr("Keyforms"), c.keyforms);
            cell(ui, tr("Points"), c.points);
            ui.end_row();
        });
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.strong(if self.step == 0 { tr("As recording began") } else { tr("Changed") });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if !self.changes.is_empty() {
                    ui.weak(egui::RichText::new(self.changes.len().to_string()).small());
                }
            });
        });
        egui::ScrollArea::vertical().id_salt("render-changes").max_height(h - 112.0).show(
            ui,
            |ui| {
                ui.set_min_width(SIDE_W - 14.0);
                if self.changes.is_empty() {
                    ui.weak(
                        egui::RichText::new(tr("Nothing changed between these steps.")).small(),
                    );
                }
                let w = ui.available_width();
                let mut toggle = None;
                for (i, ch) in self.changes.iter().enumerate().take(MAX_CHANGES) {
                    let on = self.picked == Some(i);
                    let (r, resp) =
                        ui.allocate_exact_size(egui::vec2(w, 20.0), egui::Sense::click());
                    let p = ui.painter();
                    if on {
                        p.rect_filled(r, 3.0, pal.select);
                    } else if resp.hovered() {
                        p.rect_filled(r, 3.0, pal.text_faint.gamma_multiply(0.15));
                    }
                    let col = match ch.kind {
                        Kind::Added => pal.warp_green,
                        Kind::Edited => pal.accent,
                        Kind::Removed => pal.rotation_red,
                    };
                    p.circle_filled(egui::pos2(r.min.x + 7.0, r.center().y), 3.0, col);
                    let icon = match ch.what {
                        What::Mesh => egui_phosphor::regular::POLYGON,
                        What::Warp => egui_phosphor::regular::GRID_FOUR,
                        What::Rotation => egui_phosphor::regular::ARROW_CLOCKWISE,
                        What::Parameter => egui_phosphor::regular::SLIDERS_HORIZONTAL,
                        What::Part => egui_phosphor::regular::FOLDER_SIMPLE,
                        What::Glue => egui_phosphor::regular::LINK_SIMPLE,
                        What::Physics => egui_phosphor::regular::WAVES,
                        What::ArtPath => egui_phosphor::regular::PEN_NIB,
                        What::Canvas => egui_phosphor::regular::FRAME_CORNERS,
                    };
                    let name = match ch.what {
                        What::Physics => tr("Physics").to_string(),
                        What::Canvas => tr("Canvas").to_string(),
                        _ => ch.name.clone(),
                    };
                    let detail = match ch.kind {
                        Kind::Added => tr("added").to_string(),
                        Kind::Removed => tr("removed").to_string(),
                        Kind::Edited => tr(ch.detail).to_string(),
                    };
                    let dw = p
                        .layout_no_wrap(
                            detail.clone(),
                            egui::FontId::proportional(10.5),
                            pal.text_faint,
                        )
                        .size()
                        .x;
                    let name_clip = egui::Rect::from_min_max(
                        egui::pos2(r.min.x + 16.0, r.min.y),
                        egui::pos2((r.max.x - dw - 10.0).max(r.min.x + 30.0), r.max.y),
                    );
                    p.with_clip_rect(name_clip).text(
                        egui::pos2(r.min.x + 16.0, r.center().y),
                        egui::Align2::LEFT_CENTER,
                        format!("{icon} {name}"),
                        egui::FontId::proportional(12.0),
                        pal.text,
                    );
                    p.text(
                        egui::pos2(r.max.x - 4.0, r.center().y),
                        egui::Align2::RIGHT_CENTER,
                        detail,
                        egui::FontId::proportional(10.5),
                        pal.text_faint,
                    );
                    let resp =
                        resp.on_hover_text(tr("Click to highlight only this one on the frame"));
                    if resp.clicked() {
                        toggle = Some(i);
                    }
                }
                if self.changes.len() > MAX_CHANGES {
                    ui.weak(
                        egui::RichText::new(trf(
                            "  and {count} more",
                            &[("count", &(self.changes.len() - MAX_CHANGES))],
                        ))
                        .small(),
                    );
                }
                if let Some(i) = toggle {
                    self.picked = if self.picked == Some(i) { None } else { Some(i) };
                }
            },
        );
    }

    fn queue_ui(&mut self, ui: &mut Ui) {
        ui.strong(tr("Queue"));
        if self.queue.is_empty() {
            ui.weak(tr("Nothing queued yet."));
        }
        let mut remove = None;
        for (i, j) in self.queue.iter().enumerate() {
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
                        ui.add(
                            egui::ProgressBar::new(f)
                                .desired_width(160.0)
                                .text(format!("{}/{}", j.done, j.total)),
                        );
                    }
                    State::Done(m) => {
                        ui.colored_label(
                            crate::theme::pal().warp_green,
                            egui_phosphor::regular::CHECK,
                        )
                        .on_hover_text(m);
                        if ui.small_button("×").clicked() {
                            remove = Some(i);
                        }
                    }
                    State::Failed(e) => {
                        ui.colored_label(
                            crate::theme::pal().accent,
                            egui_phosphor::regular::WARNING,
                        )
                        .on_hover_text(e);
                        if ui.small_button("×").clicked() {
                            remove = Some(i);
                        }
                    }
                }
            });
        }
        if let Some(i) = remove {
            self.queue.remove(i);
        }
    }
}

/// One parameter the way a model viewer lists them: name and value above
/// a track, the knob at the value, the stretch from the default filled in
/// when the scene drives it.
fn value_row(
    ui: &mut Ui,
    name: &str,
    value: f32,
    min: f32,
    max: f32,
    default: f32,
    decimals: i32,
    live: bool,
) {
    use egui::{Align2, FontId, Rect, Sense, pos2, vec2};
    let pal = crate::theme::pal();
    let w = ui.available_width();
    let (r, resp) = ui.allocate_exact_size(vec2(w, 30.0), Sense::hover());
    let p = ui.painter();
    let col = if live { pal.text } else { pal.text_faint };
    let txt = format!("{:.*}", decimals.clamp(0, 3) as usize, value);
    let tw = p.layout_no_wrap(txt.clone(), FontId::monospace(11.0), col).size().x;
    let name_clip = Rect::from_min_max(r.min, pos2(r.max.x - tw - 8.0, r.max.y));
    p.with_clip_rect(name_clip).text(
        pos2(r.min.x, r.min.y + 2.0),
        Align2::LEFT_TOP,
        name,
        FontId::proportional(11.5),
        col,
    );
    p.text(pos2(r.max.x, r.min.y + 2.0), Align2::RIGHT_TOP, txt, FontId::monospace(11.0), col);
    let ty = r.max.y - 7.0;
    let span = (max - min).max(1e-6);
    let fx = |v: f32| r.min.x + 4.0 + ((v - min) / span).clamp(0.0, 1.0) * (r.width() - 8.0);
    p.line_segment(
        [pos2(r.min.x + 4.0, ty), pos2(r.max.x - 4.0, ty)],
        egui::Stroke::new(3.0, pal.text_faint.gamma_multiply(0.35)),
    );
    if live {
        p.line_segment(
            [pos2(fx(default), ty), pos2(fx(value), ty)],
            egui::Stroke::new(3.0, pal.accent.gamma_multiply(0.85)),
        );
    }
    p.circle_filled(pos2(fx(value), ty), 4.0, if live { pal.accent } else { pal.text_faint });
    p.circle_stroke(pos2(fx(value), ty), 4.0, egui::Stroke::new(1.0, pal.panel_bg));
    resp.on_hover_text(format!("{name}: {min} to {max}, {default} at rest"));
}
