//! The Audio Mixer palette, laid out like a video editor's: one channel
//! strip per sound layer of the current composition and a main-out bus.
//! A strip is a dB scale, a fader and live level meters (two bars for a
//! stereo source, side by side, one for mono), the level read out under
//! them, M and S buttons on top and the name in a box at the bottom. A
//! gutter at a strip's right edge drags it narrower, down to just the
//! meters and the name. EQ and effects come later.

use std::collections::{BTreeSet, HashMap};

use ayatsuri_format::LayerSource;
use egui::{
    Align2, Color32, CursorIcon, FontId, Rect, Response, Sense, Stroke, StrokeKind, Ui, pos2, vec2,
};

use crate::audio::Levels;
use crate::i18n::tr;
use crate::session::Session;
use crate::theme;
use crate::timeline::Timeline;

/// The widest a strip goes: scale, fader and meters.
const MAX_W: f32 = 104.0;
/// The narrowest: the meters and the name.
const MIN_W: f32 = 40.0;
/// The gutter between strips that drags the width.
const GUTTER: f32 = 6.0;
/// The scale's ends in dB.
const DB_TOP: f32 = 6.0;
const DB_BOTTOM: f32 = -48.0;
/// How fast a meter falls once the sound drops, in dB per second.
const FALL_DB_PER_S: f32 = 24.0;
/// How long the peak line stays before it falls.
const HOLD_S: f64 = 1.5;

/// The palette's own state: strip widths by source, solo picks, and the
/// meters as they are shown (falling and holding).
#[derive(Default)]
pub struct Mixer {
    widths: HashMap<String, f32>,
    main_width: Option<f32>,
    /// Layers soloed; anything else is silent while one is.
    pub solo: BTreeSet<usize>,
    shown: HashMap<MeterKey, Meter>,
    last_tick: Option<f64>,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum MeterKey {
    Layer(usize),
    Main,
}

/// One meter's display: the falling level and the held peak, per channel.
#[derive(Clone, Copy, Default)]
struct Meter {
    level: [f32; 2],
    peak: [f32; 2],
    peak_at: [f64; 2],
}

/// What a frame of the palette changed: a layer's level, mute or solo
/// (the caller restarts the voices), and the master, which the caller
/// saves and applies.
#[derive(Default)]
pub struct Changed {
    pub layers: bool,
    pub master: bool,
}

enum Change {
    Weight(usize, f32),
    Mute(usize, bool),
    Pan(usize, f32),
}

fn db(gain: f32) -> f32 {
    if gain <= 1e-5 { f32::NEG_INFINITY } else { 20.0 * gain.log10() }
}

fn gain(db: f32) -> f32 {
    if db <= DB_BOTTOM + 0.01 { 0.0 } else { 10f32.powf(db / 20.0) }
}

/// Where a dB value sits in a vertical span: the top is the loudest.
fn db_y(span: Rect, d: f32) -> f32 {
    let f = ((d.max(DB_BOTTOM) - DB_BOTTOM) / (DB_TOP - DB_BOTTOM)).clamp(0.0, 1.0);
    span.max.y - f * span.height()
}

fn readout(d: f32) -> String {
    if d.is_finite() { format!("{d:.1}") } else { "-\u{221e}".to_string() }
}

fn mix(a: Color32, b: Color32, t: f32) -> Color32 {
    let l = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgb(l(a.r(), b.r()), l(a.g(), b.g()), l(a.b(), b.b()))
}

/// A level meter: a dark track with the signal filled bottom-up in green,
/// amber above -12 dB and red above 0, and a line held at the recent peak.
fn meter_bar(ui: &Ui, r: Rect, level: f32, peak: f32) {
    let pal = theme::pal();
    let p = ui.painter();
    p.rect_filled(r, 1.5, Color32::from_black_alpha(110));
    let lvl_db = db(level);
    if lvl_db > DB_BOTTOM {
        let top = db_y(r, lvl_db);
        let bands = [
            (DB_BOTTOM, -12.0, pal.warp_green),
            (-12.0, 0.0, pal.accent),
            (0.0, DB_TOP, pal.rotation_red),
        ];
        for (lo, hi, col) in bands {
            let y1 = db_y(r, lo);
            let y0 = db_y(r, hi).max(top);
            if y0 < y1 {
                p.rect_filled(
                    Rect::from_min_max(pos2(r.min.x + 1.0, y0), pos2(r.max.x - 1.0, y1)),
                    0.0,
                    col,
                );
            }
        }
    }
    let pk_db = db(peak);
    if pk_db > DB_BOTTOM {
        let y = db_y(r, pk_db);
        p.line_segment(
            [pos2(r.min.x + 1.0, y), pos2(r.max.x - 1.0, y)],
            Stroke::new(1.5, if pk_db > 0.0 { pal.rotation_red } else { pal.text }),
        );
    }
    // faint ticks every 6 dB
    let mut d = DB_TOP;
    while d >= DB_BOTTOM {
        let y = db_y(r, d);
        p.line_segment(
            [pos2(r.min.x, y), pos2(r.min.x + 2.0, y)],
            Stroke::new(1.0, Color32::from_white_alpha(40)),
        );
        d -= 6.0;
    }
}

/// The dB scale beside a fader, right-aligned to `right_x`.
fn scale(ui: &Ui, right_x: f32, span: Rect) {
    let pal = theme::pal();
    let p = ui.painter();
    for d in [6.0f32, 0.0, -6.0, -12.0, -18.0, -24.0, -36.0, -48.0] {
        let y = db_y(span, d);
        let text = if d == DB_BOTTOM { "-\u{221e}".to_string() } else { format!("{d:.0}") };
        p.text(
            pos2(right_x - 3.0, y),
            Align2::RIGHT_CENTER,
            text,
            FontId::proportional(9.0),
            if d == 0.0 { pal.text } else { pal.text_faint },
        );
        p.line_segment(
            [pos2(right_x - 2.0, y), pos2(right_x, y)],
            Stroke::new(1.0, pal.text_faint.gamma_multiply(0.6)),
        );
    }
}

/// A vertical fader on the dB scale: the track in the accent colour, a
/// handle that drags, a line across at 0 dB. Returns the new gain while it
/// changes, and whether a drag just stopped.
fn fader(ui: &mut Ui, id: egui::Id, span: Rect, gain_now: f32) -> (Option<f32>, bool) {
    let pal = theme::pal();
    let track = Rect::from_center_size(span.center(), vec2(6.0, span.height()));
    let resp = ui.interact(span.expand2(vec2(6.0, 4.0)), id, Sense::click_and_drag());
    let p = ui.painter();
    p.rect_filled(track, 3.0, mix(pal.accent, pal.panel_bg, 0.55));
    let y0 = db_y(span, 0.0);
    p.line_segment(
        [pos2(span.min.x - 2.0, y0), pos2(span.max.x + 2.0, y0)],
        Stroke::new(1.0, pal.text_faint),
    );
    let y = db_y(span, db(gain_now));
    p.rect_filled(
        Rect::from_min_max(pos2(track.min.x, y), pos2(track.max.x, track.max.y)),
        3.0,
        pal.accent,
    );
    let handle = Rect::from_center_size(pos2(span.center().x, y), vec2(18.0, 10.0));
    p.rect_filled(
        handle,
        2.0,
        if resp.hovered() || resp.dragged() { pal.text } else { mix(pal.text, pal.panel_bg, 0.15) },
    );
    p.rect_stroke(handle, 2.0, Stroke::new(1.0, pal.border), StrokeKind::Outside);
    p.line_segment(
        [pos2(handle.min.x + 4.0, y), pos2(handle.max.x - 4.0, y)],
        Stroke::new(1.0, pal.border),
    );
    let mut out = None;
    if (resp.dragged() || resp.clicked())
        && let Some(ptr) = resp.interact_pointer_pos()
    {
        let f = ((span.max.y - ptr.y) / span.height()).clamp(0.0, 1.0);
        out = Some(gain(DB_BOTTOM + f * (DB_TOP - DB_BOTTOM)));
    }
    if resp.double_clicked() {
        out = Some(1.0);
    }
    (out, resp.drag_stopped())
}

/// A small square toggle with a letter, as mixers have.
fn letter(ui: &mut Ui, letter: &str, on: bool, tip: &str, on_col: Color32) -> Response {
    let pal = theme::pal();
    let (r, resp) = ui.allocate_exact_size(vec2(18.0, 16.0), Sense::click());
    let p = ui.painter();
    p.rect_filled(r, 2.0, if on { on_col } else { pal.chrome });
    p.rect_stroke(r, 2.0, Stroke::new(1.0, pal.border), StrokeKind::Inside);
    p.text(
        r.center(),
        Align2::CENTER_CENTER,
        letter,
        FontId::proportional(10.0),
        if on { Color32::WHITE } else { pal.text_faint },
    );
    resp.on_hover_text(tip)
}

/// The gutter after a strip: drag it to make the strip wider or narrower.
fn gutter(ui: &mut Ui, h: f32, mut on_drag: impl FnMut(f32)) {
    let pal = theme::pal();
    let (r, resp) = ui.allocate_exact_size(vec2(GUTTER, h), Sense::drag());
    let resp = resp.on_hover_cursor(CursorIcon::ResizeHorizontal);
    let col = if resp.hovered() || resp.dragged() { pal.accent } else { pal.border };
    ui.painter().line_segment(
        [pos2(r.center().x, r.min.y + 4.0), pos2(r.center().x, r.max.y - 4.0)],
        Stroke::new(1.0, col),
    );
    if resp.dragged() {
        on_drag(resp.drag_delta().x);
    }
}

struct StripIn<'a> {
    id: egui::Id,
    name: &'a str,
    stereo: bool,
    gain: f32,
    /// -1 left to 1 right; None for a bus without a pan.
    pan: Option<f32>,
    meter: Meter,
    mute: Option<bool>,
    solo: Option<bool>,
    silent: bool,
    footer: Option<&'a str>,
}

#[derive(Default)]
struct StripOut {
    gain: Option<f32>,
    pan: Option<f32>,
    drag_stopped: bool,
    mute_clicked: bool,
    solo_clicked: bool,
}

/// A pan knob: a dial whose pointer swings from left to right; drag up or
/// down (or sideways) to turn it, double-click to centre. Returns the new
/// pan while it changes.
fn knob(ui: &mut Ui, id: egui::Id, center: egui::Pos2, pan: f32) -> Option<f32> {
    let pal = theme::pal();
    let r = 9.0;
    let rect = Rect::from_center_size(center, vec2(r * 2.0 + 6.0, r * 2.0 + 6.0));
    let resp = ui.interact(rect, id, Sense::click_and_drag()).on_hover_cursor(CursorIcon::Grab);
    let p = ui.painter();
    p.circle_filled(center, r, pal.chrome);
    p.circle_stroke(
        center,
        r,
        Stroke::new(1.0, if resp.hovered() { pal.accent } else { pal.border }),
    );
    // the pointer: straight up at centre, 135 degrees either way at the ends
    let a = -std::f32::consts::FRAC_PI_2 + pan.clamp(-1.0, 1.0) * std::f32::consts::FRAC_PI_4 * 3.0;
    let tip = center + vec2(a.cos(), a.sin()) * (r - 2.0);
    p.line_segment([center, tip], Stroke::new(2.0, pal.accent));
    let mut out = None;
    if resp.dragged() {
        let d = resp.drag_delta();
        let np = (pan + (d.x - d.y) / 80.0).clamp(-1.0, 1.0);
        if np != pan {
            out = Some(np);
        }
    }
    if resp.double_clicked() {
        out = Some(0.0);
    }
    out
}

fn pan_text(pan: f32) -> String {
    format!("{:.1}", pan * 100.0)
}

/// One channel strip in `rect`. Narrow strips keep the meters and the
/// name; wide ones add the scale and the fader.
fn strip_ui(ui: &mut Ui, rect: Rect, s: StripIn) -> StripOut {
    let pal = theme::pal();
    let mut out = StripOut::default();
    let wide = rect.width() >= 80.0;
    let pad = 4.0;
    let mut top = rect.min.y + pad;
    // top: the pan knob, centred; under it L and R at the ends of a line
    // with the pan marked on it, then the reading
    if let Some(pan) = s.pan {
        let c = pos2(rect.center().x, top + 12.0);
        if let Some(np) = knob(ui, s.id.with("pan"), c, pan) {
            out.pan = Some(np);
        }
        let line_y = top + 29.0;
        let (lx, rx) = (rect.min.x + pad + 11.0, rect.max.x - pad - 11.0);
        let side = FontId::proportional(9.0);
        let p = ui.painter();
        p.text(pos2(lx - 3.0, line_y), Align2::RIGHT_CENTER, "L", side.clone(), pal.text_faint);
        p.text(pos2(rx + 3.0, line_y), Align2::LEFT_CENTER, "R", side, pal.text_faint);
        if rx > lx {
            p.line_segment([pos2(lx, line_y), pos2(rx, line_y)], Stroke::new(1.0, pal.text_faint));
            let mx = lx + (pan.clamp(-1.0, 1.0) + 1.0) / 2.0 * (rx - lx);
            p.line_segment(
                [pos2(mx, line_y - 4.0), pos2(mx, line_y + 4.0)],
                Stroke::new(2.0, pal.accent),
            );
            let cx = (lx + rx) / 2.0;
            p.line_segment(
                [pos2(cx, line_y - 2.0), pos2(cx, line_y + 2.0)],
                Stroke::new(1.0, pal.text_faint),
            );
        }
        p.text(
            pos2(rect.center().x, line_y + 12.0),
            Align2::CENTER_CENTER,
            pan_text(pan),
            FontId::proportional(10.0),
            pal.accent,
        );
        top += 50.0;
    }
    // then M and S
    if s.mute.is_some() || s.solo.is_some() {
        let mut row = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(Rect::from_min_size(
                    pos2(rect.min.x + pad, top),
                    vec2(rect.width() - pad * 2.0, 16.0),
                ))
                .layout(egui::Layout::left_to_right(egui::Align::Center)),
        );
        row.spacing_mut().item_spacing.x = 3.0;
        if let Some(m) = s.mute
            && letter(&mut row, "M", m, tr("Mute"), pal.rotation_red).clicked()
        {
            out.mute_clicked = true;
        }
        if let Some(so) = s.solo
            && letter(&mut row, "S", so, tr("Solo"), pal.accent).clicked()
        {
            out.solo_clicked = true;
        }
        top += 20.0;
    }
    // bottom: the name box, the footer line and the readout
    let name_h = 18.0;
    let footer_h = if s.footer.is_some() { 12.0 } else { 0.0 };
    let readout_h = 14.0;
    let name_rect = Rect::from_min_max(
        pos2(rect.min.x + pad, rect.max.y - pad - name_h),
        pos2(rect.max.x - pad, rect.max.y - pad),
    );
    ui.painter().rect_filled(name_rect, 2.0, pal.chrome);
    ui.painter().rect_stroke(name_rect, 2.0, Stroke::new(1.0, pal.border), StrokeKind::Inside);
    let shown = crate::timeline::fit_text(
        ui.painter(),
        s.name,
        &FontId::proportional(10.5),
        pal.text,
        name_rect.width() - 6.0,
    );
    ui.painter().text(
        name_rect.center(),
        Align2::CENTER_CENTER,
        shown,
        FontId::proportional(10.5),
        if s.silent { pal.text_faint } else { pal.text },
    );
    ui.interact(name_rect, s.id.with("name"), Sense::hover()).on_hover_text(s.name);
    let mut bottom = name_rect.min.y - 2.0;
    if let Some(f) = s.footer {
        let shown = crate::timeline::fit_text(
            ui.painter(),
            f,
            &FontId::proportional(9.0),
            pal.text_faint,
            rect.width() - pad * 2.0,
        );
        ui.painter().text(
            pos2(rect.center().x, bottom - footer_h / 2.0),
            Align2::CENTER_CENTER,
            shown,
            FontId::proportional(9.0),
            pal.text_faint,
        );
        bottom -= footer_h;
    }
    ui.painter().text(
        pos2(rect.center().x, bottom - readout_h / 2.0),
        Align2::CENTER_CENTER,
        readout(db(s.gain)),
        FontId::proportional(10.5),
        pal.accent,
    );
    bottom -= readout_h + 2.0;
    // the middle: scale | fader | meters, or just the meters; the L and R
    // bars of a stereo source sit together with a gap between
    let span =
        Rect::from_min_max(pos2(rect.min.x + pad, top + 6.0), pos2(rect.max.x - pad, bottom - 8.0));
    if span.height() < 30.0 {
        return out;
    }
    let meter_w = if s.stereo { 7.0 } else { 9.0 };
    let meters_w = if s.stereo { meter_w * 2.0 + 2.0 } else { meter_w };
    let meters_x = if wide { span.max.x - meters_w } else { span.center().x - meters_w / 2.0 };
    let mut bars =
        vec![Rect::from_min_max(pos2(meters_x, span.min.y), pos2(meters_x + meter_w, span.max.y))];
    if s.stereo {
        bars.push(Rect::from_min_max(
            pos2(meters_x + meter_w + 2.0, span.min.y),
            pos2(meters_x + meters_w, span.max.y),
        ));
    }
    for (c, b) in bars.iter().enumerate() {
        let (lvl, pk) = if s.silent {
            (0.0, 0.0)
        } else {
            (
                s.meter.level.get(c).copied().unwrap_or(0.0),
                s.meter.peak.get(c).copied().unwrap_or(0.0),
            )
        };
        meter_bar(ui, *b, lvl, pk);
        if s.stereo {
            ui.painter().text(
                pos2(b.center().x, span.max.y + 5.0),
                Align2::CENTER_CENTER,
                if c == 0 { "L" } else { "R" },
                FontId::proportional(8.0),
                pal.text_faint,
            );
        }
    }
    if wide {
        let scale_w = 22.0;
        scale(ui, span.min.x + scale_w, span);
        let fader_span = Rect::from_min_max(
            pos2(span.min.x + scale_w + 6.0, span.min.y),
            pos2(meters_x - 8.0, span.max.y),
        );
        let (g, stopped) = fader(ui, s.id.with("fader"), fader_span, s.gain);
        out.gain = g;
        out.drag_stopped = stopped;
    }
    out
}

impl Mixer {
    /// Fold the fresh peaks into the shown meters: rise at once, fall at
    /// the set rate, hold the peak line a while.
    fn tick(&mut self, now: f64, fresh: &Levels) {
        let dt = self.last_tick.map_or(0.0, |t| (now - t).max(0.0)) as f32;
        self.last_tick = Some(now);
        let fall = 10f32.powf(-FALL_DB_PER_S * dt / 20.0);
        // layers that stopped sounding fall to nothing
        let stale: Vec<MeterKey> = self
            .shown
            .keys()
            .copied()
            .filter(|k| matches!(k, MeterKey::Layer(li) if !fresh.layers.contains_key(li)))
            .collect();
        let mut update = |key: MeterKey, new: [f32; 2]| {
            let m = self.shown.entry(key).or_default();
            for c in 0..2 {
                let (lvl, pk, at) = (m.level.get_mut(c), m.peak.get_mut(c), m.peak_at.get_mut(c));
                let (Some(lvl), Some(pk), Some(at)) = (lvl, pk, at) else { continue };
                let n = new.get(c).copied().unwrap_or(0.0);
                *lvl = (*lvl * fall).max(n);
                if n >= *pk {
                    *pk = n;
                    *at = now;
                } else if now - *at > HOLD_S {
                    *pk *= fall;
                }
            }
        };
        for (li, pk) in &fresh.layers {
            update(MeterKey::Layer(*li), *pk);
        }
        update(MeterKey::Main, fresh.main);
        for k in stale {
            update(k, [0.0, 0.0]);
        }
    }

    /// Draw the palette. `master` is the editor's volume in percent;
    /// `levels` are the peaks since the last frame; `route` is where the
    /// main-out bus goes.
    pub fn show(
        &mut self,
        ui: &mut Ui,
        s: &mut Session,
        tl: &mut Timeline,
        master: &mut u32,
        levels: Levels,
        route: &str,
    ) -> Changed {
        let now = ui.input(|i| i.time);
        self.tick(now, &levels);
        let mut changed = Changed::default();
        let Some(ci) = s.current_comp.filter(|ci| *ci < s.compositions.len()) else {
            ui.weak(tr("Open a composition to mix its sound layers."));
            return changed;
        };
        let Some(comp) = s.compositions.get(ci).cloned() else { return changed };
        // the strips: each audio layer, with its file name and whether its
        // clip is stereo
        let strips: Vec<(usize, String, bool)> = comp
            .layers
            .iter()
            .enumerate()
            .filter_map(|(li, l)| match &l.source {
                LayerSource::Audio(p) => {
                    let stereo = s.audio.get(p).is_some_and(|c| c.channels >= 2);
                    let name = std::path::Path::new(p)
                        .file_stem()
                        .map(|f| f.to_string_lossy().into_owned())
                        .unwrap_or_else(|| p.clone());
                    Some((li, name, stereo))
                }
                _ => None,
            })
            .collect();
        let mut changes: Vec<Change> = Vec::new();
        let h = ui.available_height().max(160.0);
        let any_solo = !self.solo.is_empty();
        egui::ScrollArea::horizontal().id_salt("mixer-strips").show(ui, |ui| {
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                for (li, name, stereo) in &strips {
                    let Some(l) = comp.layers.get(*li) else { continue };
                    let key = l.source.name().to_string();
                    let w = *self.widths.get(&key).unwrap_or(&MAX_W);
                    let meter = self.shown.get(&MeterKey::Layer(*li)).copied().unwrap_or_default();
                    let (rect, _) = ui.allocate_exact_size(vec2(w, h), Sense::hover());
                    let mut strip = ui.new_child(egui::UiBuilder::new().max_rect(rect));
                    strip.set_clip_rect(rect);
                    let soloed = self.solo.contains(li);
                    let out = strip_ui(
                        &mut strip,
                        rect,
                        StripIn {
                            id: ui.id().with(("mixer-strip", li)),
                            name,
                            stereo: *stereo,
                            gain: l.weight,
                            pan: Some(l.pan),
                            meter,
                            mute: Some(!l.enabled),
                            solo: Some(soloed),
                            silent: !l.enabled || (any_solo && !soloed),
                            footer: None,
                        },
                    );
                    if let Some(g) = out.gain {
                        changes.push(Change::Weight(*li, g));
                    }
                    if let Some(pn) = out.pan {
                        changes.push(Change::Pan(*li, pn));
                    }
                    if out.drag_stopped {
                        tl.unmerge();
                    }
                    if out.mute_clicked {
                        changes.push(Change::Mute(*li, !l.enabled));
                    }
                    if out.solo_clicked {
                        if !self.solo.remove(li) {
                            self.solo.insert(*li);
                        }
                        changed.layers = true;
                    }
                    gutter(ui, h, |dx| {
                        self.widths.insert(key.clone(), (w + dx).clamp(MIN_W, MAX_W));
                    });
                }
                // the main-out bus
                let w = self.main_width.unwrap_or(MAX_W);
                let meter = self.shown.get(&MeterKey::Main).copied().unwrap_or_default();
                let (rect, _) = ui.allocate_exact_size(vec2(w, h), Sense::hover());
                let mut strip = ui.new_child(egui::UiBuilder::new().max_rect(rect));
                strip.set_clip_rect(rect);
                let main_name = tr("Main out");
                let out = strip_ui(
                    &mut strip,
                    rect,
                    StripIn {
                        id: ui.id().with("mixer-main"),
                        name: main_name,
                        stereo: true,
                        gain: (*master).min(200) as f32 / 100.0,
                        pan: None,
                        meter,
                        mute: None,
                        solo: None,
                        silent: false,
                        footer: Some(route),
                    },
                );
                if let Some(g) = out.gain {
                    let pct = (g * 100.0).round().clamp(0.0, 200.0) as u32;
                    if pct != *master {
                        *master = pct;
                        changed.master = true;
                    }
                }
                gutter(ui, h, |dx| {
                    self.main_width = Some((w + dx).clamp(MIN_W, MAX_W));
                });
            });
        });
        if strips.is_empty() {
            ui.weak(tr("No sound layers in this composition. Add one with File > Import Audio…"));
        }
        if !changes.is_empty()
            && let Some(c) = s.compositions.get(ci).cloned()
        {
            tl.remember_comp(ci, &c, Some("mixer"));
            if let Some(c) = s.compositions.get_mut(ci) {
                for ch in changes {
                    match ch {
                        Change::Weight(li, w) => {
                            if let Some(l) = c.layers.get_mut(li) {
                                l.weight = w.clamp(0.0, 2.0);
                            }
                        }
                        Change::Mute(li, on) => {
                            if let Some(l) = c.layers.get_mut(li) {
                                l.enabled = on;
                            }
                        }
                        Change::Pan(li, pn) => {
                            if let Some(l) = c.layers.get_mut(li) {
                                l.pan = pn.clamp(-1.0, 1.0);
                            }
                        }
                    }
                }
            }
            s.modified = true;
            changed.layers = true;
        }
        changed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decibels_round_trip_and_the_floor_is_silence() {
        assert!(db(1.0).abs() < 1e-6);
        assert!((db(gain(-12.0)) + 12.0).abs() < 1e-3);
        assert_eq!(gain(DB_BOTTOM), 0.0);
        assert_eq!(db(0.0), f32::NEG_INFINITY);
    }

    #[test]
    fn the_scale_puts_the_top_at_the_top() {
        let r = Rect::from_min_max(pos2(0.0, 0.0), pos2(10.0, 100.0));
        assert_eq!(db_y(r, DB_TOP), 0.0);
        assert_eq!(db_y(r, DB_BOTTOM), 100.0);
        assert!(db_y(r, 0.0) < db_y(r, -12.0));
    }
}
