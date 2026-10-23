//! The Expression palette: the model's exp3 expressions, made from the
//! pose or by hand. An expression is parameters held at a value and
//! blended over whatever the pose is (added, multiplied or overwritten),
//! faded in and out; a preview lays the picked one over the puppet.

use ayatsuri_format::{Expression, ExpressionBlend, ExpressionParam};
use egui::{Align, Layout, RichText, Ui, vec2};

use crate::i18n::tr;
use crate::session::Session;
use crate::theme;

/// Width of a value box: room for a sign, four digits and three decimals.
const VALUE_W: f32 = 76.0;

const BLENDS: [(&str, ExpressionBlend); 3] = [
    ("Add", ExpressionBlend::Add),
    ("Multiply", ExpressionBlend::Multiply),
    ("Overwrite", ExpressionBlend::Overwrite),
];

#[derive(Default)]
pub struct Expressions {
    /// The expression picked in the list.
    pub sel: Option<usize>,
    /// Lay the picked expression over the puppet.
    pub preview: bool,
    /// The Add Parameter popup while it is open.
    add: Option<AddParam>,
}

/// The Add Parameter popup: pick a parameter, set its value on a slider
/// over the parameter's range, choose the blend, then add it.
struct AddParam {
    pick: usize,
    value: f32,
    blend: ExpressionBlend,
    filter: String,
}

/// The value range a blend makes sense over, for a parameter's min and max.
fn value_range(blend: ExpressionBlend, lo: f32, hi: f32) -> std::ops::RangeInclusive<f32> {
    let span = (hi - lo).abs().max(1e-3);
    match blend {
        ExpressionBlend::Add => -span..=span,
        ExpressionBlend::Multiply => 0.0..=10.0,
        ExpressionBlend::Overwrite => lo.min(hi)..=hi.max(lo),
    }
}

impl Expressions {
    /// The parameter the Add Parameter popup is setting up, for the
    /// preview to lay over the pose with the rest of the expression.
    pub fn pending(&self, s: &Session) -> Option<ExpressionParam> {
        let add = self.add.as_ref()?;
        let p = s.puppet.model().parameters.get(add.pick)?;
        Some(ExpressionParam { id: p.id.clone(), value: add.value, blend: add.blend })
    }

    /// The parameters the pose moved off their defaults, as the expression
    /// Cubism would make from it: each an Add of the difference.
    fn from_pose(s: &Session) -> Vec<ExpressionParam> {
        let model = s.puppet.model();
        model
            .parameters
            .iter()
            .enumerate()
            .filter_map(|(pi, p)| {
                let v = s.puppet.value(pi);
                let eps = 0.1f32.powi(p.decimals.clamp(0, 6)) * 0.5;
                ((v - p.default).abs() > eps).then(|| ExpressionParam {
                    id: p.id.clone(),
                    value: v - p.default,
                    blend: ExpressionBlend::Add,
                })
            })
            .collect()
    }

    /// Draw the palette into the space left in `ui`.
    pub fn show(&mut self, ui: &mut Ui, s: &mut Session) {
        let pal = theme::pal();
        if self.sel.is_some_and(|i| i >= s.expressions.len()) {
            self.sel = None;
        }

        // ---- the list, with new / from pose / duplicate / delete ----
        ui.horizontal(|ui| {
            if ui
                .button(RichText::new(egui_phosphor::regular::PLUS).size(14.0))
                .on_hover_text(tr("New empty expression"))
                .clicked()
            {
                let n = s.expressions.len() + 1;
                s.expressions.push(Expression {
                    name: format!("expression{n}"),
                    fade_in: None,
                    fade_out: None,
                    params: Vec::new(),
                });
                self.sel = Some(s.expressions.len() - 1);
                s.modified = true;
                log::info!("expression{n}: new expression");
            }
            let has = self.sel.is_some();
            if ui
                .add_enabled(
                    has,
                    egui::Button::new(RichText::new(egui_phosphor::regular::COPY).size(14.0)),
                )
                .on_hover_text(tr("Duplicate"))
                .clicked()
                && let Some(i) = self.sel
                && let Some(e) = s.expressions.get(i).cloned()
            {
                let mut e = e;
                e.name = format!("{} copy", e.name);
                log::info!("{}: duplicated", e.name);
                s.expressions.insert(i + 1, e);
                self.sel = Some(i + 1);
                s.modified = true;
            }
            if ui
                .add_enabled(
                    has,
                    egui::Button::new(RichText::new(egui_phosphor::regular::TRASH).size(14.0)),
                )
                .on_hover_text(tr("Delete"))
                .clicked()
                && let Some(i) = self.sel
                && i < s.expressions.len()
            {
                let gone = s.expressions.remove(i);
                log::info!("{}: expression deleted", gone.name);
                self.sel = if s.expressions.is_empty() {
                    None
                } else {
                    Some(i.min(s.expressions.len() - 1))
                };
                s.modified = true;
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let on = self.preview && self.sel.is_some();
                let eye = RichText::new(egui_phosphor::regular::EYE).size(14.0).color(if on {
                    pal.accent
                } else {
                    pal.text
                });
                if ui
                    .add_enabled(has, egui::Button::new(eye))
                    .on_hover_text(tr("Preview: lay the expression over the pose"))
                    .clicked()
                {
                    self.preview = !self.preview;
                }
            });
        });
        if s.expressions.is_empty() {
            ui.weak(tr(
                "No expressions yet. Press + for an empty one, then add parameters or set it from the pose.",
            ));
            return;
        }
        // whole rows only, so the list never ends on a half-cut row; each
        // row spans the palette
        let row_h = ui.spacing().interact_size.y + ui.spacing().item_spacing.y;
        let rows_fit = ((ui.available_height() * 0.3).clamp(60.0, 200.0) / row_h).floor().max(2.0);
        let list_h = rows_fit.min(s.expressions.len() as f32) * row_h;
        egui::ScrollArea::vertical().id_salt("expr-list").max_height(list_h).show(ui, |ui| {
            ui.with_layout(Layout::top_down_justified(Align::Min), |ui| {
                for (i, e) in s.expressions.iter().enumerate() {
                    let label = format!("{}  {}", e.name, tr_count(e.params.len()));
                    let b = egui::Button::selectable(self.sel == Some(i), label)
                        .wrap_mode(egui::TextWrapMode::Truncate);
                    if ui.add(b).clicked() {
                        self.sel = Some(i);
                    }
                }
            });
        });
        ui.separator();
        let Some(i) = self.sel else {
            ui.weak(tr("Pick an expression to edit it."));
            return;
        };

        // ---- the picked expression: name, fades, parameters ----
        let model = s.puppet.model().clone();
        let names: Vec<String> = model
            .parameters
            .iter()
            .enumerate()
            .map(|(pi, p)| s.param_names.get(pi).cloned().unwrap_or_else(|| p.id.clone()))
            .collect();
        let mut edited: Option<bool> = None; // Some(merge)
        let mut next = s.expressions.get(i).cloned().unwrap_or_default();
        egui::Grid::new("expr-head").num_columns(2).spacing(vec2(8.0, 4.0)).show(ui, |ui| {
            ui.label(tr("Name"));
            if ui
                .add(egui::TextEdit::singleline(&mut next.name).desired_width(f32::INFINITY))
                .changed()
            {
                edited = Some(true);
            }
            ui.end_row();
            for (label, slot) in
                [(tr("Fade in"), &mut next.fade_in), (tr("Fade out"), &mut next.fade_out)]
            {
                ui.label(label);
                ui.horizontal(|ui| {
                    let mut on = slot.is_some();
                    if ui
                        .checkbox(&mut on, "")
                        .on_hover_text(tr("Off leaves it to the player (1 s)"))
                        .changed()
                    {
                        *slot = if on { Some(1.0) } else { None };
                        edited = Some(false);
                    }
                    if let Some(v) = slot
                        && ui
                            .add(egui::DragValue::new(v).range(0.0..=10.0).speed(0.02).suffix(" s"))
                            .changed()
                    {
                        edited = Some(true);
                    }
                });
                ui.end_row();
            }
        });
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            if ui
                .button(tr("Set from pose"))
                .on_hover_text(tr(
                    "Replace the parameters with the pose's differences from default",
                ))
                .clicked()
            {
                next.params = Self::from_pose(s);
                log::info!("{}: set from the pose, {} parameter(s)", next.name, next.params.len());
                edited = Some(false);
            }
            // add one parameter, through the popup
            if ui
                .add_enabled(
                    self.add.is_none() && !model.parameters.is_empty(),
                    egui::Button::new(tr("Add parameter…")),
                )
                .clicked()
            {
                // the first parameter the expression does not have yet
                let pick = model
                    .parameters
                    .iter()
                    .position(|p| !next.params.iter().any(|ep| ep.id == p.id))
                    .unwrap_or(0);
                self.add = Some(AddParam {
                    pick,
                    value: 0.0,
                    blend: ExpressionBlend::Add,
                    filter: String::new(),
                });
            }
        });
        if let Some(added) = self.add_param_window(ui, s, &names, &next.params) {
            next.params.push(added);
            edited = Some(false);
        }
        // a value being set is a value worth seeing: the preview comes on
        if self.add.is_some() {
            self.preview = true;
        }
        ui.add_space(4.0);
        let mut remove: Option<usize> = None;
        egui::ScrollArea::vertical().id_salt("expr-params").show(ui, |ui| {
            // the rows stripe across the palette, not just under the text
            ui.set_min_width(ui.available_width());
            egui::Grid::new("expr-params-grid")
                .num_columns(4)
                .spacing(vec2(6.0, 2.0))
                .striped(true)
                .show(ui, |ui| {
                    for (k, ep) in next.params.iter_mut().enumerate() {
                        let name = model
                            .parameters
                            .iter()
                            .position(|p| p.id == ep.id)
                            .and_then(|pi| names.get(pi).cloned())
                            .unwrap_or_else(|| ep.id.clone());
                        ui.label(name);
                        let (lo, hi) = model
                            .parameters
                            .iter()
                            .find(|p| p.id == ep.id)
                            .map_or((-1.0, 1.0), |p| (p.min, p.max));
                        let range = value_range(ep.blend, lo, hi);
                        let span = range.end() - range.start();
                        let r = ui.add_sized(
                            vec2(VALUE_W, ui.spacing().interact_size.y),
                            egui::DragValue::new(&mut ep.value)
                                .range(range)
                                .speed(span / 300.0)
                                .fixed_decimals(3),
                        );
                        if r.changed() {
                            edited = Some(true);
                        }
                        let shown =
                            BLENDS.iter().find(|(_, b)| *b == ep.blend).map_or("Add", |(l, _)| l);
                        egui::ComboBox::from_id_salt(("expr-blend", k))
                            .selected_text(tr(shown))
                            .width(90.0)
                            .show_ui(ui, |ui| {
                                for (label, b) in BLENDS {
                                    if ui.selectable_label(ep.blend == b, tr(label)).clicked()
                                        && ep.blend != b
                                    {
                                        ep.blend = b;
                                        edited = Some(false);
                                    }
                                }
                            });
                        if ui
                            .button(RichText::new(egui_phosphor::regular::X).size(12.0))
                            .on_hover_text(tr("Remove"))
                            .clicked()
                        {
                            remove = Some(k);
                        }
                        ui.end_row();
                    }
                });
        });
        if let Some(k) = remove
            && k < next.params.len()
        {
            next.params.remove(k);
            edited = Some(false);
        }
        if let Some(merge) = edited
            && s.expressions.get(i) != Some(&next)
        {
            if merge {
                self.preview = true;
            } else {
                log::info!("{}: expression edited", next.name);
            }
            if let Some(slot) = s.expressions.get_mut(i) {
                *slot = next;
            }
            s.modified = true;
        }
    }
}

impl Expressions {
    /// The Add Parameter popup. Returns the parameter to add when Add is
    /// pressed; closes itself on Add, Cancel or the window's close box.
    fn add_param_window(
        &mut self,
        ui: &mut Ui,
        s: &Session,
        names: &[String],
        have: &[ExpressionParam],
    ) -> Option<ExpressionParam> {
        let Some(add) = &mut self.add else { return None };
        let model = s.puppet.model();
        let pal = theme::pal();
        let mut open = true;
        let mut done: Option<Option<ExpressionParam>> = None;
        // as wide as the window allows, and no wider: long names truncate
        let width = (ui.ctx().content_rect().width() - 48.0).clamp(360.0, 1600.0);
        crate::windows::Dialog::new(tr("Add Parameter")).open(&mut open).width(width).show(
            ui.ctx(),
            |ui| {
                ui.set_max_width(width);
                // the parameter: a filter box over the list, taken ones greyed
                ui.horizontal(|ui| {
                    ui.label(RichText::new(egui_phosphor::regular::MAGNIFYING_GLASS).size(13.0));
                    ui.add(
                        egui::TextEdit::singleline(&mut add.filter)
                            .hint_text(tr("Filter parameters"))
                            .desired_width(f32::INFINITY),
                    );
                });
                let needle = add.filter.to_lowercase();
                let list_h = (ui.ctx().content_rect().height() * 0.35).clamp(120.0, 400.0);
                egui::ScrollArea::vertical().id_salt("expr-add-list").max_height(list_h).show(
                    ui,
                    |ui| {
                        ui.with_layout(Layout::top_down_justified(Align::Min), |ui| {
                            for (pi, p) in model.parameters.iter().enumerate() {
                                let name = names.get(pi).cloned().unwrap_or_else(|| p.id.clone());
                                if !needle.is_empty()
                                    && !name.to_lowercase().contains(&needle)
                                    && !p.id.to_lowercase().contains(&needle)
                                {
                                    continue;
                                }
                                let taken = have.iter().any(|ep| ep.id == p.id);
                                let label = if taken {
                                    RichText::new(name).color(pal.text_faint)
                                } else {
                                    RichText::new(name)
                                };
                                let r = ui.add_enabled(
                                    !taken,
                                    egui::Button::selectable(add.pick == pi, label)
                                        .wrap_mode(egui::TextWrapMode::Truncate),
                                );
                                if taken {
                                    r.on_disabled_hover_text(tr("Already in this expression"));
                                } else if r.clicked() && add.pick != pi {
                                    add.pick = pi;
                                    add.value = 0.0;
                                }
                            }
                        });
                    },
                );
                ui.separator();
                let (lo, hi, current, id) =
                    model.parameters.get(add.pick).map_or((-1.0, 1.0, 0.0, String::new()), |p| {
                        (p.min, p.max, s.puppet.value(add.pick), p.id.clone())
                    });
                crate::windows::form(ui, "expr-add-form", |ui| {
                    ui.label(tr("Blend"));
                    ui.horizontal(|ui| {
                        for (label, b) in BLENDS {
                            if ui.selectable_label(add.blend == b, tr(label)).clicked()
                                && add.blend != b
                            {
                                add.blend = b;
                                add.value = match b {
                                    ExpressionBlend::Add => 0.0,
                                    ExpressionBlend::Multiply => 1.0,
                                    ExpressionBlend::Overwrite => current,
                                };
                            }
                        }
                    });
                    ui.end_row();
                    ui.label(tr("Value"));
                    let range = value_range(add.blend, lo, hi);
                    let span = range.end() - range.start();
                    ui.horizontal(|ui| {
                        // the value box keeps one width, so a minus sign
                        // never shifts the slider
                        ui.spacing_mut().slider_width =
                            (ui.available_width() - VALUE_W - 24.0).max(120.0);
                        ui.add(
                            egui::Slider::new(&mut add.value, range.clone())
                                .show_value(false)
                                .clamping(egui::SliderClamping::Always),
                        );
                        ui.add_sized(
                            vec2(VALUE_W, ui.spacing().interact_size.y),
                            egui::DragValue::new(&mut add.value)
                                .range(range)
                                .speed(span / 300.0)
                                .fixed_decimals(3),
                        );
                    });
                    ui.end_row();
                });
                ui.weak(match add.blend {
                    ExpressionBlend::Add => crate::i18n::trf(
                        "Added to the pose's value; the pose has {id} at {v}.",
                        &[("id", &id), ("v", &format!("{current:.3}"))],
                    ),
                    ExpressionBlend::Multiply => crate::i18n::trf(
                        "The pose's value times this; the pose has {id} at {v}.",
                        &[("id", &id), ("v", &format!("{current:.3}"))],
                    ),
                    ExpressionBlend::Overwrite => crate::i18n::trf(
                        "Replaces the pose's value; the pose has {id} at {v}.",
                        &[("id", &id), ("v", &format!("{current:.3}"))],
                    ),
                });
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    let free = model
                        .parameters
                        .get(add.pick)
                        .is_some_and(|p| !have.iter().any(|ep| ep.id == p.id));
                    if ui
                        .add_enabled(free, egui::Button::new(RichText::new(tr("Add")).strong()))
                        .clicked()
                        && let Some(p) = model.parameters.get(add.pick)
                    {
                        done = Some(Some(ExpressionParam {
                            id: p.id.clone(),
                            value: add.value,
                            blend: add.blend,
                        }));
                    }
                    if ui.button(tr("Cancel")).clicked() {
                        done = Some(None);
                    }
                });
            },
        );
        if !open {
            done = Some(None);
        }
        match done {
            Some(result) => {
                self.add = None;
                result
            }
            None => None,
        }
    }
}

fn tr_count(n: usize) -> String {
    crate::i18n::trf("{n} parameters", &[("n", &n)])
}

/// The expression laid over `base` for one parameter.
pub fn blend(base: f32, ep: &ExpressionParam) -> f32 {
    match ep.blend {
        ExpressionBlend::Add => base + ep.value,
        ExpressionBlend::Multiply => base * ep.value,
        ExpressionBlend::Overwrite => ep.value,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blends_match_exp3() {
        let add = ExpressionParam { id: "a".into(), value: 0.5, blend: ExpressionBlend::Add };
        let mul = ExpressionParam { id: "a".into(), value: 0.5, blend: ExpressionBlend::Multiply };
        let set = ExpressionParam { id: "a".into(), value: 0.5, blend: ExpressionBlend::Overwrite };
        assert_eq!(blend(1.0, &add), 1.5);
        assert_eq!(blend(1.0, &mul), 0.5);
        assert_eq!(blend(1.0, &set), 0.5);
    }
}
