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
use crate::windows::span_short;

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

/// Preview frames are drawn at the player's size on screen, within this
/// many pixels.
const PREVIEW_PX: f32 = 1400.0 * 1000.0;

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
    /// Timelapse steps per second of output; each step eases in over its
    /// share of the second.
    sps: f32,
    physics: bool,
    /// Frames drawn by the GPU, or on the CPU; and, on the CPU, at twice
    /// the size and averaged down.
    gpu: bool,
    aa: bool,
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
    Done,
    Failed(String),
    Stopped,
}

struct Job {
    name: String,
    preset: Preset,
    folder: PathBuf,
    done: usize,
    total: usize,
    state: State,
    /// Raised by the stop button; the job sees it at the next frame.
    stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
    started: Option<std::time::Instant>,
    took: Option<std::time::Duration>,
}

impl Job {
    fn label(&self) -> String {
        format!("{} · {}", self.name, self.preset.label())
    }
}

/// The error a stopped job ends with.
const STOPPED: &str = "stopped";

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
    /// Where in the timelapse the player is: a step and the way to the next.
    pos: f32,
    /// Timelapse steps per second at 1×.
    sps: f32,
    last_tick: Option<std::time::Instant>,
    /// The thread that poses, diffs and rasters frames; the editor never
    /// waits on it, it just shows the newest frame that came back.
    worker: Option<Worker>,
    /// The frame last asked of the worker, so the same one isn't sent twice.
    asked: Option<Key>,
    /// When each timelapse step was recorded (unix ms).
    stamps: Vec<u64>,
    /// The shown frame's canvas, parameter values and part opacities.
    canvas: Canvas,
    values: Vec<f32>,
    part_opacity: Vec<f32>,
    /// The shown step's changed objects, in canvas pixels, by change index.
    marks: Vec<(usize, Mark)>,
    /// The parameters the shown step is posed at, by name.
    pose: Vec<(String, f32)>,
    /// The player's size at the last layout, which sets the raster width.
    player_size: (f32, f32),
    /// The player and its panel over the whole screen.
    fullscreen: bool,
    /// The window is open; jobs go on either way.
    shown: bool,
    /// Frames drawn by the GPU; else on the CPU, smoothed or not.
    gpu: bool,
    aa: bool,
    loop_on: bool,
    /// Timelapse speed: 1 plays the whole thing in twenty seconds.
    speed: f32,
    /// Draw what changed at this step over the frame.
    show_changes: bool,
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
            pos: 0.0,
            sps: 6.0,
            last_tick: None,
            worker: None,
            asked: None,
            stamps: Vec::new(),
            canvas: Canvas::default(),
            values: Vec::new(),
            part_opacity: Vec::new(),
            marks: Vec::new(),
            pose: Vec::new(),
            player_size: (540.0, 400.0),
            fullscreen: false,
            shown: true,
            gpu: true,
            aa: true,
            loop_on: true,
            speed: 1.0,
            show_changes: true,
            changes: Vec::new(),
            counts: Counts::default(),
            picked: None,
            queue: Vec::new(),
            rx: None,
        }
    }

    /// Show the newest frame the worker sent, then ask it for the one the
    /// clock is on now. A worker is started for the source when missing.
    fn refresh_preview(&mut self, ctx: &egui::Context, s: &crate::session::Session) {
        if self.worker.as_ref().is_none_or(|w| w.source != self.source) {
            self.worker = Worker::start(ctx.clone(), s, self.source);
            self.stamps.clear();
            self.asked = None;
            self.changes.clear();
            self.marks.clear();
            self.picked = None;
        }
        let Some(w) = &self.worker else { return };
        let mut newest = None;
        while let Ok(f) = w.rx.try_recv() {
            newest = Some(f);
        }
        if let Some(f) = newest {
            let img = egui::ColorImage::from_rgba_unmultiplied([f.w, f.h], &f.px);
            match &mut self.preview {
                Some(t) => t.set(img, egui::TextureOptions::LINEAR),
                None => {
                    self.preview =
                        Some(ctx.load_texture("render-preview", img, egui::TextureOptions::LINEAR))
                }
            }
            self.canvas = f.canvas;
            if let Some(st) = f.stamps {
                self.stamps = st;
            }
            self.values = f.values;
            self.part_opacity = f.part_opacity;
            if let Some(info) = f.step {
                self.changes = info.changes;
                self.counts = info.counts;
                self.marks = info.marks;
                self.pose = info.pose;
                self.picked = None;
            }
        }
        // a smaller raster while playing keeps the cadence; the full one
        // comes once it pauses
        // the raster follows the player's size on screen, within a pixel
        // budget: a tall canvas gets a narrower frame, so the per-frame
        // convert-and-upload stays cheap while it plays
        let aspect = (self.canvas.height / self.canvas.width.max(1.0)).clamp(0.1, 10.0);
        let budget = if self.playing { PLAYING_PX } else { PREVIEW_PX };
        let want = (self.player_size.0 * ctx.pixels_per_point()).round();
        let width = want.min((budget / aspect).sqrt()).max(64.0) as u16;
        let key = match self.source {
            Source::Animation => Key::Time {
                scene: self.scene,
                expression: self.expression,
                ms: (self.time * 1000.0) as u32,
                width,
                aa: !self.playing,
            },
            Source::Timelapse => {
                let step = self.pos.floor().max(0.0);
                Key::Step {
                    step: step as usize,
                    frac: ((self.pos - step) * 255.0).round() as u8,
                    width,
                    aa: !self.playing,
                }
            }
        };
        if self.asked != Some(key) {
            let _ = w.tx.send(key);
            self.asked = Some(key);
        }
    }
}

/// A frame the worker is asked for.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Key {
    Time { scene: usize, expression: Option<usize>, ms: u32, width: u16, aa: bool },
    Step { step: usize, frac: u8, width: u16, aa: bool },
}

/// A changed object drawn over the frame, in canvas pixels: line segments
/// (a wireframe or a grid), dots (grid points) or a pivot with its angle.
struct Mark {
    kind: Kind,
    segs: Vec<[f32; 4]>,
    dots: Vec<[f32; 2]>,
    pivot: Option<[f32; 3]>,
}

/// What a timelapse step changed, how big the model is, and the marks.
struct StepInfo {
    /// The parameters the step was posed at, by name, when the edit was
    /// made off the rest pose.
    pose: Vec<(String, f32)>,
    changes: Vec<Change>,
    counts: Counts,
    marks: Vec<(usize, Mark)>,
}

struct Frame {
    w: usize,
    h: usize,
    px: Vec<u8>,
    canvas: Canvas,
    values: Vec<f32>,
    part_opacity: Vec<f32>,
    step: Option<StepInfo>,
    /// With a timelapse's first frame: when each surviving step was recorded.
    stamps: Option<Vec<u64>>,
}

struct Worker {
    source: Source,
    tx: Sender<Key>,
    rx: Receiver<Frame>,
}

/// While playing the budget is smaller, for the cadence.
const PLAYING_PX: f32 = 800.0 * 800.0;

impl Worker {
    /// Start a preview thread for the source. For a timelapse the step
    /// timestamps come back with it, for the working-time readout.
    fn start(ctx: egui::Context, s: &crate::session::Session, source: Source) -> Option<Self> {
        let (tx, req) = channel::<Key>();
        let (out, rx) = channel::<Frame>();
        let mut art = s.preview_tex.clone();
        let pngs = s.texture_pngs.clone();
        let model = s.puppet.model().clone();
        let anims = s.animations.clone();
        let exprs = s.expressions.clone();
        let sidecar = match source {
            Source::Timelapse => {
                s.aom_path.as_ref().map(|p| crate::timelapse::Recorder::sidecar(p))
            }
            Source::Animation => None,
        };
        let spawned = std::thread::Builder::new().name("render-preview".into()).spawn(move || {
            let mut player =
                sidecar.as_deref().and_then(|p| crate::timelapse::Player::open(p).ok());
            // the steps that survive undo, their timestamps, and the
            // finished model's canvas, which every frame is drawn on so
            // the frame never changes shape mid-way
            let list: Vec<usize> = player.as_ref().map(|pl| pl.surviving()).unwrap_or_default();
            let mut stamps = player.as_ref().map(|pl| {
                list.iter().filter_map(|&i| pl.steps.get(i).map(|x| x.0)).collect::<Vec<u64>>()
            });
            let final_canvas = player
                .as_mut()
                .and_then(|pl| list.last().and_then(|&i| pl.model(i)))
                .map(|m| m.canvas);
            // the first frame comes with the thumbnail art, so there is
            // something to see at once; the pages are then decoded at the
            // size the largest paused frame draws at, and it is drawn again
            let canvas = final_canvas.unwrap_or(model.canvas);
            let aspect = (canvas.height / canvas.width.max(1.0)).clamp(0.1, 10.0);
            let sharp_w = ((PREVIEW_PX / aspect).sqrt() * 2.0) as u32;
            let mut sharp = false;
            let mut again = None;
            let mut puppet = ayatsuri_core::Puppet::new(model);
            // the step on show and the one after it, blended between
            let mut cur: Option<(usize, Model)> = None;
            let mut nxt: Option<(usize, Model)> = None;
            let mut info: (Vec<Change>, Counts) = (Vec::new(), Counts::default());
            loop {
                let mut key = match again.take() {
                    Some(k) => k,
                    None => match req.recv() {
                        Ok(k) => k,
                        Err(_) => break,
                    },
                };
                // only the newest request matters: the clock has moved on
                while let Ok(k) = req.try_recv() {
                    key = k;
                }
                let frame = match key {
                    Key::Time { scene, expression, ms, width, aa } => {
                        if let Some(anim) = anims.get(scene) {
                            pose(
                                &mut puppet,
                                anim,
                                ms as f32 / 1000.0,
                                expression.and_then(|i| exprs.get(i)),
                            );
                        }
                        puppet.update();
                        let (w, h, px) = crate::thumb::render_aa(
                            &puppet,
                            &art,
                            width as usize,
                            if aa { 2 } else { 1 },
                        );
                        let n = puppet.model().parameters.len();
                        let parts = puppet.model().parts.len();
                        Some(Frame {
                            w,
                            h,
                            px,
                            canvas: puppet.model().canvas,
                            values: (0..n).map(|i| puppet.value(i)).collect(),
                            part_opacity: (0..parts).map(|i| puppet.part_user_opacity(i)).collect(),
                            step: None,
                            stamps: None,
                        })
                    }
                    Key::Step { step, frac, width, aa } => {
                        let Some(pl) = &mut player else { continue };
                        let n = list.len();
                        if n == 0 {
                            continue;
                        }
                        let step = step.min(n - 1);
                        if cur.as_ref().is_none_or(|c| c.0 != step) {
                            // going forward the next becomes the current;
                            // anywhere else both are rebuilt
                            let a = match nxt.take() {
                                Some((i, m)) if i == step => Some(m),
                                _ => list.get(step).and_then(|&i| pl.model(i)),
                            };
                            let Some(a) = a else { continue };
                            let b = list.get(step + 1).and_then(|&i| pl.model(i));
                            info = match &b {
                                Some(b) => {
                                    (crate::model_diff::diff(&a, b), crate::model_diff::counts(b))
                                }
                                None => (Vec::new(), crate::model_diff::counts(&a)),
                            };
                            cur = Some((step, a));
                            nxt = b.map(|b| (step + 1, b));
                        }
                        let Some((_, a)) = &cur else { continue };
                        let mut model = match &nxt {
                            Some((_, b)) => crate::tween::blend(a, b, frac as f32 / 255.0),
                            None => a.clone(),
                        };
                        if let Some(c) = final_canvas {
                            model.canvas = c;
                        }
                        let mut pp = ayatsuri_core::Puppet::new(model);
                        // posed where the edit was made, so an eye moved
                        // at X 1, Y 1 is seen moving
                        let pose = pose_of(&info.0);
                        let names: Vec<(String, f32)> = pose
                            .iter()
                            .filter_map(|&(i, v)| {
                                pp.model().parameters.get(i).map(|p| (p.id.clone(), v))
                            })
                            .collect();
                        for (i, v) in pose {
                            pp.set_value(i, v);
                        }
                        pp.update();
                        let (w, h, px) = crate::thumb::render_aa(
                            &pp,
                            &art,
                            width as usize,
                            if aa { 2 } else { 1 },
                        );
                        let marks = marks(&pp, &info.0);
                        Some(Frame {
                            w,
                            h,
                            px,
                            canvas: pp.model().canvas,
                            values: Vec::new(),
                            part_opacity: Vec::new(),
                            step: Some(StepInfo {
                                pose: names,
                                changes: info.0.clone(),
                                counts: info.1,
                                marks,
                            }),
                            stamps: stamps.take(),
                        })
                    }
                };
                if let Some(f) = frame {
                    if out.send(f).is_err() {
                        break;
                    }
                    ctx.request_repaint();
                }
                if !sharp {
                    sharp = true;
                    art = textures(&pngs, canvas.width.max(1.0), sharp_w);
                    // the frame on show was drawn with the thumbnails
                    again = Some(key);
                }
            }
        });
        spawned.ok().map(|_| Worker { source, tx, rx })
    }
}

/// Where a step's edits were made: the parameter values of their
/// keyforms, later edits overriding earlier ones on the same parameter.
fn pose_of(changes: &[Change]) -> Vec<(usize, f32)> {
    let mut out: Vec<(usize, f32)> = Vec::new();
    for c in changes {
        for &(i, v) in &c.pose {
            match out.iter_mut().find(|(k, _)| *k == i) {
                Some(slot) => slot.1 = v,
                None => out.push((i, v)),
            }
        }
    }
    out
}

/// The changed objects' geometry at this step, in canvas pixels.
fn marks(pp: &ayatsuri_core::Puppet, changes: &[Change]) -> Vec<(usize, Mark)> {
    let c = pp.model().canvas;
    let px =
        |x: f32, y: f32| [c.origin_x + x * c.pixels_per_unit, c.origin_y + y * c.pixels_per_unit];
    let mut out = Vec::new();
    for (ci, ch) in changes.iter().enumerate() {
        let Some(i) = ch.index else { continue };
        let mut m = Mark { kind: ch.kind, segs: Vec::new(), dots: Vec::new(), pivot: None };
        match ch.what {
            What::Mesh => {
                let Some(g) = pp.mesh_geometry(i) else { continue };
                let (pts, _) = g.positions.as_chunks::<2>();
                let (tris, _) = g.indices.as_chunks::<3>();
                // each edge once, however many triangles share it
                let mut seen = std::collections::HashSet::new();
                for &[a, b, d] in tris {
                    for (u, v) in [(a, b), (b, d), (d, a)] {
                        let e = (u.min(v), u.max(v));
                        if !seen.insert(e) {
                            continue;
                        }
                        if let (Some(p0), Some(p1)) = (pts.get(e.0 as usize), pts.get(e.1 as usize))
                        {
                            let (q0, q1) = (px(p0[0], p0[1]), px(p1[0], p1[1]));
                            m.segs.push([q0[0], q0[1], q1[0], q1[1]]);
                        }
                    }
                }
            }
            What::Warp => {
                let Some((grid, rows, cols)) = pp.warp_grid(i) else { continue };
                let (pts, _) = grid.as_chunks::<2>();
                let stride = cols as usize + 1;
                let at = |r: usize, cc: usize| pts.get(r * stride + cc).map(|q| px(q[0], q[1]));
                for r in 0..=rows as usize {
                    for cc in 0..cols as usize {
                        if let (Some(a), Some(b)) = (at(r, cc), at(r, cc + 1)) {
                            m.segs.push([a[0], a[1], b[0], b[1]]);
                        }
                    }
                }
                for cc in 0..stride {
                    for r in 0..rows as usize {
                        if let (Some(a), Some(b)) = (at(r, cc), at(r + 1, cc)) {
                            m.segs.push([a[0], a[1], b[0], b[1]]);
                        }
                    }
                }
                m.dots = pts.iter().map(|q| px(q[0], q[1])).collect();
            }
            What::Rotation => {
                let Some(rot) = pp.rotation_state(i) else { continue };
                let o = px(rot.origin.x, rot.origin.y);
                m.pivot = Some([o[0], o[1], rot.angle]);
            }
            _ => continue,
        }
        out.push((ci, m));
    }
    out
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
    Pipe(
        std::process::Child,
        std::io::BufWriter<std::process::ChildStdin>,
        std::thread::JoinHandle<String>,
    ),
}

/// What ffmpeg said before it stopped, for the error.
fn ffmpeg_said(child: &mut std::process::Child, said: std::thread::JoinHandle<String>) -> String {
    let _ = child.wait();
    let said = said.join().unwrap_or_default();
    let tail: Vec<&str> = said.lines().filter(|l| !l.trim().is_empty()).collect();
    let n = tail.len();
    tail.into_iter().skip(n.saturating_sub(3)).collect::<Vec<_>>().join(" / ")
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
                // half the cores: the editor stays usable while it encodes
                let threads =
                    std::thread::available_parallelism().map_or(2, |n| n.get() / 2).max(1);
                let mut cmd = std::process::Command::new(&spec.ffmpeg);
                cmd.args(["-hide_banner", "-loglevel", "error", "-nostats"])
                    .args(["-y", "-f", "rawvideo", "-pix_fmt", "rgba"])
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
                            "-preset",
                            "faster",
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
                            "-row-mt",
                            "1",
                            "-cpu-used",
                            "3",
                        ]);
                    }
                    _ => {
                        cmd.args(["-c:v", "png"]);
                    }
                }
                cmd.arg("-threads")
                    .arg(threads.to_string())
                    .arg(&out)
                    .stdin(std::process::Stdio::piped())
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::piped());
                let mut child = cmd.spawn().map_err(|e| format!("ffmpeg: {e}"))?;
                let stdin = child.stdin.take().ok_or("ffmpeg: no stdin")?;
                let stderr = child.stderr.take().ok_or("ffmpeg: no stderr")?;
                // read as it comes, so a chatty ffmpeg never blocks on a full pipe
                let said = std::thread::Builder::new()
                    .name("ffmpeg-stderr".into())
                    .spawn(move || {
                        use std::io::Read;
                        let mut s = String::new();
                        let _ = std::io::BufReader::new(stderr).read_to_string(&mut s);
                        s
                    })
                    .map_err(|e| format!("ffmpeg: {e}"))?;
                Ok((
                    Sink::Pipe(child, std::io::BufWriter::with_capacity(1 << 20, stdin), said),
                    out,
                ))
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
            Sink::Pipe(_, stdin, _) => {
                use std::io::Write;
                stdin.write_all(&px).map_err(|e| format!("ffmpeg: {e}"))
            }
        }
    }

    /// A job that stopped or failed: the half-written file goes; frames
    /// already in a folder stay, each one is whole.
    fn abandon(self, path: &std::path::Path) {
        match self {
            Sink::Png(..) => {}
            Sink::Gif(gif, _) => {
                drop(gif);
                let _ = std::fs::remove_file(path);
            }
            Sink::Pipe(mut child, stdin, said) => {
                drop(stdin);
                let _ = child.kill();
                let _ = child.wait();
                let _ = said.join();
                let _ = std::fs::remove_file(path);
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
            Sink::Pipe(mut child, stdin, said) => {
                drop(stdin);
                let st = child.wait().map_err(|e| format!("ffmpeg: {e}"))?;
                if st.success() {
                    Ok(())
                } else {
                    let why = ffmpeg_said(&mut child, said);
                    Err(if why.is_empty() {
                        "ffmpeg could not encode the video".into()
                    } else {
                        format!("ffmpeg could not encode the video: {why}")
                    })
                }
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

/// Where a job's frames go, and how far it is.
struct Out<'a> {
    spec: &'a Spec,
    tx: &'a Sender<Progress>,
    stop: &'a std::sync::atomic::AtomicBool,
    sink: Option<Sink>,
    path: PathBuf,
    written: usize,
}

impl Out<'_> {
    fn push(&mut self, w: usize, h: usize, mut px: Vec<u8>, total: usize) -> Result<(), String> {
        if self.stop.load(std::sync::atomic::Ordering::Relaxed) {
            return Err(STOPPED.into());
        }
        if !self.spec.transparent || !self.spec.preset.alpha() {
            flatten(&mut px);
        }
        if self.sink.is_none() {
            let (s, o) = Sink::open(self.spec, w as u32, h as u32)?;
            self.sink = Some(s);
            self.path = o;
        }
        if let Some(s) = &mut self.sink
            && let Err(e) = s.frame(w as u32, h as u32, px)
        {
            // ffmpeg's own words, when it was ffmpeg that stopped
            return Err(match self.sink.take().map(Sink::finish) {
                Some(Err(why)) => why,
                _ => e,
            });
        }
        self.written += 1;
        let _ = self.tx.send(Progress::Frames(self.written, total));
        Ok(())
    }
}

/// Frames drawn by the GPU when there is one and it was asked for, else
/// on the CPU, which decodes its own pages when first needed.
struct Drawer {
    gpu: Option<crate::gpu_frames::Frames>,
    cpu: Option<Vec<ayatsuri_render::TexData>>,
    width: usize,
    ss: usize,
}

impl Drawer {
    fn new(spec: &Spec, width: usize) -> Self {
        let canvas_w = spec.model.canvas.width.max(1.0);
        let gpu = spec
            .gpu
            .then(|| crate::gpu_frames::Frames::new(textures(&spec.pngs, canvas_w, width as u32)));
        let gpu = gpu.flatten();
        if spec.gpu && gpu.is_none() {
            log::warn!("render: no GPU to draw frames with, drawing on the CPU");
        }
        Self { gpu, cpu: None, width, ss: if spec.aa { 2 } else { 1 } }
    }

    fn frame(&mut self, spec: &Spec, puppet: &ayatsuri_core::Puppet) -> (usize, usize, Vec<u8>) {
        if let Some(g) = &mut self.gpu {
            if let Some(f) = g.draw(puppet, self.width) {
                return f;
            }
            log::warn!("render: the GPU could not draw a frame, drawing on the CPU");
            self.gpu = None;
        }
        let (width, ss) = (self.width, self.ss);
        let tex = self.cpu.get_or_insert_with(|| {
            // pages at the detail the samples need
            let canvas_w = spec.model.canvas.width.max(1.0);
            textures(&spec.pngs, canvas_w, (width * ss) as u32)
        });
        crate::thumb::render_aa(puppet, tex, width, ss)
    }
}

fn run(
    spec: Spec,
    tx: &Sender<Progress>,
    stop: &std::sync::atomic::AtomicBool,
) -> Result<String, String> {
    let width = spec.width.clamp(16, 8192) as usize;
    let mut out = Out { spec: &spec, tx, stop, sink: None, path: PathBuf::new(), written: 0 };
    let mut draw = Drawer::new(&spec, width);
    match frames(&spec, &mut draw, &mut out) {
        Ok(()) => {
            match out.sink.take() {
                Some(s) => s.finish()?,
                None => return Err("nothing to render".into()),
            }
            Ok(trf(
                "{frames} frames to {path}",
                &[("frames", &out.written), ("path", &out.path.display())],
            ))
        }
        Err(e) => {
            if let Some(s) = out.sink.take() {
                s.abandon(&out.path);
            }
            Err(e)
        }
    }
}

/// Every frame of the source, in order, to `out`.
fn frames(spec: &Spec, draw: &mut Drawer, out: &mut Out<'_>) -> Result<(), String> {
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
                let (w, h, px) = draw.frame(spec, &puppet);
                out.push(w, h, px, frames)?;
            }
        }
        Source::Timelapse => {
            let path = spec.timelapse.as_ref().ok_or("this project has no timelapse yet")?;
            log::debug!("render: timelapse {}", path.display());
            let mut player = crate::timelapse::Player::open(path)?;
            let list = player.surviving();
            let n = list.len();
            if n == 0 {
                return Err("the timelapse is empty".into());
            }
            let fps = spec.fps.max(1) as f32;
            let sps = spec.sps.max(0.1);
            let hold = (spec.hold.max(0.0) * fps).round() as usize;
            let moving = ((n - 1) as f32 / sps * fps).ceil() as usize + 1;
            let total = moving + hold;
            let final_canvas = list.last().and_then(|&i| player.model(i)).map(|m| m.canvas);
            let mut cur: Option<(usize, Model)> = None;
            let mut nxt: Option<(usize, Model)> = None;
            let mut pose: Vec<(usize, f32)> = Vec::new();
            let mut last = None;
            for f in 0..moving {
                let pos = (f as f32 / fps * sps).min((n - 1) as f32);
                let step = pos.floor() as usize;
                if cur.as_ref().is_none_or(|c| c.0 != step) {
                    let a = match nxt.take() {
                        Some((i, m)) if i == step => Some(m),
                        _ => list.get(step).and_then(|&i| player.model(i)),
                    };
                    let a = a.ok_or_else(|| {
                        format!(
                            "step {} could not be read",
                            list.get(step).copied().unwrap_or(step)
                        )
                    })?;
                    nxt = list.get(step + 1).and_then(|&i| player.model(i)).map(|b| (step + 1, b));
                    pose = nxt
                        .as_ref()
                        .map(|(_, b)| pose_of(&crate::model_diff::diff(&a, b)))
                        .unwrap_or_default();
                    cur = Some((step, a));
                }
                let Some((_, a)) = &cur else { break };
                let mut model = match &nxt {
                    Some((_, b)) => crate::tween::blend(a, b, pos - step as f32),
                    None => a.clone(),
                };
                if let Some(c) = final_canvas {
                    model.canvas = c;
                }
                let mut puppet = ayatsuri_core::Puppet::new(model);
                for &(i, v) in &pose {
                    puppet.set_value(i, v);
                }
                puppet.update();
                let (w, h, px) = draw.frame(spec, &puppet);
                last = Some((w, h, px.clone()));
                out.push(w, h, px, total)?;
            }
            if let Some((w, h, px)) = last {
                for _ in 0..hold {
                    out.push(w, h, px.clone(), total)?;
                }
            }
        }
    }
    Ok(())
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
        win.worker = None;
        win.asked = None;
        win.changes.clear();
        win.marks.clear();
        win.picked = None;
        win.playing = false;
        win.shown = true;
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
                            j.took = j.started.map(|t| t.elapsed());
                            j.state = match r {
                                Ok(m) => {
                                    log::info!("render: {}: {m}", j.label());
                                    State::Done
                                }
                                Err(e) if e == STOPPED => {
                                    log::info!("render: {} stopped", j.label());
                                    State::Stopped
                                }
                                Err(e) => {
                                    log::error!("render: {}: {e}", j.label());
                                    State::Failed(e)
                                }
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
            j.started = Some(std::time::Instant::now());
            log::info!("render: {} started", j.label());
            let stop = j.stop.clone();
            let spawned = std::thread::Builder::new().name("render".into()).spawn(move || {
                let r = run(*spec, &tx, &stop);
                let _ = tx.send(Progress::Done(r));
            });
            if spawned.is_err() {
                log::error!("render: could not start the render thread");
                j.state = State::Failed("could not start the render thread".into());
                win.rx = None;
            }
        }

        if !win.shown {
            return;
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
                    let end = win.stamps.len().saturating_sub(1) as f32;
                    win.pos += dt * win.speed * win.sps;
                    if win.pos >= end {
                        if win.loop_on && end > 0.0 {
                            win.pos %= end;
                        } else {
                            win.pos = end;
                            win.playing = false;
                        }
                    }
                }
            }
            ui.ctx().request_repaint();
        }
        win.refresh_preview(ui.ctx(), s);
        if win.fullscreen {
            let screen = ui.ctx().content_rect();
            if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                win.fullscreen = false;
            }
            egui::Area::new(egui::Id::new("render-fullscreen"))
                .order(egui::Order::Foreground)
                .fixed_pos(screen.min)
                .show(ui.ctx(), |ui| {
                    egui::Frame::new().fill(egui::Color32::from_gray(12)).inner_margin(16.0).show(
                        ui,
                        |ui| {
                            ui.set_min_size(screen.size() - egui::vec2(32.0, 32.0));
                            let avail = screen.width() - 32.0;
                            let (pw, fh) = win.fit(avail, screen.height() - 32.0, s);
                            // the room left of a tall frame goes to the panel, then
                            // to margins either side, so the pair sits in the middle
                            let sw = (avail - pw - 14.0).clamp(SIDE_W, SIDE_W * 1.8);
                            let lead = ((avail - pw - 14.0 - sw) / 2.0).max(0.0);
                            ui.horizontal_top(|ui| {
                                ui.add_space(lead);
                                win.player_ui(ui, s, pw, fh);
                                ui.add_space(6.0);
                                win.side_panel(ui, s, STRIP_H + fh + RenderWin::bar_h(pw), sw);
                            });
                        },
                    );
                });
            return;
        }
        let mut open = true;
        let mut enqueue = false;
        let mut pick_folder = false;
        let has_timelapse =
            s.aom_path.as_ref().is_some_and(|p| crate::timelapse::Recorder::sidecar(p).exists());
        egui::Window::new(tr("Render")).collapsible(false).open(&mut open).default_size([1180.0, 640.0]).min_width(860.0).show(ui.ctx(), |ui| {
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
                    ui.set_width(170.0);
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
                    ui.set_width(300.0);
                    ui.strong(tr(win.preset.label()));
                    ui.weak(tr(win.preset.blurb()));
                    ui.add_space(4.0);
                    crate::windows::form(ui, "render-settings", |ui| {
                        match win.source {
                            Source::Animation => {
                                ui.label(tr("Scene"));
                                let name = s.animations.get(win.scene).map(|a| a.name.clone()).unwrap_or_default();
                                egui::ComboBox::from_id_salt("render-scene").selected_text(name).icon(no_icon).show_ui(ui, |ui| {
                                    for (i, a) in s.animations.iter().enumerate() {
                                        ui.selectable_value(&mut win.scene, i, &a.name);
                                    }
                                });
                                ui.end_row();
                                ui.label(tr("Expression"));
                                let ename = win.expression.and_then(|i| s.expressions.get(i)).map_or_else(|| tr("None").to_string(), |e| e.name.clone());
                                egui::ComboBox::from_id_salt("render-expr").selected_text(ename).icon(no_icon).show_ui(ui, |ui| {
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
                                ui.label(tr("Steps per second"));
                                ui.horizontal(|ui| {
                                    ui.add(egui::DragValue::new(&mut win.sps).range(0.5..=60.0).speed(0.1).max_decimals(1));
                                    let secs = win.stamps.len().saturating_sub(1) as f32 / win.sps.max(0.1) + win.hold.max(0.0);
                                    ui.weak(trf("≈ {length}", &[("length", &span((secs * 1000.0) as u64))]));
                                }).response.on_hover_text(tr("Each step eases in over its share of the second, the way it was dragged; raise this for a shorter video"));
                                ui.end_row();
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
                        ui.label(tr("Draw with"));
                        ui.horizontal(|ui| {
                            ui.selectable_value(&mut win.gpu, true, tr("GPU"))
                                .on_hover_text(tr("Frames are drawn by the graphics card and read back; the processor only encodes."));
                            ui.selectable_value(&mut win.gpu, false, tr("CPU"))
                                .on_hover_text(tr("Frames are drawn by the processor, on every core. Slow for a big canvas."));
                        });
                        ui.end_row();
                        if !win.gpu {
                            ui.label(tr("Smooth edges"));
                            ui.checkbox(&mut win.aa, "")
                                .on_hover_text(tr("Draws each frame at twice the size and averages it down: four times the work."));
                            ui.end_row();
                        }
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
                    // the queue gets exactly its room under the player, never
                    // more: a column that asked for more each frame would grow
                    // the window without end
                    let room = win.queue_h();
                    let (pw, fh) = win.fit(ui.available_width(), ui.available_height() - room, s);
                    ui.horizontal_top(|ui| {
                        win.player_ui(ui, s, pw, fh);
                        ui.add_space(6.0);
                        win.side_panel(ui, s, STRIP_H + fh + RenderWin::bar_h(pw), SIDE_W);
                    });
                    let w = pw + 6.0 + SIDE_W;
                    let (slot, _) = ui.allocate_exact_size(egui::vec2(w, room), egui::Sense::hover());
                    let mut q = ui.new_child(
                        egui::UiBuilder::new()
                            .max_rect(slot.shrink2(egui::vec2(0.0, 4.0)))
                            .layout(egui::Layout::top_down(egui::Align::Min)),
                    );
                    q.set_clip_rect(slot);
                    win.queue_ui(&mut q);
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
            let spec = Spec {
                source: win.source,
                preset: win.preset,
                fps: win.fps,
                width: win.width,
                transparent: win.transparent && win.preset.alpha(),
                work_area: win.work_area,
                hold: win.hold,
                sps: win.sps,
                physics: self.physics_on,
                gpu: win.gpu,
                aa: win.aa,
                folder: folder.clone(),
                name: stem.clone(),
                model: s.puppet.model().clone(),
                pngs: s.texture_pngs.clone(),
                anim,
                expression: win.expression.and_then(|i| s.expressions.get(i).cloned()),
                timelapse: s.aom_path.as_ref().map(|p| crate::timelapse::Recorder::sidecar(p)),
                ffmpeg: win.ffmpeg.clone().unwrap_or_else(|| PathBuf::from("ffmpeg")),
            };
            win.queue.push(Job {
                name: stem,
                preset: win.preset,
                folder,
                done: 0,
                total: 0,
                state: State::Queued(Box::new(spec)),
                stop: Default::default(),
                started: None,
                took: None,
            });
        }
        if pick_folder {
            self.pick_async(crate::panels::PickFor::RenderFolder, || {
                rfd::FileDialog::new().set_title(tr("Render into which folder?")).pick_folder()
            });
        }
        if !open && let Some(win) = &mut self.render_win {
            // the queue goes on: jobs run and finish with the window shut,
            // and are there when it opens again
            win.shown = false;
            win.playing = false;
            win.fullscreen = false;
            win.worker = None;
            win.asked = None;
        }
    }
}

/// The side panel's width; the player takes the rest of the column.
const SIDE_W: f32 = 236.0;
/// The player never shrinks below this.
const MIN_PLAYER_W: f32 = 320.0;

/// A drop-down with no arrow: the text is the control.
fn no_icon(_: &Ui, _: egui::Rect, _: &egui::style::WidgetVisuals, _: bool) {}
const STRIP_H: f32 = 22.0;
const BAR_H: f32 = 34.0;
/// Under this width the clock and toggles move to a second bar row.
const NARROW: f32 = 560.0;
/// Changes listed for one step before the list says "and more".
const MAX_CHANGES: usize = 80;

/// MM:SS.ss.
fn clock(t: f32) -> String {
    let t = t.max(0.0);
    let secs = t.floor() as u64;
    format!("{:02}:{:02}.{:02}", secs / 60, secs % 60, ((t - t.floor()) * 100.0) as u32)
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
    /// The player's width and frame height for the room on offer: the
    /// frame keeps the canvas shape and the player shrinks to fit the
    /// height when the window is short.
    fn fit(&mut self, avail_w: f32, avail_h: f32, s: &crate::session::Session) -> (f32, f32) {
        let c = s.puppet.model().canvas;
        let aspect = (c.height / c.width.max(1.0)).max(0.1);
        let mut pw = (avail_w - SIDE_W - 14.0).max(MIN_PLAYER_W);
        let mut fh = pw * aspect;
        // a short window: the frame shrinks and the player narrows with it,
        // which may give the bar a second row, so the height is settled twice
        for _ in 0..2 {
            let max_fh = (avail_h - STRIP_H - Self::bar_h(pw)).max(160.0);
            if fh > max_fh {
                fh = max_fh;
                pw = (fh / aspect).clamp(MIN_PLAYER_W, pw);
            }
        }
        self.player_size = (pw, fh);
        (pw, fh)
    }

    /// The bar's height at a player width: one row, or two when narrow.
    fn bar_h(pw: f32) -> f32 {
        if pw < NARROW { BAR_H + 24.0 } else { BAR_H }
    }

    /// The player: a title strip, the frame on dark with the output's
    /// background under the model, and a transport bar.
    fn player_ui(&mut self, ui: &mut Ui, s: &crate::session::Session, pw: f32, fh: f32) {
        use egui::{Align2, Color32, FontId, Rect, Sense, pos2, vec2};
        let canvas = if self.canvas.width > 0.0 { self.canvas } else { s.puppet.model().canvas };
        let (panel, panel_resp) =
            ui.allocate_exact_size(vec2(pw, STRIP_H + fh + Self::bar_h(pw)), Sense::hover());
        let p = ui.painter_at(panel);
        p.rect_filled(panel, 8.0, Color32::from_gray(20));
        // title strip: what plays, at what size and rate
        let strip = Rect::from_min_size(panel.min, vec2(pw, STRIP_H));
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
        let frame = Rect::from_min_size(pos2(panel.min.x, panel.min.y + STRIP_H), vec2(pw, fh));
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
        let narrow = pw < NARROW;
        let bar = Rect::from_min_size(
            pos2(panel.min.x + 6.0, frame.max.y + 5.0),
            vec2(pw - 12.0, BAR_H - 10.0),
        );
        let mut b = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(bar)
                .layout(egui::Layout::left_to_right(egui::Align::Center)),
        );
        b.spacing_mut().item_spacing.x = 2.0;
        let n = self.stamps.len();
        let last = n.saturating_sub(1);
        let anim = s.animations.get(self.scene);
        let (dur, fps) = anim.map_or((0.0, 30.0), |a| (a.duration.max(0.0), a.fps));
        if icon_button(&mut b, egui_phosphor::regular::SKIP_BACK, false, tr("Start")).clicked() {
            self.time = 0.0;
            self.pos = 0.0;
        }
        let play =
            if self.playing { egui_phosphor::regular::PAUSE } else { egui_phosphor::regular::PLAY };
        if icon_button(&mut b, play, false, tr("Play or pause (Space)")).clicked() {
            self.playing = !self.playing;
            self.last_tick = None;
        }
        if icon_button(&mut b, egui_phosphor::regular::SKIP_FORWARD, false, tr("End")).clicked() {
            self.time = dur;
            self.pos = last as f32;
            self.playing = false;
        }
        b.add_space(4.0);
        // the clock and the toggles share the row when there is room, else
        // they take a second row under the scrubber
        let right_w = match (narrow, self.source) {
            (true, _) => 0.0,
            (false, Source::Animation) => 172.0,
            (false, Source::Timelapse) => 256.0,
        };
        let sw = (bar.width() - 84.0 - right_w - 8.0).max(60.0);
        match self.source {
            Source::Animation => {
                let frac = if dur > 0.0 { self.time / dur } else { 0.0 };
                if let Some(f) = scrubber(&mut b, frac, sw) {
                    self.time = f * dur;
                    self.playing = false;
                }
            }
            Source::Timelapse => {
                let frac = if last > 0 { self.pos / last as f32 } else { 0.0 };
                if let Some(f) = scrubber(&mut b, frac, sw) {
                    self.pos = f * last as f32;
                    self.playing = false;
                }
            }
        }
        let mut row2;
        let r: &mut Ui = if narrow {
            let second = Rect::from_min_size(
                pos2(bar.min.x, bar.max.y + 2.0),
                vec2(bar.width(), BAR_H - 10.0),
            );
            row2 = ui.new_child(
                egui::UiBuilder::new()
                    .max_rect(second)
                    .layout(egui::Layout::left_to_right(egui::Align::Center)),
            );
            row2.spacing_mut().item_spacing.x = 2.0;
            row2.add_space(4.0);
            &mut row2
        } else {
            b.add_space(6.0);
            &mut b
        };
        let clock_text = egui::RichText::new(match self.source {
            Source::Animation => format!("{} / {}", clock(self.time), clock(dur)),
            Source::Timelapse => {
                // the clock runs at the playback rate
                let rate = (self.sps * self.speed).max(0.01);
                format!("{} / {}", clock(self.pos / rate), clock(last as f32 / rate))
            }
        })
        .monospace()
        .size(11.5)
        .color(Color32::from_gray(200));
        let clock_tip = match self.source {
            Source::Animation => trf(
                "Frame {frame} at {fps} fps",
                &[("frame", &((self.time * fps).round() as u32)), ("fps", &fps)],
            ),
            Source::Timelapse => trf(
                "Step {step} of {steps}",
                &[("step", &(self.pos.floor() as usize + usize::from(n > 0))), ("steps", &n)],
            ),
        };
        r.label(clock_text).on_hover_text(clock_tip);
        r.add_space(2.0);
        if self.source == Source::Timelapse {
            let speed_txt = if self.speed >= 1.0 {
                format!("{}×", self.speed as u32)
            } else {
                format!("{}×", self.speed)
            };
            if text_button(r, &speed_txt, tr("Speed, on top of the steps per second")).clicked() {
                self.speed = match self.speed {
                    x if x < 0.5 => 0.5,
                    x if x < 1.0 => 1.0,
                    x if x < 2.0 => 2.0,
                    x if x < 4.0 => 4.0,
                    x if x < 8.0 => 8.0,
                    _ => 0.25,
                };
            }
            if icon_button(
                r,
                egui_phosphor::regular::EYE,
                self.show_changes,
                tr("Highlight what changed at this step: meshes in orange, new ones in green, deformer grids"),
            )
            .clicked()
            {
                self.show_changes = !self.show_changes;
            }
        }
        if icon_button(r, egui_phosphor::regular::REPEAT, self.loop_on, tr("Loop")).clicked() {
            self.loop_on = !self.loop_on;
        }
        let full_icon = if self.fullscreen {
            egui_phosphor::regular::CORNERS_IN
        } else {
            egui_phosphor::regular::CORNERS_OUT
        };
        if icon_button(r, full_icon, self.fullscreen, tr("Full screen (Esc to leave)")).clicked() {
            self.fullscreen = !self.fullscreen;
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
                        // to the next whole step, or back to the last one
                        let at =
                            if dir > 0.0 { self.pos.floor() + by } else { self.pos.ceil() - by };
                        self.pos = at.clamp(0.0, last as f32);
                    }
                }
            }
        }
    }

    /// Draw this step's changed objects over the frame: meshes as
    /// wireframes, warp grids as grids, rotation deformers as their pivot.
    fn overlay(&self, p: &egui::Painter, at: egui::Rect, c: Canvas) {
        use egui::{Stroke, pos2};
        let pal = crate::theme::pal();
        let k = at.width() / c.width.max(1.0);
        let to = |x: f32, y: f32| pos2(at.min.x + x * k, at.min.y + y * k);
        for (ci, m) in &self.marks {
            if self.picked.is_some_and(|pk| pk != *ci) {
                continue;
            }
            let col = match m.kind {
                Kind::Added => pal.warp_green,
                Kind::Edited => pal.accent,
                Kind::Removed => pal.rotation_red,
            };
            let stroke =
                Stroke::new(if m.dots.is_empty() { 1.0 } else { 1.3 }, col.gamma_multiply(0.8));
            for sg in &m.segs {
                p.line_segment([to(sg[0], sg[1]), to(sg[2], sg[3])], stroke);
            }
            for d in &m.dots {
                p.circle_filled(to(d[0], d[1]), 2.0, col);
            }
            if let Some([x, y, angle]) = m.pivot {
                let o = to(x, y);
                let a = angle.to_radians();
                let tip = o + egui::vec2(a.sin(), -a.cos()) * 26.0;
                p.circle_stroke(o, 7.0, Stroke::new(2.0, col));
                p.circle_filled(o, 2.5, col);
                p.line_segment([o, tip], Stroke::new(2.0, col));
                p.circle_filled(tip, 3.0, col);
            }
        }
    }

    /// Beside the player: the parameters a scene drives, or what a timelapse
    /// step changed.
    fn side_panel(&mut self, ui: &mut Ui, s: &crate::session::Session, h: f32, w: f32) {
        ui.allocate_ui_with_layout(
            egui::vec2(w, h),
            egui::Layout::top_down(egui::Align::Min),
            |ui| {
                ui.set_min_height(h);
                ui.set_max_width(w);
                match self.source {
                    Source::Animation => self.params_panel(ui, s, h, w),
                    Source::Timelapse => self.changes_panel(ui, h, w),
                }
            },
        );
    }

    fn params_panel(&self, ui: &mut Ui, s: &crate::session::Session, h: f32, w: f32) {
        let anim = s.animations.get(self.scene);
        let model = s.puppet.model();
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
            ui.set_min_width(w - 14.0);
            for i in order {
                let Some(p) = model.parameters.get(i) else { continue };
                let v = self.values.get(i).copied().unwrap_or(p.default);
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
                    let v = self.part_opacity.get(i).copied().unwrap_or(1.0);
                    value_row(ui, id, v, 0.0, 1.0, 1.0, 2, true);
                }
            }
        });
    }

    fn changes_panel(&mut self, ui: &mut Ui, h: f32, w: f32) {
        let pal = crate::theme::pal();
        let t0 = self.stamps.first().copied().unwrap_or(0);
        let now = self.stamps.get(self.pos.floor() as usize).copied().unwrap_or(t0);
        let end = self.stamps.last().copied().unwrap_or(t0);
        let (elapsed, total) = (now.saturating_sub(t0), end.saturating_sub(t0));
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
            ui.strong(tr("Changed"));
            if !self.pose.is_empty() {
                let at: Vec<String> =
                    self.pose.iter().map(|(n, v)| format!("{n} {v:.2}")).collect();
                ui.weak(egui::RichText::new(trf("at {pose}", &[("pose", &at.join(", "))])).small())
                    .on_hover_text(tr("The parameters the rigger had set while making this edit"));
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if !self.changes.is_empty() {
                    ui.weak(egui::RichText::new(self.changes.len().to_string()).small());
                }
            });
        });
        egui::ScrollArea::vertical().id_salt("render-changes").max_height(h - 112.0).show(
            ui,
            |ui| {
                ui.set_min_width(w - 14.0);
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

    /// The height the queue takes under the player: its header and up to
    /// two jobs, the rest scrolls.
    fn queue_h(&self) -> f32 {
        8.0 + 28.0 + self.queue_body_h() + 8.0
    }

    fn queue_body_h(&self) -> f32 {
        if self.queue.is_empty() { 22.0 } else { self.queue.len().min(2) as f32 * (JOB_H + 8.0) }
    }

    /// The jobs, one card each: name and format, a bar, and a line that
    /// says how far it is or how it ended. A running job can be stopped,
    /// a finished one opened or cleared.
    fn queue_ui(&mut self, ui: &mut Ui) {
        use egui::{Align2, FontId, Rect, Sense, pos2, vec2};
        let pal = crate::theme::pal();
        ui.horizontal(|ui| {
            ui.strong(tr("Queue"));
            let finished = self
                .queue
                .iter()
                .any(|j| matches!(j.state, State::Done | State::Failed(_) | State::Stopped));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if finished && ui.small_button(tr("Clear finished")).clicked() {
                    self.queue.retain(|j| matches!(j.state, State::Queued(_) | State::Running));
                }
            });
        });
        if self.queue.is_empty() {
            ui.weak(tr("Nothing queued yet."));
            return;
        }
        let mut remove = None;
        let mut reveal = None;
        egui::ScrollArea::vertical()
            .id_salt("render-queue")
            .min_scrolled_height(0.0)
            .max_height(self.queue_body_h())
            .show(ui, |ui| {
                for (i, j) in self.queue.iter().enumerate() {
                    let w = ui.available_width();
                    let (card, _) = ui.allocate_exact_size(vec2(w, JOB_H), Sense::hover());
                    let p = ui.painter_at(card);
                    p.rect(
                        card,
                        6.0,
                        pal.panel_bg,
                        egui::Stroke::new(1.0, pal.border),
                        egui::StrokeKind::Inside,
                    );
                    let inner = card.shrink2(vec2(12.0, 10.0));
                    // the name, then the format in a small outlined tag
                    let name = p.text(
                        inner.min,
                        Align2::LEFT_TOP,
                        &j.name,
                        FontId::proportional(13.5),
                        pal.text,
                    );
                    let tag_font = FontId::monospace(10.5);
                    let tag_w = p
                        .layout_no_wrap(
                            tr(j.preset.label()).to_string(),
                            tag_font.clone(),
                            pal.text_faint,
                        )
                        .size()
                        .x;
                    let tag = Rect::from_min_size(
                        pos2(name.max.x + 10.0, name.center().y - 9.0),
                        vec2(tag_w + 12.0, 18.0),
                    );
                    p.rect(
                        tag,
                        4.0,
                        egui::Color32::TRANSPARENT,
                        egui::Stroke::new(1.0, pal.border),
                        egui::StrokeKind::Inside,
                    );
                    p.text(
                        tag.center(),
                        Align2::CENTER_CENTER,
                        tr(j.preset.label()),
                        tag_font,
                        pal.text_faint,
                    );
                    // the buttons, from the right end of the first line
                    let mut right = inner.max.x;
                    let mut pill =
                        |ui: &mut Ui, key: &str, width: f32, tip: &str| -> (Rect, egui::Response) {
                            let r = Rect::from_min_size(
                                pos2(right - width, name.center().y - 11.0),
                                vec2(width, 22.0),
                            );
                            right -= width + 6.0;
                            let resp = ui.interact(r, ui.id().with((i, key)), Sense::click());
                            let fill = if resp.hovered() {
                                egui::Color32::from_white_alpha(18)
                            } else {
                                egui::Color32::TRANSPARENT
                            };
                            p.rect(
                                r,
                                4.0,
                                fill,
                                egui::Stroke::new(1.0, pal.border),
                                egui::StrokeKind::Inside,
                            );
                            (r, resp.on_hover_text(tip))
                        };
                    let ink = |hovered: bool| if hovered { pal.text } else { pal.text_faint };
                    match &j.state {
                        State::Running => {
                            let (r, resp) = pill(ui, "stop", 62.0, tr("Stop"));
                            let col = ink(resp.hovered());
                            // a filled square: the stop sign everyone knows
                            p.rect_filled(
                                Rect::from_center_size(
                                    pos2(r.min.x + 13.0, r.center().y),
                                    vec2(9.0, 9.0),
                                ),
                                1.5,
                                col,
                            );
                            p.text(
                                pos2(r.min.x + 23.0, r.center().y),
                                Align2::LEFT_CENTER,
                                tr("Stop"),
                                FontId::proportional(11.5),
                                col,
                            );
                            if resp.clicked() {
                                j.stop.store(true, std::sync::atomic::Ordering::Relaxed);
                            }
                        }
                        State::Done => {
                            let (r, resp) = pill(ui, "clear", 26.0, tr("Clear"));
                            p.text(
                                r.center(),
                                Align2::CENTER_CENTER,
                                egui_phosphor::regular::X,
                                FontId::proportional(13.0),
                                ink(resp.hovered()),
                            );
                            if resp.clicked() {
                                remove = Some(i);
                            }
                            let (r, resp) = pill(ui, "open", 26.0, tr("Open the folder"));
                            p.text(
                                r.center(),
                                Align2::CENTER_CENTER,
                                egui_phosphor::regular::FOLDER_SIMPLE,
                                FontId::proportional(13.0),
                                ink(resp.hovered()),
                            );
                            if resp.clicked() {
                                reveal = Some(j.folder.clone());
                            }
                        }
                        State::Queued(_) | State::Failed(_) | State::Stopped => {
                            let (r, resp) = pill(ui, "clear", 26.0, tr("Clear"));
                            p.text(
                                r.center(),
                                Align2::CENTER_CENTER,
                                egui_phosphor::regular::X,
                                FontId::proportional(13.0),
                                ink(resp.hovered()),
                            );
                            if resp.clicked() {
                                remove = Some(i);
                            }
                        }
                    }
                    // the bar, then the line under it
                    let f = if j.total > 0 {
                        (j.done as f32 / j.total as f32).clamp(0.0, 1.0)
                    } else {
                        0.0
                    };
                    let bar = Rect::from_min_size(
                        pos2(inner.min.x, inner.min.y + 28.0),
                        vec2(inner.width(), 4.0),
                    );
                    p.rect_filled(bar, 2.0, pal.track);
                    let (fill, col) = match &j.state {
                        State::Queued(_) => (0.0, pal.track),
                        State::Running => (f, pal.accent),
                        State::Done => (1.0, pal.warp_green),
                        State::Failed(_) => (f.max(0.08), pal.rotation_red),
                        State::Stopped => (f, pal.text_faint),
                    };
                    if fill > 0.0 {
                        p.rect_filled(
                            Rect::from_min_size(bar.min, vec2(bar.width() * fill, 4.0)),
                            2.0,
                            col,
                        );
                    }
                    let line = pos2(inner.min.x, inner.min.y + 38.0);
                    let small = FontId::proportional(11.0);
                    let right_small = FontId::monospace(11.0);
                    match &j.state {
                        State::Queued(_) => {
                            p.text(line, Align2::LEFT_TOP, tr("Waiting"), small, pal.text_faint);
                        }
                        State::Running => {
                            let left = match (j.started, j.done) {
                                (Some(t), d) if d > 0 && j.total > d => Some(
                                    t.elapsed().as_secs_f32() / d as f32 * (j.total - d) as f32,
                                ),
                                _ => None,
                            };
                            let mut words = trf(
                                "{done} of {total} frames",
                                &[("done", &j.done), ("total", &j.total)],
                            );
                            if let Some(left) = left {
                                words.push_str("   ");
                                words.push_str(&trf("{time} left", &[("time", &span_short(left))]));
                            }
                            p.text(line, Align2::LEFT_TOP, words, small, pal.text_faint);
                            p.text(
                                pos2(inner.max.x, line.y),
                                Align2::RIGHT_TOP,
                                format!("{:.0}%", f * 100.0),
                                right_small,
                                pal.text,
                            );
                        }
                        State::Done => {
                            let took =
                                j.took.map(|d| span_short(d.as_secs_f32())).unwrap_or_default();
                            let words = trf(
                                "{frames} frames in {time}",
                                &[("frames", &j.done), ("time", &took)],
                            );
                            let dot = p.text(
                                line,
                                Align2::LEFT_TOP,
                                egui_phosphor::regular::CHECK,
                                small.clone(),
                                pal.warp_green,
                            );
                            p.text(
                                pos2(dot.max.x + 6.0, line.y),
                                Align2::LEFT_TOP,
                                words,
                                small,
                                pal.text_faint,
                            );
                        }
                        State::Failed(e) => {
                            let dot = p.text(
                                line,
                                Align2::LEFT_TOP,
                                egui_phosphor::regular::WARNING,
                                small.clone(),
                                pal.rotation_red,
                            );
                            let clip = Rect::from_min_max(pos2(dot.max.x + 6.0, line.y), inner.max);
                            p.with_clip_rect(clip).text(
                                pos2(dot.max.x + 6.0, line.y),
                                Align2::LEFT_TOP,
                                e,
                                small,
                                pal.text_faint,
                            );
                            ui.interact(clip, ui.id().with((i, "why")), Sense::hover())
                                .on_hover_text(e);
                        }
                        State::Stopped => {
                            p.text(line, Align2::LEFT_TOP, tr("Stopped"), small, pal.text_faint);
                        }
                    }
                    ui.add_space(8.0);
                }
            });
        if let Some(i) = remove {
            self.queue.remove(i);
        }
        if let Some(folder) = reveal {
            open_folder(&folder);
        }
    }
}

/// A job card's height.
const JOB_H: f32 = 70.0;

/// Show a folder in the system's file browser.
fn open_folder(path: &std::path::Path) {
    #[cfg(target_os = "linux")]
    let cmd = "xdg-open";
    #[cfg(target_os = "macos")]
    let cmd = "open";
    #[cfg(target_os = "windows")]
    let cmd = "explorer";
    if let Err(e) = std::process::Command::new(cmd)
        .arg(path)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
    {
        log::warn!("could not open {}: {e}", path.display());
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Render a project's timelapse the way the Render window would, into
    /// the folder named by AYATSURI_EXPORT_OUT. Needs a real .aom with a
    /// timelapse beside it, so it only runs when asked for.
    #[test]
    #[ignore]
    fn exports_a_real_timelapse() {
        let aom = PathBuf::from(std::env::var("AYATSURI_EXPORT_AOM").unwrap());
        let out = PathBuf::from(std::env::var("AYATSURI_EXPORT_OUT").unwrap());
        let preset = match std::env::var("AYATSURI_EXPORT_PRESET").as_deref() {
            Ok("mp4") => Preset::Mp4,
            Ok("gif") => Preset::Gif,
            Ok("png") => Preset::PngSeq,
            Ok("mov") => Preset::Mov,
            _ => Preset::Webm,
        };
        let bytes = std::fs::read(&aom).unwrap();
        let payload = ayatsuri_format::read::read(&bytes).unwrap();
        let spec = Spec {
            source: Source::Timelapse,
            preset,
            fps: 30,
            width: 1920,
            transparent: true,
            work_area: false,
            hold: 1.0,
            sps: 6.0,
            physics: false,
            gpu: std::env::var("AYATSURI_EXPORT_CPU").is_err(),
            aa: true,
            folder: out,
            name: "timelapse-test".into(),
            model: payload.model,
            pngs: payload.textures.into_iter().map(|b| b.into_vec()).collect(),
            anim: None,
            expression: None,
            timelapse: Some(crate::timelapse::Recorder::sidecar(&aom)),
            ffmpeg: ayatsuri_update::ffmpeg::program().unwrap_or_else(|| PathBuf::from("ffmpeg")),
        };
        let (tx, rx) = channel();
        let t = std::time::Instant::now();
        let r = run(spec, &tx, &std::sync::atomic::AtomicBool::new(false));
        let mut frames = 0;
        while let Ok(p) = rx.try_recv() {
            if let Progress::Frames(a, _) = p {
                frames = a;
            }
        }
        eprintln!("{frames} frames in {:?}: {r:?}", t.elapsed());
        assert!(r.is_ok(), "{r:?}");
    }
}
