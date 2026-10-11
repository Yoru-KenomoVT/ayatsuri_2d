use std::path::PathBuf;

use ayatsuri_format::Model;
use ayatsuri_format::model::DeformerKind;
use egui::{
    Align2, CentralPanel, Color32, FontId, Frame, Panel, Pos2, Rect, ScrollArea, Sense, Stroke,
    StrokeKind, Ui, UiBuilder, Vec2, pos2, vec2,
};

use crate::dock::{self, DockAction, DockNode, PanelKind};
use crate::logging;
use crate::prefs::Prefs;
use crate::session::{Sel, Session};
use crate::theme;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tool {
    Arrow,
    Lasso,
    BrushSelect,
    MeshManual,
    MeshAuto,
    WarpCreate,
    RotationCreate,
    DeformPath,
    DeformBrush,
    Glue,
    ArtPath,
    TextureAtlas,
}

impl Tool {
    // grouped the way Cubism's modeling toolbar is
    const GROUPS: [&'static [Tool]; 4] = [
        &[Tool::TextureAtlas],
        &[Tool::MeshManual, Tool::MeshAuto],
        &[Tool::WarpCreate, Tool::RotationCreate],
        &[
            Tool::Arrow,
            Tool::Lasso,
            Tool::BrushSelect,
            Tool::DeformPath,
            Tool::DeformBrush,
            Tool::Glue,
            Tool::ArtPath,
        ],
    ];
    fn icon(self) -> &'static str {
        match self {
            Tool::Arrow => egui_phosphor::regular::CURSOR,
            Tool::Lasso => egui_phosphor::regular::LASSO,
            Tool::BrushSelect => egui_phosphor::regular::PAINT_BRUSH,
            Tool::MeshManual => egui_phosphor::regular::POLYGON,
            Tool::MeshAuto => egui_phosphor::regular::MAGIC_WAND,
            Tool::WarpCreate => egui_phosphor::regular::GRID_FOUR,
            Tool::RotationCreate => egui_phosphor::regular::ARROW_CLOCKWISE,
            Tool::DeformPath => egui_phosphor::regular::WAVE_SINE,
            Tool::DeformBrush => egui_phosphor::regular::PENCIL_SIMPLE_LINE,
            Tool::Glue => "",
            Tool::ArtPath => egui_phosphor::regular::PEN_NIB,
            Tool::TextureAtlas => egui_phosphor::regular::SQUARES_FOUR,
        }
    }
    fn label(self) -> &'static str {
        match self {
            Tool::Arrow => "Select",
            Tool::Lasso => "Lasso",
            Tool::BrushSelect => "Brush Selection",
            Tool::MeshManual => "Edit Mesh",
            Tool::MeshAuto => "Auto Mesh",
            Tool::WarpCreate => "Warp Deformer",
            Tool::RotationCreate => "Rotation Deformer",
            Tool::DeformPath => "Deform Path",
            Tool::DeformBrush => "Deform Brush",
            Tool::Glue => "Glue",
            Tool::ArtPath => "ArtPath",
            Tool::TextureAtlas => "Texture Atlas",
        }
    }
}

/// Per-tool options shown in the Tool Details palette.
struct ToolOptions {
    brush_size: f32,
    mesh_mode: usize,
    mesh_snap: bool,
    auto_preset: usize,
    auto_spacing: f32,
    auto_margin: f32,
    warp_cols: u32,
    warp_rows: u32,
    sequential: bool,
    glue_weight: f32,
}

impl Default for ToolOptions {
    fn default() -> Self {
        Self {
            brush_size: 40.0,
            mesh_mode: 0,
            mesh_snap: true,
            auto_preset: 0,
            auto_spacing: 24.0,
            auto_margin: 8.0,
            warp_cols: 5,
            warp_rows: 5,
            sequential: false,
            glue_weight: 0.5,
        }
    }
}

#[derive(Clone, PartialEq, Eq)]
enum Workspace {
    Modeling,
    Animation,
    Custom(usize),
}

impl Workspace {
    fn name(&self, prefs: &Prefs) -> String {
        match self {
            Workspace::Modeling => "Modeling".into(),
            Workspace::Animation => "Animation".into(),
            Workspace::Custom(i) => prefs
                .custom_workspaces
                .get(*i)
                .map(|(n, _)| n.clone())
                .unwrap_or_else(|| "Custom".into()),
        }
    }
}

/// The editor workspace: a dockable tree of palettes around one loaded
/// puppet, laid out and styled after Cubism Editor's Modeling workspace.
pub struct EditorUi {
    layout: DockNode,
    workspace: Workspace,
    session: Option<Session>,
    selected: Option<Sel>,
    log_view: log::Level,
    pending_import: Option<PathBuf>,
    /// View zoom relative to fit-to-panel, and pan in screen points.
    view_zoom: f32,
    view_pan: Vec2,
    prefs: Prefs,
    tool: Tool,
    show_about: bool,
    show_settings: bool,
    show_stats: bool,
    show_physics: bool,
    phys_sel: usize,
    phys_tab: usize,
    /// The physics dialog's working copy; Apply commits it as one edit.
    phys_edit: Option<ayatsuri_format::model::PhysicsRig>,
    phys_auto_max: bool,
    phys_peak_cache: Vec<(usize, f32)>,
    phys_preset: usize,
    phys_zoom: f32,
    /// The dialog preview's own little pendulum: (pos, prev) per node plus
    /// a hash of the settings that built it.
    phys_prev: Vec<(f32, f32, f32, f32)>,
    phys_prev_sig: u64,
    phys_prev_drive: f32,
    /// The physics dialog's Valid checkbox: the rig only ticks while on.
    physics_on: bool,
    /// Drive the standard face/body/eye parameters from the cursor.
    follow_cursor: bool,
    follow: Vec2,
    /// Cubism's edit level: 1 meshes, 2 deformers, 3 structure.
    edit_level: u8,
    /// The action waiting for its new shortcut in settings, if any.
    rebind: Option<&'static str>,
    /// Last states the debug log reported, so changes log exactly once.
    logged_sel: Option<Sel>,
    logged_param: Option<usize>,
    logged_tool: Tool,
    heartbeat: f32,
    new_param: Option<ParamNew>,
    new_folder: Option<String>,
    new_model: Option<ModelNew>,
    show_model_info: bool,
    gpu: bool,
    theme_applied: bool,
    last_layout: DockNode,
    logo: Option<TextureHandleCache>,
    show_ws_save: bool,
    ws_name: String,
    show_ws_manage: bool,
    settings_tab: usize,
    tool_opts: ToolOptions,
    lasso: Vec<Pos2>,
    /// The parameter rows' own selection, independent of object selection.
    sel_param: Option<usize>,
    param_edit: Option<ParamEdit>,
    /// Extra meshes picked by the lasso, beyond `selected`.
    multi_sel: Vec<usize>,
    /// A deformer creation requested from inside a borrow (canvas, tool pane).
    pending_create: Option<(bool, Option<(f32, f32)>)>,
    /// Mesh edit mode: the mesh whose vertices are being edited, if any.
    mesh_edit: Option<usize>,
    mesh_drag: Option<MeshDrag>,
    /// 0 move, 1 add vertices, 2 erase vertices.
    mesh_mode: u8,
    /// The model as it was when mesh editing began, for Cancel.
    mesh_snapshot: Option<Model>,
    deform_path: Option<PathDeform>,
    warp_drag: Option<WarpDrag>,
    rot_drag: Option<RotDrag>,
    /// 1 = quit the app, 2 = close the session, shown while unsaved work
    /// would be lost.
    confirm_close: Option<u8>,
    force_close: bool,
    home_thumbs: std::collections::HashMap<PathBuf, Option<egui::TextureHandle>>,
}

/// One vertex being dragged in mesh edit mode. The parent-space offset is
/// banked as the pointer moves and applied as a single edit on release.
struct MeshDrag {
    vertex: usize,
    /// Blended parent-space positions captured when the drag began.
    local: Vec<f32>,
    dx: f32,
    dy: f32,
}

/// A warp grid control point mid-drag, banked and applied on release the
/// same way mesh vertex drags are.
struct WarpDrag {
    deformer: usize,
    point: usize,
    /// Blended parent-space grid captured at drag start.
    local: Vec<f32>,
    dx: f32,
    dy: f32,
}

/// A rotation deformer grab: either the origin sliding or the handle
/// turning, applied as one edit when the mouse lets go.
struct RotDrag {
    deformer: usize,
    /// true while the angle handle is held, false for the origin dot.
    turning: bool,
    local_origin: (f32, f32),
    dx: f32,
    dy: f32,
    degrees: f32,
    last_ptr: Pos2,
}

/// The deform path tool's working state: control points laid over one mesh,
/// and the falloff drag in flight if there is one.
struct PathDeform {
    mesh: usize,
    /// Control points in model space.
    points: Vec<(f32, f32)>,
    drag: Option<PathDrag>,
}

struct PathDrag {
    point: usize,
    /// Affected vertices and their falloff weights, fixed at drag start.
    weights: Vec<(usize, f32)>,
    /// Blended parent-space positions captured at drag start.
    local: Vec<f32>,
    /// Accumulated drag in model units.
    mdx: f32,
    mdy: f32,
}

/// The New Model dialog's working copy.
struct ModelNew {
    name: String,
    author: String,
    width: f32,
    height: f32,
    ppu: f32,
    std_params: bool,
}

impl Default for ModelNew {
    fn default() -> Self {
        Self {
            name: "untitled".into(),
            author: String::new(),
            width: 2048.0,
            height: 2048.0,
            ppu: 1024.0,
            std_params: true,
        }
    }
}

/// The Add Parameter dialog's working copy.
struct ParamNew {
    name: String,
    id: String,
    min: f32,
    default: f32,
    max: f32,
    group: usize,
}

/// The Edit Parameter dialog's working copy.
struct ParamEdit {
    index: usize,
    name: String,
    id: String,
    min: f32,
    default: f32,
    max: f32,
    repeat: bool,
}

struct TextureHandleCache {
    dark: bool,
    handle: egui::TextureHandle,
}

impl Default for EditorUi {
    fn default() -> Self {
        let prefs = Prefs::load();
        let pending_import = if prefs.reopen_last { prefs.recents.first().cloned() } else { None };
        let layout = prefs.workspace_modeling.clone().unwrap_or_else(dock::default_workspace);
        Self {
            last_layout: layout.clone(),
            layout,
            workspace: Workspace::Modeling,
            session: None,
            selected: None,
            log_view: log::Level::Info,
            pending_import,
            view_zoom: 1.0,
            view_pan: Vec2::ZERO,
            prefs,
            tool: Tool::Arrow,
            show_about: false,
            show_settings: false,
            show_stats: false,
            show_physics: false,
            phys_sel: 0,
            phys_tab: 0,
            phys_edit: None,
            phys_auto_max: true,
            phys_peak_cache: Vec::new(),
            phys_preset: 0,
            phys_zoom: 1.0,
            phys_prev: Vec::new(),
            phys_prev_sig: 0,
            phys_prev_drive: 0.0,
            physics_on: true,
            follow_cursor: false,
            follow: Vec2::ZERO,
            edit_level: 2,
            rebind: None,
            logged_sel: None,
            logged_param: None,
            logged_tool: Tool::Arrow,
            heartbeat: 0.0,
            new_param: None,
            new_folder: None,
            new_model: None,
            show_model_info: false,
            gpu: false,
            theme_applied: false,
            logo: None,
            show_ws_save: false,
            ws_name: String::new(),
            show_ws_manage: false,
            settings_tab: 0,
            tool_opts: ToolOptions::default(),
            lasso: Vec::new(),
            sel_param: None,
            param_edit: None,
            multi_sel: Vec::new(),
            pending_create: None,
            mesh_edit: None,
            mesh_drag: None,
            mesh_mode: 0,
            mesh_snapshot: None,
            deform_path: None,
            warp_drag: None,
            rot_drag: None,
            confirm_close: None,
            force_close: false,
            home_thumbs: std::collections::HashMap::new(),
        }
    }
}

impl EditorUi {
    pub fn new() -> Self {
        Self::default()
    }

    /// Import this model on the first frame (the CLI's `ayatsuri model.moc3`).
    pub fn open_on_start(&mut self, path: PathBuf) {
        self.pending_import = Some(path);
    }

    /// The app flips this on when a wgpu renderer is registered; the view
    /// then draws through it (with clipping masks) instead of egui meshes.
    pub fn set_gpu_available(&mut self, on: bool) {
        self.gpu = on;
    }

    fn import(&mut self, ctx: &egui::Context, path: PathBuf) {
        match Session::load(ctx, &path) {
            Ok(s) => {
                self.prefs.remember(path.clone());
                self.session = Some(s);
                self.selected = None;
                self.view_zoom = 1.0;
                self.view_pan = Vec2::ZERO;
            }
            Err(e) => log::error!("open failed: {e}"),
        }
    }

    fn import_dialog(&mut self, ctx: &egui::Context) {
        let picked = rfd::FileDialog::new()
            .add_filter("Ayatsuri / Live2D model", &["aom", "moc3", "json"])
            .set_title("Open model")
            .pick_file();
        if let Some(path) = picked {
            self.import(ctx, path);
        }
    }

    fn save_session(&mut self, save_as: bool) {
        let Some(s) = &mut self.session else { return };
        let path = if save_as || s.aom_path.is_none() {
            rfd::FileDialog::new()
                .add_filter("Ayatsuri Open Model", &["aom"])
                .set_file_name(format!("{}.aom", s.name))
                .set_title("Save model")
                .save_file()
        } else {
            s.aom_path.clone()
        };
        let Some(path) = path else { return };
        match s.save_aom(&path) {
            Ok(()) => self.prefs.remember(path),
            Err(e) => log::error!("save failed: {e}"),
        }
    }

    fn set_workspace(&mut self, w: Workspace) {
        self.save_layout();
        self.layout = match &w {
            Workspace::Modeling => {
                self.prefs.workspace_modeling.clone().unwrap_or_else(dock::default_workspace)
            }
            Workspace::Animation => {
                self.prefs.workspace_animation.clone().unwrap_or_else(dock::animation_workspace)
            }
            Workspace::Custom(i) => self
                .prefs
                .custom_workspaces
                .get(*i)
                .map(|(_, l)| l.clone())
                .unwrap_or_else(dock::default_workspace),
        };
        self.workspace = w;
        self.last_layout = self.layout.clone();
    }

    fn reset_workspace(&mut self) {
        self.layout = match self.workspace {
            Workspace::Modeling | Workspace::Custom(_) => dock::default_workspace(),
            Workspace::Animation => dock::animation_workspace(),
        };
        self.save_layout();
    }

    fn save_layout(&mut self) {
        match self.workspace {
            Workspace::Modeling => self.prefs.workspace_modeling = Some(self.layout.clone()),
            Workspace::Animation => self.prefs.workspace_animation = Some(self.layout.clone()),
            Workspace::Custom(i) => {
                if let Some((_, l)) = self.prefs.custom_workspaces.get_mut(i) {
                    *l = self.layout.clone();
                }
            }
        }
        self.prefs.save();
        self.last_layout = self.layout.clone();
    }

    pub fn ui(&mut self, ui: &mut Ui) {
        if !self.theme_applied {
            let ctx = ui.ctx().clone();
            if let Some([r, g, b]) = self.prefs.accent {
                theme::set_accent(&ctx, Some(Color32::from_rgb(r, g, b)));
            }
            theme::set_dark(&ctx, self.prefs.dark.unwrap_or(true));
            if let Some(level) = self.prefs.log_level.as_deref() {
                logging::set_level(logging::level_from_str(level));
            }
            self.theme_applied = true;
        }
        if let Some(path) = self.pending_import.take() {
            self.import(&ui.ctx().clone(), path);
        }
        // one-line debug logs for every state change, so the Log panel can
        // tell the whole story when something misbehaves
        if self.selected != self.logged_sel {
            if let Some(s) = &self.session {
                let m = s.puppet.model();
                let what = match self.selected {
                    Some(Sel::Mesh(i)) => format!(
                        "mesh {i} ({})",
                        m.art_meshes.get(i).map(|a| a.id.as_str()).unwrap_or("?")
                    ),
                    Some(Sel::Deformer(i)) => format!(
                        "deformer {i} ({})",
                        m.deformers.get(i).map(|d| d.id.as_str()).unwrap_or("?")
                    ),
                    Some(Sel::Part(i)) => format!(
                        "part {i} ({})",
                        m.parts.get(i).map(|p| p.id.as_str()).unwrap_or("?")
                    ),
                    Some(Sel::Parameter(i)) => format!(
                        "parameter {i} ({})",
                        m.parameters.get(i).map(|p| p.id.as_str()).unwrap_or("?")
                    ),
                    None => "nothing".into(),
                };
                log::debug!("selection: {what}");
            }
            self.logged_sel = self.selected;
        }
        if self.sel_param != self.logged_param {
            if let (Some(s), Some(pi)) = (&self.session, self.sel_param) {
                log::debug!(
                    "parameter row selected: {pi} ({})",
                    s.puppet.model().parameters.get(pi).map(|p| p.id.as_str()).unwrap_or("?")
                );
            }
            self.logged_param = self.sel_param;
        }
        if self.tool != self.logged_tool {
            log::debug!("tool: {}", self.tool.label());
            self.logged_tool = self.tool;
        }

        // the title carries the unsaved marker, like every other editor
        {
            let title = match &self.session {
                Some(s) if s.modified => format!("Ayatsuri2D · {} *", s.name),
                Some(s) => format!("Ayatsuri2D · {}", s.name),
                None => "Ayatsuri2D".into(),
            };
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Title(title));
        }
        // closing with unsaved work stops at a dialog first
        if ui.ctx().input(|i| i.viewport().close_requested())
            && !self.force_close
            && self.session.as_ref().is_some_and(|s| s.modified)
        {
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.confirm_close = Some(1);
        }
        let undo_sc = self.bound("undo");
        let redo_sc = self.bound("redo");
        // redo first: its modifier set is a superset of undo's
        if ui.ctx().input_mut(|i| i.consume_shortcut(&redo_sc)) {
            if let Some(s) = &mut self.session {
                s.redo();
            }
        } else if ui.ctx().input_mut(|i| i.consume_shortcut(&undo_sc))
            && let Some(s) = &mut self.session
        {
            s.undo();
        }
        let save_sc = self.bound("save");
        if ui.ctx().input_mut(|i| i.consume_shortcut(&save_sc)) {
            self.save_session(false);
        }
        let export_sc = self.bound("export-image");
        if ui.ctx().input_mut(|i| i.consume_shortcut(&export_sc)) {
            self.export_image();
        }
        // Ctrl+E toggles mesh edit mode on the selected mesh, like Cubism
        let mesh_sc = self.bound("mesh-edit");
        if ui.ctx().input_mut(|i| i.consume_shortcut(&mesh_sc)) {
            if self.mesh_edit.is_some() {
                self.mesh_edit = None;
                self.mesh_drag = None;
            } else if let Some(Sel::Mesh(mi)) = self.selected {
                self.mesh_edit = Some(mi);
                self.mesh_snapshot = self.session.as_ref().map(|s| s.puppet.model().clone());
            }
        }
        // Ctrl+1 resets every parameter to its default, Ctrl+3 toggles the
        // selected meshes' visibility, both straight from Cubism
        let reset_sc = self.bound("reset-params");
        if ui.ctx().input_mut(|i| i.consume_shortcut(&reset_sc)) {
            self.reset_parameters();
        }
        let hide_sc = self.bound("toggle-visible");
        if ui.ctx().input_mut(|i| i.consume_shortcut(&hide_sc)) {
            self.toggle_selected_visibility();
        }
        let dropped: Vec<PathBuf> = ui
            .ctx()
            .input(|i| i.raw.dropped_files.iter().map(|f| f.path().to_path_buf()).collect());
        for path in dropped {
            self.import(&ui.ctx().clone(), path);
        }

        Panel::top("menu").show(ui, |ui| {
            egui::MenuBar::new().ui(ui, |ui| {
                ui.menu_button("File", |ui| {
                    if ui.button("New…").clicked() {
                        ui.close();
                        self.new_model = Some(ModelNew::default());
                    }
                    if ui.button("Open…").clicked() {
                        ui.close();
                        self.import_dialog(&ui.ctx().clone());
                    }
                    ui.menu_button("Open Recent", |ui| {
                        let recents = self.prefs.recents.clone();
                        if recents.is_empty() {
                            ui.weak("empty");
                        }
                        for r in recents {
                            let name = r
                                .file_name()
                                .map(|n| n.to_string_lossy().into_owned())
                                .unwrap_or_default();
                            if ui.button(name).clicked() {
                                ui.close();
                                self.import(&ui.ctx().clone(), r);
                            }
                        }
                    });
                    ui.separator();
                    if ui.add_enabled(self.session.is_some(), egui::Button::new("Save")).clicked() {
                        ui.close();
                        self.save_session(false);
                    }
                    if ui
                        .add_enabled(self.session.is_some(), egui::Button::new("Save As…"))
                        .clicked()
                    {
                        ui.close();
                        self.save_session(true);
                    }
                    ui.separator();
                    if ui
                        .add_enabled(self.session.is_some(), egui::Button::new("Export Image…"))
                        .clicked()
                    {
                        ui.close();
                        self.export_image();
                    }
                    let _ = ui.add_enabled(false, egui::Button::new("Export .moc3…"));
                    ui.separator();
                    if ui
                        .add_enabled(self.session.is_some(), egui::Button::new("Model Info…"))
                        .clicked()
                    {
                        self.show_model_info = true;
                        ui.close();
                    }
                    if ui
                        .add_enabled(self.session.is_some(), egui::Button::new("Model Statistics…"))
                        .clicked()
                    {
                        self.show_stats = true;
                        ui.close();
                    }
                    ui.separator();
                    if ui.button("Environment Settings…").clicked() {
                        self.show_settings = true;
                        ui.close();
                    }
                    if ui.button("Keyboard Shortcuts…").clicked() {
                        self.show_settings = true;
                        self.settings_tab = 4;
                        ui.close();
                    }
                    ui.separator();
                    if ui.add_enabled(self.session.is_some(), egui::Button::new("Close")).clicked()
                    {
                        ui.close();
                        if self.session.as_ref().is_some_and(|s| s.modified) {
                            self.confirm_close = Some(2);
                        } else {
                            self.close_session();
                        }
                    }
                    if ui.button("Exit").clicked() {
                        ui.close();
                        ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                });
                ui.menu_button("Edit", |ui| {
                    let (can_undo, can_redo) = self
                        .session
                        .as_ref()
                        .map(|s| (s.can_undo(), s.can_redo()))
                        .unwrap_or((false, false));
                    if ui.add_enabled(can_undo, egui::Button::new("Undo")).clicked() {
                        ui.close();
                        if let Some(s) = &mut self.session {
                            s.undo();
                        }
                    }
                    if ui.add_enabled(can_redo, egui::Button::new("Redo")).clicked() {
                        ui.close();
                        if let Some(s) = &mut self.session {
                            s.redo();
                        }
                    }
                    ui.separator();
                    let _ = ui.add_enabled(false, egui::Button::new("Cut"));
                    let _ = ui.add_enabled(false, egui::Button::new("Copy"));
                    let _ = ui.add_enabled(false, egui::Button::new("Paste"));
                });
                ui.menu_button("Show", |ui| {
                    let has = self.session.is_some();
                    if ui.add_enabled(has, egui::Button::new("Zoom In")).clicked() {
                        self.view_zoom = (self.view_zoom * 1.25).clamp(0.02, 40.0);
                        ui.close();
                    }
                    if ui.add_enabled(has, egui::Button::new("Zoom Out")).clicked() {
                        self.view_zoom = (self.view_zoom / 1.25).clamp(0.02, 40.0);
                        ui.close();
                    }
                    if ui.add_enabled(has, egui::Button::new("Fit")).clicked() {
                        self.view_zoom = 1.0;
                        self.view_pan = Vec2::ZERO;
                        ui.close();
                    }
                });
                ui.menu_button("Modeling", |ui| {
                    if ui.button("Modeling Workspace").clicked() {
                        self.set_workspace(Workspace::Modeling);
                        ui.close();
                    }
                    ui.separator();
                    let has_sel = !self.selection_meshes().is_empty();
                    ui.menu_button("Deformer", |ui| {
                        if ui
                            .add_enabled(has_sel, egui::Button::new("Create Warp Deformer"))
                            .clicked()
                        {
                            self.pending_create = Some((true, None));
                            ui.close();
                        }
                        if ui
                            .add_enabled(has_sel, egui::Button::new("Create Rotation Deformer"))
                            .clicked()
                        {
                            self.pending_create = Some((false, None));
                            ui.close();
                        }
                    });
                    let can_keys = self.edit_target().is_some() && self.sel_param.is_some();
                    ui.menu_button("Parameter", |ui| {
                        if ui
                            .add_enabled(can_keys, egui::Button::new("Add Keys at Min and Max"))
                            .clicked()
                        {
                            ui.close();
                            if let Some(param) = self.sel_param
                                && let Some(meta) = self
                                    .session
                                    .as_ref()
                                    .and_then(|s| s.puppet.model().parameters.get(param).cloned())
                            {
                                self.add_keys_to_selection(vec![meta.min, meta.max]);
                            }
                        }
                        if ui
                            .add_enabled(
                                can_keys,
                                egui::Button::new("Add Keys at Min, Default and Max"),
                            )
                            .clicked()
                        {
                            ui.close();
                            if let Some(param) = self.sel_param
                                && let Some(meta) = self
                                    .session
                                    .as_ref()
                                    .and_then(|s| s.puppet.model().parameters.get(param).cloned())
                            {
                                self.add_keys_to_selection(vec![meta.min, meta.default, meta.max]);
                            }
                        }
                        if ui
                            .add_enabled(can_keys, egui::Button::new("Delete Nearest Key"))
                            .clicked()
                        {
                            ui.close();
                            self.delete_key_on_selection();
                        }
                        ui.separator();
                        if ui
                            .add_enabled(
                                self.session.is_some(),
                                egui::Button::new("Reset to Default        Ctrl+1"),
                            )
                            .clicked()
                        {
                            ui.close();
                            self.reset_parameters();
                        }
                    });
                    let mesh_sel = matches!(self.selected, Some(Sel::Mesh(_)));
                    ui.menu_button("ArtMesh", |ui| {
                        if ui
                            .add_enabled(mesh_sel, egui::Button::new("Edit Mesh        Ctrl+E"))
                            .clicked()
                        {
                            ui.close();
                            if let Some(Sel::Mesh(mi)) = self.selected {
                                self.mesh_edit = Some(mi);
                                self.mesh_drag = None;
                                self.mesh_snapshot =
                                    self.session.as_ref().map(|s| s.puppet.model().clone());
                            }
                        }
                        if ui
                            .add_enabled(mesh_sel, egui::Button::new("Auto Mesh Generation"))
                            .clicked()
                        {
                            ui.close();
                            if let Some(Sel::Mesh(mi)) = self.selected {
                                self.auto_mesh(mi);
                            }
                        }
                        let _ = ui.add_enabled(false, egui::Button::new("Convert to ArtPath"));
                    });
                    ui.menu_button("ArtPath", |ui| {
                        let _ = ui.add_enabled(false, egui::Button::new("Create ArtPath"));
                    });
                    ui.menu_button("Glue", |ui| {
                        let _ = ui.add_enabled(false, egui::Button::new("Bind Glue"));
                        let _ = ui.add_enabled(false, egui::Button::new("Unbind Glue"));
                    });
                    ui.menu_button("Skinning", |ui| {
                        let _ = ui.add_enabled(false, egui::Button::new("Apply Skinning…"));
                    });
                    ui.menu_button("Edit Form", |ui| {
                        let _ = ui.add_enabled(false, egui::Button::new("Flip Horizontally"));
                        let _ = ui.add_enabled(false, egui::Button::new("Flip Vertically"));
                        let two_axis = self.two_axis_target();
                        if ui
                            .add_enabled(two_axis, egui::Button::new("Generate Four Corner Forms"))
                            .clicked()
                        {
                            ui.close();
                            self.four_corner_forms();
                        }
                    });
                    let _ = ui.add_enabled(false, egui::Button::new("Temporary Deform Tool"));
                    ui.separator();
                    if ui
                        .add_enabled(
                            !self.selection_meshes().is_empty(),
                            egui::Button::new("Show/Hide Selected        Ctrl+3"),
                        )
                        .clicked()
                    {
                        ui.close();
                        self.toggle_selected_visibility();
                    }
                    let _ = ui.add_enabled(false, egui::Button::new("Lock Selected"));
                    ui.separator();
                    ui.menu_button("Edit Level", |ui| {
                        for lvl in 1..=3u8 {
                            if ui
                                .selectable_label(self.edit_level == lvl, format!("Level {lvl}"))
                                .clicked()
                            {
                                self.edit_level = lvl;
                                ui.close();
                            }
                        }
                    });
                    ui.separator();
                    if ui
                        .add_enabled(self.session.is_some(), egui::Button::new("Physics Settings…"))
                        .clicked()
                    {
                        self.show_physics = true;
                        ui.close();
                    }
                    ui.separator();
                    let _ = ui.add_enabled(false, egui::Button::new("Texture Atlas…"));
                    let _ = ui.add_enabled(false, egui::Button::new("Model Template…"));
                });
                ui.menu_button("Animation", |ui| {
                    if ui.button("Animation Workspace").clicked() {
                        self.set_workspace(Workspace::Animation);
                        ui.close();
                    }
                });
                ui.menu_button("Window", |ui| {
                    for p in PanelKind::ALL {
                        let open = self.layout.contains(p);
                        if ui.selectable_label(open, p.title()).clicked() {
                            if open {
                                self.layout.close(p);
                            } else {
                                self.layout.open_near(p, PanelKind::Viewport);
                            }
                            ui.close();
                        }
                    }
                    ui.separator();
                    ui.menu_button("Workspace", |ui| {
                        if ui.button("Save as new workspace…").clicked() {
                            self.show_ws_save = true;
                            ui.close();
                        }
                        if ui.button("Manage workspaces…").clicked() {
                            self.show_ws_manage = true;
                            ui.close();
                        }
                        ui.separator();
                        if ui.button("Export current layout…").clicked() {
                            ui.close();
                            let picked = rfd::FileDialog::new()
                                .add_filter("Workspace", &["json"])
                                .set_file_name("workspace.json")
                                .save_file();
                            if let Some(path) = picked {
                                match serde_json::to_vec_pretty(&self.layout) {
                                    Ok(bytes) => {
                                        if let Err(e) = std::fs::write(&path, bytes) {
                                            log::error!("workspace export: {e}");
                                        } else {
                                            log::info!("workspace exported to {}", path.display());
                                        }
                                    }
                                    Err(e) => log::error!("workspace export: {e}"),
                                }
                            }
                        }
                        if ui.button("Import layout…").clicked() {
                            ui.close();
                            let picked = rfd::FileDialog::new()
                                .add_filter("Workspace", &["json"])
                                .pick_file();
                            if let Some(path) = picked {
                                match std::fs::read(&path).map_err(|e| e.to_string()).and_then(
                                    |b| {
                                        serde_json::from_slice::<DockNode>(&b)
                                            .map_err(|e| e.to_string())
                                    },
                                ) {
                                    Ok(layout) => {
                                        self.layout = layout;
                                        self.save_layout();
                                        log::info!("workspace imported from {}", path.display());
                                    }
                                    Err(e) => log::error!("workspace import: {e}"),
                                }
                            }
                        }
                    });
                    if ui.button("Reset current layout").clicked() {
                        self.reset_workspace();
                        ui.close();
                    }
                });
                ui.menu_button("Help", |ui| {
                    if ui.button("About Ayatsuri2D").clicked() {
                        self.show_about = true;
                        ui.close();
                    }
                });
            });
        });

        // the Cubism toolbar strip: workspace switcher and edit level
        Panel::top("toolbar").show(ui, |ui| {
            ui.horizontal(|ui| {
                let label = self.workspace.name(&self.prefs);
                let mut pick: Option<Workspace> = None;
                egui::ComboBox::from_id_salt("workspace")
                    .selected_text(label)
                    .width(130.0)
                    .show_ui(ui, |ui| {
                        if ui
                            .selectable_label(self.workspace == Workspace::Modeling, "Modeling")
                            .clicked()
                        {
                            pick = Some(Workspace::Modeling);
                        }
                        if ui
                            .selectable_label(self.workspace == Workspace::Animation, "Animation")
                            .clicked()
                        {
                            pick = Some(Workspace::Animation);
                        }
                        for (i, (name, _)) in self.prefs.custom_workspaces.iter().enumerate() {
                            if ui
                                .selectable_label(self.workspace == Workspace::Custom(i), name)
                                .clicked()
                            {
                                pick = Some(Workspace::Custom(i));
                            }
                        }
                        ui.separator();
                        if ui.button("Save as new workspace…").clicked() {
                            self.show_ws_save = true;
                        }
                        if ui.button("Manage workspaces…").clicked() {
                            self.show_ws_manage = true;
                        }
                    });
                if let Some(w) = pick {
                    self.set_workspace(w);
                }
                ui.separator();
                ui.weak("Edit Level:");
                for lvl in 1..=3u8 {
                    if ui.selectable_label(self.edit_level == lvl, lvl.to_string()).clicked() {
                        self.edit_level = lvl;
                    }
                }
                ui.separator();
                let has_physics = self.session.as_ref().is_some_and(|s| s.puppet.has_physics());
                if ui
                    .add_enabled_ui(has_physics, |ui| {
                        ui.selectable_label(self.physics_on && has_physics, "Physics")
                            .on_hover_text("Tick the pendulum rig (physics3)")
                    })
                    .inner
                    .clicked()
                {
                    self.physics_on = !self.physics_on;
                    log::debug!("physics {}", if self.physics_on { "on" } else { "off" });
                    if self.physics_on
                        && let Some(s) = &mut self.session
                    {
                        s.puppet.reset_physics();
                    }
                }
                if ui
                    .selectable_label(self.follow_cursor, "Follow")
                    .on_hover_text("Drive the standard face parameters with the cursor")
                    .clicked()
                {
                    self.follow_cursor = !self.follow_cursor;
                    log::debug!("follow cursor {}", if self.follow_cursor { "on" } else { "off" });
                }
                ui.separator();
                if let Some(s) = &self.session {
                    ui.weak(&s.name);
                }
            });
        });

        // the floating tool bar; right-click any tool to dock it instead
        if self.session.is_some() && !self.prefs.toolbar_docked {
            let default_pos = self.prefs.toolbar_pos.unwrap_or([320.0, 92.0]);
            let area = egui::Area::new(egui::Id::new("tools-float"))
                .default_pos(default_pos)
                .movable(true)
                .show(ui.ctx(), |ui| {
                    egui::Frame::new()
                        .fill(theme::pal().panel_bg)
                        .stroke(Stroke::new(1.0, theme::pal().border))
                        .corner_radius(6.0)
                        .inner_margin(egui::Margin::symmetric(6, 4))
                        .show(ui, |ui| {
                            ui.spacing_mut().item_spacing = vec2(3.0, 0.0);
                            ui.horizontal(|ui| {
                                let (grip, _) =
                                    ui.allocate_exact_size(vec2(10.0, 26.0), Sense::hover());
                                for dy in [-5.0, 0.0, 5.0] {
                                    ui.painter().circle_filled(
                                        grip.center() + vec2(0.0, dy),
                                        1.2,
                                        theme::pal().text_faint,
                                    );
                                }
                                self.tool_strip(ui);
                            });
                        });
                });
            let at = area.response.rect.min;
            let settled = !ui.ctx().input(|i| i.pointer.any_down());
            let moved = self
                .prefs
                .toolbar_pos
                .is_none_or(|[x, y]| (x - at.x).abs() > 1.0 || (y - at.y).abs() > 1.0);
            if settled && moved && at.x.is_finite() && at.y.is_finite() {
                self.prefs.toolbar_pos = Some([at.x, at.y]);
                self.prefs.save();
            }
        }

        // cursor following and the pendulum rig both drive parameters, so
        // they run just ahead of the per-frame evaluate
        if let Some(s) = &mut self.session {
            let dt = ui.ctx().input(|i| i.stable_dt).clamp(0.0, 0.1);
            if self.follow_cursor {
                let rect = ui.ctx().content_rect();
                let target = ui
                    .ctx()
                    .input(|i| i.pointer.latest_pos())
                    .map(|p| {
                        vec2(
                            ((p.x - rect.center().x) / (rect.width() * 0.5)).clamp(-1.0, 1.0),
                            ((p.y - rect.center().y) / (rect.height() * 0.5)).clamp(-1.0, 1.0),
                        )
                    })
                    .unwrap_or(Vec2::ZERO);
                let ease = 1.0 - (-dt * 8.0).exp();
                self.follow = self.follow + (target - self.follow) * ease;
                let f = self.follow;
                // the published standard parameter set every tracker expects
                for (id, v) in [
                    ("ParamAngleX", f.x * 30.0),
                    ("ParamAngleY", -f.y * 30.0),
                    ("ParamBodyAngleX", f.x * 10.0),
                    ("ParamBodyAngleY", -f.y * 10.0),
                    ("ParamEyeBallX", f.x),
                    ("ParamEyeBallY", -f.y),
                ] {
                    if let Some(i) = s.puppet.model().parameters.iter().position(|p| p.id == id) {
                        s.puppet.set_value(i, v);
                        s.dirty = true;
                    }
                }
            }
            let animating = self.follow_cursor || (self.physics_on && s.puppet.has_physics());
            let physics_moved =
                self.physics_on && s.puppet.has_physics() && s.puppet.step_physics(dt);
            if physics_moved {
                s.dirty = true;
            }
            if animating {
                // a once-a-second trace heartbeat for the Log panel
                self.heartbeat += dt;
                if self.heartbeat >= 1.0 {
                    self.heartbeat = 0.0;
                    if self.follow_cursor {
                        log::trace!(
                            "follow: target ({:+.2}, {:+.2})",
                            self.follow.x,
                            self.follow.y
                        );
                    }
                    if self.physics_on && s.puppet.has_physics() {
                        log::trace!(
                            "physics: ticking {} groups, outputs {}",
                            s.puppet.model().physics.as_ref().map(|r| r.groups.len()).unwrap_or(0),
                            if physics_moved { "moving" } else { "settled" }
                        );
                    }
                }
                self.request_anim(&ui.ctx().clone());
            }
        }

        // one evaluation per frame, only when something moved
        if let Some(s) = &mut self.session
            && s.dirty
        {
            let t = std::time::Instant::now();
            s.puppet.update();
            s.last_update_ms = t.elapsed().as_secs_f32() * 1000.0;
            if s.last_update_ms > 5.0 {
                log::debug!("slow evaluate: {:.2} ms", s.last_update_ms);
            }
            s.dirty = false;
        }

        if self.session.is_none() && self.pending_import.is_none() {
            CentralPanel::default().frame(Frame::NONE.fill(theme::pal().chrome)).show(ui, |ui| {
                self.home(ui);
            });
        } else {
            CentralPanel::default().frame(Frame::NONE.fill(theme::pal().chrome)).show(ui, |ui| {
                let rect = ui.available_rect_before_wrap().shrink(dock::theme::GAP);
                let mut groups = Vec::new();
                dock::layout(ui, &mut self.layout, rect, "w", &mut groups);
                let mut actions = Vec::new();
                for g in &groups {
                    actions.extend(dock::draw_group_chrome(ui, g));
                    if let Some(p) = g.panels.get(g.active).copied() {
                        self.panel_contents(ui, p, g.content);
                    }
                }
                for a in actions {
                    match a {
                        DockAction::Activate(p) => {
                            self.layout.activate(p);
                        }
                        DockAction::Close(p) => self.layout.close(p),
                    }
                }
                dock::drag_overlay(ui, &mut self.layout, &groups);
            });
        }

        // persist layout edits once the mouse is up
        let mouse_up = !ui.ctx().input(|i| i.pointer.any_down());
        if mouse_up && self.layout != self.last_layout {
            self.save_layout();
        }

        if let Some((warp, origin)) = self.pending_create.take() {
            self.create_deformer(warp, origin);
        }

        self.about_window(ui);
        self.settings_window(ui);
        self.stats_window(ui);
        self.physics_window(ui);
        self.mesh_edit_window(ui);
        self.param_new_window(ui);
        self.folder_new_window(ui);
        self.model_new_window(ui);
        self.model_info_window(ui);
        self.close_confirm_window(ui);
        self.workspace_windows(ui);
        self.param_edit_window(ui);
    }

    /// Cubism's Edit Mesh dialog, shown while mesh edit mode is live: the
    /// manual modes, the auto mesher, and the way out.
    fn mesh_edit_window(&mut self, ui: &mut Ui) {
        let Some(mi) = self.mesh_edit else { return };
        let (verts, tris, id) =
            match self.session.as_ref().and_then(|s| s.puppet.model().art_meshes.get(mi)) {
                Some(am) => (am.vertex_count as usize, am.indices.len() / 3, am.id.clone()),
                None => return,
            };
        let mut done = false;
        let mut cancel = false;
        let mut auto = false;
        egui::Window::new("Edit Mesh")
            .collapsible(false)
            .resizable(false)
            .default_width(220.0)
            .show(ui.ctx(), |ui| {
                ui.label(truncate(&id, 28));
                ui.weak(format!("{verts} vertices · {tris} triangles"));
                ui.separator();
                ui.horizontal(|ui| {
                    for (mode, icon, tip) in [
                        (0u8, egui_phosphor::regular::CURSOR, "Move vertices"),
                        (1, egui_phosphor::regular::PLUS_CIRCLE, "Add vertices (click a face)"),
                        (2, egui_phosphor::regular::ERASER, "Erase vertices"),
                    ] {
                        if ui
                            .selectable_label(
                                self.mesh_mode == mode,
                                egui::RichText::new(icon).size(16.0),
                            )
                            .on_hover_text(tip)
                            .clicked()
                        {
                            self.mesh_mode = mode;
                        }
                    }
                });
                ui.separator();
                ui.label("Auto mesh");
                ui.horizontal(|ui| {
                    ui.weak("Spacing");
                    ui.add(
                        egui::DragValue::new(&mut self.tool_opts.auto_spacing)
                            .range(4.0..=256.0)
                            .suffix(" px"),
                    );
                });
                ui.horizontal(|ui| {
                    ui.weak("Margin");
                    ui.add(
                        egui::DragValue::new(&mut self.tool_opts.auto_margin)
                            .range(0.0..=128.0)
                            .suffix(" px"),
                    );
                });
                if ui.button("Generate from texture").clicked() {
                    auto = true;
                }
                ui.separator();
                ui.horizontal(|ui| {
                    if ui.button(egui::RichText::new("Done").color(Color32::WHITE)).clicked() {
                        done = true;
                    }
                    if ui.button("Cancel").clicked() {
                        cancel = true;
                    }
                });
                ui.weak("Esc finishes too; Ctrl+Z steps back");
            });
        if auto {
            self.auto_mesh(mi);
        }
        if cancel {
            if let (Some(s), Some(snap)) = (&mut self.session, self.mesh_snapshot.take()) {
                s.edit(|m| {
                    *m = snap;
                    true
                });
            }
            done = true;
        }
        if done {
            self.mesh_edit = None;
            self.mesh_drag = None;
            self.mesh_snapshot = None;
            self.mesh_mode = 0;
            if self.tool == Tool::MeshManual {
                self.tool = Tool::Arrow;
            }
        }
    }

    /// The physics dialog, Cubism's Physics tab squeezed into a compact
    /// column: FPS and group rows, Input/Output tabs, then the pendulum
    /// table beside the preview. Everything edits a working copy that
    /// Apply commits as one undoable change.
    fn physics_window(&mut self, ui: &mut Ui) {
        if !self.show_physics {
            if self.phys_edit.take().is_some() {
                // closing re-enables every group, like Cubism's Valid reset
                if let Some(s) = &mut self.session {
                    let n = s.puppet.model().physics.as_ref().map(|r| r.groups.len()).unwrap_or(0);
                    for gi in 0..n {
                        s.puppet.set_physics_group_enabled(gi, true);
                    }
                }
            }
            return;
        }
        let Some(session_rig) =
            self.session.as_ref().map(|s| s.puppet.model().physics.clone().unwrap_or_default())
        else {
            self.show_physics = false;
            return;
        };
        let param_ids: Vec<String> = self
            .session
            .as_ref()
            .map(|s| s.puppet.model().parameters.iter().map(|p| p.id.clone()).collect())
            .unwrap_or_default();
        let live_peaks: Vec<(usize, f32)> = if self.phys_auto_max {
            self.session
                .as_ref()
                .and_then(|s| s.puppet.physics_output_peaks(self.phys_sel))
                .unwrap_or_default()
        } else {
            self.phys_peak_cache.clone()
        };
        if self.phys_auto_max {
            self.phys_peak_cache = live_peaks.clone();
        }
        let group_enabled =
            self.session.as_ref().is_none_or(|s| s.puppet.physics_group_enabled(self.phys_sel));

        let mut rig = self.phys_edit.take().unwrap_or_else(|| session_rig.clone());
        let mut open = self.show_physics;
        let mut apply = false;
        let mut revert = false;
        let mut toggle_enable = false;
        let mut reset_peaks = false;
        egui::Window::new("Physics Settings")
            .open(&mut open)
            .collapsible(false)
            .resizable(true)
            .default_width(400.0)
            .show(ui.ctx(), |ui| {
                ui.spacing_mut().item_spacing = vec2(5.0, 4.0);
                // fps, wind and the group picker share the first rows
                ui.horizontal(|ui| {
                    ui.label("FPS:");
                    let mut fps = if rig.fps > 0.0 { rig.fps } else { 60.0 };
                    egui::ComboBox::from_id_salt("phys-fps")
                        .selected_text(format!("{fps:.0}"))
                        .width(52.0)
                        .show_ui(ui, |ui| {
                            for v in [15.0f32, 30.0, 60.0, 120.0] {
                                if ui.selectable_label(fps == v, format!("{v:.0}")).clicked() {
                                    fps = v;
                                }
                            }
                        });
                    rig.fps = fps;
                    ui.weak("Wind");
                    ui.add(
                        egui::DragValue::new(&mut rig.wind_x)
                            .speed(0.05)
                            .prefix("X ")
                            .update_while_editing(false),
                    );
                    ui.add(
                        egui::DragValue::new(&mut rig.wind_y)
                            .speed(0.05)
                            .prefix("Y ")
                            .update_while_editing(false),
                    );
                });
                ui.horizontal(|ui| {
                    let current = rig
                        .groups
                        .get(self.phys_sel)
                        .map(|g| if g.name.is_empty() { g.id.clone() } else { g.name.clone() })
                        .unwrap_or_else(|| "(none)".into());
                    egui::ComboBox::from_id_salt("phys-group")
                        .selected_text(truncate(&current, 16))
                        .width(130.0)
                        .show_ui(ui, |ui| {
                            for (gi, g) in rig.groups.iter().enumerate() {
                                let label = if g.name.is_empty() { &g.id } else { &g.name };
                                if ui
                                    .selectable_label(self.phys_sel == gi, truncate(label, 24))
                                    .clicked()
                                {
                                    self.phys_sel = gi;
                                }
                            }
                        });
                    let mut on = group_enabled;
                    if ui.checkbox(&mut on, "Enable").changed() {
                        toggle_enable = true;
                    }
                    if ui.button(egui_phosphor::regular::PLUS).on_hover_text("Add group").clicked()
                    {
                        let n = rig.groups.len() + 1;
                        rig.groups.push(default_physics_group(n));
                        self.phys_sel = rig.groups.len() - 1;
                        log::debug!("physics: group {n} added");
                    }
                    if ui
                        .button(egui_phosphor::regular::COPY)
                        .on_hover_text("Duplicate group")
                        .clicked()
                        && let Some(g) = rig.groups.get(self.phys_sel).cloned()
                    {
                        let mut g = g;
                        g.id = format!("{}Copy", g.id);
                        g.name = format!("{} copy", g.name);
                        rig.groups.push(g);
                        self.phys_sel = rig.groups.len() - 1;
                    }
                    if ui
                        .button(egui_phosphor::regular::TRASH)
                        .on_hover_text("Delete group")
                        .clicked()
                        && self.phys_sel < rig.groups.len()
                    {
                        rig.groups.remove(self.phys_sel);
                        self.phys_sel = self.phys_sel.min(rig.groups.len().saturating_sub(1));
                    }
                    if let Some(g) = rig.groups.get_mut(self.phys_sel) {
                        ui.add_sized(
                            vec2(ui.available_width().max(60.0), 18.0),
                            egui::TextEdit::singleline(&mut g.name),
                        );
                    }
                });
                ui.separator();

                ui.horizontal(|ui| {
                    for (i, label) in ["Input", "Output"].iter().enumerate() {
                        if ui.selectable_label(self.phys_tab == i, *label).clicked() {
                            self.phys_tab = i;
                        }
                    }
                });
                let reach = rig_groups_reach(&rig, self.phys_sel);
                let Some(g) = rig.groups.get_mut(self.phys_sel) else {
                    ui.weak("Add a group to begin.");
                    return;
                };
                let combo = |ui: &mut Ui, salt: String, current: &mut String| {
                    egui::ComboBox::from_id_salt(salt)
                        .selected_text(truncate(current, 14))
                        .width(116.0)
                        .show_ui(ui, |ui| {
                            for id in &param_ids {
                                ui.selectable_value(current, id.clone(), id);
                            }
                        });
                };
                match self.phys_tab {
                    0 => {
                        ui.horizontal(|ui| {
                            egui::ComboBox::from_id_salt("phys-in-preset")
                                .selected_text(
                                    ["Head input", "Torso input", "Chest input"]
                                        .get(self.phys_preset)
                                        .copied()
                                        .unwrap_or("Head input"),
                                )
                                .width(100.0)
                                .show_ui(ui, |ui| {
                                    for (i, n) in ["Head input", "Torso input", "Chest input"]
                                        .iter()
                                        .enumerate()
                                    {
                                        ui.selectable_value(&mut self.phys_preset, i, *n);
                                    }
                                });
                            if ui.button("Import").clicked() {
                                g.inputs = input_preset(self.phys_preset);
                                log::debug!("physics: input preset imported");
                            }
                            if ui.button("Add…").clicked() {
                                g.inputs.push(ayatsuri_format::model::PhysicsInput {
                                    param: param_ids.first().cloned().unwrap_or_default(),
                                    weight: 100.0,
                                    kind: ayatsuri_format::model::PhysicsIo::X,
                                    reflect: false,
                                });
                            }
                        });
                        let mut kill = None;
                        egui::Grid::new("phys-in").num_columns(5).spacing([5.0, 2.0]).show(
                            ui,
                            |ui| {
                                ui.weak("Input");
                                ui.weak("Type");
                                ui.weak("%").on_hover_text("Effectiveness");
                                ui.weak("Flip");
                                ui.weak("");
                                ui.end_row();
                                for (i, inp) in g.inputs.iter_mut().enumerate() {
                                    combo(ui, format!("pi{i}"), &mut inp.param);
                                    egui::ComboBox::from_id_salt(format!("pit{i}"))
                                        .selected_text(match inp.kind {
                                            ayatsuri_format::model::PhysicsIo::Angle => "Angle",
                                            _ => "Pos X",
                                        })
                                        .width(70.0)
                                        .show_ui(ui, |ui| {
                                            ui.selectable_value(
                                                &mut inp.kind,
                                                ayatsuri_format::model::PhysicsIo::X,
                                                "Position X",
                                            );
                                            ui.selectable_value(
                                                &mut inp.kind,
                                                ayatsuri_format::model::PhysicsIo::Angle,
                                                "Angle",
                                            );
                                        });
                                    ui.add(
                                        egui::DragValue::new(&mut inp.weight).range(0.0..=100.0),
                                    );
                                    ui.checkbox(&mut inp.reflect, "");
                                    if ui
                                        .button(egui_phosphor::regular::X)
                                        .on_hover_text("Delete")
                                        .clicked()
                                    {
                                        kill = Some(i);
                                    }
                                    ui.end_row();
                                }
                            },
                        );
                        if let Some(i) = kill {
                            g.inputs.remove(i);
                        }
                        ui.add_space(2.0);
                        ui.weak("Normalization  ·  min / center / max");
                        egui::Grid::new("phys-norm").num_columns(4).spacing([5.0, 2.0]).show(
                            ui,
                            |ui| {
                                ui.weak("Angle");
                                ui.add(egui::DragValue::new(&mut g.norm_angle.min).speed(0.5));
                                ui.add(egui::DragValue::new(&mut g.norm_angle.default).speed(0.5));
                                ui.add(egui::DragValue::new(&mut g.norm_angle.max).speed(0.5));
                                ui.end_row();
                                ui.weak("Pos X");
                                ui.add(egui::DragValue::new(&mut g.norm_position.min).speed(0.5));
                                ui.add(
                                    egui::DragValue::new(&mut g.norm_position.default).speed(0.5),
                                );
                                ui.add(egui::DragValue::new(&mut g.norm_position.max).speed(0.5));
                                ui.end_row();
                            },
                        );
                    }
                    _ => {
                        ui.horizontal(|ui| {
                            if ui.button("Add…").clicked() {
                                g.outputs.push(ayatsuri_format::model::PhysicsOutput {
                                    param: param_ids.first().cloned().unwrap_or_default(),
                                    vertex: 1,
                                    scale: 1.0,
                                    weight: 100.0,
                                    kind: ayatsuri_format::model::PhysicsIo::Angle,
                                    reflect: false,
                                });
                            }
                            ui.checkbox(&mut self.phys_auto_max, "Auto max")
                                .on_hover_text("Auto-update maximum of output");
                        });
                        let mut kill = None;
                        egui::Grid::new("phys-out").num_columns(7).spacing([5.0, 2.0]).show(
                            ui,
                            |ui| {
                                ui.weak("No").on_hover_text("Pendulum No");
                                ui.weak("Output");
                                ui.weak("%").on_hover_text("Effectiveness");
                                ui.weak("Flip");
                                ui.weak("Scale");
                                ui.weak("Max%").on_hover_text("Output Maximum (%)");
                                ui.weak("");
                                ui.end_row();
                                for (i, out) in g.outputs.iter_mut().enumerate() {
                                    ui.add(egui::DragValue::new(&mut out.vertex).range(1..=99));
                                    combo(ui, format!("po{i}"), &mut out.param);
                                    ui.add(
                                        egui::DragValue::new(&mut out.weight).range(0.0..=100.0),
                                    );
                                    ui.checkbox(&mut out.reflect, "");
                                    ui.add(egui::DragValue::new(&mut out.scale).speed(0.05));
                                    let peak = live_peaks
                                        .iter()
                                        .find(|(oi, _)| *oi == i)
                                        .map(|(_, p)| *p)
                                        .unwrap_or(0.0);
                                    ui.label(format!("{peak:.0}"));
                                    if ui
                                        .button(egui_phosphor::regular::X)
                                        .on_hover_text("Delete")
                                        .clicked()
                                    {
                                        kill = Some(i);
                                    }
                                    ui.end_row();
                                }
                            },
                        );
                        if let Some(i) = kill {
                            g.outputs.remove(i);
                        }
                        ui.horizontal(|ui| {
                            let adjust = |outs: &mut Vec<ayatsuri_format::model::PhysicsOutput>,
                                          peaks: &[(usize, f32)],
                                          up: bool| {
                                for (i, o) in outs.iter_mut().enumerate() {
                                    let Some(&(_, peak)) = peaks.iter().find(|(oi, _)| *oi == i)
                                    else {
                                        continue;
                                    };
                                    if peak > 1e-3
                                        && ((up && peak < 100.0) || (!up && peak > 100.0))
                                    {
                                        o.scale *= 100.0 / peak;
                                    }
                                }
                            };
                            if ui
                                .small_button("Output ↑")
                                .on_hover_text("Increase Output")
                                .clicked()
                            {
                                adjust(&mut g.outputs, &live_peaks, true);
                                reset_peaks = true;
                            }
                            if ui
                                .small_button("Output ↓")
                                .on_hover_text("Decrease Output")
                                .clicked()
                            {
                                adjust(&mut g.outputs, &live_peaks, false);
                                reset_peaks = true;
                            }
                            if ui
                                .small_button("Scale 1.0")
                                .on_hover_text("Reset Magnification")
                                .clicked()
                            {
                                for o in &mut g.outputs {
                                    o.scale = 1.0;
                                }
                            }
                            if ui
                                .small_button("Clear max")
                                .on_hover_text("Reset Output Maximum")
                                .clicked()
                            {
                                reset_peaks = true;
                            }
                        });
                        ui.weak("Move the model (Follow helps) to collect Max%, then adjust.");
                    }
                }
                ui.separator();

                // the pendulum table beside the live preview
                ui.horizontal_top(|ui| {
                    ui.vertical(|ui| {
                        ui.horizontal(|ui| {
                            ui.label("Pendulums");
                            if ui.small_button(egui_phosphor::regular::PLUS).clicked() {
                                let last = g.nodes.last().cloned().unwrap_or(
                                    ayatsuri_format::model::PhysicsNode {
                                        x: 0.0,
                                        y: 10.0,
                                        mobility: 0.95,
                                        delay: 1.0,
                                        acceleration: 1.0,
                                        radius: 10.0,
                                    },
                                );
                                g.nodes.push(last);
                            }
                            let can_del = g.nodes.len() > 2;
                            if ui
                                .add_enabled(
                                    can_del,
                                    egui::Button::new(egui_phosphor::regular::MINUS).small(),
                                )
                                .clicked()
                            {
                                g.nodes.pop();
                                for o in &mut g.outputs {
                                    if o.vertex as usize >= g.nodes.len() {
                                        o.vertex = g.nodes.len().saturating_sub(1) as u32;
                                    }
                                }
                            }
                        });
                        egui::Grid::new("phys-nodes").num_columns(5).spacing([5.0, 2.0]).show(
                            ui,
                            |ui| {
                                ui.weak("No");
                                ui.weak("Len").on_hover_text("Duration (stage length)");
                                ui.weak("Sway").on_hover_text("Shaking influence");
                                ui.weak("React").on_hover_text("Reaction time");
                                ui.weak("Accel").on_hover_text("Overall acceleration");
                                ui.end_row();
                                for (i, n) in g.nodes.iter_mut().enumerate().skip(1) {
                                    ui.label(i.to_string());
                                    let mut len = (n.x * n.x + n.y * n.y).sqrt();
                                    let before = len;
                                    if ui
                                        .add(
                                            egui::DragValue::new(&mut len)
                                                .range(0.1..=200.0)
                                                .speed(0.1),
                                        )
                                        .changed()
                                    {
                                        if before > 1e-6 {
                                            let f = len / before;
                                            n.x *= f;
                                            n.y *= f;
                                        } else {
                                            n.y = len;
                                        }
                                        n.radius = len;
                                    }
                                    ui.add(
                                        egui::DragValue::new(&mut n.mobility)
                                            .range(0.0..=1.0)
                                            .speed(0.005),
                                    );
                                    ui.add(
                                        egui::DragValue::new(&mut n.delay)
                                            .range(0.0..=10.0)
                                            .speed(0.01),
                                    );
                                    ui.add(
                                        egui::DragValue::new(&mut n.acceleration)
                                            .range(0.0..=10.0)
                                            .speed(0.01),
                                    );
                                    ui.end_row();
                                }
                            },
                        );
                    });
                    let nodes_copy = g.nodes.clone();
                    ui.vertical(|ui| {
                        self.pendulum_preview(ui, reach, &nodes_copy);
                    });
                });
                ui.separator();
                ui.horizontal(|ui| {
                    if ui.button(egui::RichText::new("Apply").strong()).clicked() {
                        apply = true;
                    }
                    if ui.button("Revert").clicked() {
                        revert = true;
                    }
                });
            });
        if toggle_enable && let Some(s) = &mut self.session {
            let now = !group_enabled;
            s.puppet.set_physics_group_enabled(self.phys_sel, now);
            s.dirty = true;
            log::debug!(
                "physics: group {} {}",
                self.phys_sel,
                if now { "enabled" } else { "muted" }
            );
        }
        if reset_peaks && let Some(s) = &mut self.session {
            s.puppet.reset_physics_output_peaks();
            self.phys_peak_cache.clear();
        }
        if apply && let Some(s) = &mut self.session {
            let keep = if rig.groups.is_empty() { None } else { Some(rig.clone()) };
            if s.edit(|m| {
                m.physics = keep.clone();
                true
            }) {
                log::info!("physics settings applied");
            }
        }
        if revert {
            log::debug!("physics: working copy reverted");
        } else {
            self.phys_edit = Some(rig);
        }
        self.show_physics = open;
        if !self.show_physics {
            self.phys_edit = None;
            if let Some(s) = &mut self.session {
                let n = s.puppet.model().physics.as_ref().map(|r| r.groups.len()).unwrap_or(0);
                for gi in 0..n {
                    s.puppet.set_physics_group_enabled(gi, true);
                }
            }
        }
    }

    /// The pendulum preview pane: its own tiny simulation of the working
    /// copy's chain, the way Cubism previews the physical model without the
    /// input and output wiring. Drag sideways to swing it.
    fn pendulum_preview(
        &mut self,
        ui: &mut Ui,
        reach: f32,
        nodes: &[ayatsuri_format::model::PhysicsNode],
    ) {
        let (prect, presp) = ui.allocate_exact_size(vec2(126.0, 132.0), Sense::click_and_drag());
        let pp = ui.painter_at(prect);
        pp.rect_filled(prect, 4.0, theme::pal().canvas_bg);
        pp.rect_stroke(prect, 4.0, Stroke::new(1.0, theme::pal().border), StrokeKind::Inside);
        for f in [0.25f32, 0.5, 0.75] {
            let x = prect.min.x + prect.width() * f;
            let y = prect.min.y + prect.height() * f;
            let grid = Stroke::new(
                if f == 0.5 { 1.0 } else { 0.5 },
                theme::pal().border.gamma_multiply(if f == 0.5 { 1.0 } else { 0.5 }),
            );
            pp.line_segment([pos2(x, prect.min.y), pos2(x, prect.max.y)], grid);
            pp.line_segment([pos2(prect.min.x, y), pos2(prect.max.x, y)], grid);
        }
        if presp.hovered() {
            let scroll = ui.ctx().input(|i| i.smooth_scroll_delta.y);
            if scroll != 0.0 {
                self.phys_zoom = (self.phys_zoom * (scroll * 0.002).exp()).clamp(0.2, 8.0);
            }
        }

        // rebuild the private sim whenever the settings change shape
        let mut sig = 0u64;
        for n in nodes {
            for v in [n.x, n.y, n.mobility, n.delay, n.acceleration] {
                sig = sig.wrapping_mul(31).wrapping_add(v.to_bits() as u64);
            }
        }
        if sig != self.phys_prev_sig || self.phys_prev.len() != nodes.len() {
            self.phys_prev_sig = sig;
            self.phys_prev.clear();
            let (mut ax, mut ay) = (0.0f32, 0.0f32);
            for (i, n) in nodes.iter().enumerate() {
                if i > 0 {
                    ax += n.x;
                    ay += n.y;
                }
                self.phys_prev.push((ax, ay, ax, ay));
            }
            self.phys_prev_drive = 0.0;
        }

        // dragging sideways pushes the anchor; it eases home on release
        let sc_units = (prect.height() * 0.44) / reach.max(1.0) * self.phys_zoom;
        if presp.dragged() {
            self.phys_prev_drive += presp.drag_delta().x / sc_units.max(1e-3);
        } else {
            self.phys_prev_drive *= 0.9;
        }

        // the same integrator the runtime uses, one step per frame
        let dt = ui.ctx().input(|i| i.stable_dt).clamp(0.001, 0.05);
        if let Some(anchor) = self.phys_prev.get_mut(0) {
            anchor.2 = anchor.0;
            anchor.3 = anchor.1;
            anchor.0 = self.phys_prev_drive;
            anchor.1 = 0.0;
        }
        for i in 1..self.phys_prev.len() {
            let (px, py) = {
                let p = self.phys_prev.get(i - 1).copied().unwrap_or_default();
                (p.0, p.1)
            };
            let (Some(n), Some(slot)) = (nodes.get(i), self.phys_prev.get_mut(i)) else {
                break;
            };
            let len = (n.x * n.x + n.y * n.y).sqrt().max(1e-3);
            let (hx, hy) = (n.x / len, n.y / len);
            let vel_x = (slot.0 - slot.2) * n.mobility;
            let vel_y = (slot.1 - slot.3) * n.mobility;
            let dtq = dt * n.delay.clamp(0.01, 10.0);
            let grav = 1500.0 * n.acceleration;
            slot.2 = slot.0;
            slot.3 = slot.1;
            let mut nx = slot.0 + vel_x + hx * grav * dtq * dtq;
            let mut ny = slot.1 + vel_y + hy * grav * dtq * dtq;
            let dx = nx - px;
            let dy = ny - py;
            let dist = (dx * dx + dy * dy).sqrt();
            if dist > 1e-6 {
                nx = px + dx / dist * len;
                ny = py + dy / dist * len;
            }
            slot.0 = nx;
            slot.1 = ny;
        }
        ui.ctx().request_repaint();

        let cx = prect.center().x;
        let cy = prect.min.y + prect.height() * 0.28;
        let map = |x: f32, y: f32| pos2(cx + x * sc_units, cy + y * sc_units);
        let stroke = Stroke::new(1.6, theme::pal().rotation_red);
        let mut prev: Option<Pos2> = None;
        for (i, &(x, y, ..)) in self.phys_prev.iter().enumerate() {
            let sp = map(x, y);
            if let Some(pr) = prev {
                pp.line_segment([pr, sp], stroke);
            }
            if i == 0 {
                pp.rect_filled(Rect::from_center_size(sp, vec2(6.0, 6.0)), 1.0, theme::pal().text);
            } else {
                pp.circle_filled(sp, 4.0, theme::pal().rotation_red);
                pp.circle_stroke(sp, 4.0, Stroke::new(1.0, theme::pal().canvas_bg));
                pp.text(
                    sp + vec2(8.0, 0.0),
                    Align2::LEFT_CENTER,
                    i.to_string(),
                    FontId::proportional(9.0),
                    theme::pal().text_faint,
                );
            }
            prev = Some(sp);
        }
        pp.text(
            pos2(prect.min.x + 6.0, prect.min.y + 8.0),
            Align2::LEFT_CENTER,
            format!("Scale: {:.1}", self.phys_zoom),
            FontId::proportional(9.0),
            theme::pal().text_faint,
        );
        ui.horizontal(|ui| {
            if ui.small_button("1:1").clicked() {
                self.phys_zoom = 1.0;
            }
            if ui.small_button("Swing").clicked() {
                self.phys_prev_drive = reach * 0.4;
            }
        });
    }

    /// Cubism's Model Statistics dialog: object and geometry counts.
    fn stats_window(&mut self, ui: &mut Ui) {
        if !self.show_stats {
            return;
        }
        let mut open = self.show_stats;
        egui::Window::new("Model Statistics")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .show(ui.ctx(), |ui| {
                let Some(s) = &self.session else {
                    ui.weak("No model loaded.");
                    return;
                };
                let m = s.puppet.model();
                let (mut verts, mut tris, mut keyforms) = (0usize, 0usize, 0usize);
                for am in &m.art_meshes {
                    verts += am.vertex_count as usize;
                    tris += am.indices.len() / 3;
                    let stride = (am.vertex_count as usize).saturating_mul(2);
                    keyforms += am.positions.len().checked_div(stride).unwrap_or(0);
                }
                let warps =
                    m.deformers.iter().filter(|d| matches!(d.kind, DeformerKind::Warp(_))).count();
                ui.set_width(260.0);
                egui::Grid::new("stats").num_columns(2).spacing([24.0, 4.0]).show(ui, |ui| {
                    for (label, value) in [
                        ("Canvas", format!("{} × {} px", m.canvas.width, m.canvas.height)),
                        ("Parameters", m.parameters.len().to_string()),
                        ("Parts", m.parts.len().to_string()),
                        ("Deformers", m.deformers.len().to_string()),
                        ("  Warp", warps.to_string()),
                        ("  Rotation", (m.deformers.len() - warps).to_string()),
                        ("ArtMeshes", m.art_meshes.len().to_string()),
                        ("Vertices", verts.to_string()),
                        ("Triangles", tris.to_string()),
                        ("Keyforms", keyforms.to_string()),
                        ("Textures", s.texture_names.len().to_string()),
                    ] {
                        ui.weak(label);
                        ui.label(value);
                        ui.end_row();
                    }
                });
            });
        self.show_stats = open;
    }

    fn workspace_windows(&mut self, ui: &mut Ui) {
        let mut open = self.show_ws_save;
        let mut saved = false;
        egui::Window::new("Save Workspace")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .show(ui.ctx(), |ui| {
                ui.label("Name");
                ui.text_edit_singleline(&mut self.ws_name);
                if ui.button("Save").clicked() && !self.ws_name.trim().is_empty() {
                    let name = self.ws_name.trim().to_string();
                    self.prefs.custom_workspaces.push((name, self.layout.clone()));
                    let idx = self.prefs.custom_workspaces.len() - 1;
                    self.workspace = Workspace::Custom(idx);
                    self.prefs.save();
                    self.ws_name.clear();
                    saved = true;
                }
            });
        self.show_ws_save = open && !saved;

        let mut open = self.show_ws_manage;
        egui::Window::new("Workspaces").open(&mut open).collapsible(false).resizable(false).show(
            ui.ctx(),
            |ui| {
                let mut delete: Option<usize> = None;
                if self.prefs.custom_workspaces.is_empty() {
                    ui.weak("No custom workspaces yet.");
                }
                for (i, (name, _)) in self.prefs.custom_workspaces.iter().enumerate() {
                    ui.horizontal(|ui| {
                        ui.label(name);
                        if ui.small_button("Delete").clicked() {
                            delete = Some(i);
                        }
                    });
                }
                if let Some(i) = delete {
                    self.prefs.custom_workspaces.remove(i);
                    if self.workspace == Workspace::Custom(i) {
                        self.workspace = Workspace::Modeling;
                        self.layout = self
                            .prefs
                            .workspace_modeling
                            .clone()
                            .unwrap_or_else(dock::default_workspace);
                    }
                    self.prefs.save();
                }
            },
        );
        self.show_ws_manage = open;
    }

    fn home(&mut self, ui: &mut Ui) {
        let rect = ui.available_rect_before_wrap();
        ui.painter().rect_filled(rect, 0.0, theme::pal().chrome);
        // a Photoshop-style home: brand band, actions, recents with previews
        let band = Rect::from_min_size(rect.min, vec2(rect.width(), 150.0));
        ui.painter().rect_filled(band, 0.0, theme::pal().header);
        // a quiet accent wash fading out of the left edge, Adobe style
        let accent = theme::pal().accent;
        for i in 0..24 {
            let t = i as f32 / 24.0;
            let w = band.width() * 0.45;
            ui.painter().rect_filled(
                Rect::from_min_size(
                    pos2(band.min.x + w * t, band.min.y),
                    vec2(w / 24.0 + 1.0, band.height()),
                ),
                0.0,
                accent.gamma_multiply(0.05 * (1.0 - t)),
            );
        }
        ui.painter().line_segment(
            [band.left_bottom(), band.right_bottom()],
            Stroke::new(1.0, theme::pal().border),
        );
        match self.logo_texture(&ui.ctx().clone()) {
            Some(id) => {
                // the full wordmark, scaled to sit comfortably in the band
                let h = 64.0;
                let aspect = self
                    .logo
                    .as_ref()
                    .map(|c| {
                        let sz = c.handle.size_vec2();
                        if sz.y > 0.0 { sz.x / sz.y } else { 4.0 }
                    })
                    .unwrap_or(4.0);
                let draw = Rect::from_min_size(
                    pos2(band.min.x + 36.0, band.center().y - h * 0.5 - 10.0),
                    vec2(h * aspect, h),
                );
                ui.painter().image(
                    id,
                    draw,
                    Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                    Color32::WHITE,
                );
            }
            None => {
                ui.painter().text(
                    pos2(band.min.x + 36.0, band.center().y - 10.0),
                    Align2::LEFT_CENTER,
                    "操り2D",
                    FontId::proportional(40.0),
                    theme::pal().accent,
                );
            }
        }
        ui.painter().text(
            pos2(band.min.x + 38.0, band.max.y - 24.0),
            Align2::LEFT_CENTER,
            format!("Ayatsuri2D {} · open 2D puppet studio", env!("CARGO_PKG_VERSION")),
            FontId::proportional(12.5),
            theme::pal().text_faint,
        );

        let body = Rect::from_min_max(pos2(rect.min.x + 36.0, band.max.y + 18.0), rect.max);
        let mut ui = ui.new_child(UiBuilder::new().max_rect(body));
        ui.horizontal(|ui| {
            if ui.add_sized([150.0, 34.0], egui::Button::new("Open…")).clicked() {
                self.import_dialog(&ui.ctx().clone());
            }
            if ui.add_sized([150.0, 34.0], egui::Button::new("New model…")).clicked() {
                self.new_model = Some(ModelNew::default());
            }
            ui.add_space(10.0);
            ui.weak("or drop a .aom / .moc3 / .model3.json anywhere");
        });
        ui.add_space(16.0);

        if self.prefs.recents.is_empty() {
            ui.weak("Models you open appear here.");
            return;
        }
        ui.label(egui::RichText::new("Recent").size(13.0).strong());
        ui.add_space(8.0);
        let recents: Vec<PathBuf> = self.prefs.recents.iter().take(8).cloned().collect();
        let mut open_this: Option<PathBuf> = None;
        ScrollArea::vertical().id_salt("home-recents").show(&mut ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                for r in &recents {
                    let card_w = 150.0;
                    let card_h = 230.0;
                    let (card, resp) = ui.allocate_exact_size(vec2(card_w, card_h), Sense::click());
                    let hovered = resp.hovered();
                    ui.painter().rect_filled(
                        card,
                        6.0,
                        if hovered { theme::pal().select } else { theme::pal().panel_bg },
                    );
                    ui.painter().rect_stroke(
                        card,
                        6.0,
                        Stroke::new(
                            1.0,
                            if hovered { theme::pal().accent } else { theme::pal().border },
                        ),
                        StrokeKind::Inside,
                    );
                    let thumb_rect = Rect::from_min_size(
                        card.min + vec2(7.0, 7.0),
                        vec2(card_w - 14.0, card_h - 44.0),
                    );
                    match self.home_thumb(&ui.ctx().clone(), r) {
                        Some((id, size)) => {
                            // fit, keep aspect
                            let s = (thumb_rect.width() / size.x).min(thumb_rect.height() / size.y);
                            let draw = Rect::from_center_size(
                                thumb_rect.center(),
                                vec2(size.x * s, size.y * s),
                            );
                            ui.painter().image(
                                id,
                                draw,
                                Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                                Color32::WHITE,
                            );
                        }
                        None => {
                            ui.painter().text(
                                thumb_rect.center(),
                                Align2::CENTER_CENTER,
                                "操",
                                FontId::proportional(42.0),
                                theme::pal().text_faint,
                            );
                        }
                    }
                    let name =
                        r.file_stem().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                    ui.painter().text(
                        pos2(card.center().x, card.max.y - 20.0),
                        Align2::CENTER_CENTER,
                        truncate(&name, 16),
                        FontId::proportional(12.0),
                        theme::pal().text,
                    );
                    if resp.clicked() {
                        open_this = Some(r.clone());
                    }
                }
            });
        });
        if let Some(path) = open_this {
            self.import(&ui.ctx().clone(), path);
        }
    }

    /// Cached home-screen preview for a recent file, loaded lazily.
    fn home_thumb(
        &mut self,
        ctx: &egui::Context,
        source: &PathBuf,
    ) -> Option<(egui::TextureId, Vec2)> {
        if let Some(entry) = self.home_thumbs.get(source) {
            return entry.as_ref().map(|t| (t.id(), t.size_vec2()));
        }
        let loaded = crate::thumb::cache_path(source)
            .filter(|p| p.exists())
            .and_then(|p| std::fs::read(p).ok())
            .and_then(|b| image::load_from_memory(&b).ok())
            .map(|img| {
                let img = img.into_rgba8();
                let size = [img.width() as usize, img.height() as usize];
                let color = egui::ColorImage::from_rgba_unmultiplied(size, img.as_raw());
                ctx.load_texture("home-thumb", color, egui::TextureOptions::LINEAR)
            });
        let out = loaded.as_ref().map(|t| (t.id(), t.size_vec2()));
        self.home_thumbs.insert(source.clone(), loaded);
        out
    }
    fn logo_texture(&mut self, ctx: &egui::Context) -> Option<egui::TextureId> {
        let want_dark = theme::is_dark();
        if let Some(c) = &self.logo
            && c.dark == want_dark
        {
            return Some(c.handle.id());
        }
        let bytes: &[u8] = if want_dark {
            include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/logo-dark.png"))
        } else {
            include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/logo-light.png"))
        };
        let img = image::load_from_memory(bytes).ok()?.into_rgba8();
        let size = [img.width() as usize, img.height() as usize];
        let color = egui::ColorImage::from_rgba_unmultiplied(size, img.as_raw());
        let handle = ctx.load_texture("logo", color, egui::TextureOptions::LINEAR);
        let id = handle.id();
        self.logo = Some(TextureHandleCache { dark: want_dark, handle });
        Some(id)
    }

    fn about_window(&mut self, ui: &mut Ui) {
        if !self.show_about {
            return;
        }
        let logo = self.logo_texture(&ui.ctx().clone());
        let logo_size = self.logo.as_ref().map(|c| c.handle.size_vec2()).unwrap_or(vec2(1.0, 1.0));
        let mut open = self.show_about;
        egui::Window::new("About Ayatsuri2D")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .default_pos(ui.ctx().content_rect().center() - vec2(170.0, 150.0))
            .show(ui.ctx(), |ui| {
                ui.set_width(330.0);
                ui.vertical_centered(|ui| {
                    ui.add_space(10.0);
                    if let Some(id) = logo {
                        let w = 230.0f32.min(logo_size.x);
                        ui.image((id, vec2(w, w * logo_size.y / logo_size.x.max(1.0))));
                    } else {
                        ui.label(
                            egui::RichText::new("操り2D").color(theme::pal().accent).size(34.0),
                        );
                    }
                    ui.add_space(4.0);
                    ui.label(
                        egui::RichText::new(format!("version {}", env!("CARGO_PKG_VERSION")))
                            .color(theme::pal().text_faint)
                            .size(11.5),
                    );
                    ui.add_space(10.0);
                    ui.separator();
                    ui.add_space(6.0);
                    ui.label("An open, native 2D puppet runtime and editor.");
                    ui.label("Imports and round-trips moc3; .aom is the native format.");
                    ui.add_space(6.0);
                    ui.label(
                        egui::RichText::new(
                            "Format research by the Alterion project, cross-checked against \
                             PurismCore and OpenL2D.",
                        )
                        .size(11.0)
                        .color(theme::pal().text_faint),
                    );
                    ui.label(
                        egui::RichText::new("GPL-3.0 · openmoc3 MIT · C ABI Apache-2.0")
                            .size(11.0)
                            .color(theme::pal().text_faint),
                    );
                    ui.add_space(10.0);
                });
            });
        self.show_about = open;
    }

    fn settings_window(&mut self, ui: &mut Ui) {
        let mut open = self.show_settings;
        egui::Window::new("Preferences").open(&mut open).collapsible(false).resizable(false).show(
            ui.ctx(),
            |ui| {
                ui.set_width(640.0);
                ui.set_height(380.0);
                ui.horizontal_top(|ui| {
                    // the Adobe-style category rail
                    ui.vertical(|ui| {
                        ui.set_width(150.0);
                        for (i, name) in
                            ["General", "Appearance", "View", "Performance", "Keyboard"]
                                .iter()
                                .enumerate()
                        {
                            let on = self.settings_tab == i;
                            let (rect, resp) =
                                ui.allocate_exact_size(vec2(146.0, 24.0), Sense::click());
                            if on {
                                ui.painter().rect_filled(rect, 3.0, theme::pal().select);
                                ui.painter().rect_filled(
                                    Rect::from_min_size(rect.min, vec2(2.5, 24.0)),
                                    1.0,
                                    theme::pal().accent,
                                );
                            } else if resp.hovered() {
                                ui.painter().rect_filled(rect, 3.0, theme::pal().header);
                            }
                            ui.painter().text(
                                rect.left_center() + vec2(10.0, 0.0),
                                Align2::LEFT_CENTER,
                                *name,
                                FontId::proportional(12.0),
                                theme::pal().text,
                            );
                            if resp.clicked() {
                                self.settings_tab = i;
                            }
                        }
                    });
                    ui.separator();
                    ui.vertical(|ui| {
                        ui.set_width(450.0);
                        match self.settings_tab {
                            0 => self.settings_general(ui),
                            1 => self.settings_appearance(ui),
                            2 => self.settings_view(ui),
                            3 => self.settings_performance(ui),
                            _ => self.settings_keyboard(ui),
                        }
                    });
                });
            },
        );
        self.show_settings = open;
    }

    fn settings_general(&mut self, ui: &mut Ui) {
        ui.heading("General");
        ui.add_space(6.0);
        let mut reopen = self.prefs.reopen_last;
        if ui.checkbox(&mut reopen, "Reopen the last model on launch").changed() {
            self.prefs.reopen_last = reopen;
            self.prefs.save();
        }
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.label(format!("{} recent files", self.prefs.recents.len()));
            if ui.small_button("Clear").clicked() {
                self.prefs.recents.clear();
                self.prefs.save();
            }
        });
        ui.separator();
        ui.heading("Logging");
        ui.add_space(4.0);
        let current = self.prefs.log_level.clone().unwrap_or_else(|| "info".into());
        ui.horizontal(|ui| {
            ui.label("Level");
            for name in ["error", "warn", "info", "debug", "trace"] {
                if ui.selectable_label(current == name, name).clicked() {
                    self.prefs.log_level = Some(name.to_string());
                    self.prefs.save();
                    logging::set_level(logging::level_from_str(name));
                }
            }
        });
        if let Some(p) = logging::log_path() {
            ui.weak(format!("Logs save to {}", p.display()));
        }
    }

    fn settings_appearance(&mut self, ui: &mut Ui) {
        ui.heading("Appearance");
        ui.add_space(6.0);
        let dark = theme::is_dark();
        ui.horizontal(|ui| {
            ui.label("Theme");
            if ui.selectable_label(!dark, "Light").clicked() && dark {
                theme::set_dark(&ui.ctx().clone(), false);
                self.prefs.dark = Some(false);
                self.prefs.save();
            }
            if ui.selectable_label(dark, "Dark").clicked() && !dark {
                theme::set_dark(&ui.ctx().clone(), true);
                self.prefs.dark = Some(true);
                self.prefs.save();
            }
        });
        ui.horizontal(|ui| {
            ui.label("Primary color");
            let c = theme::accent();
            let mut rgb = [c.r(), c.g(), c.b()];
            if ui.color_edit_button_srgb(&mut rgb).changed() {
                theme::set_accent(
                    &ui.ctx().clone(),
                    Some(Color32::from_rgb(rgb[0], rgb[1], rgb[2])),
                );
                self.prefs.accent = Some(rgb);
                self.prefs.save();
            }
            if ui.small_button("Reset").clicked() {
                theme::set_accent(&ui.ctx().clone(), None);
                self.prefs.accent = None;
                self.prefs.save();
            }
        });
    }

    fn settings_view(&mut self, ui: &mut Ui) {
        ui.heading("View");
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.label("Canvas background");
            let cur = self.prefs.view_bg.unwrap_or({
                let c = theme::pal().canvas_bg;
                [c.r(), c.g(), c.b()]
            });
            let mut rgb = cur;
            if ui.color_edit_button_srgb(&mut rgb).changed() {
                self.prefs.view_bg = Some(rgb);
                self.prefs.save();
            }
            if ui.small_button("Reset").clicked() {
                self.prefs.view_bg = None;
                self.prefs.save();
            }
        });
        ui.add_space(6.0);
        ui.weak("Anti-aliasing: 4x MSAA on the model, mipmapped textures.");
    }

    fn settings_performance(&mut self, ui: &mut Ui) {
        ui.heading("Performance");
        ui.add_space(6.0);
        if let Some(s) = &self.session {
            let verts: usize = s.puppet.drawables().map(|d| d.positions.len() / 2).sum();
            egui::Grid::new("perf").num_columns(2).spacing([24.0, 2.0]).show(ui, |ui| {
                ui.weak("Last evaluate");
                ui.weak(format!("{:.2} ms", s.last_update_ms));
                ui.end_row();
                ui.weak("Visible meshes");
                ui.weak(format!("{}", s.puppet.drawables().count()));
                ui.end_row();
                ui.weak("Vertices");
                ui.weak(format!("{verts}"));
                ui.end_row();
            });
        } else {
            ui.weak("No model loaded.");
        }
        ui.add_space(8.0);
        ui.separator();
        ui.heading("Limits");
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.label("Animation FPS cap");
            for (label, v) in [("Off", 0u32), ("30", 30), ("60", 60), ("120", 120)] {
                if ui.selectable_label(self.prefs.fps_cap == v, label).clicked() {
                    self.prefs.fps_cap = v;
                    self.prefs.save();
                }
            }
        });
        ui.weak("Caps how often physics and cursor follow redraw. 30 is easy on laptops.");
        ui.add_space(6.0);
        let cur_msaa = match self.prefs.msaa {
            1 | 2 | 4 | 8 => self.prefs.msaa,
            _ => 4,
        };
        ui.horizontal(|ui| {
            ui.label("Model anti-aliasing");
            for (label, v) in [("Off", 1u32), ("2x", 2), ("4x", 4), ("8x", 8)] {
                if ui.selectable_label(cur_msaa == v, label).clicked() {
                    self.prefs.msaa = v;
                    self.prefs.save();
                }
            }
        });
        ui.weak("Takes effect after a restart. Off is the cheapest on old GPUs.");
        ui.add_space(6.0);
        ui.weak("Renderer: wgpu (native GPU), shared with the UI.");
    }

    fn settings_keyboard(&mut self, ui: &mut Ui) {
        ui.heading("Keyboard");
        ui.add_space(6.0);
        ui.weak("Click a binding, then press the new keys. Esc cancels.");
        ui.add_space(4.0);

        // an armed row eats the next real key press
        if let Some(action) = self.rebind {
            let captured = ui.ctx().input(|i| {
                i.events.iter().find_map(|e| {
                    if let egui::Event::Key { key, pressed: true, modifiers, .. } = e {
                        Some((*key, *modifiers))
                    } else {
                        None
                    }
                })
            });
            if let Some((key, modifiers)) = captured {
                if key == egui::Key::Escape {
                    self.rebind = None;
                } else {
                    let sc = egui::KeyboardShortcut::new(modifiers, key);
                    let text = format_shortcut(&sc);
                    self.prefs.keybinds.retain(|(a, _)| a != action);
                    self.prefs.keybinds.push((action.to_string(), text.clone()));
                    self.prefs.save();
                    log::info!("{action} rebound to {text}");
                    self.rebind = None;
                }
            }
        }

        egui::Grid::new("keys").num_columns(3).spacing([24.0, 5.0]).show(ui, |ui| {
            for (action, label, m, k) in KEY_ACTIONS {
                ui.label(label);
                let armed = self.rebind == Some(action);
                let text = if armed {
                    "press keys…".to_string()
                } else {
                    format_shortcut(&self.bound(action))
                };
                let btn = ui.add_sized(
                    vec2(130.0, 20.0),
                    egui::Button::new(if armed {
                        egui::RichText::new(text).color(theme::pal().accent)
                    } else {
                        egui::RichText::new(text)
                    }),
                );
                if btn.clicked() {
                    self.rebind = Some(action);
                }
                let custom = self.prefs.keybinds.iter().any(|(a, _)| a == action);
                if custom {
                    if ui.small_button("Reset").clicked() {
                        self.prefs.keybinds.retain(|(a, _)| a != action);
                        self.prefs.save();
                    }
                } else {
                    let _ = egui::KeyboardShortcut::new(m, k);
                    ui.label("");
                }
                ui.end_row();
            }
        });
        ui.add_space(8.0);
        if !self.prefs.keybinds.is_empty() && ui.button("Reset all to defaults").clicked() {
            self.prefs.keybinds.clear();
            self.prefs.save();
        }
        ui.add_space(8.0);
        ui.separator();
        ui.weak("Built in: Space+drag pans, scroll zooms, Alt+click walks up deformers.");
    }

    fn panel_contents(&mut self, ui: &mut Ui, p: PanelKind, content: Rect) {
        if p == PanelKind::Viewport {
            let mut view = content;
            if self.prefs.toolbar_docked && self.session.is_some() {
                let bar = Rect::from_min_size(content.min, vec2(content.width(), 34.0));
                view = Rect::from_min_max(pos2(content.min.x, bar.max.y), content.max);
                ui.painter().rect_filled(bar, 0.0, theme::pal().panel_bg);
                ui.painter().line_segment(
                    [bar.left_bottom(), bar.right_bottom()],
                    Stroke::new(1.0, theme::pal().border),
                );
                let mut strip =
                    ui.new_child(UiBuilder::new().max_rect(bar.shrink2(vec2(8.0, 4.0))));
                strip.spacing_mut().item_spacing = vec2(3.0, 0.0);
                strip.horizontal_centered(|ui| {
                    self.tool_strip(ui);
                    ui.add_space(8.0);
                    ui.label(
                        egui::RichText::new(self.tool.label())
                            .size(11.0)
                            .color(theme::pal().text_faint),
                    );
                });
            }
            self.viewport(ui, view);
            return;
        }
        let inner = content.shrink(4.0);
        let mut child = ui.new_child(UiBuilder::new().max_rect(inner));
        let ui = &mut child;
        ui.set_clip_rect(inner);
        match p {
            PanelKind::Parameters => self.parameters_panel(ui),
            PanelKind::Parts => self.parts_panel(ui),
            PanelKind::Deformers => self.deformers_panel(ui),
            PanelKind::Textures => self.textures_panel(ui),
            PanelKind::Inspector => self.inspector_panel(ui),
            PanelKind::ToolDetails => self.tool_details_panel(ui),
            PanelKind::Timeline => {
                ui.weak("No animation loaded.");
            }
            PanelKind::Log => {
                ui.horizontal(|ui| {
                    ui.weak("Show:");
                    for (name, lv) in [
                        ("Error", log::Level::Error),
                        ("Warn", log::Level::Warn),
                        ("Info", log::Level::Info),
                        ("Debug", log::Level::Debug),
                    ] {
                        if ui.selectable_label(self.log_view == lv, name).clicked() {
                            self.log_view = lv;
                        }
                    }
                    if let Some(p) = logging::log_path() {
                        ui.weak(format!("· {}", p.display()));
                    }
                });
                ui.separator();
                ScrollArea::vertical().stick_to_bottom(true).show(ui, |ui| {
                    for line in logging::lines(self.log_view) {
                        ui.label(
                            egui::RichText::new(line)
                                .size(10.5)
                                .family(egui::FontFamily::Monospace),
                        );
                    }
                });
            }
            PanelKind::Viewport => {}
        }
    }

    // ---- Parameter palette -------------------------------------------------

    fn edit_target(&self) -> Option<(ayatsuri_core::edit::Target, String)> {
        let s = self.session.as_ref()?;
        match self.selected? {
            Sel::Mesh(i) => Some((
                ayatsuri_core::edit::Target::ArtMesh(i),
                s.puppet.model().art_meshes.get(i).map(|m| m.id.clone()).unwrap_or_default(),
            )),
            Sel::Deformer(i) => Some((
                ayatsuri_core::edit::Target::Deformer(i),
                s.puppet.model().deformers.get(i).map(|d| d.id.clone()).unwrap_or_default(),
            )),
            Sel::Part(i) => Some((
                ayatsuri_core::edit::Target::Part(i),
                s.puppet.model().parts.get(i).map(|q| q.id.clone()).unwrap_or_default(),
            )),
            Sel::Parameter(_) => None,
        }
    }

    fn add_keys_to_selection(&mut self, values: Vec<f32>) {
        let Some((target, name)) = self.edit_target() else {
            log::warn!("select an object before adding keys");
            return;
        };
        let Some(param) = self.sel_param else {
            log::warn!("select a parameter before adding keys");
            return;
        };
        if let Some(s) = &mut self.session {
            let pid =
                s.puppet.model().parameters.get(param).map(|p| p.id.clone()).unwrap_or_default();
            if s.edit(|m| ayatsuri_core::edit::add_keys(m, target, param, &values)) {
                log::info!("added keys {values:?} on {pid} for {name}");
            }
        }
    }

    fn delete_key_on_selection(&mut self) {
        let Some((target, name)) = self.edit_target() else { return };
        let Some(param) = self.sel_param else { return };
        if let Some(s) = &mut self.session {
            let at = s.puppet.value(param);
            if s.edit(|m| ayatsuri_core::edit::delete_key(m, target, param, at)) {
                log::info!("deleted key near {at:.3} for {name}");
            }
        }
    }

    fn unbind_selection(&mut self) {
        let Some((target, name)) = self.edit_target() else { return };
        let Some(param) = self.sel_param else { return };
        if let Some(s) = &mut self.session {
            let default = s.puppet.model().parameters.get(param).map(|p| p.default).unwrap_or(0.0);
            if s.edit(|m| ayatsuri_core::edit::delete_binding(m, target, param, default)) {
                log::info!("removed binding for {name}");
            }
        }
    }

    /// Meshes the next deformer wraps: the lasso set, or the single pick.
    fn selection_meshes(&self) -> Vec<usize> {
        if !self.multi_sel.is_empty() {
            self.multi_sel.clone()
        } else if let Some(Sel::Mesh(mi)) = self.selected {
            vec![mi]
        } else {
            Vec::new()
        }
    }

    /// Ask for the next animation frame, honouring the FPS cap from the
    /// performance settings on low end machines.
    fn request_anim(&self, ctx: &egui::Context) {
        if self.prefs.fps_cap > 0 {
            ctx.request_repaint_after(std::time::Duration::from_secs_f32(
                1.0 / self.prefs.fps_cap as f32,
            ));
        } else {
            ctx.request_repaint();
        }
    }

    /// The shortcut for an action: the user's remap when one is saved, the
    /// Cubism-style default otherwise.
    fn bound(&self, action: &str) -> egui::KeyboardShortcut {
        if let Some((_, text)) = self.prefs.keybinds.iter().find(|(a, _)| a == action)
            && let Some(sc) = parse_shortcut(text)
        {
            return sc;
        }
        KEY_ACTIONS
            .iter()
            .find(|(a, ..)| *a == action)
            .map(|(_, _, m, k)| egui::KeyboardShortcut::new(*m, *k))
            .unwrap_or(egui::KeyboardShortcut::new(egui::Modifiers::NONE, egui::Key::F35))
    }

    /// Whether the current edit target is bound on exactly two parameters,
    /// which is what four corner generation needs.
    fn two_axis_target(&self) -> bool {
        let Some((t, _)) = self.edit_target() else { return false };
        self.session.as_ref().is_some_and(|s| {
            let m = s.puppet.model();
            let axes = match t {
                ayatsuri_core::edit::Target::ArtMesh(i) => {
                    m.art_meshes.get(i).map(|a| a.binding.axes.len())
                }
                ayatsuri_core::edit::Target::Deformer(i) => {
                    m.deformers.get(i).map(|d| d.binding.axes.len())
                }
                _ => None,
            };
            axes == Some(2)
        })
    }

    fn four_corner_forms(&mut self) {
        let Some((target, name)) = self.edit_target() else { return };
        if let Some(s) = &mut self.session {
            if s.edit(|m| ayatsuri_core::edit::four_corner_forms(m, target)) {
                log::info!("four corner forms generated on {name}");
            } else {
                log::warn!("four corner forms need a two-axis binding on {name}");
            }
        }
    }

    /// Rebuild the mesh as a regular grid over its texture alpha, the way
    /// Cubism's auto mesh presets do. Spacing and margin are texture pixels.
    fn auto_mesh(&mut self, mi: usize) {
        let spacing = self.tool_opts.auto_spacing.clamp(4.0, 256.0);
        let margin = self.tool_opts.auto_margin.clamp(0.0, 128.0);
        let Some(s) = &mut self.session else { return };
        let Some(am) = s.puppet.model().art_meshes.get(mi) else { return };
        let tex = am.texture as usize;
        let Some(png) = s.texture_pngs.get(tex) else { return };
        let img = match image::load_from_memory(png) {
            Ok(i) => i.to_rgba8(),
            Err(e) => {
                log::error!("auto mesh: texture decode failed: {e}");
                return;
            }
        };
        let (tw, th) = (img.width() as f32, img.height() as f32);

        // the mesh's patch of the atlas, padded by the margin
        let (mut u0, mut v0, mut u1, mut v1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
        let (uvp, _) = am.uvs.as_chunks::<2>();
        for &[u, v] in uvp {
            u0 = u0.min(u);
            v0 = v0.min(v);
            u1 = u1.max(u);
            v1 = v1.max(v);
        }
        if u0 > u1 || v0 > v1 {
            return;
        }
        let x0 = (u0 * tw - margin).max(0.0);
        let y0 = (v0 * th - margin).max(0.0);
        let x1 = (u1 * tw + margin).min(tw);
        let y1 = (v1 * th + margin).min(th);

        let cols = (((x1 - x0) / spacing).ceil() as usize).clamp(1, 512);
        let rows = (((y1 - y0) / spacing).ceil() as usize).clamp(1, 512);

        // a cell stays when any texel in it has alpha, dilated by the margin
        let probe = (spacing / 6.0).max(1.0);
        let mut covered = vec![false; cols * rows];
        for (ci, slot) in covered.iter_mut().enumerate() {
            let (cx, cy) = (ci % cols, ci / cols);
            let px0 = x0 + cx as f32 * spacing;
            let py0 = y0 + cy as f32 * spacing;
            let mut y = py0;
            'cell: while y < (py0 + spacing).min(y1) {
                let mut x = px0;
                while x < (px0 + spacing).min(x1) {
                    let p = img.get_pixel(
                        (x as u32).min(img.width() - 1),
                        (y as u32).min(img.height() - 1),
                    );
                    if p.0[3] > 8 {
                        *slot = true;
                        break 'cell;
                    }
                    x += probe;
                }
                y += probe;
            }
        }
        let rings = (margin / spacing).ceil() as i64;
        if rings > 0 {
            let src = covered.clone();
            for (ci, slot) in covered.iter_mut().enumerate() {
                if *slot {
                    continue;
                }
                let (cx, cy) = ((ci % cols) as i64, (ci / cols) as i64);
                'dilate: for dy in -rings..=rings {
                    for dx in -rings..=rings {
                        let (nx, ny) = (cx + dx, cy + dy);
                        if nx >= 0
                            && ny >= 0
                            && (nx as usize) < cols
                            && (ny as usize) < rows
                            && src.get(ny as usize * cols + nx as usize) == Some(&true)
                        {
                            *slot = true;
                            break 'dilate;
                        }
                    }
                }
            }
        }

        // shared grid corners become the vertices, two triangles per cell
        let mut corner_ids: std::collections::HashMap<(usize, usize), u16> =
            std::collections::HashMap::new();
        let mut uvs: Vec<f32> = Vec::new();
        let mut indices: Vec<u16> = Vec::new();
        let vid = |corner_ids: &mut std::collections::HashMap<(usize, usize), u16>,
                   uvs: &mut Vec<f32>,
                   gx: usize,
                   gy: usize|
         -> Option<u16> {
            if let Some(&id) = corner_ids.get(&(gx, gy)) {
                return Some(id);
            }
            if uvs.len() / 2 >= u16::MAX as usize - 1 {
                return None;
            }
            let id = (uvs.len() / 2) as u16;
            uvs.push(((x0 + gx as f32 * spacing).min(x1)) / tw);
            uvs.push(((y0 + gy as f32 * spacing).min(y1)) / th);
            corner_ids.insert((gx, gy), id);
            Some(id)
        };
        for (ci, &on) in covered.iter().enumerate() {
            if !on {
                continue;
            }
            let (cx, cy) = (ci % cols, ci / cols);
            let (Some(a), Some(b), Some(c), Some(d)) = (
                vid(&mut corner_ids, &mut uvs, cx, cy),
                vid(&mut corner_ids, &mut uvs, cx + 1, cy),
                vid(&mut corner_ids, &mut uvs, cx + 1, cy + 1),
                vid(&mut corner_ids, &mut uvs, cx, cy + 1),
            ) else {
                log::error!("auto mesh: too many vertices, raise the spacing");
                return;
            };
            indices.extend_from_slice(&[a, b, c, a, c, d]);
        }
        if indices.is_empty() {
            log::warn!("auto mesh: no opaque texels found under this mesh");
            return;
        }
        let verts = uvs.len() / 2;
        if s.edit(|m| ayatsuri_core::edit::remesh(m, mi, &uvs, &indices)) {
            log::info!(
                "auto mesh: {verts} vertices, {} triangles at {spacing:.0}px spacing",
                indices.len() / 3
            );
        }
    }

    fn delete_parameter(&mut self, pi: usize) {
        let Some(s) = &mut self.session else { return };
        if !s.edit(|m| ayatsuri_core::edit::delete_parameter(m, pi)) {
            log::warn!("parameter is still used by a binding or physics; unbind it first");
            return;
        }
        if pi < s.param_names.len() {
            s.param_names.remove(pi);
        }
        if pi < s.param_colors.len() {
            s.param_colors.remove(pi);
        }
        for g in &mut s.groups {
            g.params.retain(|&p| p != pi);
            for p in &mut g.params {
                if *p > pi {
                    *p -= 1;
                }
            }
        }
        s.combined.retain(|&(a, b)| a != pi && b != pi);
        for pair in &mut s.combined {
            if pair.0 > pi {
                pair.0 -= 1;
            }
            if pair.1 > pi {
                pair.1 -= 1;
            }
        }
        self.sel_param = None;
        log::info!("parameter deleted");
    }

    fn model_new_window(&mut self, ui: &mut Ui) {
        let Some(new) = &mut self.new_model else { return };
        let mut open = true;
        let mut create = false;
        egui::Window::new("New Model").open(&mut open).collapsible(false).resizable(false).show(
            ui.ctx(),
            |ui| {
                ui.set_width(300.0);
                egui::Grid::new("mnew").num_columns(2).spacing([10.0, 6.0]).show(ui, |ui| {
                    ui.label("Name");
                    ui.text_edit_singleline(&mut new.name);
                    ui.end_row();
                    ui.label("Author");
                    ui.text_edit_singleline(&mut new.author);
                    ui.end_row();
                    ui.label("Canvas width");
                    ui.add(
                        egui::DragValue::new(&mut new.width).range(64.0..=16384.0).suffix(" px"),
                    );
                    ui.end_row();
                    ui.label("Canvas height");
                    ui.add(
                        egui::DragValue::new(&mut new.height).range(64.0..=16384.0).suffix(" px"),
                    );
                    ui.end_row();
                    ui.label("Pixels per unit");
                    ui.add(egui::DragValue::new(&mut new.ppu).range(1.0..=8192.0));
                    ui.end_row();
                });
                ui.checkbox(&mut new.std_params, "Start with the standard parameter set");
                ui.weak("Angle, eyes, mouth, body and breath, the ids trackers expect.");
                ui.add_space(6.0);
                if ui.button("Create").clicked() {
                    create = true;
                }
            },
        );
        if create {
            let Some(new) = self.new_model.take() else { return };
            let mut model = Model::new();
            model.canvas = ayatsuri_format::model::Canvas {
                width: new.width.max(1.0),
                height: new.height.max(1.0),
                origin_x: new.width.max(1.0) * 0.5,
                origin_y: new.height.max(1.0) * 0.5,
                pixels_per_unit: new.ppu.max(1.0),
            };
            if new.std_params {
                // the published standard parameter list, ranges included
                let std: [(&str, f32, f32, f32); 14] = [
                    ("ParamAngleX", -30.0, 30.0, 0.0),
                    ("ParamAngleY", -30.0, 30.0, 0.0),
                    ("ParamAngleZ", -30.0, 30.0, 0.0),
                    ("ParamEyeLOpen", 0.0, 1.0, 1.0),
                    ("ParamEyeROpen", 0.0, 1.0, 1.0),
                    ("ParamEyeBallX", -1.0, 1.0, 0.0),
                    ("ParamEyeBallY", -1.0, 1.0, 0.0),
                    ("ParamBrowLY", -1.0, 1.0, 0.0),
                    ("ParamBrowRY", -1.0, 1.0, 0.0),
                    ("ParamMouthForm", -1.0, 1.0, 0.0),
                    ("ParamMouthOpenY", 0.0, 1.0, 0.0),
                    ("ParamBreath", 0.0, 1.0, 0.0),
                    ("ParamBodyAngleX", -10.0, 10.0, 0.0),
                    ("ParamBodyAngleY", -10.0, 10.0, 0.0),
                ];
                for (id, min, max, default) in std {
                    model.parameters.push(ayatsuri_format::model::Parameter {
                        id: id.into(),
                        min,
                        max,
                        default,
                        repeat: false,
                        decimals: 3,
                    });
                }
            }
            let name = if new.name.is_empty() { "untitled".into() } else { new.name };
            match Session::create(&ui.ctx().clone(), name, new.author, model) {
                Ok(s) => {
                    self.session = Some(s);
                    self.selected = None;
                    self.multi_sel.clear();
                    self.sel_param = None;
                    log::info!("new model created");
                }
                Err(e) => log::error!("new model: {e}"),
            }
        } else if !open {
            self.new_model = None;
        }
    }

    fn model_info_window(&mut self, ui: &mut Ui) {
        if !self.show_model_info {
            return;
        }
        let mut open = self.show_model_info;
        egui::Window::new("Model Info").open(&mut open).collapsible(false).resizable(false).show(
            ui.ctx(),
            |ui| {
                let Some(s) = &mut self.session else {
                    ui.weak("No model loaded.");
                    return;
                };
                ui.set_width(280.0);
                egui::Grid::new("minfo").num_columns(2).spacing([10.0, 6.0]).show(ui, |ui| {
                    ui.label("Name");
                    ui.text_edit_singleline(&mut s.name);
                    ui.end_row();
                    ui.label("Author");
                    ui.text_edit_singleline(&mut s.author);
                    ui.end_row();
                    let c = s.puppet.model().canvas;
                    ui.label("Canvas");
                    ui.weak(format!(
                        "{} × {} px, {} px/unit",
                        c.width, c.height, c.pixels_per_unit
                    ));
                    ui.end_row();
                });
                ui.weak("Name and author save into the .aom meta block.");
            },
        );
        self.show_model_info = open;
    }

    fn param_new_window(&mut self, ui: &mut Ui) {
        let Some(new) = &mut self.new_param else { return };
        let mut open = true;
        let mut create = false;
        let group_names: Vec<String> = self
            .session
            .as_ref()
            .map(|s| {
                s.groups
                    .iter()
                    .map(|g| if g.name.is_empty() { "Parameters".into() } else { g.name.clone() })
                    .collect()
            })
            .unwrap_or_default();
        egui::Window::new("New Parameter")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .show(ui.ctx(), |ui| {
                ui.set_width(280.0);
                egui::Grid::new("pnew").num_columns(2).spacing([10.0, 6.0]).show(ui, |ui| {
                    ui.label("Name");
                    ui.text_edit_singleline(&mut new.name);
                    ui.end_row();
                    ui.label("ID");
                    ui.text_edit_singleline(&mut new.id);
                    ui.end_row();
                    ui.label("Minimum");
                    ui.add(egui::DragValue::new(&mut new.min).speed(0.1));
                    ui.end_row();
                    ui.label("Default");
                    ui.add(egui::DragValue::new(&mut new.default).speed(0.1));
                    ui.end_row();
                    ui.label("Maximum");
                    ui.add(egui::DragValue::new(&mut new.max).speed(0.1));
                    ui.end_row();
                    ui.label("Folder");
                    egui::ComboBox::from_id_salt("pnew-group")
                        .selected_text(group_names.get(new.group).cloned().unwrap_or_default())
                        .show_ui(ui, |ui| {
                            for (i, n) in group_names.iter().enumerate() {
                                ui.selectable_value(&mut new.group, i, n);
                            }
                        });
                    ui.end_row();
                });
                ui.add_space(6.0);
                if ui.button("Create").clicked() {
                    create = true;
                }
            });
        if create {
            let (name, id, min, default, max, group) = {
                let n = self.new_param.take();
                let Some(n) = n else { return };
                (n.name, n.id, n.min, n.default, n.max, n.group)
            };
            let id = if id.is_empty() { name.replace(' ', "") } else { id };
            if let Some(s) = &mut self.session {
                if s.edit(|m| {
                    ayatsuri_core::edit::create_parameter(m, id.clone(), min, default, max)
                        .is_some()
                }) {
                    let pi = s.puppet.model().parameters.len() - 1;
                    s.param_names.push(if name.is_empty() { id.clone() } else { name });
                    let slot = group.min(s.groups.len().saturating_sub(1));
                    if let Some(g) = s.groups.get_mut(slot) {
                        g.params.push(pi);
                    }
                    self.sel_param = Some(pi);
                    log::info!("parameter {id} created");
                } else {
                    log::warn!("a parameter with id {id} already exists");
                }
            }
        } else if !open {
            self.new_param = None;
        }
    }

    fn folder_new_window(&mut self, ui: &mut Ui) {
        let Some(name) = &mut self.new_folder else { return };
        let mut open = true;
        let mut create = false;
        egui::Window::new("New Folder").open(&mut open).collapsible(false).resizable(false).show(
            ui.ctx(),
            |ui| {
                ui.set_width(220.0);
                ui.text_edit_singleline(name);
                if ui.button("Create").clicked() {
                    create = true;
                }
            },
        );
        if create {
            if let (Some(s), Some(name)) = (&mut self.session, self.new_folder.take()) {
                let name = if name.is_empty() { "Folder".into() } else { name };
                s.groups.push(crate::session::ParamGroup { name, params: Vec::new(), open: true });
            }
        } else if !open {
            self.new_folder = None;
        }
    }

    fn close_session(&mut self) {
        self.session = None;
        self.selected = None;
        self.multi_sel.clear();
        self.sel_param = None;
        self.mesh_edit = None;
        self.mesh_drag = None;
        self.mesh_snapshot = None;
        self.deform_path = None;
        self.warp_drag = None;
        self.rot_drag = None;
    }

    fn close_confirm_window(&mut self, ui: &mut Ui) {
        let Some(action) = self.confirm_close else { return };
        let name = self.session.as_ref().map(|s| s.name.clone()).unwrap_or_default();
        let mut choice: Option<u8> = None; // 0 cancel, 1 save, 2 discard
        egui::Window::new("Unsaved Changes")
            .collapsible(false)
            .resizable(false)
            .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
            .show(ui.ctx(), |ui| {
                ui.set_width(300.0);
                ui.label(format!("Save changes to {name} before closing?"));
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    if ui.button(egui::RichText::new("Save").strong()).clicked() {
                        choice = Some(1);
                    }
                    if ui.button("Don't Save").clicked() {
                        choice = Some(2);
                    }
                    if ui.button("Cancel").clicked() {
                        choice = Some(0);
                    }
                });
            });
        match choice {
            Some(0) => self.confirm_close = None,
            Some(1) => {
                self.save_session(false);
                // a cancelled file picker leaves it modified; stay open then
                if self.session.as_ref().is_some_and(|s| !s.modified) {
                    self.confirm_close = None;
                    if action == 1 {
                        self.force_close = true;
                        ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                    } else {
                        self.close_session();
                    }
                }
            }
            Some(2) => {
                self.confirm_close = None;
                if action == 1 {
                    self.force_close = true;
                    ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                } else {
                    self.close_session();
                }
            }
            _ => {}
        }
    }

    fn reset_parameters(&mut self) {
        if let Some(s) = &mut self.session {
            for i in 0..s.puppet.model().parameters.len() {
                let Some(d) = s.puppet.model().parameters.get(i).map(|p| p.default) else {
                    break;
                };
                s.puppet.set_value(i, d);
            }
            log::info!("parameters reset to defaults");
        }
    }

    fn toggle_selected_visibility(&mut self) {
        let meshes = self.selection_meshes();
        if let Some(s) = &mut self.session {
            for mi in meshes {
                if let Some(f) = s.mesh_shown.get_mut(mi) {
                    *f = !*f;
                    s.puppet.set_mesh_visible(mi, *f);
                }
            }
        }
    }

    /// Render the current pose to a PNG at canvas resolution.
    fn export_image(&mut self) {
        let Some(s) = &self.session else { return };
        let picked = rfd::FileDialog::new()
            .add_filter("PNG image", &["png"])
            .set_file_name(format!("{}.png", s.name))
            .save_file();
        let Some(path) = picked else { return };
        let mut tex = Vec::new();
        for png in &s.texture_pngs {
            match image::load_from_memory(png) {
                Ok(img) => {
                    let rgba = img.to_rgba8();
                    tex.push(ayatsuri_render::TexData {
                        width: rgba.width(),
                        height: rgba.height(),
                        rgba: rgba.into_raw(),
                    });
                }
                Err(e) => {
                    log::error!("image export: texture decode failed: {e}");
                    tex.push(ayatsuri_render::TexData { width: 1, height: 1, rgba: vec![255; 4] });
                }
            }
        }
        let width = s.puppet.model().canvas.width.max(1.0) as usize;
        let (w, h, rgba) = crate::thumb::render(&s.puppet, &tex, width);
        match image::RgbaImage::from_raw(w as u32, h as u32, rgba) {
            Some(img) => match img.save(&path) {
                Ok(()) => log::info!("image exported to {}", path.display()),
                Err(e) => log::error!("image export: {e}"),
            },
            None => log::error!("image export: buffer size mismatch"),
        }
    }

    fn create_deformer(&mut self, warp: bool, click_origin: Option<(f32, f32)>) {
        let meshes = self.selection_meshes();
        if meshes.is_empty() {
            log::warn!("select the meshes the deformer should hold first");
            return;
        }
        let Some(s) = &mut self.session else { return };
        // the click lands in model space; the deformer stores its origin in
        // parent space, so nested parents invert through the local jacobian
        // around the first mesh's centroid
        let at_root = meshes.iter().all(|&mi| {
            s.puppet.model().art_meshes.get(mi).is_some_and(|m| m.parent_deformer.is_none())
        });
        let origin = match (click_origin, at_root) {
            (Some(o), true) => Some(o),
            (Some((mx, my)), false) => meshes.first().and_then(|&mi| {
                let parent = s.puppet.model().art_meshes.get(mi).and_then(|m| m.parent_deformer);
                let local = s.puppet.mesh_local_positions(mi)?;
                let (lp, _) = local.as_chunks::<2>();
                let n = lp.len().max(1) as f32;
                let (clx, cly) =
                    lp.iter().fold((0.0, 0.0), |(x, y), q| (x + q[0] / n, y + q[1] / n));
                let (cmx, cmy) = {
                    let g = s.puppet.mesh_geometry(mi)?;
                    let (gp, _) = g.positions.as_chunks::<2>();
                    let n = gp.len().max(1) as f32;
                    gp.iter().fold((0.0, 0.0), |(x, y), q| (x + q[0] / n, y + q[1] / n))
                };
                let (dx, dy) = parent_delta(&s.puppet, parent, clx, cly, mx - cmx, my - cmy);
                Some((clx + dx, cly + dy))
            }),
            _ => None,
        };
        let new_index = s.puppet.model().deformers.len();
        let (cols, rows) = (self.tool_opts.warp_cols, self.tool_opts.warp_rows);
        let id = if warp { format!("Warp{new_index}") } else { format!("Rotation{new_index}") };
        let made = s.edit(|m| {
            if warp {
                ayatsuri_core::edit::create_warp_deformer(m, &meshes, rows, cols, id.clone())
                    .is_some()
            } else {
                ayatsuri_core::edit::create_rotation_deformer(m, &meshes, origin, id.clone())
                    .is_some()
            }
        });
        if made {
            log::info!(
                "created {} around {} mesh(es)",
                if warp { "warp deformer" } else { "rotation deformer" },
                meshes.len()
            );
            self.selected = Some(Sel::Deformer(new_index));
        } else {
            log::warn!("could not create deformer: meshes must share one parent deformer");
        }
    }

    fn parameters_panel(&mut self, ui: &mut Ui) {
        if self.session.is_none() {
            ui.weak("No model loaded.");
            return;
        }

        // the palette's own little toolbar, like Cubism's icon row
        ui.horizontal(|ui| {
            let all_open = self.session.as_ref().is_some_and(|s| s.groups.iter().all(|g| g.open));
            let chev = if all_open {
                egui_phosphor::regular::CARET_UP
            } else {
                egui_phosphor::regular::CARET_DOWN
            };
            if ui.button(chev).on_hover_text("Collapse or expand all groups").clicked()
                && let Some(s) = &mut self.session
            {
                for g in &mut s.groups {
                    g.open = !all_open;
                }
            }
            ui.separator();
            let have_target = self.edit_target().is_some() && self.sel_param.is_some();
            let two = ui
                .add_enabled(have_target, egui::Button::new("••+"))
                .on_hover_text("Add keys at min and max (selected object)");
            let three = ui
                .add_enabled(have_target, egui::Button::new("•••+"))
                .on_hover_text("Add keys at min, default and max");
            let one = ui
                .add_enabled(have_target, egui::Button::new("•+"))
                .on_hover_text("Add a key at the current value");
            let minus = ui
                .add_enabled(have_target, egui::Button::new("•−"))
                .on_hover_text("Delete the key nearest the current value");
            let unbind = ui
                .add_enabled(have_target, egui::Button::new(egui_phosphor::regular::LINK_BREAK))
                .on_hover_text("Remove the whole binding");
            if let Some(param) = self.sel_param
                && let Some(meta) = self
                    .session
                    .as_ref()
                    .and_then(|s| s.puppet.model().parameters.get(param).cloned())
            {
                let current =
                    self.session.as_ref().map(|s| s.puppet.value(param)).unwrap_or(meta.default);
                if two.clicked() {
                    self.add_keys_to_selection(vec![meta.min, meta.max]);
                }
                if three.clicked() {
                    self.add_keys_to_selection(vec![meta.min, meta.default, meta.max]);
                }
                if one.clicked() {
                    self.add_keys_to_selection(vec![current]);
                }
                if minus.clicked() {
                    self.delete_key_on_selection();
                }
                if unbind.clicked() {
                    self.unbind_selection();
                }
            }
            match self.edit_target() {
                Some((_, name)) => ui.weak(truncate(&name, 14)),
                None => ui.weak("no object selected"),
            };
        });
        ui.separator();

        // rows borrow the session hard, so it steps out of self for a moment
        let Some(mut s) = self.session.take() else { return };
        let can_edit = matches!(
            self.selected,
            Some(Sel::Mesh(_)) | Some(Sel::Deformer(_)) | Some(Sel::Part(_))
        );
        let mut actions: Vec<ParamAction> = Vec::new();

        let pad_partner: std::collections::HashMap<usize, usize> =
            s.combined.iter().map(|&(a, b)| (a, b)).collect();
        let pad_skip: std::collections::HashSet<usize> =
            s.combined.iter().map(|&(_, b)| b).collect();

        ScrollArea::vertical().id_salt("params").show(ui, |ui| {
            for gi in 0..s.groups.len() {
                let (name, open, params) = {
                    let Some(g) = s.groups.get(gi) else { break };
                    (g.name.clone(), g.open, g.params.clone())
                };
                if !name.is_empty() {
                    let (rect, resp) =
                        ui.allocate_exact_size(vec2(ui.available_width(), 18.0), Sense::click());
                    ui.painter().rect_filled(rect, 2.0, theme::pal().header);
                    let arrow = if open { "⏷" } else { "⏵" };
                    ui.painter().text(
                        rect.left_center() + vec2(4.0, 0.0),
                        Align2::LEFT_CENTER,
                        format!("{arrow} {name}"),
                        FontId::proportional(11.0),
                        theme::pal().text,
                    );
                    if resp.clicked()
                        && let Some(g) = s.groups.get_mut(gi)
                    {
                        g.open = !g.open;
                    }
                    if !open {
                        continue;
                    }
                }
                for &pi in &params {
                    if pad_skip.contains(&pi) {
                        continue;
                    }
                    if let Some(&partner) = pad_partner.get(&pi) {
                        combined_pad(ui, &mut s, pi, partner, &mut self.sel_param, &mut actions);
                        continue;
                    }
                    parameter_row(ui, &mut s, pi, &mut self.sel_param, can_edit, &mut actions);
                }
            }
        });
        self.session = Some(s);

        // the footer: new parameters and folders live here, like Cubism
        ui.separator();
        ui.horizontal(|ui| {
            if ui.button(egui_phosphor::regular::PLUS).on_hover_text("Add a parameter").clicked() {
                self.new_param = Some(ParamNew {
                    name: String::new(),
                    id: String::new(),
                    min: 0.0,
                    default: 0.0,
                    max: 1.0,
                    group: self
                        .session
                        .as_ref()
                        .map(|s| s.groups.len().saturating_sub(1))
                        .unwrap_or(0),
                });
            }
            if ui
                .button(egui_phosphor::regular::FOLDER_PLUS)
                .on_hover_text("Add a folder")
                .clicked()
            {
                self.new_folder = Some(String::new());
            }
            let deletable = self.sel_param.is_some();
            if ui
                .add_enabled(deletable, egui::Button::new(egui_phosphor::regular::TRASH))
                .on_hover_text("Delete the selected parameter (must be unused)")
                .clicked()
                && let Some(pi) = self.sel_param
            {
                self.delete_parameter(pi);
            }
        });

        for action in actions {
            match action {
                ParamAction::Select(pi) => {
                    self.sel_param = Some(pi);
                    self.selected = Some(Sel::Parameter(pi));
                }
                ParamAction::AddSingle(pi, v) => {
                    self.sel_param = Some(pi);
                    self.add_keys_to_selection(vec![v]);
                }
                ParamAction::DeleteKey(pi) => {
                    self.sel_param = Some(pi);
                    self.delete_key_on_selection();
                }
                ParamAction::OpenEdit(pi) => {
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
            }
        }
    }

    fn param_edit_window(&mut self, ui: &mut Ui) {
        let Some(edit) = &mut self.param_edit else { return };
        let mut open = true;
        let mut apply = false;
        egui::Window::new("Edit Parameter")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .show(ui.ctx(), |ui| {
                ui.set_width(280.0);
                egui::Grid::new("pedit").num_columns(2).spacing([10.0, 6.0]).show(ui, |ui| {
                    ui.label("Name");
                    ui.text_edit_singleline(&mut edit.name);
                    ui.end_row();
                    ui.label("ID");
                    ui.text_edit_singleline(&mut edit.id);
                    ui.end_row();
                    ui.label("Minimum");
                    ui.add(egui::DragValue::new(&mut edit.min).speed(0.1));
                    ui.end_row();
                    ui.label("Default");
                    ui.add(egui::DragValue::new(&mut edit.default).speed(0.1));
                    ui.end_row();
                    ui.label("Maximum");
                    ui.add(egui::DragValue::new(&mut edit.max).speed(0.1));
                    ui.end_row();
                    ui.label("Repeat");
                    ui.checkbox(&mut edit.repeat, "");
                    ui.end_row();
                });
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    if ui.button("OK").clicked() {
                        apply = true;
                    }
                });
            });
        if apply {
            let Some(edit) = self.param_edit.take() else { return };
            if let Some(s) = &mut self.session {
                let idx = edit.index;
                let (id, min, max, default, repeat) =
                    (edit.id.clone(), edit.min, edit.max, edit.default, edit.repeat);
                let changed = s.edit(move |m| {
                    let Some(p) = m.parameters.get_mut(idx) else { return false };
                    let was = (p.id.clone(), p.min, p.max, p.default, p.repeat);
                    p.id = id;
                    p.min = min.min(max);
                    p.max = max.max(min);
                    p.default = default.clamp(p.min, p.max);
                    p.repeat = repeat;
                    was != (p.id.clone(), p.min, p.max, p.default, p.repeat)
                });
                if let Some(n) = s.param_names.get_mut(idx) {
                    *n = edit.name;
                }
                if changed {
                    log::info!("parameter {idx} settings changed");
                }
            }
        } else if !open {
            self.param_edit = None;
        }
    }
    // ---- Part palette ------------------------------------------------------

    fn parts_panel(&mut self, ui: &mut Ui) {
        let Some(s) = &mut self.session else {
            ui.weak("No model loaded.");
            return;
        };
        // the real tree: children render under their parent, depth-indented
        let n = s.puppet.model().parts.len();
        let mut children: Vec<Vec<usize>> = vec![Vec::new(); n];
        let mut roots: Vec<usize> = Vec::new();
        for (i, part) in s.puppet.model().parts.iter().enumerate() {
            match part.parent_part {
                Some(p) if (p as usize) < n => {
                    if let Some(c) = children.get_mut(p as usize) {
                        c.push(i);
                    }
                }
                _ => roots.push(i),
            }
        }
        let mut order: Vec<(usize, usize)> = Vec::new();
        let mut stack: Vec<(usize, usize)> = roots.into_iter().rev().map(|i| (i, 0)).collect();
        let mut guard = 0;
        while let Some((i, depth)) = stack.pop() {
            order.push((i, depth));
            if let Some(kids) = children.get(i) {
                for &k in kids.iter().rev() {
                    stack.push((k, depth + 1));
                }
            }
            guard += 1;
            if guard > n * 2 + 16 {
                break;
            }
        }

        // live stack positions, front-most last, for the right-hand column
        let stack: std::collections::HashMap<usize, usize> =
            s.puppet.drawables().enumerate().map(|(pos, d)| (d.mesh, pos + 1)).collect();
        ScrollArea::vertical().id_salt("parts").show(ui, |ui| {
            for (i, depth) in order {
                let Some((id, parent)) =
                    s.puppet.model().parts.get(i).map(|p| (p.id.clone(), p.parent_part))
                else {
                    continue;
                };
                let name = s.part_names.get(i).cloned().unwrap_or(id);
                let shown = s.part_shown.get(i).copied().unwrap_or(true);
                let row_w = ui.available_width();
                let (rect, resp) = ui.allocate_exact_size(vec2(row_w, 18.0), Sense::click());
                if self.selected == Some(Sel::Part(i)) {
                    ui.painter().rect_filled(rect, 0.0, theme::pal().select);
                }
                let effective = shown && ancestors_shown(s, parent);
                let eye = Rect::from_min_size(rect.min, vec2(18.0, 18.0));
                paint_eye(ui, eye, shown, effective);
                let eresp = ui.interact(eye, egui::Id::new(("part-eye", i)), Sense::click());
                if eresp.clicked() {
                    let now = !shown;
                    if let Some(f) = s.part_shown.get_mut(i) {
                        *f = now;
                    }
                    s.puppet.set_part_opacity(i, if now { 1.0 } else { 0.0 });
                    s.dirty = true;
                }
                let ring = pos2(rect.min.x + 27.0, rect.center().y);
                ui.painter().circle_stroke(ring, 4.0, Stroke::new(1.2, theme::pal().key_ring));
                let label_x = rect.min.x + 38.0 + depth as f32 * 14.0;
                paint_folder(ui, pos2(label_x, rect.center().y));
                ui.painter().text(
                    pos2(label_x + 16.0, rect.center().y),
                    Align2::LEFT_CENTER,
                    truncate(&name, 22),
                    FontId::proportional(11.5),
                    theme::pal().text,
                );
                if resp.clicked() {
                    self.selected = Some(Sel::Part(i));
                }
                resp.context_menu(|ui| {
                    if ui.button(if shown { "Hide" } else { "Show" }).clicked() {
                        if let Some(f) = s.part_shown.get_mut(i) {
                            *f = !shown;
                        }
                        s.puppet.set_part_opacity(i, if shown { 0.0 } else { 1.0 });
                        s.dirty = true;
                        ui.close();
                    }
                    if ui.button("Select").clicked() {
                        self.selected = Some(Sel::Part(i));
                        ui.close();
                    }
                });

                // the part's meshes, nested one level deeper
                let part_effective = effective;
                let meshes: Vec<(usize, String)> = s
                    .puppet
                    .model()
                    .art_meshes
                    .iter()
                    .enumerate()
                    .filter(|(_, m)| m.parent_part == Some(i as u32))
                    .map(|(mi, m)| (mi, m.id.clone()))
                    .collect();
                for (mi, mid) in meshes {
                    let (rect, resp) =
                        ui.allocate_exact_size(vec2(ui.available_width(), 16.0), Sense::click());
                    if self.selected == Some(Sel::Mesh(mi)) {
                        ui.painter().rect_filled(rect, 0.0, theme::pal().select);
                    }
                    let mesh_on = s.mesh_shown.get(mi).copied().unwrap_or(true);
                    let eye = Rect::from_min_size(rect.min, vec2(18.0, 16.0));
                    paint_eye(ui, eye, mesh_on, mesh_on && part_effective);
                    let eresp = ui.interact(eye, egui::Id::new(("mesh-eye", mi)), Sense::click());
                    if eresp.clicked() {
                        let now = !mesh_on;
                        if let Some(f) = s.mesh_shown.get_mut(mi) {
                            *f = now;
                        }
                        s.puppet.set_mesh_visible(mi, now);
                        s.dirty = true;
                    }
                    let x = rect.min.x + 56.0 + depth as f32 * 14.0;
                    ui.painter().circle_filled(
                        pos2(x, rect.center().y),
                        2.0,
                        theme::pal().text_faint,
                    );
                    ui.painter().text(
                        pos2(x + 8.0, rect.center().y),
                        Align2::LEFT_CENTER,
                        truncate(&mid, 20),
                        FontId::proportional(10.5),
                        theme::pal().text,
                    );
                    // the live stack position, what actually decides overlap
                    let pos_label =
                        stack.get(&mi).map(|p| format!("#{p}")).unwrap_or_else(|| "–".into());
                    ui.painter().text(
                        rect.right_center() - vec2(6.0, 0.0),
                        Align2::RIGHT_CENTER,
                        pos_label,
                        FontId::proportional(10.0),
                        theme::pal().text_faint,
                    );
                    if resp.clicked() {
                        self.selected = Some(Sel::Mesh(mi));
                    }
                }
            }
        });
    }
    fn tool_strip(&mut self, ui: &mut Ui) {
        // fixed-width name slot first, so switching tools never resizes the bar
        if !self.prefs.toolbar_docked {
            let (nr, _) = ui.allocate_exact_size(vec2(96.0, 26.0), Sense::hover());
            ui.painter().text(
                nr.left_center() + vec2(2.0, 0.0),
                Align2::LEFT_CENTER,
                self.tool.label(),
                FontId::proportional(11.0),
                theme::pal().text,
            );
        }
        for (gi, group) in Tool::GROUPS.iter().enumerate() {
            if gi > 0 {
                let (sep, _) = ui.allocate_exact_size(vec2(7.0, 26.0), Sense::hover());
                ui.painter().line_segment(
                    [sep.center() + vec2(0.0, -9.0), sep.center() + vec2(0.0, 9.0)],
                    Stroke::new(1.0, theme::pal().border),
                );
            }
            for &t in group.iter() {
                let active = self.tool == t;
                let (rect, resp) = ui.allocate_exact_size(vec2(26.0, 26.0), Sense::click());
                if active {
                    ui.painter().rect_filled(rect, 5.0, theme::pal().select);
                    ui.painter().rect_stroke(
                        rect,
                        5.0,
                        Stroke::new(1.0, theme::pal().accent),
                        StrokeKind::Inside,
                    );
                } else if resp.hovered() {
                    ui.painter().rect_filled(rect, 5.0, theme::pal().header);
                }
                let icon_color = if active { theme::pal().accent } else { theme::pal().text };
                if t == Tool::Glue {
                    paint_glue_bottle(ui, rect, icon_color);
                } else {
                    ui.painter().text(
                        rect.center(),
                        Align2::CENTER_CENTER,
                        t.icon(),
                        FontId::proportional(16.0),
                        icon_color,
                    );
                }
                let docked = self.prefs.toolbar_docked;
                resp.context_menu(|ui| {
                    if ui.button(if docked { "Float toolbar" } else { "Dock toolbar" }).clicked() {
                        self.prefs.toolbar_docked = !docked;
                        self.prefs.save();
                        ui.close();
                    }
                });
                let resp = resp.on_hover_text(t.label());
                if resp.clicked() {
                    self.tool = t;
                }
            }
        }
    }

    fn tool_details_panel(&mut self, ui: &mut Ui) {
        ui.label(egui::RichText::new(self.tool.label()).size(12.0).strong());
        ui.add_space(4.0);
        let o = &mut self.tool_opts;
        match self.tool {
            Tool::MeshManual => {
                ui.horizontal(|ui| {
                    for (i, m) in ["Select", "Add", "Erase"].iter().enumerate() {
                        if ui.selectable_label(o.mesh_mode == i, *m).clicked() {
                            o.mesh_mode = i;
                        }
                    }
                });
                ui.checkbox(&mut o.mesh_snap, "Snap to nearby vertices");
                ui.weak("Mesh editing lands with the modeling tools.");
            }
            Tool::MeshAuto => {
                ui.horizontal(|ui| {
                    ui.label("Preset");
                    for (i, m) in ["Standard", "Deform", "Light"].iter().enumerate() {
                        if ui.selectable_label(o.auto_preset == i, *m).clicked() {
                            o.auto_preset = i;
                        }
                    }
                });
                ui.horizontal(|ui| {
                    ui.label("Point spacing");
                    ui.add(egui::DragValue::new(&mut o.auto_spacing).range(4.0..=128.0));
                });
                ui.horizontal(|ui| {
                    ui.label("Margin");
                    ui.add(egui::DragValue::new(&mut o.auto_margin).range(0.0..=64.0));
                });
                let _ = ui.add_enabled(false, egui::Button::new("Generate"));
                ui.weak("Automatic meshing is on the roadmap.");
            }
            Tool::Glue => {
                ui.horizontal(|ui| {
                    ui.label("Weight");
                    ui.add(egui::Slider::new(&mut o.glue_weight, 0.0..=1.0));
                });
                let _ = ui.add_enabled(false, egui::Button::new("Bind selected vertices"));
                ui.weak("Glue evaluation already runs; binding UI is next.");
            }
            Tool::WarpCreate => {
                ui.horizontal(|ui| {
                    ui.label("Columns");
                    ui.add(egui::DragValue::new(&mut o.warp_cols).range(1..=32));
                    ui.label("Rows");
                    ui.add(egui::DragValue::new(&mut o.warp_rows).range(1..=32));
                });
                ui.checkbox(&mut o.sequential, "Sequential creation");
                if ui.button("Create around selection").clicked() {
                    self.pending_create = Some((true, None));
                }
                ui.weak("Or click the canvas with meshes selected.");
            }
            Tool::RotationCreate => {
                ui.checkbox(&mut o.sequential, "Sequential creation");
                if ui.button("Create at selection center").clicked() {
                    self.pending_create = Some((false, None));
                }
                ui.weak("Or click the canvas to place the origin.");
            }
            Tool::BrushSelect | Tool::DeformBrush => {
                ui.horizontal(|ui| {
                    ui.label("Brush size");
                    ui.add(egui::Slider::new(&mut o.brush_size, 2.0..=200.0));
                });
            }
            Tool::Arrow | Tool::Lasso => {
                ui.weak("Shift adds to the selection; drag for a rectangle.");
            }
            Tool::DeformPath | Tool::ArtPath | Tool::TextureAtlas => {
                ui.weak("No options yet for this tool.");
            }
        }
    }

    fn deformers_panel(&mut self, ui: &mut Ui) {
        let Some(s) = &mut self.session else {
            ui.weak("No model loaded.");
            return;
        };
        // hierarchy: deformers under their parents, meshes under deformers
        let model = s.puppet.model();
        let nd = model.deformers.len();
        let mut children: Vec<Vec<usize>> = vec![Vec::new(); nd];
        let mut roots: Vec<usize> = Vec::new();
        for (i, d) in model.deformers.iter().enumerate() {
            match d.parent_deformer {
                Some(p) if (p as usize) < nd => {
                    if let Some(c) = children.get_mut(p as usize) {
                        c.push(i);
                    }
                }
                _ => roots.push(i),
            }
        }
        let mut order: Vec<(usize, usize)> = Vec::new();
        let mut stack: Vec<(usize, usize)> = roots.into_iter().rev().map(|i| (i, 0)).collect();
        let mut guard = 0;
        while let Some((i, depth)) = stack.pop() {
            order.push((i, depth));
            if let Some(kids) = children.get(i) {
                for &k in kids.iter().rev() {
                    stack.push((k, depth + 1));
                }
            }
            guard += 1;
            if guard > nd * 2 + 16 {
                break;
            }
        }
        let rows: Vec<(usize, usize, String, bool)> = order
            .into_iter()
            .filter_map(|(i, depth)| {
                model
                    .deformers
                    .get(i)
                    .map(|d| (i, depth, d.id.clone(), matches!(d.kind, DeformerKind::Warp(_))))
            })
            .collect();

        ScrollArea::vertical().id_salt("deformers").show(ui, |ui| {
            for (i, dep, id, is_warp) in rows {
                let (rect, resp) =
                    ui.allocate_exact_size(vec2(ui.available_width(), 18.0), Sense::click());
                if self.selected == Some(Sel::Deformer(i)) {
                    ui.painter().rect_filled(rect, 0.0, theme::pal().select);
                }
                let shown = s.deformer_shown.get(i).copied().unwrap_or(true);
                let eye = Rect::from_min_size(rect.min, vec2(18.0, 18.0));
                paint_eye(ui, eye, shown, shown);
                let eresp = ui.interact(eye, egui::Id::new(("def-eye", i)), Sense::click());
                if eresp.clicked() {
                    let now = !shown;
                    if let Some(f) = s.deformer_shown.get_mut(i) {
                        *f = now;
                    }
                    s.puppet.set_deformer_visible(i, now);
                    s.dirty = true;
                }
                let x = rect.min.x + 24.0 + dep as f32 * 14.0;
                if is_warp {
                    paint_warp_glyph(ui, pos2(x, rect.center().y));
                } else {
                    paint_rotation_glyph(ui, pos2(x, rect.center().y));
                }
                ui.painter().text(
                    pos2(x + 16.0, rect.center().y),
                    Align2::LEFT_CENTER,
                    truncate(&id, 24),
                    FontId::proportional(11.5),
                    theme::pal().text,
                );
                if resp.clicked() {
                    self.selected = Some(Sel::Deformer(i));
                }
                resp.context_menu(|ui| {
                    if ui.button("Select children").clicked() {
                        self.multi_sel = s
                            .puppet
                            .model()
                            .art_meshes
                            .iter()
                            .enumerate()
                            .filter(|(_, m)| m.parent_deformer == Some(i as u32))
                            .map(|(mi, _)| mi)
                            .collect();
                        if let Some(&first) = self.multi_sel.last() {
                            self.selected = Some(Sel::Mesh(first));
                        }
                        ui.close();
                    }
                });

                // this deformer's meshes, nested
                let meshes: Vec<(usize, String)> = s
                    .puppet
                    .model()
                    .art_meshes
                    .iter()
                    .enumerate()
                    .filter(|(_, m)| m.parent_deformer == Some(i as u32))
                    .map(|(mi, m)| (mi, m.id.clone()))
                    .collect();
                for (mi, mid) in meshes {
                    let (rect, resp) =
                        ui.allocate_exact_size(vec2(ui.available_width(), 15.0), Sense::click());
                    if self.selected == Some(Sel::Mesh(mi)) {
                        ui.painter().rect_filled(rect, 0.0, theme::pal().select);
                    }
                    let x = rect.min.x + 40.0 + dep as f32 * 14.0;
                    ui.painter().circle_filled(
                        pos2(x, rect.center().y),
                        1.8,
                        theme::pal().text_faint,
                    );
                    ui.painter().text(
                        pos2(x + 7.0, rect.center().y),
                        Align2::LEFT_CENTER,
                        truncate(&mid, 22),
                        FontId::proportional(10.5),
                        theme::pal().text,
                    );
                    if resp.clicked() {
                        self.selected = Some(Sel::Mesh(mi));
                    }
                }
            }
        });
    }
    fn inspector_panel(&mut self, ui: &mut Ui) {
        ScrollArea::vertical().id_salt("props").show(ui, |ui| self.inspector_contents(ui));
    }

    fn inspector_contents(&mut self, ui: &mut Ui) {
        let Some(s) = &mut self.session else {
            ui.weak("Nothing selected.");
            return;
        };
        let row = |ui: &mut Ui, k: &str, v: String| {
            ui.horizontal(|ui| {
                ui.add_sized(
                    [74.0, 16.0],
                    egui::Label::new(
                        egui::RichText::new(k).color(theme::pal().text_faint).size(11.0),
                    ),
                );
                ui.label(egui::RichText::new(v).size(11.5));
            });
        };
        match self.selected {
            Some(Sel::Mesh(i)) => {
                let Some(m) = s.puppet.model().art_meshes.get(i).cloned() else { return };
                row(ui, "ID", m.id.clone());
                row(
                    ui,
                    "Part",
                    m.parent_part
                        .and_then(|p| s.part_names.get(p as usize).cloned())
                        .unwrap_or_else(|| "-".into()),
                );
                row(
                    ui,
                    "Deformer",
                    m.parent_deformer
                        .and_then(|d| s.puppet.model().deformers.get(d as usize))
                        .map_or("-".into(), |d| d.id.clone()),
                );
                row(ui, "Texture", format!("{}", m.texture));
                row(ui, "Vertices", format!("{}", m.vertex_count));
                let stack = s.puppet.drawables().position(|d| d.mesh == i);
                row(ui, "Render #", stack.map_or("hidden".into(), |n| format!("{n}")));
                row(
                    ui,
                    "Clipping ID",
                    if m.masks.is_empty() {
                        "-".into()
                    } else {
                        m.masks
                            .iter()
                            .map(|x| {
                                s.puppet
                                    .model()
                                    .art_meshes
                                    .get(*x as usize)
                                    .map(|am| am.id.clone())
                                    .unwrap_or_else(|| x.to_string())
                            })
                            .collect::<Vec<_>>()
                            .join(", ")
                    },
                );
                ui.separator();

                // the editable half, like Cubism's inspector
                use ayatsuri_format::model::flags;
                let blend = if m.flags & flags::BLEND_ADDITIVE != 0 {
                    1
                } else if m.flags & flags::BLEND_MULTIPLICATIVE != 0 {
                    2
                } else {
                    0
                };
                let mut new_blend = blend;
                ui.horizontal(|ui| {
                    ui.add_sized(
                        [74.0, 16.0],
                        egui::Label::new(
                            egui::RichText::new("Blend mode")
                                .color(theme::pal().text_faint)
                                .size(11.0),
                        ),
                    );
                    egui::ComboBox::from_id_salt(("blend", i))
                        .selected_text(
                            ["Normal", "Additive", "Multiply"]
                                .get(new_blend.min(2))
                                .copied()
                                .unwrap_or("Normal"),
                        )
                        .show_ui(ui, |ui| {
                            for (bi, name) in ["Normal", "Additive", "Multiply"].iter().enumerate()
                            {
                                ui.selectable_value(&mut new_blend, bi, *name);
                            }
                        });
                });
                let mut double_sided = m.flags & flags::DOUBLE_SIDED != 0;
                let mut inverted = m.flags & flags::INVERTED_MASK != 0;
                ui.horizontal(|ui| {
                    ui.add_space(78.0);
                    ui.checkbox(&mut double_sided, "Double-sided");
                });
                ui.horizontal(|ui| {
                    ui.add_space(78.0);
                    ui.checkbox(&mut inverted, "Invert mask");
                });
                let base_order = m.draw_orders.first().copied().unwrap_or(500.0);
                let mut order = base_order;
                ui.horizontal(|ui| {
                    ui.add_sized(
                        [74.0, 16.0],
                        egui::Label::new(
                            egui::RichText::new("Draw order")
                                .color(theme::pal().text_faint)
                                .size(11.0),
                        ),
                    );
                    ui.add(egui::DragValue::new(&mut order).range(0.0..=1000.0).speed(1));
                });

                let mut new_flags = m.flags
                    & !(flags::BLEND_ADDITIVE
                        | flags::BLEND_MULTIPLICATIVE
                        | flags::DOUBLE_SIDED
                        | flags::INVERTED_MASK);
                if new_blend == 1 {
                    new_flags |= flags::BLEND_ADDITIVE;
                }
                if new_blend == 2 {
                    new_flags |= flags::BLEND_MULTIPLICATIVE;
                }
                if double_sided {
                    new_flags |= flags::DOUBLE_SIDED;
                }
                if inverted {
                    new_flags |= flags::INVERTED_MASK;
                }
                // clipping is editable: comma-separated mesh IDs
                let clip_id = egui::Id::new(("clipedit", i));
                let mut clip_text = ui.ctx().data_mut(|d| {
                    d.get_temp::<String>(clip_id).unwrap_or_else(|| {
                        m.masks
                            .iter()
                            .filter_map(|x| {
                                s.puppet.model().art_meshes.get(*x as usize).map(|am| am.id.clone())
                            })
                            .collect::<Vec<_>>()
                            .join(", ")
                    })
                });
                ui.horizontal(|ui| {
                    ui.add_sized(
                        [74.0, 16.0],
                        egui::Label::new(
                            egui::RichText::new("Clip by")
                                .color(theme::pal().text_faint)
                                .size(11.0),
                        ),
                    );
                    let edit =
                        ui.add(egui::TextEdit::singleline(&mut clip_text).desired_width(130.0));
                    if edit.changed() {
                        ui.ctx().data_mut(|d| d.insert_temp(clip_id, clip_text.clone()));
                    }
                    if ui.small_button("Apply").clicked() {
                        let wanted: Vec<u32> = clip_text
                            .split(',')
                            .map(str::trim)
                            .filter(|t| !t.is_empty())
                            .filter_map(|t| {
                                s.puppet
                                    .model()
                                    .art_meshes
                                    .iter()
                                    .position(|am| am.id == t)
                                    .map(|x| x as u32)
                            })
                            .filter(|&x| x as usize != i)
                            .collect();
                        let changed = s.edit(|model| {
                            let Some(am) = model.art_meshes.get_mut(i) else { return false };
                            if am.masks == wanted {
                                return false;
                            }
                            am.masks = wanted.clone();
                            true
                        });
                        if changed {
                            log::info!(
                                "clipping of mesh {i} set to {} source(s)",
                                clip_text.split(',').count()
                            );
                        }
                        ui.ctx().data_mut(|d| d.remove::<String>(clip_id));
                    }
                });

                if new_flags != m.flags || (order - base_order).abs() > 0.5 {
                    let changed = s.edit(|model| {
                        let Some(am) = model.art_meshes.get_mut(i) else { return false };
                        am.flags = new_flags;
                        if (order - base_order).abs() > 0.5 {
                            let delta = order - base_order;
                            for d in &mut am.draw_orders {
                                *d += delta;
                            }
                        }
                        true
                    });
                    if changed {
                        log::debug!("mesh {i} properties changed");
                    }
                }
            }
            Some(Sel::Deformer(i)) => {
                let Some(d) = s.puppet.model().deformers.get(i) else { return };
                row(ui, "ID", d.id.clone());
                row(
                    ui,
                    "Part",
                    d.parent_part
                        .and_then(|p| s.part_names.get(p as usize).cloned())
                        .unwrap_or_else(|| "-".into()),
                );
                row(
                    ui,
                    "Deformer",
                    d.parent_deformer
                        .and_then(|p| s.puppet.model().deformers.get(p as usize))
                        .map_or("-".into(), |p| p.id.clone()),
                );
                match &d.kind {
                    DeformerKind::Warp(w) => {
                        row(ui, "Type", format!("Warp {}x{}", w.cols, w.rows));
                        row(ui, "Keyforms", format!("{}", w.opacities.len()));
                    }
                    DeformerKind::Rotation(r) => {
                        row(ui, "Type", "Rotation".into());
                        row(ui, "Base angle", format!("{:.1}", r.base_angle));
                        row(ui, "Keyforms", format!("{}", r.opacities.len()));
                    }
                }
                let kids = s
                    .puppet
                    .model()
                    .art_meshes
                    .iter()
                    .filter(|m| m.parent_deformer == Some(i as u32))
                    .count();
                row(ui, "Children", format!("{kids} meshes"));
            }
            Some(Sel::Part(i)) => {
                let Some(p) = s.puppet.model().parts.get(i) else { return };
                row(ui, "Name", s.part_names.get(i).cloned().unwrap_or_default());
                row(ui, "ID", p.id.clone());
                row(
                    ui,
                    "Parent",
                    p.parent_part
                        .and_then(|q| s.part_names.get(q as usize).cloned())
                        .unwrap_or_else(|| "-".into()),
                );
                ui.separator();
                let shown = s.part_shown.get(i).copied().unwrap_or(true);
                let mut opacity = if shown { 1.0f32 } else { 0.0 };
                ui.horizontal(|ui| {
                    ui.add_sized(
                        [74.0, 16.0],
                        egui::Label::new(
                            egui::RichText::new("Opacity")
                                .color(theme::pal().text_faint)
                                .size(11.0),
                        ),
                    );
                    if ui.add(egui::Slider::new(&mut opacity, 0.0..=1.0)).changed() {
                        s.puppet.set_part_opacity(i, opacity);
                        if let Some(f) = s.part_shown.get_mut(i) {
                            *f = opacity > 0.0;
                        }
                        s.dirty = true;
                    }
                });
            }
            Some(Sel::Parameter(i)) => {
                let Some(p) = s.puppet.model().parameters.get(i) else { return };
                row(ui, "Name", s.param_names.get(i).cloned().unwrap_or_default());
                row(ui, "ID", p.id.clone());
                row(ui, "Range", format!("{} to {}", p.min, p.max));
                row(ui, "Default", format!("{}", p.default));
                row(ui, "Keys", format!("{}", s.param_keys.get(i).map_or(0, Vec::len)));
                row(ui, "Repeat", if p.repeat { "yes".into() } else { "no".into() });
            }
            None => {
                if self.multi_sel.len() > 1 {
                    ui.weak(format!("{} meshes selected", self.multi_sel.len()));
                } else {
                    ui.weak("Nothing selected.");
                }
            }
        }
    }
    fn textures_panel(&mut self, ui: &mut Ui) {
        let Some(s) = &self.session else {
            ui.weak("No textures.");
            return;
        };
        ScrollArea::vertical().id_salt("textures").show(ui, |ui| {
            for (i, t) in s.textures.iter().enumerate() {
                match t {
                    Some(t) => {
                        ui.weak(format!(
                            "{}: {}",
                            i,
                            s.texture_names.get(i).cloned().unwrap_or_else(|| "?".into())
                        ));
                        let size = t.size_vec2();
                        let w = ui.available_width().min(160.0);
                        ui.image((t.id(), vec2(w, w * size.y / size.x.max(1.0))));
                    }
                    None => {
                        ui.weak(format!("{i}: failed to load"));
                    }
                }
            }
        });
    }

    // ---- View ----------------------------------------------------------------

    /// The view area: gray surround, the white canvas page, the puppet, and
    /// the zoom strip along the bottom, Cubism-style.
    fn viewport(&mut self, ui: &mut Ui, content: Rect) {
        let painter = ui.painter_at(content);
        let view_bg = self
            .prefs
            .view_bg
            .map(|[r, g, b]| Color32::from_rgb(r, g, b))
            .unwrap_or(theme::pal().canvas_bg);
        painter.rect_filled(content, 0.0, view_bg);

        let strip_h = 26.0;
        let view = Rect::from_min_max(content.min, pos2(content.max.x, content.max.y - strip_h));

        let Some(s) = &mut self.session else {
            painter.text(
                view.center(),
                Align2::CENTER_CENTER,
                "操り2D · drop a .moc3 or .model3.json here",
                FontId::proportional(13.0),
                theme::pal().text_faint,
            );
            return;
        };

        // space + drag pans, with the grab cursor; middle drag always pans
        let space = ui.ctx().input(|i| i.key_down(egui::Key::Space));
        let resp = ui.interact(view, egui::Id::new("view-nav"), Sense::click_and_drag());
        if space && resp.hovered() {
            ui.ctx().set_cursor_icon(if resp.dragged() {
                egui::CursorIcon::Grabbing
            } else {
                egui::CursorIcon::Grab
            });
        }
        let middle = ui.ctx().input(|i| i.pointer.middle_down());
        if resp.dragged() && (space || middle) {
            self.view_pan += resp.drag_delta();
        }
        resp.context_menu(|ui| {
            if ui.button("Fit").clicked() {
                self.view_zoom = 1.0;
                self.view_pan = Vec2::ZERO;
                ui.close();
            }
            if ui.button("Reset view").clicked() {
                self.view_zoom = 1.0;
                self.view_pan = Vec2::ZERO;
                ui.close();
            }
        });
        let scroll = ui.ctx().input(|i| if resp.hovered() { i.smooth_scroll_delta.y } else { 0.0 });
        if scroll != 0.0 {
            let factor = (scroll * 0.0015).exp();
            let pivot = ui.ctx().input(|i| i.pointer.latest_pos()).unwrap_or(view.center());
            let center = view.center() + self.view_pan;
            self.view_pan += (center - pivot) * (factor - 1.0);
            self.view_zoom = (self.view_zoom * factor).clamp(0.02, 40.0);
        }

        let c = s.puppet.model().canvas;
        let (cw, ch) = (c.width.max(1.0), c.height.max(1.0));
        let fit = ((view.width() / cw).min(view.height() / ch) * 0.92).max(1e-4);
        let scale = fit * self.view_zoom;
        let origin = view.center() + self.view_pan - vec2(cw, ch) * scale * 0.5;

        // the canvas page
        let page = Rect::from_min_size(pos2(origin.x, origin.y), vec2(cw * scale, ch * scale));
        let vp = ui.painter_at(view);
        vp.rect_filled(page, 0.0, theme::pal().canvas);
        vp.rect_stroke(page, 0.0, Stroke::new(1.0, Color32::from_gray(0x8c)), StrokeKind::Outside);

        let to_screen = |x: f32, y: f32| -> Pos2 {
            let px = c.origin_x + x * c.pixels_per_unit;
            let py = c.origin_y + y * c.pixels_per_unit;
            pos2(origin.x + px * scale, origin.y + py * scale)
        };

        if self.gpu {
            let cb = crate::gpu_view::callback(
                view,
                &s.puppet,
                s.gpu_pending.take(),
                pos2(origin.x, origin.y),
                scale,
                ui.ctx().pixels_per_point(),
            );
            vp.add(cb);
        } else {
            for d in s.puppet.drawables() {
                if d.opacity <= 0.0 || d.indices.is_empty() {
                    continue;
                }
                let texture = s.textures.get(d.texture as usize).and_then(|t| t.as_ref());
                let tint = Color32::from_white_alpha((d.opacity * 255.0) as u8);
                let mut mesh = match texture {
                    Some(t) => egui::Mesh::with_texture(t.id()),
                    None => egui::Mesh::default(),
                };
                mesh.vertices.reserve(d.positions.len() / 2);
                let (pairs, _) = d.positions.as_chunks::<2>();
                let (uv_pairs, _) = d.uvs.as_chunks::<2>();
                let mut uvs = uv_pairs.iter();
                for &[x, y] in pairs {
                    let uv = match (texture, uvs.next()) {
                        (Some(_), Some(&[u, v])) => pos2(u, v),
                        _ => egui::epaint::WHITE_UV,
                    };
                    mesh.vertices.push(egui::epaint::Vertex {
                        pos: to_screen(x, y),
                        uv,
                        color: tint,
                    });
                }
                mesh.indices.extend(d.indices.iter().map(|&i| i as u32));
                vp.add(egui::Shape::mesh(mesh));
            }
        }

        // the Select tool: hover previews a mesh, click selects it, empty
        // click deselects, double-click jumps into mesh editing
        if self.tool == Tool::Arrow && !space && self.mesh_edit.is_none() {
            let unit_of = |ptr: Pos2| {
                (
                    ((ptr.x - origin.x) / scale - c.origin_x) / c.pixels_per_unit,
                    ((ptr.y - origin.y) / scale - c.origin_y) / c.pixels_per_unit,
                )
            };
            let hover_ptr = ui.ctx().input(|i| i.pointer.latest_pos());
            let hovered_mesh = if resp.hovered() && !resp.dragged() {
                hover_ptr.and_then(|ptr| {
                    let (ux, uy) = unit_of(ptr);
                    hit_mesh(&s.puppet, ux, uy)
                })
            } else {
                None
            };
            if let Some(mi) = hovered_mesh
                && self.selected != Some(Sel::Mesh(mi))
                && let Some(g) = s.puppet.mesh_geometry(mi)
            {
                // a faint wireframe preview under the cursor
                let stroke = Stroke::new(1.0, theme::pal().accent.gamma_multiply(0.45));
                let (pos, _) = g.positions.as_chunks::<2>();
                let (tris, _) = g.indices.as_chunks::<3>();
                for &[a, b, cx] in tris {
                    let (Some(p0), Some(p1), Some(p2)) =
                        (pos.get(a as usize), pos.get(b as usize), pos.get(cx as usize))
                    else {
                        continue;
                    };
                    let s0 = to_screen(p0[0], p0[1]);
                    let s1 = to_screen(p1[0], p1[1]);
                    let s2 = to_screen(p2[0], p2[1]);
                    vp.line_segment([s0, s1], stroke);
                    vp.line_segment([s1, s2], stroke);
                    vp.line_segment([s2, s0], stroke);
                }
            }
            if resp.clicked()
                && let Some(ptr) = resp.interact_pointer_pos()
            {
                let (ux, uy) = unit_of(ptr);
                let alt = ui.ctx().input(|i| i.modifiers.alt);
                let hit = hit_mesh(&s.puppet, ux, uy);
                if alt {
                    // alt+click selects the mesh's deformer; again walks up
                    let start = match (self.selected, hit) {
                        (Some(Sel::Deformer(d)), _) => {
                            s.puppet.model().deformers.get(d).and_then(|d| d.parent_deformer)
                        }
                        (_, Some(mi)) => {
                            s.puppet.model().art_meshes.get(mi).and_then(|m| m.parent_deformer)
                        }
                        _ => None,
                    };
                    if let Some(d) = start {
                        self.selected = Some(Sel::Deformer(d as usize));
                    }
                } else {
                    // clicking empty canvas deselects
                    self.selected = hit.map(Sel::Mesh);
                    self.multi_sel.clear();
                    if let Some(mi) = hit {
                        self.multi_sel.push(mi);
                    }
                }
            }
            if resp.double_clicked()
                && let Some(ptr) = resp.interact_pointer_pos()
            {
                let (ux, uy) = unit_of(ptr);
                if let Some(mi) = hit_mesh(&s.puppet, ux, uy) {
                    self.selected = Some(Sel::Mesh(mi));
                    self.mesh_edit = Some(mi);
                    self.mesh_drag = None;
                    self.mesh_snapshot = Some(s.puppet.model().clone());
                }
            }
        }

        // the Edit Mesh tool: clicking a mesh jumps straight into edit mode
        if self.tool == Tool::MeshManual
            && !space
            && self.mesh_edit.is_none()
            && resp.clicked()
            && let Some(ptr) = resp.interact_pointer_pos()
        {
            let ux = ((ptr.x - origin.x) / scale - c.origin_x) / c.pixels_per_unit;
            let uy = ((ptr.y - origin.y) / scale - c.origin_y) / c.pixels_per_unit;
            if let Some(mi) = hit_mesh(&s.puppet, ux, uy) {
                self.selected = Some(Sel::Mesh(mi));
                self.mesh_edit = Some(mi);
                self.mesh_drag = None;
                self.mesh_snapshot = Some(s.puppet.model().clone());
            }
        }

        // creation tools: a click places the deformer around the selection
        if matches!(self.tool, Tool::WarpCreate | Tool::RotationCreate)
            && !space
            && self.mesh_edit.is_none()
            && resp.clicked()
            && let Some(ptr) = resp.interact_pointer_pos()
        {
            let ux = ((ptr.x - origin.x) / scale - c.origin_x) / c.pixels_per_unit;
            let uy = ((ptr.y - origin.y) / scale - c.origin_y) / c.pixels_per_unit;
            self.pending_create = Some((self.tool == Tool::WarpCreate, Some((ux, uy))));
        }

        // the lasso: draw the freehand outline while dragging, select the
        // front-most mesh caught inside on release
        if self.tool == Tool::Lasso && !space && self.mesh_edit.is_none() {
            if resp.drag_started() {
                self.lasso.clear();
            }
            if resp.dragged()
                && let Some(ptr) = resp.interact_pointer_pos()
                && self.lasso.last().is_none_or(|l| l.distance(ptr) > 2.0)
            {
                self.lasso.push(ptr);
            }
            if self.lasso.len() > 1 {
                vp.add(egui::Shape::line(
                    self.lasso.clone(),
                    Stroke::new(1.0, theme::pal().accent),
                ));
                if let (Some(&first), Some(&last)) = (self.lasso.first(), self.lasso.last()) {
                    vp.add(egui::Shape::dashed_line(
                        &[last, first],
                        Stroke::new(1.0, theme::pal().accent),
                        4.0,
                        3.0,
                    ));
                }
            }
            if resp.drag_stopped() {
                let mut caught_all: Vec<usize> = Vec::new();
                if self.lasso.len() > 2 {
                    let order: Vec<_> = s.puppet.drawables().map(|d| d.mesh).collect();
                    for &mi in &order {
                        let Some(g) = s.puppet.mesh_geometry(mi) else { continue };
                        let (pos, _) = g.positions.as_chunks::<2>();
                        let caught = pos
                            .iter()
                            .any(|xy| point_in_polygon(to_screen(xy[0], xy[1]), &self.lasso));
                        if caught {
                            caught_all.push(mi);
                        }
                    }
                }
                // the front-most is primary; the rest stay selected with it
                self.selected = caught_all.last().copied().map(Sel::Mesh);
                self.multi_sel = caught_all;
                if self.multi_sel.len() > 1 {
                    log::info!("lasso selected {} meshes", self.multi_sel.len());
                }
                self.lasso.clear();
            } else if resp.clicked() {
                self.selected = None;
                self.multi_sel.clear();
            }
        }

        // the lasso's extra picks get a lighter wireframe
        for &mi in &self.multi_sel {
            if self.selected == Some(Sel::Mesh(mi)) {
                continue;
            }
            if let Some(g) = s.puppet.mesh_geometry(mi) {
                let stroke = Stroke::new(1.0, theme::pal().accent.gamma_multiply(0.5));
                let (pos, _) = g.positions.as_chunks::<2>();
                let (tris, _) = g.indices.as_chunks::<3>();
                for &[a, b, cx] in tris {
                    let (Some(p0), Some(p1), Some(p2)) =
                        (pos.get(a as usize), pos.get(b as usize), pos.get(cx as usize))
                    else {
                        continue;
                    };
                    vp.line_segment([to_screen(p0[0], p0[1]), to_screen(p1[0], p1[1])], stroke);
                    vp.line_segment([to_screen(p1[0], p1[1]), to_screen(p2[0], p2[1])], stroke);
                    vp.line_segment([to_screen(p2[0], p2[1]), to_screen(p0[0], p0[1])], stroke);
                }
            }
        }

        // overlays for whatever is selected
        match self.selected {
            Some(Sel::Mesh(mi)) => {
                if let Some(g) = s.puppet.mesh_geometry(mi) {
                    let (pos, _) = g.positions.as_chunks::<2>();
                    let (tris, _) = g.indices.as_chunks::<3>();
                    let stroke = Stroke::new(1.0, theme::pal().accent);
                    for &[a, b, cx] in tris {
                        let (Some(p0), Some(p1), Some(p2)) =
                            (pos.get(a as usize), pos.get(b as usize), pos.get(cx as usize))
                        else {
                            continue;
                        };
                        let s0 = to_screen(p0[0], p0[1]);
                        let s1 = to_screen(p1[0], p1[1]);
                        let s2 = to_screen(p2[0], p2[1]);
                        vp.line_segment([s0, s1], stroke);
                        vp.line_segment([s1, s2], stroke);
                        vp.line_segment([s2, s0], stroke);
                    }
                    for xy in pos {
                        vp.circle_filled(to_screen(xy[0], xy[1]), 1.6, theme::pal().accent);
                    }
                }
            }
            Some(Sel::Deformer(di)) => {
                if let Some((grid, rows, cols)) = s.puppet.warp_grid(di) {
                    let stride = cols as usize + 1;
                    let total = (rows as usize + 1) * stride;
                    let mut screen_pts: Vec<Pos2> = Vec::with_capacity(total);
                    {
                        let (pts, _) = grid.as_chunks::<2>();
                        for p in pts {
                            screen_pts.push(to_screen(p[0], p[1]));
                        }
                    }

                    // grid points drag like mesh vertices: grab, bank the
                    // parent-space delta, apply one edit on release
                    let ptr = ui.ctx().input(|i| i.pointer.latest_pos());
                    let mut near: Option<(usize, f32)> = None;
                    if !space && let Some(ptr) = ptr {
                        for (gi, sp) in screen_pts.iter().enumerate() {
                            let d = sp.distance(ptr);
                            if d < 8.0 && near.is_none_or(|(_, nd)| d < nd) {
                                near = Some((gi, d));
                            }
                        }
                    }
                    if !space {
                        if resp.drag_started()
                            && let Some((gi, _)) = near
                            && let Some(local) = s.puppet.warp_local_grid(di)
                        {
                            self.warp_drag =
                                Some(WarpDrag { deformer: di, point: gi, local, dx: 0.0, dy: 0.0 });
                        }
                        if resp.dragged()
                            && let Some(d) = &mut self.warp_drag
                        {
                            let parent = s
                                .puppet
                                .model()
                                .deformers
                                .get(di)
                                .and_then(|de| de.parent_deformer);
                            let delta = resp.drag_delta();
                            let mdx = delta.x / (scale * c.pixels_per_unit);
                            let mdy = delta.y / (scale * c.pixels_per_unit);
                            let at = d.point * 2;
                            let px = d.local.get(at).copied().unwrap_or(0.0) + d.dx;
                            let py = d.local.get(at + 1).copied().unwrap_or(0.0) + d.dy;
                            let (ldx, ldy) = parent_delta(&s.puppet, parent, px, py, mdx, mdy);
                            d.dx += ldx;
                            d.dy += ldy;
                        }
                        if near.is_some() {
                            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                        }
                    }
                    // the dragged point rides the pointer while the grab lasts
                    if let (Some(d), Some(ptr)) = (&self.warp_drag, ptr)
                        && d.deformer == di
                        && resp.dragged()
                        && let Some(sp) = screen_pts.get_mut(d.point)
                    {
                        *sp = ptr;
                    }

                    let stroke = Stroke::new(1.0, theme::pal().warp_green);
                    let drag_pt = self.warp_drag.as_ref().map(|d| d.point);
                    let hover_pt = near.map(|(gi, _)| gi);
                    for r in 0..=rows as usize {
                        for q in 0..=cols as usize {
                            let gi = r * stride + q;
                            let Some(&pa) = screen_pts.get(gi) else { continue };
                            if q < cols as usize
                                && let Some(&b) = screen_pts.get(gi + 1)
                            {
                                vp.line_segment([pa, b], stroke);
                            }
                            if r < rows as usize
                                && let Some(&b) = screen_pts.get(gi + stride)
                            {
                                vp.line_segment([pa, b], stroke);
                            }
                            let hot =
                                drag_pt == Some(gi) || (drag_pt.is_none() && hover_pt == Some(gi));
                            if hot {
                                vp.circle(
                                    pa,
                                    4.0,
                                    theme::pal().accent,
                                    Stroke::new(1.0, theme::pal().warp_green),
                                );
                            } else {
                                vp.circle_filled(pa, 1.8, theme::pal().warp_green);
                            }
                        }
                    }
                } else if let Some(rot) = s.puppet.rotation_state(di) {
                    let mut o = to_screen(rot.origin.x, rot.origin.y);
                    let mut angle = rot.angle;
                    let ptr = ui.ctx().input(|i| i.pointer.latest_pos());

                    // live preview while a grab is in flight
                    if let Some(d) = &self.rot_drag
                        && d.deformer == di
                    {
                        if d.turning {
                            angle += d.degrees;
                        } else if let Some(ptr) = ptr
                            && resp.dragged()
                        {
                            o = ptr;
                        }
                    }
                    let a = angle.to_radians();
                    let dir = vec2(a.sin(), -a.cos()) * 26.0;
                    let tip = o + dir;

                    let near_origin =
                        ptr.is_some_and(|p| p.distance(o) < 9.0) && self.rot_drag.is_none();
                    let near_tip = ptr.is_some_and(|p| p.distance(tip) < 9.0)
                        && !near_origin
                        && self.rot_drag.is_none();
                    if !space {
                        if resp.drag_started()
                            && (near_origin || near_tip)
                            && let Some(ptr) = ptr
                            && let Some(local) = s.puppet.rotation_local_origin(di)
                        {
                            self.rot_drag = Some(RotDrag {
                                deformer: di,
                                turning: near_tip,
                                local_origin: local,
                                dx: 0.0,
                                dy: 0.0,
                                degrees: 0.0,
                                last_ptr: ptr,
                            });
                        }
                        if resp.dragged()
                            && let Some(ptr) = ptr
                            && let Some(d) = &mut self.rot_drag
                            && d.deformer == di
                        {
                            if d.turning {
                                let prev = d.last_ptr - o;
                                let cur = ptr - o;
                                if prev.length() > 2.0 && cur.length() > 2.0 {
                                    let mut diff = cur.y.atan2(cur.x) - prev.y.atan2(prev.x);
                                    while diff > std::f32::consts::PI {
                                        diff -= std::f32::consts::TAU;
                                    }
                                    while diff < -std::f32::consts::PI {
                                        diff += std::f32::consts::TAU;
                                    }
                                    d.degrees += diff.to_degrees();
                                }
                                d.last_ptr = ptr;
                            } else {
                                let parent = s
                                    .puppet
                                    .model()
                                    .deformers
                                    .get(di)
                                    .and_then(|de| de.parent_deformer);
                                let delta = resp.drag_delta();
                                let mdx = delta.x / (scale * c.pixels_per_unit);
                                let mdy = delta.y / (scale * c.pixels_per_unit);
                                let px = d.local_origin.0 + d.dx;
                                let py = d.local_origin.1 + d.dy;
                                let (ldx, ldy) = parent_delta(&s.puppet, parent, px, py, mdx, mdy);
                                d.dx += ldx;
                                d.dy += ldy;
                            }
                        }
                        if resp.drag_stopped()
                            && let Some(d) = self.rot_drag.take()
                            && d.deformer == di
                        {
                            if d.turning && d.degrees.abs() > 1e-3 {
                                let (deg, dfm) = (d.degrees, d.deformer);
                                if s.edit(|m| ayatsuri_core::edit::rotate_rotation(m, dfm, deg)) {
                                    log::debug!("rotation {dfm} turned {deg:.2} deg");
                                }
                            } else if !d.turning && (d.dx != 0.0 || d.dy != 0.0) {
                                let (dx, dy, dfm) = (d.dx, d.dy, d.deformer);
                                if s.edit(|m| {
                                    ayatsuri_core::edit::move_rotation_origin(m, dfm, dx, dy)
                                }) {
                                    log::debug!("rotation {dfm} origin moved ({dx:.3}, {dy:.3})");
                                }
                            }
                        }
                        if near_origin || near_tip {
                            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                        }
                    }

                    let stroke = Stroke::new(1.4, theme::pal().rotation_red);
                    vp.circle_stroke(o, 26.0, stroke);
                    vp.line_segment([o, tip], stroke);
                    let turning = self.rot_drag.as_ref().is_some_and(|d| d.turning);
                    if near_tip || turning {
                        vp.circle(tip, 4.5, theme::pal().accent, stroke);
                    } else {
                        vp.circle_filled(tip, 3.0, theme::pal().rotation_red);
                    }
                    if near_origin || self.rot_drag.as_ref().is_some_and(|d| !d.turning) {
                        vp.circle(o, 4.5, theme::pal().accent, stroke);
                    } else {
                        vp.circle_filled(o, 2.5, theme::pal().rotation_red);
                    }
                }
            }
            _ => {}
        }

        // the deform path tool: lay control points over the selected mesh,
        // then drag any of them to pull nearby vertices with a smooth falloff
        if self.tool != Tool::DeformPath {
            self.deform_path = None;
        } else if !space {
            let unit_of = |ptr: Pos2| {
                (
                    ((ptr.x - origin.x) / scale - c.origin_x) / c.pixels_per_unit,
                    ((ptr.y - origin.y) / scale - c.origin_y) / c.pixels_per_unit,
                )
            };
            // the path follows whichever mesh is selected
            if let Some(Sel::Mesh(mi)) = self.selected
                && self.deform_path.as_ref().map(|p| p.mesh) != Some(mi)
            {
                self.deform_path = Some(PathDeform { mesh: mi, points: Vec::new(), drag: None });
            }
            if self.deform_path.is_none()
                && resp.clicked()
                && let Some(ptr) = resp.interact_pointer_pos()
            {
                let (ux, uy) = unit_of(ptr);
                if let Some(mi) = hit_mesh(&s.puppet, ux, uy) {
                    self.selected = Some(Sel::Mesh(mi));
                    self.deform_path =
                        Some(PathDeform { mesh: mi, points: Vec::new(), drag: None });
                }
            }
            let mut drop_path = false;
            if let Some(path) = &mut self.deform_path {
                let mi = path.mesh;
                let radius = self.tool_opts.brush_size.max(4.0);
                let ptr_now = ui.ctx().input(|i| i.pointer.latest_pos());

                // the nearest control point in reach, for hover and grabbing
                let mut near: Option<(usize, f32)> = None;
                if let Some(ptr) = ptr_now {
                    for (pi, &(px, py)) in path.points.iter().enumerate() {
                        let d = to_screen(px, py).distance(ptr);
                        if d < 8.0 && near.is_none_or(|(_, nd)| d < nd) {
                            near = Some((pi, d));
                        }
                    }
                }

                if resp.clicked()
                    && near.is_none()
                    && let Some(ptr) = resp.interact_pointer_pos()
                {
                    let (ux, uy) = unit_of(ptr);
                    path.points.push((ux, uy));
                }
                if ui.ctx().input(|i| i.key_pressed(egui::Key::Backspace)) {
                    path.points.pop();
                }
                if ui.ctx().input(|i| i.key_pressed(egui::Key::Escape)) {
                    drop_path = true;
                }

                if resp.drag_started()
                    && let Some((pi, _)) = near
                    && let Some(local) = s.puppet.mesh_local_positions(mi)
                {
                    // falloff weights are fixed the moment the grab starts
                    let center = path.points.get(pi).copied().unwrap_or((0.0, 0.0));
                    let center_s = to_screen(center.0, center.1);
                    let mut weights = Vec::new();
                    if let Some(g) = s.puppet.mesh_geometry(mi) {
                        let (pairs, _) = g.positions.as_chunks::<2>();
                        for (vi, xy) in pairs.iter().enumerate() {
                            let d = to_screen(xy[0], xy[1]).distance(center_s);
                            if d < radius {
                                let t = 1.0 - d / radius;
                                weights.push((vi, t * t * (3.0 - 2.0 * t)));
                            }
                        }
                    }
                    path.drag = Some(PathDrag { point: pi, weights, local, mdx: 0.0, mdy: 0.0 });
                }
                if resp.dragged()
                    && let Some(d) = &mut path.drag
                {
                    let delta = resp.drag_delta();
                    d.mdx += delta.x / (scale * c.pixels_per_unit);
                    d.mdy += delta.y / (scale * c.pixels_per_unit);
                }
                if resp.drag_stopped()
                    && let Some(d) = path.drag.take()
                {
                    if let Some(pt) = path.points.get_mut(d.point) {
                        pt.0 += d.mdx;
                        pt.1 += d.mdy;
                    }
                    if d.mdx != 0.0 || d.mdy != 0.0 {
                        // every vertex converts its weighted pull through its
                        // own local jacobian before the edit lands
                        let parent =
                            s.puppet.model().art_meshes.get(mi).and_then(|m| m.parent_deformer);
                        let mut moves: Vec<(usize, f32, f32)> = Vec::new();
                        for &(vi, w) in &d.weights {
                            let at = vi * 2;
                            let px = d.local.get(at).copied().unwrap_or(0.0);
                            let py = d.local.get(at + 1).copied().unwrap_or(0.0);
                            let (ldx, ldy) =
                                parent_delta(&s.puppet, parent, px, py, d.mdx * w, d.mdy * w);
                            moves.push((vi, ldx, ldy));
                        }
                        if s.edit(|m| ayatsuri_core::edit::move_vertices(m, mi, &moves)) {
                            log::debug!("deform path moved {} vertices of mesh {mi}", moves.len());
                        }
                    }
                }

                // draw: the live-shifted wireframe, the falloff ring, the path
                let drag_vi = path.drag.as_ref().map(|d| d.point);
                let shift = path
                    .drag
                    .as_ref()
                    .map(|d| {
                        vec2(d.mdx * c.pixels_per_unit * scale, d.mdy * c.pixels_per_unit * scale)
                    })
                    .unwrap_or(Vec2::ZERO);
                if let Some(g) = s.puppet.mesh_geometry(mi) {
                    let (pairs, _) = g.positions.as_chunks::<2>();
                    let mut pts: Vec<Pos2> =
                        pairs.iter().map(|xy| to_screen(xy[0], xy[1])).collect();
                    if let Some(d) = &path.drag {
                        for &(vi, w) in &d.weights {
                            if let Some(sp) = pts.get_mut(vi) {
                                *sp += shift * w;
                            }
                        }
                    }
                    let stroke = Stroke::new(1.0, theme::pal().accent.gamma_multiply(0.6));
                    let (tris, _) = g.indices.as_chunks::<3>();
                    for &[a, b, cx] in tris {
                        let (Some(&s0), Some(&s1), Some(&s2)) =
                            (pts.get(a as usize), pts.get(b as usize), pts.get(cx as usize))
                        else {
                            continue;
                        };
                        vp.line_segment([s0, s1], stroke);
                        vp.line_segment([s1, s2], stroke);
                        vp.line_segment([s2, s0], stroke);
                    }
                }
                let screen_path: Vec<Pos2> = path
                    .points
                    .iter()
                    .enumerate()
                    .map(|(pi, &(px, py))| {
                        let sp = to_screen(px, py);
                        if drag_vi == Some(pi) { sp + shift } else { sp }
                    })
                    .collect();
                if screen_path.len() > 1 {
                    vp.add(egui::Shape::line(
                        screen_path.clone(),
                        Stroke::new(1.4, theme::pal().warp_green),
                    ));
                }
                let hover_pi = near.map(|(pi, _)| pi);
                for (pi, sp) in screen_path.iter().enumerate() {
                    let hot = drag_vi == Some(pi) || (drag_vi.is_none() && hover_pi == Some(pi));
                    let (r, fill) =
                        if hot { (5.0, theme::pal().accent) } else { (3.4, Color32::WHITE) };
                    vp.circle(*sp, r, fill, Stroke::new(1.2, theme::pal().warp_green));
                    if hot {
                        vp.circle_stroke(
                            *sp,
                            radius,
                            Stroke::new(1.0, theme::pal().accent.gamma_multiply(0.4)),
                        );
                    }
                }
                if hover_pi.is_some() {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                }
            }
            if drop_path {
                self.deform_path = None;
            }
        }

        // mesh edit mode: dim everything, redraw the target mesh on top with
        // a strong wireframe and draggable vertex handles
        if let Some(mi) = self.mesh_edit
            && s.puppet.model().art_meshes.get(mi).is_none()
        {
            self.mesh_edit = None;
            self.mesh_drag = None;
        }
        enum MeshOp {
            Move(usize, usize, f32, f32),
            Add(usize, usize, [f32; 3]),
            Del(usize, usize),
        }
        let mut mesh_op: Option<MeshOp> = None;
        if let Some(mi) = self.mesh_edit {
            vp.rect_filled(view, 0.0, Color32::from_black_alpha(150));
            let ptr = ui.ctx().input(|i| i.pointer.latest_pos());
            let mesh_id = s.puppet.model().art_meshes.get(mi).map(|m| m.id.clone());
            let parent = s.puppet.model().art_meshes.get(mi).and_then(|m| m.parent_deformer);
            let tex_slot =
                s.puppet.model().art_meshes.get(mi).map(|m| m.texture as usize).unwrap_or(0);
            // the palette thumbs are 256px; editing wants the real texture
            let tex_id = s.full_texture(&ui.ctx().clone(), tex_slot).map(|t| t.id());

            if let Some(g) = s.puppet.mesh_geometry(mi) {
                let (pairs, _) = g.positions.as_chunks::<2>();
                let mut screen_pts: Vec<Pos2> =
                    pairs.iter().map(|xy| to_screen(xy[0], xy[1])).collect();

                // the nearest vertex within reach, for hover and drag start
                let mut near: Option<(usize, f32)> = None;
                if let Some(ptr) = ptr {
                    for (vi, sp) in screen_pts.iter().enumerate() {
                        let d = sp.distance(ptr);
                        if d < 8.0 && near.is_none_or(|(_, nd)| d < nd) {
                            near = Some((vi, d));
                        }
                    }
                }

                if !space && self.mesh_mode == 1 && resp.clicked() && near.is_none() {
                    // add: split whichever triangle is under the pointer
                    if let Some(ptr) = resp.interact_pointer_pos() {
                        let (tris, _) = g.indices.as_chunks::<3>();
                        for (ti, &[a, b, cc]) in tris.iter().enumerate() {
                            let (Some(&s0), Some(&s1), Some(&s2)) = (
                                screen_pts.get(a as usize),
                                screen_pts.get(b as usize),
                                screen_pts.get(cc as usize),
                            ) else {
                                continue;
                            };
                            let den = (s1.y - s2.y) * (s0.x - s2.x) + (s2.x - s1.x) * (s0.y - s2.y);
                            if den.abs() < 1e-9 {
                                continue;
                            }
                            let w0 = ((s1.y - s2.y) * (ptr.x - s2.x)
                                + (s2.x - s1.x) * (ptr.y - s2.y))
                                / den;
                            let w1 = ((s2.y - s0.y) * (ptr.x - s2.x)
                                + (s0.x - s2.x) * (ptr.y - s2.y))
                                / den;
                            let w2 = 1.0 - w0 - w1;
                            if w0 >= 0.0 && w1 >= 0.0 && w2 >= 0.0 {
                                mesh_op = Some(MeshOp::Add(mi, ti, [w0, w1, w2]));
                                break;
                            }
                        }
                    }
                }
                if !space
                    && self.mesh_mode == 2
                    && resp.clicked()
                    && let Some((vi, _)) = near
                {
                    mesh_op = Some(MeshOp::Del(mi, vi));
                }
                if !space && self.mesh_mode == 0 {
                    if resp.drag_started()
                        && let Some((vi, _)) = near
                        && let Some(local) = s.puppet.mesh_local_positions(mi)
                    {
                        self.mesh_drag = Some(MeshDrag { vertex: vi, local, dx: 0.0, dy: 0.0 });
                    }
                    if resp.dragged()
                        && let Some(d) = &mut self.mesh_drag
                    {
                        // screen motion to model units, then through the
                        // parent deformer's local jacobian to parent space
                        let delta = resp.drag_delta();
                        let mdx = delta.x / (scale * c.pixels_per_unit);
                        let mdy = delta.y / (scale * c.pixels_per_unit);
                        let at = d.vertex * 2;
                        let px = d.local.get(at).copied().unwrap_or(0.0) + d.dx;
                        let py = d.local.get(at + 1).copied().unwrap_or(0.0) + d.dy;
                        let (ldx, ldy) = parent_delta(&s.puppet, parent, px, py, mdx, mdy);
                        d.dx += ldx;
                        d.dy += ldy;
                    }
                    if resp.drag_stopped()
                        && let Some(d) = self.mesh_drag.take()
                        && (d.dx != 0.0 || d.dy != 0.0)
                    {
                        mesh_op = Some(MeshOp::Move(mi, d.vertex, d.dx, d.dy));
                    }
                }

                // live preview: the dragged vertex follows the pointer
                if let (Some(d), Some(ptr)) = (&self.mesh_drag, ptr)
                    && let Some(sp) = screen_pts.get_mut(d.vertex)
                {
                    *sp = ptr;
                }

                // the mesh itself, textured, above the scrim
                let mut mesh = match tex_id {
                    Some(id) => egui::Mesh::with_texture(id),
                    None => egui::Mesh::default(),
                };
                let (uv_pairs, _) = g.uvs.as_chunks::<2>();
                let mut uvs = uv_pairs.iter();
                for sp in &screen_pts {
                    let uv = match (tex_id, uvs.next()) {
                        (Some(_), Some(&[u, v])) => pos2(u, v),
                        _ => egui::epaint::WHITE_UV,
                    };
                    mesh.vertices.push(egui::epaint::Vertex {
                        pos: *sp,
                        uv,
                        color: Color32::WHITE,
                    });
                }
                mesh.indices.extend(g.indices.iter().map(|&i| i as u32));
                vp.add(egui::Shape::mesh(mesh));

                let stroke = Stroke::new(1.2, theme::pal().accent);
                let (tris, _) = g.indices.as_chunks::<3>();
                for &[a, b, cx] in tris {
                    let (Some(&s0), Some(&s1), Some(&s2)) = (
                        screen_pts.get(a as usize),
                        screen_pts.get(b as usize),
                        screen_pts.get(cx as usize),
                    ) else {
                        continue;
                    };
                    vp.line_segment([s0, s1], stroke);
                    vp.line_segment([s1, s2], stroke);
                    vp.line_segment([s2, s0], stroke);
                }
                let drag_vi = self.mesh_drag.as_ref().map(|d| d.vertex);
                let hover_vi = near.map(|(vi, _)| vi);
                for (vi, sp) in screen_pts.iter().enumerate() {
                    let hot = drag_vi == Some(vi) || (drag_vi.is_none() && hover_vi == Some(vi));
                    let (r, fill) =
                        if hot { (4.0, theme::pal().accent) } else { (2.6, Color32::WHITE) };
                    vp.circle(*sp, r, fill, Stroke::new(1.0, theme::pal().accent));
                }
                if hover_vi.is_some() && !space {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                }
            }

            // a small header bar with the mesh name and the way out
            let bar = Rect::from_min_size(
                pos2(view.center().x - 150.0, view.min.y + 8.0),
                vec2(300.0, 26.0),
            );
            vp.rect_filled(bar, 6.0, theme::pal().chrome);
            vp.rect_stroke(bar, 6.0, Stroke::new(1.0, theme::pal().border), StrokeKind::Outside);
            vp.text(
                pos2(bar.min.x + 10.0, bar.center().y),
                Align2::LEFT_CENTER,
                format!("Editing {}", truncate(mesh_id.as_deref().unwrap_or("mesh"), 24)),
                FontId::proportional(11.5),
                theme::pal().text,
            );
            let done =
                Rect::from_min_size(pos2(bar.max.x - 56.0, bar.min.y + 4.0), vec2(48.0, 18.0));
            let dresp = ui.interact(done, egui::Id::new("mesh-edit-done"), Sense::click());
            vp.rect_filled(done, 4.0, theme::pal().accent);
            vp.text(
                done.center(),
                Align2::CENTER_CENTER,
                "Done",
                FontId::proportional(11.0),
                Color32::WHITE,
            );
            let esc = ui.ctx().input(|i| i.key_pressed(egui::Key::Escape));
            if dresp.clicked() || esc {
                self.mesh_edit = None;
                self.mesh_drag = None;
                if self.tool == Tool::MeshManual {
                    self.tool = Tool::Arrow;
                }
            }
        }
        if let Some(op) = mesh_op {
            let applied = match op {
                MeshOp::Move(mi, vi, dx, dy) => {
                    s.edit(|m| ayatsuri_core::edit::move_vertex(m, mi, vi, dx, dy))
                }
                MeshOp::Add(mi, ti, w) => {
                    s.edit(|m| ayatsuri_core::edit::add_vertex(m, mi, ti, w).is_some())
                }
                MeshOp::Del(mi, vi) => s.edit(|m| ayatsuri_core::edit::delete_vertex(m, mi, vi)),
            };
            if !applied {
                log::warn!("mesh edit had nothing to do");
            }
        }

        // zoom strip
        let strip = Rect::from_min_max(pos2(content.min.x, view.max.y), content.max);
        painter.rect_filled(strip, 0.0, theme::pal().chrome);
        painter.line_segment(
            [strip.left_top(), strip.right_top()],
            Stroke::new(1.0, theme::pal().border),
        );
        let mut sb = ui.new_child(UiBuilder::new().max_rect(strip.shrink2(vec2(8.0, 2.0))));
        sb.horizontal_centered(|ui| {
            ui.label(egui::RichText::new(format!("{:.1} %", scale * 100.0)).size(11.0));
            if ui.small_button("−").clicked() {
                self.view_zoom = (self.view_zoom / 1.25).clamp(0.02, 40.0);
            }
            if ui.small_button("+").clicked() {
                self.view_zoom = (self.view_zoom * 1.25).clamp(0.02, 40.0);
            }
            if ui.small_button("1:1").clicked() {
                self.view_zoom = 1.0 / fit;
                self.view_pan = Vec2::ZERO;
            }
            if ui.small_button("Fit").clicked() {
                self.view_zoom = 1.0;
                self.view_pan = Vec2::ZERO;
            }
        });
    }
}

/// Convert a model-space delta to a point's parent space by probing the
/// parent deformer's local jacobian with finite differences.
fn parent_delta(
    puppet: &ayatsuri_core::Puppet,
    parent: Option<u32>,
    px: f32,
    py: f32,
    mdx: f32,
    mdy: f32,
) -> (f32, f32) {
    let Some(pd) = parent else { return (mdx, mdy) };
    let pd = pd as usize;
    let e = 1e-3;
    let (fx, fy) = puppet.deformer_point(pd, px, py);
    let (ax, ay) = puppet.deformer_point(pd, px + e, py);
    let (bx, by) = puppet.deformer_point(pd, px, py + e);
    let j00 = (ax - fx) / e;
    let j10 = (ay - fy) / e;
    let j01 = (bx - fx) / e;
    let j11 = (by - fy) / e;
    let det = j00 * j11 - j01 * j10;
    if det.abs() < 1e-9 {
        (mdx, mdy)
    } else {
        ((j11 * mdx - j01 * mdy) / det, (j00 * mdy - j10 * mdx) / det)
    }
}

/// Everything a shortcut can be rebound to, with Cubism-flavoured defaults.
const KEY_ACTIONS: [(&str, &str, egui::Modifiers, egui::Key); 7] = [
    ("undo", "Undo", egui::Modifiers::COMMAND, egui::Key::Z),
    ("redo", "Redo", egui::Modifiers::COMMAND.plus(egui::Modifiers::SHIFT), egui::Key::Z),
    ("save", "Save", egui::Modifiers::COMMAND, egui::Key::S),
    ("mesh-edit", "Edit Mesh", egui::Modifiers::COMMAND, egui::Key::E),
    ("reset-params", "Reset Parameters", egui::Modifiers::COMMAND, egui::Key::Num1),
    ("toggle-visible", "Show/Hide Selected", egui::Modifiers::COMMAND, egui::Key::Num3),
    (
        "export-image",
        "Export Image",
        egui::Modifiers::COMMAND.plus(egui::Modifiers::SHIFT),
        egui::Key::E,
    ),
];

fn format_shortcut(sc: &egui::KeyboardShortcut) -> String {
    let mut out = String::new();
    if sc.modifiers.command || sc.modifiers.ctrl {
        out.push_str("Ctrl+");
    }
    if sc.modifiers.shift {
        out.push_str("Shift+");
    }
    if sc.modifiers.alt {
        out.push_str("Alt+");
    }
    out.push_str(sc.logical_key.name());
    out
}

fn parse_shortcut(text: &str) -> Option<egui::KeyboardShortcut> {
    let mut mods = egui::Modifiers::NONE;
    let mut key = None;
    for part in text.split('+') {
        match part {
            "Ctrl" | "Cmd" => mods = mods.plus(egui::Modifiers::COMMAND),
            "Shift" => mods = mods.plus(egui::Modifiers::SHIFT),
            "Alt" => mods = mods.plus(egui::Modifiers::ALT),
            name => key = egui::Key::from_name(name),
        }
    }
    key.map(|k| egui::KeyboardShortcut::new(mods, k))
}

/// A fresh pendulum group with the manual's example-ish defaults.
fn default_physics_group(n: usize) -> ayatsuri_format::model::PhysicsGroup {
    ayatsuri_format::model::PhysicsGroup {
        id: format!("PhysicsSetting{n}"),
        name: format!("Group {n}"),
        inputs: Vec::new(),
        outputs: Vec::new(),
        nodes: vec![
            ayatsuri_format::model::PhysicsNode {
                x: 0.0,
                y: 0.0,
                mobility: 1.0,
                delay: 1.0,
                acceleration: 1.0,
                radius: 0.0,
            },
            ayatsuri_format::model::PhysicsNode {
                x: 0.0,
                y: 10.0,
                mobility: 0.95,
                delay: 0.8,
                acceleration: 1.12,
                radius: 10.0,
            },
        ],
        norm_position: ayatsuri_format::model::PhysicsRange { min: -10.0, default: 0.0, max: 10.0 },
        norm_angle: ayatsuri_format::model::PhysicsRange { min: -10.0, default: 0.0, max: 10.0 },
    }
}

/// Cubism's three stock input presets, by their standard parameter ids.
fn input_preset(which: usize) -> Vec<ayatsuri_format::model::PhysicsInput> {
    use ayatsuri_format::model::{PhysicsInput, PhysicsIo};
    let rows: &[(&str, PhysicsIo, f32)] = match which {
        1 => {
            &[("ParamBodyAngleX", PhysicsIo::X, 60.0), ("ParamBodyAngleZ", PhysicsIo::Angle, 60.0)]
        }
        2 => &[
            ("ParamBodyAngleX", PhysicsIo::X, 40.0),
            ("ParamBodyAngleZ", PhysicsIo::Angle, 40.0),
            ("ParamBreath", PhysicsIo::Angle, 20.0),
        ],
        _ => &[
            ("ParamAngleX", PhysicsIo::X, 60.0),
            ("ParamAngleZ", PhysicsIo::Angle, 60.0),
            ("ParamBodyAngleX", PhysicsIo::X, 40.0),
            ("ParamBodyAngleZ", PhysicsIo::Angle, 40.0),
        ],
    };
    rows.iter()
        .map(|&(id, kind, weight)| PhysicsInput { param: id.into(), weight, kind, reflect: false })
        .collect()
}

/// How far the selected group's chain reaches, for a stable preview scale.
fn rig_groups_reach(rig: &ayatsuri_format::model::PhysicsRig, sel: usize) -> f32 {
    rig.groups
        .get(sel)
        .map(|g| {
            g.nodes.iter().skip(1).map(|n| (n.x * n.x + n.y * n.y).sqrt()).sum::<f32>().max(1.0)
                + 10.0
        })
        .unwrap_or(20.0)
}

fn truncate(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        s.to_string()
    } else {
        s.chars().take(max_chars.saturating_sub(1)).chain(['…']).collect()
    }
}

/// The Cubism parameter slider: a track, hollow circles on every key, and
/// the filled pink handle at the current value. Returns true when dragged.
fn param_slider(
    ui: &mut Ui,
    width: f32,
    value: &mut f32,
    min: f32,
    max: f32,
    default: f32,
    keys: &[f32],
) -> (bool, egui::Response) {
    let (rect, resp) = ui.allocate_exact_size(vec2(width, 16.0), Sense::click_and_drag());
    let painter = ui.painter();
    let pad = 6.0;
    let span = (max - min).max(1e-6);
    let x_of =
        |v: f32| rect.min.x + pad + ((v - min) / span).clamp(0.0, 1.0) * (rect.width() - pad * 2.0);
    let cy = rect.center().y;
    painter.line_segment(
        [pos2(x_of(min), cy), pos2(x_of(max), cy)],
        Stroke::new(1.0, theme::pal().track),
    );
    for &k in keys {
        painter.circle(
            pos2(x_of(k), cy),
            3.0,
            Color32::WHITE,
            Stroke::new(1.0, theme::pal().key_ring),
        );
    }
    // a faint tick at the default, the way Cubism hints the rest value
    let dx = x_of(default);
    painter.line_segment(
        [pos2(dx, cy - 4.0), pos2(dx, cy + 4.0)],
        Stroke::new(1.0, theme::pal().track),
    );
    painter.circle_filled(pos2(x_of(*value), cy), 4.5, theme::pal().accent);

    let mut changed = false;
    if (resp.dragged() || resp.clicked())
        && let Some(p) = resp.interact_pointer_pos()
    {
        let f = ((p.x - rect.min.x - pad) / (rect.width() - pad * 2.0)).clamp(0.0, 1.0);
        *value = min + f * span;
        changed = true;
    }
    (changed, resp)
}

fn paint_eye(ui: &Ui, rect: Rect, own: bool, effective: bool) {
    let icon = if own { egui_phosphor::regular::EYE } else { egui_phosphor::regular::EYE_CLOSED };
    let color = if effective { theme::pal().text } else { theme::pal().text_faint };
    ui.painter().text(
        rect.center(),
        Align2::CENTER_CENTER,
        icon,
        FontId::proportional(12.0),
        color,
    );
}

fn paint_folder(ui: &Ui, at: Pos2) {
    let p = ui.painter();
    let r = Rect::from_min_size(at - vec2(0.0, 4.0), vec2(11.0, 8.0));
    p.rect_filled(r, 1.0, Color32::from_rgb(0xC9, 0xB4, 0x8A));
    p.rect_filled(
        Rect::from_min_size(r.min - vec2(0.0, 2.0), vec2(5.0, 3.0)),
        1.0,
        Color32::from_rgb(0xC9, 0xB4, 0x8A),
    );
}

/// The little green lattice Cubism marks warp deformers with.
fn paint_warp_glyph(ui: &Ui, at: Pos2) {
    let p = ui.painter();
    let r = Rect::from_center_size(at + vec2(5.0, 0.0), vec2(10.0, 10.0));
    let s = Stroke::new(1.0, theme::pal().warp_green);
    p.rect_stroke(r, 0.0, s, StrokeKind::Inside);
    p.line_segment([pos2(r.center().x, r.min.y), pos2(r.center().x, r.max.y)], s);
    p.line_segment([pos2(r.min.x, r.center().y), pos2(r.max.x, r.center().y)], s);
}

/// The red fan Cubism marks rotation deformers with.
fn paint_rotation_glyph(ui: &Ui, at: Pos2) {
    let p = ui.painter();
    let c = at + vec2(5.0, 0.0);
    p.circle_stroke(c, 4.5, Stroke::new(1.3, theme::pal().rotation_red));
    p.line_segment([c, c + vec2(0.0, -4.5)], Stroke::new(1.3, theme::pal().rotation_red));
}

fn point_in_triangle(x: f32, y: f32, a: &[f32; 2], b: &[f32; 2], c: &[f32; 2]) -> bool {
    let sign = |p: &[f32; 2], q: &[f32; 2]| (x - q[0]) * (p[1] - q[1]) - (p[0] - q[0]) * (y - q[1]);
    let d1 = sign(a, b);
    let d2 = sign(b, c);
    let d3 = sign(c, a);
    let neg = d1 < 0.0 || d2 < 0.0 || d3 < 0.0;
    let pos = d1 > 0.0 || d2 > 0.0 || d3 > 0.0;
    !(neg && pos)
}

// a tilted glue bottle with a falling drop, like the classic icon
fn paint_glue_bottle(ui: &Ui, rect: Rect, color: Color32) {
    let p = ui.painter();
    let c = rect.center() + vec2(1.0, -1.0);
    let rot = |v: Vec2| {
        let a = -0.785_f32;
        let (s, co) = a.sin_cos();
        c + vec2(v.x * co - v.y * s, v.x * s + v.y * co)
    };
    // body: a square outline standing on its corner
    let body: Vec<Pos2> = [vec2(-3.2, -7.8), vec2(3.2, -7.8), vec2(3.2, -1.8), vec2(-3.2, -1.8)]
        .into_iter()
        .map(rot)
        .collect();
    p.add(egui::Shape::convex_polygon(body, Color32::TRANSPARENT, Stroke::new(1.4, color)));
    // nozzle
    let nozzle: Vec<Pos2> = [vec2(-2.3, -1.0), vec2(2.3, -1.0), vec2(1.0, 4.2), vec2(-1.0, 4.2)]
        .into_iter()
        .map(rot)
        .collect();
    p.add(egui::Shape::convex_polygon(nozzle, color, Stroke::NONE));
    // the drop
    p.circle_filled(rot(vec2(0.0, 7.2)), 1.5, color);
}

fn point_in_polygon(p: Pos2, poly: &[Pos2]) -> bool {
    let mut inside = false;
    let n = poly.len();
    let mut j = n.saturating_sub(1);
    for i in 0..n {
        let (Some(a), Some(b)) = (poly.get(i), poly.get(j)) else { break };
        if (a.y > p.y) != (b.y > p.y) {
            let x = a.x + (p.y - a.y) / (b.y - a.y) * (b.x - a.x);
            if p.x < x {
                inside = !inside;
            }
        }
        j = i;
    }
    inside
}

fn hit_mesh(puppet: &ayatsuri_core::Puppet, ux: f32, uy: f32) -> Option<usize> {
    let order: Vec<_> = puppet.drawables().map(|d| d.mesh).collect();
    for &mi in order.iter().rev() {
        let Some(g) = puppet.mesh_geometry(mi) else { continue };
        let (pos, _) = g.positions.as_chunks::<2>();
        let (tris, _) = g.indices.as_chunks::<3>();
        let inside = tris.iter().any(|&[a, b, cx]| {
            let (Some(p0), Some(p1), Some(p2)) =
                (pos.get(a as usize), pos.get(b as usize), pos.get(cx as usize))
            else {
                return false;
            };
            point_in_triangle(ux, uy, p0, p1, p2)
        });
        if inside {
            return Some(mi);
        }
    }
    None
}

fn ancestors_shown(s: &Session, mut parent: Option<u32>) -> bool {
    let mut guard = 0;
    while let Some(p) = parent {
        if !s.part_shown.get(p as usize).copied().unwrap_or(true) {
            return false;
        }
        parent = s.puppet.model().parts.get(p as usize).and_then(|q| q.parent_part);
        guard += 1;
        if guard > 64 {
            break;
        }
    }
    true
}

enum ParamAction {
    Select(usize),
    AddSingle(usize, f32),
    DeleteKey(usize),
    OpenEdit(usize),
}

fn parameter_row(
    ui: &mut Ui,
    s: &mut Session,
    pi: usize,
    sel_param: &mut Option<usize>,
    can_edit: bool,
    actions: &mut Vec<ParamAction>,
) {
    let Some((min, max, default, decimals)) =
        s.puppet.model().parameters.get(pi).map(|p| (p.min, p.max, p.default, p.decimals))
    else {
        return;
    };
    let display = s.param_names.get(pi).cloned().unwrap_or_default();
    let keys = s.param_keys.get(pi).cloned().unwrap_or_default();
    let mut v = s.puppet.value(pi);
    ui.horizontal(|ui| {
        let (nr, nresp) = ui.allocate_exact_size(vec2(92.0, 16.0), Sense::click());
        if *sel_param == Some(pi) {
            ui.painter().rect_filled(nr, 2.0, theme::pal().select);
        }
        ui.painter().text(
            nr.left_center(),
            Align2::LEFT_CENTER,
            truncate(&display, 13),
            FontId::proportional(11.5),
            theme::pal().text,
        );
        if let Some(Some([r, g, b])) = s.param_colors.get(pi) {
            ui.painter().rect_filled(
                Rect::from_min_size(nr.min, vec2(3.0, nr.height())),
                1.0,
                Color32::from_rgb(*r, *g, *b),
            );
        }
        if nresp.clicked() {
            actions.push(ParamAction::Select(pi));
        }
        if nresp.double_clicked() {
            v = default;
            s.puppet.set_value(pi, v);
            s.dirty = true;
        }
        nresp.context_menu(|ui| {
            if ui.button("Edit Parameter…").clicked() {
                actions.push(ParamAction::OpenEdit(pi));
                ui.close();
            }
            if ui.button("Reset to default").clicked() {
                s.puppet.set_value(pi, default);
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
                        let (rect, resp) = ui.allocate_exact_size(vec2(16.0, 16.0), Sense::click());
                        ui.painter().rect_filled(rect, 3.0, Color32::from_rgb(c[0], c[1], c[2]));
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
        });
        let slider_w = (ui.available_width() - 68.0).max(40.0);
        let (changed, slider_resp) = param_slider(ui, slider_w, &mut v, min, max, default, &keys);
        if changed {
            s.puppet.set_value(pi, v);
            s.dirty = true;
        }
        if slider_resp.drag_stopped() {
            log::debug!("parameter {display} dragged to {v:.3}");
        }
        slider_resp.context_menu(|ui| {
            if ui.add_enabled(can_edit, egui::Button::new("Add Single Keyform")).clicked() {
                actions.push(ParamAction::AddSingle(pi, v));
                ui.close();
            }
            if ui.add_enabled(can_edit, egui::Button::new("Delete Keyform")).clicked() {
                actions.push(ParamAction::DeleteKey(pi));
                ui.close();
            }
        });
        let dec = decimals.clamp(1, 4) as usize;
        let step = 0.1f32.powi(dec as i32);
        let mut dv = v;
        if ui
            .add_sized(
                [52.0, 16.0],
                egui::DragValue::new(&mut dv)
                    .speed(step)
                    .range(min..=max)
                    .min_decimals(dec)
                    .max_decimals(dec),
            )
            .changed()
        {
            s.puppet.set_value(pi, dv);
            s.dirty = true;
        }
    });
}

// the Cubism two-parameter pad: names left, the square pad centered in the
// slider column, value boxes right, each aligned with its name
fn combined_pad(
    ui: &mut Ui,
    s: &mut Session,
    pa: usize,
    pb: usize,
    sel_param: &mut Option<usize>,
    actions: &mut Vec<ParamAction>,
) {
    let Some((amin, amax, adec)) =
        s.puppet.model().parameters.get(pa).map(|p| (p.min, p.max, p.decimals))
    else {
        return;
    };
    let Some((bmin, bmax, bdec)) =
        s.puppet.model().parameters.get(pb).map(|p| (p.min, p.max, p.decimals))
    else {
        return;
    };
    let name_a = s.param_names.get(pa).cloned().unwrap_or_default();
    let name_b = s.param_names.get(pb).cloned().unwrap_or_default();
    let keys_a = s.param_keys.get(pa).cloned().unwrap_or_default();
    let keys_b = s.param_keys.get(pb).cloned().unwrap_or_default();
    let va = s.puppet.value(pa);
    let vb = s.puppet.value(pb);

    let name_w = 92.0;
    let value_w = 52.0;
    let avail = ui.available_width();
    let slider_w = (avail - name_w - value_w - 18.0).max(60.0);
    let side = 96.0_f32.min(slider_w);
    let (full, _) = ui.allocate_exact_size(vec2(avail, side + 20.0), Sense::hover());
    // the bottom 10 px stay empty so the next row never crowds the pad
    let block = Rect::from_min_max(full.min, pos2(full.max.x, full.max.y - 10.0));
    let cy = block.center().y;

    // names align with the pad's top and bottom edges, like Cubism
    let edge = (side - 6.0) * 0.5 - 8.0;
    for (p, label, dy) in [(pa, &name_a, -edge), (pb, &name_b, edge)] {
        let nr = Rect::from_min_size(pos2(block.min.x, cy + dy - 8.0), vec2(name_w, 16.0));
        let nresp = ui.interact(nr, egui::Id::new(("pad-name", pa, p)), Sense::click());
        if *sel_param == Some(p) {
            ui.painter().rect_filled(nr, 2.0, theme::pal().select);
        }
        ui.painter().text(
            nr.left_center() + vec2(2.0, 0.0),
            Align2::LEFT_CENTER,
            truncate(label, 13),
            FontId::proportional(11.5),
            theme::pal().text,
        );
        if nresp.clicked() {
            actions.push(ParamAction::Select(p));
        }
        if nresp.secondary_clicked() {
            actions.push(ParamAction::OpenEdit(p));
        }
    }

    // the pad, centered in the slider column
    let col_left = block.min.x + name_w + 6.0;
    let pad =
        Rect::from_center_size(pos2(col_left + slider_w * 0.5, cy), vec2(side - 6.0, side - 6.0));
    let resp =
        ui.interact(pad.expand(5.0), egui::Id::new(("pad", pa, pb)), Sense::click_and_drag());
    let painter = ui.painter();
    // dashed border, the way Cubism draws its pads
    for (from, to) in [
        (pad.left_top(), pad.right_top()),
        (pad.right_top(), pad.right_bottom()),
        (pad.right_bottom(), pad.left_bottom()),
        (pad.left_bottom(), pad.left_top()),
    ] {
        painter.add(egui::Shape::dashed_line(
            &[from, to],
            Stroke::new(1.0, theme::pal().key_ring),
            3.0,
            3.0,
        ));
    }
    let xa =
        |v: f32| pad.min.x + ((v - amin) / (amax - amin).max(1e-6)).clamp(0.0, 1.0) * pad.width();
    // the second parameter grows upward, like Cubism's pads
    let yb =
        |v: f32| pad.max.y - ((v - bmin) / (bmax - bmin).max(1e-6)).clamp(0.0, 1.0) * pad.height();
    for k in &keys_a {
        painter.line_segment(
            [pos2(xa(*k), pad.min.y), pos2(xa(*k), pad.max.y)],
            Stroke::new(1.0, theme::pal().track.gamma_multiply(0.5)),
        );
    }
    for k in &keys_b {
        painter.line_segment(
            [pos2(pad.min.x, yb(*k)), pos2(pad.max.x, yb(*k))],
            Stroke::new(1.0, theme::pal().track.gamma_multiply(0.5)),
        );
    }
    for ka in &keys_a {
        for kb in &keys_b {
            painter.circle(
                pos2(xa(*ka), yb(*kb)),
                2.6,
                theme::pal().panel_bg,
                Stroke::new(1.0, theme::pal().key_ring),
            );
        }
    }
    painter.circle_filled(pos2(xa(va), yb(vb)), 4.5, theme::pal().accent);
    if (resp.dragged() || resp.clicked())
        && let Some(ptr) = resp.interact_pointer_pos()
    {
        let fa = ((ptr.x - pad.min.x) / pad.width()).clamp(0.0, 1.0);
        let fb = ((pad.max.y - ptr.y) / pad.height()).clamp(0.0, 1.0);
        s.puppet.set_value(pa, amin + fa * (amax - amin));
        s.puppet.set_value(pb, bmin + fb * (bmax - bmin));
        s.dirty = true;
    }
    resp.context_menu(|ui| {
        if ui.button("Reset both").clicked() {
            let da = s.puppet.model().parameters.get(pa).map(|p| p.default).unwrap_or(0.0);
            let db = s.puppet.model().parameters.get(pb).map(|p| p.default).unwrap_or(0.0);
            s.puppet.set_value(pa, da);
            s.puppet.set_value(pb, db);
            s.dirty = true;
            ui.close();
        }
    });

    // value boxes, each on its name's line
    for (p, v, dec, dy) in [(pa, va, adec, -edge), (pb, vb, bdec, edge)] {
        let vr =
            Rect::from_min_size(pos2(block.max.x - value_w, cy + dy - 8.0), vec2(value_w, 16.0));
        let digits = dec.clamp(1, 4) as usize;
        let step = 0.1f32.powi(digits as i32);
        let mut dv = v;
        if ui
            .put(
                vr,
                egui::DragValue::new(&mut dv).speed(step).min_decimals(digits).max_decimals(digits),
            )
            .changed()
        {
            s.puppet.set_value(p, dv);
            s.dirty = true;
        }
    }
}
