// The New Warp / New Rotation Deformer dialog, laid out like Cubism's:
// destination part, name, where the deformer goes (as parent of the
// selection, as child of the selected deformer, or under a parent picked by
// hand), then Continuous creation / Create / Close.

use ayatsuri_core::edit::{Child, NewDeformer};
use ayatsuri_format::model::DeformerKind;
use egui::Ui;

use super::{EditorUi, Sel, parent_delta};

/// Where the new deformer goes.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Placement {
    ParentOfSelection,
    ChildOfSelection,
    Manual,
}

pub(super) struct DeformerDialog {
    warp: bool,
    name: String,
    part: Option<u32>,
    placement: Placement,
    /// Set Parent Manually's choice; None is the root.
    manual_parent: Option<u32>,
    cols: u32,
    rows: u32,
    /// Where the canvas was clicked, model space (rotation origin).
    click: Option<(f32, f32)>,
}

impl EditorUi {
    /// Open the dialog for the current selection, prefilled the way Cubism
    /// does: the selection's part, a fresh name, as parent of the selection.
    pub(super) fn open_deformer_dialog(&mut self, warp: bool, click: Option<(f32, f32)>) {
        let Some(s) = &self.session else { return };
        let model = s.puppet.model();
        let meshes = self.selection_meshes();
        let part = match self.selected {
            Some(Sel::Deformer(d)) => model.deformers.get(d).and_then(|x| x.parent_part),
            _ => meshes.first().and_then(|&m| model.art_meshes.get(m)).and_then(|m| m.parent_part),
        };
        let kind = if warp { "Warp" } else { "Rotation" };
        let mut n = model.deformers.len();
        let mut name = format!("{kind}{n}");
        while model.deformers.iter().any(|d| d.id == name) {
            n += 1;
            name = format!("{kind}{n}");
        }
        let has_selection = !meshes.is_empty() || matches!(self.selected, Some(Sel::Deformer(_)));
        self.deformer_dialog = Some(DeformerDialog {
            warp,
            name,
            part,
            placement: if has_selection { Placement::ParentOfSelection } else { Placement::Manual },
            manual_parent: None,
            cols: self.tool_opts.warp_cols,
            rows: self.tool_opts.warp_rows,
            click,
        });
    }

    pub(super) fn deformer_dialog_window(&mut self, ui: &mut Ui) {
        let deformer_selected = matches!(self.selected, Some(Sel::Deformer(_)));
        let has_selection = deformer_selected || !self.selection_meshes().is_empty();
        let Some(dlg) = &mut self.deformer_dialog else { return };
        let Some(s) = &self.session else {
            self.deformer_dialog = None;
            return;
        };
        let model = s.puppet.model();
        let part_label = |i: usize| {
            s.part_names
                .get(i)
                .cloned()
                .filter(|n| !n.is_empty())
                .or_else(|| model.parts.get(i).map(|p| p.id.clone()))
                .unwrap_or_default()
        };
        let title = if dlg.warp {
            crate::i18n::tr("New Warp Deformer")
        } else {
            crate::i18n::tr("New Rotation Deformer")
        };
        let mut open = true;
        let (mut create, mut keep_open, mut close) = (false, false, false);
        crate::windows::Dialog::new(title).open(&mut open).show(ui.ctx(), |ui| {
            ui.set_width(330.0);
            if !has_selection {
                // an empty deformer moves nothing until something is under it
                ui.label(
                    egui::RichText::new(crate::i18n::tr(
                        "Nothing is selected, so this deformer will hold nothing and moving it \
                         won't move the model. Select meshes (or click one) first.",
                    ))
                    .size(11.0)
                    .color(crate::theme::pal().accent),
                );
                ui.add_space(6.0);
            }
            crate::windows::form(ui, "newdef", |ui| {
                ui.label(crate::i18n::tr("Destination Part"));
                egui::ComboBox::from_id_salt("newdef-part")
                    .width(200.0)
                    .selected_text(
                        dlg.part
                            .map_or(crate::i18n::tr("(none)").into(), |p| part_label(p as usize)),
                    )
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut dlg.part, None, crate::i18n::tr("(none)"));
                        for i in 0..model.parts.len() {
                            ui.selectable_value(&mut dlg.part, Some(i as u32), part_label(i));
                        }
                    });
                ui.end_row();
                ui.label(crate::i18n::tr("Name"));
                ui.add(egui::TextEdit::singleline(&mut dlg.name).desired_width(200.0));
                ui.end_row();
                if dlg.warp {
                    ui.label(crate::i18n::tr("Conversion divisions"));
                    ui.horizontal(|ui| {
                        ui.add(egui::DragValue::new(&mut dlg.cols).range(1..=32).prefix("W "));
                        ui.add(egui::DragValue::new(&mut dlg.rows).range(1..=32).prefix("H "));
                    });
                    ui.end_row();
                }
            });
            ui.separator();
            ui.horizontal(|ui| {
                ui.label(crate::i18n::tr("Add"));
                ui.add_space(14.0);
                ui.vertical(|ui| {
                    ui.add_enabled_ui(has_selection, |ui| {
                        ui.radio_value(
                            &mut dlg.placement,
                            Placement::ParentOfSelection,
                            crate::i18n::tr("As Parent of Selected Object"),
                        );
                    });
                    ui.add_enabled_ui(deformer_selected, |ui| {
                        ui.radio_value(
                            &mut dlg.placement,
                            Placement::ChildOfSelection,
                            crate::i18n::tr("As Child of Selected Object"),
                        );
                    });
                    ui.radio_value(
                        &mut dlg.placement,
                        Placement::Manual,
                        crate::i18n::tr("Set Parent Manually"),
                    );
                    ui.add_enabled_ui(dlg.placement == Placement::Manual, |ui| {
                        let label = |p: Option<u32>| {
                            p.and_then(|i| model.deformers.get(i as usize))
                                .map_or(crate::i18n::tr("Root Deformer").to_string(), |d| {
                                    d.id.clone()
                                })
                        };
                        egui::ComboBox::from_id_salt("newdef-parent")
                            .width(180.0)
                            .selected_text(label(dlg.manual_parent))
                            .show_ui(ui, |ui| {
                                ui.selectable_value(
                                    &mut dlg.manual_parent,
                                    None,
                                    crate::i18n::tr("Root Deformer"),
                                );
                                for (i, d) in model.deformers.iter().enumerate() {
                                    ui.selectable_value(
                                        &mut dlg.manual_parent,
                                        Some(i as u32),
                                        &d.id,
                                    );
                                }
                            });
                    });
                });
            });
            ui.separator();
            ui.horizontal(|ui| {
                if ui
                    .button(crate::i18n::tr("Continuous creation"))
                    .on_hover_text(crate::i18n::tr(
                        "Create, and keep this dialog open for the next one",
                    ))
                    .clicked()
                {
                    create = true;
                    keep_open = true;
                }
                if ui.button(crate::i18n::tr("Create")).clicked() {
                    create = true;
                }
                if ui.button(crate::i18n::tr("Close")).clicked() {
                    close = true;
                }
            });
        });
        if create {
            self.create_from_dialog();
            if keep_open {
                // next name, same settings
                let warp = self.deformer_dialog.as_ref().is_some_and(|d| d.warp);
                if let Some(s) = &self.session
                    && let Some(d) = &mut self.deformer_dialog
                {
                    let kind = if warp { "Warp" } else { "Rotation" };
                    let mut n = s.puppet.model().deformers.len();
                    d.name = format!("{kind}{n}");
                    while s.puppet.model().deformers.iter().any(|x| x.id == d.name) {
                        n += 1;
                        d.name = format!("{kind}{n}");
                    }
                }
            } else {
                self.deformer_dialog = None;
            }
        } else if !open || close {
            self.deformer_dialog = None;
        }
    }

    fn create_from_dialog(&mut self) {
        let Some(dlg) = &self.deformer_dialog else { return };
        let (warp, name, part, placement, manual_parent, cols, rows, click) = (
            dlg.warp,
            dlg.name.trim().to_string(),
            dlg.part,
            dlg.placement,
            dlg.manual_parent,
            dlg.cols,
            dlg.rows,
            dlg.click,
        );
        if name.is_empty() {
            log::warn!("give the deformer a name");
            return;
        }
        self.tool_opts.warp_cols = cols;
        self.tool_opts.warp_rows = rows;
        let meshes = self.selection_meshes();
        let selected_def = match self.selected {
            Some(Sel::Deformer(d)) => Some(d),
            _ => None,
        };
        // several deformers picked: the new one wraps them all
        let picked_defs = super::def_group(self.selected, &self.def_pick);
        let selection_note = format!(
            "selected {:?}, deformer pick {:?}, mesh pick {:?}, part pick {:?}",
            self.selected, self.def_pick, self.multi_sel, self.part_pick
        );
        let Some(s) = &mut self.session else { return };
        let model = s.puppet.model();
        if model.deformers.iter().any(|d| d.id == name) {
            log::warn!("a deformer named {name} already exists");
            return;
        }
        // what the new deformer wraps
        let mut children: Vec<Child> = match placement {
            Placement::ParentOfSelection => {
                let mut c: Vec<Child> = meshes.iter().map(|&m| Child::Mesh(m)).collect();
                c.extend(picked_defs.iter().map(|&d| Child::Deformer(d)));
                c
            }
            Placement::ChildOfSelection => {
                let Some(d) = selected_def else {
                    log::warn!("select a deformer to add a child to");
                    return;
                };
                let d32 = Some(d as u32);
                let mut c: Vec<Child> = model
                    .art_meshes
                    .iter()
                    .enumerate()
                    .filter(|(_, m)| m.parent_deformer == d32)
                    .map(|(i, _)| Child::Mesh(i))
                    .collect();
                c.extend(
                    model
                        .deformers
                        .iter()
                        .enumerate()
                        .filter(|(_, x)| x.parent_deformer == d32)
                        .map(|(i, _)| Child::Deformer(i)),
                );
                c
            }
            Placement::Manual => Vec::new(),
        };
        // the parent the deformer lands under, for an empty one
        let empty_parent = match placement {
            Placement::ChildOfSelection => selected_def.map(|d| d as u32),
            _ => manual_parent,
        };
        // Set Parent Manually on the selection's own parent still wraps it
        if placement == Placement::Manual {
            let mut sel: Vec<Child> = meshes.iter().map(|&m| Child::Mesh(m)).collect();
            sel.extend(picked_defs.iter().map(|&d| Child::Deformer(d)));
            let same = !sel.is_empty()
                && sel.iter().all(|&c| {
                    let p = match c {
                        Child::Mesh(i) => model.art_meshes.get(i).map(|m| m.parent_deformer),
                        Child::Deformer(i) => model.deformers.get(i).map(|x| x.parent_deformer),
                    };
                    p == Some(manual_parent)
                });
            if same {
                children = sel;
            }
        }
        // an empty deformer gets a modest box in its parent's space
        let empty_extent = match empty_parent.and_then(|p| model.deformers.get(p as usize)) {
            Some(d) if matches!(d.kind, DeformerKind::Warp(_)) => (0.25, 0.25, 0.75, 0.75),
            Some(_) => (-0.1, -0.1, 0.1, 0.1),
            None => (-0.2, -0.2, 0.2, 0.2),
        };
        // a rotation's origin: the clicked point, carried into the parent's
        // space through its local jacobian around the first mesh
        let origin = if warp {
            None
        } else {
            click.and_then(|(mx, my)| {
                let first = children.iter().find_map(|c| match c {
                    Child::Mesh(m) => Some(*m),
                    _ => None,
                });
                match first {
                    None => (empty_parent.is_none() && children.is_empty()).then_some((mx, my)),
                    Some(mi) => {
                        let parent = model.art_meshes.get(mi).and_then(|m| m.parent_deformer);
                        if parent.is_none() {
                            return Some((mx, my));
                        }
                        let local = s.puppet.mesh_local_positions(mi)?;
                        let (lp, _) = local.as_chunks::<2>();
                        let n = lp.len().max(1) as f32;
                        let (clx, cly) =
                            lp.iter().fold((0.0, 0.0), |(x, y), q| (x + q[0] / n, y + q[1] / n));
                        let g = s.puppet.mesh_geometry(mi)?;
                        let (gp, _) = g.positions.as_chunks::<2>();
                        let n = gp.len().max(1) as f32;
                        let (cmx, cmy) =
                            gp.iter().fold((0.0, 0.0), |(x, y), q| (x + q[0] / n, y + q[1] / n));
                        let (dx, dy) =
                            parent_delta(&s.puppet, parent, clx, cly, mx - cmx, my - cmy);
                        Some((clx + dx, cly + dy))
                    }
                }
            })
        };
        // what is asked for, by name, to set against what it ends up holding
        let name_of = |model: &ayatsuri_format::Model, c: &Child| match *c {
            Child::Mesh(i) => {
                format!("mesh {}", model.art_meshes.get(i).map_or("?", |m| m.id.as_str()))
            }
            Child::Deformer(i) => {
                format!("deformer {}", model.deformers.get(i).map_or("?", |d| d.id.as_str()))
            }
        };
        let asked: Vec<String> = children.iter().map(|c| name_of(s.puppet.model(), c)).collect();
        log::info!(
            "New Deformer {name}: {selection_note}; {} wraps {} object(s): {}",
            match placement {
                Placement::ParentOfSelection => "as parent of the selection",
                Placement::ChildOfSelection => "as child of the selected deformer",
                Placement::Manual => "with its parent set by hand",
            },
            asked.len(),
            asked.join(", ")
        );
        let kind =
            if warp { NewDeformer::Warp { rows, cols } } else { NewDeformer::Rotation { origin } };
        let mut made = None;
        s.edit(|m| {
            made = ayatsuri_core::edit::create_deformer(
                m,
                kind,
                &children,
                empty_parent,
                empty_extent,
                part,
                name.clone(),
            );
            made.is_some()
        });
        match made {
            Some(di) => {
                // what it holds now, against what was asked
                let model = s.puppet.model();
                let d32 = Some(di as u32);
                let mut got: Vec<String> = model
                    .art_meshes
                    .iter()
                    .filter(|m| m.parent_deformer == d32)
                    .map(|m| format!("mesh {}", m.id))
                    .collect();
                got.extend(
                    model
                        .deformers
                        .iter()
                        .filter(|d| d.parent_deformer == d32)
                        .map(|d| format!("deformer {}", d.id)),
                );
                let missing: Vec<&String> = asked.iter().filter(|a| !got.contains(a)).collect();
                if missing.is_empty() {
                    log::info!("New Deformer {name}: holds {}", got.join(", "));
                } else {
                    log::warn!(
                        "New Deformer {name}: holds {} but not {}",
                        got.join(", "),
                        missing.iter().map(|m| m.as_str()).collect::<Vec<_>>().join(", ")
                    );
                }
                // indices at or past the insert point moved up one
                let shift = |set: &std::collections::HashSet<usize>| {
                    set.iter().map(|&i| if i >= di { i + 1 } else { i }).collect()
                };
                self.def_locked = shift(&self.def_locked);
                self.def_collapsed = shift(&self.def_collapsed);
                log::info!(
                    "created {name} ({}) holding {} object(s)",
                    if warp { "warp" } else { "rotation" },
                    children.len()
                );
                self.selected = Some(Sel::Deformer(di));
                self.multi_sel.clear();
                self.def_pick.clear();
                self.def_anchor = None;
            }
            None => log::warn!(
                "could not create {name}: the selected objects must share one parent deformer"
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::Session;
    use ayatsuri_format::Model;

    #[test]
    fn a_new_parent_wraps_every_picked_deformer() {
        let mut m = Model::new();
        let warp = |m: &mut Model, id: &str| {
            let kind = NewDeformer::Warp { rows: 1, cols: 1 };
            ayatsuri_core::edit::create_deformer(
                m,
                kind,
                &[],
                None,
                (0.0, 0.0, 1.0, 1.0),
                None,
                id.into(),
            )
            .unwrap()
        };
        for id in ["EyeL", "WhiteEyeL", "LowEyeL"] {
            warp(&mut m, id);
        }
        let ctx = egui::Context::default();
        let mut ed = EditorUi::new();
        ed.session = Some(Session::create(&ctx, "t".into(), String::new(), m).unwrap());
        ed.selected = Some(Sel::Deformer(0));
        ed.def_pick = vec![0, 1, 2];
        ed.open_deformer_dialog(true, None);
        if let Some(d) = &mut ed.deformer_dialog {
            d.name = "Eyes".into();
        }
        ed.create_from_dialog();
        let model = ed.session.as_ref().unwrap().puppet.model();
        let parent = model.deformers.iter().position(|d| d.id == "Eyes").unwrap() as u32;
        for id in ["EyeL", "WhiteEyeL", "LowEyeL"] {
            let d = model.deformers.iter().find(|d| d.id == id).unwrap();
            assert_eq!(d.parent_deformer, Some(parent), "{id} goes under the new warp");
        }
        assert!(ed.def_pick.is_empty());
    }
}
