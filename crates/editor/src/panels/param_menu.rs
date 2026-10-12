// The Parameter palette's right-click menus, following the Cubism manual:
// the menu on a parameter name, the popup on a slider (keys, then Select /
// Reflect / Adjust / Change), and the menu on a folder (parameter group).
// Rows are drawn while the session is borrowed, so the menus hand back a
// command and the palette runs it afterwards.

use ayatsuri_core::edit::Target;
use ayatsuri_core::keyform;
use egui::Ui;

use super::{EditorUi, ParamEdit, ParamNew, Sel};
use crate::session::Session;

#[derive(Clone, Copy, Debug)]
pub(super) enum ParamCmd {
    Edit(usize),
    /// Create a parameter, optionally inside a given folder.
    Create(Option<usize>),
    Replicate(usize),
    CreateFolder,
    Delete(usize),
    MoveSubject(usize),
    /// Move the subject in front of this parameter.
    MoveHere(usize),
    /// Move the subject to the end of this folder.
    MoveIntoGroup(usize),
    RenameGroup(usize),
    MoveGroupSubject(usize),
    ResetAll,
    DeleteEmptyParams,
    DeleteEmptyGroups,
    ReplicateGroup(usize),
    DeleteGroup(usize),
    AddSingle(usize, f32),
    DeleteKey(usize),
    Select(usize),
    Reflect(usize),
    Adjust(usize),
    Change(usize),
}

/// A parameter or a whole folder picked with Select for Movement.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Moving {
    Param(usize),
    Group(usize),
}

/// The palette's small bits of state between frames.
#[derive(Default)]
pub(super) struct ParamMenuState {
    /// "Select for Movement": what waits for "Move Here".
    pub(super) subject: Option<Moving>,
    rename: Option<(usize, String)>,
    adjust: Option<(usize, f32, f32)>,
    change: Option<(usize, usize)>,
}

const LABEL_COLORS: [[u8; 3]; 6] = [
    [0xE5, 0x48, 0x48],
    [0xF0, 0x8E, 0x1E],
    [0xE8, 0xC6, 0x3B],
    [0x5F, 0xB3, 0x5F],
    [0x4E, 0x8F, 0xD9],
    [0x9B, 0x6B, 0xD3],
];

/// The label colour picker shared by parameters and folders. Returns the
/// pick: Some(Some(rgb)) for a colour, Some(None) for clear.
fn color_menu(ui: &mut Ui) -> Option<Option<[u8; 3]>> {
    let mut out = None;
    ui.menu_button("Label color", |ui| {
        ui.horizontal(|ui| {
            for c in LABEL_COLORS {
                let (rect, resp) =
                    ui.allocate_exact_size(egui::vec2(16.0, 16.0), egui::Sense::click());
                ui.painter().rect_filled(rect, 3.0, egui::Color32::from_rgb(c[0], c[1], c[2]));
                if resp.clicked() {
                    out = Some(Some(c));
                    ui.close();
                }
            }
        });
        if ui.button("Clear").clicked() {
            out = Some(None);
            ui.close();
        }
    });
    out
}

fn item(ui: &mut Ui, enabled: bool, label: &str, cmd: ParamCmd, out: &mut Option<ParamCmd>) {
    if ui.add_enabled(enabled, egui::Button::new(label)).clicked() {
        *out = Some(cmd);
        ui.close();
    }
}

/// Right-click on a parameter name, in the manual's order.
pub(super) fn name_menu(
    ui: &mut Ui,
    s: &mut Session,
    pi: usize,
    subject: Option<Moving>,
) -> Option<ParamCmd> {
    let mut out = None;
    item(ui, true, "Edit Parameter…", ParamCmd::Edit(pi), &mut out);
    item(ui, true, "Create Parameter…", ParamCmd::Create(None), &mut out);
    item(ui, true, "Replicate parameters", ParamCmd::Replicate(pi), &mut out);
    item(ui, true, "Create folder", ParamCmd::CreateFolder, &mut out);
    item(ui, true, "Delete", ParamCmd::Delete(pi), &mut out);
    ui.separator();
    item(ui, true, "Select for Movement", ParamCmd::MoveSubject(pi), &mut out);
    let can_move = subject.is_some_and(|m| m != Moving::Param(pi));
    item(ui, can_move, "Move Here", ParamCmd::MoveHere(pi), &mut out);
    ui.separator();
    if ui.button("Reset to default").clicked() {
        let d = s.puppet.model().parameters.get(pi).map(|p| p.default).unwrap_or(0.0);
        s.puppet.set_value(pi, d);
        s.dirty = true;
        ui.close();
    }
    if let Some(pick) = color_menu(ui)
        && let Some(slot) = s.param_colors.get_mut(pi)
    {
        *slot = pick;
        s.modified = true;
    }
    out
}

/// The popup on a slider: keys first, then Select / Reflect / Adjust /
/// Change, which work on the selected objects.
pub(super) fn slider_menu(ui: &mut Ui, pi: usize, v: f32, can_edit: bool) -> Option<ParamCmd> {
    let mut out = None;
    item(ui, can_edit, "Add Single Keyform", ParamCmd::AddSingle(pi, v), &mut out);
    item(ui, can_edit, "Delete Keyform", ParamCmd::DeleteKey(pi), &mut out);
    ui.separator();
    item(ui, true, "Select", ParamCmd::Select(pi), &mut out);
    item(ui, can_edit, "Reflect", ParamCmd::Reflect(pi), &mut out);
    item(ui, can_edit, "Adjust…", ParamCmd::Adjust(pi), &mut out);
    item(ui, can_edit, "Change…", ParamCmd::Change(pi), &mut out);
    out
}

/// Right-click on a folder header.
pub(super) fn group_menu(
    ui: &mut Ui,
    s: &mut Session,
    gi: usize,
    subject: Option<Moving>,
) -> Option<ParamCmd> {
    let mut out = None;
    // Cubism's folder menu first, then our extras
    if let Some(pick) = color_menu(ui)
        && let Some(g) = s.groups.get_mut(gi)
    {
        g.color = pick;
        s.modified = true;
    }
    item(ui, true, "Create Parameter…", ParamCmd::Create(Some(gi)), &mut out);
    item(ui, true, "Replicate parameters", ParamCmd::ReplicateGroup(gi), &mut out);
    item(ui, true, "Create folder", ParamCmd::CreateFolder, &mut out);
    item(ui, true, "Delete", ParamCmd::DeleteGroup(gi), &mut out);
    ui.separator();
    item(ui, true, "Select for Movement", ParamCmd::MoveGroupSubject(gi), &mut out);
    let can_move = subject.is_some_and(|m| m != Moving::Group(gi));
    item(ui, can_move, "Move Here", ParamCmd::MoveIntoGroup(gi), &mut out);
    ui.separator();
    item(ui, true, "Rename…        (double-click)", ParamCmd::RenameGroup(gi), &mut out);
    if ui.button("Expand all").clicked() {
        s.groups.iter_mut().for_each(|g| g.open = true);
        ui.close();
    }
    if ui.button("Collapse all").clicked() {
        s.groups.iter_mut().for_each(|g| g.open = false);
        ui.close();
    }
    out
}

/// Right-click on the palette's empty space: creating things, plus the
/// whole-palette items from Cubism's palette menu.
pub(super) fn panel_menu(ui: &mut Ui, s: &mut Session) -> Option<ParamCmd> {
    let mut out = None;
    item(ui, true, "Create Parameter…", ParamCmd::Create(None), &mut out);
    item(ui, true, "Create folder", ParamCmd::CreateFolder, &mut out);
    ui.separator();
    item(ui, true, "Reset to default values        Ctrl+1", ParamCmd::ResetAll, &mut out);
    if ui.button("Expand all").clicked() {
        s.groups.iter_mut().for_each(|g| g.open = true);
        ui.close();
    }
    if ui.button("Collapse all").clicked() {
        s.groups.iter_mut().for_each(|g| g.open = false);
        ui.close();
    }
    ui.separator();
    item(ui, true, "Delete empty parameters", ParamCmd::DeleteEmptyParams, &mut out);
    item(ui, true, "Delete empty parameter groups", ParamCmd::DeleteEmptyGroups, &mut out);
    out
}

/// Move folder `src` so it sits just before folder `dest`.
fn move_group(s: &mut Session, src: usize, dest: usize) {
    if src == dest || src >= s.groups.len() || dest >= s.groups.len() {
        return;
    }
    let g = s.groups.remove(src);
    let at = if src < dest { dest - 1 } else { dest };
    s.groups.insert(at.min(s.groups.len()), g);
    s.modified = true;
}

/// The objects with keys on parameter `p`.
fn bound_on(s: &Session, p: usize) -> Vec<Target> {
    let m = s.puppet.model();
    let has =
        |b: &ayatsuri_format::model::Binding| b.axes.iter().any(|a| a.parameter as usize == p);
    let mut out: Vec<Target> = Vec::new();
    out.extend(
        m.art_meshes
            .iter()
            .enumerate()
            .filter(|(_, a)| has(&a.binding))
            .map(|(i, _)| Target::ArtMesh(i)),
    );
    out.extend(
        m.deformers
            .iter()
            .enumerate()
            .filter(|(_, d)| has(&d.binding))
            .map(|(i, _)| Target::Deformer(i)),
    );
    out.extend(
        m.parts.iter().enumerate().filter(|(_, q)| has(&q.binding)).map(|(i, _)| Target::Part(i)),
    );
    out
}

fn label(s: &Session, p: usize) -> String {
    s.param_names
        .get(p)
        .cloned()
        .filter(|n| !n.is_empty())
        .or_else(|| s.puppet.model().parameters.get(p).map(|q| q.id.clone()))
        .unwrap_or_default()
}

impl EditorUi {
    /// The selection as edit targets, lasso picks included.
    fn param_targets(&self) -> Vec<Target> {
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
        log::debug!("parameter palette: {cmd:?}");
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
            ParamCmd::Create(group) => {
                let last =
                    self.session.as_ref().map(|s| s.groups.len().saturating_sub(1)).unwrap_or(0);
                self.new_param = Some(ParamNew {
                    name: String::new(),
                    id: String::new(),
                    min: 0.0,
                    default: 0.0,
                    max: 1.0,
                    group: group.unwrap_or(last),
                });
            }
            ParamCmd::CreateFolder => self.new_folder = Some(String::new()),
            ParamCmd::Delete(pi) => self.delete_parameter(pi),
            ParamCmd::Replicate(pi) => {
                self.replicate_parameter(pi, true);
            }
            ParamCmd::MoveSubject(pi) => {
                self.pmenu.subject = Some(Moving::Param(pi));
                log::info!(
                    "parameter picked to move: right-click a parameter or folder and choose Move here"
                );
            }
            ParamCmd::MoveHere(dest) => {
                let Some(moving) = self.pmenu.subject.take() else { return };
                let Some(s) = &mut self.session else { return };
                match moving {
                    Moving::Param(src) => {
                        for g in &mut s.groups {
                            g.params.retain(|&p| p != src);
                        }
                        if let Some(g) = s.groups.iter_mut().find(|g| g.params.contains(&dest)) {
                            let at = g.params.iter().position(|&p| p == dest).unwrap_or(0);
                            g.params.insert(at, src);
                        }
                        s.modified = true;
                    }
                    Moving::Group(src) => {
                        // a folder dropped on a parameter lands before its folder
                        if let Some(to) = s.groups.iter().position(|g| g.params.contains(&dest)) {
                            move_group(s, src, to);
                        }
                    }
                }
            }
            ParamCmd::MoveIntoGroup(gi) => {
                let Some(moving) = self.pmenu.subject.take() else { return };
                let Some(s) = &mut self.session else { return };
                if gi >= s.groups.len() {
                    return;
                }
                match moving {
                    Moving::Param(src) => {
                        for g in &mut s.groups {
                            g.params.retain(|&p| p != src);
                        }
                        if let Some(g) = s.groups.get_mut(gi) {
                            g.params.push(src);
                            g.open = true;
                        }
                        s.modified = true;
                    }
                    Moving::Group(src) => move_group(s, src, gi),
                }
            }
            ParamCmd::ResetAll => self.reset_parameters(),
            ParamCmd::DeleteEmptyParams => {
                // no keys anywhere and not used by physics (delete_parameter
                // refuses those), highest index first so indices hold
                let Some(s) = &self.session else { return };
                let mut empty: Vec<usize> = (0..s.puppet.model().parameters.len())
                    .filter(|&p| bound_on(s, p).is_empty())
                    .collect();
                empty.reverse();
                let before = s.puppet.model().parameters.len();
                for p in empty {
                    self.delete_parameter(p);
                }
                let after =
                    self.session.as_ref().map_or(before, |s| s.puppet.model().parameters.len());
                log::info!("deleted {} parameter(s) with no keys", before - after);
            }
            ParamCmd::DeleteEmptyGroups => {
                if let Some(s) = &mut self.session {
                    let before = s.groups.len();
                    s.groups.retain(|g| !g.params.is_empty() || g.name.is_empty());
                    log::info!("deleted {} empty folder(s)", before - s.groups.len());
                    s.modified = true;
                }
            }
            ParamCmd::MoveGroupSubject(gi) => {
                self.pmenu.subject = Some(Moving::Group(gi));
                log::info!("folder picked to move: right-click where it goes and choose Move Here");
            }
            ParamCmd::ReplicateGroup(gi) => {
                let (name, params) = match self.session.as_ref().and_then(|s| s.groups.get(gi)) {
                    Some(g) => (g.name.clone(), g.params.clone()),
                    None => return,
                };
                let copies: Vec<usize> =
                    params.iter().filter_map(|&p| self.replicate_parameter(p, false)).collect();
                if let Some(s) = &mut self.session {
                    let color = s.groups.get(gi).and_then(|g| g.color);
                    let at = (gi + 1).min(s.groups.len());
                    s.groups.insert(
                        at,
                        crate::session::ParamGroup {
                            name: format!("{name} copy"),
                            params: copies,
                            open: true,
                            color,
                        },
                    );
                    s.modified = true;
                }
            }
            ParamCmd::RenameGroup(gi) => {
                let name = self
                    .session
                    .as_ref()
                    .and_then(|s| s.groups.get(gi).map(|g| g.name.clone()))
                    .unwrap_or_default();
                self.pmenu.rename = Some((gi, name));
            }
            ParamCmd::DeleteGroup(gi) => self.delete_group(gi),
            ParamCmd::AddSingle(pi, v) => {
                self.sel_param = Some(pi);
                self.add_keys_to_selection(vec![v]);
            }
            ParamCmd::DeleteKey(pi) => {
                self.sel_param = Some(pi);
                self.delete_key_on_selection();
            }
            ParamCmd::Select(pi) => {
                let Some(s) = &self.session else { return };
                let objs = bound_on(s, pi);
                log::info!("{} object(s) have keys on {}", objs.len(), label(s, pi));
                let meshes: Vec<usize> = objs
                    .iter()
                    .filter_map(|t| if let Target::ArtMesh(i) = t { Some(*i) } else { None })
                    .collect();
                self.multi_sel = meshes.clone();
                self.selected = match (meshes.last(), objs.first()) {
                    (Some(&m), _) => Some(Sel::Mesh(m)),
                    (None, Some(Target::Deformer(d))) => Some(Sel::Deformer(*d)),
                    (None, Some(Target::Part(p))) => Some(Sel::Part(*p)),
                    _ => self.selected,
                };
            }
            ParamCmd::Reflect(pi) => {
                let targets = self.param_targets();
                let Some(s) = &mut self.session else { return };
                let mut n = 0;
                s.edit(|m| {
                    for &t in &targets {
                        n += keyform::reflect_keys(m, t, pi) as usize;
                    }
                    n > 0
                });
                if n == 0 {
                    log::warn!("reflect: the selection has no keys on this parameter");
                } else {
                    log::info!("reflected the keys on {n} object(s)");
                }
            }
            ParamCmd::Adjust(pi) => {
                let v = self.session.as_ref().map_or(0.0, |s| s.puppet.value(pi));
                self.pmenu.adjust = Some((pi, v, v));
            }
            ParamCmd::Change(pi) => self.pmenu.change = Some((pi, pi)),
        }
    }

    /// Settings only, no keys, as Cubism's Replicate parameters. `place`
    /// puts the copy right after the original in its folder.
    fn replicate_parameter(&mut self, pi: usize, place: bool) -> Option<usize> {
        let s = self.session.as_mut()?;
        let meta = s.puppet.model().parameters.get(pi).cloned()?;
        let mut n = 2;
        let mut id = format!("{}{n}", meta.id);
        while s.puppet.model().parameters.iter().any(|p| p.id == id) {
            n += 1;
            id = format!("{}{n}", meta.id);
        }
        let ok = s.edit(|m| {
            ayatsuri_core::edit::create_parameter(m, id.clone(), meta.min, meta.default, meta.max)
                .is_some()
        });
        if !ok {
            return None;
        }
        let new = s.puppet.model().parameters.len().checked_sub(1)?;
        let name = format!("{} {n}", label(s, pi));
        s.param_names.push(name);
        let color = s.param_colors.get(pi).copied().flatten();
        s.param_colors.push(color);
        if place && let Some(g) = s.groups.iter_mut().find(|g| g.params.contains(&pi)) {
            let at = g.params.iter().position(|&p| p == pi).map_or(g.params.len(), |a| a + 1);
            g.params.insert(at, new);
        }
        self.sel_param = Some(new);
        log::info!("replicated parameter as {id} (settings only, no keys)");
        Some(new)
    }

    /// Deleting a folder deletes the parameters in it, as in Cubism. Ones
    /// still keyed or used by physics can't go, so they move out to the
    /// top level instead of vanishing with the folder.
    fn delete_group(&mut self, gi: usize) {
        let ids: Vec<String> = {
            let Some(s) = &self.session else { return };
            let Some(g) = s.groups.get(gi) else { return };
            g.params
                .iter()
                .filter_map(|&p| s.puppet.model().parameters.get(p).map(|q| q.id.clone()))
                .collect()
        };
        let name = self
            .session
            .as_ref()
            .and_then(|s| s.groups.get(gi).map(|g| g.name.clone()))
            .unwrap_or_default();
        let mut deleted = 0;
        for id in &ids {
            let idx = self
                .session
                .as_ref()
                .and_then(|s| s.puppet.model().parameters.iter().position(|p| &p.id == id));
            let Some(idx) = idx else { continue };
            let before = self.session.as_ref().map_or(0, |s| s.puppet.model().parameters.len());
            self.delete_parameter(idx);
            let after = self.session.as_ref().map_or(0, |s| s.puppet.model().parameters.len());
            deleted += usize::from(after < before);
        }
        let Some(s) = &mut self.session else { return };
        let Some(pos) = s.groups.iter().position(|g| g.name == name) else { return };
        let left = s.groups.remove(pos).params;
        if !left.is_empty() {
            match s.groups.iter_mut().find(|g| g.name.is_empty()) {
                Some(root) => root.params.extend(left.iter().copied()),
                None => s.groups.push(crate::session::ParamGroup {
                    name: String::new(),
                    params: left.clone(),
                    open: true,
                    color: None,
                }),
            }
            log::warn!(
                "folder {name} deleted; {} parameter(s) still in use were kept and moved to the top level",
                left.len()
            );
        } else {
            log::info!("folder {name} deleted with its {deleted} parameter(s)");
        }
        s.modified = true;
    }

    /// Rename, Adjust and Change, drawn each frame while open.
    pub(super) fn param_menu_windows(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        if let Some((gi, name)) = &mut self.pmenu.rename {
            let mut open = true;
            let mut ok = false;
            egui::Window::new("Rename Folder")
                .open(&mut open)
                .collapsible(false)
                .resizable(false)
                .show(&ctx, |ui| {
                    let r = ui.text_edit_singleline(name);
                    if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                        ok = true;
                    }
                    if ui.button("OK").clicked() {
                        ok = true;
                    }
                });
            if ok {
                let (gi, name) = (*gi, name.trim().to_string());
                self.pmenu.rename = None;
                if let Some(s) = &mut self.session
                    && let Some(g) = s.groups.get_mut(gi)
                    && !name.is_empty()
                {
                    g.name = name;
                    s.modified = true;
                }
            } else if !open {
                self.pmenu.rename = None;
            }
        }

        let names: Vec<String> = self
            .session
            .as_ref()
            .map(|s| (0..s.puppet.model().parameters.len()).map(|p| label(s, p)).collect())
            .unwrap_or_default();

        if let Some((p, from, to)) = &mut self.pmenu.adjust {
            let mut open = true;
            let mut ok = false;
            egui::Window::new("Adjust Value")
                .open(&mut open)
                .collapsible(false)
                .resizable(false)
                .show(&ctx, |ui| {
                    ui.label(names.get(*p).cloned().unwrap_or_default());
                    egui::Grid::new("p-adjust").num_columns(2).show(ui, |ui| {
                        ui.label("Key value");
                        ui.add(egui::DragValue::new(from).speed(0.1));
                        ui.end_row();
                        ui.label("New value");
                        ui.add(egui::DragValue::new(to).speed(0.1));
                        ui.end_row();
                    });
                    ui.weak("Moves the key (on the selected objects) keeping its form.");
                    if ui.button("OK").clicked() {
                        ok = true;
                    }
                });
            if ok {
                let (p, from, to) = (*p, *from, *to);
                self.pmenu.adjust = None;
                let targets = self.param_targets();
                if let Some(s) = &mut self.session {
                    let mut n = 0;
                    s.edit(|m| {
                        for &t in &targets {
                            n += keyform::adjust_key(m, t, p, from, to) as usize;
                        }
                        n > 0
                    });
                    if n == 0 {
                        log::warn!(
                            "adjust: no key moved (it would cross its neighbour, or the selection has no keys here)"
                        );
                    }
                }
            } else if !open {
                self.pmenu.adjust = None;
            }
        }

        if let Some((from, to)) = &mut self.pmenu.change {
            let mut open = true;
            let mut ok = false;
            egui::Window::new("Change Parameters")
                .open(&mut open)
                .collapsible(false)
                .resizable(false)
                .show(&ctx, |ui| {
                    ui.label(format!(
                        "Move the keys of {} to:",
                        names.get(*from).cloned().unwrap_or_default()
                    ));
                    egui::ComboBox::from_id_salt("p-change")
                        .selected_text(names.get(*to).cloned().unwrap_or_default())
                        .show_ui(ui, |ui| {
                            for (i, n) in names.iter().enumerate() {
                                ui.selectable_value(to, i, n);
                            }
                        });
                    if ui.button("OK").clicked() {
                        ok = true;
                    }
                });
            if ok {
                let (from, to) = (*from, *to);
                self.pmenu.change = None;
                let targets = self.param_targets();
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
                            "change: the selection has no keys here, or already has keys on that parameter"
                        );
                    }
                }
            } else if !open {
                self.pmenu.change = None;
            }
        }
    }
}
