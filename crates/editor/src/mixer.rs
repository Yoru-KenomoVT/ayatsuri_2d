//! The Audio Mixer palette: one strip per sound layer of the current
//! composition, with a fader for its level and a mute, and a master strip
//! for the editor's own volume. EQ and effects come later.

use ayatsuri_format::LayerSource;
use egui::{Align, Layout, RichText, Ui, vec2};

use crate::i18n::tr;
use crate::session::Session;
use crate::theme;
use crate::timeline::Timeline;

const STRIP_W: f32 = 64.0;

/// A strip's worth of changes, applied after drawing.
enum Change {
    Weight(usize, f32),
    Mute(usize, bool),
}

/// What a frame of the palette changed: a layer's level or mute, and the
/// master, which the caller saves and applies.
#[derive(Default)]
pub struct Changed {
    pub layers: bool,
    pub master: bool,
}

/// Draw the palette. `master` is the editor's volume in percent.
pub fn show(
    ui: &mut Ui,
    s: &mut Session,
    tl: &mut Timeline,
    master: &mut u32,
    route: &str,
) -> Changed {
    let pal = theme::pal();
    let Some(ci) = s.current_comp.filter(|ci| *ci < s.compositions.len()) else {
        ui.weak(tr("Open a composition to mix its sound layers."));
        return Changed::default();
    };
    let Some(comp) = s.compositions.get(ci).cloned() else { return Changed::default() };
    let strips: Vec<(usize, String)> = comp
        .layers
        .iter()
        .enumerate()
        .filter_map(|(li, l)| match &l.source {
            LayerSource::Audio(p) => Some((
                li,
                std::path::Path::new(p)
                    .file_stem()
                    .map(|f| f.to_string_lossy().into_owned())
                    .unwrap_or_else(|| p.clone()),
            )),
            _ => None,
        })
        .collect();
    let mut changes: Vec<Change> = Vec::new();
    let mut master_changed = false;
    let fader_h = (ui.available_height() - 70.0).clamp(60.0, 220.0);
    egui::ScrollArea::horizontal().id_salt("mixer-strips").show(ui, |ui| {
        ui.horizontal_top(|ui| {
            for (li, name) in &strips {
                let Some(l) = comp.layers.get(*li) else { continue };
                ui.allocate_ui_with_layout(
                    vec2(STRIP_W, fader_h + 66.0),
                    Layout::top_down(Align::Center),
                    |ui| {
                        let shown = crate::timeline::fit_text(
                            ui.painter(),
                            name,
                            &egui::FontId::proportional(11.0),
                            pal.text,
                            STRIP_W - 6.0,
                        );
                        ui.label(RichText::new(shown).size(11.0)).on_hover_text(name);
                        let mut pct = l.weight * 100.0;
                        let r = ui.add(
                            egui::Slider::new(&mut pct, 0.0..=100.0)
                                .vertical()
                                .show_value(false)
                                .trailing_fill(true),
                        );
                        if r.changed() {
                            changes.push(Change::Weight(*li, pct / 100.0));
                        }
                        if r.drag_stopped() {
                            tl.unmerge();
                        }
                        ui.label(
                            RichText::new(format!("{pct:.0} %")).size(10.5).color(pal.text_faint),
                        );
                        let icon = if l.enabled {
                            egui_phosphor::regular::SPEAKER_HIGH
                        } else {
                            egui_phosphor::regular::SPEAKER_SLASH
                        };
                        let text = RichText::new(icon).size(14.0).color(if l.enabled {
                            pal.text
                        } else {
                            pal.rotation_red
                        });
                        if ui
                            .add(egui::Button::new(text).min_size(vec2(24.0, 20.0)))
                            .on_hover_text(if l.enabled { tr("Mute") } else { tr("Unmute") })
                            .clicked()
                        {
                            changes.push(Change::Mute(*li, !l.enabled));
                        }
                    },
                );
                ui.separator();
            }
            // the main-out bus: the editor's own level and where it goes
            ui.allocate_ui_with_layout(
                vec2(STRIP_W, fader_h + 66.0),
                Layout::top_down(Align::Center),
                |ui| {
                    ui.label(RichText::new(tr("Main out")).size(11.0).strong());
                    let mut v = (*master).min(100);
                    let r = ui.add(
                        egui::Slider::new(&mut v, 0..=100)
                            .vertical()
                            .show_value(false)
                            .trailing_fill(true),
                    );
                    if r.changed() {
                        *master = v;
                        master_changed = true;
                    }
                    ui.label(RichText::new(format!("{v} %")).size(10.5).color(pal.text_faint));
                    // where the bus goes: the device that is open, or the one picked
                    let shown = crate::timeline::fit_text(
                        ui.painter(),
                        route,
                        &egui::FontId::proportional(10.0),
                        pal.text_faint,
                        STRIP_W - 4.0,
                    );
                    ui.label(RichText::new(shown).size(10.0).color(pal.text_faint))
                        .on_hover_text(route);
                },
            );
        });
    });
    if strips.is_empty() {
        ui.weak(tr("No sound layers in this composition. Add one with File > Import Audio…"));
    }
    let layers_changed = !changes.is_empty();
    if layers_changed && let Some(c) = s.compositions.get(ci).cloned() {
        tl.remember_comp(ci, &c, Some("mixer"));
        if let Some(c) = s.compositions.get_mut(ci) {
            for ch in changes {
                match ch {
                    Change::Weight(li, w) => {
                        if let Some(l) = c.layers.get_mut(li) {
                            l.weight = w.clamp(0.0, 1.0);
                        }
                    }
                    Change::Mute(li, on) => {
                        if let Some(l) = c.layers.get_mut(li) {
                            l.enabled = on;
                        }
                    }
                }
            }
        }
        s.modified = true;
    }
    Changed { layers: layers_changed, master: master_changed }
}
