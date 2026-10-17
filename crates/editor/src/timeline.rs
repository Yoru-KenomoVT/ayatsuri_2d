//! The Timeline palette: a dope sheet in the After Effects layout over the
//! current scene's curves. Names on the left (the scene, parameter folders,
//! parameters, parts), lanes of keys on the right under a frame ruler, a
//! navigator for zoom and scroll, a work area bar, the playhead, and the
//! palette's own undo of key edits.

use std::collections::{BTreeMap, BTreeSet};

use ayatsuri_format::{Animation, CurveTarget, Interp, Key};
use egui::{
    Align, Align2, Color32, CursorIcon, FontId, Id, Layout, Painter, Pos2, Rect, Sense, Shape,
    Stroke, Ui, UiBuilder, pos2, vec2,
};

use crate::i18n::tr;
use crate::session::Session;
use crate::theme;

const HEADER_H: f32 = 48.0;
const COLHDR_H: f32 = 20.0;
const FOOTER_H: f32 = 22.0;
const ROW_H: f32 = 19.0;
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
    Auto,
    Linear,
    Hold,
    Bezier,
}

fn half_of(i: Interp) -> Half {
    match i {
        Interp::Auto => Half::Auto,
        Interp::Linear => Half::Linear,
        Interp::Stepped | Interp::InverseStepped => Half::Hold,
        Interp::Bezier { .. } => Half::Bezier,
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
fn fit_text(p: &Painter, text: &str, font: &FontId, color: Color32, max_w: f32) -> String {
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
            Half::Auto => {
                let h = 0.8 * r;
                (0..=10)
                    .map(|i| {
                        let a =
                            -std::f32::consts::FRAC_PI_2 + std::f32::consts::PI * i as f32 / 10.0;
                        c + vec2(sign * a.cos() * h, a.sin() * h)
                    })
                    .collect()
            }
        };
        let pts = if sign < 0.0 { pts.into_iter().rev().collect() } else { pts };
        p.add(Shape::convex_polygon(pts, fill, outline));
    }
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
}

impl Timeline {
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

        if s.animations.is_empty() {
            let mut h = ui.new_child(
                UiBuilder::new()
                    .max_rect(rect.shrink(8.0))
                    .layout(Layout::left_to_right(Align::Min)),
            );
            h.weak(tr("No animation yet."));
            if h.button(tr("New Animation…")).clicked() {
                request = Some(Request::NewScene);
            }
            return request;
        }
        if s.current_anim.is_none_or(|i| i >= s.animations.len()) {
            s.current_anim = Some(0);
        }
        let ai = s.current_anim.unwrap_or(0);
        let anim = s.animations.get(ai).cloned()?;
        let fps = anim.fps.max(1.0);
        let frame_s = 1.0 / fps;
        let duration = anim.duration.max(frame_s);
        let mut edits: Vec<Edit> = Vec::new();

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
        if h.add(egui::Button::new(egui_phosphor::regular::PLUS).frame(false))
            .on_hover_text(tr("New Animation…"))
            .clicked()
        {
            request = Some(Request::NewScene);
        }
        let w = (pick.width() - 26.0).clamp(40.0, 140.0);
        egui::ComboBox::from_id_salt("tl-scene")
            .selected_text(fit_text(
                &p,
                &anim.name,
                &FontId::proportional(12.0),
                pal.text,
                w - 24.0,
            ))
            .width(w)
            .show_ui(&mut h, |ui| {
                for (i, a) in s.animations.iter().enumerate() {
                    if ui.selectable_label(i == ai, &a.name).clicked() {
                        s.current_anim = Some(i);
                        *tp.applied = f32::NAN;
                    }
                }
            });

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
        if icon_button(&mut h, egui_phosphor::regular::RECORD, tr("Auto key"), *tp.autokey) {
            *tp.autokey = !*tp.autokey;
        }
        let looped = anim.looped;
        if icon_button(&mut h, egui_phosphor::regular::REPEAT, tr("Loop"), looped)
            && let Some(a) = s.animations.get_mut(ai)
        {
            a.looped = !looped;
            s.modified = true;
        }
        h.add_space(6.0);
        if icon_button(
            &mut h,
            egui_phosphor::regular::ROWS,
            tr("Show every parameter"),
            self.show_all,
        ) {
            self.show_all = !self.show_all;
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
        let rows = self.rows(s, &anim);
        let total_h = rows.len() as f32 * ROW_H;
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
        for (i, row) in rows.iter().enumerate() {
            let y = rows_top - self.scroll_y + i as f32 * ROW_H;
            if y + ROW_H < rows_top || y > rows_bottom {
                continue;
            }
            let rl = Rect::from_min_size(pos2(left.min.x, y), vec2(left.width(), ROW_H));
            let rr = Rect::from_min_size(pos2(lanes.min.x, y), vec2(lanes.width(), ROW_H));
            let cy = y + ROW_H / 2.0;
            let selected_param =
                matches!(row.kind, RowKind::Param(pi) if *tp.sel_param == Some(pi));
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
                    lp.text(
                        pos2(indent + 2.0, cy),
                        Align2::LEFT_CENTER,
                        fit_text(&lp, &row.label, &name_font, pal.text, value_x - indent - 8.0),
                        name_font.clone(),
                        pal.text,
                    );
                    let vr =
                        Rect::from_min_size(pos2(value_x, y + 1.0), vec2(VALUE_W, ROW_H - 2.0));
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
                    let bar = Rect::from_min_max(
                        pos2(tm.x(0.0), y + 4.0),
                        pos2(tm.x(duration), y + ROW_H - 4.0),
                    );
                    gp.rect_filled(bar.intersect(rr), 3.0, mix(pal.track, pal.accent, 0.35));
                    let end =
                        Rect::from_center_size(pos2(tm.x(duration), cy), vec2(8.0, ROW_H - 4.0));
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
                RowKind::Folder(name) => {
                    let open = rows.get(i + 1).is_some_and(|n| n.depth > row.depth);
                    twirl(&lp, pos2(indent + 5.0, cy), open, pal.text_faint);
                    let r_fold = ui.interact(rl, ui.id().with(("tl-fold", i)), Sense::click());
                    if r_fold.clicked() {
                        toggles.push((name.clone(), open));
                    }
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
                    // a hollow diamond wherever any child has a key
                    let mut frames: BTreeSet<i64> = BTreeSet::new();
                    for (target, id) in &row.children {
                        if let Some(c) =
                            anim.curves.iter().find(|c| c.target == *target && &c.id == id)
                        {
                            frames.extend(c.keys.iter().map(|k| (k.time * fps).round() as i64));
                        }
                    }
                    for fr in frames {
                        let x = tm.x(fr as f32 * frame_s);
                        if x < rr.min.x - 8.0 || x > rr.max.x + 8.0 {
                            continue;
                        }
                        let c = pos2(x, cy);
                        gp.add(Shape::convex_polygon(
                            vec![
                                c + vec2(0.0, -4.5),
                                c + vec2(4.5, 0.0),
                                c + vec2(0.0, 4.5),
                                c + vec2(-4.5, 0.0),
                            ],
                            Color32::TRANSPARENT,
                            Stroke::new(1.0, pal.text_faint),
                        ));
                    }
                }
                RowKind::Param(_) | RowKind::Part(_) => {
                    let mut x = indent;
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

                    ui.set_clip_rect(rows_right.intersect(outer_clip));
                    for (ki, k) in keys.iter().enumerate() {
                        let x = tm.x(k.time);
                        if x < rr.min.x - 8.0 || x > rr.max.x + 8.0 {
                            continue;
                        }
                        let left_half = keys
                            .get(ki.wrapping_sub(1))
                            .map(|prev| half_of(prev.interp))
                            .unwrap_or(half_of(k.interp));
                        key_glyph(&gp, pos2(x, cy), KEY_PX, left_half, half_of(k.interp), pal.text);
                    }
                }
            }
        }
        ui.set_clip_rect(rect.intersect(outer_clip));
        for (name, was_open) in toggles {
            self.folded.insert(name, was_open);
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

        // ---- playhead ----
        let px = tm.x(*tp.time);
        if px >= lanes.min.x - 1.0 && px <= lanes.max.x + 1.0 {
            p.line_segment(
                [pos2(px, ruler.min.y + 10.0), pos2(px, rows_bottom)],
                Stroke::new(1.0, pal.accent),
            );
            p.add(Shape::convex_polygon(
                vec![
                    pos2(px - 6.0, ruler.min.y + 8.0),
                    pos2(px + 6.0, ruler.min.y + 8.0),
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
        let z0 = lanes.min.x + 26.0;
        let z1 = (z0 + 140.0).min(lanes.max.x - 26.0);
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
        if (r_zoom.dragged() || r_zoom.clicked())
            && let Some(ptr) = r_zoom.interact_pointer_pos()
        {
            let f = ((ptr.x - z0) / (z1 - z0).max(1.0)).clamp(0.0, 1.0);
            let npps = fit * (top / fit).powf(f);
            let at_px = (tm.x(*tp.time) - tm.x0).clamp(0.0, lanes.width());
            self.set_zoom(npps, fit, duration, *tp.time, at_px);
        }
        let status =
            format!("{} / {} f", (*tp.time * fps).round() as i64, (duration * fps).round() as i64);
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
            self.apply(s, ai, e, tp, fps);
        }
        request
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
                if let Some(c) = ayatsuri_core::anim::curve_mut(a, CurveTarget::PartOpacity, &id) {
                    ayatsuri_core::anim::set_key(c, *tp.time, v, fps, Interp::Auto);
                }
                s.modified = true;
                *tp.applied = f32::NAN;
            }
            Edit::Duration(d) => {
                if let Some(a) = s.animations.get_mut(ai) {
                    a.duration = d.max(1.0 / fps);
                    if let Some((w0, w1)) = a.work_area {
                        a.work_area = Some((w0.min(a.duration), w1.min(a.duration)));
                    }
                    s.modified = true;
                }
            }
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
    fn stepped_keys_draw_as_squares() {
        assert_eq!(half_of(Interp::Stepped), Half::Hold);
        assert_eq!(half_of(Interp::InverseStepped), Half::Hold);
        assert_eq!(half_of(Interp::Auto), Half::Auto);
    }
}
