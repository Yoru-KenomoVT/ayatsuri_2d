// The Parameter palette's menus and the dialogs behind them, following the
// Cubism manual: the palette menu (also Modeling > Parameter), the
// right-click menu on a parameter name, and the popup on a slider with
// Select / Reflect / Adjust / Change. Everything that edits the model goes
// through one Session::edit, so each command is one undo step.

use ayatsuri_core::edit::Target;
use ayatsuri_core::keyform::{self, CornerRef};
use egui::Ui;

use super::{EditorUi, ParamEdit, ParamNew, Sel};
use crate::session::Session;

/// Everything the menus can ask for. Rows are drawn while the session is
/// borrowed, so they hand these back and the palette runs them after.
#[derive(Clone, Copy, Debug)]
pub(super) enum ParamCmd {
    Edit(usize),
    Create,
    Replicate(usize),
    CreateFolder,
    Delete(usize),
    MoveSubject(usize),
    MoveHere(usize),
    Combine(usize),
    Uncombine(usize),
    AddSingle(usize, f32),
    DeleteKey(usize),
    SelectBound(usize),
    Reflect(usize),
    Adjust(usize),
    Change(usize),
    Mirror(usize),
    FourCorners,
    BatchReflect,
    ResetAll,
    SetAllDefault,
    DeleteEmptyParams,
    DeleteEmptyGroups,
    CopyForm,
    PasteForm,
}

/// Open dialogs and the palette's small bits of state.
#[derive(Default)]
pub(super) struct ParamDialogs {
    corners: Option<Corners>,
    mirror: Option<Mirror>,
    batch: Option<Vec<bool>>,
    adjust: Option<(usize, f32, f32)>,
    change: Option<(usize, usize)>,
    /// "Subject to move": the parameter waiting for "Move here".
    pub(super) subject: Option<usize>,
    form: Option<keyform::Form>,
}

struct Corners {
    pa: usize,
    pb: usize,
    centre: bool,
    /// Top left, top right, bottom left, bottom right.
    overwrite: [bool; 4],
    /// Every object bound on both, not just the selection.
    all: bool,
}

struct Mirror {
    param: usize,
    horizontal: bool,
    vertical: bool,
}

/// The objects bound on `p`, deformers first (parents before children) so
/// mirroring works down the hierarchy the way Cubism's does.
fn bound_on(s: &Session, p: usize) -> Vec<Target> {
    let m = s.puppet.model();
    let has =
        |b: &ayatsuri_format::model::Binding| b.axes.iter().any(|a| a.parameter as usize == p);
    let mut defs: Vec<(usize, usize)> = m
        .deformers
        .iter()
        .enumerate()
        .filter(|(_, d)| has(&d.binding))
        .map(|(i, _)| (i, depth(s, i)))
        .collect();
    defs.sort_by_key(|&(_, d)| d);
    let mut out: Vec<Target> = defs.into_iter().map(|(i, _)| Target::Deformer(i)).collect();
    out.extend(
        m.art_meshes
            .iter()
            .enumerate()
            .filter(|(_, a)| has(&a.binding))
            .map(|(i, _)| Target::ArtMesh(i)),
    );
    out.extend(
        m.parts.iter().enumerate().filter(|(_, q)| has(&q.binding)).map(|(i, _)| Target::Part(i)),
    );
    out
}

fn depth(s: &Session, d: usize) -> usize {
    let m = s.puppet.model();
    let mut cur = m.deformers.get(d).and_then(|x| x.parent_deformer);
    let mut n = 0;
    while let Some(p) = cur {
        n += 1;
        cur = m.deformers.get(p as usize).and_then(|x| x.parent_deformer);
        if n > 256 {
            break;
        }
    }
    n
}

fn param_label(s: &Session, p: usize) -> String {
    s.param_names
        .get(p)
        .cloned()
        .filter(|n| !n.is_empty())
        .or_else(|| s.puppet.model().parameters.get(p).map(|q| q.id.clone()))
        .unwrap_or_default()
}

/// The right-click menu on a parameter name.
pub(super) fn name_menu(
    ui: &mut Ui,
    s: &mut Session,
    pi: usize,
    subject: Option<usize>,
) -> Option<ParamCmd> {
    let mut cmd = None;
    let mut item = |ui: &mut Ui, enabled: bool, label: &str, c: ParamCmd| {
        if ui.add_enabled(enabled, egui::Button::new(label)).clicked() {
            cmd = Some(c);
            ui.close();
        }
    };
    item(ui, true, "Edit Parameter…", ParamCmd::Edit(pi));
    item(ui, true, "Create Parameter…", ParamCmd::Create);
    item(ui, true, "Replicate parameter", ParamCmd::Replicate(pi));
    item(ui, true, "Create folder", ParamCmd::CreateFolder);
    item(ui, true, "Delete", ParamCmd::Delete(pi));
    ui.separator();
    item(ui, true, "Subject to move", ParamCmd::MoveSubject(pi));
    item(ui, subject.is_some_and(|x| x != pi), "Move here", ParamCmd::MoveHere(pi));
    ui.separator();
    let combined = s.combined.iter().any(|&(a, b)| a == pi || b == pi);
    if combined {
        item(ui, true, "Release combination", ParamCmd::Uncombine(pi));
    } else {
        item(ui, true, "Combine with parameter below", ParamCmd::Combine(pi));
    }
    if ui.button("Reset to default").clicked() {
        let d = s.puppet.model().parameters.get(pi).map(|p| p.default).unwrap_or(0.0);
        s.puppet.set_value(pi, d);
        s.dirty = true;
        ui.close();
    }
    ui.menu_button("Label color", |ui| {
        ui.horizontal(|ui| {
            for c in [
                [0xE5u8, 0x48, 0x48],
                [0xF0, 0x8E, 0x1E],
                [0xE8, 0xC6, 0x3B],
                [0x5F, 0xB3, 0x5F],
                [0x4E, 0x8F, 0xD9],
                [0x9B, 0x6B, 0xD3],
            ] {
                let (rect, resp) =
                    ui.allocate_exact_size(egui::vec2(16.0, 16.0), egui::Sense::click());
                ui.painter().rect_filled(rect, 3.0, egui::Color32::from_rgb(c[0], c[1], c[2]));
                if resp.clicked() {
                    if let Some(slot) = s.param_colors.get_mut(pi) {
                        *slot = Some(c);
                        s.dirty = true;
                    }
                    ui.close();
                }
            }
        });
        if ui.button("Clear").clicked() {
            if let Some(slot) = s.param_colors.get_mut(pi) {
                *slot = None;
            }
            ui.close();
        }
    });
    cmd
}

/// The popup on a slider: keys, then Select / Reflect / Adjust / Change,
/// then the keyform generators that work from this parameter.
pub(super) fn slider_menu(ui: &mut Ui, pi: usize, v: f32, can_edit: bool) -> Option<ParamCmd> {
    let mut cmd = None;
    let mut item = |ui: &mut Ui, enabled: bool, label: &str, tip: &str, c: ParamCmd| {
        if ui.add_enabled(enabled, egui::Button::new(label)).on_hover_text(tip).clicked() {
            cmd = Some(c);
            ui.close();
        }
    };
    item(
        ui,
        can_edit,
        "Add Single Keyform",
        "A key at the current value",
        ParamCmd::AddSingle(pi, v),
    );
    item(
        ui,
        can_edit,
        "Delete Keyform",
        "The key at (or nearest) the current value",
        ParamCmd::DeleteKey(pi),
    );
    ui.separator();
    item(
        ui,
        true,
        "Select",
        "Select every object this parameter has keys on",
        ParamCmd::SelectBound(pi),
    );
    item(
        ui,
        can_edit,
        "Reflect",
        "Flip the key values: the form at the minimum moves to the maximum",
        ParamCmd::Reflect(pi),
    );
    item(
        ui,
        can_edit,
        "Adjust…",
        "Move a key to another value, keeping its form",
        ParamCmd::Adjust(pi),
    );
    item(ui, can_edit, "Change…", "Move the keys to a different parameter", ParamCmd::Change(pi));
    ui.separator();
    item(
        ui,
        can_edit,
        "Motion Mirroring…",
        "Mirror this key's form into the opposite key",
        ParamCmd::Mirror(pi),
    );
    item(
        ui,
        can_edit,
        "Copy Form",
        "Copy the selected object's form at this key",
        ParamCmd::CopyForm,
    );
    item(ui, can_edit, "Paste Form", "Paste the copied form into this key", ParamCmd::PasteForm);
    cmd
}

impl EditorUi {
    /// The palette menu, shared with Modeling > Parameter.
    pub(super) fn parameter_menu(&mut self, ui: &mut Ui) {
        let have = self.session.is_some();
        let mut cmd = None;
        let mut item = |ui: &mut Ui, enabled: bool, label: &str, c: ParamCmd| {
            if ui.add_enabled(enabled, egui::Button::new(label)).clicked() {
                cmd = Some(c);
                ui.close();
            }
        };
        item(ui, have, "Reset to default values        Ctrl+1", ParamCmd::ResetAll);
        item(ui, have, "Set all parameter values as default", ParamCmd::SetAllDefault);
        ui.separator();
        item(ui, have, "Automatic Generation of Four Corner Forms…", ParamCmd::FourCorners);
        item(
            ui,
            have && self.sel_param.is_some(),
            "Motion Mirroring…",
            ParamCmd::Mirror(self.sel_param.unwrap_or(0)),
        );
        item(ui, have, "Batch Reflect…", ParamCmd::BatchReflect);
        item(
            ui,
            have && self.sel_param.is_some(),
            "Select object associated with key",
            ParamCmd::SelectBound(self.sel_param.unwrap_or(0)),
        );
        ui.separator();
        item(ui, have, "Copy Form", ParamCmd::CopyForm);
        item(ui, have && self.pdlg.form.is_some(), "Paste Form", ParamCmd::PasteForm);
        ui.separator();
        item(ui, have, "Delete empty parameters", ParamCmd::DeleteEmptyParams);
        item(ui, have, "Delete empty parameter groups", ParamCmd::DeleteEmptyGroups);
        if let Some(c) = cmd {
            self.run_param_cmd(c);
        }
    }

    /// The objects a key command works on: the selection, meshes picked
    /// together included.
    fn selected_targets(&self) -> Vec<Target> {
        let mut out = Vec::new();
        match self.selected {
            Some(Sel::Mesh(i)) => out.push(Target::ArtMesh(i)),
            Some(Sel::Deformer(i)) => out.push(Target::Deformer(i)),
            Some(Sel::Part(i)) => out.push(Target::Part(i)),
            _ => {}
        }
        for &mi in &self.multi_sel {
            if !out.contains(&Target::ArtMesh(mi)) {
                out.push(Target::ArtMesh(mi));
            }
        }
        out
    }

    pub(super) fn run_param_cmd(&mut self, cmd: ParamCmd) {
        log::debug!("parameter command {cmd:?}");
        match cmd {
            ParamCmd::Edit(pi) => {
                if let Some(s) = &self.session
                    && let Some(meta) = s.puppet.model().parameters.get(pi)
                {
                    self.param_edit = Some(ParamEdit {
                        index: pi,
                        name: s.param_names.get(pi).cloned().unwrap_or_default(),
                        id: meta.id.clone(),
                        min: meta.min,
                        default: meta.default,
                        max: meta.max,
                        repeat: meta.repeat,
                    });
                }
            }
            ParamCmd::Create => {
                let group =
                    self.session.as_ref().map(|s| s.groups.len().saturating_sub(1)).unwrap_or(0);
                self.new_param = Some(ParamNew {
                    name: String::new(),
                    id: String::new(),
                    min: 0.0,
                    default: 0.0,
                    max: 1.0,
                    group,
                });
            }
            ParamCmd::CreateFolder => self.new_folder = Some(String::new()),
            ParamCmd::Delete(pi) => self.delete_parameter(pi),
            ParamCmd::Replicate(pi) => self.replicate_parameter(pi),
            ParamCmd::MoveSubject(pi) => {
                self.pdlg.subject = Some(pi);
                log::info!(
                    "parameter picked to move; right-click where it goes and choose Move here"
                );
            }
            ParamCmd::MoveHere(dest) => {
                let Some(src) = self.pdlg.subject.take() else { return };
                let Some(s) = &mut self.session else { return };
                for g in &mut s.groups {
                    g.params.retain(|&p| p != src);
                }
                if let Some(g) = s.groups.iter_mut().find(|g| g.params.contains(&dest)) {
                    let at = g.params.iter().position(|&p| p == dest).unwrap_or(0);
                    g.params.insert(at, src);
                }
                s.modified = true;
            }
            ParamCmd::Combine(pi) => {
                let Some(s) = &mut self.session else { return };
                let next = s.groups.iter().find_map(|g| {
                    let at = g.params.iter().position(|&p| p == pi)?;
                    g.params.get(at + 1).copied()
                });
                match next {
                    Some(b)
                        if !s
                            .combined
                            .iter()
                            .any(|&(x, y)| [x, y].contains(&pi) || [x, y].contains(&b)) =>
                    {
                        s.combined.push((pi, b));
                        s.modified = true;
                    }
                    _ => log::warn!(
                        "combine needs an uncombined parameter right below in the same folder"
                    ),
                }
            }
            ParamCmd::Uncombine(pi) => {
                if let Some(s) = &mut self.session {
                    s.combined.retain(|&(a, b)| a != pi && b != pi);
                    s.modified = true;
                }
            }
            ParamCmd::AddSingle(pi, v) => {
                self.sel_param = Some(pi);
                self.add_keys_to_selection(vec![v]);
            }
            ParamCmd::DeleteKey(pi) => {
                self.sel_param = Some(pi);
                self.delete_key_on_selection();
            }
            ParamCmd::SelectBound(pi) => {
                let Some(s) = &self.session else { return };
                let objs = bound_on(s, pi);
                let meshes: Vec<usize> = objs
                    .iter()
                    .filter_map(|t| if let Target::ArtMesh(i) = t { Some(*i) } else { None })
                    .collect();
                log::info!("{} objects have keys on {}", objs.len(), param_label(s, pi));
                self.multi_sel = meshes.clone();
                self.selected = match (meshes.last(), objs.first()) {
                    (Some(&m), _) => Some(Sel::Mesh(m)),
                    (None, Some(Target::Deformer(d))) => Some(Sel::Deformer(*d)),
                    (None, Some(Target::Part(p))) => Some(Sel::Part(*p)),
                    _ => self.selected,
                };
            }
            ParamCmd::Reflect(pi) => {
                let targets = self.selected_targets();
                let Some(s) = &mut self.session else { return };
                let mut n = 0;
                s.edit(|m| {
                    for &t in &targets {
                        n += keyform::reflect_keys(m, t, pi) as usize;
                    }
                    n > 0
                });
                log::info!("reflected keys on {n} object(s)");
            }
            ParamCmd::Adjust(pi) => {
                let Some(s) = &self.session else { return };
                let v = s.puppet.value(pi);
                self.pdlg.adjust = Some((pi, v, v));
            }
            ParamCmd::Change(pi) => self.pdlg.change = Some((pi, pi)),
            ParamCmd::Mirror(pi) => {
                self.pdlg.mirror = Some(Mirror { param: pi, horizontal: true, vertical: false });
            }
            ParamCmd::FourCorners => {
                let Some(s) = &self.session else { return };
                // start from the conventional pair, else the first combined
                let find = |id: &str| s.puppet.model().parameters.iter().position(|p| p.id == id);
                let (pa, pb) = match (find("ParamAngleX"), find("ParamAngleY")) {
                    (Some(a), Some(b)) => (a, b),
                    _ => s.combined.first().copied().unwrap_or((0, 1)),
                };
                self.pdlg.corners = Some(Corners {
                    pa,
                    pb,
                    centre: false,
                    overwrite: [true; 4],
                    all: self.selected.is_none(),
                });
            }
            ParamCmd::BatchReflect => {
                let n = self.session.as_ref().map_or(0, |s| s.puppet.model().parameters.len());
                self.pdlg.batch = Some(vec![false; n]);
            }
            ParamCmd::ResetAll => self.reset_parameters(),
            ParamCmd::SetAllDefault => {
                let Some(s) = &mut self.session else { return };
                let values: Vec<f32> =
                    (0..s.puppet.model().parameters.len()).map(|i| s.puppet.value(i)).collect();
                s.edit(|m| {
                    for (p, v) in m.parameters.iter_mut().zip(&values) {
                        p.default = v.clamp(p.min.min(p.max), p.max.max(p.min));
                    }
                    true
                });
                log::info!("current values are the new defaults");
            }
            ParamCmd::DeleteEmptyParams => {
                let Some(s) = &self.session else { return };
                let used = |p: usize| !bound_on(s, p).is_empty();
                let mut empty: Vec<usize> =
                    (0..s.puppet.model().parameters.len()).filter(|&p| !used(p)).collect();
                empty.reverse();
                let n = empty.len();
                for p in empty {
                    self.delete_parameter(p);
                }
                log::info!("deleted {n} parameter(s) with no keys");
            }
            ParamCmd::DeleteEmptyGroups => {
                if let Some(s) = &mut self.session {
                    let before = s.groups.len();
                    s.groups.retain(|g| !g.params.is_empty() || g.name.is_empty());
                    log::info!("deleted {} empty folder(s)", before - s.groups.len());
                    s.modified = true;
                }
            }
            ParamCmd::CopyForm | ParamCmd::PasteForm => {
                let Some((target, name)) = self.edit_target() else {
                    log::warn!("select an object first");
                    return;
                };
                let Some(s) = &mut self.session else { return };
                let values: Vec<f32> =
                    (0..s.puppet.model().parameters.len()).map(|i| s.puppet.value(i)).collect();
                let Some(k) = keyform::keyform_at(s.puppet.model(), target, &values) else {
                    log::warn!("the parameters are between keys; move them onto a key first");
                    return;
                };
                if matches!(cmd, ParamCmd::CopyForm) {
                    let mut model = s.puppet.model().clone();
                    self.pdlg.form = keyform::copy_form(&mut model, target, k);
                    log::info!("copied the form of {name}");
                } else if let Some(form) = self.pdlg.form.clone() {
                    if s.edit(|m| keyform::paste_form(m, target, k, &form)) {
                        log::info!("pasted the form into {name}");
                    } else {
                        log::warn!("the copied form does not fit {name} (different kind or size)");
                    }
                }
            }
        }
    }

    fn replicate_parameter(&mut self, pi: usize) {
        let Some(s) = &mut self.session else { return };
        let Some(meta) = s.puppet.model().parameters.get(pi).cloned() else { return };
        let mut id = format!("{}2", meta.id);
        let mut n = 2;
        while s.puppet.model().parameters.iter().any(|p| p.id == id) {
            n += 1;
            id = format!("{}{n}", meta.id);
        }
        let ok = s.edit(|m| {
            ayatsuri_core::edit::create_parameter(m, id.clone(), meta.min, meta.default, meta.max)
                .is_some()
        });
        if ok {
            let new = s.puppet.model().parameters.len() - 1;
            s.param_names.push(format!("{} copy", param_label(s, pi)));
            s.param_colors.push(s.param_colors.get(pi).copied().flatten());
            if let Some(g) = s.groups.iter_mut().find(|g| g.params.contains(&pi)) {
                let at = g.params.iter().position(|&p| p == pi).map_or(g.params.len(), |a| a + 1);
                g.params.insert(at, new);
            }
            self.sel_param = Some(new);
            log::info!("replicated parameter as {id} (settings only, no keys)");
        }
    }

    /// The dialogs, drawn every frame like the other windows.
    pub(super) fn param_dialogs(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        let names: Vec<String> = self
            .session
            .as_ref()
            .map(|s| (0..s.puppet.model().parameters.len()).map(|p| param_label(s, p)).collect())
            .unwrap_or_default();
        let pick = |ui: &mut Ui, id: &str, v: &mut usize| {
            egui::ComboBox::from_id_salt(id)
                .selected_text(names.get(*v).cloned().unwrap_or_default())
                .show_ui(ui, |ui| {
                    for (i, n) in names.iter().enumerate() {
                        ui.selectable_value(v, i, n);
                    }
                });
        };

        if let Some(c) = &mut self.pdlg.corners {
            let mut open = true;
            let mut run = false;
            egui::Window::new("Automatic Generation of Four Corner Forms")
                .open(&mut open)
                .collapsible(false)
                .resizable(false)
                .show(&ctx, |ui| {
                    egui::Grid::new("corners").num_columns(2).spacing([10.0, 6.0]).show(ui, |ui| {
                        ui.label("Parameter 1 (left/right)");
                        pick(ui, "c-pa", &mut c.pa);
                        ui.end_row();
                        ui.label("Parameter 2 (down/up)");
                        pick(ui, "c-pb", &mut c.pb);
                        ui.end_row();
                        ui.label("Parameter value to reference");
                        ui.horizontal(|ui| {
                            ui.radio_value(&mut c.centre, false, "Default value");
                            ui.radio_value(&mut c.centre, true, "Center of parameters");
                        });
                        ui.end_row();
                        ui.label("Overwrite key");
                        egui::Grid::new("corners-ow").show(ui, |ui| {
                            ui.checkbox(&mut c.overwrite[0], "Top left");
                            ui.checkbox(&mut c.overwrite[1], "Top right");
                            ui.end_row();
                            ui.checkbox(&mut c.overwrite[2], "Bottom left");
                            ui.checkbox(&mut c.overwrite[3], "Bottom right");
                            ui.end_row();
                        });
                        ui.end_row();
                        ui.label("Target");
                        ui.vertical(|ui| {
                            ui.radio_value(&mut c.all, false, "Selected objects");
                            ui.radio_value(&mut c.all, true, "All objects keyed on both");
                        });
                        ui.end_row();
                    });
                    ui.weak("Each corner becomes its left/right form plus its up/down form minus the centre. Make the four directions first.");
                    if ui.button("OK").clicked() {
                        run = true;
                    }
                });
            if run {
                let (pa, pb, centre, ow, all) = (c.pa, c.pb, c.centre, c.overwrite, c.all);
                self.pdlg.corners = None;
                let targets = if all {
                    self.session
                        .as_ref()
                        .map(|s| {
                            let b = bound_on(s, pb);
                            bound_on(s, pa).into_iter().filter(|t| b.contains(t)).collect()
                        })
                        .unwrap_or_default()
                } else {
                    self.selected_targets()
                };
                let reference = if centre { CornerRef::Centre } else { CornerRef::Default };
                if let Some(s) = &mut self.session {
                    let mut n = 0;
                    s.edit(|m| {
                        for &t in &targets {
                            n += keyform::four_corners(m, t, pa, pb, reference, ow) as usize;
                        }
                        n > 0
                    });
                    if n == 0 {
                        log::warn!(
                            "four corner forms: no target has keys on both parameters (two keys or more each)"
                        );
                    } else {
                        log::info!("four corner forms generated on {n} object(s)");
                    }
                }
            } else if !open {
                self.pdlg.corners = None;
            }
        }

        if let Some(mi) = &mut self.pdlg.mirror {
            let mut open = true;
            let mut run = false;
            egui::Window::new("Motion Mirroring").open(&mut open).collapsible(false).resizable(false).show(
                &ctx,
                |ui| {
                    egui::Grid::new("mirror").num_columns(2).spacing([10.0, 6.0]).show(ui, |ui| {
                        ui.label("Parameter");
                        pick(ui, "m-p", &mut mi.param);
                        ui.end_row();
                        ui.label("Flip settings");
                        ui.vertical(|ui| {
                            ui.checkbox(&mut mi.horizontal, "Flip horizontally");
                            ui.checkbox(&mut mi.vertical, "Flip vertically");
                        });
                        ui.end_row();
                    });
                    ui.weak("The key at the current value is mirrored across the default into the opposite key, on the selected objects, parents first. Warps mirror around their centre; meshes mirror their movement in place.");
                    if ui.button("OK").clicked() {
                        run = true;
                    }
                },
            );
            if run {
                let (p, h, v) = (mi.param, mi.horizontal, mi.vertical);
                self.pdlg.mirror = None;
                let mut targets = self.selected_targets();
                // parents first
                if let Some(s) = &self.session {
                    targets.sort_by_key(|t| match t {
                        Target::Deformer(d) => depth(s, *d),
                        _ => usize::MAX,
                    });
                }
                if let Some(s) = &mut self.session {
                    let from = s.puppet.value(p);
                    let mut wrote = None;
                    s.edit(|m| {
                        for &t in &targets {
                            if let Some(k) = keyform::mirror_motion(m, t, p, from, h, v) {
                                wrote = Some(k);
                            }
                        }
                        wrote.is_some()
                    });
                    match wrote {
                        Some(k) => log::info!("mirrored into the key at {k}"),
                        None => log::warn!(
                            "motion mirroring: move the parameter onto a key other than the default, with an opposite key on the selection"
                        ),
                    }
                }
            } else if !open {
                self.pdlg.mirror = None;
            }
        }

        if let Some(checks) = &mut self.pdlg.batch {
            let mut open = true;
            let mut run = false;
            egui::Window::new("Batch Reflect").open(&mut open).collapsible(false).show(&ctx, |ui| {
                let mut all = checks.iter().all(|c| *c);
                if ui.checkbox(&mut all, "All").changed() {
                    checks.iter_mut().for_each(|c| *c = all);
                }
                egui::ScrollArea::vertical().max_height(320.0).show(ui, |ui| {
                    for (i, c) in checks.iter_mut().enumerate() {
                        ui.checkbox(c, names.get(i).cloned().unwrap_or_default());
                    }
                });
                ui.weak("Flips the key values of the checked parameters on every object keyed on them.");
                if ui.button("OK").clicked() {
                    run = true;
                }
            });
            if run {
                let chosen: Vec<usize> =
                    checks.iter().enumerate().filter(|(_, c)| **c).map(|(i, _)| i).collect();
                self.pdlg.batch = None;
                if let Some(s) = &mut self.session {
                    let work: Vec<(usize, Vec<Target>)> =
                        chosen.iter().map(|&p| (p, bound_on(s, p))).collect();
                    let mut n = 0;
                    s.edit(|m| {
                        for (p, ts) in &work {
                            for &t in ts {
                                n += keyform::reflect_keys(m, t, *p) as usize;
                            }
                        }
                        n > 0
                    });
                    log::info!("batch reflect: {n} binding(s) flipped");
                }
            } else if !open {
                self.pdlg.batch = None;
            }
        }

        if let Some((p, from, to)) = &mut self.pdlg.adjust {
            let mut open = true;
            let mut run = false;
            egui::Window::new("Adjust Value")
                .open(&mut open)
                .collapsible(false)
                .resizable(false)
                .show(&ctx, |ui| {
                    ui.label(names.get(*p).cloned().unwrap_or_default());
                    egui::Grid::new("adjust").num_columns(2).show(ui, |ui| {
                        ui.label("Key");
                        ui.add(egui::DragValue::new(from).speed(0.1));
                        ui.end_row();
                        ui.label("New value");
                        ui.add(egui::DragValue::new(to).speed(0.1));
                        ui.end_row();
                    });
                    if ui.button("OK").clicked() {
                        run = true;
                    }
                });
            if run {
                let (p, from, to) = (*p, *from, *to);
                self.pdlg.adjust = None;
                let targets = self.selected_targets();
                if let Some(s) = &mut self.session {
                    let mut n = 0;
                    s.edit(|m| {
                        for &t in &targets {
                            n += keyform::adjust_key(m, t, p, from, to) as usize;
                        }
                        n > 0
                    });
                    if n == 0 {
                        log::warn!("adjust: the new value would cross a neighbouring key");
                    }
                }
            } else if !open {
                self.pdlg.adjust = None;
            }
        }

        if let Some((from, to)) = &mut self.pdlg.change {
            let mut open = true;
            let mut run = false;
            egui::Window::new("Change Parameters")
                .open(&mut open)
                .collapsible(false)
                .resizable(false)
                .show(&ctx, |ui| {
                    ui.label(format!(
                        "Move the keys of {} to",
                        names.get(*from).cloned().unwrap_or_default()
                    ));
                    pick(ui, "chg-to", to);
                    if ui.button("OK").clicked() {
                        run = true;
                    }
                });
            if run {
                let (from, to) = (*from, *to);
                self.pdlg.change = None;
                let targets = self.selected_targets();
                if let Some(s) = &mut self.session {
                    let mut n = 0;
                    s.edit(|m| {
                        for &t in &targets {
                            n += keyform::change_parameter(m, t, from, to) as usize;
                        }
                        n > 0
                    });
                    if n == 0 {
                        log::warn!(
                            "change: the selection is already keyed on that parameter, or has no keys on this one"
                        );
                    }
                }
            } else if !open {
                self.pdlg.change = None;
            }
        }
    }
}
