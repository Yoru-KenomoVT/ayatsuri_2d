// The Parameter palette's right-click menus, following the Cubism manual:
// the menu on a parameter name, the popup on a slider (keys, then Select /
// Reflect / Adjust / Change), and the menu on a folder (parameter group).
// Rows are drawn while the session is borrowed, so the menus hand back a
// command and the palette runs it afterwards.

use ayatsuri_core::edit::Target;

use crate::color_pick::Pick;
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
    /// A parameter or folder dragged onto a parameter: it lands before it.
    DropOnParam(Moving, usize),
    /// A parameter or folder dragged onto a folder's header: into the folder,
    /// or before it for a folder.
    DropOnGroup(Moving, usize),
    /// Dropped under the last row: a folder goes last, a parameter to the
    /// end of the last folder.
    DropAtEnd(Moving),
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
    /// Extended Interpolation for a parameter on the selected objects.
    Curve(usize),
    /// A colour for one key (None clears it).
    KeyColor(usize, f32, Option<[u8; 3]>),
    /// The colour window for a label colour.
    CustomColor(ColorTarget, Option<[u8; 3]>),
    ResetAll,
    DeleteEmptyParams,
    DeleteEmptyGroups,
    ReplicateGroup(usize),
    DeleteGroup(usize),
    AddSingle(usize, f32),
    DeleteKey(usize),
    Select(usize),
    Reflect(usize),
    BatchReflect,
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

/// The Extended Interpolation dialog's settings: method 0 linear, 1
/// ellipse, 2 SNS, as Cubism lists them.
struct CurveDlg {
    param: usize,
    method: usize,
    points: u8,
    scale: f32,
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

/// A parameter or a whole folder picked with Select for Movement, or
/// dragged by its name.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Moving {
    Param(usize),
    Group(usize),
}

/// Whose label colour the custom colour window sets.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum ColorTarget {
    Param(usize),
    Group(usize),
    Key(usize, f32),
}

/// Set (or clear) a label colour.
fn set_color(s: &mut Session, t: ColorTarget, c: Option<[u8; 3]>) {
    log::debug!("label colour: {t:?} set to {}", c.map_or("none".into(), crate::color_pick::hex));
    match t {
        ColorTarget::Param(pi) => {
            if let Some(slot) = s.param_colors.get_mut(pi) {
                *slot = c;
            }
        }
        ColorTarget::Group(gi) => {
            if let Some(g) = s.groups.get_mut(gi) {
                g.color = c;
            }
        }
        ColorTarget::Key(pi, k) => {
            s.key_colors.retain(|e| !(e.0 == pi && (e.1 - k).abs() < 1e-5));
            if let Some(c) = c {
                s.key_colors.push((pi, k, c));
            }
        }
    }
    s.modified = true;
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
    curve: Option<CurveDlg>,
    /// The colour window: whose colour, and the colour now.
    custom: Option<(ColorTarget, [u8; 3])>,
    /// The Batch Reflect window: a tick per parameter.
    pub(super) batch_reflect: Option<Vec<bool>>,
    /// Delete asked for on a parameter still in use: (parameter, objects
    /// bound on it, physics settings naming it), waiting for an answer.
    pub(super) delete_ask: Option<(usize, usize, usize)>,
    adjust: Option<(usize, f32, f32)>,
    change: Option<(usize, usize)>,
}

/// The label colour picker shared by parameters, folders and keys: a
/// submenu holding the full picker. Returns the pick: Some(Some(rgb)) for
/// a colour, Some(None) for clear.
fn color_menu(ui: &mut Ui, label: &str, current: Option<[u8; 3]>) -> Option<Pick> {
    use egui::containers::menu::{MenuConfig, SubMenuButton};
    let mut out = None;
    // a click inside must not close it, or the wheel tile could never open
    // its picker; the presets and Clear close it themselves
    SubMenuButton::new(label)
        .config(MenuConfig::new().close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside))
        .ui(ui, |ui| {
            out = crate::color_pick::picker(ui, current);
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
    item(ui, true, crate::i18n::tr("Edit Parameter…"), ParamCmd::Edit(pi), &mut out);
    item(ui, true, crate::i18n::tr("Create Parameter…"), ParamCmd::Create(None), &mut out);
    item(ui, true, crate::i18n::tr("Replicate parameters"), ParamCmd::Replicate(pi), &mut out);
    item(ui, true, crate::i18n::tr("Create folder"), ParamCmd::CreateFolder, &mut out);
    item(ui, true, crate::i18n::tr("Delete"), ParamCmd::Delete(pi), &mut out);
    ui.separator();
    if s.combined.iter().any(|&(a, b)| a == pi || b == pi) {
        item(ui, true, crate::i18n::tr("Unlink parameters"), ParamCmd::Uncombine(pi), &mut out);
    } else {
        item(
            ui,
            true,
            crate::i18n::tr("Link with parameter below"),
            ParamCmd::Combine(pi),
            &mut out,
        );
    }
    // a right-click on a picked row acts on the whole pick
    let n = if s.param_picked.contains(&pi) { s.param_picked.len().max(1) } else { 1 };
    let move_label = if n > 1 {
        crate::i18n::trf("Select {count} Parameters for Movement", &[("count", &n)])
    } else {
        crate::i18n::tr("Select for Movement").into()
    };
    item(ui, true, &move_label, ParamCmd::MoveSubject(pi), &mut out);
    let can_move = subject.is_some_and(|m| m != Moving::Param(pi));
    item(ui, can_move, crate::i18n::tr("Move Here"), ParamCmd::MoveHere(pi), &mut out);
    let group_label = if n > 1 {
        crate::i18n::trf("Group {count} Parameters into New Folder…", &[("count", &n)])
    } else {
        crate::i18n::tr("Group into New Folder…").into()
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
        ui.menu_button(crate::i18n::tr("Move to Folder"), |ui| {
            for (gi, name) in &folders {
                item(ui, true, name, ParamCmd::MoveSelectedTo(pi, *gi), &mut out);
            }
        });
    });
    ui.separator();
    if ui.button(crate::i18n::tr("Reset to default")).clicked() {
        let d = s.puppet.model().parameters.get(pi).map(|p| p.default).unwrap_or(0.0);
        s.puppet.set_value(pi, d);
        s.dirty = true;
        ui.close();
    }
    let now = s.param_colors.get(pi).copied().flatten();
    match color_menu(ui, crate::i18n::tr("Label color"), now) {
        Some(Pick::Set(c)) => set_color(s, ColorTarget::Param(pi), c),
        Some(Pick::Custom) => out = Some(ParamCmd::CustomColor(ColorTarget::Param(pi), now)),
        None => {}
    }
    out
}

/// The popup on a slider: keys first, then Select / Reflect / Adjust /
/// Change, which work on the selected objects.
pub(super) fn slider_menu(
    ui: &mut Ui,
    pi: usize,
    v: f32,
    key: Option<(f32, Option<[u8; 3]>)>,
    can_edit: bool,
) -> Option<ParamCmd> {
    let mut out = None;
    // on a key: its own colour first
    if let Some((k, now)) = key {
        match color_menu(ui, crate::i18n::tr("Key color"), now) {
            Some(Pick::Set(c)) => out = Some(ParamCmd::KeyColor(pi, k, c)),
            Some(Pick::Custom) => {
                out = Some(ParamCmd::CustomColor(ColorTarget::Key(pi, k), now));
            }
            None => {}
        }
    }
    item(ui, can_edit, crate::i18n::tr("Add Single Keyform"), ParamCmd::AddSingle(pi, v), &mut out);
    item(ui, can_edit, crate::i18n::tr("Delete Keyform"), ParamCmd::DeleteKey(pi), &mut out);
    ui.separator();
    item(ui, true, crate::i18n::tr("Select"), ParamCmd::Select(pi), &mut out);
    item(ui, can_edit, crate::i18n::tr("Reflect"), ParamCmd::Reflect(pi), &mut out);
    item(ui, can_edit, crate::i18n::tr("Adjust…"), ParamCmd::Adjust(pi), &mut out);
    item(ui, can_edit, crate::i18n::tr("Change…"), ParamCmd::Change(pi), &mut out);
    ui.separator();
    item(ui, can_edit, crate::i18n::tr("Motion Mirroring…"), ParamCmd::Mirror(pi), &mut out);
    item(ui, can_edit, crate::i18n::tr("Extended Interpolation…"), ParamCmd::Curve(pi), &mut out);
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
    let now = s.groups.get(gi).and_then(|g| g.color);
    match color_menu(ui, crate::i18n::tr("Label color"), now) {
        Some(Pick::Set(c)) => set_color(s, ColorTarget::Group(gi), c),
        Some(Pick::Custom) => out = Some(ParamCmd::CustomColor(ColorTarget::Group(gi), now)),
        None => {}
    }
    item(ui, true, crate::i18n::tr("Create Parameter…"), ParamCmd::Create(Some(gi)), &mut out);
    item(ui, true, crate::i18n::tr("Replicate parameters"), ParamCmd::ReplicateGroup(gi), &mut out);
    item(ui, true, crate::i18n::tr("Create folder"), ParamCmd::CreateFolder, &mut out);
    item(ui, true, crate::i18n::tr("Delete"), ParamCmd::DeleteGroup(gi), &mut out);
    ui.separator();
    item(
        ui,
        true,
        crate::i18n::tr("Select for Movement"),
        ParamCmd::MoveGroupSubject(gi),
        &mut out,
    );
    let can_move = subject.is_some_and(|m| m != Moving::Group(gi));
    item(ui, can_move, crate::i18n::tr("Move Here"), ParamCmd::MoveIntoGroup(gi), &mut out);
    ui.separator();
    item(
        ui,
        true,
        crate::i18n::tr("Rename…        (double-click)"),
        ParamCmd::RenameGroup(gi),
        &mut out,
    );
    if ui.button(crate::i18n::tr("Expand all")).clicked() {
        s.groups.iter_mut().for_each(|g| g.open = true);
        ui.close();
    }
    if ui.button(crate::i18n::tr("Collapse all")).clicked() {
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
    item(ui, true, crate::i18n::tr("Create Parameter…"), ParamCmd::Create(None), &mut out);
    item(ui, true, crate::i18n::tr("Create folder"), ParamCmd::CreateFolder, &mut out);
    ui.separator();
    item(ui, true, crate::i18n::tr("Batch Reflect…"), ParamCmd::BatchReflect, &mut out);
    item_sc(
        ui,
        crate::i18n::tr("Reset to default values"),
        &sc("reset-params"),
        ParamCmd::ResetAll,
        &mut out,
    );
    item_sc(
        ui,
        crate::i18n::tr("Automatic Generation of Four Corner Forms…"),
        &sc("four-corners"),
        ParamCmd::FourCorners,
        &mut out,
    );
    if ui.button(crate::i18n::tr("Expand all")).clicked() {
        s.groups.iter_mut().for_each(|g| g.open = true);
        ui.close();
    }
    if ui.button(crate::i18n::tr("Collapse all")).clicked() {
        s.groups.iter_mut().for_each(|g| g.open = false);
        ui.close();
    }
    ui.separator();
    item(
        ui,
        true,
        crate::i18n::tr("Delete empty parameters"),
        ParamCmd::DeleteEmptyParams,
        &mut out,
    );
    item(
        ui,
        true,
        crate::i18n::tr("Delete empty parameter groups"),
        ParamCmd::DeleteEmptyGroups,
        &mut out,
    );
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

    /// Batch Reflect: Reflect on every object keyed on each ticked
    /// parameter, as one undo step.
    pub(super) fn batch_reflect(&mut self, ticks: &[bool]) {
        let Some(s) = &mut self.session else { return };
        let params: Vec<usize> =
            ticks.iter().enumerate().filter(|(_, t)| **t).map(|(i, _)| i).collect();
        let mut n = 0;
        s.edit(|m| {
            let objects: Vec<Target> = (0..m.art_meshes.len())
                .map(Target::ArtMesh)
                .chain((0..m.deformers.len()).map(Target::Deformer))
                .chain((0..m.parts.len()).map(Target::Part))
                .collect();
            for &p in &params {
                for &t in &objects {
                    n += usize::from(keyform::reflect_keys(m, t, p));
                }
            }
            n > 0
        });
        log::info!("batch reflect: {} parameter(s), {n} keyed object(s)", params.len());
    }

    /// What a drag carries becomes the move's subject: a dragged parameter
    /// brings everything picked with it, as Select for Movement does.
    fn drop_subject(&mut self, moving: Moving) {
        self.pmenu.subject_set = match moving {
            Moving::Param(p) => self.picked_with(p),
            Moving::Group(_) => Vec::new(),
        };
        self.pmenu.subject = Some(moving);
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
                    blend_shape: false,
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
                let vertical =
                    self.session.as_ref().is_some_and(|s| up_and_down(s.puppet.model(), pi));
                self.pmenu.mirror = Some(Mirror { param: pi, horizontal: !vertical, vertical });
            }
            ParamCmd::Curve(pi) => {
                // start from what the first selected object has
                use ayatsuri_format::model::CurveKind;
                let have = self.session.as_ref().and_then(|s| {
                    let m = s.puppet.model();
                    self.param_targets().into_iter().find_map(|t| {
                        let b = match t {
                            Target::ArtMesh(i) => m.art_meshes.get(i).map(|x| &x.binding),
                            Target::Deformer(i) => m.deformers.get(i).map(|x| &x.binding),
                            Target::Part(i) => m.parts.get(i).map(|x| &x.binding),
                        };
                        b.and_then(|b| b.curve(pi as u32))
                    })
                });
                self.pmenu.curve = Some(match have {
                    Some(CurveKind::Ellipse { points, scale }) => {
                        CurveDlg { param: pi, method: 1, points, scale }
                    }
                    Some(CurveKind::Sns { points }) => {
                        CurveDlg { param: pi, method: 2, points, scale: 1.0 }
                    }
                    None => CurveDlg { param: pi, method: 2, points: 5, scale: 1.0 },
                });
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
            ParamCmd::DropOnParam(moving, dest) => {
                self.drop_subject(moving);
                self.run_param_cmd(ParamCmd::MoveHere(dest));
            }
            ParamCmd::DropOnGroup(moving, gi) => {
                self.drop_subject(moving);
                self.run_param_cmd(ParamCmd::MoveIntoGroup(gi));
            }
            ParamCmd::DropAtEnd(moving) => {
                let Some(s) = &mut self.session else { return };
                let last = s.groups.len().saturating_sub(1);
                match moving {
                    Moving::Group(src) if src < s.groups.len() && src != last => {
                        let g = s.groups.remove(src);
                        s.groups.push(g);
                        s.modified = true;
                    }
                    Moving::Group(_) => {}
                    Moving::Param(_) => {
                        self.drop_subject(moving);
                        self.run_param_cmd(ParamCmd::MoveIntoGroup(last));
                    }
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
                // Cubism's Reflect: the key values turn around across the
                // parameter's range on the selected objects, so a mouth keyed
                // open at 0 and shut at 1 becomes shut at 0 and open at 1
                let targets = self.param_targets();
                let Some(s) = &mut self.session else { return };
                let name = s.puppet.model().parameters.get(pi).map(|p| p.id.clone());
                let mut n = 0;
                s.edit(|m| {
                    for &t in &targets {
                        n += usize::from(keyform::reflect_keys(m, t, pi));
                    }
                    n > 0
                });
                if n > 0 {
                    log::info!("reflected the keys of {name:?} on {n} object(s)");
                } else {
                    super::set_notice(
                        &mut self.notice,
                        crate::i18n::tr("Select the objects keyed on this parameter first."),
                    );
                }
            }
            ParamCmd::BatchReflect => {
                let n = self.session.as_ref().map_or(0, |s| s.puppet.model().parameters.len());
                self.pmenu.batch_reflect = Some(vec![false; n]);
            }
            ParamCmd::Adjust(pi) => {
                let v = self.session.as_ref().map_or(0.0, |s| s.puppet.value(pi));
                self.pmenu.adjust = Some((pi, v, v));
            }
            ParamCmd::Change(pi) => self.pmenu.change = Some((pi, pi)),
            ParamCmd::KeyColor(pi, k, pick) => {
                let Some(s) = &mut self.session else { return };
                set_color(s, ColorTarget::Key(pi, k), pick);
            }
            ParamCmd::CustomColor(t, now) => {
                self.pmenu.custom = Some((t, now.unwrap_or([0xE5, 0x48, 0x48])));
            }
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
            crate::windows::Dialog::new(crate::i18n::tr("Rename Folder")).open(&mut open).show(
                &ctx,
                |ui| {
                    let r = ui.text_edit_singleline(name);
                    if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                        ok = true;
                    }
                    if ui.button(crate::i18n::tr("OK")).clicked() {
                        ok = true;
                    }
                },
            );
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
            crate::windows::Dialog::new(crate::i18n::tr("Automatic Generation of Four Corner Forms")).open(&mut open).show(&ctx, |ui| {
                    crate::windows::form(ui, "corners", |ui| {
                        ui.label(crate::i18n::tr("Parameter 1 (left/right)"));
                        pick(ui, "c-pa", &mut c.pa);
                        ui.end_row();
                        ui.label(crate::i18n::tr("Parameter 2 (down/up)"));
                        pick(ui, "c-pb", &mut c.pb);
                        ui.end_row();
                        ui.label(crate::i18n::tr("Parameter value to reference"));
                        ui.horizontal(|ui| {
                            ui.radio_value(&mut c.centre, false, crate::i18n::tr("Default value"));
                            ui.radio_value(&mut c.centre, true, crate::i18n::tr("Center of parameters"));
                        });
                        ui.end_row();
                        ui.label(crate::i18n::tr("Overwrite key"));
                        egui::Grid::new("corners-ow").show(ui, |ui| {
                            ui.checkbox(&mut c.overwrite[0], crate::i18n::tr("Top left"));
                            ui.checkbox(&mut c.overwrite[1], crate::i18n::tr("Top right"));
                            ui.end_row();
                            ui.checkbox(&mut c.overwrite[2], crate::i18n::tr("Bottom left"));
                            ui.checkbox(&mut c.overwrite[3], crate::i18n::tr("Bottom right"));
                            ui.end_row();
                        });
                        ui.end_row();
                        ui.label(crate::i18n::tr("Target"));
                        ui.vertical(|ui| {
                            ui.radio_value(&mut c.all, false, crate::i18n::tr("Selected objects"));
                            ui.radio_value(&mut c.all, true, crate::i18n::tr("All objects keyed on both"));
                        });
                        ui.end_row();
                    });
                    ui.weak(crate::i18n::tr("Each corner becomes its left/right form plus its up/down form minus the centre. Make the four directions first."));
                    if ui.button(crate::i18n::tr("OK")).clicked() {
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
            crate::windows::Dialog::new(crate::i18n::tr("Delete Parameter"))
                .open(&mut open)
                .width(360.0)
                .show(&ctx, |ui| {
                    let mut uses = Vec::new();
                    if objects > 0 {
                        uses.push(crate::i18n::trf("{count} object(s)", &[("count", &objects)]));
                    }
                    if physics > 0 {
                        uses.push(crate::i18n::trf(
                            "{count} physics setting(s)",
                            &[("count", &physics)],
                        ));
                    }
                    ui.label(crate::i18n::trf(
                        "{name} is still used by {uses}.",
                        &[("name", &name), ("uses", &uses.join(crate::i18n::tr(" and ")))],
                    ));
                    ui.weak(crate::i18n::tr(
                        "Unbind it everywhere and delete it? Every object keeps the shape it \
                         has at the parameter's default, physics stops reading or driving it, \
                         and a physics group left with nothing to do is removed. Undo puts it \
                         all back.",
                    ));
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        if ui.button(crate::i18n::tr("Unbind and delete")).clicked() {
                            go = true;
                        }
                        if ui.button(crate::i18n::tr("Cancel")).clicked() {
                            cancel = true;
                        }
                    });
                });
            if go {
                self.pmenu.delete_ask = None;
                self.delete_parameter_unbinding(p);
            } else if cancel || !open {
                self.pmenu.delete_ask = None;
                log::warn!("{name} was not deleted: it is still in use");
            }
        }

        if let Some(mut ticks) = self.pmenu.batch_reflect.take() {
            let mut open = true;
            let mut run = false;
            let mut keep = true;
            crate::windows::Dialog::new(crate::i18n::tr("Batch Reflect"))
                .open(&mut open)
                .width(320.0)
                .show(&ctx, |ui| {
                    ui.weak(crate::i18n::tr(
                        "The key values of the ticked parameters turn around across their range, on every object keyed on them.",
                    ));
                    ui.add_space(4.0);
                    let mut all = !ticks.is_empty() && ticks.iter().all(|t| *t);
                    if ui.checkbox(&mut all, crate::i18n::tr("All")).changed() {
                        ticks.iter_mut().for_each(|t| *t = all);
                    }
                    ui.separator();
                    egui::ScrollArea::vertical().max_height(320.0).show(ui, |ui| {
                        for (i, t) in ticks.iter_mut().enumerate() {
                            ui.checkbox(t, names.get(i).cloned().unwrap_or_default());
                        }
                    });
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        if ui
                            .add_enabled(ticks.iter().any(|t| *t), egui::Button::new(crate::i18n::tr("OK")))
                            .clicked()
                        {
                            run = true;
                        }
                        if ui.button(crate::i18n::tr("Cancel")).clicked() {
                            keep = false;
                        }
                    });
                });
            if run {
                self.batch_reflect(&ticks);
            } else if keep && open {
                self.pmenu.batch_reflect = Some(ticks);
            }
        }

        if let Some((t, mut rgb)) = self.pmenu.custom {
            // the window holds Settings' own colour button
            let mut open = true;
            let mut done = false;
            let mut changed = false;
            let title = match t {
                ColorTarget::Key(..) => crate::i18n::tr("Key color"),
                _ => crate::i18n::tr("Label color"),
            };
            crate::windows::Dialog::new(title).open(&mut open).show(&ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label(crate::i18n::tr("Colour"));
                    changed = ui.color_edit_button_srgb(&mut rgb).changed();
                    ui.weak(crate::color_pick::hex(rgb));
                });
                ui.add_space(6.0);
                if ui.button(crate::i18n::tr("Done")).clicked() {
                    done = true;
                }
            });
            if changed {
                log::debug!("colour window: {}", crate::color_pick::hex(rgb));
                if let Some(s) = &mut self.session {
                    set_color(s, t, Some(rgb));
                }
            }
            self.pmenu.custom = if done || !open { None } else { Some((t, rgb)) };
        }

        if let Some(cd) = &mut self.pmenu.curve {
            let mut open = true;
            let mut run = false;
            crate::windows::Dialog::new(crate::i18n::tr("Extended Interpolation")).open(&mut open).show(
                &ctx,
                |ui| {
                    crate::windows::form(ui, "curve", |ui| {
                        ui.label(crate::i18n::tr("Parameter"));
                        pick(ui, "c-p", &mut cd.param);
                        ui.end_row();
                        ui.label(crate::i18n::tr("Interpolation method"));
                        ui.vertical(|ui| {
                            ui.radio_value(&mut cd.method, 0, crate::i18n::tr("Linear Interpolation"));
                            ui.radio_value(&mut cd.method, 1, crate::i18n::tr("Ellipse Interpolation"));
                            ui.radio_value(&mut cd.method, 2, crate::i18n::tr("SNS Interpolation"));
                        });
                        ui.end_row();
                        if cd.method != 0 {
                            ui.label(crate::i18n::tr("Points"));
                            ui.add(egui::DragValue::new(&mut cd.points).range(1..=20));
                            ui.end_row();
                        }
                        if cd.method == 1 {
                            ui.label(crate::i18n::tr("Scale"));
                            ui.add(egui::DragValue::new(&mut cd.scale).range(0.1..=3.0).speed(0.01));
                            ui.end_row();
                        }
                    });
                    ui.weak(crate::i18n::tr("Keyforms blend along a curve instead of a straight line, so a sway made of turned keys keeps its size. Needs three keys or more; applies to the selected objects. Points is how many keys a moc3 export adds between each pair so other apps follow the curve."));
                    if ui.button(crate::i18n::tr("OK")).clicked() {
                        run = true;
                    }
                },
            );
            if run {
                use ayatsuri_format::model::CurveKind;
                let (p, kind) = (
                    cd.param,
                    match cd.method {
                        1 => Some(CurveKind::Ellipse { points: cd.points, scale: cd.scale }),
                        2 => Some(CurveKind::Sns { points: cd.points }),
                        _ => None,
                    },
                );
                self.pmenu.curve = None;
                let targets = self.param_targets();
                if let Some(s) = &mut self.session {
                    let few = targets
                        .iter()
                        .filter(|&&t| {
                            let m = s.puppet.model();
                            let b = match t {
                                Target::ArtMesh(i) => m.art_meshes.get(i).map(|x| &x.binding),
                                Target::Deformer(i) => m.deformers.get(i).map(|x| &x.binding),
                                Target::Part(i) => m.parts.get(i).map(|x| &x.binding),
                            };
                            b.and_then(|b| b.axes.iter().find(|a| a.parameter as usize == p))
                                .is_some_and(|a| a.keys.len() < 3)
                        })
                        .count();
                    let mut n = 0;
                    s.edit(|m| {
                        for &t in &targets {
                            n += usize::from(ayatsuri_core::edit::set_curve(m, t, p, kind));
                        }
                        n > 0
                    });
                    s.dirty = true;
                    if n == 0 {
                        log::warn!(
                            "extended interpolation: select objects keyed on this parameter"
                        );
                    } else {
                        log::info!("extended interpolation set on {n} object(s)");
                    }
                    if few > 0 && kind.is_some() {
                        log::warn!(
                            "extended interpolation needs three keys: {few} object(s) stay straight until they have them"
                        );
                    }
                }
            } else if !open {
                self.pmenu.curve = None;
            }
        }

        if let Some(mi) = &mut self.pmenu.mirror {
            let mut open = true;
            let mut run = false;
            crate::windows::Dialog::new(crate::i18n::tr("Motion Mirroring")).open(&mut open).show(
                &ctx,
                |ui| {
                    crate::windows::form(ui, "mirror", |ui| {
                        ui.label(crate::i18n::tr("Parameter"));
                        pick(ui, "m-p", &mut mi.param);
                        ui.end_row();
                        ui.label(crate::i18n::tr("Flip settings"));
                        ui.vertical(|ui| {
                            ui.checkbox(&mut mi.horizontal, crate::i18n::tr("Flip horizontally"));
                            ui.checkbox(&mut mi.vertical, crate::i18n::tr("Flip vertically"));
                        });
                        ui.end_row();
                    });
                    ui.weak(crate::i18n::tr("The key at the current value is mirrored across the default into the opposite key, on the selected objects, parents first. Warps mirror around their centre; meshes mirror their movement in place."));
                    if ui.button(crate::i18n::tr("OK")).clicked() {
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
                    if let Some(k) = wrote {
                        self.mirror_mark = Some((p, k, self.param_targets()));
                    }
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
            crate::windows::Dialog::new(crate::i18n::tr("Adjust Value")).open(&mut open).show(
                &ctx,
                |ui| {
                    ui.label(names.get(*p).cloned().unwrap_or_default());
                    crate::windows::form(ui, "p-adjust", |ui| {
                        ui.label(crate::i18n::tr("Key value"));
                        ui.add(egui::DragValue::new(from).speed(0.1));
                        ui.end_row();
                        ui.label(crate::i18n::tr("New value"));
                        ui.add(egui::DragValue::new(to).speed(0.1));
                        ui.end_row();
                    });
                    ui.weak(crate::i18n::tr(
                        "Moves the key (on the selected objects) keeping its form.",
                    ));
                    if ui.button(crate::i18n::tr("OK")).clicked() {
                        ok = true;
                    }
                },
            );
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
            crate::windows::Dialog::new(crate::i18n::tr("Change Parameters")).open(&mut open).show(
                &ctx,
                |ui| {
                    ui.label(crate::i18n::trf(
                        "Move the keys of {name} to:",
                        &[("name", &names.get(*from).cloned().unwrap_or_default())],
                    ));
                    egui::ComboBox::from_id_salt("p-change")
                        .selected_text(names.get(*to).cloned().unwrap_or_default())
                        .show_ui(ui, |ui| {
                            for (i, n) in names.iter().enumerate() {
                                ui.selectable_value(to, i, n);
                            }
                        });
                    if ui.button(crate::i18n::tr("OK")).clicked() {
                        ok = true;
                    }
                },
            );
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

/// A name that drags: the drag carries `what`, and a drop of another
/// parameter or folder here pushes `drop(what_was_dropped)`. While one
/// hovers, a line in the accent colour shows where it lands.
pub(super) fn drag_target(
    ui: &Ui,
    resp: &egui::Response,
    line: egui::Rect,
    what: Moving,
    drop: impl FnOnce(Moving) -> ParamCmd,
    out: &mut Option<ParamCmd>,
) {
    if resp.drag_started_by(egui::PointerButton::Primary) {
        resp.dnd_set_drag_payload(what);
    }
    if let Some(m) = resp.dnd_hover_payload::<Moving>()
        && *m != what
    {
        ui.painter().hline(
            line.x_range(),
            line.top(),
            egui::Stroke::new(2.0, crate::theme::pal().accent),
        );
    }
    if let Some(m) = resp.dnd_release_payload::<Moving>()
        && *m != what
    {
        *out = Some(drop(*m));
    }
}

/// An up and down parameter: its id ends in Y, as Angle Y, Body Angle Y,
/// Eye Ball Y and Brow L Y do in the standard set. Motion Mirroring starts
/// with Flip vertically ticked for these, Flip horizontally otherwise.
pub(super) fn up_and_down(model: &ayatsuri_format::model::Model, param: usize) -> bool {
    model.parameters.get(param).is_some_and(|p| {
        let id = p.id.trim_end_matches(|c: char| c.is_ascii_digit() || c == '_');
        id.ends_with('Y') && id.len() > 1
    })
}
