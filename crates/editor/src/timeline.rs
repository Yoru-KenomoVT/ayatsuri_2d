//! The Timeline palette: a dope sheet in the After Effects layout over the
//! current scene's curves, or the layer bars of a composition. Names on the left (the scene, parameter folders,
//! parameters, parts), lanes of keys on the right under a frame ruler, a
//! navigator for zoom and scroll, a work area bar that bounds playback, the
//! playhead, and the palette's own undo of key edits.

use std::collections::{BTreeMap, BTreeSet};

use ayatsuri_format::{Animation, Composition, CurveTarget, Interp, Key, Layer, LayerSource};
use egui::{
    Align, Align2, Color32, CursorIcon, FontId, Id, Layout, Modifiers, Painter, Pos2, Rect, Sense,
    Shape, Stroke, StrokeKind, Ui, UiBuilder, pos2, vec2,
};

use crate::i18n::tr;
use crate::session::Session;
use crate::theme;

const HEADER_H: f32 = 48.0;
const COLHDR_H: f32 = 20.0;
const FOOTER_H: f32 = 22.0;
const ROW_H: f32 = 19.0;
/// A layer's row, taller so its bar and waveform read.
const LAYER_ROW_H: f32 = 23.0;
const RULER_H: f32 = 24.0;
const NAV_H: f32 = 8.0;
const MAX_PPS: f32 = 4000.0;
const KEY_PX: f32 = 11.0;
const MIN_SPLIT: f32 = 190.0;
const VALUE_W: f32 = 62.0;
const KEYS_W: f32 = 50.0;

/// The playhead and transport the editor owns; the palette drives them.
pub struct Transport<'a> {
    pub time: &'a mut f32,
    pub playing: &'a mut bool,
    pub autokey: &'a mut bool,
    /// The time the model was last sampled at; NaN asks for a resample.
    pub applied: &'a mut f32,
    pub sel_param: &'a mut Option<usize>,
}

/// What the palette asks the editor to do outside its own state.
pub enum Request {
    NewScene,
    NewComp,
    /// File > Import Audio, from a composition's context menu.
    ImportAudio,
    /// Animation > Edit Animation Details, for what is on show.
    EditDetails,
}

/// What the palette is showing: a scene's curves or a composition's layers.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum View {
    Scene(usize),
    Comp(usize),
}

/// A scene or composition as it was before an edit, for the palette's undo.
enum Snap {
    /// A scene, and the composition it was being edited through, if any.
    Scene(usize, Animation, Option<usize>),
    Comp(usize, Composition),
}

/// The fold key of the top row, which hides everything under it.
const TOP_FOLD: &str = "\u{0}top";

fn layer_fold(li: usize) -> String {
    format!("\u{1}layer{li}")
}

/// Seconds to pixels for one frame of the palette.
#[derive(Clone, Copy)]
struct TMap {
    x0: f32,
    start: f32,
    pps: f32,
}

impl TMap {
    fn x(&self, t: f32) -> f32 {
        self.x0 + (t - self.start) * self.pps
    }
    fn t(&self, x: f32) -> f32 {
        self.start + (x - self.x0) / self.pps
    }
}

/// One line of the tree on the left.
#[derive(Clone)]
enum RowKind {
    Scene,
    Folder(String),
    Param(usize),
    Part(usize),
    /// A composition's layer, by index.
    Layer(usize),
    /// One curve of a layer's scene, shown read-only under it.
    LayerCurve(usize),
    /// One channel of a stereo sound layer, opened under it.
    AudioChannel(usize, usize),
}

#[derive(Clone)]
struct Row {
    kind: RowKind,
    depth: u8,
    label: String,
    curve: Option<(CurveTarget, String)>,
    color: Option<Color32>,
    /// The curves a folder sums up in its lane.
    children: Vec<(CurveTarget, String)>,
}

/// One half of a key glyph: the shape says how the segment on that side
/// moves, the way After Effects draws them.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Half {
    Linear,
    Hold,
    Bezier,
}

/// Auto smooth and linear both draw the diamond, as Live2D does; a hold
/// is a square and a free bezier an hourglass.
fn half_of(i: Interp) -> Half {
    match i {
        Interp::Auto | Interp::Linear => Half::Linear,
        Interp::Stepped | Interp::InverseStepped => Half::Hold,
        Interp::Bezier { .. } => Half::Bezier,
    }
}

/// One key, named by what its curve drives and the frame it sits on, so
/// a selection survives the keys being redrawn.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Debug)]
struct KeyRef {
    target: u8,
    id: String,
    frame: i64,
}

fn tgt(t: CurveTarget) -> u8 {
    match t {
        CurveTarget::Parameter => 0,
        CurveTarget::PartOpacity => 1,
        CurveTarget::Model => 2,
    }
}

/// A drag of the selected keys: where it started and how many frames it
/// has moved them so far, polled from the pointer each frame because the
/// key widgets change ids as they move.
struct KeyDrag {
    origin_x: f32,
    anchor: f32,
    applied: i64,
}

/// A drag of a layer bar: its body, or one of its ends, from where the
/// layer sat when the pointer went down.
struct LayerDrag {
    li: usize,
    part: LayerPart,
    origin_x: f32,
    start: f32,
    in_point: f32,
    out_point: f32,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum LayerPart {
    Body,
    Left,
    Right,
}

/// Where a layer's span lands after a drag of `dx` seconds: the body
/// slides it, the left end trims the in point and moves the start with it
/// so the content stays put, the right end trims the out point (past the
/// source's end only when it loops).
fn layer_span_after(
    d: &LayerDrag,
    dx: f32,
    speed: f32,
    len: f32,
    looped: bool,
    frame_s: f32,
) -> (f32, f32, f32) {
    let snap = |t: f32| (t / frame_s).round() * frame_s;
    let speed = speed.max(1e-3);
    match d.part {
        LayerPart::Body => (snap(d.start + dx).max(0.0), d.in_point, d.out_point),
        LayerPart::Left => {
            let end = d.start + (d.out_point - d.in_point) / speed;
            let ns = snap(d.start + dx).clamp(0.0, (end - frame_s).max(0.0));
            let nin = (d.in_point + (ns - d.start) * speed).max(0.0);
            let ns = d.start + (nin - d.in_point) / speed;
            (ns, nin, d.out_point)
        }
        LayerPart::Right => {
            let end = d.start + (d.out_point - d.in_point) / speed;
            let ne = snap(end + dx).max(d.start + frame_s);
            let mut nout = d.in_point + (ne - d.start) * speed;
            if !looped {
                nout = nout.min(len.max(d.in_point + frame_s));
            }
            (d.start, d.in_point, nout.max(d.in_point + frame_s * speed))
        }
    }
}

pub struct Timeline {
    /// Pixels per second; None fits the whole scene.
    pps: Option<f32>,
    /// The time at the left edge of the lanes.
    start: f32,
    scroll_y: f32,
    split: f32,
    /// Folders the user opened or closed by hand; the rest open when they
    /// hold an animated parameter.
    folded: BTreeMap<String, bool>,
    show_all: bool,
    editing: Option<(Id, String)>,
    scrub: Option<f32>,
    selected: BTreeSet<KeyRef>,
    drag: Option<KeyDrag>,
    box_from: Option<Pos2>,
    /// The layer picked in a composition.
    sel_layer: Option<usize>,
    /// A layer row being dragged up or down the stack by its name.
    layer_reorder: Option<usize>,
    layer_drag: Option<LayerDrag>,
    /// The composition to go back to after opening one of its layers.
    back: Option<usize>,
    /// In a composition, the layer whose scene keys are selected and
    /// edited in place; its keys are named in the scene's own frames.
    edit_layer: Option<usize>,
    /// The composition on show while a scene snapshot is kept, so undo
    /// comes back to it.
    in_comp: Option<usize>,
    /// The last click landed in the palette, so Delete and Escape are its
    /// and its shortcuts go before the editor's.
    pub focused: bool,
    /// Shortcut actions and frame edits asked for since the last frame.
    pending: Vec<String>,
    /// Frames to open up (positive) or take out at the playhead.
    frames: i64,
    /// Scenes and compositions as they were before each edit; one drag
    /// folds into one step by its merge key.
    undo: Vec<Snap>,
    redo: Vec<Snap>,
    merge: Option<&'static str>,
    /// The graph editor in place of the key lanes.
    pub graph: bool,
    /// The value range held still while something in the graph is dragged.
    graph_range: Option<(f32, f32)>,
    grab: Option<GraphGrab>,
}

/// A drag of the selected keys in the graph: frames and value moved so far.
struct GraphGrab {
    press: Pos2,
    applied_f: i64,
    applied_v: f32,
}

const GRAPH_COLORS: [Color32; 6] = [
    Color32::from_rgb(0xe8, 0xc8, 0x40),
    Color32::from_rgb(0xd0, 0x70, 0xe0),
    Color32::from_rgb(0x40, 0xc8, 0xc8),
    Color32::from_rgb(0xf0, 0x90, 0x40),
    Color32::from_rgb(0x9c, 0xd8, 0x50),
    Color32::from_rgb(0xf0, 0x70, 0x90),
];

/// A grid step of 1, 2 or 5 times a power of ten near `x`.
fn nice_step(x: f32) -> f32 {
    let e = 10f32.powf(x.max(1e-6).log10().floor());
    let m = x / e;
    e * if m <= 1.0 {
        1.0
    } else if m <= 2.0 {
        2.0
    } else if m <= 5.0 {
        5.0
    } else {
        10.0
    }
}

fn step_decimals(step: f32) -> usize {
    if step >= 1.0 { 0 } else { (-step.log10().floor()).clamp(0.0, 4.0) as usize }
}

impl Default for Timeline {
    fn default() -> Self {
        Self {
            pps: None,
            start: 0.0,
            scroll_y: 0.0,
            split: 300.0,
            folded: BTreeMap::new(),
            show_all: true,
            editing: None,
            scrub: None,
            selected: BTreeSet::new(),
            drag: None,
            box_from: None,
            sel_layer: None,
            layer_reorder: None,
            layer_drag: None,
            back: None,
            edit_layer: None,
            in_comp: None,
            focused: false,
            pending: Vec::new(),
            frames: 0,
            undo: Vec::new(),
            redo: Vec::new(),
            merge: None,
            graph: false,
            graph_range: None,
            grab: None,
        }
    }
}

/// A frame number as the ruler writes it: seconds and frames, minutes once
/// the scene runs that long.
fn timecode(t: f32, fps: f32) -> String {
    let per = fps.round().max(1.0) as i64;
    let frames = (t * fps).round().max(0.0) as i64;
    let (secs, f) = (frames / per, frames % per);
    if secs >= 60 {
        format!("{}:{:02}:{:02}f", secs / 60, secs % 60, f)
    } else {
        format!("{secs:02}:{f:02}f")
    }
}

fn fit_pps(width: f32, duration: f32) -> f32 {
    (width - 12.0).max(1.0) / duration.max(0.01)
}

fn mix(a: Color32, b: Color32, k: f32) -> Color32 {
    let l = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * k).round() as u8;
    Color32::from_rgb(l(a.r(), b.r()), l(a.g(), b.g()), l(a.b(), b.b()))
}

/// Text cut to a width with an ellipsis.
pub(crate) fn fit_text(
    p: &Painter,
    text: &str,
    font: &FontId,
    color: Color32,
    max_w: f32,
) -> String {
    if p.layout_no_wrap(text.to_string(), font.clone(), color).size().x <= max_w {
        return text.to_string();
    }
    let chars: Vec<char> = text.chars().collect();
    let mut n = chars.len();
    while n > 1 {
        n -= 1;
        let s: String = chars.iter().take(n).chain(['…'].iter()).collect();
        if p.layout_no_wrap(s.clone(), font.clone(), color).size().x <= max_w {
            return s;
        }
    }
    "…".into()
}

/// The open or closed chevron in front of a folder.
fn twirl(p: &Painter, c: Pos2, open: bool, color: Color32) {
    let k = 2.6;
    let pts = if open {
        vec![c + vec2(-1.6 * k, -0.8 * k), c + vec2(0.0, 0.8 * k), c + vec2(1.6 * k, -0.8 * k)]
    } else {
        vec![c + vec2(-0.8 * k, -1.6 * k), c + vec2(0.8 * k, 0.0), c + vec2(-0.8 * k, 1.6 * k)]
    };
    p.add(Shape::line(pts, Stroke::new(1.4, color)));
}

/// A key glyph: each half drawn for its own interpolation, with a dark
/// outline so it reads on any lane.
fn key_glyph(p: &Painter, c: Pos2, size: f32, left: Half, right: Half, fill: Color32) {
    let r = size / 2.0;
    let outline = Stroke::new(1.0, Color32::from_black_alpha(200));
    let mut rim: Vec<Pos2> = Vec::with_capacity(8);
    for (half, sign) in [(left, -1.0f32), (right, 1.0f32)] {
        let pts: Vec<Pos2> = match half {
            Half::Linear => vec![c + vec2(0.0, -r), c + vec2(sign * r, 0.0), c + vec2(0.0, r)],
            Half::Hold => {
                let h = 0.8 * r;
                vec![
                    c + vec2(0.0, -h),
                    c + vec2(sign * h, -h),
                    c + vec2(sign * h, h),
                    c + vec2(0.0, h),
                ]
            }
            Half::Bezier => vec![
                c + vec2(sign * 0.75 * r, -r),
                c + vec2(0.0, -0.35 * r),
                c + vec2(0.0, 0.35 * r),
                c + vec2(sign * 0.75 * r, r),
            ],
        };
        let pts = if sign < 0.0 { pts.into_iter().rev().collect() } else { pts };
        p.add(Shape::convex_polygon(pts.clone(), fill, Stroke::NONE));
        if sign < 0.0 {
            rim.extend(pts);
        } else {
            rim.splice(0..0, pts);
        }
    }
    // one outline round both halves, so a key whose sides agree reads as
    // one whole diamond rather than two halves with a seam
    p.add(Shape::closed_line(rim, outline));
}

/// A number in the accent colour that drags to change and clicks to type.
/// Returns the new value while it changes.
fn hot_number(
    ui: &mut Ui,
    editing: &mut Option<(Id, String)>,
    rect: Rect,
    id: Id,
    value: f32,
    speed: f32,
    range: (f32, f32),
    decimals: usize,
    suffix: &str,
) -> Option<f32> {
    let pal = theme::pal();
    let font = FontId::proportional(11.5);
    if editing.as_ref().is_some_and(|(e, _)| *e == id) {
        let mut done: Option<Option<f32>> = None;
        if let Some((_, text)) = editing.as_mut() {
            let edit = ui.put(
                rect,
                egui::TextEdit::singleline(text)
                    .font(font)
                    .id(id.with("edit"))
                    .margin(vec2(3.0, 1.0)),
            );
            if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                done = Some(None);
            } else if edit.lost_focus() || ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                let parsed =
                    text.trim().trim_end_matches(suffix).trim().replace(',', ".").parse::<f32>();
                done = Some(parsed.ok().map(|v| v.clamp(range.0, range.1)));
            }
        }
        if let Some(result) = done {
            *editing = None;
            return result;
        }
        return None;
    }
    let resp = ui
        .interact(rect, id, Sense::click_and_drag())
        .on_hover_cursor(CursorIcon::ResizeHorizontal);
    let hot = resp.hovered() || resp.dragged();
    let color = if hot { mix(pal.accent, Color32::WHITE, 0.25) } else { pal.accent };
    let text = format!("{value:.decimals$}{suffix}");
    let p = ui.painter();
    let r = p.text(pos2(rect.min.x + 3.0, rect.center().y), Align2::LEFT_CENTER, text, font, color);
    p.line_segment(
        [pos2(r.min.x, r.max.y + 1.0), pos2(r.max.x, r.max.y + 1.0)],
        Stroke::new(1.0, color.gamma_multiply(if hot { 0.8 } else { 0.35 })),
    );
    if resp.dragged() {
        let m = ui.input(|i| i.modifiers);
        let mult = if m.shift {
            10.0
        } else if m.command {
            0.1
        } else {
            1.0
        };
        let d = resp.drag_delta().x * speed * mult;
        if d != 0.0 {
            return Some((value + d).clamp(range.0, range.1));
        }
    } else if resp.clicked() {
        *editing = Some((id, format!("{value:.decimals$}")));
        ui.memory_mut(|m| m.request_focus(id.with("edit")));
    }
    None
}

fn icon_button(ui: &mut Ui, icon: &str, tip: &str, on: bool) -> bool {
    let pal = theme::pal();
    let text = egui::RichText::new(icon).size(14.0).color(if on { pal.accent } else { pal.text });
    ui.add(egui::Button::new(text).frame(false).min_size(vec2(20.0, 20.0)))
        .on_hover_text(tip)
        .clicked()
}

/// What one frame of drawing asks to change, applied once the frame is
/// drawn so the scene is not edited while it is read.
enum Edit {
    Seek(f32),
    Value(usize, f32),
    PartValue(usize, f32),
    Duration(f32),
    WorkArea(Option<(f32, f32)>),
    /// Add a key at the playhead on this curve, or remove the one there.
    ToggleKey(CurveTarget, String),
    /// Key the live value at the playhead, replacing a key on the frame.
    AddKey(CurveTarget, String),
    /// Jump to the previous (false) or next (true) key of a curve.
    JumpKey(CurveTarget, String, bool),
    /// Move the selected keys by whole frames.
    Move(i64),
    /// Give the selected keys an interpolation.
    SetInterp(Interp),
    Delete,
    /// Move the selected keys' values.
    Offset(f32),
    /// Place a bezier handle: the key's out handle (true) or the handle
    /// into it from the segment before, each (seconds, value) from the key.
    SetHandle(KeyRef, bool, (f32, f32)),
    /// Open up frames at the playhead: later keys move on and the scene
    /// grows by as much (Live2D's insert blank frames).
    InsertFrames(i64),
    /// Take frames out at the playhead: keys on them go, later keys move
    /// back and the scene shrinks by as much.
    RemoveFrames(i64),
    /// A layer's start, in point and out point.
    LayerSpan(usize, f32, f32, f32),
    /// A layer as a whole (weight, fades, speed, on or off).
    LayerSet(usize, Layer),
    /// A new layer of this source on top, at the playhead.
    LayerAdd(LayerSource),
    LayerRemove(usize),
    /// Move a layer up (true) or down the stack.
    LayerOrder(usize, bool),
    LayerDup(usize),
    /// Cut a layer in two at the playhead.
    LayerSplit(usize),
    /// Put a layer's start or end at the playhead (slide), or trim it there.
    LayerSnap(usize, LayerSnap),
    /// Move a layer to sit at this place in the stack.
    LayerMoveTo(usize, usize),
}

/// The log line for an edit made with one click. Drags (moving keys or
/// handles, sliders, trimming) arrive every frame, so they are not logged.
fn describe(e: &Edit) -> Option<String> {
    Some(match e {
        Edit::ToggleKey(_, id) => format!("toggled a key on {id}"),
        Edit::AddKey(_, id) => format!("keyed {id}"),
        Edit::SetInterp(interp) => format!("selected keys set to {interp:?}"),
        Edit::Delete => "deleted the selected keys".into(),
        Edit::InsertFrames(n) => format!("inserted {n} frame(s)"),
        Edit::RemoveFrames(n) => format!("removed {n} frame(s)"),
        Edit::LayerAdd(source) => format!("added a layer of {source:?}"),
        Edit::LayerRemove(i) => format!("removed layer {i}"),
        Edit::LayerOrder(i, up) => format!("moved layer {i} {}", if *up { "up" } else { "down" }),
        Edit::LayerDup(i) => format!("duplicated layer {i}"),
        Edit::LayerSplit(i) => format!("split layer {i} at the playhead"),
        Edit::LayerSnap(i, snap) => format!("layer {i}: {snap:?} at the playhead"),
        Edit::Seek(_)
        | Edit::Value(..)
        | Edit::PartValue(..)
        | Edit::Duration(_)
        | Edit::WorkArea(_)
        | Edit::JumpKey(..)
        | Edit::Move(_)
        | Edit::Offset(_)
        | Edit::SetHandle(..)
        | Edit::LayerSpan(..)
        | Edit::LayerSet(..)
        | Edit::LayerMoveTo(..) => return None,
    })
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum LayerSnap {
    StartHere,
    EndHere,
    TrimStart,
    TrimEnd,
}

const INTERPS: [(&str, Interp); 5] = [
    ("Auto smooth", Interp::Auto),
    ("Linear", Interp::Linear),
    ("Stepped", Interp::Stepped),
    ("Inverse stepped", Interp::InverseStepped),
    ("Bezier", Interp::Bezier { out: (0.0, 0.0), into: (0.0, 0.0) }),
];

/// The handles a segment shows in the graph: none for a step, along the
/// line for linear, else the ones the sampler uses (auto's flat thirds or
/// the free bezier's own). Each is relative to its own key.
fn seg_handles(k0: &Key, k1: &Key) -> Option<((f32, f32), (f32, f32))> {
    match k0.interp {
        Interp::Stepped | Interp::InverseStepped => None,
        Interp::Linear => {
            let span = (k1.time - k0.time).max(0.0);
            let dv = k1.value - k0.value;
            Some(((span / 3.0, dv / 3.0), (-span / 3.0, -dv / 3.0)))
        }
        _ => Some(ayatsuri_core::anim::handles(k0, k1)),
    }
}

/// Drop the curves left without keys, naming what they drove.
fn drop_empty(a: &mut Animation) -> Vec<(CurveTarget, String)> {
    let gone =
        a.curves.iter().filter(|c| c.keys.is_empty()).map(|c| (c.target, c.id.clone())).collect();
    a.curves.retain(|c| !c.keys.is_empty());
    gone
}

/// A curve that lost its last key no longer drives anything, so what it
/// drove goes back to rest; otherwise the old value would sit there and
/// auto key would write the key straight back.
fn reset_emptied(s: &mut Session, gone: &[(CurveTarget, String)]) {
    for (target, id) in gone {
        match target {
            CurveTarget::Parameter => {
                if let Some((pi, d)) = s
                    .puppet
                    .model()
                    .parameters
                    .iter()
                    .enumerate()
                    .find(|(_, p)| &p.id == id)
                    .map(|(pi, p)| (pi, p.default))
                {
                    s.puppet.set_value(pi, d);
                }
            }
            CurveTarget::PartOpacity => {
                if let Some(i) = s.puppet.model().parts.iter().position(|p| &p.id == id) {
                    let shown = s.part_shown.get(i).copied().unwrap_or(true);
                    s.puppet.set_part_opacity(i, if shown { 1.0 } else { 0.0 });
                }
            }
            CurveTarget::Model => {}
        }
    }
}

fn same_kind(a: Interp, b: Interp) -> bool {
    std::mem::discriminant(&a) == std::mem::discriminant(&b)
}

const UNDO_STEPS: usize = 64;

impl Timeline {
    /// The next edit starts a fresh undo step, whatever it merges with.
    pub fn unmerge(&mut self) {
        self.merge = None;
    }

    /// An action from the shortcut table, run on the next frame.
    pub fn act(&mut self, id: &str) {
        self.pending.push(id.to_string());
    }

    /// Open up `n` frames at the playhead on the next frame, or take them
    /// out when `n` is negative.
    pub fn ripple(&mut self, n: i64) {
        self.frames += n;
    }

    /// Keep the scene as it is before an edit. Edits that share a merge key
    /// while the pointer stays down fold into the step already kept.
    pub fn remember(&mut self, ai: usize, anim: &Animation, merge: Option<&'static str>) {
        self.keep(Snap::Scene(ai, anim.clone(), self.in_comp), merge);
    }

    /// The same for a composition before a layer edit.
    pub fn remember_comp(&mut self, ci: usize, comp: &Composition, merge: Option<&'static str>) {
        self.keep(Snap::Comp(ci, comp.clone()), merge);
    }

    fn keep(&mut self, snap: Snap, merge: Option<&'static str>) {
        if merge.is_some() && merge == self.merge {
            return;
        }
        self.merge = merge;
        self.redo.clear();
        self.undo.push(snap);
        if self.undo.len() > UNDO_STEPS {
            self.undo.remove(0);
        }
    }

    /// Put a kept scene or composition back, handing out what it replaced.
    fn swap(s: &mut Session, snap: Snap) -> Option<Snap> {
        match snap {
            Snap::Scene(ai, before, via) => {
                let a = s.animations.get_mut(ai)?;
                s.current_anim = Some(ai);
                s.current_comp = via.filter(|ci| *ci < s.compositions.len());
                Some(Snap::Scene(ai, std::mem::replace(a, before), via))
            }
            Snap::Comp(ci, before) => {
                let c = s.compositions.get_mut(ci)?;
                s.current_comp = Some(ci);
                Some(Snap::Comp(ci, std::mem::replace(c, before)))
            }
        }
    }

    /// Take the last key or layer edit back. True when there was one.
    pub fn undo(&mut self, s: &mut Session) -> bool {
        let Some(snap) = self.undo.pop() else { return false };
        let Some(was) = Self::swap(s, snap) else { return false };
        self.redo.push(was);
        s.modified = true;
        self.merge = None;
        self.selected.clear();
        log::info!("timeline undo ({} left)", self.undo.len());
        true
    }

    pub fn redo(&mut self, s: &mut Session) -> bool {
        let Some(snap) = self.redo.pop() else { return false };
        let Some(was) = Self::swap(s, snap) else { return false };
        self.undo.push(was);
        s.modified = true;
        self.merge = None;
        self.selected.clear();
        log::info!("timeline redo");
        true
    }

    /// The rows for a composition: the composition, then each layer with
    /// the curves of its scene under it when opened.
    fn comp_rows(&self, s: &Session, comp: &Composition) -> Vec<Row> {
        let model = s.puppet.model();
        let mut rows = vec![Row {
            kind: RowKind::Scene,
            depth: 0,
            label: comp.name.clone(),
            curve: None,
            color: None,
            children: Vec::new(),
        }];
        if self.folded.get(TOP_FOLD).copied().unwrap_or(false) {
            return rows;
        }
        for (li, l) in comp.layers.iter().enumerate() {
            rows.push(Row {
                kind: RowKind::Layer(li),
                depth: 1,
                label: l.source.name().to_string(),
                curve: None,
                color: GRAPH_COLORS.get(li % GRAPH_COLORS.len()).copied(),
                children: Vec::new(),
            });
            let open = self.folded.get(&layer_fold(li)).map(|f| !f).unwrap_or(false);
            if !open {
                continue;
            }
            if let LayerSource::Audio(p) = &l.source {
                let channels = s.audio.get(p).map_or(0, |c| c.channels);
                for c in 0..channels.min(2) as usize {
                    rows.push(Row {
                        kind: RowKind::AudioChannel(li, c),
                        depth: 2,
                        label: if c == 0 { "L".to_string() } else { "R".to_string() },
                        curve: None,
                        color: GRAPH_COLORS.get(li % GRAPH_COLORS.len()).copied(),
                        children: Vec::new(),
                    });
                }
                continue;
            }
            let LayerSource::Scene(name) = &l.source else { continue };
            let Some(a) = s.animations.iter().find(|a| &a.name == name) else { continue };
            for c in &a.curves {
                if c.keys.is_empty() {
                    continue;
                }
                let label = match c.target {
                    CurveTarget::Parameter => model
                        .parameters
                        .iter()
                        .position(|p| p.id == c.id)
                        .and_then(|pi| s.param_names.get(pi).cloned())
                        .unwrap_or_else(|| c.id.clone()),
                    CurveTarget::PartOpacity => model
                        .parts
                        .iter()
                        .position(|p| p.id == c.id)
                        .and_then(|i| s.part_names.get(i).cloned())
                        .unwrap_or_else(|| c.id.clone()),
                    CurveTarget::Model => c.id.clone(),
                };
                rows.push(Row {
                    kind: RowKind::LayerCurve(li),
                    depth: 2,
                    label,
                    curve: Some((c.target, c.id.clone())),
                    color: None,
                    children: Vec::new(),
                });
            }
        }
        rows
    }

    fn set_zoom(&mut self, pps: f32, fit: f32, duration: f32, at_t: f32, at_px: f32) {
        let npps = pps.clamp(fit, MAX_PPS.max(fit));
        if npps <= fit {
            self.pps = None;
            self.start = 0.0;
            return;
        }
        self.start = (at_t - at_px / npps).clamp(0.0, duration);
        self.pps = Some(npps);
    }

    /// The rows of the tree for this scene: the scene, each parameter
    /// folder with its parameters, the parameters outside folders, and the
    /// parts with an opacity curve (every part when showing all).
    fn rows(&self, s: &Session, anim: &Animation) -> Vec<Row> {
        let model = s.puppet.model();
        let animated = |pi: usize| {
            model
                .parameters
                .get(pi)
                .and_then(|p| ayatsuri_core::anim::param_curve(anim, &p.id))
                .is_some_and(|c| !c.keys.is_empty())
        };
        let param_row = |pi: usize, depth: u8| {
            let p = model.parameters.get(pi)?;
            Some(Row {
                kind: RowKind::Param(pi),
                depth,
                label: s.param_names.get(pi).cloned().unwrap_or_else(|| p.id.clone()),
                curve: Some((CurveTarget::Parameter, p.id.clone())),
                color: s
                    .param_colors
                    .get(pi)
                    .copied()
                    .flatten()
                    .map(|[r, g, b]| Color32::from_rgb(r, g, b)),
                children: Vec::new(),
            })
        };
        let mut rows = vec![Row {
            kind: RowKind::Scene,
            depth: 0,
            label: anim.name.clone(),
            curve: None,
            color: None,
            children: Vec::new(),
        }];
        if self.folded.get(TOP_FOLD).copied().unwrap_or(false) {
            return rows;
        }
        let mut grouped = vec![false; model.parameters.len()];
        for g in &s.groups {
            let members: Vec<usize> = g
                .params
                .iter()
                .copied()
                .filter(|&pi| pi < model.parameters.len() && (self.show_all || animated(pi)))
                .collect();
            for &pi in &g.params {
                if let Some(x) = grouped.get_mut(pi) {
                    *x = true;
                }
            }
            if members.is_empty() {
                continue;
            }
            let open = self
                .folded
                .get(&g.name)
                .map(|f| !f)
                .unwrap_or_else(|| members.iter().any(|&pi| animated(pi)));
            rows.push(Row {
                kind: RowKind::Folder(g.name.clone()),
                depth: 1,
                label: g.name.clone(),
                curve: None,
                color: g.color.map(|[r, g, b]| Color32::from_rgb(r, g, b)),
                children: members
                    .iter()
                    .filter_map(|&pi| model.parameters.get(pi))
                    .map(|p| (CurveTarget::Parameter, p.id.clone()))
                    .collect(),
            });
            if open {
                rows.extend(members.iter().filter_map(|&pi| param_row(pi, 2)));
            }
        }
        for pi in 0..model.parameters.len() {
            if !grouped.get(pi).copied().unwrap_or(false) && (self.show_all || animated(pi)) {
                rows.extend(param_row(pi, 1));
            }
        }
        let parts: Vec<usize> = (0..model.parts.len())
            .filter(|&i| {
                self.show_all
                    || model.parts.get(i).is_some_and(|p| {
                        anim.curves.iter().any(|c| {
                            c.target == CurveTarget::PartOpacity
                                && c.id == p.id
                                && !c.keys.is_empty()
                        })
                    })
            })
            .collect();
        if !parts.is_empty() {
            let name = tr("Parts");
            let open = self.folded.get(name).map(|f| !f).unwrap_or_else(|| {
                anim.curves
                    .iter()
                    .any(|c| c.target == CurveTarget::PartOpacity && !c.keys.is_empty())
            });
            rows.push(Row {
                kind: RowKind::Folder(name.to_string()),
                depth: 1,
                label: name.to_string(),
                curve: None,
                color: None,
                children: parts
                    .iter()
                    .filter_map(|&i| model.parts.get(i))
                    .map(|p| (CurveTarget::PartOpacity, p.id.clone()))
                    .collect(),
            });
            if open {
                for i in parts {
                    let Some(p) = model.parts.get(i) else { continue };
                    rows.push(Row {
                        kind: RowKind::Part(i),
                        depth: 2,
                        label: s.part_names.get(i).cloned().unwrap_or_else(|| p.id.clone()),
                        curve: Some((CurveTarget::PartOpacity, p.id.clone())),
                        color: None,
                        children: Vec::new(),
                    });
                }
            }
        }
        rows
    }

    /// Draw the palette into the space left in `ui`.
    pub fn show(&mut self, ui: &mut Ui, s: &mut Session, tp: &mut Transport) -> Option<Request> {
        let pal = theme::pal();
        let rect = ui.available_rect_before_wrap();
        ui.allocate_rect(rect, Sense::hover());
        let outer_clip = ui.clip_rect();
        ui.set_clip_rect(rect.intersect(outer_clip));
        let mut request = None;

        if s.animations.is_empty() && s.compositions.is_empty() {
            let mut h = ui.new_child(
                UiBuilder::new()
                    .max_rect(rect.shrink(8.0))
                    .layout(Layout::left_to_right(Align::Min)),
            );
            h.weak(tr("No animation yet."));
            if h.button(tr("New Animation…")).clicked() {
                request = Some(Request::NewScene);
            }
            if h.button(tr("New Composition…")).clicked() {
                request = Some(Request::NewComp);
            }
            return request;
        }
        // a composition when one is picked, else a scene
        if s.current_comp.is_some_and(|ci| ci >= s.compositions.len()) {
            s.current_comp = None;
        }
        if s.current_comp.is_none() && s.animations.is_empty() {
            s.current_comp = Some(0);
        }
        if s.current_anim.is_none_or(|i| i >= s.animations.len()) {
            s.current_anim = if s.animations.is_empty() { None } else { Some(0) };
        }
        let view = match s.current_comp {
            Some(ci) => View::Comp(ci),
            None => View::Scene(s.current_anim?),
        };
        let comp: Option<Composition> = match view {
            View::Comp(ci) => s.compositions.get(ci).cloned(),
            View::Scene(_) => None,
        };
        // the clock the header runs on: the scene, or a stand-in carrying
        // the composition's name, rate, length, loop and work area
        let anim: Animation = match (&comp, view) {
            (Some(c), _) => Animation {
                name: c.name.clone(),
                fps: c.fps,
                duration: c.duration,
                looped: c.looped,
                work_area: c.work_area,
                ..Animation::default()
            },
            (None, View::Scene(ai)) => s.animations.get(ai).cloned()?,
            (None, View::Comp(_)) => return None,
        };
        if comp.is_some() {
            self.graph = false;
        } else {
            self.edit_layer = None;
        }
        self.in_comp = s.current_comp;
        let fps = anim.fps.max(1.0);
        let frame_s = 1.0 / fps;
        let duration = anim.duration.max(frame_s);
        let mut edits: Vec<Edit> = Vec::new();
        if ui.input(|i| i.pointer.any_released()) {
            self.merge = None;
        }

        // ---- geometry ----
        self.split = self.split.clamp(MIN_SPLIT, (rect.width() - 160.0).max(MIN_SPLIT));
        let left = Rect::from_min_max(rect.min, pos2(rect.min.x + self.split, rect.max.y));
        let lanes = Rect::from_min_max(pos2(left.max.x + 1.0, rect.min.y), rect.max);
        let rows_top = rect.min.y + HEADER_H + COLHDR_H;
        let rows_bottom = rect.max.y - FOOTER_H;
        let rows_left =
            Rect::from_min_max(pos2(left.min.x, rows_top), pos2(left.max.x, rows_bottom));
        let rows_right =
            Rect::from_min_max(pos2(lanes.min.x, rows_top), pos2(lanes.max.x, rows_bottom));

        let fit = fit_pps(lanes.width(), duration);
        let pps = match self.pps {
            Some(p) if p > fit => p,
            _ => {
                self.pps = None;
                self.start = 0.0;
                fit
            }
        };
        let span = (lanes.width() - 12.0).max(1.0) / pps;
        self.start = self.start.clamp(0.0, (duration - span).max(0.0));
        let tm = TMap { x0: lanes.min.x + 6.0, start: self.start, pps };

        let p = ui.painter().clone();
        p.rect_filled(left, 0.0, pal.panel_bg);
        p.rect_filled(lanes, 0.0, pal.chrome);
        p.rect_filled(
            Rect::from_min_max(pos2(left.max.x, rect.min.y), pos2(lanes.min.x, rect.max.y)),
            0.0,
            pal.border,
        );

        // ---- header, left: timecode, scene picker, transport ----
        let hdr = Rect::from_min_size(rect.min, vec2(left.width(), HEADER_H));
        p.rect_filled(hdr, 0.0, pal.header);
        let tc_rect = Rect::from_min_size(hdr.min + vec2(10.0, 4.0), vec2(96.0, 20.0));
        let tc_resp = ui
            .interact(tc_rect, ui.id().with("tl-timecode"), Sense::click_and_drag())
            .on_hover_cursor(CursorIcon::ResizeHorizontal)
            .on_hover_text(tr("Drag to scrub"));
        p.text(
            pos2(tc_rect.min.x, tc_rect.center().y),
            Align2::LEFT_CENTER,
            timecode(*tp.time, fps),
            FontId::proportional(16.0),
            pal.accent,
        );
        p.text(
            pos2(tc_rect.min.x + 72.0, tc_rect.center().y + 1.0),
            Align2::LEFT_CENTER,
            format!("{} f", (*tp.time * fps).round() as i64),
            FontId::proportional(10.0),
            pal.text_faint,
        );
        if tc_resp.drag_started() {
            self.scrub = Some(*tp.time);
        }
        if tc_resp.dragged()
            && let Some(from) = self.scrub
            && let Some(origin) = ui.input(|i| i.pointer.press_origin())
            && let Some(now) = ui.input(|i| i.pointer.latest_pos())
        {
            let t = from + (now.x - origin.x) * 0.5 * frame_s;
            edits.push(Edit::Seek((t * fps).round() / fps));
        }
        if tc_resp.drag_stopped() {
            self.scrub = None;
        }

        let pick = Rect::from_min_max(
            pos2(hdr.min.x + 112.0, hdr.min.y + 4.0),
            pos2(hdr.max.x - 4.0, hdr.min.y + 24.0),
        );
        let mut h = ui.new_child(
            UiBuilder::new().max_rect(pick).layout(Layout::right_to_left(Align::Center)),
        );
        h.spacing_mut().item_spacing.x = 2.0;
        h.menu_button(egui_phosphor::regular::PLUS, |ui| {
            if ui.button(tr("New Animation…")).clicked() {
                request = Some(Request::NewScene);
                ui.close();
            }
            if ui.button(tr("New Composition…")).clicked() {
                request = Some(Request::NewComp);
                ui.close();
            }
        });
        let w = (pick.width() - 26.0).clamp(40.0, 140.0);
        let shown_name = match view {
            View::Comp(_) => format!("{} {}", egui_phosphor::regular::STACK, anim.name),
            View::Scene(_) => anim.name.clone(),
        };
        let mut go: Option<View> = None;
        egui::ComboBox::from_id_salt("tl-scene")
            .selected_text(fit_text(
                &p,
                &shown_name,
                &FontId::proportional(12.0),
                pal.text,
                w - 24.0,
            ))
            .width(w)
            .show_ui(&mut h, |ui| {
                for (i, a) in s.animations.iter().enumerate() {
                    if ui.selectable_label(view == View::Scene(i), &a.name).clicked() {
                        go = Some(View::Scene(i));
                    }
                }
                if !s.compositions.is_empty() && !s.animations.is_empty() {
                    ui.separator();
                }
                for (i, c) in s.compositions.iter().enumerate() {
                    let label = format!("{} {}", egui_phosphor::regular::STACK, c.name);
                    if ui.selectable_label(view == View::Comp(i), label).clicked() {
                        go = Some(View::Comp(i));
                    }
                }
            });
        if let Some(v) = go {
            match v {
                View::Scene(i) => {
                    s.current_anim = Some(i);
                    s.current_comp = None;
                }
                View::Comp(i) => s.current_comp = Some(i),
            }
            self.back = None;
            self.sel_layer = None;
            self.edit_layer = None;
            self.selected.clear();
            *tp.applied = f32::NAN;
        }

        let row2 = Rect::from_min_max(
            pos2(hdr.min.x + 6.0, hdr.min.y + 26.0),
            pos2(hdr.max.x - 4.0, hdr.max.y - 2.0),
        );
        let mut h = ui.new_child(
            UiBuilder::new().max_rect(row2).layout(Layout::left_to_right(Align::Center)),
        );
        h.spacing_mut().item_spacing.x = 0.0;
        let mut seek = None;
        if icon_button(&mut h, egui_phosphor::regular::SKIP_BACK, tr("To start"), false) {
            seek = Some(0.0);
        }
        if icon_button(&mut h, egui_phosphor::regular::CARET_LEFT, tr("Previous frame"), false) {
            seek = Some((*tp.time - frame_s).max(0.0));
        }
        let play =
            if *tp.playing { egui_phosphor::regular::PAUSE } else { egui_phosphor::regular::PLAY };
        if icon_button(
            &mut h,
            play,
            if *tp.playing { tr("Pause") } else { tr("Play") },
            *tp.playing,
        ) {
            *tp.playing = !*tp.playing;
        }
        if icon_button(&mut h, egui_phosphor::regular::CARET_RIGHT, tr("Next frame"), false) {
            seek = Some((*tp.time + frame_s).min(duration));
        }
        if icon_button(&mut h, egui_phosphor::regular::SKIP_FORWARD, tr("To end"), false) {
            seek = Some(duration);
        }
        if let Some(t) = seek {
            *tp.playing = false;
            edits.push(Edit::Seek(t));
        }
        h.add_space(6.0);
        if comp.is_none()
            && icon_button(&mut h, egui_phosphor::regular::RECORD, tr("Auto key"), *tp.autokey)
        {
            *tp.autokey = !*tp.autokey;
        }
        let looped = anim.looped;
        if icon_button(&mut h, egui_phosphor::regular::REPEAT, tr("Loop"), looped) {
            match view {
                View::Scene(ai) => {
                    if let Some(a) = s.animations.get_mut(ai) {
                        a.looped = !looped;
                    }
                }
                View::Comp(ci) => {
                    if let Some(c) = s.compositions.get_mut(ci) {
                        c.looped = !looped;
                    }
                }
            }
            s.modified = true;
        }
        h.add_space(6.0);
        if let Some(c) = &comp {
            // a layer of any scene, or of a composition that does not lead
            // back here
            let scope = s.comp_scope();
            let sources: Vec<(String, LayerSource)> = s
                .animations
                .iter()
                .map(|a| (a.name.clone(), LayerSource::Scene(a.name.clone())))
                .chain(
                    s.compositions
                        .iter()
                        .filter(|o| o.name != c.name && !scope.reaches(o, &c.name))
                        .map(|o| {
                            (
                                format!("{} {}", egui_phosphor::regular::STACK, o.name),
                                LayerSource::Composition(o.name.clone()),
                            )
                        }),
                )
                .collect();
            let add =
                egui::RichText::new(egui_phosphor::regular::STACK_PLUS).size(14.0).color(pal.text);
            let mut pick_audio = false;
            h.menu_button(add, |ui| {
                ui.label(tr("Add layer"));
                for (label, src) in sources {
                    if ui.button(label).clicked() {
                        edits.push(Edit::LayerAdd(src));
                        ui.close();
                    }
                }
                ui.separator();
                if ui.button(format!("{} {}", egui_phosphor::regular::SPEAKER_HIGH, tr("Audio file…")))
                    .on_hover_text(tr("WAV, MP3, FLAC or Ogg: plays with the composition and goes into exported video"))
                    .clicked()
                {
                    pick_audio = true;
                    ui.close();
                }
            })
            .response
            .on_hover_text(tr("Add layer"));
            if pick_audio
                && let Some(path) = rfd::FileDialog::new()
                    .add_filter("Audio", &["wav", "mp3", "flac", "ogg", "oga"])
                    .pick_file()
            {
                let rel = s.audio_rel(&path);
                match s.load_audio(&rel) {
                    Ok(()) => edits.push(Edit::LayerAdd(LayerSource::Audio(rel))),
                    Err(e) => log::error!("audio: {e}"),
                }
            }
        }
        if comp.is_none()
            && icon_button(
                &mut h,
                egui_phosphor::regular::ROWS,
                tr("Show every parameter"),
                self.show_all,
            )
        {
            self.show_all = !self.show_all;
        }
        if let (Some(ci), View::Scene(_)) = (self.back, view)
            && let Some(name) = s.compositions.get(ci).map(|c| c.name.clone())
        {
            // the way back up to the composition this scene was opened from
            let text = format!("{} {}", egui_phosphor::regular::ARROW_ELBOW_LEFT_UP, name);
            if h.add(egui::Button::new(egui::RichText::new(text).size(11.0)).frame(false))
                .on_hover_text(crate::i18n::trf("Back to {name}", &[("name", &name)]))
                .clicked()
            {
                s.current_comp = Some(ci);
                self.back = None;
                self.selected.clear();
                *tp.applied = f32::NAN;
            }
        }
        if comp.is_none()
            && icon_button(
                &mut h,
                egui_phosphor::regular::CHART_LINE,
                tr("Graph editor"),
                self.graph,
            )
        {
            self.graph = !self.graph;
            self.grab = None;
            self.graph_range = None;
        }

        // ---- header, right: navigator and ruler ----
        let nav = Rect::from_min_max(
            pos2(lanes.min.x + 6.0, rect.min.y + 5.0),
            pos2(lanes.max.x - 6.0, rect.min.y + 5.0 + NAV_H),
        );
        p.rect_filled(nav, 3.0, pal.border);
        let nav_w = nav.width().max(1.0);
        let vis0 = nav.min.x + (self.start / duration) * nav_w;
        let vis1 = nav.min.x + ((self.start + span).min(duration) / duration) * nav_w;
        let bar = Rect::from_min_max(pos2(vis0, nav.min.y), pos2(vis1, nav.max.y));
        p.rect_filled(bar, 3.0, pal.track);
        let handle =
            |x: f32| Rect::from_center_size(pos2(x, nav.center().y), vec2(6.0, NAV_H + 2.0));
        p.rect_filled(handle(vis0 + 3.0), 3.0, pal.accent);
        p.rect_filled(handle(vis1 - 3.0), 3.0, pal.accent);
        let id = ui.id().with("tl-nav");
        let r_mid = ui
            .interact(bar.shrink2(vec2(6.0, 0.0)), id.with("mid"), Sense::drag())
            .on_hover_cursor(CursorIcon::Grab);
        let r_l = ui
            .interact(handle(vis0 + 3.0).expand2(vec2(2.0, 3.0)), id.with("l"), Sense::drag())
            .on_hover_cursor(CursorIcon::ResizeHorizontal);
        let r_r = ui
            .interact(handle(vis1 - 3.0).expand2(vec2(2.0, 3.0)), id.with("r"), Sense::drag())
            .on_hover_cursor(CursorIcon::ResizeHorizontal);
        let per_px = duration / nav_w;
        if r_mid.dragged() {
            self.start =
                (self.start + r_mid.drag_delta().x * per_px).clamp(0.0, (duration - span).max(0.0));
        }
        if r_l.dragged() {
            let end = self.start + span;
            let s0 = (self.start + r_l.drag_delta().x * per_px).clamp(0.0, end - 8.0 / MAX_PPS);
            let lane_w = (lanes.width() - 12.0).max(1.0);
            self.set_zoom(lane_w / (end - s0), fit, duration, end, lane_w);
        }
        if r_r.dragged() {
            let new_span = (span + r_r.drag_delta().x * per_px).max(8.0 / MAX_PPS);
            let npps = (lanes.width() - 12.0).max(1.0) / new_span;
            self.set_zoom(npps, fit, duration, self.start, 0.0);
        }

        let ruler = Rect::from_min_max(
            pos2(lanes.min.x, rect.min.y + HEADER_H - RULER_H),
            pos2(lanes.max.x, rect.min.y + HEADER_H),
        );
        p.rect_filled(ruler, 0.0, pal.header);
        let tick = mix(pal.text_faint, pal.header, 0.3);
        let label_step = [
            1.0,
            2.0,
            5.0,
            10.0,
            fps / 2.0,
            fps,
            2.0 * fps,
            5.0 * fps,
            10.0 * fps,
            30.0 * fps,
            60.0 * fps,
        ]
        .into_iter()
        .map(|f: f32| f.round().max(1.0))
        .find(|f| f * frame_s * pps >= 52.0)
        .unwrap_or(60.0 * fps);
        let first = (self.start * fps / label_step).floor() * label_step;
        let last = ((self.start + span) * fps).ceil();
        let mut f = first;
        while f <= last {
            let x = tm.x(f * frame_s);
            if x >= lanes.min.x && x <= lanes.max.x {
                p.line_segment(
                    [pos2(x, ruler.max.y - 10.0), pos2(x, ruler.max.y)],
                    Stroke::new(1.0, tick),
                );
                p.text(
                    pos2(x + 3.0, ruler.max.y - 13.0),
                    Align2::LEFT_CENTER,
                    timecode(f * frame_s, fps),
                    FontId::proportional(10.0),
                    pal.text_faint,
                );
            }
            for m in 1..5 {
                let xm = tm.x((f + label_step * m as f32 / 5.0) * frame_s);
                if xm >= lanes.min.x && xm <= lanes.max.x {
                    p.line_segment(
                        [pos2(xm, ruler.max.y - 4.0), pos2(xm, ruler.max.y)],
                        Stroke::new(1.0, tick.gamma_multiply(0.6)),
                    );
                }
            }
            f += label_step;
        }
        p.line_segment(
            [pos2(lanes.min.x, ruler.max.y), pos2(lanes.max.x, ruler.max.y)],
            Stroke::new(1.0, pal.border),
        );
        let r_ruler = ui.interact(ruler, ui.id().with("tl-ruler"), Sense::click_and_drag());
        if (r_ruler.clicked() || r_ruler.dragged())
            && let Some(ptr) = r_ruler.interact_pointer_pos()
        {
            let t = tm.t(ptr.x).clamp(0.0, duration);
            edits.push(Edit::Seek((t * fps).round() / fps));
            *tp.playing = false;
        }

        // ---- column header row ----
        let colhdr_l =
            Rect::from_min_max(pos2(left.min.x, rows_top - COLHDR_H), pos2(left.max.x, rows_top));
        let colhdr_r =
            Rect::from_min_max(pos2(lanes.min.x, rows_top - COLHDR_H), pos2(lanes.max.x, rows_top));
        p.rect_filled(colhdr_l, 0.0, pal.panel_bg);
        p.rect_filled(colhdr_r, 0.0, pal.chrome);
        {
            // the work area: the part of the scene that plays and exports
            let (w0, w1) = anim.work_area.unwrap_or((0.0, duration));
            let (w0, w1) = (w0.clamp(0.0, duration), w1.clamp(0.0, duration));
            let bar = Rect::from_min_max(
                pos2(tm.x(w0), colhdr_r.min.y + 3.0),
                pos2(tm.x(w1), colhdr_r.max.y - 4.0),
            );
            p.rect_filled(bar.intersect(colhdr_r), 2.0, pal.track);
            let h0 = Rect::from_center_size(
                pos2(bar.min.x + 3.0, bar.center().y),
                vec2(6.0, bar.height() + 2.0),
            );
            let h1 = Rect::from_center_size(
                pos2(bar.max.x - 3.0, bar.center().y),
                vec2(6.0, bar.height() + 2.0),
            );
            let hp = p.with_clip_rect(colhdr_r);
            hp.rect_filled(h0, 2.0, pal.accent);
            hp.rect_filled(h1, 2.0, pal.accent);
            ui.set_clip_rect(colhdr_r.intersect(outer_clip));
            let wid = ui.id().with("tl-work");
            let r_mid = ui
                .interact(bar.shrink2(vec2(6.0, 0.0)), wid.with("mid"), Sense::drag())
                .on_hover_cursor(CursorIcon::Grab)
                .on_hover_text(tr("Work area: the part that plays and exports"));
            let r0 = ui
                .interact(h0.expand2(vec2(4.0, 3.0)), wid.with("a"), Sense::drag())
                .on_hover_cursor(CursorIcon::ResizeHorizontal)
                .on_hover_text(tr("Work area start (B sets it at the playhead)"));
            let r1 = ui
                .interact(h1.expand2(vec2(4.0, 3.0)), wid.with("b"), Sense::drag())
                .on_hover_cursor(CursorIcon::ResizeHorizontal)
                .on_hover_text(tr("Work area end (N sets it at the playhead)"));
            ui.set_clip_rect(rect.intersect(outer_clip));
            let snap = |t: f32| ((t * fps).round() / fps).clamp(0.0, duration);
            let mut next = None;
            if r0.dragged()
                && let Some(ptr) = r0.interact_pointer_pos()
            {
                next = Some((snap(tm.t(ptr.x)).min(w1 - frame_s), w1));
            }
            if r1.dragged()
                && let Some(ptr) = r1.interact_pointer_pos()
            {
                next = Some((w0, snap(tm.t(ptr.x)).max(w0 + frame_s)));
            }
            if r_mid.dragged() {
                let len = w1 - w0;
                let s0 = snap(w0 + r_mid.drag_delta().x / pps).min(duration - len);
                next = Some((s0, s0 + len));
            }
            if let Some((a, b)) = next {
                let whole = a <= 0.0 && b >= duration - frame_s * 0.5;
                edits.push(Edit::WorkArea(if whole { None } else { Some((a, b)) }));
            }
        }
        let keys_x = left.max.x - KEYS_W - 4.0;
        let value_x = keys_x - VALUE_W - 4.0;
        let small = FontId::proportional(10.5);
        p.text(
            pos2(left.min.x + 10.0, colhdr_l.center().y),
            Align2::LEFT_CENTER,
            tr("Name"),
            small.clone(),
            pal.text_faint,
        );
        p.text(
            pos2(value_x + 3.0, colhdr_l.center().y),
            Align2::LEFT_CENTER,
            tr("Value"),
            small.clone(),
            pal.text_faint,
        );
        p.text(
            pos2(keys_x + 4.0, colhdr_l.center().y),
            Align2::LEFT_CENTER,
            tr("Keys"),
            small.clone(),
            pal.text_faint,
        );
        p.line_segment(
            [pos2(rect.min.x, rows_top - 0.5), pos2(rect.max.x, rows_top - 0.5)],
            Stroke::new(1.0, pal.border),
        );

        // ---- rows ----
        let rows = match &comp {
            Some(c) => self.comp_rows(s, c),
            None => self.rows(s, &anim),
        };
        // rows are not all one height: a layer's is taller
        let row_h = |r: &Row| if matches!(r.kind, RowKind::Layer(_)) { LAYER_ROW_H } else { ROW_H };
        let mut row_ys: Vec<f32> = Vec::with_capacity(rows.len());
        let mut total_h = 0.0f32;
        for r in &rows {
            row_ys.push(total_h);
            total_h += row_h(r);
        }
        let max_scroll = (total_h - rows_left.height()).max(0.0);
        let rows_all = Rect::from_min_max(rows_left.min, rows_right.max);
        if ui.rect_contains_pointer(rows_all) {
            let (dy, dx, alt) =
                ui.input(|i| (i.smooth_scroll_delta.y, i.smooth_scroll_delta.x, i.modifiers.alt));
            if alt
                && dy != 0.0
                && ui.rect_contains_pointer(rows_right)
                && let Some(ptr) = ui.input(|i| i.pointer.latest_pos())
            {
                let at_t = tm.t(ptr.x);
                self.set_zoom(pps * (dy / 200.0).exp(), fit, duration, at_t, ptr.x - tm.x0);
            } else {
                self.scroll_y = (self.scroll_y - dy).clamp(0.0, max_scroll);
            }
            if dx != 0.0 && self.pps.is_some() {
                self.start = (self.start - dx / pps).clamp(0.0, (duration - span).max(0.0));
            }
        }
        self.scroll_y = self.scroll_y.clamp(0.0, max_scroll);
        let tm = TMap { x0: lanes.min.x + 6.0, start: self.start, pps };

        let lp = p.with_clip_rect(rows_left);
        let gp = p.with_clip_rect(rows_right);
        let name_font = FontId::proportional(11.5);
        let playhead_frame = (*tp.time * fps).round() as i64;
        let mut toggles: Vec<(String, bool)> = Vec::new();
        // the empty lane goes in first so the keys drawn over it win the pointer
        ui.set_clip_rect(rows_right.intersect(outer_clip));
        let bg = ui.interact(rows_right, ui.id().with("tl-lanes"), Sense::click_and_drag());
        ui.set_clip_rect(rect.intersect(outer_clip));
        let mods = ui.input(|i| i.modifiers);
        let mut key_hits: Vec<(KeyRef, Pos2)> = Vec::new();
        // the graph shows the curves holding selected keys, else the selected
        // parameter's, else every animated curve
        let shown: Vec<((CurveTarget, String), Color32)> = if self.graph {
            let mut list: Vec<(CurveTarget, String)> = anim
                .curves
                .iter()
                .filter(|c| self.selected.iter().any(|k| k.target == tgt(c.target) && k.id == c.id))
                .map(|c| (c.target, c.id.clone()))
                .collect();
            if list.is_empty()
                && let Some(pi) = *tp.sel_param
                && let Some(pr) = s.puppet.model().parameters.get(pi)
                && ayatsuri_core::anim::param_curve(&anim, &pr.id)
                    .is_some_and(|c| !c.keys.is_empty())
            {
                list.push((CurveTarget::Parameter, pr.id.clone()));
            }
            if list.is_empty() {
                list = anim
                    .curves
                    .iter()
                    .filter(|c| c.target != CurveTarget::Model && !c.keys.is_empty())
                    .take(12)
                    .map(|c| (c.target, c.id.clone()))
                    .collect();
            }
            list.into_iter()
                .enumerate()
                .map(|(i, c)| {
                    (c, GRAPH_COLORS.get(i % GRAPH_COLORS.len()).copied().unwrap_or(pal.accent))
                })
                .collect()
        } else {
            Vec::new()
        };
        let mut snap_times: Vec<f32> = vec![0.0, duration, *tp.time];
        if let Some((w0, w1)) = anim.work_area {
            snap_times.extend([w0, w1]);
        }
        for (i, row) in rows.iter().enumerate() {
            let rh = row_h(row);
            let y = rows_top - self.scroll_y + row_ys.get(i).copied().unwrap_or(0.0);
            if y + rh < rows_top || y > rows_bottom {
                continue;
            }
            let rl = Rect::from_min_size(pos2(left.min.x, y), vec2(left.width(), rh));
            let rr = Rect::from_min_size(pos2(lanes.min.x, y), vec2(lanes.width(), rh));
            let cy = y + rh / 2.0;
            let selected_param = matches!(row.kind, RowKind::Param(pi) if *tp.sel_param == Some(pi))
                || matches!(row.kind, RowKind::Layer(li) if self.sel_layer == Some(li));
            let left_bg = if selected_param {
                pal.select
            } else if matches!(row.kind, RowKind::Scene | RowKind::Folder(_)) {
                mix(pal.panel_bg, pal.header, 0.6)
            } else if i % 2 == 1 {
                mix(pal.panel_bg, pal.chrome, 0.35)
            } else {
                pal.panel_bg
            };
            lp.rect_filled(rl, 0.0, left_bg);
            if selected_param {
                gp.rect_filled(rr, 0.0, mix(pal.chrome, pal.select, 0.35));
            }
            gp.line_segment(
                [pos2(rr.min.x, rr.max.y - 0.5), pos2(rr.max.x, rr.max.y - 0.5)],
                Stroke::new(1.0, pal.border.gamma_multiply(0.6)),
            );

            let indent = left.min.x + 8.0 + 14.0 * row.depth as f32;
            ui.set_clip_rect(rows_left.intersect(outer_clip));
            match &row.kind {
                RowKind::Scene => {
                    let open = !self.folded.get(TOP_FOLD).copied().unwrap_or(false);
                    twirl(&lp, pos2(indent + 5.0, cy), open, pal.text_faint);
                    let r_fold = ui.interact(
                        Rect::from_min_size(pos2(indent - 2.0, y), vec2(16.0, rh)),
                        ui.id().with("tl-fold-top"),
                        Sense::click(),
                    );
                    if r_fold.clicked() {
                        toggles.push((TOP_FOLD.to_string(), open));
                    }
                    lp.text(
                        pos2(indent + 16.0, cy),
                        Align2::LEFT_CENTER,
                        fit_text(&lp, &row.label, &name_font, pal.text, value_x - indent - 22.0),
                        name_font.clone(),
                        pal.text,
                    );
                    // right-click on the name: what the scene or the
                    // composition as a whole can do
                    let r_title = ui.interact(
                        Rect::from_min_max(pos2(indent + 16.0, y), pos2(value_x - 4.0, y + rh)),
                        ui.id().with("tl-top-name"),
                        Sense::click(),
                    );
                    r_title.context_menu(|ui| {
                        if ui.button(tr("Edit Details…")).clicked() {
                            request = Some(Request::EditDetails);
                            ui.close();
                        }
                        ui.separator();
                        if let Some(c) = &comp {
                            ui.menu_button(tr("Add animation layer"), |ui| {
                                if s.animations.is_empty() {
                                    ui.weak(tr("No animations yet"));
                                }
                                for a in &s.animations {
                                    if ui.button(&a.name).clicked() {
                                        edits.push(Edit::LayerAdd(LayerSource::Scene(
                                            a.name.clone(),
                                        )));
                                        ui.close();
                                    }
                                }
                            });
                            ui.menu_button(tr("Add composition layer"), |ui| {
                                let others: Vec<&Composition> =
                                    s.compositions.iter().filter(|o| o.name != c.name).collect();
                                if others.is_empty() {
                                    ui.weak(tr("No other compositions yet"));
                                }
                                for o in others {
                                    if ui.button(&o.name).clicked() {
                                        edits.push(Edit::LayerAdd(LayerSource::Composition(
                                            o.name.clone(),
                                        )));
                                        ui.close();
                                    }
                                }
                            });
                            if ui.button(tr("Import Audio…")).clicked() {
                                request = Some(Request::ImportAudio);
                                ui.close();
                            }
                        } else {
                            let model = s.puppet.model();
                            ui.menu_button(tr("Add parameter track"), |ui| {
                                let free: Vec<(usize, String)> = model
                                    .parameters
                                    .iter()
                                    .enumerate()
                                    .filter(|(_, p)| {
                                        !anim.curves.iter().any(|c| {
                                            c.target == CurveTarget::Parameter && c.id == p.id
                                        })
                                    })
                                    .map(|(pi, p)| {
                                        (pi, s.param_names.get(pi).cloned().unwrap_or(p.id.clone()))
                                    })
                                    .collect();
                                if free.is_empty() {
                                    ui.weak(tr("Every parameter is animated already"));
                                }
                                egui::ScrollArea::vertical().max_height(320.0).show(ui, |ui| {
                                    for (pi, name) in free {
                                        if ui.button(name).clicked()
                                            && let Some(p) = model.parameters.get(pi)
                                        {
                                            edits.push(Edit::AddKey(
                                                CurveTarget::Parameter,
                                                p.id.clone(),
                                            ));
                                            *tp.sel_param = Some(pi);
                                            ui.close();
                                        }
                                    }
                                });
                            });
                            if ui
                                .add_enabled(
                                    tp.sel_param.is_some(),
                                    egui::Button::new(tr("Key the selected parameter")),
                                )
                                .clicked()
                            {
                                self.pending.push("anim-add-key".into());
                                ui.close();
                            }
                            if ui.button(tr("Select every key")).clicked() {
                                self.pending.push("anim-select-all".into());
                                ui.close();
                            }
                            if ui
                                .selectable_label(self.show_all, tr("Show every parameter"))
                                .clicked()
                            {
                                self.show_all = !self.show_all;
                                ui.close();
                            }
                        }
                        ui.separator();
                        if ui.button(tr("Work area start at the playhead")).clicked() {
                            self.pending.push("anim-work-start".into());
                            ui.close();
                        }
                        if ui.button(tr("Work area end at the playhead")).clicked() {
                            self.pending.push("anim-work-end".into());
                            ui.close();
                        }
                        if comp.is_none() {
                            if ui.button(tr("Insert a frame at the playhead")).clicked() {
                                edits.push(Edit::InsertFrames(1));
                                ui.close();
                            }
                            if ui.button(tr("Remove the frame at the playhead")).clicked() {
                                edits.push(Edit::RemoveFrames(1));
                                ui.close();
                            }
                        }
                        ui.separator();
                        if ui.button(tr("New Animation…")).clicked() {
                            request = Some(Request::NewScene);
                            ui.close();
                        }
                        if ui.button(tr("New Composition…")).clicked() {
                            request = Some(Request::NewComp);
                            ui.close();
                        }
                    });
                    let vr = Rect::from_min_size(pos2(value_x, y + 1.0), vec2(VALUE_W, rh - 2.0));
                    if let Some(v) = hot_number(
                        ui,
                        &mut self.editing,
                        vr,
                        ui.id().with("tl-duration"),
                        anim.duration,
                        0.05,
                        (frame_s, 600.0),
                        2,
                        " s",
                    ) {
                        edits.push(Edit::Duration(v));
                    }
                    lp.text(
                        pos2(keys_x + 4.0, cy),
                        Align2::LEFT_CENTER,
                        format!("{} fps", fps.round() as i64),
                        small.clone(),
                        pal.text_faint,
                    );
                    if !self.graph {
                        let bar = Rect::from_min_max(
                            pos2(tm.x(0.0), y + 4.0),
                            pos2(tm.x(duration), y + rh - 4.0),
                        );
                        gp.rect_filled(bar.intersect(rr), 3.0, mix(pal.track, pal.accent, 0.35));
                        let end =
                            Rect::from_center_size(pos2(tm.x(duration), cy), vec2(8.0, rh - 4.0));
                        ui.set_clip_rect(rows_right.intersect(outer_clip));
                        let r_end = ui
                            .interact(end, ui.id().with("tl-scene-end"), Sense::drag())
                            .on_hover_cursor(CursorIcon::ResizeHorizontal)
                            .on_hover_text(tr("Drag to change the length"));
                        if r_end.dragged()
                            && let Some(ptr) = r_end.interact_pointer_pos()
                        {
                            let t = tm.t(ptr.x).max(frame_s);
                            edits.push(Edit::Duration((t * fps).round() / fps));
                        }
                    }
                }
                RowKind::Folder(name) => {
                    let open = rows.get(i + 1).is_some_and(|n| n.depth > row.depth);
                    twirl(&lp, pos2(indent + 5.0, cy), open, pal.text_faint);
                    let r_fold = ui.interact(rl, ui.id().with(("tl-fold", i)), Sense::click());
                    if r_fold.clicked() {
                        toggles.push((name.clone(), open));
                    }
                    r_fold.context_menu(|ui| {
                        if ui.button(if open { tr("Fold") } else { tr("Unfold") }).clicked() {
                            toggles.push((name.clone(), open));
                            ui.close();
                        }
                        ui.separator();
                        let refs = || {
                            row.children.iter().flat_map(|(target, id)| {
                                anim.curves
                                    .iter()
                                    .filter(move |c| c.target == *target && &c.id == id)
                                    .flat_map(move |c| {
                                        c.keys.iter().map(move |k| KeyRef {
                                            target: tgt(*target),
                                            id: id.clone(),
                                            frame: (k.time * fps).round() as i64,
                                        })
                                    })
                            })
                        };
                        if ui.button(tr("Key every parameter here")).clicked() {
                            for (target, id) in &row.children {
                                if *target == CurveTarget::Parameter {
                                    edits.push(Edit::AddKey(*target, id.clone()));
                                }
                            }
                            ui.close();
                        }
                        if ui.button(tr("Select the keys in this folder")).clicked() {
                            self.selected.extend(refs());
                            ui.close();
                        }
                        if ui.button(tr("Delete the keys in this folder")).clicked() {
                            self.selected = refs().collect();
                            edits.push(Edit::Delete);
                            ui.close();
                        }
                    });
                    let mut x = indent + 14.0;
                    if let Some(c) = row.color {
                        lp.rect_filled(
                            Rect::from_center_size(pos2(x + 4.0, cy), vec2(8.0, 8.0)),
                            2.0,
                            c,
                        );
                        x += 13.0;
                    }
                    lp.text(
                        pos2(x, cy),
                        Align2::LEFT_CENTER,
                        fit_text(&lp, &row.label, &name_font, pal.text, value_x - x - 6.0),
                        name_font.clone(),
                        pal.text,
                    );
                    if !self.graph {
                        // a hollow diamond wherever any child has a key; it
                        // selects and drags every child key on that frame
                        let mut frames: BTreeMap<i64, Vec<KeyRef>> = BTreeMap::new();
                        for (target, id) in &row.children {
                            if let Some(c) =
                                anim.curves.iter().find(|c| c.target == *target && &c.id == id)
                            {
                                for k in &c.keys {
                                    let fr = (k.time * fps).round() as i64;
                                    frames.entry(fr).or_default().push(KeyRef {
                                        target: tgt(*target),
                                        id: id.clone(),
                                        frame: fr,
                                    });
                                }
                            }
                        }
                        ui.set_clip_rect(rows_right.intersect(outer_clip));
                        for (fr, refs) in frames {
                            let x = tm.x(fr as f32 * frame_s);
                            if x < rr.min.x - 8.0 || x > rr.max.x + 8.0 {
                                continue;
                            }
                            let all_sel = refs.iter().all(|k| self.selected.contains(k));
                            let c = pos2(x, cy);
                            gp.add(Shape::convex_polygon(
                                vec![
                                    c + vec2(0.0, -4.5),
                                    c + vec2(4.5, 0.0),
                                    c + vec2(0.0, 4.5),
                                    c + vec2(-4.5, 0.0),
                                ],
                                if all_sel { pal.accent } else { Color32::TRANSPARENT },
                                Stroke::new(1.0, if all_sel { pal.accent } else { pal.text_faint }),
                            ));
                            let r_sum = ui.interact(
                                Rect::from_center_size(c, vec2(12.0, 14.0)),
                                ui.id().with(("tl-sum", i, fr)),
                                Sense::click_and_drag(),
                            );
                            if r_sum.clicked() {
                                self.focused = true;
                                if !mods.shift {
                                    self.selected.clear();
                                }
                                self.selected.extend(refs.iter().cloned());
                            }
                            if r_sum.drag_started()
                                && let Some(origin) = ui.input(|i| i.pointer.press_origin())
                            {
                                if !all_sel {
                                    if !mods.shift {
                                        self.selected.clear();
                                    }
                                    self.selected.extend(refs.iter().cloned());
                                }
                                self.drag = Some(KeyDrag {
                                    origin_x: origin.x,
                                    anchor: fr as f32 * frame_s,
                                    applied: 0,
                                });
                            }
                        }
                    }
                }
                RowKind::Layer(li) => {
                    let li = *li;
                    let Some(c) = &comp else { continue };
                    let Some(l) = c.layers.get(li) else { continue };
                    let scope = s.comp_scope();
                    let len = scope.source_len(&l.source).unwrap_or(0.0);
                    let looped = scope.source_looped(&l.source);
                    let speed = l.speed.max(1e-3);
                    let end = ayatsuri_core::comp::layer_end(l);
                    let col = row.color.unwrap_or(pal.accent);
                    let is_scene = matches!(l.source, LayerSource::Scene(_));
                    let clip = match &l.source {
                        LayerSource::Audio(p) => s.audio.get(p).cloned(),
                        _ => None,
                    };
                    let is_audio = matches!(l.source, LayerSource::Audio(_));
                    let open = rows.get(i + 1).is_some_and(|n| n.depth > row.depth);
                    let stereo = clip.as_ref().is_some_and(|c| c.channels >= 2);
                    if is_scene || stereo {
                        twirl(&lp, pos2(indent + 5.0, cy), open, pal.text_faint);
                        let r_fold = ui.interact(
                            Rect::from_min_size(pos2(indent - 2.0, y), vec2(16.0, rh)),
                            ui.id().with(("tl-fold-layer", li)),
                            Sense::click(),
                        );
                        if r_fold.clicked() {
                            toggles.push((layer_fold(li), open));
                        }
                    }
                    let mut x = indent + 14.0;
                    // the eye: the layer plays or sits out
                    let eye_rect = Rect::from_center_size(pos2(x + 6.0, cy), vec2(14.0, rh));
                    lp.text(
                        eye_rect.center(),
                        Align2::CENTER_CENTER,
                        if l.enabled {
                            egui_phosphor::regular::EYE
                        } else {
                            egui_phosphor::regular::EYE_SLASH
                        },
                        FontId::proportional(12.0),
                        if l.enabled { pal.text } else { pal.text_faint },
                    );
                    let r_eye = ui
                        .interact(eye_rect, ui.id().with(("tl-layer-eye", li)), Sense::click())
                        .on_hover_text(tr("Enabled"));
                    if r_eye.clicked() {
                        let mut nl = l.clone();
                        nl.enabled = !nl.enabled;
                        edits.push(Edit::LayerSet(li, nl));
                    }
                    x += 16.0;
                    lp.rect_filled(
                        Rect::from_center_size(pos2(x + 4.0, cy), vec2(8.0, 8.0)),
                        2.0,
                        col,
                    );
                    x += 13.0;
                    let name_rect = Rect::from_min_max(pos2(x, y), pos2(value_x - 4.0, y + rh));
                    let label = match &l.source {
                        LayerSource::Scene(_) => row.label.clone(),
                        LayerSource::Composition(_) => {
                            format!("{} {}", egui_phosphor::regular::STACK, row.label)
                        }
                        LayerSource::Audio(p) => {
                            let file = std::path::Path::new(p)
                                .file_name()
                                .map(|f| f.to_string_lossy().into_owned())
                                .unwrap_or_else(|| p.clone());
                            format!("{} {}", egui_phosphor::regular::SPEAKER_HIGH, file)
                        }
                    };
                    lp.text(
                        pos2(x, cy),
                        Align2::LEFT_CENTER,
                        fit_text(&lp, &label, &name_font, pal.text, name_rect.width() - 2.0),
                        name_font.clone(),
                        if l.enabled { pal.text } else { pal.text_faint },
                    );
                    let r_name = ui.interact(
                        name_rect,
                        ui.id().with(("tl-layer-name", li)),
                        Sense::click_and_drag(),
                    );
                    if r_name.drag_started() {
                        self.layer_reorder = Some(li);
                        self.sel_layer = Some(li);
                    }
                    if r_name.hovered() {
                        ui.ctx().set_cursor_icon(CursorIcon::Grab);
                    }
                    // the weight, as a percentage
                    let vr = Rect::from_min_size(pos2(value_x, y + 1.0), vec2(VALUE_W, rh - 2.0));
                    if let Some(v) = hot_number(
                        ui,
                        &mut self.editing,
                        vr,
                        ui.id().with(("tl-layer-weight", li)),
                        l.weight * 100.0,
                        0.5,
                        (0.0, 100.0),
                        0,
                        " %",
                    ) {
                        let mut nl = l.clone();
                        nl.weight = v / 100.0;
                        edits.push(Edit::LayerSet(li, nl));
                    }
                    lp.text(
                        pos2(keys_x + 4.0, cy),
                        Align2::LEFT_CENTER,
                        timecode(end - l.start, fps),
                        small.clone(),
                        pal.text_faint,
                    );

                    // the bar in the lane
                    ui.set_clip_rect(rows_right.intersect(outer_clip));
                    let (x0, x1) = (tm.x(l.start), tm.x(end));
                    let bar = Rect::from_min_max(pos2(x0, y + 1.0), pos2(x1, y + rh - 1.0));
                    let is_sel = self.sel_layer == Some(li);
                    let fill = if l.enabled {
                        mix(col, pal.chrome, 0.55)
                    } else {
                        mix(col, pal.chrome, 0.8)
                    };
                    gp.rect_filled(bar, 3.0, fill);
                    // a sound's waveform across its bar; red when the file
                    // is missing
                    if is_audio {
                        match &clip {
                            Some(clip) => {
                                let wave = mix(col, Color32::WHITE, 0.35);
                                let x0v = bar.min.x.max(rr.min.x);
                                let x1v = bar.max.x.min(rr.max.x);
                                let mut x = x0v;
                                while x < x1v {
                                    let st = l.in_point + (tm.t(x) - l.start) * speed;
                                    let idx = (st * crate::audio::PEAKS_PER_S) as usize;
                                    let pk = clip.peaks.get(idx).copied().unwrap_or(0.0);
                                    let hh = pk * (bar.height() / 2.0 - 1.0);
                                    if hh > 0.3 {
                                        gp.line_segment(
                                            [pos2(x, cy - hh), pos2(x, cy + hh)],
                                            Stroke::new(1.0, wave),
                                        );
                                    }
                                    x += 1.0;
                                }
                            }
                            None => {
                                gp.rect_filled(bar, 3.0, pal.rotation_red.gamma_multiply(0.5));
                            }
                        }
                    }
                    // fades drawn as ramps inside the bar
                    let ramp = Color32::from_white_alpha(28);
                    if l.fade_in > 0.0 {
                        let fx = tm.x(l.start + l.fade_in).min(x1);
                        gp.add(Shape::convex_polygon(
                            vec![pos2(x0, bar.max.y), pos2(fx, bar.min.y), pos2(fx, bar.max.y)],
                            ramp,
                            Stroke::NONE,
                        ));
                    }
                    if l.fade_out > 0.0 {
                        let fx = tm.x(end - l.fade_out).max(x0);
                        gp.add(Shape::convex_polygon(
                            vec![pos2(fx, bar.min.y), pos2(x1, bar.max.y), pos2(fx, bar.max.y)],
                            ramp,
                            Stroke::NONE,
                        ));
                    }
                    gp.rect_stroke(
                        bar,
                        3.0,
                        Stroke::new(1.0, if is_sel { pal.accent } else { col }),
                        StrokeKind::Inside,
                    );
                    let bp = gp.with_clip_rect(bar.intersect(rows_right));
                    let label_font = FontId::proportional(10.5);
                    let label_w =
                        bp.layout_no_wrap(label.clone(), label_font.clone(), pal.text).size().x;
                    if is_audio {
                        // the bar's own fill behind the name, fading out to the
                        // right, so it reads over the waveform; it starts past the
                        // rounded corner so nothing pokes out
                        let fade = Rect::from_min_max(
                            pos2(x0 + 3.0, bar.min.y),
                            pos2((x0 + label_w + 22.0).min(x1 - 3.0), bar.max.y),
                        );
                        let mut mesh = egui::Mesh::default();
                        let dark =
                            Color32::from_rgba_unmultiplied(fill.r(), fill.g(), fill.b(), 245);
                        let clear = Color32::TRANSPARENT;
                        let mid_x = fade.min.x + (fade.width() - 14.0).max(0.0);
                        mesh.colored_vertex(fade.left_top(), dark);
                        mesh.colored_vertex(pos2(mid_x, fade.min.y), dark);
                        mesh.colored_vertex(pos2(mid_x, fade.max.y), dark);
                        mesh.colored_vertex(fade.left_bottom(), dark);
                        mesh.colored_vertex(pos2(mid_x, fade.min.y), dark);
                        mesh.colored_vertex(fade.right_top(), clear);
                        mesh.colored_vertex(fade.right_bottom(), clear);
                        mesh.colored_vertex(pos2(mid_x, fade.max.y), dark);
                        mesh.add_triangle(0, 1, 2);
                        mesh.add_triangle(0, 2, 3);
                        mesh.add_triangle(4, 5, 6);
                        mesh.add_triangle(4, 6, 7);
                        bp.add(Shape::mesh(mesh));
                    }
                    bp.text(
                        pos2(x0 + 6.0, cy),
                        Align2::LEFT_CENTER,
                        &label,
                        label_font,
                        if l.enabled { pal.text } else { pal.text_faint },
                    );
                    if self.layer_drag.as_ref().is_none_or(|d| d.li != li) {
                        snap_times.extend([l.start, end]);
                    }
                    let r_body =
                        ui.interact(bar, ui.id().with(("tl-layer", li)), Sense::click_and_drag());
                    let lh =
                        Rect::from_min_max(pos2(x0 - 2.0, bar.min.y), pos2(x0 + 6.0, bar.max.y));
                    let rh =
                        Rect::from_min_max(pos2(x1 - 6.0, bar.min.y), pos2(x1 + 2.0, bar.max.y));
                    let r_l = ui
                        .interact(lh, ui.id().with(("tl-layer-l", li)), Sense::drag())
                        .on_hover_cursor(CursorIcon::ResizeHorizontal);
                    let r_r = ui
                        .interact(rh, ui.id().with(("tl-layer-r", li)), Sense::drag())
                        .on_hover_cursor(CursorIcon::ResizeHorizontal);
                    // fade handles: a square at each top corner that drags
                    // inward to set the fade, as in a video editor
                    let span = (end - l.start).max(0.0);
                    let fade_px = |f: f32| (f / span.max(1e-6)) * bar.width();
                    let fi_x = (x0 + 8.0 + fade_px(l.fade_in)).min(x1 - 8.0);
                    let fo_x = (x1 - 8.0 - fade_px(l.fade_out)).max(x0 + 8.0);
                    let fi = Rect::from_center_size(pos2(fi_x, bar.min.y + 4.0), vec2(8.0, 8.0));
                    let fo = Rect::from_center_size(pos2(fo_x, bar.min.y + 4.0), vec2(8.0, 8.0));
                    let r_fi = ui
                        .interact(fi, ui.id().with(("tl-layer-fi", li)), Sense::drag())
                        .on_hover_cursor(CursorIcon::ResizeHorizontal)
                        .on_hover_text(tr("Fade in"));
                    let r_fo = ui
                        .interact(fo, ui.id().with(("tl-layer-fo", li)), Sense::drag())
                        .on_hover_cursor(CursorIcon::ResizeHorizontal)
                        .on_hover_text(tr("Fade out"));
                    let show_fades = r_body.hovered()
                        || r_fi.hovered()
                        || r_fo.hovered()
                        || r_fi.dragged()
                        || r_fo.dragged()
                        || is_sel;
                    if show_fades && bar.width() > 30.0 {
                        for (r, on) in [
                            (fi, l.fade_in > 0.0 || r_fi.hovered()),
                            (fo, l.fade_out > 0.0 || r_fo.hovered()),
                        ] {
                            gp.rect_filled(
                                r.shrink(1.5),
                                1.0,
                                if on { pal.accent } else { mix(col, Color32::WHITE, 0.5) },
                            );
                        }
                    }
                    for (resp, is_in) in [(&r_fi, true), (&r_fo, false)] {
                        if resp.dragged()
                            && let Some(ptr) = resp.interact_pointer_pos()
                        {
                            let t = tm.t(ptr.x);
                            let mut nl = l.clone();
                            let v = if is_in { t - l.start } else { end - t };
                            let v = ((v * fps).round() / fps).clamp(0.0, span);
                            if is_in {
                                nl.fade_in = v;
                            } else {
                                nl.fade_out = v;
                            }
                            edits.push(Edit::LayerSet(li, nl));
                        }
                        if resp.drag_stopped() {
                            self.merge = None;
                        }
                    }
                    ui.set_clip_rect(rect.intersect(outer_clip));
                    if r_body.hovered() {
                        ui.ctx().set_cursor_icon(CursorIcon::Grab);
                    }
                    for (resp, part) in [
                        (&r_body, LayerPart::Body),
                        (&r_l, LayerPart::Left),
                        (&r_r, LayerPart::Right),
                    ] {
                        if resp.drag_started()
                            && let Some(origin) = ui.input(|i| i.pointer.press_origin())
                        {
                            self.focused = true;
                            self.sel_layer = Some(li);
                            self.layer_drag = Some(LayerDrag {
                                li,
                                part,
                                origin_x: origin.x,
                                start: l.start,
                                in_point: l.in_point,
                                out_point: l.out_point,
                            });
                        }
                    }
                    if r_body.clicked() || r_name.clicked() {
                        self.focused = true;
                        self.sel_layer = Some(li);
                        self.selected.clear();
                    }
                    if r_body.secondary_clicked() || r_name.secondary_clicked() {
                        self.sel_layer = Some(li);
                    }
                    let mut open_source = r_body.double_clicked() || r_name.double_clicked();
                    let menu = |ui: &mut Ui,
                                edits: &mut Vec<Edit>,
                                open_source: &mut bool,
                                request: &mut Option<Request>| {
                        let mut nl = l.clone();
                        let mut changed = false;
                        egui::Grid::new(("tl-layer-menu", li)).num_columns(2).show(ui, |ui| {
                            ui.label(tr("Fade in"));
                            changed |= ui
                                .add(
                                    egui::DragValue::new(&mut nl.fade_in)
                                        .range(0.0..=60.0)
                                        .speed(0.02)
                                        .suffix(" s"),
                                )
                                .changed();
                            ui.end_row();
                            ui.label(tr("Fade out"));
                            changed |= ui
                                .add(
                                    egui::DragValue::new(&mut nl.fade_out)
                                        .range(0.0..=60.0)
                                        .speed(0.02)
                                        .suffix(" s"),
                                )
                                .changed();
                            ui.end_row();
                            ui.label(tr("Speed"));
                            changed |= ui
                                .add(
                                    egui::DragValue::new(&mut nl.speed)
                                        .range(0.1..=10.0)
                                        .speed(0.02)
                                        .suffix("x"),
                                )
                                .changed();
                            ui.end_row();
                        });
                        if changed {
                            edits.push(Edit::LayerSet(li, nl));
                        }
                        ui.separator();
                        if ui.button(tr("Start at playhead")).clicked() {
                            edits.push(Edit::LayerSnap(li, LayerSnap::StartHere));
                            ui.close();
                        }
                        if ui.button(tr("End at playhead")).clicked() {
                            edits.push(Edit::LayerSnap(li, LayerSnap::EndHere));
                            ui.close();
                        }
                        if ui.button(tr("Trim start to playhead")).clicked() {
                            edits.push(Edit::LayerSnap(li, LayerSnap::TrimStart));
                            ui.close();
                        }
                        if ui.button(tr("Trim end to playhead")).clicked() {
                            edits.push(Edit::LayerSnap(li, LayerSnap::TrimEnd));
                            ui.close();
                        }
                        if ui.button(tr("Split at playhead")).clicked() {
                            edits.push(Edit::LayerSplit(li));
                            ui.close();
                        }
                        ui.separator();
                        if ui.button(tr("Move up")).clicked() {
                            edits.push(Edit::LayerOrder(li, true));
                            ui.close();
                        }
                        if ui.button(tr("Move down")).clicked() {
                            edits.push(Edit::LayerOrder(li, false));
                            ui.close();
                        }
                        if ui.button(tr("Duplicate layer")).clicked() {
                            edits.push(Edit::LayerDup(li));
                            ui.close();
                        }
                        if ui.button(tr("Open the source")).clicked() {
                            *open_source = true;
                            ui.close();
                        }
                        ui.separator();
                        if ui.button(tr("Import Audio…")).clicked() {
                            *request = Some(Request::ImportAudio);
                            ui.close();
                        }
                        if ui.button(tr("Edit Details…")).clicked() {
                            *request = Some(Request::EditDetails);
                            ui.close();
                        }
                        ui.separator();
                        if ui.button(tr("Delete layer")).clicked() {
                            edits.push(Edit::LayerRemove(li));
                            ui.close();
                        }
                    };
                    r_body.context_menu(|ui| menu(ui, &mut edits, &mut open_source, &mut request));
                    r_name.context_menu(|ui| menu(ui, &mut edits, &mut open_source, &mut request));
                    if open_source {
                        match &l.source {
                            LayerSource::Scene(n) => {
                                if let Some(i) = s.animations.iter().position(|a| &a.name == n) {
                                    s.current_anim = Some(i);
                                    s.current_comp = None;
                                    if let View::Comp(ci) = view {
                                        self.back = Some(ci);
                                    }
                                }
                            }
                            LayerSource::Composition(n) => {
                                if let Some(i) = s.compositions.iter().position(|c| &c.name == n) {
                                    s.current_comp = Some(i);
                                }
                            }
                            LayerSource::Audio(_) => {}
                        }
                        self.sel_layer = None;
                        self.selected.clear();
                        *tp.applied = f32::NAN;
                    }
                    let _ = (len, looped, speed);
                }
                RowKind::AudioChannel(li, ch) => {
                    let (li, ch) = (*li, *ch);
                    let Some(c) = &comp else { continue };
                    let Some(l) = c.layers.get(li) else { continue };
                    let LayerSource::Audio(p) = &l.source else { continue };
                    let Some(clip) = s.audio.get(p).cloned() else { continue };
                    let col = row.color.unwrap_or(pal.accent);
                    lp.text(
                        pos2(indent, cy),
                        Align2::LEFT_CENTER,
                        &row.label,
                        name_font.clone(),
                        pal.text_faint,
                    );
                    // the channel's own waveform over the layer's span
                    let speed = l.speed.max(1e-3);
                    let end = ayatsuri_core::comp::layer_end(l);
                    let (x0, x1) = (tm.x(l.start), tm.x(end));
                    let bar = Rect::from_min_max(pos2(x0, y + 1.0), pos2(x1, y + rh - 1.0));
                    ui.set_clip_rect(rows_right.intersect(outer_clip));
                    gp.rect_filled(bar, 2.0, mix(col, pal.chrome, 0.75));
                    if let Some(peaks) = clip.peaks_ch.get(ch) {
                        let wave = mix(col, Color32::WHITE, 0.35);
                        let x0v = bar.min.x.max(rr.min.x);
                        let x1v = bar.max.x.min(rr.max.x);
                        let mut x = x0v;
                        while x < x1v {
                            let st = l.in_point + (tm.t(x) - l.start) * speed;
                            let idx = (st * crate::audio::PEAKS_PER_S) as usize;
                            let pk = peaks.get(idx).copied().unwrap_or(0.0);
                            let hh = pk * (bar.height() / 2.0 - 1.0);
                            if hh > 0.3 {
                                gp.line_segment(
                                    [pos2(x, cy - hh), pos2(x, cy + hh)],
                                    Stroke::new(1.0, wave),
                                );
                            }
                            x += 1.0;
                        }
                    }
                    ui.set_clip_rect(rect.intersect(outer_clip));
                }
                RowKind::LayerCurve(li) => {
                    let li = *li;
                    let Some(c) = &comp else { continue };
                    let Some(l) = c.layers.get(li) else { continue };
                    let LayerSource::Scene(name) = &l.source else { continue };
                    let Some(a) = s.animations.iter().find(|a| &a.name == name) else { continue };
                    let sfps = a.fps.max(1.0);
                    let speed = l.speed.max(1e-3);
                    let to_comp = |t: f32| l.start + (t - l.in_point) / speed;
                    let in_scene = |t: f32| l.in_point + (t - l.start) * speed;
                    lp.text(
                        pos2(indent, cy),
                        Align2::LEFT_CENTER,
                        fit_text(&lp, &row.label, &name_font, pal.text, value_x - indent - 6.0),
                        name_font.clone(),
                        pal.text,
                    );
                    let Some((target, id)) = row.curve.clone() else { continue };
                    let keys: &[Key] = a
                        .curves
                        .iter()
                        .find(|c| c.target == target && c.id == id)
                        .map(|c| c.keys.as_slice())
                        .unwrap_or(&[]);
                    // the playhead in the scene's own frames
                    let here = (in_scene(*tp.time) * sfps).round() as i64;
                    let on_key = keys.iter().any(|k| (k.time * sfps).round() as i64 == here);
                    let nav_c = pos2(keys_x + 25.0, cy);
                    let has_prev =
                        keys.first().is_some_and(|k| ((k.time * sfps).round() as i64) < here);
                    let has_next =
                        keys.last().is_some_and(|k| (k.time * sfps).round() as i64 > here);
                    lp.text(
                        nav_c + vec2(-16.0, 0.0),
                        Align2::CENTER_CENTER,
                        egui_phosphor::regular::CARET_LEFT,
                        FontId::proportional(11.0),
                        if has_prev { pal.text } else { pal.text_faint.gamma_multiply(0.5) },
                    );
                    lp.text(
                        nav_c + vec2(16.0, 0.0),
                        Align2::CENTER_CENTER,
                        egui_phosphor::regular::CARET_RIGHT,
                        FontId::proportional(11.0),
                        if has_next { pal.text } else { pal.text_faint.gamma_multiply(0.5) },
                    );
                    lp.add(Shape::convex_polygon(
                        vec![
                            nav_c + vec2(0.0, -5.0),
                            nav_c + vec2(5.0, 0.0),
                            nav_c + vec2(0.0, 5.0),
                            nav_c + vec2(-5.0, 0.0),
                        ],
                        if on_key { pal.accent } else { Color32::TRANSPARENT },
                        Stroke::new(1.0, if on_key { pal.accent } else { pal.text_faint }),
                    ));
                    let nav_id = ui.id().with(("tl-lnav", i));
                    let r_prev = ui.interact(
                        Rect::from_center_size(nav_c + vec2(-16.0, 0.0), vec2(14.0, 16.0)),
                        nav_id.with("prev"),
                        Sense::click(),
                    );
                    let r_key = ui.interact(
                        Rect::from_center_size(nav_c, vec2(14.0, 16.0)),
                        nav_id.with("key"),
                        Sense::click(),
                    );
                    let r_next = ui.interact(
                        Rect::from_center_size(nav_c + vec2(16.0, 0.0), vec2(14.0, 16.0)),
                        nav_id.with("next"),
                        Sense::click(),
                    );
                    let mut take = false;
                    if r_prev.on_hover_text(tr("Previous key")).clicked() && has_prev {
                        edits.push(Edit::JumpKey(target, id.clone(), false));
                        take = true;
                    }
                    if r_next.on_hover_text(tr("Next key")).clicked() && has_next {
                        edits.push(Edit::JumpKey(target, id.clone(), true));
                        take = true;
                    }
                    if r_key.on_hover_text(tr("Add or remove a key here")).clicked() {
                        edits.push(Edit::ToggleKey(target, id.clone()));
                        take = true;
                    }
                    if take && self.edit_layer != Some(li) {
                        self.selected.clear();
                        self.edit_layer = Some(li);
                    }

                    // the keys where they land on this clock; a click or
                    // drag edits them in the scene through the layer
                    ui.set_clip_rect(rows_right.intersect(outer_clip));
                    let mine = self.edit_layer == Some(li);
                    for (ki, k) in keys.iter().enumerate() {
                        if k.time < l.in_point - 1e-6 || k.time > l.out_point + 1e-6 {
                            continue;
                        }
                        let frame = (k.time * sfps).round() as i64;
                        let kref = KeyRef { target: tgt(target), id: id.clone(), frame };
                        let is_sel = mine && self.selected.contains(&kref);
                        let ct = to_comp(k.time);
                        if !is_sel {
                            snap_times.push(ct);
                        }
                        let x = tm.x(ct);
                        if x < rr.min.x - 8.0 || x > rr.max.x + 8.0 {
                            continue;
                        }
                        let c = pos2(x, cy);
                        if mine {
                            key_hits.push((kref.clone(), c));
                        }
                        let left_half = keys
                            .get(ki.wrapping_sub(1))
                            .map(|prev| half_of(prev.interp))
                            .unwrap_or(half_of(k.interp));
                        let fill = if is_sel { pal.accent } else { pal.text };
                        key_glyph(&gp, c, KEY_PX, left_half, half_of(k.interp), fill);
                        let resp = ui
                            .interact(
                                Rect::from_center_size(c, vec2(12.0, 14.0)),
                                ui.id().with(("tl-lkey", li, kref.target, &kref.id, frame)),
                                Sense::click_and_drag(),
                            )
                            .on_hover_text(format!(
                                "{:.3} {} {}",
                                k.value,
                                tr("at"),
                                timecode(ct, fps)
                            ));
                        if (resp.clicked() || resp.secondary_clicked() || resp.drag_started())
                            && !mine
                        {
                            self.selected.clear();
                            self.edit_layer = Some(li);
                            self.sel_layer = None;
                        }
                        if self
                            .key_clicks(&resp, &kref, k, keys, target, &id, sfps, mods, &mut edits)
                        {
                            edits.push(Edit::Seek(ct));
                            *tp.playing = false;
                        }
                        if resp.drag_started()
                            && let Some(origin) = ui.input(|i| i.pointer.press_origin())
                        {
                            self.focused = true;
                            self.sel_layer = None;
                            if !is_sel {
                                if !mods.shift {
                                    self.selected.clear();
                                }
                                self.selected.insert(kref.clone());
                            }
                            self.drag =
                                Some(KeyDrag { origin_x: origin.x, anchor: ct, applied: 0 });
                        }
                    }
                    ui.set_clip_rect(rect.intersect(outer_clip));
                }
                RowKind::Param(_) | RowKind::Part(_) => {
                    let mut x = indent;
                    if let Some((_, col)) =
                        shown.iter().find(|(c, _)| Some(c) == row.curve.as_ref())
                    {
                        lp.rect_filled(
                            Rect::from_min_max(
                                pos2(left.min.x + 1.0, y + 2.0),
                                pos2(left.min.x + 4.0, y + ROW_H - 2.0),
                            ),
                            1.0,
                            *col,
                        );
                    }
                    if let Some(c) = row.color {
                        lp.rect_filled(
                            Rect::from_center_size(pos2(x + 4.0, cy), vec2(8.0, 8.0)),
                            2.0,
                            c,
                        );
                        x += 13.0;
                    }
                    let name_rect = Rect::from_min_max(pos2(x, y), pos2(value_x - 4.0, y + ROW_H));
                    lp.text(
                        pos2(x, cy),
                        Align2::LEFT_CENTER,
                        fit_text(&lp, &row.label, &name_font, pal.text, name_rect.width() - 2.0),
                        name_font.clone(),
                        pal.text,
                    );
                    let r_name =
                        ui.interact(name_rect, ui.id().with(("tl-name", i)), Sense::click());
                    if r_name.clicked()
                        && let RowKind::Param(pi) = row.kind
                    {
                        *tp.sel_param = Some(pi);
                    }
                    // value: the parameter's live value, a part's sampled opacity
                    let vr =
                        Rect::from_min_size(pos2(value_x, y + 1.0), vec2(VALUE_W, ROW_H - 2.0));
                    match row.kind {
                        RowKind::Param(pi) => {
                            if let Some(pr) = s.puppet.model().parameters.get(pi) {
                                let dec = pr.decimals.clamp(0, 4) as usize;
                                let speed = (pr.max - pr.min).abs().max(1e-3) / 300.0;
                                if let Some(v) = hot_number(
                                    ui,
                                    &mut self.editing,
                                    vr,
                                    ui.id().with(("tl-value", pi)),
                                    s.puppet.value(pi),
                                    speed,
                                    (pr.min, pr.max),
                                    dec,
                                    "",
                                ) {
                                    edits.push(Edit::Value(pi, v));
                                }
                            }
                        }
                        RowKind::Part(part) => {
                            let cur = row
                                .curve
                                .as_ref()
                                .and_then(|(t, id)| {
                                    anim.curves.iter().find(|c| c.target == *t && &c.id == id)
                                })
                                .and_then(|c| ayatsuri_core::anim::sample(&anim, c, *tp.time))
                                .unwrap_or(1.0);
                            if let Some(v) = hot_number(
                                ui,
                                &mut self.editing,
                                vr,
                                ui.id().with(("tl-part", part)),
                                cur * 100.0,
                                0.5,
                                (0.0, 100.0),
                                0,
                                " %",
                            ) {
                                edits.push(Edit::PartValue(part, v / 100.0));
                            }
                        }
                        _ => {}
                    }
                    // keys in the lane
                    let curve = row.curve.as_ref().and_then(|(t, id)| {
                        anim.curves.iter().find(|c| c.target == *t && &c.id == id)
                    });
                    let keys: &[Key] = curve.map(|c| c.keys.as_slice()).unwrap_or(&[]);
                    let on_key =
                        keys.iter().any(|k| (k.time * fps).round() as i64 == playhead_frame);
                    if let Some((target, id)) = row.curve.clone() {
                        let has_prev = keys
                            .first()
                            .is_some_and(|k| ((k.time * fps).round() as i64) < playhead_frame);
                        let has_next = keys
                            .last()
                            .is_some_and(|k| (k.time * fps).round() as i64 > playhead_frame);
                        let row_refs = || {
                            keys.iter().map(|k| KeyRef {
                                target: tgt(target),
                                id: id.clone(),
                                frame: (k.time * fps).round() as i64,
                            })
                        };
                        r_name.context_menu(|ui| {
                            match row.kind {
                                RowKind::Param(pi) => {
                                    if ui.button(tr("Key at the playhead")).clicked() {
                                        edits.push(Edit::AddKey(target, id.clone()));
                                        *tp.sel_param = Some(pi);
                                        ui.close();
                                    }
                                }
                                RowKind::Part(part)
                                    if ui
                                        .button(tr("Key the opacity at the playhead"))
                                        .clicked() =>
                                {
                                    let cur = curve
                                        .and_then(|c| {
                                            ayatsuri_core::anim::sample(&anim, c, *tp.time)
                                        })
                                        .unwrap_or(1.0);
                                    edits.push(Edit::PartValue(part, cur));
                                    ui.close();
                                }
                                _ => {}
                            }
                            if ui
                                .add_enabled(
                                    on_key,
                                    egui::Button::new(tr("Remove the key at the playhead")),
                                )
                                .clicked()
                            {
                                edits.push(Edit::ToggleKey(target, id.clone()));
                                ui.close();
                            }
                            ui.separator();
                            if ui
                                .add_enabled(has_prev, egui::Button::new(tr("Previous key")))
                                .clicked()
                            {
                                edits.push(Edit::JumpKey(target, id.clone(), false));
                                ui.close();
                            }
                            if ui.add_enabled(has_next, egui::Button::new(tr("Next key"))).clicked()
                            {
                                edits.push(Edit::JumpKey(target, id.clone(), true));
                                ui.close();
                            }
                            ui.separator();
                            let any = !keys.is_empty();
                            if ui
                                .add_enabled(
                                    any,
                                    egui::Button::new(tr("Select all keys in this row")),
                                )
                                .clicked()
                            {
                                self.selected.extend(row_refs());
                                ui.close();
                            }
                            ui.add_enabled_ui(any, |ui| {
                                ui.menu_button(tr("Interpolation for this row"), |ui| {
                                    for (label, interp) in INTERPS {
                                        if ui.button(tr(label)).clicked() {
                                            self.selected = row_refs().collect();
                                            edits.push(Edit::SetInterp(interp));
                                            ui.close();
                                        }
                                    }
                                });
                            });
                            if ui
                                .add_enabled(
                                    any,
                                    egui::Button::new(tr("Delete all keys in this row")),
                                )
                                .clicked()
                            {
                                self.selected = row_refs().collect();
                                edits.push(Edit::Delete);
                                ui.close();
                            }
                            if let RowKind::Param(pi) = row.kind
                                && let Some(pr) = s.puppet.model().parameters.get(pi)
                            {
                                ui.separator();
                                if ui.button(tr("Reset to default")).clicked() {
                                    edits.push(Edit::Value(pi, pr.default));
                                    ui.close();
                                }
                            }
                        });
                    }
                    // navigator: previous key, key here, next key
                    let nav_c = pos2(keys_x + 25.0, cy);
                    let has_prev = keys
                        .first()
                        .is_some_and(|k| ((k.time * fps).round() as i64) < playhead_frame);
                    let has_next =
                        keys.last().is_some_and(|k| (k.time * fps).round() as i64 > playhead_frame);
                    lp.text(
                        nav_c + vec2(-16.0, 0.0),
                        Align2::CENTER_CENTER,
                        egui_phosphor::regular::CARET_LEFT,
                        FontId::proportional(11.0),
                        if has_prev { pal.text } else { pal.text_faint.gamma_multiply(0.5) },
                    );
                    lp.text(
                        nav_c + vec2(16.0, 0.0),
                        Align2::CENTER_CENTER,
                        egui_phosphor::regular::CARET_RIGHT,
                        FontId::proportional(11.0),
                        if has_next { pal.text } else { pal.text_faint.gamma_multiply(0.5) },
                    );
                    lp.add(Shape::convex_polygon(
                        vec![
                            nav_c + vec2(0.0, -5.0),
                            nav_c + vec2(5.0, 0.0),
                            nav_c + vec2(0.0, 5.0),
                            nav_c + vec2(-5.0, 0.0),
                        ],
                        if on_key { pal.accent } else { Color32::TRANSPARENT },
                        Stroke::new(1.0, if on_key { pal.accent } else { pal.text_faint }),
                    ));
                    if let Some((target, id)) = &row.curve {
                        let nav_id = ui.id().with(("tl-nav", i));
                        let r_prev = ui.interact(
                            Rect::from_center_size(nav_c + vec2(-16.0, 0.0), vec2(14.0, 16.0)),
                            nav_id.with("prev"),
                            Sense::click(),
                        );
                        let r_key = ui.interact(
                            Rect::from_center_size(nav_c, vec2(14.0, 16.0)),
                            nav_id.with("key"),
                            Sense::click(),
                        );
                        let r_next = ui.interact(
                            Rect::from_center_size(nav_c + vec2(16.0, 0.0), vec2(14.0, 16.0)),
                            nav_id.with("next"),
                            Sense::click(),
                        );
                        if r_prev.on_hover_text(tr("Previous key")).clicked() && has_prev {
                            edits.push(Edit::JumpKey(*target, id.clone(), false));
                        }
                        if r_next.on_hover_text(tr("Next key")).clicked() && has_next {
                            edits.push(Edit::JumpKey(*target, id.clone(), true));
                        }
                        if r_key.on_hover_text(tr("Add or remove a key here")).clicked() {
                            edits.push(Edit::ToggleKey(*target, id.clone()));
                        }
                    }

                    if self.graph {
                        continue;
                    }
                    ui.set_clip_rect(rows_right.intersect(outer_clip));
                    let Some((target, id)) = row.curve.clone() else { continue };
                    for (ki, k) in keys.iter().enumerate() {
                        let frame = (k.time * fps).round() as i64;
                        let kref = KeyRef { target: tgt(target), id: id.clone(), frame };
                        let is_sel = self.selected.contains(&kref);
                        if !is_sel {
                            snap_times.push(k.time);
                        }
                        let x = tm.x(k.time);
                        if x < rr.min.x - 8.0 || x > rr.max.x + 8.0 {
                            continue;
                        }
                        let c = pos2(x, cy);
                        key_hits.push((kref.clone(), c));
                        let left_half = keys
                            .get(ki.wrapping_sub(1))
                            .map(|prev| half_of(prev.interp))
                            .unwrap_or(half_of(k.interp));
                        let fill = if is_sel { pal.accent } else { pal.text };
                        key_glyph(&gp, c, KEY_PX, left_half, half_of(k.interp), fill);
                        let resp = ui
                            .interact(
                                Rect::from_center_size(c, vec2(12.0, 14.0)),
                                ui.id().with(("tl-key", kref.target, &kref.id, frame)),
                                Sense::click_and_drag(),
                            )
                            .on_hover_text(format!(
                                "{:.3} {} {}",
                                k.value,
                                tr("at"),
                                timecode(k.time, fps)
                            ));
                        if self
                            .key_clicks(&resp, &kref, k, keys, target, &id, fps, mods, &mut edits)
                        {
                            edits.push(Edit::Seek(k.time));
                            *tp.playing = false;
                        }
                        if resp.drag_started()
                            && let Some(origin) = ui.input(|i| i.pointer.press_origin())
                        {
                            self.focused = true;
                            if !is_sel {
                                if !mods.shift {
                                    self.selected.clear();
                                }
                                self.selected.insert(kref.clone());
                            }
                            self.drag =
                                Some(KeyDrag { origin_x: origin.x, anchor: k.time, applied: 0 });
                        }
                    }
                }
            }
        }
        ui.set_clip_rect(rect.intersect(outer_clip));
        // ---- a layer dragged up or down the stack by its name ----
        if let (Some(from), Some(_)) = (self.layer_reorder, &comp) {
            let (down, ptr) = ui.input(|i| (i.pointer.primary_down(), i.pointer.latest_pos()));
            // every layer row's top, on screen or not, in stack order
            let tops: Vec<(usize, f32)> = rows
                .iter()
                .enumerate()
                .filter_map(|(i, r)| match r.kind {
                    RowKind::Layer(li) => {
                        Some((li, rows_top - self.scroll_y + row_ys.get(i).copied().unwrap_or(0.0)))
                    }
                    _ => None,
                })
                .collect();
            if let Some(ptr) = ptr
                && !tops.is_empty()
            {
                // the slot the pointer is over: before the first row whose
                // middle lies below it
                let to = tops
                    .iter()
                    .position(|(_, y)| ptr.y < y + LAYER_ROW_H / 2.0)
                    .unwrap_or(tops.len());
                let line_y = match tops.get(to) {
                    Some((_, y)) => *y,
                    None => tops.last().map_or(rows_top, |(_, y)| y + LAYER_ROW_H),
                };
                let lp2 = p.with_clip_rect(rows_left.intersect(outer_clip));
                lp2.line_segment(
                    [pos2(left.min.x + 4.0, line_y), pos2(left.max.x - 4.0, line_y)],
                    Stroke::new(2.0, pal.accent),
                );
                ui.ctx().set_cursor_icon(CursorIcon::Grabbing);
                if !down {
                    if to != from && to != from + 1 {
                        edits.push(Edit::LayerMoveTo(from, to));
                    }
                    self.layer_reorder = None;
                }
            } else if !down {
                self.layer_reorder = None;
            }
        } else {
            self.layer_reorder = None;
        }
        for (name, was_open) in toggles {
            self.folded.insert(name, was_open);
        }

        // ---- the graph editor over the lanes ----
        if self.graph {
            let plot = Rect::from_min_max(
                pos2(rows_right.min.x, rows_right.min.y + 6.0),
                pos2(rows_right.max.x, rows_right.max.y - 26.0),
            );
            p.rect_filled(rows_right, 0.0, pal.chrome);
            let n = ((plot.width() / 2.0) as usize).max(2);
            let (mut lo, mut hi) = (f32::MAX, f32::MIN);
            let mut lines: Vec<(Color32, Vec<(f32, f32)>)> = Vec::new();
            for ((target, id), col) in &shown {
                let Some(c) = anim.curves.iter().find(|c| c.target == *target && &c.id == id)
                else {
                    continue;
                };
                let pts: Vec<(f32, f32)> = (0..=n)
                    .map(|i| {
                        let x = plot.min.x + plot.width() * i as f32 / n as f32;
                        let t = tm.t(x).clamp(0.0, duration);
                        let v = ayatsuri_core::anim::sample_curve(c, t, anim.restricted_beziers)
                            .unwrap_or(0.0);
                        (x, v)
                    })
                    .collect();
                for (_, v) in &pts {
                    lo = lo.min(*v);
                    hi = hi.max(*v);
                }
                for (ki, k) in c.keys.iter().enumerate() {
                    lo = lo.min(k.value);
                    hi = hi.max(k.value);
                    if let (Interp::Bezier { out, into }, Some(next)) =
                        (k.interp, c.keys.get(ki + 1))
                    {
                        lo = lo.min(k.value + out.1).min(next.value + into.1);
                        hi = hi.max(k.value + out.1).max(next.value + into.1);
                    }
                }
                lines.push((*col, pts));
            }
            if lines.is_empty() {
                (lo, hi) = (0.0, 1.0);
            }
            if hi - lo < 1e-6 {
                hi = lo + 1.0;
            }
            let pad = (hi - lo) * 0.08;
            let (lo, hi) = self.graph_range.unwrap_or((lo - pad, hi + pad));
            let ymap = |v: f32| plot.max.y - (v - lo) / (hi - lo) * plot.height();
            let vmap = |y: f32| lo + (plot.max.y - y) / plot.height() * (hi - lo);

            let step = nice_step((hi - lo) / 6.0);
            let dec = step_decimals(step);
            let mut g = (lo / step).ceil() * step;
            while g <= hi {
                let y = ymap(g);
                gp.line_segment(
                    [pos2(plot.min.x, y), pos2(plot.max.x, y)],
                    Stroke::new(1.0, pal.text.gamma_multiply(0.08)),
                );
                gp.text(
                    pos2(plot.min.x + 4.0, y - 1.0),
                    Align2::LEFT_BOTTOM,
                    format!("{g:.dec$}"),
                    FontId::proportional(10.0),
                    pal.text_faint,
                );
                g += step;
            }
            for (col, pts) in &lines {
                gp.add(Shape::line(
                    pts.iter().map(|(x, v)| pos2(*x, ymap(*v))).collect(),
                    Stroke::new(1.5, *col),
                ));
            }

            ui.set_clip_rect(rows_right.intersect(outer_clip));
            let handle_col = Color32::from_rgb(0xd8, 0xd8, 0x60);
            for ((target, id), col) in &shown {
                let Some(c) = anim.curves.iter().find(|c| c.target == *target && &c.id == id)
                else {
                    continue;
                };
                let keys = c.keys.as_slice();
                for (ki, k) in keys.iter().enumerate() {
                    let frame = (k.time * fps).round() as i64;
                    let kref = KeyRef { target: tgt(*target), id: id.clone(), frame };
                    let is_sel = self.selected.contains(&kref);
                    if !is_sel {
                        snap_times.push(k.time);
                    }
                    let pos = pos2(tm.x(k.time), ymap(k.value));
                    if pos.x < plot.min.x - 8.0 || pos.x > plot.max.x + 8.0 {
                        continue;
                    }
                    key_hits.push((kref.clone(), pos));
                    if is_sel {
                        // every selected key shows its handles, as in After
                        // Effects: out of this key and into it from the segment
                        // before. A step has none; auto and linear show the
                        // handles their curve really uses, and a drag turns the
                        // segment into a free bezier.
                        let mut handles: Vec<(bool, (f32, f32), f32)> = Vec::new();
                        if let Some(next) = keys.get(ki + 1)
                            && let Some((out, _)) = seg_handles(k, next)
                        {
                            handles.push((true, out, next.time - k.time));
                        }
                        if let Some(prev) = keys.get(ki.wrapping_sub(1))
                            && let Some((_, into)) = seg_handles(prev, k)
                        {
                            handles.push((false, into, k.time - prev.time));
                        }
                        let ys = plot.height() / (hi - lo).max(1e-6);
                        for (out, h, span) in handles.clone() {
                            let hp = pos2(tm.x(k.time + h.0), ymap(k.value + h.1));
                            gp.line_segment([pos, hp], Stroke::new(1.0, handle_col));
                            gp.circle_filled(hp, 3.5, handle_col);
                            let r_h = ui
                                .interact(
                                    Rect::from_center_size(hp, vec2(10.0, 10.0)),
                                    ui.id().with(("tl-handle", kref.target, &kref.id, frame, out)),
                                    Sense::drag(),
                                )
                                .on_hover_cursor(CursorIcon::Move);
                            if r_h.drag_started() {
                                self.graph_range = Some((lo, hi));
                            }
                            if r_h.dragged()
                                && let Some(ptr) = r_h.interact_pointer_pos()
                            {
                                let dt = tm.t(ptr.x) - k.time;
                                let dt =
                                    if out { dt.clamp(0.0, span) } else { dt.clamp(-span, 0.0) };
                                let dv = vmap(ptr.y) - k.value;
                                edits.push(Edit::SetHandle(kref.clone(), out, (dt, dv)));
                                // the other handle swings round to stay in line
                                // (on screen), keeping its length; Alt breaks them
                                let d = vec2(dt * pps, -dv * ys);
                                if !mods.alt
                                    && d.length() > 1e-3
                                    && let Some((_, o, ospan)) =
                                        handles.iter().find(|(oo, ..)| *oo != out)
                                {
                                    let len = vec2(o.0 * pps, -o.1 * ys).length();
                                    let nd = -d.normalized() * len;
                                    let nt = if out {
                                        (nd.x / pps).clamp(-*ospan, 0.0)
                                    } else {
                                        (nd.x / pps).clamp(0.0, *ospan)
                                    };
                                    edits.push(Edit::SetHandle(
                                        kref.clone(),
                                        !out,
                                        (nt, -nd.y / ys),
                                    ));
                                }
                            }
                            if r_h.drag_stopped() {
                                self.graph_range = None;
                            }
                        }
                    }
                    let fill = if is_sel { pal.accent } else { mix(pal.text, Color32::WHITE, 0.3) };
                    gp.rect_filled(Rect::from_center_size(pos, vec2(7.0, 7.0)), 0.0, fill);
                    gp.rect_stroke(
                        Rect::from_center_size(pos, vec2(7.0, 7.0)),
                        0.0,
                        Stroke::new(1.0, *col),
                        StrokeKind::Outside,
                    );
                    let resp = ui
                        .interact(
                            Rect::from_center_size(pos, vec2(11.0, 11.0)),
                            ui.id().with(("tl-gkey", kref.target, &kref.id, frame)),
                            Sense::click_and_drag(),
                        )
                        .on_hover_text(format!(
                            "{:.3} {} {}",
                            k.value,
                            tr("at"),
                            timecode(k.time, fps)
                        ));
                    if self.key_clicks(&resp, &kref, k, keys, *target, id, fps, mods, &mut edits) {
                        edits.push(Edit::Seek(k.time));
                        *tp.playing = false;
                    }
                    if resp.drag_started()
                        && let Some(origin) = ui.input(|i| i.pointer.press_origin())
                    {
                        self.focused = true;
                        if !is_sel {
                            if !mods.shift {
                                self.selected.clear();
                            }
                            self.selected.insert(kref.clone());
                        }
                        self.grab = Some(GraphGrab { press: origin, applied_f: 0, applied_v: 0.0 });
                        self.graph_range = Some((lo, hi));
                    }
                }
            }
            ui.set_clip_rect(rect.intersect(outer_clip));

            // a drag in the graph moves the selected keys in time and value
            let mut grab_done = false;
            if let Some(g) = &mut self.grab {
                let (down, ptr) = ui.input(|i| (i.pointer.primary_down(), i.pointer.latest_pos()));
                match ptr {
                    Some(ptr) if down => {
                        let dx = ptr.x - g.press.x;
                        let dy = ptr.y - g.press.y;
                        let mut df = (dx / pps * fps).round() as i64;
                        let mut dv = vmap(ptr.y) - vmap(g.press.y);
                        if mods.shift {
                            if dx.abs() >= dy.abs() {
                                dv = 0.0;
                            } else {
                                df = 0;
                            }
                        }
                        if df != g.applied_f {
                            edits.push(Edit::Move(df - g.applied_f));
                            g.applied_f = df;
                        }
                        if (dv - g.applied_v).abs() > 1e-6 {
                            edits.push(Edit::Offset(dv - g.applied_v));
                            g.applied_v = dv;
                        }
                        ui.ctx().set_cursor_icon(CursorIcon::Grabbing);
                    }
                    _ => grab_done = true,
                }
            }
            if grab_done {
                self.grab = None;
                self.graph_range = None;
            }

            // the pills under the plot set the selected keys' interpolation
            let mut x = plot.min.x + 6.0;
            let by = plot.max.y + 4.0;
            for (label, interp) in INTERPS {
                let text = tr(label);
                let font = FontId::proportional(11.0);
                let w = p.layout_no_wrap(text.to_string(), font.clone(), pal.text).size().x + 14.0;
                let r = Rect::from_min_size(pos2(x, by), vec2(w, 18.0));
                if r.max.x > plot.max.x {
                    break;
                }
                let resp = ui.interact(r, ui.id().with(("tl-pill", label)), Sense::click());
                let on = !self.selected.is_empty()
                    && anim.curves.iter().any(|c| {
                        c.keys.iter().any(|k| {
                            same_kind(k.interp, interp)
                                && self.selected.contains(&KeyRef {
                                    target: tgt(c.target),
                                    id: c.id.clone(),
                                    frame: (k.time * fps).round() as i64,
                                })
                        })
                    });
                let fill = if on {
                    pal.accent.gamma_multiply(0.55)
                } else if resp.hovered() {
                    mix(pal.chrome, pal.text, 0.12)
                } else {
                    mix(pal.chrome, pal.panel_bg, 0.8)
                };
                p.rect_filled(r, 3.0, fill);
                p.text(
                    r.center(),
                    Align2::CENTER_CENTER,
                    text,
                    font,
                    if on { Color32::WHITE } else { pal.text },
                );
                if resp.clicked() && !self.selected.is_empty() {
                    edits.push(Edit::SetInterp(interp));
                }
                x += w + 4.0;
            }
        }

        // ---- a drag of the selected keys, polled from the pointer ----
        let mut drag_done = false;
        if let Some(d) = &mut self.drag {
            let (down, ptr) = ui.input(|i| (i.pointer.primary_down(), i.pointer.latest_pos()));
            match ptr {
                Some(ptr) if down => {
                    let mut target = d.anchor + (ptr.x - d.origin_x) / pps;
                    if mods.shift {
                        let tol = 8.0 / pps;
                        if let Some(c) = snap_times
                            .iter()
                            .copied()
                            .filter(|c| (c - target).abs() <= tol)
                            .min_by(|a, b| (a - target).abs().total_cmp(&(b - target).abs()))
                        {
                            target = c;
                        }
                    }
                    let total = ((target - d.anchor) * fps).round() as i64;
                    if total != d.applied {
                        edits.push(Edit::Move(total - d.applied));
                        d.applied = total;
                    }
                    ui.ctx().set_cursor_icon(CursorIcon::Grabbing);
                    let text = format!(
                        "{}   {:+} f",
                        timecode(d.anchor + d.applied as f32 * frame_s, fps),
                        d.applied
                    );
                    let font = FontId::proportional(11.0);
                    let size = p.layout_no_wrap(text.clone(), font.clone(), pal.text).size();
                    let tip = Rect::from_min_size(ptr + vec2(14.0, 14.0), size + vec2(10.0, 6.0));
                    p.rect_filled(tip, 3.0, pal.panel_bg);
                    p.rect_stroke(tip, 3.0, Stroke::new(1.0, pal.border), StrokeKind::Middle);
                    p.text(tip.min + vec2(5.0, 3.0), Align2::LEFT_TOP, text, font, pal.text);
                }
                _ => drag_done = true,
            }
        }
        if drag_done {
            self.drag = None;
        }

        // ---- a drag of a layer bar, polled the same way ----
        let mut layer_done = false;
        if let (Some(d), Some(c)) = (&self.layer_drag, &comp) {
            let (down, ptr) = ui.input(|i| (i.pointer.primary_down(), i.pointer.latest_pos()));
            match (ptr, c.layers.get(d.li)) {
                (Some(ptr), Some(l)) if down => {
                    let mut dx = (ptr.x - d.origin_x) / pps;
                    if mods.shift {
                        // the moving edge snaps to other layers' ends and the playhead
                        let edge = match d.part {
                            LayerPart::Right => {
                                d.start + (d.out_point - d.in_point) / l.speed.max(1e-3)
                            }
                            _ => d.start,
                        };
                        let tol = 8.0 / pps;
                        if let Some(t) = snap_times
                            .iter()
                            .copied()
                            .filter(|t| (t - (edge + dx)).abs() <= tol)
                            .min_by(|a, b| {
                                (a - (edge + dx)).abs().total_cmp(&(b - (edge + dx)).abs())
                            })
                        {
                            dx = t - edge;
                        }
                    }
                    let scope = s.comp_scope();
                    let len = scope.source_len(&l.source).unwrap_or(0.0);
                    let looped = scope.source_looped(&l.source);
                    let (ns, nin, nout) = layer_span_after(d, dx, l.speed, len, looped, frame_s);
                    if (ns - l.start).abs() > 1e-6
                        || (nin - l.in_point).abs() > 1e-6
                        || (nout - l.out_point).abs() > 1e-6
                    {
                        edits.push(Edit::LayerSpan(d.li, ns, nin, nout));
                    }
                    ui.ctx().set_cursor_icon(match d.part {
                        LayerPart::Body => CursorIcon::Grabbing,
                        _ => CursorIcon::ResizeHorizontal,
                    });
                    let text = match d.part {
                        LayerPart::Body => timecode(ns, fps),
                        LayerPart::Left => {
                            format!("{}  in {}", timecode(ns, fps), timecode(nin, fps))
                        }
                        LayerPart::Right => format!(
                            "{}  out {}",
                            timecode(ns + (nout - nin) / l.speed.max(1e-3), fps),
                            timecode(nout, fps)
                        ),
                    };
                    let font = FontId::proportional(11.0);
                    let size = p.layout_no_wrap(text.clone(), font.clone(), pal.text).size();
                    let tip = Rect::from_min_size(ptr + vec2(14.0, 14.0), size + vec2(10.0, 6.0));
                    p.rect_filled(tip, 3.0, pal.panel_bg);
                    p.rect_stroke(tip, 3.0, Stroke::new(1.0, pal.border), StrokeKind::Middle);
                    p.text(tip.min + vec2(5.0, 3.0), Align2::LEFT_TOP, text, font, pal.text);
                }
                _ => layer_done = true,
            }
        }
        if layer_done {
            self.layer_drag = None;
            self.merge = None;
        }

        // ---- box select on the empty lane ----
        if bg.clicked() {
            self.focused = true;
            if !mods.shift {
                self.selected.clear();
            }
            if comp.is_some() {
                self.sel_layer = None;
            }
        }
        if bg.drag_started() {
            self.focused = true;
            self.box_from = ui.input(|i| i.pointer.press_origin());
        }
        if bg.dragged()
            && let (Some(a), Some(b)) = (self.box_from, ui.input(|i| i.pointer.latest_pos()))
        {
            let r = Rect::from_two_pos(a, b);
            p.rect_filled(r, 0.0, pal.accent.gamma_multiply(0.12));
            p.rect_stroke(r, 0.0, Stroke::new(1.0, pal.accent), StrokeKind::Middle);
        }
        if bg.drag_stopped() {
            if let (Some(a), Some(b)) = (self.box_from, ui.input(|i| i.pointer.latest_pos())) {
                let r = Rect::from_two_pos(a, b);
                if !mods.shift {
                    self.selected.clear();
                }
                for (k, c) in &key_hits {
                    if r.contains(*c) {
                        self.selected.insert(k.clone());
                    }
                }
            }
            self.box_from = None;
        }

        // ---- keys on the keyboard, once the palette was clicked ----
        if ui.input(|i| i.pointer.any_pressed()) && !ui.rect_contains_pointer(rect) {
            self.focused = false;
        }
        if self.focused && !ui.ctx().egui_wants_keyboard_input() {
            let (del, esc) = ui.input_mut(|i| {
                (
                    i.consume_key(Modifiers::NONE, egui::Key::Delete)
                        || i.consume_key(Modifiers::NONE, egui::Key::Backspace),
                    i.consume_key(Modifiers::NONE, egui::Key::Escape),
                )
            });
            if del {
                if !self.selected.is_empty() {
                    edits.push(Edit::Delete);
                } else if let (Some(li), Some(_)) = (self.sel_layer, &comp) {
                    edits.push(Edit::LayerRemove(li));
                }
            }
            if esc {
                self.selected.clear();
                self.sel_layer = None;
            }
        }

        // ---- actions from the shortcut table (Settings > Shortcuts) ----
        let (mut wa_start, mut wa_end) = (false, false);
        for id in std::mem::take(&mut self.pending) {
            let nudge = match id.as_str() {
                "anim-nudge-left" => -1,
                "anim-nudge-right" => 1,
                "anim-nudge-left-10" => -10,
                "anim-nudge-right-10" => 10,
                _ => 0,
            };
            if nudge != 0 {
                if !self.selected.is_empty() {
                    self.merge = None;
                    edits.push(Edit::Move(nudge));
                }
                continue;
            }
            match id.as_str() {
                "anim-add-key" => {
                    if comp.is_none()
                        && let Some(pi) = *tp.sel_param
                        && let Some(p) = s.puppet.model().parameters.get(pi)
                    {
                        edits.push(Edit::AddKey(CurveTarget::Parameter, p.id.clone()));
                    }
                }
                "anim-work-start" => wa_start = true,
                "anim-work-end" => wa_end = true,
                "anim-select-all" => {
                    self.selected = anim
                        .curves
                        .iter()
                        .flat_map(|c| {
                            c.keys.iter().map(move |k| KeyRef {
                                target: tgt(c.target),
                                id: c.id.clone(),
                                frame: (k.time * fps).round() as i64,
                            })
                        })
                        .collect();
                }
                "anim-layer-start" | "anim-layer-end" | "anim-layer-dup" | "anim-layer-split" => {
                    if let (Some(li), Some(_)) = (self.sel_layer, &comp) {
                        edits.push(match id.as_str() {
                            "anim-layer-start" => Edit::LayerSnap(li, LayerSnap::StartHere),
                            "anim-layer-end" => Edit::LayerSnap(li, LayerSnap::EndHere),
                            "anim-layer-dup" => Edit::LayerDup(li),
                            _ => Edit::LayerSplit(li),
                        });
                    }
                }
                _ => {}
            }
        }
        // the work area's start or end at the playhead, as in After Effects;
        // the whole scene again when they meet its ends
        if wa_start || wa_end {
            let (w0, w1) = anim.work_area.unwrap_or((0.0, duration));
            let at = (*tp.time * fps).round() / fps;
            let (a, b) = if wa_start {
                (at.min(duration - frame_s), w1.max(at + frame_s).min(duration))
            } else {
                (w0.min(at - frame_s).max(0.0), at.max(frame_s))
            };
            let whole = a <= 0.0 && b >= duration - frame_s * 0.5;
            self.merge = None;
            edits.push(Edit::WorkArea(if whole { None } else { Some((a, b)) }));
        }
        // frames asked for by the Animation details dialog
        let frames = std::mem::take(&mut self.frames);
        if frames > 0 {
            edits.push(Edit::InsertFrames(frames));
        } else if frames < 0 {
            edits.push(Edit::RemoveFrames(-frames));
        }

        // a slim scrollbar on the right when the rows overflow
        if max_scroll > 0.0 {
            let track = Rect::from_min_max(
                pos2(lanes.max.x - 6.0, rows_top + 2.0),
                pos2(lanes.max.x - 2.0, rows_bottom - 2.0),
            );
            let frac = rows_left.height() / total_h;
            let th = (track.height() * frac).max(18.0);
            let ty = track.min.y + (track.height() - th) * (self.scroll_y / max_scroll);
            let thumb = Rect::from_min_size(pos2(track.min.x, ty), vec2(track.width(), th));
            let r_thumb = ui.interact(thumb.expand(2.0), ui.id().with("tl-vscroll"), Sense::drag());
            p.rect_filled(
                thumb,
                2.0,
                if r_thumb.hovered() || r_thumb.dragged() { pal.text_faint } else { pal.track },
            );
            if r_thumb.dragged() {
                let per_px = max_scroll / (track.height() - th).max(1.0);
                self.scroll_y =
                    (self.scroll_y + r_thumb.drag_delta().y * per_px).clamp(0.0, max_scroll);
            }
        }

        // ---- the lanes outside the work area sit dimmed ----
        if let Some((w0, w1)) = anim.work_area {
            let dim = Color32::from_black_alpha(70);
            let (x0, x1) = (tm.x(w0.max(0.0)), tm.x(w1.min(duration)));
            if x0 > rows_right.min.x {
                gp.rect_filled(
                    Rect::from_min_max(rows_right.min, pos2(x0, rows_right.max.y)),
                    0.0,
                    dim,
                );
            }
            if x1 < rows_right.max.x {
                gp.rect_filled(
                    Rect::from_min_max(pos2(x1, rows_right.min.y), rows_right.max),
                    0.0,
                    dim,
                );
            }
        }

        // ---- playhead ----
        let px = tm.x(*tp.time);
        if px >= lanes.min.x - 1.0 && px <= lanes.max.x + 1.0 {
            p.line_segment(
                [pos2(px, ruler.max.y - 14.0), pos2(px, rows_bottom)],
                Stroke::new(1.0, pal.accent),
            );
            p.add(Shape::convex_polygon(
                vec![
                    pos2(px - 6.0, ruler.max.y - 16.0),
                    pos2(px + 6.0, ruler.max.y - 16.0),
                    pos2(px + 6.0, ruler.max.y - 8.0),
                    pos2(px, ruler.max.y - 2.0),
                    pos2(px - 6.0, ruler.max.y - 8.0),
                ],
                pal.accent,
                Stroke::NONE,
            ));
        }

        // ---- footer: zoom slider and a status line ----
        let footer = Rect::from_min_max(pos2(rect.min.x, rows_bottom), rect.max);
        p.rect_filled(footer, 0.0, pal.header);
        p.line_segment(
            [pos2(rect.min.x, rows_bottom + 0.5), pos2(rect.max.x, rows_bottom + 0.5)],
            Stroke::new(1.0, pal.border),
        );
        let fy = footer.center().y;
        let z0 = left.min.x + 72.0;
        let z1 = (left.max.x - 22.0).max(z0 + 40.0);
        p.text(
            pos2(z0 - 14.0, fy),
            Align2::CENTER_CENTER,
            egui_phosphor::regular::MAGNIFYING_GLASS_MINUS,
            FontId::proportional(12.0),
            pal.text_faint,
        );
        p.text(
            pos2(z1 + 14.0, fy),
            Align2::CENTER_CENTER,
            egui_phosphor::regular::MAGNIFYING_GLASS_PLUS,
            FontId::proportional(12.0),
            pal.text_faint,
        );
        p.line_segment([pos2(z0, fy), pos2(z1, fy)], Stroke::new(3.0, pal.border));
        let top = MAX_PPS.max(fit * 1.001);
        let frac = ((pps / fit).ln() / (top / fit).ln()).clamp(0.0, 1.0);
        let kx = z0 + (z1 - z0) * frac;
        let r_zoom = ui.interact(
            Rect::from_min_max(pos2(z0 - 6.0, fy - 8.0), pos2(z1 + 6.0, fy + 8.0)),
            ui.id().with("tl-zoom"),
            Sense::click_and_drag(),
        );
        p.circle_filled(
            pos2(kx, fy),
            4.0,
            if r_zoom.hovered() || r_zoom.dragged() { pal.accent } else { pal.text },
        );
        if r_zoom.double_clicked() {
            self.pps = None;
            self.start = 0.0;
        } else if (r_zoom.dragged() || r_zoom.clicked())
            && let Some(ptr) = r_zoom.interact_pointer_pos()
        {
            let f = ((ptr.x - z0) / (z1 - z0).max(1.0)).clamp(0.0, 1.0);
            let npps = fit * (top / fit).powf(f);
            let at_px = (tm.x(*tp.time) - tm.x0).clamp(0.0, lanes.width());
            self.set_zoom(npps, fit, duration, *tp.time, at_px);
        }
        let status = if self.selected.is_empty() {
            format!("{} / {} f", (*tp.time * fps).round() as i64, (duration * fps).round() as i64)
        } else {
            crate::i18n::trf("{n} keys selected", &[("n", &self.selected.len())])
        };
        p.text(
            pos2(rect.max.x - 8.0, fy),
            Align2::RIGHT_CENTER,
            status,
            small.clone(),
            pal.text_faint,
        );
        let n = rows.len().saturating_sub(1);
        p.text(
            pos2(rect.min.x + 10.0, fy),
            Align2::LEFT_CENTER,
            crate::i18n::trf("{n} rows", &[("n", &n)]),
            small,
            pal.text_faint,
        );
        // ---- the split between names and lanes ----
        let grip = Rect::from_min_max(
            pos2(left.max.x - 2.0, rect.min.y),
            pos2(lanes.min.x + 2.0, rows_bottom),
        );
        let r_split = ui
            .interact(grip, ui.id().with("tl-split"), Sense::drag())
            .on_hover_cursor(CursorIcon::ResizeHorizontal);
        if r_split.dragged() {
            self.split += r_split.drag_delta().x;
        }

        for e in edits {
            if let Some(words) = describe(&e) {
                log::info!("timeline: {words}");
            }
            match view {
                View::Scene(ai) => self.apply(s, ai, e, tp, fps),
                View::Comp(ci) => self.apply_comp(s, ci, e, tp, fps),
            }
        }
        request
    }

    /// A key edit made through a layer in the composition view: the
    /// layer's scene takes it, with times mapped through the layer's
    /// start, in point and speed, and the playhead mapped back.
    fn apply_in_layer(
        &mut self,
        s: &mut Session,
        ci: usize,
        li: usize,
        e: Edit,
        tp: &mut Transport,
        fps: f32,
    ) {
        let Some(l) = s.compositions.get(ci).and_then(|c| c.layers.get(li)).cloned() else {
            return;
        };
        let LayerSource::Scene(name) = &l.source else { return };
        let Some(si) = s.animations.iter().position(|a| &a.name == name) else { return };
        let sfps = s.animations.get(si).map_or(30.0, |a| a.fps).max(1.0);
        let speed = l.speed.max(1e-3);
        let e = match e {
            Edit::Move(n) => Edit::Move((n as f32 / fps.max(1.0) * speed * sfps).round() as i64),
            Edit::SetHandle(k, out, (dt, dv)) => Edit::SetHandle(k, out, (dt * speed, dv)),
            other => other,
        };
        let mut t = l.in_point + (*tp.time - l.start) * speed;
        let was = t;
        {
            let mut inner = Transport {
                time: &mut t,
                playing: &mut *tp.playing,
                autokey: &mut *tp.autokey,
                applied: &mut *tp.applied,
                sel_param: &mut *tp.sel_param,
            };
            self.apply(s, si, e, &mut inner, sfps);
        }
        if (t - was).abs() > 1e-6 {
            *tp.time = l.start + (t - l.in_point) / speed;
        }
        *tp.applied = f32::NAN;
    }

    /// Edits of a composition: its clock and its layers.
    fn apply_comp(&mut self, s: &mut Session, ci: usize, e: Edit, tp: &mut Transport, fps: f32) {
        let frame_s = 1.0 / fps.max(1.0);
        let at = (*tp.time * fps).round() / fps;
        let Some(c) = s.compositions.get(ci).cloned() else { return };
        let scope = ayatsuri_core::comp::Scope { scenes: &s.animations, comps: &s.compositions };
        // the edit as a changed copy, then kept and swapped in as one step
        let mut next = c.clone();
        let mut merge: Option<&'static str> = None;
        match e {
            Edit::Seek(t) => {
                *tp.time = t.max(0.0);
                return;
            }
            Edit::Value(..) | Edit::PartValue(..) => {
                // the puppet's own, the same in either view
                self.apply(s, usize::MAX, e, tp, fps);
                return;
            }
            Edit::Move(_)
            | Edit::SetInterp(_)
            | Edit::Delete
            | Edit::Offset(_)
            | Edit::SetHandle(..)
            | Edit::ToggleKey(..)
            | Edit::AddKey(..)
            | Edit::JumpKey(..) => {
                if let Some(li) = self.edit_layer {
                    self.apply_in_layer(s, ci, li, e, tp, fps);
                }
                return;
            }
            Edit::Duration(d) => {
                merge = Some("duration");
                next.duration = d.max(frame_s);
                if let Some((w0, w1)) = next.work_area {
                    next.work_area = Some((w0.min(next.duration), w1.min(next.duration)));
                }
            }
            Edit::WorkArea(w) => {
                merge = Some("work-area");
                next.work_area = w;
            }
            Edit::InsertFrames(n) => {
                // later layers move on; a layer across the playhead is cut
                // there and its tail moves on, so a gap opens inside it
                let dt = n as f32 / fps;
                let mut out = Vec::with_capacity(next.layers.len() + 1);
                for mut l in next.layers.drain(..) {
                    let end = ayatsuri_core::comp::layer_end(&l);
                    if l.start >= at - frame_s * 0.5 {
                        l.start += dt;
                        out.push(l);
                    } else if end > at + frame_s * 0.5 {
                        let cut = l.in_point + (at - l.start) * l.speed.max(1e-3);
                        let mut tail = l.clone();
                        tail.start = at + dt;
                        tail.in_point = cut;
                        tail.fade_in = 0.0;
                        l.out_point = cut;
                        l.fade_out = 0.0;
                        out.push(tail);
                        out.push(l);
                    } else {
                        out.push(l);
                    }
                }
                next.layers = out;
                next.duration += dt;
                self.edit_layer = None;
                self.selected.clear();
            }
            Edit::RemoveFrames(n) => {
                // the span [at, at + dt) comes out: later layers move back,
                // a layer starting inside it loses that much of its head,
                // one across it loses that much of its middle
                let dt = (n as f32 / fps).min((next.duration - frame_s).max(0.0));
                let mut out = Vec::with_capacity(next.layers.len() + 1);
                for mut l in next.layers.drain(..) {
                    let speed = l.speed.max(1e-3);
                    let end = ayatsuri_core::comp::layer_end(&l);
                    let eps = frame_s * 0.5;
                    if l.start >= at + dt - eps {
                        l.start -= dt;
                        out.push(l);
                    } else if l.start >= at - eps {
                        let lost = (at + dt - l.start) * speed;
                        l.in_point += lost;
                        l.start = at;
                        if l.out_point - l.in_point > eps * speed {
                            out.push(l);
                        }
                    } else if end > at + eps {
                        let cut = l.in_point + (at - l.start) * speed;
                        if end <= at + dt + eps {
                            l.out_point = cut;
                            out.push(l);
                        } else {
                            let mut tail = l.clone();
                            tail.start = at;
                            tail.in_point = cut + dt * speed;
                            tail.fade_in = 0.0;
                            l.out_point = cut;
                            l.fade_out = 0.0;
                            out.push(tail);
                            out.push(l);
                        }
                    } else {
                        out.push(l);
                    }
                }
                next.layers = out;
                next.duration = (next.duration - dt).max(frame_s);
                self.edit_layer = None;
                self.selected.clear();
            }
            Edit::LayerSpan(li, start, in_point, out_point) => {
                merge = Some("layer-span");
                let Some(l) = next.layers.get_mut(li) else { return };
                l.start = start.max(0.0);
                l.in_point = in_point.max(0.0);
                l.out_point = out_point.max(l.in_point + frame_s * l.speed.max(1e-3));
            }
            Edit::LayerSet(li, layer) => {
                merge = Some("layer-set");
                let Some(l) = next.layers.get_mut(li) else { return };
                *l = layer;
                l.speed = l.speed.clamp(0.1, 10.0);
                let top = if matches!(l.source, LayerSource::Audio(_)) { 2.0 } else { 1.0 };
                l.weight = l.weight.clamp(0.0, top);
                l.pan = l.pan.clamp(-1.0, 1.0);
                l.fade_in = l.fade_in.max(0.0);
                l.fade_out = l.fade_out.max(0.0);
            }
            Edit::LayerAdd(source) => {
                let len = match &source {
                    LayerSource::Audio(p) => s.audio.get(p).map(|c| c.len()),
                    _ => scope.source_len(&source),
                }
                .unwrap_or(1.0)
                .max(frame_s);
                next.layers.insert(
                    0,
                    Layer {
                        source,
                        start: at.min(next.duration),
                        out_point: len,
                        ..Layer::default()
                    },
                );
                self.sel_layer = Some(0);
            }
            Edit::LayerRemove(li) => {
                if li >= next.layers.len() {
                    return;
                }
                next.layers.remove(li);
                self.sel_layer = None;
                self.edit_layer = None;
                self.selected.clear();
            }
            Edit::LayerOrder(li, up) => {
                let to = if up { li.checked_sub(1) } else { Some(li + 1) };
                let Some(to) = to.filter(|t| *t < next.layers.len() && li < next.layers.len())
                else {
                    return;
                };
                next.layers.swap(li, to);
                self.sel_layer = Some(to);
                self.edit_layer = None;
                self.selected.clear();
            }
            Edit::LayerDup(li) => {
                let Some(l) = next.layers.get(li).cloned() else { return };
                next.layers.insert(li, l);
                self.sel_layer = Some(li);
                self.edit_layer = None;
                self.selected.clear();
            }
            Edit::LayerSplit(li) => {
                let Some(l) = next.layers.get(li).cloned() else { return };
                let end = ayatsuri_core::comp::layer_end(&l);
                if at <= l.start + frame_s * 0.5 || at >= end - frame_s * 0.5 {
                    return;
                }
                let cut = l.in_point + (at - l.start) * l.speed.max(1e-3);
                let mut tail = l.clone();
                tail.start = at;
                tail.in_point = cut;
                tail.fade_in = 0.0;
                if let Some(head) = next.layers.get_mut(li) {
                    head.out_point = cut;
                    head.fade_out = 0.0;
                }
                next.layers.insert(li, tail);
                self.sel_layer = Some(li);
            }
            Edit::LayerMoveTo(from, to) => {
                if from >= next.layers.len() || to > next.layers.len() {
                    return;
                }
                let l = next.layers.remove(from);
                let to = if to > from { to - 1 } else { to };
                next.layers.insert(to, l);
                self.sel_layer = Some(to);
                self.edit_layer = None;
                self.selected.clear();
            }
            Edit::LayerSnap(li, how) => {
                let Some(l) = next.layers.get_mut(li) else { return };
                let speed = l.speed.max(1e-3);
                let span = (l.out_point - l.in_point) / speed;
                match how {
                    LayerSnap::StartHere => l.start = at,
                    LayerSnap::EndHere => l.start = (at - span).max(0.0),
                    LayerSnap::TrimStart => {
                        let end = l.start + span;
                        if at < end - frame_s * 0.5 {
                            l.in_point = (l.in_point + (at - l.start) * speed).max(0.0);
                            l.start = at.max(0.0);
                        }
                    }
                    LayerSnap::TrimEnd => {
                        if at > l.start + frame_s * 0.5 {
                            l.out_point = l.in_point + (at - l.start) * speed;
                        }
                    }
                }
            }
        }
        if next == c {
            return;
        }
        self.remember_comp(ci, &c, merge);
        if let Some(slot) = s.compositions.get_mut(ci) {
            *slot = next;
        }
        s.modified = true;
        *tp.applied = f32::NAN;
    }

    /// What a click on a key does, in the lanes or the graph: plain selects
    /// it alone, Shift toggles it, Ctrl flips linear and auto (Ctrl+Alt
    /// holds), a right click selects it and opens the menu. True on a
    /// double click.
    #[allow(clippy::too_many_arguments)]
    fn key_clicks(
        &mut self,
        resp: &egui::Response,
        kref: &KeyRef,
        k: &Key,
        keys: &[Key],
        target: CurveTarget,
        id: &str,
        fps: f32,
        mods: Modifiers,
        edits: &mut Vec<Edit>,
    ) -> bool {
        let is_sel = self.selected.contains(kref);
        if resp.clicked() {
            self.focused = true;
            if mods.command {
                self.selected.clear();
                self.selected.insert(kref.clone());
                let next = if mods.alt {
                    if matches!(k.interp, Interp::Stepped) { Interp::Auto } else { Interp::Stepped }
                } else if matches!(k.interp, Interp::Linear) {
                    Interp::Auto
                } else {
                    Interp::Linear
                };
                edits.push(Edit::SetInterp(next));
            } else if mods.shift {
                if !self.selected.remove(kref) {
                    self.selected.insert(kref.clone());
                }
            } else {
                self.selected.clear();
                self.selected.insert(kref.clone());
            }
        }
        if resp.secondary_clicked() && !is_sel {
            self.selected.clear();
            self.selected.insert(kref.clone());
        }
        let selected = &mut self.selected;
        resp.context_menu(|ui| {
            for (label, interp) in INTERPS {
                if ui.selectable_label(same_kind(k.interp, interp), tr(label)).clicked() {
                    edits.push(Edit::SetInterp(interp));
                    ui.close();
                }
            }
            ui.separator();
            if ui.button(tr("Select all keys in this row")).clicked() {
                selected.extend(keys.iter().map(|k| KeyRef {
                    target: tgt(target),
                    id: id.to_string(),
                    frame: (k.time * fps).round() as i64,
                }));
                ui.close();
            }
            if ui.button(tr("Delete keys")).clicked() {
                edits.push(Edit::Delete);
                ui.close();
            }
        });
        resp.double_clicked()
    }

    fn apply(&mut self, s: &mut Session, ai: usize, e: Edit, tp: &mut Transport, fps: f32) {
        match e {
            Edit::Seek(t) => {
                *tp.time = t.max(0.0);
            }
            Edit::Value(pi, v) => {
                s.puppet.set_value(pi, v);
                s.dirty = true;
            }
            Edit::PartValue(part, v) => {
                let Some(id) = s.puppet.model().parts.get(part).map(|p| p.id.clone()) else {
                    return;
                };
                let Some(a) = s.animations.get_mut(ai) else { return };
                self.remember(ai, a, Some("part"));
                if let Some(c) = ayatsuri_core::anim::curve_mut(a, CurveTarget::PartOpacity, &id) {
                    ayatsuri_core::anim::set_key(c, *tp.time, v, fps, Interp::Auto);
                }
                s.modified = true;
                *tp.applied = f32::NAN;
            }
            Edit::WorkArea(w) => {
                if let Some(a) = s.animations.get_mut(ai) {
                    self.remember(ai, a, Some("work-area"));
                    a.work_area = w;
                    s.modified = true;
                }
            }
            Edit::Duration(d) => {
                if let Some(a) = s.animations.get_mut(ai) {
                    self.remember(ai, a, Some("duration"));
                    a.duration = d.max(1.0 / fps);
                    if let Some((w0, w1)) = a.work_area {
                        a.work_area = Some((w0.min(a.duration), w1.min(a.duration)));
                    }
                    s.modified = true;
                }
            }
            Edit::ToggleKey(target, id) => {
                let value = match target {
                    CurveTarget::Parameter => {
                        let Some(pi) = s.puppet.model().parameters.iter().position(|p| p.id == id)
                        else {
                            return;
                        };
                        s.puppet.value(pi)
                    }
                    CurveTarget::PartOpacity => s
                        .animations
                        .get(ai)
                        .and_then(|a| {
                            a.curves
                                .iter()
                                .find(|c| c.target == target && c.id == id)
                                .and_then(|c| ayatsuri_core::anim::sample(a, c, *tp.time))
                        })
                        .unwrap_or(1.0),
                    CurveTarget::Model => return,
                };
                let Some(a) = s.animations.get_mut(ai) else { return };
                self.remember(ai, a, None);
                let t = (*tp.time * fps).round() / fps;
                if let Some(c) = ayatsuri_core::anim::curve_mut(a, target, &id)
                    && !ayatsuri_core::anim::remove_key(c, t, fps)
                {
                    ayatsuri_core::anim::set_key(c, t, value, fps, Interp::Auto);
                }
                let emptied = drop_empty(a);
                reset_emptied(s, &emptied);
                s.modified = true;
                *tp.applied = f32::NAN;
            }
            Edit::AddKey(target, id) => {
                let CurveTarget::Parameter = target else { return };
                let Some(pi) = s.puppet.model().parameters.iter().position(|p| p.id == id) else {
                    return;
                };
                let value = s.puppet.value(pi);
                let Some(a) = s.animations.get_mut(ai) else { return };
                self.remember(ai, a, None);
                let t = (*tp.time * fps).round() / fps;
                if let Some(c) = ayatsuri_core::anim::curve_mut(a, target, &id) {
                    ayatsuri_core::anim::set_key(c, t, value, fps, Interp::Auto);
                }
                s.modified = true;
                *tp.applied = f32::NAN;
            }
            Edit::JumpKey(target, id, next) => {
                let t = *tp.time;
                let Some(a) = s.animations.get(ai) else { return };
                let Some(c) = a.curves.iter().find(|c| c.target == target && c.id == id) else {
                    return;
                };
                let to = if next {
                    c.keys.iter().map(|k| k.time).find(|k| *k > t + 1e-4)
                } else {
                    c.keys.iter().map(|k| k.time).rev().find(|k| *k < t - 1e-4)
                };
                if let Some(to) = to {
                    *tp.time = to;
                    *tp.playing = false;
                }
            }
            Edit::Move(delta) => {
                let Some(a) = s.animations.get_mut(ai) else { return };
                // the earliest selected key stops at frame 0
                let first = self.selected.iter().map(|k| k.frame).min().unwrap_or(0);
                let delta = delta.max(-first);
                if delta == 0 {
                    return;
                }
                self.remember(ai, a, Some("key-drag"));
                let mut moved = BTreeSet::new();
                for c in &mut a.curves {
                    let t = tgt(c.target);
                    let picked: BTreeSet<i64> = self
                        .selected
                        .iter()
                        .filter(|k| k.target == t && k.id == c.id)
                        .map(|k| k.frame)
                        .collect();
                    if picked.is_empty() {
                        continue;
                    }
                    let frame_of = |k: &Key| (k.time * fps).round() as i64;
                    let mut sel: Vec<Key> = Vec::new();
                    let mut rest: Vec<Key> = Vec::new();
                    for k in c.keys.drain(..) {
                        if picked.contains(&frame_of(&k)) {
                            let fr = frame_of(&k) + delta;
                            sel.push(Key { time: fr as f32 / fps, ..k });
                        } else {
                            rest.push(k);
                        }
                    }
                    // a moved key takes the place of any key it lands on
                    let taken: BTreeSet<i64> = sel.iter().map(frame_of).collect();
                    rest.retain(|k| !taken.contains(&frame_of(k)));
                    rest.extend(sel);
                    rest.sort_by(|x, y| x.time.total_cmp(&y.time));
                    c.keys = rest;
                    moved.extend(taken.into_iter().map(|frame| KeyRef {
                        target: t,
                        id: c.id.clone(),
                        frame,
                    }));
                }
                self.selected = moved;
                s.modified = true;
                *tp.applied = f32::NAN;
            }
            Edit::SetInterp(interp) => {
                let Some(a) = s.animations.get_mut(ai) else { return };
                self.remember(ai, a, None);
                for c in &mut a.curves {
                    let t = tgt(c.target);
                    let picked: BTreeSet<i64> = self
                        .selected
                        .iter()
                        .filter(|k| k.target == t && k.id == c.id)
                        .map(|k| k.frame)
                        .collect();
                    if picked.is_empty() {
                        continue;
                    }
                    // a free bezier starts from the auto handles of its segment
                    let spans: Vec<f32> = (0..c.keys.len())
                        .map(|i| match (c.keys.get(i), c.keys.get(i + 1)) {
                            (Some(k0), Some(k1)) => (k1.time - k0.time).max(0.0),
                            _ => 1.0 / fps,
                        })
                        .collect();
                    for (k, span) in c.keys.iter_mut().zip(spans) {
                        if !picked.contains(&((k.time * fps).round() as i64)) {
                            continue;
                        }
                        k.interp = match interp {
                            Interp::Bezier { .. } => {
                                Interp::Bezier { out: (span / 3.0, 0.0), into: (-span / 3.0, 0.0) }
                            }
                            other => other,
                        };
                    }
                }
                s.modified = true;
                *tp.applied = f32::NAN;
            }
            Edit::Offset(dv) => {
                let ranges: Vec<(String, f32, f32)> = s
                    .puppet
                    .model()
                    .parameters
                    .iter()
                    .map(|p| (p.id.clone(), p.min, p.max))
                    .collect();
                let Some(a) = s.animations.get_mut(ai) else { return };
                self.remember(ai, a, Some("key-drag"));
                for c in &mut a.curves {
                    let t = tgt(c.target);
                    let picked: BTreeSet<i64> = self
                        .selected
                        .iter()
                        .filter(|k| k.target == t && k.id == c.id)
                        .map(|k| k.frame)
                        .collect();
                    if picked.is_empty() {
                        continue;
                    }
                    let (lo, hi) = match c.target {
                        CurveTarget::Parameter => ranges
                            .iter()
                            .find(|(id, ..)| *id == c.id)
                            .map(|(_, lo, hi)| (*lo, *hi))
                            .unwrap_or((f32::MIN, f32::MAX)),
                        CurveTarget::PartOpacity => (0.0, 1.0),
                        CurveTarget::Model => (f32::MIN, f32::MAX),
                    };
                    for k in &mut c.keys {
                        if picked.contains(&((k.time * fps).round() as i64)) {
                            k.value = (k.value + dv).clamp(lo, hi);
                        }
                    }
                }
                s.modified = true;
                *tp.applied = f32::NAN;
            }
            Edit::SetHandle(kref, out, h) => {
                let Some(a) = s.animations.get_mut(ai) else { return };
                self.remember(ai, a, Some("handle"));
                let Some(c) =
                    a.curves.iter_mut().find(|c| tgt(c.target) == kref.target && c.id == kref.id)
                else {
                    return;
                };
                let Some(i) =
                    c.keys.iter().position(|k| (k.time * fps).round() as i64 == kref.frame)
                else {
                    return;
                };
                // the side not dragged keeps the shape the segment had
                if out {
                    let seed =
                        c.keys.get(i).zip(c.keys.get(i + 1)).and_then(|(k, n)| seg_handles(k, n));
                    if let Some(k) = c.keys.get_mut(i) {
                        let into = match k.interp {
                            Interp::Bezier { into, .. } => into,
                            _ => seed.map(|s| s.1).unwrap_or((0.0, 0.0)),
                        };
                        k.interp = Interp::Bezier { out: h, into };
                    }
                } else if i > 0 {
                    let seed =
                        c.keys.get(i - 1).zip(c.keys.get(i)).and_then(|(p, k)| seg_handles(p, k));
                    if let Some(prev) = c.keys.get_mut(i - 1) {
                        let out = match prev.interp {
                            Interp::Bezier { out, .. } => out,
                            _ => seed.map(|s| s.0).unwrap_or((0.0, 0.0)),
                        };
                        prev.interp = Interp::Bezier { out, into: h };
                    }
                }
                s.modified = true;
                *tp.applied = f32::NAN;
            }
            Edit::InsertFrames(n) => {
                let Some(a) = s.animations.get_mut(ai) else { return };
                self.remember(ai, a, None);
                let at = (*tp.time * fps).round() as i64;
                let dt = n as f32 / fps;
                for c in &mut a.curves {
                    for k in &mut c.keys {
                        if (k.time * fps).round() as i64 >= at {
                            k.time += dt;
                        }
                    }
                }
                a.duration += dt;
                if let Some((w0, w1)) = a.work_area {
                    let at_t = at as f32 / fps;
                    a.work_area = Some((
                        if w0 >= at_t { w0 + dt } else { w0 },
                        if w1 >= at_t { w1 + dt } else { w1 },
                    ));
                }
                self.selected.clear();
                s.modified = true;
                *tp.applied = f32::NAN;
            }
            Edit::RemoveFrames(n) => {
                let Some(a) = s.animations.get_mut(ai) else { return };
                self.remember(ai, a, None);
                let at = (*tp.time * fps).round() as i64;
                let dt = n as f32 / fps;
                for c in &mut a.curves {
                    c.keys.retain(|k| {
                        let f = (k.time * fps).round() as i64;
                        f < at || f >= at + n
                    });
                    for k in &mut c.keys {
                        if (k.time * fps).round() as i64 >= at + n {
                            k.time -= dt;
                        }
                    }
                }
                let emptied = drop_empty(a);
                a.duration = (a.duration - dt).max(1.0 / fps);
                if let Some((w0, w1)) = a.work_area {
                    a.work_area = Some((w0.min(a.duration), w1.min(a.duration)));
                }
                reset_emptied(s, &emptied);
                self.selected.clear();
                s.modified = true;
                *tp.applied = f32::NAN;
            }
            Edit::Delete => {
                let Some(a) = s.animations.get_mut(ai) else { return };
                self.remember(ai, a, None);
                for c in &mut a.curves {
                    let t = tgt(c.target);
                    let picked: BTreeSet<i64> = self
                        .selected
                        .iter()
                        .filter(|k| k.target == t && k.id == c.id)
                        .map(|k| k.frame)
                        .collect();
                    if !picked.is_empty() {
                        c.keys.retain(|k| !picked.contains(&((k.time * fps).round() as i64)));
                    }
                }
                let emptied = drop_empty(a);
                reset_emptied(s, &emptied);
                self.selected.clear();
                s.modified = true;
                *tp.applied = f32::NAN;
            }
            // layer edits only mean something to a composition
            Edit::LayerSpan(..)
            | Edit::LayerSet(..)
            | Edit::LayerAdd(_)
            | Edit::LayerRemove(_)
            | Edit::LayerOrder(..)
            | Edit::LayerDup(_)
            | Edit::LayerSplit(_)
            | Edit::LayerSnap(..)
            | Edit::LayerMoveTo(..) => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timecode_counts_seconds_and_frames() {
        assert_eq!(timecode(0.0, 30.0), "00:00f");
        assert_eq!(timecode(1.5, 30.0), "01:15f");
        assert_eq!(timecode(65.0, 30.0), "1:05:00f");
    }

    #[test]
    fn zoom_never_goes_below_fit() {
        let mut tl = Timeline::default();
        tl.set_zoom(10.0, 50.0, 3.0, 1.0, 0.0);
        assert!(tl.pps.is_none());
        tl.set_zoom(200.0, 50.0, 3.0, 1.0, 100.0);
        assert_eq!(tl.pps, Some(200.0));
        assert!((tl.start - 0.5).abs() < 1e-6);
    }

    #[test]
    fn a_drag_is_one_undo_step() {
        let mut tl = Timeline::default();
        let a = Animation { name: "a".into(), fps: 30.0, duration: 1.0, ..Animation::default() };
        tl.remember(0, &a, Some("key-drag"));
        tl.remember(0, &a, Some("key-drag"));
        tl.remember(0, &a, Some("key-drag"));
        assert_eq!(tl.undo.len(), 1);
        tl.merge = None;
        tl.remember(0, &a, Some("key-drag"));
        assert_eq!(tl.undo.len(), 2);
        tl.remember(0, &a, None);
        tl.remember(0, &a, None);
        assert_eq!(tl.undo.len(), 4);
    }

    #[test]
    fn grid_steps_are_round_numbers() {
        assert_eq!(nice_step(0.17), 0.2);
        assert_eq!(nice_step(3.0), 5.0);
        assert_eq!(nice_step(0.01), 0.01);
        assert_eq!(step_decimals(0.2), 1);
        assert_eq!(step_decimals(5.0), 0);
    }

    #[test]
    fn a_layer_drag_keeps_its_content_put_when_trimmed_from_the_left() {
        let d = LayerDrag {
            li: 0,
            part: LayerPart::Left,
            origin_x: 0.0,
            start: 1.0,
            in_point: 0.5,
            out_point: 2.5,
        };
        // half a second later: the start and the in point both move on
        let (s, i, o) = layer_span_after(&d, 0.5, 1.0, 3.0, false, 0.1);
        assert!((s - 1.5).abs() < 1e-6 && (i - 1.0).abs() < 1e-6 && (o - 2.5).abs() < 1e-6);
        // dragged back past the source's start, the in point stops at zero
        let (s, i, _) = layer_span_after(&d, -2.0, 1.0, 3.0, false, 0.1);
        assert!((i - 0.0).abs() < 1e-6 && (s - 0.5).abs() < 1e-6);
        // the right end cannot pass the source's end unless it loops
        let r = LayerDrag { part: LayerPart::Right, ..d };
        let (_, _, o) = layer_span_after(&r, 5.0, 1.0, 3.0, false, 0.1);
        assert!((o - 3.0).abs() < 1e-6);
        let (_, _, o) = layer_span_after(&r, 5.0, 1.0, 3.0, true, 0.1);
        assert!((o - 7.5).abs() < 1e-6);
    }

    #[test]
    fn stepped_keys_draw_as_squares() {
        assert_eq!(half_of(Interp::Stepped), Half::Hold);
        assert_eq!(half_of(Interp::InverseStepped), Half::Hold);
        assert_eq!(half_of(Interp::Auto), Half::Linear);
    }
}
