// The Parameter palette's right-click menus, following the Cubism manual:
// the menu on a parameter name, the popup on a slider (keys, then Select /
// Reflect / Adjust / Change), and the menu on a folder (parameter group).
// Rows are drawn while the session is borrowed, so the menus hand back a
// command and the palette runs it afterwards.

use ayatsuri_core::edit::Target;
use ayatsuri_core::keyform::{self, CornerRef};
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
    /// Make a folder holding this parameter and everything picked with it.
    GroupSelected(usize),
    /// Move this parameter and everything picked with it into a folder.
    MoveSelectedTo(usize, usize),
    /// Join a parameter with the one below it into a 2D pad.
    Combine(usize),
    Uncombine(usize),
    /// Automatic Generation of Four Corner Forms (Ctrl+4).
    FourCorners,
    /// Motion Mirroring for a parameter.
    Mirror(usize),
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

/// The Automatic Generation of Four Corner Forms dialog's settings.
struct Corners {
    pa: usize,
    pb: usize,
    centre: bool,
    /// Top left, top right, bottom left, bottom right.
    overwrite: [bool; 4],
    /// Every object keyed on both, not just the selection.
    all: bool,
}

/// The Motion Mirroring dialog's settings.
struct Mirror {
    param: usize,
    horizontal: bool,
    vertical: bool,
}

/// How deep a deformer sits, so mirroring can go parents first.
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
    /// Every parameter moving with it when the subject was multi-picked.
    pub(super) subject_set: Vec<usize>,
    rename: Option<(usize, String)>,
    corners: Option<Corners>,
    mirror: Option<Mirror>,
    /// Reflect waiting for the overwrite confirmation: (parameter, the
    /// key it mirrors from, the opposite key it overwrites).
    pub(super) reflect: Option<(usize, f32, Option<f32>)>,
    /// Delete asked for on a parameter still in use: (parameter, objects
    /// bound on it, physics settings naming it), waiting for an answer.
    pub(super) delete_ask: Option<(usize, usize, usize)>,
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

/// A menu item with its shortcut shown on the right.
fn item_sc(ui: &mut Ui, label: &str, shortcut: &str, cmd: ParamCmd, out: &mut Option<ParamCmd>) {
    if ui.add(egui::Button::new(label).shortcut_text(shortcut)).clicked() {
        *out = Some(cmd);
        ui.close();
    }
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
    if s.combined.iter().any(|&(a, b)| a == pi || b == pi) {
        item(ui, true, "Unlink parameters", ParamCmd::Uncombine(pi), &mut out);
    } else {
        item(ui, true, "Link with parameter below", ParamCmd::Combine(pi), &mut out);
    }
    // a right-click on a picked row acts on the whole pick
    let n = if s.param_picked.contains(&pi) { s.param_picked.len().max(1) } else { 1 };
    let what = if n > 1 { format!("{n} Parameters") } else { String::new() };
    let move_label =
        if n > 1 { format!("Select {what} for Movement") } else { "Select for Movement".into() };
    item(ui, true, &move_label, ParamCmd::MoveSubject(pi), &mut out);
    let can_move = subject.is_some_and(|m| m != Moving::Param(pi));
    item(ui, can_move, "Move Here", ParamCmd::MoveHere(pi), &mut out);
    let group_label = if n > 1 {
        format!("Group {what} into New Folder…")
    } else {
        "Group into New Folder…".into()
    };
    item(ui, true, &group_label, ParamCmd::GroupSelected(pi), &mut out);
    let folders: Vec<(usize, String)> = s
        .groups
        .iter()
        .enumerate()
        .filter(|(_, g)| !g.name.is_empty())
        .map(|(gi, g)| (gi, g.name.clone()))
        .collect();
    ui.add_enabled_ui(!folders.is_empty(), |ui| {
        ui.menu_button("Move to Folder", |ui| {
            for (gi, name) in &folders {
                item(ui, true, name, ParamCmd::MoveSelectedTo(pi, *gi), &mut out);
            }
        });
    });
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
    ui.separator();
    item(ui, can_edit, "Motion Mirroring…", ParamCmd::Mirror(pi), &mut out);
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
pub(super) fn panel_menu(
    ui: &mut Ui,
    s: &mut Session,
    sc: &dyn Fn(&str) -> String,
) -> Option<ParamCmd> {
    let mut out = None;
    item(ui, true, "Create Parameter…", ParamCmd::Create(None), &mut out);
    item(ui, true, "Create folder", ParamCmd::CreateFolder, &mut out);
    ui.separator();
    item_sc(ui, "Reset to default values", &sc("reset-params"), ParamCmd::ResetAll, &mut out);
    item_sc(
        ui,
        "Automatic Generation of Four Corner Forms…",
        &sc("four-corners"),
        ParamCmd::FourCorners,
        &mut out,
    );
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
    pub(super) fn param_targets(&self) -> Vec<Target> {
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

    /// Mirror the key at `from` horizontally into its opposite key on the
    /// selected objects, parents first, as one undo step.
    pub(super) fn reflect_into_opposite(&mut self, param: usize, from: f32) {
        let mut targets = self.param_targets();
        if let Some(s) = &self.session {
            targets.sort_by_key(|t| match t {
                Target::Deformer(d) => depth(s, *d),
                _ => usize::MAX,
            });
        }
        let Some(s) = &mut self.session else { return };
        let mut wrote = None;
        s.edit(|m| {
            for &t in &targets {
                if let Some(k) = keyform::mirror_motion(m, t, param, from, true, false) {
                    wrote = Some(k);
                }
            }
            wrote.is_some()
        });
        match wrote {
            Some(k) => log::info!("reflected the key at {from} into the key at {k}"),
            None => log::warn!("reflect: nothing on the selection has both keys"),
        }
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
            ParamCmd::CreateFolder => {
                self.folder_from.clear();
                self.new_folder = Some(String::new());
            }
            ParamCmd::GroupSelected(pi) => {
                self.folder_from = self.picked_with(pi);
                self.new_folder = Some(String::new());
            }
            ParamCmd::MoveSelectedTo(pi, gi) => {
                let set = self.picked_with(pi);
                let Some(s) = &mut self.session else { return };
                move_params_into(s, &set, gi);
            }
            ParamCmd::Delete(pi) => self.delete_parameter(pi),
            ParamCmd::Replicate(pi) => {
                self.replicate_parameter(pi, true);
            }
            ParamCmd::MoveSubject(pi) => {
                self.pmenu.subject_set = self.picked_with(pi);
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
                        let mut set = std::mem::take(&mut self.pmenu.subject_set);
                        if set.is_empty() {
                            set.push(src);
                        }
                        // dropping a pick onto one of its own rows is a no-op
                        if set.contains(&dest) {
                            return;
                        }
                        for g in &mut s.groups {
                            g.params.retain(|p| !set.contains(p));
                        }
                        if let Some(g) = s.groups.iter_mut().find(|g| g.params.contains(&dest)) {
                            let at = g.params.iter().position(|&p| p == dest).unwrap_or(0);
                            for (k, &p) in set.iter().enumerate() {
                                g.params.insert((at + k).min(g.params.len()), p);
                            }
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
                        let mut set = std::mem::take(&mut self.pmenu.subject_set);
                        if set.is_empty() {
                            set.push(src);
                        }
                        move_params_into(s, &set, gi);
                    }
                    Moving::Group(src) => move_group(s, src, gi),
                }
            }
            ParamCmd::ResetAll => self.reset_parameters(),
            ParamCmd::Mirror(pi) => {
                self.pmenu.mirror = Some(Mirror { param: pi, horizontal: true, vertical: false });
            }
            ParamCmd::FourCorners => {
                let Some(s) = &self.session else { return };
                // start from the conventional pair, else the first pad
                let find = |id: &str| s.puppet.model().parameters.iter().position(|p| p.id == id);
                let (pa, pb) = match (find("ParamAngleX"), find("ParamAngleY")) {
                    (Some(a), Some(b)) => (a, b),
                    _ => s.combined.first().copied().unwrap_or((0, 1)),
                };
                self.pmenu.corners = Some(Corners {
                    pa,
                    pb,
                    centre: false,
                    overwrite: [true; 4],
                    all: self.param_targets().is_empty(),
                });
            }
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
            ParamCmd::Combine(pi) => {
                let Some(s) = &mut self.session else { return };
                // the parameter directly below it in its folder
                let below = s.groups.iter().find_map(|g| {
                    let at = g.params.iter().position(|&p| p == pi)?;
                    g.params.get(at + 1).copied()
                });
                let taken =
                    |p: usize, c: &[(usize, usize)]| c.iter().any(|&(a, b)| a == p || b == p);
                match below {
                    Some(b) if !taken(pi, &s.combined) && !taken(b, &s.combined) => {
                        s.combined.push((pi, b));
                        s.modified = true;
                        log::info!("linked {} and {} into a pad", label(s, pi), label(s, b));
                    }
                    Some(_) => log::warn!("one of the two is already part of a pad"),
                    None => log::warn!("link needs a parameter below it in the same folder"),
                }
            }
            ParamCmd::Uncombine(pi) => {
                if let Some(s) = &mut self.session {
                    s.combined.retain(|&(a, b)| a != pi && b != pi);
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
                // Cubism's Reflect: the key under the slider is mirrored into
                // the opposite key, after asking before it overwrites that one
                let targets = self.param_targets();
                let Some(s) = &self.session else { return };
                let model = s.puppet.model();
                let Some(default) = model.parameters.get(pi).map(|p| p.default) else { return };
                let value = s.puppet.value(pi);
                let keys: Vec<f32> = targets
                    .iter()
                    .filter_map(|&t| keyform::keys_on(model, t, pi))
                    .flatten()
                    .collect();
                let from = keys
                    .iter()
                    .copied()
                    .min_by(|a, b| (a - value).abs().total_cmp(&(b - value).abs()))
                    .unwrap_or(value);
                let mirrored = 2.0 * default - from;
                let opposite = keys
                    .iter()
                    .copied()
                    .find(|k| (k - mirrored).abs() < 1e-3 && (from - default).abs() > 1e-4);
                self.pmenu.reflect = Some((pi, from, opposite));
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
            crate::windows::Dialog::new("Rename Folder").open(&mut open).show(&ctx, |ui| {
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

        let pick = |ui: &mut Ui, id: &str, v: &mut usize| {
            egui::ComboBox::from_id_salt(id)
                .selected_text(names.get(*v).cloned().unwrap_or_default())
                .show_ui(ui, |ui| {
                    for (i, n) in names.iter().enumerate() {
                        ui.selectable_value(v, i, n);
                    }
                });
        };

        if let Some(c) = &mut self.pmenu.corners {
            let mut open = true;
            let mut run = false;
            crate::windows::Dialog::new("Automatic Generation of Four Corner Forms").open(&mut open).show(&ctx, |ui| {
                    crate::windows::form(ui, "corners", |ui| {
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
                self.pmenu.corners = None;
                let targets = if all {
                    self.session
                        .as_ref()
                        .map(|s| {
                            let b = bound_on(s, pb);
                            bound_on(s, pa).into_iter().filter(|t| b.contains(t)).collect()
                        })
                        .unwrap_or_default()
                } else {
                    self.param_targets()
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
                self.pmenu.corners = None;
            }
        }

        if let Some((p, objects, physics)) = self.pmenu.delete_ask {
            let mut open = true;
            let mut go = false;
            let mut cancel = false;
            let name = names.get(p).cloned().unwrap_or_default();
            crate::windows::Dialog::new("Delete Parameter").open(&mut open).width(360.0).show(
                &ctx,
                |ui| {
                    let mut uses = Vec::new();
                    if objects > 0 {
                        uses.push(format!("{objects} object(s)"));
                    }
                    if physics > 0 {
                        uses.push(format!("{physics} physics setting(s)"));
                    }
                    ui.label(format!("{name} is still used by {}.", uses.join(" and ")));
                    ui.weak(
                        "Unbind it everywhere and delete it? Every object keeps the shape it \
                         has at the parameter's default, physics stops reading or driving it, \
                         and a physics group left with nothing to do is removed. Undo puts it \
                         all back.",
                    );
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        if ui.button("Unbind and delete").clicked() {
                            go = true;
                        }
                        if ui.button("Cancel").clicked() {
                            cancel = true;
                        }
                    });
                },
            );
            if go {
                self.pmenu.delete_ask = None;
                self.delete_parameter_unbinding(p);
            } else if cancel || !open {
                self.pmenu.delete_ask = None;
                log::warn!("{name} was not deleted: it is still in use");
            }
        }

        if let Some((p, from, opposite)) = self.pmenu.reflect {
            let mut open = true;
            let mut run = false;
            let name = names.get(p).cloned().unwrap_or_default();
            crate::windows::Dialog::new("Reflect").open(&mut open).width(340.0).show(&ctx, |ui| {
                match opposite {
                    Some(to) => {
                        ui.label(format!(
                            "The keyform at {to} on {name} will be overwritten with a mirror \
                             image of the keyform at {from}."
                        ));
                        ui.weak("This applies to the selected objects. Undo brings it back.");
                        ui.add_space(6.0);
                        ui.horizontal(|ui| {
                            if ui.button("Overwrite").clicked() {
                                run = true;
                            }
                            if ui.button("Cancel").clicked() {
                                self.pmenu.reflect = None;
                            }
                        });
                    }
                    None => {
                        ui.label(format!(
                            "Move {name} onto a key other than the default, on an object that \
                             also has the opposite key."
                        ));
                        if ui.button("Close").clicked() {
                            self.pmenu.reflect = None;
                        }
                    }
                }
            });
            if run {
                self.pmenu.reflect = None;
                self.reflect_into_opposite(p, from);
            } else if !open {
                self.pmenu.reflect = None;
            }
        }

        if let Some(mi) = &mut self.pmenu.mirror {
            let mut open = true;
            let mut run = false;
            crate::windows::Dialog::new("Motion Mirroring").open(&mut open).show(
                &ctx,
                |ui| {
                    crate::windows::form(ui, "mirror", |ui| {
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
                self.pmenu.mirror = None;
                let mut targets = self.param_targets();
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
                self.pmenu.mirror = None;
            }
        }

        if let Some((p, from, to)) = &mut self.pmenu.adjust {
            let mut open = true;
            let mut ok = false;
            crate::windows::Dialog::new("Adjust Value").open(&mut open).show(&ctx, |ui| {
                ui.label(names.get(*p).cloned().unwrap_or_default());
                crate::windows::form(ui, "p-adjust", |ui| {
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
            crate::windows::Dialog::new("Change Parameters").open(&mut open).show(&ctx, |ui| {
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

/// Take these parameters out of whatever folders hold them and append them,
/// in order, to folder `gi`, which opens to show them.
pub(super) fn move_params_into(s: &mut Session, set: &[usize], gi: usize) {
    if gi >= s.groups.len() || set.is_empty() {
        return;
    }
    for g in &mut s.groups {
        g.params.retain(|p| !set.contains(p));
    }
    if let Some(g) = s.groups.get_mut(gi) {
        g.params.extend_from_slice(set);
        g.open = true;
    }
    s.modified = true;
}
