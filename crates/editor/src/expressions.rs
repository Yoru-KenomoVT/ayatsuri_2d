//! The Expression palette: the model's exp3 expressions, made from the
//! pose or by hand. An expression is parameters held at a value and
//! blended over whatever the pose is (added, multiplied or overwritten),
//! faded in and out; a preview lays the picked one over the puppet.

use ayatsuri_format::{Expression, ExpressionBlend, ExpressionParam};
use egui::{Align, Layout, RichText, Ui, vec2};

use crate::i18n::tr;
use crate::session::Session;
use crate::theme;

const UNDO_STEPS: usize = 64;

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
    /// The parameter picked in the add row.
    add_pick: usize,
    /// The whole list as it was before each edit.
    undo: Vec<Vec<Expression>>,
    redo: Vec<Vec<Expression>>,
    /// A drag of a value folds into one step while it lasts.
    dragging: bool,
}

impl Expressions {
    fn keep(&mut self, s: &Session, merge: bool) {
        if merge && self.dragging {
            return;
        }
        self.dragging = merge;
        self.redo.clear();
        self.undo.push(s.expressions.clone());
        if self.undo.len() > UNDO_STEPS {
            self.undo.remove(0);
        }
    }

    pub fn undo(&mut self, s: &mut Session) -> bool {
        let Some(before) = self.undo.pop() else { return false };
        self.redo.push(std::mem::replace(&mut s.expressions, before));
        s.modified = true;
        self.dragging = false;
        true
    }

    pub fn redo(&mut self, s: &mut Session) -> bool {
        let Some(after) = self.redo.pop() else { return false };
        self.undo.push(std::mem::replace(&mut s.expressions, after));
        s.modified = true;
        self.dragging = false;
        true
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
        if ui.input(|i| i.pointer.any_released()) {
            self.dragging = false;
        }
        if self.sel.is_some_and(|i| i >= s.expressions.len()) {
            self.sel = None;
        }

        // ---- the list, with new / from pose / duplicate / delete ----
        ui.horizontal(|ui| {
            if ui
                .button(RichText::new(egui_phosphor::regular::PLUS).size(14.0))
                .on_hover_text(tr(
                    "New expression from the pose: every parameter moved off its default, added",
                ))
                .clicked()
            {
                self.keep(s, false);
                let params = Self::from_pose(s);
                let n = s.expressions.len() + 1;
                s.expressions.push(Expression {
                    name: format!("expression{n}"),
                    fade_in: None,
                    fade_out: None,
                    params,
                });
                self.sel = Some(s.expressions.len() - 1);
                s.modified = true;
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
                self.keep(s, false);
                let mut e = e;
                e.name = format!("{} copy", e.name);
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
                self.keep(s, false);
                s.expressions.remove(i);
                self.sel = if s.expressions.is_empty() {
                    None
                } else {
                    Some(i.min(s.expressions.len() - 1))
                };
                s.modified = true;
            }
            ui.add_space(6.0);
            if ui
                .add_enabled(
                    !self.undo.is_empty(),
                    egui::Button::new(
                        RichText::new(egui_phosphor::regular::ARROW_COUNTER_CLOCKWISE).size(14.0),
                    ),
                )
                .on_hover_text(tr("Undo"))
                .clicked()
            {
                self.undo(s);
            }
            if ui
                .add_enabled(
                    !self.redo.is_empty(),
                    egui::Button::new(
                        RichText::new(egui_phosphor::regular::ARROW_CLOCKWISE).size(14.0),
                    ),
                )
                .on_hover_text(tr("Redo"))
                .clicked()
            {
                self.redo(s);
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
                "No expressions yet. Pose the model, then press + to keep the pose as one.",
            ));
            return;
        }
        let list_h = (ui.available_height() * 0.3).clamp(60.0, 160.0);
        egui::ScrollArea::vertical().id_salt("expr-list").max_height(list_h).show(ui, |ui| {
            for (i, e) in s.expressions.iter().enumerate() {
                let label = format!("{}  {}", e.name, tr_count(e.params.len()));
                if ui.selectable_label(self.sel == Some(i), label).clicked() {
                    self.sel = Some(i);
                }
            }
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
            if ui.text_edit_singleline(&mut next.name).changed() {
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
                edited = Some(false);
            }
            // add one parameter, picked by name
            let w = (ui.available_width() - 40.0).max(80.0);
            egui::ComboBox::from_id_salt("expr-add")
                .selected_text(names.get(self.add_pick).cloned().unwrap_or_default())
                .width(w)
                .show_ui(ui, |ui| {
                    for (pi, n) in names.iter().enumerate() {
                        ui.selectable_value(&mut self.add_pick, pi, n);
                    }
                });
            if ui
                .button(RichText::new(egui_phosphor::regular::PLUS).size(13.0))
                .on_hover_text(tr("Add parameter"))
                .clicked()
                && let Some(p) = model.parameters.get(self.add_pick)
                && !next.params.iter().any(|ep| ep.id == p.id)
            {
                next.params.push(ExpressionParam {
                    id: p.id.clone(),
                    value: 0.0,
                    blend: ExpressionBlend::Add,
                });
                edited = Some(false);
            }
        });
        ui.add_space(4.0);
        let mut remove: Option<usize> = None;
        egui::ScrollArea::vertical().id_salt("expr-params").show(ui, |ui| {
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
                        let span = (hi - lo).abs().max(1e-3);
                        let range = match ep.blend {
                            ExpressionBlend::Add => -span..=span,
                            ExpressionBlend::Multiply => 0.0..=10.0,
                            ExpressionBlend::Overwrite => lo..=hi,
                        };
                        let r = ui.add(
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
            self.keep(s, merge);
            if let Some(slot) = s.expressions.get_mut(i) {
                *slot = next;
            }
            s.modified = true;
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
