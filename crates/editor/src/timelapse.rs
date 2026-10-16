//! Timelapse: every step of building a model, kept beside it.
//!
//! After each edit (and undo or redo) the model is serialized and compared
//! with the step before in 4 KB blocks; only the blocks that changed are
//! stored, compressed, appended to `<model>.aom.timelapse`. Every
//! `KEY_EVERY` steps a whole model is stored, so scrubbing never replays
//! more than that many deltas. Textures are not recorded: a timelapse plays
//! with the model's current art.
//!
//! ```text
//!   file:  "AYTL1\n"  then per step:
//!          u32 length | u64 unix ms | deflate( step )
//!   step:  u8 kind | u64 base (kinds 2 and 3) | u32 total length
//!          kind 0 or 2, whole: the bytes
//!          kind 1 or 3, delta: u32 blocks changed, then per block: u32 index, u32 len, bytes
//!          base: the state the step was recorded on, hashed (0 for none),
//!          so two editors writing one file stay on their own chains
//! ```

use std::io::Write;
use std::path::{Path, PathBuf};

use ayatsuri_format::Model;

const MAGIC: &[u8] = b"AYTL1\n";
const BLOCK: usize = 4096;
const KEY_EVERY: usize = 50;

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}

fn put_u32(out: &mut Vec<u8>, v: usize) {
    out.extend_from_slice(&(v as u32).to_le_bytes());
}

fn get_u32(b: &[u8], at: &mut usize) -> Option<usize> {
    let s: [u8; 4] = b.get(*at..*at + 4)?.try_into().ok()?;
    *at += 4;
    Some(u32::from_le_bytes(s) as usize)
}

/// One step's record: the whole model, or the blocks that changed from
/// `before`. `base` names the state it was recorded on.
fn encode_step(before: Option<&[u8]>, now: &[u8], whole: bool, base: u64) -> Vec<u8> {
    let mut out = Vec::new();
    match before {
        Some(prev) if !whole => {
            out.push(3);
            out.extend_from_slice(&base.to_le_bytes());
            put_u32(&mut out, now.len());
            let changed: Vec<usize> = (0..now.len().div_ceil(BLOCK))
                .filter(|&i| {
                    let r = i * BLOCK..((i + 1) * BLOCK).min(now.len());
                    prev.get(r.clone()) != now.get(r)
                })
                .collect();
            put_u32(&mut out, changed.len());
            for i in changed {
                let r = i * BLOCK..((i + 1) * BLOCK).min(now.len());
                let bytes = now.get(r).unwrap_or_default();
                put_u32(&mut out, i);
                put_u32(&mut out, bytes.len());
                out.extend_from_slice(bytes);
            }
        }
        _ => {
            out.push(2);
            out.extend_from_slice(&base.to_le_bytes());
            put_u32(&mut out, now.len());
            out.extend_from_slice(now);
        }
    }
    miniz_oxide::deflate::compress_to_vec(&out, 6)
}

/// One step inflated: whole or delta, the state it was recorded on (not
/// kept by the first files), and where its payload starts.
struct Raw {
    whole: bool,
    base: Option<u64>,
    raw: Vec<u8>,
    at: usize,
}

fn inflate(packed: &[u8]) -> Option<Raw> {
    let raw = miniz_oxide::inflate::decompress_to_vec(packed).ok()?;
    let kind = *raw.first()?;
    let mut at = 1;
    let base = if kind >= 2 {
        let b: [u8; 8] = raw.get(at..at + 8)?.try_into().ok()?;
        at += 8;
        Some(u64::from_le_bytes(b))
    } else {
        None
    };
    Some(Raw { whole: kind % 2 == 0, base, raw, at })
}

/// Apply one step to the bytes of the step it was recorded on. Also says
/// which blocks it touched, or None for a whole step.
fn apply(before: &[u8], step: &Raw) -> Option<(Vec<u8>, Option<Vec<usize>>)> {
    let mut at = step.at;
    let total = get_u32(&step.raw, &mut at)?;
    if step.whole {
        return step.raw.get(at..at + total).map(|b| (b.to_vec(), None));
    }
    let mut out = before.to_vec();
    out.resize(total, 0);
    let n = get_u32(&step.raw, &mut at)?;
    let mut touched = Vec::with_capacity(n);
    for _ in 0..n {
        let i = get_u32(&step.raw, &mut at)?;
        let len = get_u32(&step.raw, &mut at)?;
        let bytes = step.raw.get(at..at + len)?;
        at += len;
        let start = i * BLOCK;
        out.get_mut(start..start + len)?.copy_from_slice(bytes);
        touched.push(i);
    }
    Some((out, Some(touched)))
}

fn decode_step(before: &[u8], packed: &[u8]) -> Option<Vec<u8>> {
    apply(before, &inflate(packed)?).map(|(b, _)| b)
}

fn hash_block(b: &[u8]) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    b.hash(&mut h);
    h.finish()
}

fn block_at(bytes: &[u8], k: usize) -> u64 {
    hash_block(bytes.get(k * BLOCK..((k + 1) * BLOCK).min(bytes.len())).unwrap_or(&[]))
}

fn block_hashes(bytes: &[u8]) -> Vec<u64> {
    (0..bytes.len().div_ceil(BLOCK)).map(|k| block_at(bytes, k)).collect()
}

/// One number for a state, from its block hashes: equal states hash
/// equal, and a step only rehashes the blocks it touched.
fn state_of(blocks: &[u64]) -> u64 {
    blocks
        .iter()
        .enumerate()
        .fold(0u64, |acc, (k, h)| acc.wrapping_add(h.wrapping_mul(k as u64 * 2 + 1)))
}

fn state_hash(bytes: &[u8]) -> u64 {
    state_of(&block_hashes(bytes))
}

/// The block hashes after a step: the touched blocks and the tail, which
/// changes with the length, are hashed again; a whole step hashes all.
fn rehashed(mut blocks: Vec<u64>, bytes: &[u8], touched: Option<Vec<usize>>) -> Vec<u64> {
    let Some(touched) = touched else { return block_hashes(bytes) };
    let count = bytes.len().div_ceil(BLOCK);
    let had = blocks.len();
    blocks.resize(count, 0);
    let tail = had.min(count).saturating_sub(1);
    for k in touched.into_iter().chain(tail..count) {
        if let Some(slot) = blocks.get_mut(k) {
            *slot = block_at(bytes, k);
        }
    }
    blocks
}

/// Records steps as the model changes. Before the model has a file, steps
/// wait in memory and go out with the first save.
#[derive(Default)]
pub struct Recorder {
    path: Option<PathBuf>,
    last: Option<Vec<u8>>,
    /// The state of `last`, named by the next step as what it builds on.
    last_hash: u64,
    steps: usize,
    waiting: Vec<(u64, Vec<u8>)>,
}

impl Recorder {
    pub fn sidecar(model_path: &Path) -> PathBuf {
        let mut name = model_path.as_os_str().to_os_string();
        name.push(".timelapse");
        PathBuf::from(name)
    }

    /// Start recording for a model file (or none yet). An existing timelapse
    /// is continued: its step count decides where the next whole step goes.
    pub fn open(model_path: Option<&Path>) -> Self {
        let path = model_path.map(Self::sidecar);
        let steps = path.as_deref().and_then(|p| index(p).ok()).map_or(0, |i| i.len());
        Self { path, last: None, last_hash: 0, steps, waiting: Vec::new() }
    }

    /// Record the model as it is now. Same bytes as the last step: nothing.
    pub fn record(&mut self, model: &Model) {
        let Ok(now) = rmp_serde::to_vec(model) else { return };
        if self.last.as_deref() == Some(now.as_slice()) {
            return;
        }
        // the first step after opening is whole: the file's last step may
        // be from a different state of the model
        let whole = self.last.is_none() || self.steps.is_multiple_of(KEY_EVERY);
        let packed = encode_step(self.last.as_deref(), &now, whole, self.last_hash);
        self.last_hash = state_hash(&now);
        self.last = Some(now);
        self.steps += 1;
        self.waiting.push((now_ms(), packed));
        self.flush();
    }

    /// The model now has a file: steps so far go to its timelapse.
    pub fn saved_as(&mut self, model_path: &Path) {
        let path = Self::sidecar(model_path);
        if self.path.as_deref() != Some(path.as_path()) {
            self.path = Some(path);
            self.flush();
        }
    }

    fn flush(&mut self) {
        let Some(path) = &self.path else { return };
        if self.waiting.is_empty() {
            return;
        }
        let fresh = !path.exists();
        let file = std::fs::OpenOptions::new().create(true).append(true).open(path);
        let Ok(mut file) = file else {
            log::warn!("timelapse: can't write {}", path.display());
            return;
        };
        let mut out = Vec::new();
        if fresh {
            out.extend_from_slice(MAGIC);
        }
        for (ms, packed) in self.waiting.drain(..) {
            put_u32(&mut out, packed.len());
            out.extend_from_slice(&ms.to_le_bytes());
            out.extend_from_slice(&packed);
        }
        if let Err(e) = file.write_all(&out) {
            log::warn!("timelapse: {e}");
        }
    }
}

/// Where each step sits in a timelapse file: (unix ms, offset, length).
pub fn index(path: &Path) -> Result<Vec<(u64, usize, usize)>, String> {
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    if !bytes.starts_with(MAGIC) {
        return Err("not an Ayatsuri timelapse".into());
    }
    let mut at = MAGIC.len();
    let mut out = Vec::new();
    while at < bytes.len() {
        let Some(len) = get_u32(&bytes, &mut at) else { break };
        let Some(ms) = bytes.get(at..at + 8) else { break };
        let ms = u64::from_le_bytes(ms.try_into().unwrap_or_default());
        at += 8;
        if at + len > bytes.len() {
            break;
        }
        out.push((ms, at, len));
        at += len;
    }
    Ok(out)
}

/// A timelapse opened for playback: steps are rebuilt on demand, back
/// along the steps they were recorded on to a whole step, then forward.
pub struct Player {
    bytes: Vec<u8>,
    pub steps: Vec<(u64, usize, usize)>,
    /// Per step: the step it was recorded on, whether it holds the whole
    /// model, its state, and whether it could be read at all.
    parent: Vec<Option<usize>>,
    whole: Vec<bool>,
    state: Vec<u64>,
    readable: Vec<bool>,
    cached: Option<(usize, Vec<u8>)>,
}

/// How many chain ends are kept in memory while a file is linked: one
/// per editor that wrote it at the same time, with room to spare.
const HEADS: usize = 4;

impl Player {
    pub fn open(path: &Path) -> Result<Self, String> {
        let steps = index(path)?;
        let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
        if steps.is_empty() {
            return Err("this timelapse has no steps yet".into());
        }
        let n = steps.len();
        let mut p = Self {
            bytes,
            steps,
            parent: vec![None; n],
            whole: vec![false; n],
            state: vec![0; n],
            readable: vec![false; n],
            cached: None,
        };
        p.link();
        Ok(p)
    }

    fn packed(&self, i: usize) -> Option<&[u8]> {
        let &(_, at, len) = self.steps.get(i)?;
        self.bytes.get(at..at + len)
    }

    /// Work out which step each step was recorded on. Steps name the
    /// state they build on; a step from the first files is tried on the
    /// live chain ends newest first, and a whole one continues from the
    /// latest step it equals. Two editors writing one file give two
    /// chains, and an undo lands on a state seen before.
    fn link(&mut self) {
        let mut by_state: std::collections::HashMap<u64, usize> = std::collections::HashMap::new();
        // chain ends nothing has been recorded on yet: step, bytes, block hashes
        let mut heads: Vec<(usize, Vec<u8>, Vec<u64>)> = Vec::new();
        for i in 0..self.steps.len() {
            let Some(step) = self.packed(i).and_then(inflate) else { continue };
            let (parent, bytes, blocks) = if step.whole {
                let Some((b, _)) = apply(&[], &step) else { continue };
                let blocks = block_hashes(&b);
                let h = state_of(&blocks);
                // named, or the latest step it equals (opened again after
                // a save), or else the newest chain end
                let parent = step
                    .base
                    .filter(|&b| b != 0)
                    .and_then(|b| by_state.get(&b).copied())
                    .or_else(|| by_state.get(&h).copied())
                    .or_else(|| heads.last().map(|x| x.0));
                if let Some(j) = parent {
                    heads.retain(|x| x.0 != j);
                }
                (parent, b, blocks)
            } else {
                let mut pick = None;
                match step.base {
                    Some(b) => {
                        if let Some(k) = heads.iter().rposition(|x| self.state.get(x.0) == Some(&b))
                        {
                            let (j, hb, hbl) = heads.remove(k);
                            if let Some((nb, t)) = apply(&hb, &step) {
                                pick = Some((j, rehashed(hbl, &nb, t), nb));
                            }
                        } else if let Some(&j) = by_state.get(&b)
                            && let Some(base) = self.bytes_at(j)
                            && let Some((nb, _)) = apply(&base, &step)
                        {
                            pick = Some((j, block_hashes(&nb), nb));
                        }
                    }
                    None => {
                        for k in (0..heads.len()).rev() {
                            let Some((nb, t)) = heads.get(k).and_then(|x| apply(&x.1, &step))
                            else {
                                continue;
                            };
                            if heads.len() > 1 && rmp_serde::from_slice::<Model>(&nb).is_err() {
                                continue;
                            }
                            let (j, _, hbl) = heads.remove(k);
                            pick = Some((j, rehashed(hbl, &nb, t), nb));
                            break;
                        }
                    }
                }
                let Some((j, blocks, nb)) = pick else { continue };
                (Some(j), nb, blocks)
            };
            let h = state_of(&blocks);
            if let (Some(p), Some(w), Some(st), Some(r)) = (
                self.parent.get_mut(i),
                self.whole.get_mut(i),
                self.state.get_mut(i),
                self.readable.get_mut(i),
            ) {
                *p = parent;
                *w = step.whole;
                *st = h;
                *r = true;
            }
            by_state.insert(h, i);
            heads.push((i, bytes, blocks));
            if heads.len() > HEADS {
                heads.remove(0);
            }
        }
        self.cached = None;
    }

    /// The steps that play, in order: the chain the file ends on, without
    /// the steps an undo took back. A step that brings the model back to
    /// the state of an earlier step is an undo: it and everything after
    /// that earlier step are dropped. A redo lands as a fresh step.
    pub fn surviving(&self) -> Vec<usize> {
        let Some(end) = (0..self.steps.len()).rev().find(|&i| self.readable.get(i) == Some(&true))
        else {
            return Vec::new();
        };
        let mut chain = vec![end];
        let mut at = end;
        while let Some(&Some(p)) = self.parent.get(at) {
            chain.push(p);
            at = p;
        }
        chain.reverse();
        let mut keep: Vec<usize> = Vec::new();
        let mut seen: std::collections::HashMap<u64, usize> = std::collections::HashMap::new();
        for i in chain {
            let state = self.state.get(i).copied().unwrap_or(0);
            match seen.get(&state).copied() {
                Some(at) if at < keep.len() => {
                    keep.truncate(at + 1);
                    seen.retain(|_, v| *v <= at);
                }
                _ => {
                    seen.insert(state, keep.len());
                    keep.push(i);
                }
            }
        }
        keep
    }

    /// The bytes of step `i`: back along the parents to a whole step or
    /// the cached one, then forward.
    fn bytes_at(&mut self, i: usize) -> Option<Vec<u8>> {
        let mut path = Vec::new();
        let mut at = i;
        let mut start = None;
        loop {
            if self.readable.get(at) != Some(&true) {
                return None;
            }
            if let Some((c, b)) = &self.cached
                && *c == at
            {
                start = Some(b.clone());
                break;
            }
            path.push(at);
            if self.whole.get(at) == Some(&true) {
                break;
            }
            at = (*self.parent.get(at)?)?;
        }
        let mut bytes = match start {
            Some(b) => b,
            None => decode_step(&[], self.packed(path.pop()?)?)?,
        };
        for k in path.into_iter().rev() {
            bytes = decode_step(&bytes, self.packed(k)?)?;
        }
        self.cached = Some((i, bytes.clone()));
        Some(bytes)
    }

    /// The model at step `i`.
    pub fn model(&mut self, i: usize) -> Option<Model> {
        let i = i.min(self.steps.len().saturating_sub(1));
        let bytes = self.bytes_at(i)?;
        rmp_serde::from_slice(&bytes).ok()
    }
}

/// How an export is going: frames written of the total, then the outcome.
enum Progress {
    Frames(usize, usize),
    Done(Result<String, String>),
}

/// Window > Timelapse: scrub or play the steps, export them.
pub struct View {
    path: PathBuf,
    player: Player,
    at: usize,
    playing: bool,
    speed: f32,
    carry: f32,
    shown: Option<usize>,
    tex: Option<egui::TextureHandle>,
    width: u32,
    export: Option<std::sync::mpsc::Receiver<Progress>>,
    progress: (usize, usize),
    outcome: Option<Result<String, String>>,
}

/// Render one step: the model at its parameters' defaults.
fn render_step(
    model: Model,
    textures: &[ayatsuri_render::TexData],
    width: usize,
) -> (usize, usize, Vec<u8>) {
    let mut puppet = ayatsuri_core::Puppet::new(model);
    puppet.update();
    crate::thumb::render(&puppet, textures, width)
}

/// Onto white, for formats without transparency.
fn flatten(px: &mut [u8]) {
    for p in px.chunks_mut(4) {
        if let [r, g, b, a] = p {
            let k = *a as u32;
            for c in [r, g, b] {
                *c = ((*c as u32 * k + 255 * (255 - k)) / 255) as u8;
            }
            *a = 255;
        }
    }
}

fn export(
    path: PathBuf,
    folder: PathBuf,
    stem: String,
    pngs: Vec<Vec<u8>>,
    width: u32,
    tx: std::sync::mpsc::Sender<Progress>,
) -> Result<String, String> {
    let mut player = Player::open(&path)?;
    // pages only need the detail the export width shows: decoded one at a
    // time and scaled down, so a 4096 atlas set never sits in memory whole
    let canvas_w = player.model(0).map_or(1.0, |m| m.canvas.width.max(1.0));
    let k = (width as f32 / canvas_w * 1.25).min(1.0);
    let textures: Vec<ayatsuri_render::TexData> = pngs
        .iter()
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
        .collect();
    let n = player.steps.len();
    let stride = n.div_ceil(600).max(1);
    let picks: Vec<usize> = (0..n).step_by(stride).chain(std::iter::once(n - 1)).collect();
    let mut picks = picks;
    picks.dedup();
    let fps = 30u32;
    let hold = (fps * 2) as usize;
    let total = picks.len() + hold;
    let frames_dir = folder.join(format!("{stem}-timelapse"));
    std::fs::create_dir_all(&frames_dir).map_err(|e| e.to_string())?;
    let gif_w = width.min(480) as usize;
    let gif_path = folder.join(format!("{stem}-timelapse.gif"));
    let gif_file = std::fs::File::create(&gif_path).map_err(|e| e.to_string())?;
    let mut gif =
        image::codecs::gif::GifEncoder::new_with_speed(std::io::BufWriter::new(gif_file), 10);
    gif.set_repeat(image::codecs::gif::Repeat::Infinite).map_err(|e| e.to_string())?;
    let delay = image::Delay::from_numer_denom_ms(1000, fps);
    let mut written = 0usize;
    let mut last: Option<(usize, usize, Vec<u8>, image::RgbaImage)> = None;
    for (k, &i) in picks.iter().enumerate() {
        let model = player.model(i).ok_or("a step could not be read")?;
        let (w, h, px) = render_step(model.clone(), &textures, width as usize);
        let img = image::RgbaImage::from_raw(w as u32, h as u32, px.clone()).ok_or("bad frame")?;
        img.save(frames_dir.join(format!("frame_{:05}.png", k + 1))).map_err(|e| e.to_string())?;
        let (gw, gh, mut gpx) = render_step(model, &textures, gif_w);
        flatten(&mut gpx);
        let gimg = image::RgbaImage::from_raw(gw as u32, gh as u32, gpx).ok_or("bad frame")?;
        gif.encode_frame(image::Frame::from_parts(gimg.clone(), 0, 0, delay))
            .map_err(|e| e.to_string())?;
        last = Some((w, h, px, gimg));
        written += 1;
        let _ = tx.send(Progress::Frames(written, total));
    }
    if let Some((w, h, px, gimg)) = last {
        let img = image::RgbaImage::from_raw(w as u32, h as u32, px).ok_or("bad frame")?;
        for j in 0..hold {
            let at = picks.len() + j + 1;
            img.save(frames_dir.join(format!("frame_{at:05}.png"))).map_err(|e| e.to_string())?;
            gif.encode_frame(image::Frame::from_parts(gimg.clone(), 0, 0, delay))
                .map_err(|e| e.to_string())?;
            written += 1;
            let _ = tx.send(Progress::Frames(written, total));
        }
    }
    drop(gif);
    let mut made = format!("{} frames, {}", written, gif_path.display());
    // MP4 through ffmpeg when it is installed: nothing patent-encumbered ships
    let mp4 = folder.join(format!("{stem}-timelapse.mp4"));
    let program = ayatsuri_update::ffmpeg::program().unwrap_or_else(|| PathBuf::from("ffmpeg"));
    let ffmpeg = std::process::Command::new(program)
        .arg("-y")
        .args(["-framerate", &fps.to_string()])
        .arg("-i")
        .arg(frames_dir.join("frame_%05d.png"))
        .args(["-vf", "pad=ceil(iw/2)*2:ceil(ih/2)*2:color=white,format=yuv420p"])
        .arg(&mp4)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();
    match ffmpeg {
        Ok(st) if st.success() => made.push_str(&format!(", {}", mp4.display())),
        _ => made.push_str(" (install ffmpeg from the Render window for an MP4 too)"),
    }
    Ok(made)
}

impl View {
    /// The timelapse of a model file.
    pub fn open(model_path: &Path) -> Result<Self, String> {
        let path = Recorder::sidecar(model_path);
        let player = Player::open(&path)?;
        let last = player.steps.len().saturating_sub(1);
        Ok(Self {
            path,
            player,
            at: last,
            playing: false,
            speed: 12.0,
            carry: 0.0,
            shown: None,
            tex: None,
            width: 1080,
            export: None,
            progress: (0, 0),
            outcome: None,
        })
    }

    /// The window. False once it is closed.
    pub fn show(&mut self, ctx: &egui::Context, session: &crate::session::Session) -> bool {
        use crate::i18n::{tr, trf};
        let n = self.player.steps.len();
        if self.playing {
            self.carry += ctx.input(|i| i.stable_dt) * self.speed;
            let step = self.carry.floor() as usize;
            self.carry -= step as f32;
            self.at = (self.at + step).min(n.saturating_sub(1));
            if self.at + 1 >= n {
                self.playing = false;
            }
            ctx.request_repaint();
        }
        if self.shown != Some(self.at)
            && let Some(model) = self.player.model(self.at)
        {
            let (w, h, px) = render_step(model, &session.preview_tex, 480);
            let img = egui::ColorImage::from_rgba_unmultiplied([w, h], &px);
            match &mut self.tex {
                Some(t) => t.set(img, egui::TextureOptions::LINEAR),
                None => {
                    self.tex =
                        Some(ctx.load_texture("timelapse", img, egui::TextureOptions::LINEAR))
                }
            }
            self.shown = Some(self.at);
        }
        if let Some(rx) = &self.export {
            while let Ok(p) = rx.try_recv() {
                match p {
                    Progress::Frames(a, b) => self.progress = (a, b),
                    Progress::Done(r) => {
                        self.outcome = Some(r);
                        self.export = None;
                        break;
                    }
                }
            }
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }
        let mut open = true;
        egui::Window::new(tr("Timelapse")).collapsible(false).open(&mut open).default_width(520.0).show(ctx, |ui| {
            let span = match (self.player.steps.first(), self.player.steps.last()) {
                (Some(a), Some(b)) => (b.0.saturating_sub(a.0)) / 60_000,
                _ => 0,
            };
            ui.weak(trf("{steps} steps over {minutes} minutes of work", &[("steps", &n), ("minutes", &span)]));
            if let Some(t) = &self.tex {
                let size = t.size_vec2();
                let k = (ui.available_width() / size.x).min(360.0 / size.y).min(1.0);
                ui.vertical_centered(|ui| {
                    egui::Frame::new().fill(egui::Color32::WHITE).show(ui, |ui| {
                        ui.image((t.id(), size * k));
                    });
                });
            }
            ui.horizontal(|ui| {
                let label = if self.playing { tr("Pause") } else { tr("Play") };
                if ui.button(label).clicked() {
                    if !self.playing && self.at + 1 >= n {
                        self.at = 0;
                    }
                    self.playing = !self.playing;
                }
                let mut at = self.at as u32;
                let last = n.saturating_sub(1) as u32;
                if ui.add(egui::Slider::new(&mut at, 0..=last).text(tr("step"))).changed() {
                    self.at = at as usize;
                    self.playing = false;
                }
            });
            ui.horizontal(|ui| {
                ui.label(tr("Speed"));
                ui.add(egui::Slider::new(&mut self.speed, 1.0..=120.0).logarithmic(true).suffix(" /s"));
            });
            ui.separator();
            ui.horizontal(|ui| {
                ui.label(tr("Export width"));
                ui.add(egui::DragValue::new(&mut self.width).range(64..=4096).suffix(" px"));
                let busy = self.export.is_some();
                if ui.add_enabled(!busy, egui::Button::new(tr("Export"))).on_hover_text(tr(
                    "PNG frames and a GIF next to the model, and an MP4 when ffmpeg is installed",
                )).clicked()
                    && let Some(model_path) = session.aom_path.clone()
                {
                    let folder = model_path.parent().map(Path::to_path_buf).unwrap_or_default();
                    let stem = model_path
                        .file_stem()
                        .map(|s| s.to_string_lossy().into_owned())
                        .unwrap_or_else(|| "model".into());
                    let (tx, rx) = std::sync::mpsc::channel();
                    let (path, pngs, width) = (self.path.clone(), session.texture_pngs.clone(), self.width);
                    let spawned = std::thread::Builder::new().name("timelapse-export".into()).spawn(move || {
                        let r = export(path, folder, stem, pngs, width, tx.clone());
                        let _ = tx.send(Progress::Done(r));
                    });
                    match spawned {
                        Ok(_) => {
                            self.export = Some(rx);
                            self.outcome = None;
                            self.progress = (0, 0);
                        }
                        Err(e) => self.outcome = Some(Err(e.to_string())),
                    }
                }
            });
            if self.export.is_some() {
                let (a, b) = self.progress;
                let f = if b > 0 { a as f32 / b as f32 } else { 0.0 };
                ui.add(egui::ProgressBar::new(f).text(trf("{done} of {total} frames", &[("done", &a), ("total", &b)])));
            }
            match &self.outcome {
                Some(Ok(what)) => {
                    ui.weak(trf("Exported: {what}", &[("what", what)]));
                }
                Some(Err(e)) => {
                    ui.colored_label(crate::theme::pal().rotation_red, trf("Export failed: {error}", &[("error", e)]));
                }
                None => {}
            }
        });
        open
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn steps_come_back_as_they_were_recorded() {
        let dir = std::env::temp_dir().join(format!("ay-tl-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let aom = dir.join("m.aom");
        let mut rec = Recorder::open(None);
        let mut m = Model::new();
        let mut widths = Vec::new();
        for w in 0..120 {
            m.canvas.width = w as f32;
            rec.record(&m);
            widths.push(w as f32);
            if w == 10 {
                // the first save sends what was waiting
                rec.saved_as(&aom);
            }
        }
        rec.record(&m); // unchanged: no step
        let mut p = Player::open(&Recorder::sidecar(&aom)).unwrap();
        assert_eq!(p.steps.len(), widths.len());
        for i in [0, 1, 49, 50, 51, 99, 119, 5, 77] {
            assert_eq!(p.model(i).unwrap().canvas.width, widths[i], "step {i}");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn undone_steps_do_not_survive() {
        let dir = std::env::temp_dir().join(format!("ay-tl-undo-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let aom = dir.join("m.aom");
        let mut rec = Recorder::open(Some(&aom));
        let mut m = Model::new();
        // widths: 1, 2, 3, undo to 2, undo to 1, redo to 2, then 7
        for w in [1.0, 2.0, 3.0, 2.0, 1.0, 2.0, 7.0] {
            m.canvas.width = w;
            rec.record(&m);
        }
        let p = Player::open(&Recorder::sidecar(&aom)).unwrap();
        // step 0 (1), step 5 (the redo to 2, a fresh step), step 6 (7)
        assert_eq!(p.surviving(), vec![0, 5, 6]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn two_editors_on_one_file_keep_their_own_chains() {
        let dir = std::env::temp_dir().join(format!("ay-tl-two-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let aom = dir.join("m.aom");
        let mut a = Recorder::open(Some(&aom));
        let mut m = Model::new();
        for w in [1.0, 2.0, 3.0] {
            m.canvas.width = w;
            a.record(&m);
        }
        // a second editor opens the file as it was saved, at width 1,
        // and records two edits of its own: steps 3 (whole), 4 and 5
        let mut b = Recorder::open(Some(&aom));
        let mut mb = Model::new();
        for w in [1.0, 10.0, 11.0] {
            mb.canvas.width = w;
            b.record(&mb);
        }
        // the first editor goes on from its own step 2: steps 6 and 7
        for w in [4.0, 5.0] {
            m.canvas.width = w;
            a.record(&m);
        }
        let mut p = Player::open(&Recorder::sidecar(&aom)).unwrap();
        assert_eq!(p.surviving(), vec![0, 1, 2, 6, 7]);
        for (i, w) in [(3, 1.0), (4, 10.0), (5, 11.0), (6, 4.0), (7, 5.0), (2, 3.0), (1, 2.0)] {
            assert_eq!(p.model(i).unwrap().canvas.width, w, "step {i}");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn reopening_a_saved_file_continues_its_chain() {
        let dir = std::env::temp_dir().join(format!("ay-tl-reopen-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let aom = dir.join("m.aom");
        let mut m = Model::new();
        let mut a = Recorder::open(Some(&aom));
        for w in [1.0, 2.0] {
            m.canvas.width = w;
            a.record(&m);
        }
        drop(a);
        // opened again later: the first record is the model as saved
        let mut a = Recorder::open(Some(&aom));
        for w in [2.0, 3.0] {
            m.canvas.width = w;
            a.record(&m);
        }
        let p = Player::open(&Recorder::sidecar(&aom)).unwrap();
        // step 2 equals step 1, so it continues from it and does not play twice
        assert_eq!(p.surviving(), vec![0, 1, 3]);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
