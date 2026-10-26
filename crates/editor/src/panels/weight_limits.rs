// Limit Settings for Blend Shape Weights (Parameter palette menu), after
// the Cubism manual page of that name: pick a blend shape the selected
// objects have, add the parameters that limit it, and give each a graph,
// from a preset or by Edit Graph.

use ayatsuri_core::blendshape::{self, GraphError, MAX_POINTS, Preset};
use ayatsuri_core::edit::{self, LimitError, Target};
use egui::{Color32, Pos2, Rect, Sense, Stroke, Ui, pos2, vec2};

use super::EditorUi;
use super::param_menu::label;
use crate::i18n::{tr, trf};
use crate::session::Session;

/// The dialog's state between frames.
#[derive(Default)]
pub(super) struct LimitsDlg {
    /// The blend shape parameter being limited.
    shape: Option<usize>,
    /// The Parameter list's pick for Add.
    add: Option<usize>,
    /// The limiting parameter picked in the list.
    picked: Option<usize>,
    /// Edit Graph open: its rows as (parameter value, weight in percent).
    table: Option<Vec<(f32, f32)>>,
    /// The selected row in Edit Graph.
    row: Option<usize>,
    rules: bool,
    error: Option<String>,
}

/// The selected meshes and, per blend shape parameter they have, which of
/// them have it, in parameter order.
fn shapes_on(s: &Session, targets: &[Target]) -> Vec<(usize, Vec<usize>)> {
    let mut out: Vec<(usize, Vec<usize>)> = Vec::new();
    for t in targets {
        let Target::ArtMesh(mi) = *t else { continue };
        let Some(m) = s.puppet.model().art_meshes.get(mi) else { continue };
        for shape in &m.blend_shapes {
            let p = shape.parameter as usize;
            match out.iter_mut().find(|(q, _)| *q == p) {
                Some((_, meshes)) => meshes.push(mi),
                None => out.push((p, vec![mi])),
            }
        }
    }
    out.sort_by_key(|(p, _)| *p);
    out
}

/// The parameters limiting `shape` on any of `meshes`, first seen first,
/// with the graph of the first mesh that has each.
fn limits_of(s: &Session, meshes: &[usize], shape: usize) -> Vec<(usize, Vec<f32>, Vec<f32>)> {
    let mut out: Vec<(usize, Vec<f32>, Vec<f32>)> = Vec::new();
    for &mi in meshes {
        let Some(sh) = s
            .puppet
            .model()
            .art_meshes
            .get(mi)
            .and_then(|m| m.blend_shapes.iter().find(|b| b.parameter as usize == shape))
        else {
            continue;
        };
        for l in &sh.limits {
            if !out.iter().any(|(p, _, _)| *p == l.parameter as usize) {
                out.push((l.parameter as usize, l.keys.clone(), l.weights.clone()));
            }
        }
    }
    out
}

fn preset_label(p: Preset) -> &'static str {
    match p {
        Preset::Straight1 => tr("Line1"),
        Preset::Straight2 => tr("Line2"),
        Preset::Straight3 => tr("Line3"),
        Preset::Fold1 => tr("Polyline1"),
        Preset::Fold2 => tr("Polyline2"),
        Preset::Fold3 => tr("Polyline3"),
        Preset::Fold4 => tr("Polyline4"),
        Preset::Fold5 => tr("Polyline5"),
        Preset::Step1 => tr("Step1"),
        Preset::Step2 => tr("Step2"),
    }
}

fn graph_error(e: GraphError) -> &'static str {
    match e {
        GraphError::Points => tr("A graph needs 2 to 5 rows."),
        GraphError::Order => {
            tr("Parameter values must go up row by row, each at least 0.01 above the one before.")
        }
        GraphError::Range => tr("A parameter value is outside the parameter's range."),
        GraphError::Weight => tr("A weight limit is outside 0% to 100%."),
    }
}

/// Short numbers for the graph's labels: 0.0, 0.5, 1.0, -30.0.
fn num(v: f32) -> String {
    let s = format!("{v:.2}");
    let s = s.trim_end_matches('0');
    if s.ends_with('.') { format!("{s}0") } else { s.to_string() }
}

/// Draw a limit graph: grid, line, points labelled (value, weight%), the
/// parameter's range under it and a mark at its current value.
fn draw_graph(ui: &mut Ui, keys: &[f32], weights: &[f32], range: (f32, f32), now: f32, name: &str) {
    let pal = crate::theme::pal();
    let (resp, painter) = ui.allocate_painter(vec2(260.0, 220.0), Sense::hover());
    let r = resp.rect;
    let plot = Rect::from_min_max(r.min + vec2(28.0, 20.0), r.max - vec2(14.0, 34.0));
    let span = (range.1 - range.0).abs().max(1e-6);
    let at = |k: f32, w: f32| -> Pos2 {
        pos2(
            plot.left() + (k - range.0) / span * plot.width(),
            plot.bottom() - w.clamp(0.0, 1.0) * plot.height(),
        )
    };
    painter.rect_filled(plot, 0.0, pal.panel_bg);
    let faint = Stroke::new(1.0, pal.border);
    for i in 0..=5 {
        let f = i as f32 / 5.0;
        let x = plot.left() + f * plot.width();
        let y = plot.top() + f * plot.height();
        painter.line_segment([pos2(x, plot.top()), pos2(x, plot.bottom())], faint);
        painter.line_segment([pos2(plot.left(), y), pos2(plot.right(), y)], faint);
    }
    let font = egui::FontId::proportional(10.0);
    let text = |p: Pos2, a: egui::Align2, s: String, c: Color32| {
        painter.text(p, a, s, font.clone(), c);
    };
    text(
        plot.left_top() - vec2(4.0, 0.0),
        egui::Align2::RIGHT_CENTER,
        "100%".into(),
        pal.text_faint,
    );
    text(
        plot.left_bottom() - vec2(4.0, 0.0),
        egui::Align2::RIGHT_CENTER,
        "0%".into(),
        pal.text_faint,
    );
    text(plot.left_bottom() + vec2(0.0, 4.0), egui::Align2::CENTER_TOP, num(range.0), pal.text);
    text(plot.right_bottom() + vec2(0.0, 4.0), egui::Align2::CENTER_TOP, num(range.1), pal.text);
    text(
        pos2(plot.center().x, plot.bottom() + 18.0),
        egui::Align2::CENTER_TOP,
        name.to_string(),
        pal.text,
    );
    // where the parameter sits now, and the limit it gives there
    if now.is_finite() {
        let x = at(now.clamp(range.0, range.1), 0.0).x;
        painter.line_segment(
            [pos2(x, plot.top()), pos2(x, plot.bottom())],
            Stroke::new(1.0, crate::theme::accent()),
        );
    }
    let green = pal.warp_green;
    let pts: Vec<Pos2> = keys.iter().zip(weights).map(|(&k, &w)| at(k, w)).collect();
    if let (Some(first), Some(last)) = (pts.first(), pts.last()) {
        // held past the ends
        let stroke = Stroke::new(2.0, green);
        painter.line_segment([pos2(plot.left(), first.y), *first], stroke);
        painter.line_segment([*last, pos2(plot.right(), last.y)], stroke);
    }
    for w in pts.windows(2) {
        if let [a, b] = w {
            painter.line_segment([*a, *b], Stroke::new(2.0, green));
        }
    }
    for ((p, k), w) in pts.iter().zip(keys).zip(weights) {
        painter.circle_filled(*p, 4.0, green);
        let anchor = if p.x > plot.center().x {
            egui::Align2::RIGHT_BOTTOM
        } else {
            egui::Align2::LEFT_BOTTOM
        };
        text(*p - vec2(0.0, 5.0), anchor, format!("({}, {:.0}%)", num(*k), w * 100.0), green);
    }
}

impl EditorUi {
    pub(super) fn open_weight_limits(&mut self) {
        let targets = self.param_targets();
        let first = self.session.as_ref().and_then(|s| shapes_on(s, &targets).first().map(|x| x.0));
        self.pmenu.limits = Some(LimitsDlg { shape: first, ..LimitsDlg::default() });
    }

    pub(super) fn weight_limits_window(&mut self, ui: &mut Ui) {
        let Some(mut d) = self.pmenu.limits.take() else { return };
        let ctx = ui.ctx().clone();
        let targets = self.param_targets();
        let Some(s) = &mut self.session else { return };
        let shapes = shapes_on(s, &targets);
        if d.shape.is_none_or(|p| !shapes.iter().any(|(q, _)| *q == p)) {
            d.shape = shapes.first().map(|x| x.0);
            d.picked = None;
        }
        let meshes: Vec<usize> = shapes
            .iter()
            .find(|(p, _)| Some(*p) == d.shape)
            .map(|(_, m)| m.clone())
            .unwrap_or_default();
        let limits = d.shape.map(|sh| limits_of(s, &meshes, sh)).unwrap_or_default();
        if d.picked.is_some_and(|p| !limits.iter().any(|l| l.0 == p)) {
            d.picked = None;
        }
        if d.picked.is_none() {
            d.picked = limits.first().map(|l| l.0);
        }
        let n_params = s.puppet.model().parameters.len();
        let names: Vec<String> = (0..n_params).map(|p| label(s, p)).collect();
        let name = |p: usize| names.get(p).cloned().unwrap_or_default();

        // what the dialog asks for this frame, run after it is drawn
        enum Act {
            Add(usize),
            Delete(usize),
            Set(usize, Vec<f32>, Vec<f32>),
        }
        let mut act = None;
        let mut open = true;
        crate::windows::Dialog::new(tr("Limit Settings for Blend Shape Weights"))
            .open(&mut open)
            .width(520.0)
            .show(&ctx, |ui| {
                if shapes.is_empty() {
                    ui.weak(tr("Select the objects with blend shapes first."));
                    return;
                }
                ui.label(tr("Blend Shape"));
                ui.horizontal(|ui| {
                    let count = |p: usize| limits_of(s, &meshes_of(&shapes, p), p).len();
                    egui::ComboBox::from_id_salt("bsl-shape")
                        .width(200.0)
                        .selected_text(d.shape.map(name).unwrap_or_default())
                        .show_ui(ui, |ui| {
                            for (p, _) in &shapes {
                                let text = format!("{}    ({})", name(*p), count(*p));
                                if ui.selectable_label(d.shape == Some(*p), text).clicked() {
                                    d.shape = Some(*p);
                                    d.picked = None;
                                }
                            }
                        });
                    let objects: Vec<String> = meshes.iter().map(|&mi| s.mesh_name(mi)).collect();
                    ui.label(trf(
                        "Number of Target Objects to be Edited: {count}",
                        &[("count", &meshes.len())],
                    ))
                    .on_hover_text(objects.join(", "));
                });
                ui.add_space(4.0);
                ui.horizontal_top(|ui| {
                    ui.vertical(|ui| {
                        ui.set_width(200.0);
                        ui.label(tr("Parameter"));
                        ui.horizontal(|ui| {
                            egui::ComboBox::from_id_salt("bsl-add")
                                .width(130.0)
                                .selected_text(d.add.map(name).unwrap_or_default())
                                .show_ui(ui, |ui| {
                                    for p in 0..n_params {
                                        let taken =
                                            Some(p) == d.shape || limits.iter().any(|l| l.0 == p);
                                        if !taken {
                                            ui.selectable_value(&mut d.add, Some(p), name(p));
                                        }
                                    }
                                });
                            if ui
                                .add_enabled(d.add.is_some(), egui::Button::new(tr("Add")))
                                .clicked()
                                && let Some(p) = d.add.take()
                            {
                                act = Some(Act::Add(p));
                            }
                        });
                        egui::Frame::group(ui.style()).show(ui, |ui| {
                            ui.set_min_size(vec2(184.0, 180.0));
                            for (p, _, _) in &limits {
                                if ui.selectable_label(d.picked == Some(*p), name(*p)).clicked() {
                                    d.picked = Some(*p);
                                }
                            }
                        });
                        if ui
                            .add_enabled(d.picked.is_some(), egui::Button::new(tr("Delete")))
                            .clicked()
                            && let Some(p) = d.picked
                        {
                            act = Some(Act::Delete(p));
                        }
                    });
                    ui.vertical(|ui| {
                        ui.label(tr("Weight Limit Graph"));
                        let Some((p, keys, weights)) =
                            limits.iter().find(|l| Some(l.0) == d.picked).cloned()
                        else {
                            ui.weak(tr("Add a parameter to limit this blend shape by."));
                            return;
                        };
                        let meta = s.puppet.model().parameters.get(p).cloned();
                        let range = meta.as_ref().map_or((0.0, 1.0), |m| (m.min, m.max));
                        let now = Preset::of(&keys, &weights, range.0, range.1);
                        ui.horizontal(|ui| {
                            ui.label(tr("Preset"));
                            egui::ComboBox::from_id_salt("bsl-preset")
                                .width(140.0)
                                .selected_text(now.map_or(tr("Custom"), preset_label))
                                .show_ui(ui, |ui| {
                                    for pr in Preset::ALL {
                                        if ui
                                            .selectable_label(now == Some(pr), preset_label(pr))
                                            .clicked()
                                        {
                                            let (k, w) = pr.graph(range.0, range.1);
                                            act = Some(Act::Set(p, k, w));
                                        }
                                    }
                                });
                        });
                        draw_graph(ui, &keys, &weights, range, s.puppet.value(p), &name(p));
                        ui.horizontal(|ui| {
                            ui.weak(trf(
                                "Limit now: {value}%",
                                &[(
                                    "value",
                                    &format!(
                                        "{:.0}",
                                        blendshape::graph(&keys, &weights, s.puppet.value(p))
                                            * 100.0
                                    ),
                                )],
                            ));
                            if ui.button(tr("Edit Graph…")).clicked() {
                                d.table = Some(
                                    keys.iter()
                                        .zip(&weights)
                                        .map(|(k, w)| (*k, w * 100.0))
                                        .collect(),
                                );
                                d.row = None;
                                d.error = None;
                            }
                        });
                    });
                });
            });

        // Edit Graph: the table of the picked limit
        let mut table_ok = None;
        if let (Some(rows), Some(p)) = (&mut d.table, d.picked) {
            let mut keep = true;
            let mut ok = false;
            let range = s.puppet.model().parameters.get(p).map_or((0.0, 1.0), |m| (m.min, m.max));
            crate::windows::Dialog::new(tr("Edit Graph")).open(&mut keep).width(320.0).show(
                &ctx,
                |ui| {
                    ui.horizontal(|ui| {
                        if ui
                            .add_enabled(
                                rows.len() < MAX_POINTS,
                                egui::Button::new(tr("Add a line")),
                            )
                            .clicked()
                        {
                            let last = rows.last().map_or(range.0, |r| r.0);
                            rows.push(((last + (range.1 - range.0) * 0.1).min(range.1), 100.0));
                        }
                        if ui.button(tr("Delete line")).clicked() {
                            match d.row.take() {
                                Some(i) if i < rows.len() => {
                                    rows.remove(i);
                                }
                                _ => {
                                    rows.pop();
                                }
                            }
                        }
                        ui.toggle_value(&mut d.rules, tr("Show Rules"));
                    });
                    egui::Grid::new("bsl-table").striped(true).num_columns(3).show(ui, |ui| {
                        ui.label("");
                        ui.strong(tr("Parameter Value"));
                        ui.strong(tr("Weight limit (%)"));
                        ui.end_row();
                        for (i, (k, w)) in rows.iter_mut().enumerate() {
                            if ui.selectable_label(d.row == Some(i), format!("{}", i + 1)).clicked()
                            {
                                d.row = Some(i);
                            }
                            ui.add(egui::DragValue::new(k).speed(0.01).range(range.0..=range.1));
                            ui.add(egui::DragValue::new(w).speed(1.0).range(0.0..=100.0));
                            ui.end_row();
                        }
                    });
                    if d.rules {
                        ui.weak(tr(
                            "2 to 5 rows. Parameter values go up row by row, each at least \
                             0.01 above the one before, inside the parameter's range. Weight \
                             limits run 0% to 100%. Past the first and last rows the graph \
                             holds their weight.",
                        ));
                    }
                    if let Some(e) = &d.error {
                        ui.colored_label(crate::theme::pal().rotation_red, e);
                    }
                    if ui.button(tr("OK")).clicked() {
                        ok = true;
                    }
                },
            );
            if ok {
                let keys: Vec<f32> = rows.iter().map(|r| r.0).collect();
                let weights: Vec<f32> = rows.iter().map(|r| r.1 / 100.0).collect();
                match blendshape::check_graph(&keys, &weights, range.0, range.1) {
                    Ok(()) => table_ok = Some((p, keys, weights)),
                    Err(e) => d.error = Some(graph_error(e).to_string()),
                }
            }
            if !keep {
                d.table = None;
            }
        }
        if let Some((p, k, w)) = table_ok {
            d.table = None;
            act = Some(Act::Set(p, k, w));
        }

        if let (Some(a), Some(shape)) = (act, d.shape) {
            let mut failed = None;
            s.edit(|m| {
                let mut n = 0;
                for &mi in &meshes {
                    let r = match &a {
                        Act::Add(p) => {
                            let range = m.parameters.get(*p).map_or((0.0, 1.0), |q| (q.min, q.max));
                            let (k, w) = Preset::Straight3.graph(range.0, range.1);
                            edit::add_weight_limit(m, mi, shape, *p, k, w)
                        }
                        Act::Delete(p) => {
                            if edit::remove_weight_limit(m, mi, shape, *p) {
                                Ok(())
                            } else {
                                Err(LimitError::NoShape)
                            }
                        }
                        Act::Set(p, k, w) => {
                            edit::set_weight_limit(m, mi, shape, *p, k.clone(), w.clone())
                        }
                    };
                    match r {
                        Ok(()) => n += 1,
                        // a mesh lacking that limit, or already having it
                        Err(LimitError::NoShape | LimitError::Duplicate) => {}
                        Err(e) => failed = Some(e),
                    }
                }
                n > 0
            });
            if let Some(e) = failed {
                log::warn!("weight limit: {e:?}");
            }
            if let Act::Add(p) = a {
                d.picked = Some(p);
            }
        }
        if open {
            self.pmenu.limits = Some(d);
        }
    }
}

fn meshes_of(shapes: &[(usize, Vec<usize>)], p: usize) -> Vec<usize> {
    shapes.iter().find(|(q, _)| *q == p).map(|(_, m)| m.clone()).unwrap_or_default()
}
