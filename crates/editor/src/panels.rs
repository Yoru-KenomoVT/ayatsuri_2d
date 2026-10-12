use std::path::{Path, PathBuf};

use ayatsuri_format::Model;
use ayatsuri_format::model::DeformerKind;
use egui::{
    Align2, CentralPanel, Color32, FontId, Frame, Panel, Pos2, Rect, ScrollArea, Sense, Stroke,
    StrokeKind, Ui, UiBuilder, Vec2, pos2, vec2,
};

use crate::dock::{self, DockAction, DockNode, PanelKind};
use crate::point_edit;

mod deformer_dialog;
mod param_menu;
mod physics_page;
mod tool_details;
use crate::logging;
use crate::prefs::Prefs;
use crate::session::{Sel, Session};
use crate::theme;
use param_menu::ParamCmd;

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
    auto_preset: usize,
    auto_spacing: f32,
    auto_inner: f32,
    auto_margin: f32,
    warp_cols: u32,
    warp_rows: u32,
}

impl Default for ToolOptions {
    fn default() -> Self {
        Self {
            brush_size: 40.0,
            auto_preset: 0,
            auto_spacing: 24.0,
            auto_inner: 48.0,
            auto_margin: 8.0,
            warp_cols: 5,
            warp_rows: 5,
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
    pub(crate) session: Option<Session>,
    selected: Option<Sel>,
    log_view: log::Level,
    pending_import: Option<PathBuf>,
    /// View zoom relative to fit-to-panel, and pan in screen points.
    view_zoom: f32,
    view_pan: Vec2,
    pub(crate) prefs: Prefs,
    tool: Tool,
    pub(crate) show_about: bool,
    pub(crate) about: crate::windows::about::AboutState,
    pub(crate) show_settings: bool,
    /// The keyboard settings' search text.
    pub(crate) key_filter: String,
    /// A file picker open on its own thread, and a model loading on one.
    picking: Option<std::sync::mpsc::Receiver<BgJob>>,
    loading: Option<(PathBuf, std::sync::mpsc::Receiver<BgJob>)>,

    /// Discord Rich Presence while enabled, and the app id it runs as.
    rpc: Option<crate::discord::Presence>,
    rpc_id: String,

    dev_physics_done: bool,
    /// One-time startup work (fullscreen) has run.
    startup_done: bool,

    pub(crate) show_stats: bool,
    show_physics: bool,
    phys_sel: usize,
    phys_tab: usize,
    /// The physics dialog's working copy; Apply commits it as one edit.
    phys_edit: Option<ayatsuri_format::model::PhysicsRig>,
    phys_auto_max: bool,
    phys_peak_cache: Vec<(usize, f32)>,
    phys_preset: usize,
    phys_zoom: f32,
    phys_page: physics_page::PhysPage,

    /// The dialog preview's own little pendulum: (pos, prev) per node plus
    /// a hash of the settings that built it.
    phys_prev: Vec<(f32, f32, f32, f32)>,
    phys_prev_sig: u64,
    phys_prev_drive: f32,
    /// The preview simulates until this time (seconds), then rests.
    phys_prev_until: f64,

    /// The physics dialog's Valid checkbox: the rig only ticks while on.
    physics_on: bool,
    /// Drive the standard face/body/eye parameters from the cursor.
    follow_cursor: bool,
    follow: Vec2,
    /// Cubism's edit level: 1 meshes, 2 deformers, 3 structure.
    edit_level: u8,
    /// The action waiting for its new shortcut in settings, if any.
    pub(crate) rebind: Option<&'static str>,
    /// Last states the debug log reported, so changes log exactly once.
    logged_sel: Option<Sel>,
    logged_param: Option<usize>,
    logged_tool: Tool,
    heartbeat: f32,
    /// Timeline state: playhead seconds, transport, auto-key, and the time
    /// the tracks were last written into the puppet.
    anim_time: f32,
    anim_playing: bool,
    anim_autokey: bool,
    anim_applied: f32,
    new_anim: Option<AnimNew>,
    pub(crate) export_dlg: Option<ExportDlg>,
    /// Collapsed rows in the Deformer palette, and canvas-locked deformers.
    def_collapsed: std::collections::HashSet<usize>,
    def_locked: std::collections::HashSet<usize>,
    /// Locked meshes: the canvas looks past them and won't edit them.
    mesh_locked: std::collections::HashSet<usize>,

    /// A crash the shell caught this session, shown until dismissed.
    crash: Option<CrashShown>,
    /// When unsaved work was last copied to the autosave file.
    last_autosave: std::time::Instant,
    /// Folded folders in the Parts palette.
    part_collapsed: std::collections::HashSet<usize>,
    new_param: Option<ParamNew>,
    new_folder: Option<String>,
    new_model: Option<ModelNew>,
    pub(crate) show_model_info: bool,
    pub(crate) gpu: bool,
    theme_applied: bool,
    last_layout: DockNode,
    pub(crate) logo: Option<TextureHandleCache>,
    /// The home header's dithered accent wash, keyed by pixel size and
    /// colours so it is only rebuilt on resize or a theme change.
    home_wash: Option<(WashKey, egui::TextureHandle)>,
    show_ws_save: bool,
    ws_name: String,
    show_ws_manage: bool,
    pub(crate) settings_tab: usize,
    /// Where the Settings window sits (its centre) once dragged.
    pub(crate) settings_pos: Option<egui::Pos2>,
    tool_opts: ToolOptions,
    lasso: Vec<Pos2>,
    /// The parameter rows' own selection, independent of object selection.
    sel_param: Option<usize>,
    param_edit: Option<ParamEdit>,
    pmenu: param_menu::ParamMenuState,
    deformer_dialog: Option<deformer_dialog::DeformerDialog>,
    /// Extra meshes picked by the lasso, beyond `selected`.
    multi_sel: Vec<usize>,
    /// A deformer creation requested from inside a borrow (canvas, tool pane).
    pending_create: Option<(bool, Option<(f32, f32)>)>,
    /// Mesh edit mode: the mesh whose vertices are being edited, if any.
    mesh_edit: Option<usize>,
    mesh_drag: Option<MeshDrag>,
    /// 0 move, 1 add vertices, 2 erase vertices, 3 connect points, 4 cut
    /// edges, 5 erase brush.
    mesh_mode: u8,
    /// Points the erase brush has swept over during the current stroke.
    erase_marked: std::collections::BTreeSet<usize>,
    /// How close (screen px) a click must be to pick a point.
    pick_radius: f32,
    /// Auto connect's options: redo every face, and keep faces off empty
    /// texture (Cubism's Mesh reconstruction / Protect edge exterior).
    connect_rebuild: bool,
    connect_protect: bool,
    /// The glue being edited, its weight tool (0 A:B, 1 A, 2 B, 3 re-merge),
    /// brush strength, bind distance in canvas px, and the stroke so far.
    selected_glue: Option<usize>,
    glue_mode: u8,
    glue_strength: f32,
    glue_tolerance: f32,
    glue_stroke: std::collections::HashMap<usize, f32>,
    glue_compat: Option<f32>,
    /// New points close a face onto the nearest edge when on.
    mesh_autoconnect: bool,
    /// The edge tool's first pick.
    edge_first: Option<usize>,
    /// Lines drawn with the edge tool that have not closed a face yet,
    /// per mesh; they turn into faces as soon as a triangle closes.
    pending_edges: std::collections::HashMap<usize, Vec<(u16, u16)>>,
    /// The model as it was when mesh editing began, for Cancel.
    mesh_snapshot: Option<Model>,
    /// Deform paths per (mesh, edit level 2 or 3), as in Cubism.
    deform_paths: std::collections::HashMap<(usize, u8), Vec<PathPin>>,
    path_grab: Option<PathGrab>,
    /// Path width in canvas pixels (the red circle) and the share of it that
    /// keeps its shape fully (the blue circle).
    path_width: f32,
    path_hardness: f32,

    /// Grid point picking and the transform box for the selected warp.
    pts: point_edit::PointState,
    /// Warps edit through Bezier control points (Cubism's default) or the
    /// division points directly.
    warp_bezier: bool,
    /// The arrow tool's display settings (line widths and colours).
    disp: tool_details::DisplayOpts,
    /// Brush selection weights per object: (1, mesh) or (2, warp).
    brush_w: std::collections::HashMap<(u8, usize), Vec<f32>>,
    /// How much one brush pass adds, 0..1.
    brush_weight: f32,
    /// Where the last brush dab landed, for spacing dabs by distance.
    brush_last: Option<Pos2>,

    /// Bezier divisions per warp, columns by rows.
    bezier_div: std::collections::HashMap<usize, (usize, usize)>,
    bez_drag: Option<BezierDrag>,

    rot_drag: Option<RotDrag>,
    /// 1 = quit the app, 2 = close the session, shown while unsaved work
    /// would be lost.
    confirm_close: Option<u8>,
    force_close: bool,
    home_thumbs: std::collections::HashMap<PathBuf, Option<egui::TextureHandle>>,
    /// Home card being renamed: the file and the name typed so far.
    home_rename: Option<(PathBuf, String)>,
    /// One line under the home actions after a file operation fails.
    home_note: Option<String>,
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

/// A Bezier control point of a warp mid-drag: the lattice fitted when the
/// grab began, and the grid it was fitted to (parent space).
struct BezierDrag {
    deformer: usize,
    idx: usize,
    start: Pos2,
    ptr: Pos2,
    lattice: ayatsuri_core::bezier::Lattice,
    local: Vec<f32>,
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

/// A deform path control point, pinned to a triangle of its mesh so the
/// path rides along as the mesh deforms between poses.
#[derive(Clone, Copy)]
struct PathPin {
    tri: usize,
    w: [f32; 3],
}

/// Results coming back from background threads.
enum BgJob {
    Picked(Option<PathBuf>),
    Loaded(PathBuf, Box<Result<Session, String>>),
}

/// A finished path bend: the control point, where it was dropped, each
/// vertex's screen offset, and the parent-space positions at the grab.
type PathFinish = (usize, Pos2, Vec<(usize, Vec2)>, Vec<f32>);

/// A deform path control point being dragged. With the Deform Path tool
/// it only moves the point; with the arrow tool the mesh follows the path.
struct PathGrab {
    mesh: usize,
    point: usize,
    start: Pos2,
    ptr: Pos2,
    /// Present when the drag deforms (arrow tool): ties fixed at the grab.
    bind: Option<point_edit::PathBind>,
    verts: Vec<Pos2>,
    local: Vec<f32>,
}
/// The one-click mesh operations Cubism's Tool Details offers.
#[derive(Clone, Copy, PartialEq)]
enum MeshAction {
    AutoMesh,
    Reset,
    AutoConnect,
    Divide,
    Bind,
    Unbind,
}

/// What the crash dialog shows, plus where the work was rescued to.
struct CrashShown {
    message: String,
    location: String,
    recovery: Option<PathBuf>,
    count: u32,
}

/// The New Animation dialog's working copy.
struct AnimNew {
    name: String,
    fps: f32,
    duration: f32,
    looped: bool,
}

/// The Export as moc3 dialog's working copy.
pub(crate) struct ExportDlg {
    pub(crate) name: String,
    pub(crate) version: usize,
    pub(crate) physics: bool,
    pub(crate) motions: bool,
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

pub(crate) struct TextureHandleCache {
    pub(crate) dark: bool,
    pub(crate) handle: egui::TextureHandle,
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
            about: crate::windows::about::AboutState::default(),
            show_settings: false,
            key_filter: String::new(),
            picking: None,
            loading: None,

            rpc: None,
            rpc_id: String::new(),

            dev_physics_done: false,
            startup_done: false,

            show_stats: false,
            show_physics: false,
            phys_sel: 0,
            phys_tab: 0,
            phys_edit: None,
            phys_auto_max: true,
            phys_peak_cache: Vec::new(),
            phys_preset: 0,
            phys_zoom: 1.0,
            phys_page: physics_page::PhysPage::default(),

            phys_prev: Vec::new(),
            phys_prev_sig: 0,
            phys_prev_drive: 0.0,
            phys_prev_until: 0.0,

            physics_on: true,
            follow_cursor: false,
            follow: Vec2::ZERO,
            edit_level: 2,
            rebind: None,
            logged_sel: None,
            logged_param: None,
            logged_tool: Tool::Arrow,
            heartbeat: 0.0,
            anim_time: 0.0,
            anim_playing: false,
            anim_autokey: true,
            anim_applied: f32::NAN,
            new_anim: None,
            export_dlg: None,
            def_collapsed: std::collections::HashSet::new(),
            def_locked: std::collections::HashSet::new(),
            mesh_locked: std::collections::HashSet::new(),

            part_collapsed: std::collections::HashSet::new(),
            crash: None,
            last_autosave: std::time::Instant::now(),
            new_param: None,
            new_folder: None,
            new_model: None,
            show_model_info: false,
            gpu: false,
            theme_applied: false,
            logo: None,
            home_wash: None,
            show_ws_save: false,
            ws_name: String::new(),
            show_ws_manage: false,
            settings_tab: 0,
            settings_pos: None,
            tool_opts: ToolOptions::default(),
            lasso: Vec::new(),
            sel_param: None,
            param_edit: None,
            pmenu: param_menu::ParamMenuState::default(),
            deformer_dialog: None,
            multi_sel: Vec::new(),
            pending_create: None,
            mesh_edit: None,
            mesh_drag: None,
            mesh_mode: 0,
            mesh_autoconnect: true,
            erase_marked: std::collections::BTreeSet::new(),
            pick_radius: 8.0,
            connect_rebuild: true,
            connect_protect: true,
            selected_glue: None,
            glue_mode: 0,
            glue_strength: 0.3,
            glue_tolerance: 4.0,
            glue_stroke: std::collections::HashMap::new(),
            glue_compat: None,
            edge_first: None,
            pending_edges: std::collections::HashMap::new(),
            mesh_snapshot: None,
            deform_paths: std::collections::HashMap::new(),
            path_grab: None,
            path_width: 80.0,
            path_hardness: 0.5,

            pts: point_edit::PointState::default(),
            warp_bezier: true,
            disp: tool_details::DisplayOpts::default(),
            brush_w: std::collections::HashMap::new(),
            brush_weight: 0.11,
            brush_last: None,

            bezier_div: std::collections::HashMap::new(),
            bez_drag: None,

            rot_drag: None,
            confirm_close: None,
            force_close: false,
            home_thumbs: std::collections::HashMap::new(),
            home_rename: None,
            home_note: None,
        }
    }
}

impl EditorUi {
    pub fn new() -> Self {
        Self::default()
    }

    /// The app shell calls this after catching a panic out of `ui`: the
    /// work gets rescued to a recovery file and the next frames show what
    /// happened instead of the editor vanishing.
    pub fn on_crash(&mut self) {
        let info = logging::take_crash();
        // stop whatever was mid-flight; it is the likeliest culprit
        self.mesh_drag = None;
        self.pts.clear();
        self.rot_drag = None;
        self.anim_playing = false;
        let recovery = self.session.as_ref().and_then(|s| s.write_recovery());
        match &recovery {
            Some(p) => log::error!("work saved to {} after the crash", p.display()),
            None if self.session.is_some() => log::error!("could not write a recovery copy"),
            None => {}
        }
        let count = self.crash.as_ref().map_or(0, |c| c.count) + 1;
        self.crash = Some(CrashShown {
            message: info
                .as_ref()
                .map(|c| c.message.clone())
                .unwrap_or_else(|| "unknown error".into()),
            location: info.map(|c| c.location).unwrap_or_default(),
            recovery,
            count,
        });
    }

    fn crash_window(&mut self, ui: &mut Ui) {
        let Some(c) = &self.crash else { return };
        let mut keep = true;
        let mut copy = false;
        let mut open_dir = false;
        egui::Window::new("Something went wrong")
            .collapsible(false)
            .resizable(false)
            .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
            .show(ui.ctx(), |ui| {
                ui.set_width(440.0);
                ui.label(
                    egui::RichText::new(format!("{} {}", egui_phosphor::regular::WARNING, c.message))
                        .color(theme::pal().accent)
                        .strong(),
                );
                if !c.location.is_empty() {
                    ui.weak(format!("at {}", c.location));
                }
                ui.add_space(6.0);
                ui.label("The editor caught the error and kept running. The step that failed was undone where possible.");
                match &c.recovery {
                    Some(p) => {
                        ui.label(format!("Your model was saved to {}", p.display()));
                    }
                    None => {
                        ui.weak("No model was open, nothing needed rescuing.");
                    }
                }
                if let Some(p) = logging::log_path() {
                    ui.weak(format!("Full details, with a backtrace, are in {}", p.display()));
                }
                if c.count > 1 {
                    ui.weak(format!("This has happened {} times this session.", c.count));
                }
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button("Copy details").clicked() {
                        copy = true;
                    }
                    if ui.button("Open log folder").clicked() {
                        open_dir = true;
                    }
                    if ui.button(egui::RichText::new("Keep working").strong()).clicked() {
                        keep = false;
                    }
                });
            });
        if copy {
            let tail = logging::lines(log::Level::Trace);
            let start = tail.len().saturating_sub(80);
            let report = format!(
                "Ayatsuri2D {} on {} {}\n{} at {}\n\n{}",
                env!("CARGO_PKG_VERSION"),
                std::env::consts::OS,
                std::env::consts::ARCH,
                c.message,
                c.location,
                tail.get(start..).unwrap_or(&[]).join("\n")
            );
            ui.ctx().copy_text(report);
        }
        if open_dir
            && let Some(dir) = logging::log_path().and_then(|p| p.parent().map(|d| d.to_path_buf()))
        {
            // the platform's file browser; failure here is harmless
            let opener = if cfg!(target_os = "macos") {
                "open"
            } else if cfg!(target_os = "windows") {
                "explorer"
            } else {
                "xdg-open"
            };
            let _ = std::process::Command::new(opener).arg(dir).spawn();
        }
        if !keep {
            log::debug!("crash dialog dismissed, editing continues");
            self.crash = None;
        }
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

    /// Open a model or artwork on a worker thread. Decoding a PSD and its
    /// texture pages takes seconds; on the UI thread that froze the window
    /// long enough for the desktop to call it "not responding". The window
    /// keeps drawing (a loading overlay) and picks the result up when ready.
    fn import(&mut self, ctx: &egui::Context, path: PathBuf) {
        if self.loading.is_some() {
            log::warn!("still opening the previous file");
            return;
        }
        let (tx, rx) = std::sync::mpsc::channel();
        let ctx2 = ctx.clone();
        let p2 = path.clone();
        let spawned = std::thread::Builder::new().name("model-load".into()).spawn(move || {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                Session::load(&ctx2, &p2)
            }))
            .unwrap_or_else(|_| Err("the loader crashed on this file".into()));
            let _ = tx.send(BgJob::Loaded(p2, Box::new(result)));
            ctx2.request_repaint();
        });
        match spawned {
            Ok(_) => {
                log::info!("opening {}", path.display());
                self.loading = Some((path, rx));
            }
            Err(e) => log::error!("open failed: could not start the loader ({e})"),
        }
    }

    /// The Open dialog runs on its own thread too: the system file picker
    /// blocks until closed, and on the UI thread that stalls the window.
    fn import_dialog(&mut self, ctx: &egui::Context) {
        if self.picking.is_some() {
            return;
        }
        let (tx, rx) = std::sync::mpsc::channel();
        let ctx2 = ctx.clone();
        let spawned = std::thread::Builder::new().name("file-picker".into()).spawn(move || {
            let picked = rfd::FileDialog::new()
                .add_filter("All supported", &["aom", "moc3", "json", "psd", "kra"])
                .add_filter("Ayatsuri / Live2D model", &["aom", "moc3", "json"])
                .add_filter("Artwork (Photoshop, Krita)", &["psd", "kra"])
                .set_title("Open model or artwork")
                .pick_file();
            let _ = tx.send(BgJob::Picked(picked));
            ctx2.request_repaint();
        });
        match spawned {
            Ok(_) => self.picking = Some(rx),
            Err(e) => log::error!("could not open the file picker: {e}"),
        }
    }

    /// Collect finished background work: a picked file starts loading, a
    /// loaded model becomes the session.
    fn poll_background(&mut self, ctx: &egui::Context) {
        if let Some(rx) = &self.picking {
            match rx.try_recv() {
                Ok(BgJob::Picked(Some(path))) => {
                    self.picking = None;
                    self.import(ctx, path);
                }
                Ok(_) | Err(std::sync::mpsc::TryRecvError::Disconnected) => self.picking = None,
                Err(std::sync::mpsc::TryRecvError::Empty) => {}
            }
        }
        let done = match &self.loading {
            Some((_, rx)) => match rx.try_recv() {
                Ok(BgJob::Loaded(path, result)) => Some(Some((path, *result))),
                Ok(_) | Err(std::sync::mpsc::TryRecvError::Disconnected) => Some(None),
                Err(std::sync::mpsc::TryRecvError::Empty) => None,
            },
            None => None,
        };
        match done {
            Some(Some((path, Ok(s)))) => {
                self.loading = None;
                self.prefs.remember(path);
                self.session = Some(s);
                self.selected = None;
                self.view_zoom = 1.0;
                self.view_pan = Vec2::ZERO;
            }
            Some(Some((_, Err(e)))) => {
                self.loading = None;
                log::error!("open failed: {e}");
            }
            Some(None) => {
                self.loading = None;
                log::error!("open failed: the loader stopped");
            }
            None => {}
        }
    }

    /// While a model loads, a small card over everything says so and the
    /// window keeps redrawing.
    fn loading_overlay(&mut self, ui: &mut Ui) {
        let Some((path, _)) = &self.loading else { return };
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        egui::Area::new(egui::Id::new("loading-card"))
            .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
            .order(egui::Order::Foreground)
            .show(ui.ctx(), |ui| {
                egui::Frame::popup(ui.style()).inner_margin(egui::Margin::same(16)).show(
                    ui,
                    |ui| {
                        ui.horizontal(|ui| {
                            ui.add(egui::Spinner::new().size(18.0));
                            ui.label(format!("Opening {name}…"));
                        });
                    },
                );
            });
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(100));
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
        // overlay colours: saved choices, else the theme, so mesh lines follow
        // a primary colour changed in Settings
        self.disp = tool_details::DisplayOpts::from_prefs(&self.prefs.overlay);
        // the edit level decides how warps are edited, as in Cubism: level 1
        // division points only, 2 the Bezier control points, 3 a 1x1 Bezier
        // on the outer frame
        self.warp_bezier = self.edit_level >= 2;
        self.update_presence();
        self.poll_background(&ui.ctx().clone());
        self.loading_overlay(ui);
        // whatever slider was held last frame is kept off physics this frame,
        // so a physics-driven parameter can be dragged by hand
        if let Some(s) = &mut self.session {
            let held = std::mem::take(&mut s.params_held);
            s.puppet.hold_physics_params(&held);
        }
        if !self.startup_done {
            self.startup_done = true;
            if self.prefs.start_fullscreen {
                ui.ctx().send_viewport_cmd(egui::ViewportCommand::Fullscreen(true));
            }
            // developer hook for screenshots: AYATSURI_SHOW_SETTINGS=1 (page)
            if let Some(page) = std::env::var_os("AYATSURI_SHOW_SETTINGS") {
                self.show_settings = true;
                self.settings_tab = page.to_string_lossy().parse().unwrap_or(0);
            }
            // developer hook for screenshots: AYATSURI_SHOW_ABOUT=credits
            if let Some(tab) = std::env::var_os("AYATSURI_SHOW_ABOUT") {
                self.show_about = true;
                self.about.open_tab(&tab.to_string_lossy());
            }
        }
        // developer hook for screenshots: AYATSURI_SHOW_PHYSICS=1 opens the
        // physics page once, as soon as a model is loaded
        if !self.dev_physics_done
            && self.session.is_some()
            && std::env::var_os("AYATSURI_SHOW_PHYSICS").is_some()
        {
            self.dev_physics_done = true;
            self.show_physics = true;
        }
        // a crash caught outside this frame (the GPU callback) surfaces here
        if logging::has_crash() {
            self.on_crash();
        }
        self.crash_window(ui);
        // render-time crashes skip the rescue save, so unsaved work also
        // goes to an autosave file every couple of minutes
        if self.last_autosave.elapsed().as_secs() >= 120 {
            self.last_autosave = std::time::Instant::now();
            if let Some(s) = &self.session
                && s.modified
            {
                s.autosave();
            }
        }
        // follow the desktop's light/dark switch while asked to
        if self.prefs.follow_system
            && let Some(sys) = ui.ctx().system_theme()
        {
            let want = sys == egui::Theme::Dark;
            if want != theme::is_dark() {
                theme::set_dark(&ui.ctx().clone(), want);
            }
        }
        if !self.theme_applied {
            let ctx = ui.ctx().clone();
            if let Some([r, g, b]) = self.prefs.accent {
                theme::set_accent(&ctx, Some(Color32::from_rgb(r, g, b)));
            }
            theme::set_look(&ctx, self.prefs.look());
            theme::set_dark(&ctx, self.prefs.dark.unwrap_or(true));
            if let Some(z) = self.prefs.ui_scale {
                ctx.set_zoom_factor(z.clamp(0.5, 3.0));
            }
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
                    Some(Sel::Mesh(i)) => format!("mesh {i} ({})", s.mesh_name(i)),
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
            if self.tool != Tool::Glue {
                self.selected_glue = None;
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
        // shortcuts: the more modifiers a binding has, the earlier it gets a
        // look, so Ctrl+Shift+Z is taken before Ctrl+Z and Ctrl+1 before 1.
        // Plain-key bindings stay quiet while a text field has the keyboard,
        // and everything waits while Settings is capturing a new binding.
        if self.rebind.is_none() {
            let typing = ui.ctx().egui_wants_keyboard_input();
            let weight = |m: egui::Modifiers| {
                usize::from(m.command || m.ctrl) + usize::from(m.shift) + usize::from(m.alt)
            };
            let mut fired: Vec<&'static str> = Vec::new();
            let mut bound: Vec<(&'static str, egui::KeyboardShortcut)> =
                KEY_ACTIONS.iter().map(|a| (a.1, self.bound(a.1))).collect();
            bound.sort_by_key(|(_, sc)| std::cmp::Reverse(weight(sc.modifiers)));
            for (id, sc) in bound {
                if typing && weight(sc.modifiers) == 0 {
                    continue;
                }
                if ui.ctx().input_mut(|i| i.consume_shortcut(&sc)) {
                    fired.push(id);
                }
            }
            for id in fired {
                log::debug!("shortcut: {id}");
                self.run_key_action(&ui.ctx().clone(), id);
            }
        }
        if let Some(gi) = self.selected_glue
            && self.mesh_edit.is_none()
            && !ui.ctx().egui_wants_keyboard_input()
            && ui
                .ctx()
                .input(|i| i.key_pressed(egui::Key::Delete) || i.key_pressed(egui::Key::Backspace))
            && let Some(s) = &mut self.session
            && s.edit(|m| ayatsuri_core::edit::unbind_glue(m, gi))
        {
            log::info!("glue {gi} deleted");
            self.selected_glue = None;
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
                    if ui
                        .add(egui::Button::new("New…").shortcut_text(self.sc("new-model")))
                        .clicked()
                    {
                        ui.close();
                        self.new_model = Some(ModelNew::default());
                    }
                    if ui
                        .add_enabled(self.session.is_some(), egui::Button::new("New Animation…"))
                        .clicked()
                    {
                        ui.close();
                        self.new_anim = Some(AnimNew {
                            name: "idle".into(),
                            fps: 30.0,
                            duration: 3.0,
                            looped: true,
                        });
                    }
                    if ui.add(egui::Button::new("Open…").shortcut_text(self.sc("open"))).clicked()
                    {
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
                    if ui
                        .add_enabled(
                            self.session.is_some(),
                            egui::Button::new("Save").shortcut_text(self.sc("save")),
                        )
                        .clicked()
                    {
                        ui.close();
                        self.save_session(false);
                    }
                    if ui
                        .add_enabled(
                            self.session.is_some(),
                            egui::Button::new("Save As…").shortcut_text(self.sc("save-as")),
                        )
                        .clicked()
                    {
                        ui.close();
                        self.save_session(true);
                    }
                    ui.separator();
                    if ui
                        .add_enabled(
                            self.session.is_some(),
                            egui::Button::new("Export Image…")
                                .shortcut_text(self.sc("export-image")),
                        )
                        .clicked()
                    {
                        ui.close();
                        self.export_image();
                    }
                    if ui
                        .add_enabled(
                            self.session.is_some(),
                            egui::Button::new("Export as moc3…")
                                .shortcut_text(self.sc("export-moc3")),
                        )
                        .clicked()
                    {
                        ui.close();
                        let name =
                            self.session.as_ref().map(|s| s.name.clone()).unwrap_or_default();
                        self.export_dlg = Some(ExportDlg {
                            name,
                            version: ayatsuri_import::ExportVersion::ALL.len() - 1,
                            physics: true,
                            motions: true,
                        });
                    }
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
                    if ui
                        .add_enabled(
                            self.session.is_some(),
                            egui::Button::new("Close").shortcut_text(self.sc("close")),
                        )
                        .clicked()
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
                                egui::Button::new("Reset to Default")
                                    .shortcut_text(self.sc("reset-params")),
                            )
                            .clicked()
                        {
                            ui.close();
                            self.reset_parameters();
                        }
                        ui.separator();
                        if ui
                            .add_enabled(
                                self.session.is_some(),
                                egui::Button::new("Automatic Generation of Four Corner Forms…")
                                    .shortcut_text(self.sc("four-corners")),
                            )
                            .clicked()
                        {
                            ui.close();
                            self.run_param_cmd(ParamCmd::FourCorners);
                        }
                        if ui
                            .add_enabled(
                                self.session.is_some() && self.sel_param.is_some(),
                                egui::Button::new("Motion Mirroring…"),
                            )
                            .on_disabled_hover_text("Select a parameter first")
                            .clicked()
                            && let Some(pi) = self.sel_param
                        {
                            ui.close();
                            self.run_param_cmd(ParamCmd::Mirror(pi));
                        }
                    });
                    let mesh_sel = matches!(self.selected, Some(Sel::Mesh(_)));
                    ui.menu_button("ArtMesh", |ui| {
                        if ui
                            .add_enabled(
                                mesh_sel,
                                egui::Button::new("Edit Mesh").shortcut_text(self.sc("mesh-edit")),
                            )
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
                        if ui
                            .add_enabled(
                                self.session.is_some(),
                                egui::Button::new("Bind Glue").shortcut_text(self.sc("glue-bind")),
                            )
                            .clicked()
                        {
                            ui.close();
                            self.run_mesh_action(MeshAction::Bind);
                        }
                        if ui
                            .add_enabled(
                                self.session.is_some(),
                                egui::Button::new("Unbind Glue")
                                    .shortcut_text(self.sc("glue-unbind")),
                            )
                            .clicked()
                        {
                            ui.close();
                            self.run_mesh_action(MeshAction::Unbind);
                        }
                    });
                    ui.menu_button("Skinning", |ui| {
                        let _ = ui.add_enabled(false, egui::Button::new("Apply Skinning…"));
                    });
                    ui.menu_button("Edit Form", |ui| {
                        let flippable = self.edit_target().is_some();
                        for (label, horizontal) in
                            [("Flip Horizontally", true), ("Flip Vertically", false)]
                        {
                            if ui.add_enabled(flippable, egui::Button::new(label)).clicked() {
                                ui.close();
                                self.flip_selection(horizontal);
                            }
                        }
                        if ui
                            .add_enabled(
                                self.session.is_some(),
                                egui::Button::new("Automatic Generation of Four Corner Forms…"),
                            )
                            .clicked()
                        {
                            ui.close();
                            self.run_param_cmd(ParamCmd::FourCorners);
                        }
                    });
                    let _ = ui.add_enabled(false, egui::Button::new("Temporary Deform Tool"));
                    ui.separator();
                    if ui
                        .add_enabled(
                            !self.selection_meshes().is_empty(),
                            egui::Button::new("Show/Hide Selected")
                                .shortcut_text(self.sc("toggle-visible")),
                        )
                        .clicked()
                    {
                        ui.close();
                        self.toggle_selected_visibility();
                    }
                    if ui
                        .add_enabled(
                            self.session.is_some(),
                            egui::Button::new("Lock/Unlock Selected")
                                .shortcut_text(self.sc("lock-selected")),
                        )
                        .clicked()
                    {
                        ui.close();
                        self.toggle_selected_lock();
                    }
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
                    ui.separator();
                    if ui
                        .add_enabled(self.session.is_some(), egui::Button::new("New Animation…"))
                        .clicked()
                    {
                        ui.close();
                        self.new_anim = Some(AnimNew {
                            name: "idle".into(),
                            fps: 30.0,
                            duration: 3.0,
                            looped: true,
                        });
                    }
                    let has_anim = self.session.as_ref().is_some_and(|s| s.current_anim.is_some());
                    if ui
                        .add_enabled(
                            has_anim,
                            egui::Button::new(if self.anim_playing { "Pause" } else { "Play" }),
                        )
                        .clicked()
                    {
                        ui.close();
                        self.anim_playing = !self.anim_playing;
                    }
                    ui.checkbox(&mut self.anim_autokey, "Auto key");
                });
                ui.menu_button("Window", |ui| {
                    // one checkbox per palette; the menu stays open so several
                    // can be toggled in one go
                    for p in PanelKind::ALL {
                        let was = self.layout.contains(p);
                        let mut open = was;
                        if ui.checkbox(&mut open, p.title()).changed() {
                            if was {
                                self.layout.close(p);
                            } else {
                                self.layout.open_near(p, PanelKind::Viewport);
                            }
                            self.save_layout();
                        }
                    }
                    ui.separator();
                    if ui.button("Settings…").clicked() {
                        self.show_settings = true;
                        ui.close();
                    }
                    ui.separator();
                    ui.menu_button("Workspace", |ui| {
                        if ui.button("Save as New Workspace…").clicked() {
                            self.show_ws_save = true;
                            ui.close();
                        }
                        if ui.button("Manage Workspaces…").clicked() {
                            self.show_ws_manage = true;
                            ui.close();
                        }
                        ui.separator();
                        if ui.button("Export Current Layout…").clicked() {
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
                        if ui.button("Import Layout…").clicked() {
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
                    if ui.button("Reset Current Layout").clicked() {
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

        // the animation clip drives parameters first, so follow and physics
        // layer on top of it the way the reference player stacks them
        if let Some(s) = &mut self.session
            && let Some(ai) = s.current_anim
            && let Some(anim) = s.animations.get(ai).cloned()
        {
            let dt = ui.ctx().input(|i| i.stable_dt).clamp(0.0, 0.1);
            if self.anim_playing {
                self.anim_time += dt;
                if self.anim_time > anim.duration {
                    if anim.looped && anim.duration > 0.0 {
                        self.anim_time %= anim.duration;
                    } else {
                        self.anim_time = anim.duration;
                        self.anim_playing = false;
                    }
                }
                ui.ctx().request_repaint();
            }
            let moved = self.anim_time != self.anim_applied;
            if moved || self.anim_playing {
                for t in &anim.tracks {
                    if let (Some(v), Some(pi)) = (
                        ayatsuri_core::anim::sample(t, self.anim_time),
                        s.puppet.model().parameters.iter().position(|p| p.id == t.param),
                    ) {
                        s.puppet.set_value(pi, v);
                    }
                }
                self.anim_applied = self.anim_time;
                s.dirty = true;
            } else if self.anim_autokey && !self.follow_cursor {
                // auto key: whatever the user moved since the playhead
                // stopped becomes a key at this time; physics outputs are
                // left alone while the rig is writing them
                let physics_out: std::collections::HashSet<String> = if self.physics_on {
                    s.puppet
                        .model()
                        .physics
                        .as_ref()
                        .map(|r| {
                            r.groups
                                .iter()
                                .flat_map(|g| g.outputs.iter().map(|o| o.param.clone()))
                                .collect()
                        })
                        .unwrap_or_default()
                } else {
                    std::collections::HashSet::new()
                };
                let mut keyed = Vec::new();
                for (pi, p) in s.puppet.model().parameters.iter().enumerate() {
                    if physics_out.contains(&p.id) {
                        continue;
                    }
                    let reference = anim
                        .tracks
                        .iter()
                        .find(|t| t.param == p.id)
                        .and_then(|t| ayatsuri_core::anim::sample(t, self.anim_time))
                        .unwrap_or(p.default);
                    let v = s.puppet.value(pi);
                    let eps = 0.1f32.powi(p.decimals.clamp(0, 6)) * 0.5;
                    if (v - reference).abs() > eps {
                        keyed.push((p.id.clone(), v));
                    }
                }
                if !keyed.is_empty()
                    && !ui.ctx().input(|i| i.pointer.any_down())
                    && let Some(a) = s.animations.get_mut(ai)
                {
                    let fps_a = a.fps;
                    for (id, v) in &keyed {
                        if let Some(t) = ayatsuri_core::anim::track_mut(a, id) {
                            ayatsuri_core::anim::set_key(t, self.anim_time, *v, fps_a);
                        }
                    }
                    s.modified = true;
                    log::debug!("auto key: {} parameter(s) at {:.3}s", keyed.len(), self.anim_time);
                }
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
        } else if self.show_physics && self.session.is_some() {
            CentralPanel::default().frame(Frame::NONE.fill(theme::pal().chrome)).show(ui, |ui| {
                self.physics_page(ui);
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
            self.open_deformer_dialog(warp, origin);
        }
        self.deformer_dialog_window(ui);

        self.about_window(ui);
        self.settings_window(ui);
        self.stats_window(ui);
        self.mesh_edit_window(ui);
        self.param_new_window(ui);
        self.folder_new_window(ui);
        self.model_new_window(ui);
        self.model_info_window(ui);
        self.new_anim_window(ui);
        self.export_window(ui);
        self.close_confirm_window(ui);
        self.workspace_windows(ui);
        self.param_edit_window(ui);
        self.param_menu_windows(ui);
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
        let mut dialog_action: Option<MeshAction> = None;
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
                        (
                            3,
                            egui_phosphor::regular::LINE_SEGMENT,
                            "Connect two points (click one, then the other)",
                        ),
                        (4, egui_phosphor::regular::SCISSORS, "Cut the edge between two points"),
                        (
                            5,
                            egui_phosphor::regular::PAINT_BRUSH_BROAD,
                            "Erase brush: drag over points to remove them ([ and ] resize)",
                        ),
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
                if self.mesh_mode == 5 {
                    ui.horizontal(|ui| {
                        ui.weak("Brush");
                        ui.add(
                            egui::Slider::new(&mut self.tool_opts.brush_size, 4.0..=400.0)
                                .logarithmic(true)
                                .suffix(" px"),
                        );
                    });
                }
                ui.horizontal(|ui| {
                    if ui
                        .button("Auto connect")
                        .on_hover_text("Triangulate the points you placed")
                        .clicked()
                    {
                        dialog_action = Some(MeshAction::AutoConnect);
                    }
                    if ui.button("4 division").clicked() {
                        dialog_action = Some(MeshAction::Divide);
                    }
                    if ui
                        .button("Reset")
                        .on_hover_text("Back to the mesh as the file had it")
                        .clicked()
                    {
                        dialog_action = Some(MeshAction::Reset);
                    }
                });
                ui.checkbox(&mut self.mesh_autoconnect, "Auto connect new points")
                    .on_hover_text("Off: added points stay loose until joined with the edge tool");
                ui.separator();
                ui.label("Auto mesh");
                ui.horizontal(|ui| {
                    for (i, (name, outline, inner, margin)) in Self::AUTO_PRESETS.iter().enumerate()
                    {
                        if ui.selectable_label(self.tool_opts.auto_preset == i, *name).clicked() {
                            self.tool_opts.auto_preset = i;
                            self.tool_opts.auto_spacing = *outline;
                            self.tool_opts.auto_inner = *inner;
                            self.tool_opts.auto_margin = *margin;
                        }
                    }
                });
                ui.horizontal(|ui| {
                    ui.weak("Outline");
                    ui.add(
                        egui::DragValue::new(&mut self.tool_opts.auto_spacing)
                            .range(4.0..=256.0)
                            .suffix(" px"),
                    );
                    ui.weak("Inner");
                    ui.add(
                        egui::DragValue::new(&mut self.tool_opts.auto_inner)
                            .range(4.0..=512.0)
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
        if let Some(a) = dialog_action {
            self.run_mesh_action(a);
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
            self.edge_first = None;
            self.pending_edges.clear();
            if self.tool == Tool::MeshManual {
                self.tool = Tool::Arrow;
            }
        }
    }

    /// The Physics page's editor column: FPS and group rows, Input/Output
    /// tabs, then the pendulum table beside the preview. Everything edits a
    /// working copy that Apply commits as one undoable change.
    /// Leaving the Physics page re-enables every group, the way the
    /// reference resets its Valid flags when the window closes.
    fn close_physics_page(&mut self) {
        self.show_physics = false;
        if let Some(s) = &mut self.session {
            let n = s.puppet.model().physics.as_ref().map(|r| r.groups.len()).unwrap_or(0);
            for gi in 0..n {
                s.puppet.set_physics_group_enabled(gi, true);
            }
        }
    }

    /// The page's live parameter rail: drive anything while physics runs,
    /// with the reference's Reset Parameter to Default button up top.
    fn physics_param_rail(&mut self, ui: &mut Ui) {
        if ui.button("Reset Parameter to Default").clicked() {
            self.reset_parameters();
        }
        ui.separator();
        // the Parameter palette's own list, folders and pads included, but
        // only for moving values: no link buttons, menus or add/delete
        let h = ui.available_height().max(40.0);
        let actions = self.param_list(ui, "phys-params", false, h);
        for a in actions {
            if let ParamAction::Select(pi) = a {
                self.sel_param = Some(pi);
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
        size: Vec2,
    ) {
        // sized by its panel, which the user drags; never by itself
        let (prect, presp) = ui.allocate_exact_size(size, Sense::click_and_drag());
        let pp = ui.painter_at(prect);
        let ink = theme::pal().text_faint;
        let node_col = theme::pal().rotation_red;
        pp.rect_filled(prect, 2.0, theme::pal().canvas_bg);
        pp.rect_stroke(prect, 2.0, Stroke::new(1.0, theme::pal().border), StrokeKind::Inside);
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
        // the preview only moves when pushed: it rests on its rest pose until a
        // drag (or Swing) wakes it, and settles back a few seconds after
        let now = ui.input(|i| i.time);
        if presp.drag_started() || presp.dragged() {
            self.phys_prev_until = now + 4.0;
        }
        let live = now < self.phys_prev_until;
        if sig != self.phys_prev_sig || self.phys_prev.len() != nodes.len() || !live {
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
        // the chain hangs from near the top and fills most of the pane
        let sc_units = (prect.height() * 0.78) / reach.max(1.0) * self.phys_zoom;
        if presp.dragged() {
            self.phys_prev_drive += presp.drag_delta().x / sc_units.max(1e-3);
        } else {
            self.phys_prev_drive *= 0.9;
        }

        if live {
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
        }

        let cx = prect.center().x;
        let cy = prect.min.y + prect.height() * 0.1;
        // the preview can be turned in 45 degree steps; it changes only the view
        let (rs, rc) = self.phys_page.prev_rot.to_radians().sin_cos();
        let map = |x: f32, y: f32| {
            let (rx, ry) = (x * rc - y * rs, x * rs + y * rc);
            pos2(cx + rx * sc_units, cy + ry * sc_units)
        };
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
                pp.circle_filled(sp, 4.0, node_col);
                pp.circle_stroke(sp, 4.0, Stroke::new(1.0, theme::pal().canvas_bg));
                pp.text(
                    sp + vec2(8.0, 0.0),
                    Align2::LEFT_CENTER,
                    i.to_string(),
                    FontId::proportional(9.0),
                    ink,
                );
            }
            prev = Some(sp);
        }
        pp.text(
            pos2(prect.min.x + 6.0, prect.min.y + 4.0),
            Align2::LEFT_TOP,
            format!("Scale: {:.1}\nAngle: {:.1}", self.phys_zoom * 10.0, -self.phys_page.prev_rot),
            FontId::proportional(9.5),
            ink,
        );
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 3.0;
            if ui.small_button("1:1").on_hover_text("Full scale").clicked() {
                self.phys_zoom = 1.0;
            }
            if ui
                .small_button(egui_phosphor::regular::CORNERS_OUT)
                .on_hover_text("All: fit the whole pendulum")
                .clicked()
            {
                self.phys_zoom = 1.0;
                self.phys_page.prev_rot = 0.0;
            }
            if ui
                .small_button(egui_phosphor::regular::ARROW_COUNTER_CLOCKWISE)
                .on_hover_text("Rotate left 45°")
                .clicked()
            {
                self.phys_page.prev_rot -= 45.0;
            }
            if ui
                .small_button(egui_phosphor::regular::ARROW_CLOCKWISE)
                .on_hover_text("Rotate right 45°")
                .clicked()
            {
                self.phys_page.prev_rot += 45.0;
            }
            if ui.small_button("Swing").on_hover_text("Give the pendulum a push").clicked() {
                self.phys_prev_drive = reach * 0.4;
                self.phys_prev_until = ui.input(|i| i.time) + 4.0;
            }
        });
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
        // a quiet accent wash fading across the header, Adobe style. The
        // whole fade spans only a dozen 8-bit shades, so drawn directly each
        // shade shows as a flat stripe; baking it with per-pixel noise lets
        // the eye average neighbouring shades into a smooth ramp
        let ppp = ui.ctx().pixels_per_point();
        let key = WashKey {
            w: (band.width() * ppp).round().max(1.0) as u32,
            h: (band.height() * ppp).round().max(1.0) as u32,
            base: theme::pal().header.to_array(),
            accent: theme::pal().accent.to_array(),
        };
        if self.home_wash.as_ref().is_none_or(|(k, _)| *k != key) {
            let img = dithered_wash(&key);
            let tex = ui.ctx().load_texture("home-wash", img, egui::TextureOptions::NEAREST);
            self.home_wash = Some((key, tex));
        }
        if let Some((_, tex)) = &self.home_wash {
            ui.painter().image(
                tex.id(),
                band,
                Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                Color32::WHITE,
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
        if let Some(note) = &self.home_note {
            ui.add_space(6.0);
            ui.colored_label(theme::pal().rotation_red, note);
        }
        ui.add_space(16.0);

        if self.prefs.recents.is_empty() {
            ui.weak("Models you open appear here.");
            return;
        }
        ui.label(egui::RichText::new("Recent").size(13.0).strong());
        ui.add_space(8.0);
        let recents: Vec<PathBuf> = self.prefs.recents.iter().take(8).cloned().collect();
        let mut open_this: Option<PathBuf> = None;
        let mut card_cmd: Option<HomeCmd> = None;
        ScrollArea::vertical().id_salt("home-recents").show(&mut ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                for r in &recents {
                    let card_w = 150.0;
                    let card_h = 230.0;
                    let (card, resp) = ui.allocate_exact_size(vec2(card_w, card_h), Sense::click());
                    let exists = r.exists();
                    resp.context_menu(|ui| home_card_menu(ui, r, exists, &mut card_cmd));
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
                    let name = card_name(r);
                    ui.painter().text(
                        pos2(card.center().x, card.max.y - 20.0),
                        Align2::CENTER_CENTER,
                        truncate(&name, 16),
                        FontId::proportional(12.0),
                        theme::pal().text,
                    );
                    if !exists {
                        // moved or deleted since: say so, the menu offers removal
                        ui.painter().rect_filled(
                            card,
                            6.0,
                            theme::pal().chrome.gamma_multiply(0.7),
                        );
                        ui.painter().text(
                            card.center(),
                            Align2::CENTER_CENTER,
                            "File not found",
                            FontId::proportional(12.0),
                            theme::pal().text_faint,
                        );
                    }
                    let resp = resp.on_hover_text(r.display().to_string());
                    if resp.clicked() && exists {
                        open_this = Some(r.clone());
                    }
                }
            });
        });
        if let Some(path) = open_this {
            self.import(&ui.ctx().clone(), path);
        }
        if let Some(cmd) = card_cmd {
            self.run_home_cmd(&ui.ctx().clone(), cmd);
        }
        self.home_rename_window(&ui.ctx().clone());
    }

    fn run_home_cmd(&mut self, ctx: &egui::Context, cmd: HomeCmd) {
        self.home_note = None;
        match cmd {
            HomeCmd::Open(p) => self.import(ctx, p),
            HomeCmd::OpenNewWindow(p) => {
                let spawned = std::env::current_exe()
                    .and_then(|exe| std::process::Command::new(exe).arg(&p).spawn());
                if let Err(e) = spawned {
                    self.home_note = Some(format!("Could not open a new window: {e}"));
                }
            }
            HomeCmd::ShowInFolder(p) => {
                if let Err(e) = show_in_folder(&p) {
                    self.home_note = Some(format!("Could not open the folder: {e}"));
                }
            }
            HomeCmd::CopyPath(p) => ctx.copy_text(p.display().to_string()),
            HomeCmd::Rename(p) => {
                let name = card_name(&p);
                self.home_rename = Some((p, name));
            }
            HomeCmd::Duplicate(p) => match duplicate_file(&p) {
                Ok(copy) => {
                    // a copy is new work: list it right after the original
                    let at = self.prefs.recents.iter().position(|r| r == &p).map_or(0, |i| i + 1);
                    self.prefs.recents.insert(at.min(self.prefs.recents.len()), copy);
                    self.prefs.recents.truncate(8);
                    self.prefs.save();
                }
                Err(e) => self.home_note = Some(format!("Could not duplicate: {e}")),
            },
            HomeCmd::Remove(p) => {
                self.prefs.recents.retain(|r| r != &p);
                self.home_thumbs.remove(&p);
                self.prefs.save();
            }
            HomeCmd::RemoveMissing => {
                self.prefs.recents.retain(|r| r.exists());
                self.prefs.save();
            }
            HomeCmd::ClearAll => {
                self.prefs.recents.clear();
                self.home_thumbs.clear();
                self.prefs.save();
            }
        }
    }

    /// Rename a recent file on disk, keeping its extension (all of it, so
    /// `.model3.json` stays whole), its place in the list and its preview.
    fn home_rename_window(&mut self, ctx: &egui::Context) {
        let Some((path, mut name)) = self.home_rename.take() else { return };
        let mut keep = true;
        let mut commit = false;
        egui::Window::new("Rename")
            .collapsible(false)
            .resizable(false)
            .pivot(Align2::CENTER_CENTER)
            .default_pos(ctx.content_rect().center())
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    let edit = ui.add(egui::TextEdit::singleline(&mut name).desired_width(220.0));
                    if !edit.has_focus() && !edit.lost_focus() {
                        edit.request_focus();
                    }
                    if edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                        commit = true;
                    }
                    ui.weak(full_extension(&path));
                });
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    if ui.button("Rename").clicked() {
                        commit = true;
                    }
                    if ui.button("Cancel").clicked() {
                        keep = false;
                    }
                });
            });
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            keep = false;
        }
        if commit {
            match rename_file(&path, name.trim()) {
                Ok(new) => {
                    for r in &mut self.prefs.recents {
                        if *r == path {
                            *r = new.clone();
                        }
                    }
                    self.home_thumbs.remove(&path);
                    self.prefs.save();
                }
                Err(e) => self.home_note = Some(format!("Could not rename: {e}")),
            }
            keep = false;
        }
        if keep {
            self.home_rename = Some((path, name));
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
    pub(crate) fn logo_texture(&mut self, ctx: &egui::Context) -> Option<egui::TextureId> {
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
            PanelKind::Timeline => self.timeline_panel(ui),
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

    // ---- Timeline ------------------------------------------------------------

    fn timeline_panel(&mut self, ui: &mut Ui) {
        let Some(s) = &mut self.session else {
            ui.weak("No model loaded.");
            return;
        };
        if s.animations.is_empty() {
            ui.horizontal(|ui| {
                ui.weak("No animation yet.");
                if ui.button("New Animation…").clicked() {
                    self.new_anim = Some(AnimNew {
                        name: "idle".into(),
                        fps: 30.0,
                        duration: 3.0,
                        looped: true,
                    });
                }
            });
            return;
        }
        if s.current_anim.is_none_or(|i| i >= s.animations.len()) {
            s.current_anim = Some(0);
        }
        let ai = s.current_anim.unwrap_or(0);

        // transport row
        let mut seek: Option<f32> = None;
        let (fps, duration) = s
            .animations
            .get(ai)
            .map(|a| (a.fps.max(1.0), a.duration.max(0.0)))
            .unwrap_or((30.0, 0.0));
        let frame = 1.0 / fps;
        ui.horizontal(|ui| {
            let current = s.animations.get(ai).map(|a| a.name.clone()).unwrap_or_default();
            egui::ComboBox::from_id_salt("anim-pick")
                .selected_text(truncate(&current, 14))
                .width(110.0)
                .show_ui(ui, |ui| {
                    for (i, a) in s.animations.iter().enumerate() {
                        if ui.selectable_label(i == ai, &a.name).clicked() {
                            s.current_anim = Some(i);
                            self.anim_applied = f32::NAN;
                        }
                    }
                });
            if ui.button(egui_phosphor::regular::SKIP_BACK).on_hover_text("To start").clicked() {
                seek = Some(0.0);
            }
            if ui.button(egui_phosphor::regular::MINUS).on_hover_text("Previous frame").clicked() {
                seek = Some((self.anim_time - frame).max(0.0));
            }
            let play_icon = if self.anim_playing {
                egui_phosphor::regular::PAUSE
            } else {
                egui_phosphor::regular::PLAY
            };
            if ui.button(play_icon).clicked() {
                self.anim_playing = !self.anim_playing;
            }
            if ui.button(egui_phosphor::regular::PLUS).on_hover_text("Next frame").clicked() {
                seek = Some((self.anim_time + frame).min(duration));
            }
            if ui.button(egui_phosphor::regular::SKIP_FORWARD).on_hover_text("To end").clicked() {
                seek = Some(duration);
            }
            ui.label(format!(
                "{:>3} / {:.0}",
                (self.anim_time * fps).round() as i64,
                (duration * fps).round()
            ));
            ui.separator();
            ui.checkbox(&mut self.anim_autokey, "Auto key");
            if let Some(a) = s.animations.get_mut(ai) {
                ui.checkbox(&mut a.looped, "Loop");
                ui.weak("Len");
                if ui
                    .add(
                        egui::DragValue::new(&mut a.duration)
                            .range(0.1..=600.0)
                            .speed(0.05)
                            .suffix("s"),
                    )
                    .changed()
                {
                    s.modified = true;
                }
            }
        });
        ui.separator();

        // dope sheet: ruler on top, one row per track, diamonds on keys
        let names: Vec<(String, String)> = s
            .animations
            .get(ai)
            .map(|a| {
                a.tracks
                    .iter()
                    .map(|t| {
                        let label = s
                            .puppet
                            .model()
                            .parameters
                            .iter()
                            .position(|p| p.id == t.param)
                            .and_then(|pi| s.param_names.get(pi).cloned())
                            .unwrap_or_else(|| t.param.clone());
                        (t.param.clone(), label)
                    })
                    .collect()
            })
            .unwrap_or_default();
        let label_w = 110.0;
        let avail = ui.available_width();
        let lane_w = (avail - label_w - 8.0).max(60.0);
        let x_of = |t: f32, left: f32| left + label_w + (t / duration.max(1e-3)) * lane_w;

        let (ruler, rresp) = ui.allocate_exact_size(vec2(avail, 18.0), Sense::click_and_drag());
        let p = ui.painter();
        p.rect_filled(ruler, 0.0, theme::pal().header);
        let frames = (duration * fps).round().max(1.0) as i64;
        let step = ((frames as f32 / (lane_w / 40.0)).ceil() as i64).max(1);
        let mut f = 0i64;
        while f <= frames {
            let x = x_of(f as f32 / fps, ruler.min.x);
            p.line_segment(
                [pos2(x, ruler.max.y - 5.0), pos2(x, ruler.max.y)],
                Stroke::new(1.0, theme::pal().text_faint),
            );
            p.text(
                pos2(x + 2.0, ruler.min.y + 2.0),
                Align2::LEFT_TOP,
                f.to_string(),
                FontId::proportional(9.0),
                theme::pal().text_faint,
            );
            f += step;
        }
        if (rresp.dragged() || rresp.clicked())
            && let Some(ptr) = rresp.interact_pointer_pos()
        {
            let t = ((ptr.x - ruler.min.x - label_w) / lane_w * duration).clamp(0.0, duration);
            // snap to whole frames, like every dope sheet
            seek = Some((t * fps).round() / fps);
            self.anim_playing = false;
        }

        let mut delete: Option<(String, f32)> = None;
        let rows_top = ui.cursor().min.y;
        ScrollArea::vertical().id_salt("anim-rows").show(ui, |ui| {
            if names.is_empty() {
                ui.weak(
                    "Move a parameter to record a key (Auto key), or press K on a selected row.",
                );
            }
            for (param, label) in &names {
                let (row, _) = ui.allocate_exact_size(vec2(avail, 16.0), Sense::hover());
                let p = ui.painter();
                p.text(
                    pos2(row.min.x + 4.0, row.center().y),
                    Align2::LEFT_CENTER,
                    truncate(label, 15),
                    FontId::proportional(10.5),
                    theme::pal().text,
                );
                p.line_segment(
                    [pos2(row.min.x + label_w, row.max.y), pos2(row.max.x, row.max.y)],
                    Stroke::new(0.5, theme::pal().border),
                );
                let points = s
                    .animations
                    .get(ai)
                    .and_then(|a| a.tracks.iter().find(|t| &t.param == param))
                    .map(|t| t.points.clone())
                    .unwrap_or_default();
                for (t, _) in points {
                    let c = pos2(x_of(t, row.min.x), row.center().y);
                    let r = Rect::from_center_size(c, vec2(9.0, 9.0));
                    let kresp = ui.interact(
                        r,
                        egui::Id::new(("key", param, (t * 1000.0) as i64)),
                        Sense::click(),
                    );
                    let col = if kresp.hovered() { theme::pal().accent } else { theme::pal().text };
                    ui.painter().add(egui::Shape::convex_polygon(
                        vec![
                            c + vec2(0.0, -4.0),
                            c + vec2(4.0, 0.0),
                            c + vec2(0.0, 4.0),
                            c + vec2(-4.0, 0.0),
                        ],
                        col,
                        Stroke::NONE,
                    ));
                    if kresp.clicked() {
                        seek = Some(t);
                        self.anim_playing = false;
                    }
                    kresp.context_menu(|ui| {
                        if ui.button("Delete key").clicked() {
                            delete = Some((param.clone(), t));
                            ui.close();
                        }
                    });
                }
            }
        });
        // the playhead runs through ruler and rows
        let px = x_of(self.anim_time, ruler.min.x);
        ui.painter().line_segment(
            [pos2(px, ruler.min.y), pos2(px, ui.cursor().min.y.max(rows_top + 16.0))],
            Stroke::new(1.5, theme::pal().accent),
        );

        if let Some((param, t)) = delete
            && let Some(a) = s.animations.get_mut(ai)
        {
            let fps_a = a.fps;
            if let Some(tr) = a.tracks.iter_mut().find(|tr| tr.param == param) {
                ayatsuri_core::anim::remove_key(tr, t, fps_a);
            }
            a.tracks.retain(|tr| !tr.points.is_empty());
            s.modified = true;
            self.anim_applied = f32::NAN;
        }
        // K keys the selected parameter row by hand
        if ui.ctx().input(|i| i.key_pressed(egui::Key::K))
            && !ui.ctx().egui_wants_keyboard_input()
            && let Some(pi) = self.sel_param
            && let Some(id) = s.puppet.model().parameters.get(pi).map(|p| p.id.clone())
        {
            let v = s.puppet.value(pi);
            if let Some(a) = s.animations.get_mut(ai) {
                let fps_a = a.fps;
                if let Some(tr) = ayatsuri_core::anim::track_mut(a, &id) {
                    ayatsuri_core::anim::set_key(tr, self.anim_time, v, fps_a);
                }
                s.modified = true;
            }
        }
        if let Some(t) = seek {
            self.anim_time = t;
        }
    }

    fn new_anim_window(&mut self, ui: &mut Ui) {
        let Some(new) = &mut self.new_anim else { return };
        let mut open = true;
        let mut create = false;
        egui::Window::new("New Animation")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .show(ui.ctx(), |ui| {
                egui::Grid::new("anew").num_columns(2).spacing([10.0, 6.0]).show(ui, |ui| {
                    ui.label("Name");
                    ui.text_edit_singleline(&mut new.name);
                    ui.end_row();
                    ui.label("FPS");
                    ui.add(egui::DragValue::new(&mut new.fps).range(1.0..=120.0));
                    ui.end_row();
                    ui.label("Length");
                    ui.add(
                        egui::DragValue::new(&mut new.duration)
                            .range(0.1..=600.0)
                            .speed(0.05)
                            .suffix(" s"),
                    );
                    ui.end_row();
                    ui.label("Loop");
                    ui.checkbox(&mut new.looped, "");
                    ui.end_row();
                });
                ui.weak("Saved inside the .aom; exported as motions/<name>.motion3.json.");
                if ui.button("Create").clicked() {
                    create = true;
                }
            });
        if create {
            if let (Some(n), Some(s)) = (self.new_anim.take(), &mut self.session) {
                let name = if n.name.trim().is_empty() { "motion".into() } else { n.name };
                s.animations.push(ayatsuri_format::Animation {
                    name: name.clone(),
                    fps: n.fps,
                    duration: n.duration,
                    looped: n.looped,
                    tracks: Vec::new(),
                });
                s.current_anim = Some(s.animations.len() - 1);
                s.modified = true;
                self.anim_time = 0.0;
                self.anim_applied = f32::NAN;
                if !self.layout.contains(PanelKind::Timeline) {
                    self.layout.open_near(PanelKind::Timeline, PanelKind::Viewport);
                }
                log::info!("animation {name} created");
            }
        } else if !open {
            self.new_anim = None;
        }
    }
    // ---- Parameter palette -------------------------------------------------

    fn edit_target(&self) -> Option<(ayatsuri_core::edit::Target, String)> {
        let s = self.session.as_ref()?;
        match self.selected? {
            Sel::Mesh(i) => Some((ayatsuri_core::edit::Target::ArtMesh(i), s.mesh_name(i))),
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
    pub(crate) fn bound(&self, action: &str) -> egui::KeyboardShortcut {
        if let Some((_, text)) = self.prefs.keybinds.iter().find(|(a, _)| a == action)
            && let Some(sc) = parse_shortcut(text)
        {
            return sc;
        }
        KEY_ACTIONS
            .iter()
            .find(|(_, a, ..)| *a == action)
            .map(|(_, _, _, m, k)| egui::KeyboardShortcut::new(*m, *k))
            .unwrap_or(egui::KeyboardShortcut::new(egui::Modifiers::NONE, egui::Key::F35))
    }

    /// Rebuild the mesh as a regular grid over its texture alpha, the way
    /// Cubism's auto mesh presets do. Spacing and margin are texture pixels.
    /// The meshes a mesh action works on: the one being edited, else the
    /// selection.
    fn action_targets(&self) -> Vec<usize> {
        match self.mesh_edit {
            Some(mi) => vec![mi],
            None => self.selection_meshes(),
        }
    }

    fn run_mesh_action(&mut self, action: MeshAction) {
        let targets = self.action_targets();
        match action {
            MeshAction::AutoMesh => self.auto_mesh_many(&targets),
            MeshAction::Reset => {
                let Some(s) = &mut self.session else { return };
                let originals: Vec<(usize, Vec<f32>, Vec<u16>)> = targets
                    .iter()
                    .filter_map(|&mi| {
                        s.original_meshes.get(mi).map(|(u, i)| (mi, u.clone(), i.clone()))
                    })
                    .collect();
                if s.edit(|m| {
                    let mut any = false;
                    for (mi, uvs, idx) in &originals {
                        any |= ayatsuri_core::edit::remesh(m, *mi, uvs, idx);
                    }
                    any
                }) {
                    self.pending_edges.clear();
                    log::info!("mesh reset to its original form on {} mesh(es)", originals.len());
                }
            }
            MeshAction::AutoConnect => {
                let rebuild = self.connect_rebuild;
                let protect = self.connect_protect;
                let Some(s) = &mut self.session else { return };
                // the texture pages the targets use, for the edge protection
                let pages: Vec<(usize, Option<ayatsuri_render::TexData>)> = targets
                    .iter()
                    .map(|&mi| {
                        let tex = s.puppet.model().art_meshes.get(mi).map(|m| m.texture as usize);
                        (mi, tex.and_then(|t| s.preview_tex.get(t).cloned()))
                    })
                    .collect();
                let mut added = 0;
                s.edit(|m| {
                    for (mi, page) in &pages {
                        let keep = |u: f32, v: f32| -> bool {
                            if !protect {
                                return true;
                            }
                            let Some(t) = page else { return true };
                            let x =
                                ((u * t.width as f32) as i64).clamp(0, t.width as i64 - 1) as usize;
                            let y = ((v * t.height as f32) as i64).clamp(0, t.height as i64 - 1)
                                as usize;
                            t.rgba.get((y * t.width as usize + x) * 4 + 3).copied().unwrap_or(255)
                                > 8
                        };
                        added += ayatsuri_core::edit::auto_connect(m, *mi, rebuild, &keep);
                    }
                    added > 0
                });
                for &mi in &targets {
                    self.pending_edges.remove(&mi);
                }
                log::info!("auto connect: {added} face(s) on {} mesh(es)", targets.len());
            }
            MeshAction::Divide => {
                let Some(s) = &mut self.session else { return };
                if s.edit(|m| {
                    let mut any = false;
                    for &mi in &targets {
                        any |= ayatsuri_core::edit::subdivide4(m, mi);
                    }
                    any
                }) {
                    log::info!("mesh 4 division on {} mesh(es)", targets.len());
                }
            }
            MeshAction::Bind => {
                let sel = self.selection_meshes();
                let (Some(&a), Some(&b)) = (sel.first(), sel.get(1)) else {
                    log::warn!("bind glue: select the two meshes to glue (lasso or Ctrl+click)");
                    return;
                };
                let tol_px = self.glue_tolerance;
                let Some(s) = &mut self.session else { return };
                let ppu = s.puppet.model().canvas.pixels_per_unit.max(1e-3);
                let (Some(wa), Some(wb)) = (
                    s.puppet.mesh_geometry(a).map(|g| g.positions.to_vec()),
                    s.puppet.mesh_geometry(b).map(|g| g.positions.to_vec()),
                ) else {
                    return;
                };
                let mut result = None;
                s.edit(|m| {
                    result = ayatsuri_core::edit::bind_glue(m, a, b, &wa, &wb, tol_px / ppu);
                    result.is_some()
                });
                match result {
                    Some((gi, n)) => {
                        log::info!("glue {gi}: bound {n} overlapping point pair(s)");
                        self.selected_glue = Some(gi);
                        self.tool = Tool::Glue;
                    }
                    None => log::warn!(
                        "bind glue: no points of the two meshes overlap within {tol_px} px; add points at the same spots in both, or raise the bind distance"
                    ),
                }
            }
            MeshAction::Unbind => {
                let sel = self.selection_meshes();
                let Some(s) = &mut self.session else { return };
                let doomed: Vec<usize> = match self.selected_glue {
                    Some(gi) => vec![gi],
                    None => s
                        .puppet
                        .model()
                        .glues
                        .iter()
                        .enumerate()
                        .filter(|(_, g)| {
                            sel.contains(&(g.art_mesh_a as usize))
                                && sel.contains(&(g.art_mesh_b as usize))
                        })
                        .map(|(i, _)| i)
                        .collect(),
                };
                if doomed.is_empty() {
                    log::warn!("unbind glue: no glue between the selected meshes");
                    return;
                }
                if s.edit(|m| {
                    // highest first, so earlier indices stay put
                    let mut any = false;
                    for &gi in doomed.iter().rev() {
                        any |= ayatsuri_core::edit::unbind_glue(m, gi);
                    }
                    any
                }) {
                    log::info!("unbound {} glue(s)", doomed.len());
                    self.selected_glue = None;
                }
            }
        }
    }

    /// Cubism's auto mesh presets: outline spacing, inner spacing, margin.
    const AUTO_PRESETS: [(&'static str, f32, f32, f32); 3] =
        [("Standard", 24.0, 48.0, 8.0), ("Deform", 14.0, 28.0, 6.0), ("Light", 40.0, 80.0, 10.0)];

    /// One mesh's new triangulation from its texture's alpha, or why not.
    fn auto_mesh_data(&self, mi: usize) -> Result<(Vec<f32>, Vec<u16>), String> {
        let o = &self.tool_opts;
        let params = ayatsuri_core::automesh::Params {
            outline_spacing: o.auto_spacing.clamp(4.0, 256.0),
            inner_spacing: o.auto_inner.clamp(4.0, 512.0),
            margin: o.auto_margin.clamp(0.0, 128.0),
            threshold: 8,
        };
        let s = self.session.as_ref().ok_or("no model open")?;
        let am = s.puppet.model().art_meshes.get(mi).ok_or("mesh is gone")?;
        let png = s.texture_pngs.get(am.texture as usize).ok_or("mesh has no texture")?;
        let img = image::load_from_memory(png)
            .map_err(|e| format!("texture decode failed: {e}"))?
            .to_rgba8();
        let (u0, v0, u1, v1) = am
            .uvs
            .as_chunks::<2>()
            .0
            .iter()
            .fold((f32::MAX, f32::MAX, f32::MIN, f32::MIN), |(a, b, c, d), &[u, v]| {
                (a.min(u), b.min(v), c.max(u), d.max(v))
            });
        if u0 > u1 || v0 > v1 {
            return Err("mesh has no extent".into());
        }
        let started = std::time::Instant::now();
        let alpha_at = |x: u32, y: u32| img.get_pixel_checked(x, y).map_or(0, |p| p.0[3]);
        let out = ayatsuri_core::automesh::for_patch(
            &alpha_at,
            (img.width(), img.height()),
            (u0, v0, u1, v1),
            &params,
        )
        .ok_or("no opaque pixels under this mesh")?;
        log::debug!(
            "auto mesh {mi}: {} points, {} triangles in {:.0} ms",
            out.0.len() / 2,
            out.1.len() / 3,
            started.elapsed().as_secs_f32() * 1000.0
        );
        Ok(out)
    }

    /// Re-mesh every given mesh from its artwork, as a single undo step.
    fn auto_mesh_many(&mut self, meshes: &[usize]) {
        let mut jobs = Vec::new();
        for &mi in meshes {
            match self.auto_mesh_data(mi) {
                Ok(job) => jobs.push((mi, job)),
                Err(e) => log::warn!("auto mesh skipped mesh {mi}: {e}"),
            }
        }
        if jobs.is_empty() {
            return;
        }
        let Some(s) = &mut self.session else { return };
        let (verts, tris) =
            jobs.iter().fold((0, 0), |(v, t), (_, (uv, ix))| (v + uv.len() / 2, t + ix.len() / 3));
        if s.edit(|m| {
            let mut any = false;
            for (mi, (uvs, idx)) in &jobs {
                any |= ayatsuri_core::edit::remesh(m, *mi, uvs, idx);
            }
            any
        }) {
            log::info!("auto mesh: {} mesh(es), {verts} vertices, {tris} triangles", jobs.len());
        }
    }

    fn auto_mesh(&mut self, mi: usize) {
        self.auto_mesh_many(&[mi]);
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
                    s.param_colors.push(None);
                    // a model with no folders yet (any fresh import) gets a
                    // top-level group, or the parameter would exist unseen
                    if s.groups.is_empty() {
                        s.groups.push(crate::session::ParamGroup {
                            name: String::new(),
                            params: Vec::new(),
                            open: true,
                            color: None,
                        });
                    }
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
                s.groups.push(crate::session::ParamGroup {
                    name,
                    params: Vec::new(),
                    open: true,
                    color: None,
                });
            }
        } else if !open {
            self.new_folder = None;
        }
    }

    fn close_session(&mut self) {
        self.session = None;
        self.mesh_locked.clear();
        self.def_locked.clear();
        self.selected = None;
        self.multi_sel.clear();
        self.sel_param = None;
        self.mesh_edit = None;
        self.mesh_drag = None;
        self.mesh_snapshot = None;
        self.deform_paths.clear();
        self.path_grab = None;
        self.pts.clear();
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

    fn run_key_action(&mut self, ctx: &egui::Context, id: &str) {
        let have = self.session.is_some();
        match id {
            "new-model" => self.new_model = Some(ModelNew::default()),
            "open" => self.import_dialog(ctx),
            "save" if have => self.save_session(false),
            "save-as" if have => self.save_session(true),
            "close" if have => {
                if self.session.as_ref().is_some_and(|s| s.modified) {
                    self.confirm_close = Some(2);
                } else {
                    self.close_session();
                }
            }
            "export-image" if have => self.export_image(),
            "export-moc3" if have => {
                let name = self.session.as_ref().map(|s| s.name.clone()).unwrap_or_default();
                self.export_dlg = Some(ExportDlg {
                    name,
                    version: ayatsuri_import::ExportVersion::ALL.len() - 1,
                    physics: true,
                    motions: true,
                });
            }
            "shortcuts" => {
                self.show_settings = true;
                self.settings_tab = 4;
            }
            "undo" => {
                if let Some(s) = &mut self.session {
                    s.undo();
                }
            }
            "redo" | "redo-alt" => {
                if let Some(s) = &mut self.session {
                    s.redo();
                }
            }
            "select-parent" => self.select_parent(),
            "mesh-edit" => {
                if self.mesh_edit.is_some() {
                    self.mesh_edit = None;
                    self.mesh_drag = None;
                } else if let Some(Sel::Mesh(mi)) = self.selected {
                    self.mesh_edit = Some(mi);
                    self.mesh_snapshot = self.session.as_ref().map(|s| s.puppet.model().clone());
                }
            }
            "auto-mesh" if have => self.run_mesh_action(MeshAction::AutoMesh),
            "auto-connect" if have => self.run_mesh_action(MeshAction::AutoConnect),
            "glue-bind" if have => self.run_mesh_action(MeshAction::Bind),
            "glue-unbind" if have => self.run_mesh_action(MeshAction::Unbind),
            "reset-params" => self.reset_parameters(),
            "four-corners" if have => self.run_param_cmd(ParamCmd::FourCorners),
            "toggle-visible" => self.toggle_selected_visibility(),
            "lock-selected" => self.toggle_selected_lock(),
            "hide-unselected" => self.set_unselected_visible(false),
            "show-all" => self.set_unselected_visible(true),
            "tool-arrow" => self.tool = Tool::Arrow,
            "tool-lasso" => self.tool = Tool::Lasso,
            "tool-brush" => self.tool = Tool::BrushSelect,
            "tool-path" => self.tool = Tool::DeformPath,
            "level-1" => self.edit_level = 1,
            "level-2" => self.edit_level = 2,
            "level-3" => self.edit_level = 3,
            "display-all" => {
                self.view_zoom = 1.0;
                self.view_pan = Vec2::ZERO;
            }
            _ => {}
        }
    }

    /// Keep Discord Rich Presence in step with what's on screen. Starting and
    /// stopping follow the settings; text only goes out when it changes.
    fn update_presence(&mut self) {
        let custom = self.prefs.discord_app_id.trim();
        let id = if custom.is_empty() { crate::discord::APP_ID } else { custom }.to_string();
        let want = !self.prefs.discord_off;
        if !want {
            self.rpc = None;
            return;
        }
        if self.rpc.is_none() || self.rpc_id != id {
            self.rpc = crate::discord::Presence::start(id.clone());
            self.rpc_id = id;
        }
        let details = match &self.session {
            None => "On the home screen".to_string(),
            Some(s) if self.prefs.discord_show_name => format!("Editing {}", s.name),
            Some(_) => "Editing a model".to_string(),
        };
        let state = if self.session.is_none() {
            "Idle"
        } else if self.show_physics {
            "Physics Settings"
        } else if self.mesh_edit.is_some() {
            "Editing a mesh"
        } else {
            match self.workspace {
                Workspace::Animation => "Animation",
                _ => "Modeling",
            }
        };
        if let Some(p) = &mut self.rpc {
            p.show(&details, state);
        }
    }

    /// An action's current shortcut as menu text, from the keybind settings.
    fn sc(&self, action: &str) -> String {
        format_shortcut(&self.bound(action))
    }

    /// Ctrl+D: the selection's parent deformer.
    fn select_parent(&mut self) {
        let Some(s) = &self.session else { return };
        let m = s.puppet.model();
        let parent = match self.selected {
            Some(Sel::Mesh(mi)) => m.art_meshes.get(mi).and_then(|a| a.parent_deformer),
            Some(Sel::Deformer(d)) => m.deformers.get(d).and_then(|x| x.parent_deformer),
            _ => None,
        };
        if let Some(p) = parent {
            self.selected = Some(Sel::Deformer(p as usize));
            self.multi_sel.clear();
        }
    }

    /// Hide everything but the selection, or show everything again.
    fn set_unselected_visible(&mut self, show_all: bool) {
        let keep = self.selection_meshes();
        let Some(s) = &mut self.session else { return };
        for mi in 0..s.puppet.model().art_meshes.len() {
            let on = show_all || keep.contains(&mi);
            if let Some(f) = s.mesh_shown.get_mut(mi) {
                *f = on;
            }
            s.puppet.set_mesh_visible(mi, on);
        }
        s.dirty = true;
    }

    /// Edit Form > Flip on the selected mesh(es) or warp, one undo step.
    fn flip_selection(&mut self, horizontal: bool) {
        let mut targets: Vec<ayatsuri_core::edit::Target> =
            self.selection_meshes().into_iter().map(ayatsuri_core::edit::Target::ArtMesh).collect();
        if let Some(Sel::Deformer(d)) = self.selected {
            targets.push(ayatsuri_core::edit::Target::Deformer(d));
        }
        let Some(s) = &mut self.session else { return };
        let mut n = 0;
        s.edit(|m| {
            for &t in &targets {
                n += ayatsuri_core::edit::flip_object(m, t, horizontal) as usize;
            }
            n > 0
        });
        if n == 0 {
            log::warn!("flip works on meshes and warp deformers");
        } else {
            log::info!(
                "flipped {n} object(s) {}",
                if horizontal { "horizontally" } else { "vertically" }
            );
        }
    }

    /// Ctrl+2: lock or unlock the selection.
    fn toggle_selected_lock(&mut self) {
        let meshes = self.selection_meshes();
        let all_locked = !meshes.is_empty() && meshes.iter().all(|m| self.mesh_locked.contains(m));
        for mi in meshes {
            if all_locked {
                self.mesh_locked.remove(&mi);
            } else {
                self.mesh_locked.insert(mi);
            }
        }
        if let Some(Sel::Deformer(d)) = self.selected
            && !self.def_locked.remove(&d)
        {
            self.def_locked.insert(d);
        }
        log::info!("{} locked mesh(es)", self.mesh_locked.len());
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

    /// The parameter list: folders, rows and 2D pads. `full` is the
    /// Parameter palette (link buttons, every right-click menu); without it
    /// the list only moves values, as on the physics page.
    fn param_list(&mut self, ui: &mut Ui, id: &str, full: bool, list_h: f32) -> Vec<ParamAction> {
        // rows borrow the session hard, so it steps out of self for a moment
        let Some(mut s) = self.session.take() else { return Vec::new() };
        // a parameter in no folder would never be drawn: file it at the top
        // level (the unnamed group), making one if needed
        let n_params = s.puppet.model().parameters.len();
        let mut grouped = vec![false; n_params];
        for g in &s.groups {
            for &p in &g.params {
                if let Some(f) = grouped.get_mut(p) {
                    *f = true;
                }
            }
        }
        let loose: Vec<usize> =
            (0..n_params).filter(|&p| !grouped.get(p).copied().unwrap_or(true)).collect();
        if !loose.is_empty() {
            match s.groups.iter_mut().find(|g| g.name.is_empty()) {
                Some(g) => g.params.extend(loose),
                None => s.groups.insert(
                    0,
                    crate::session::ParamGroup {
                        name: String::new(),
                        params: loose,
                        open: true,
                        color: None,
                    },
                ),
            }
        }
        let can_edit = matches!(
            self.selected,
            Some(Sel::Mesh(_)) | Some(Sel::Deformer(_)) | Some(Sel::Part(_))
        );
        let mut actions: Vec<ParamAction> = Vec::new();
        // shortcut hints for the right-click menu, from the keybind settings
        let shortcuts: Vec<(&str, String)> =
            ["reset-params", "four-corners"].iter().map(|a| (*a, self.sc(a))).collect();
        let sc = |a: &str| {
            shortcuts.iter().find(|(k, _)| *k == a).map(|(_, v)| v.clone()).unwrap_or_default()
        };
        let subject = self.pmenu.subject;

        let pad_partner: std::collections::HashMap<usize, usize> =
            s.combined.iter().map(|&(a, b)| (a, b)).collect();
        let pad_skip: std::collections::HashSet<usize> =
            s.combined.iter().map(|&(_, b)| b).collect();

        ScrollArea::vertical().id_salt(id).max_height(list_h).auto_shrink([false, false]).show(
            ui,
            |ui| {
                for gi in 0..s.groups.len() {
                    let (name, open, params) = {
                        let Some(g) = s.groups.get(gi) else { break };
                        (g.name.clone(), g.open, g.params.clone())
                    };
                    if !name.is_empty() {
                        let (rect, resp) = ui
                            .allocate_exact_size(vec2(ui.available_width(), 18.0), Sense::click());
                        ui.painter().rect_filled(rect, 2.0, theme::pal().header);
                        let color = s.groups.get(gi).and_then(|g| g.color);
                        if let Some([r, g, b]) = color {
                            ui.painter().rect_filled(
                                Rect::from_min_size(rect.min, vec2(3.0, rect.height())),
                                1.0,
                                Color32::from_rgb(r, g, b),
                            );
                        }
                        let arrow = if open { "⏷" } else { "⏵" };
                        ui.painter().text(
                            rect.left_center() + vec2(6.0, 0.0),
                            Align2::LEFT_CENTER,
                            format!("{arrow} {name}"),
                            FontId::proportional(11.0),
                            theme::pal().text,
                        );
                        if full && resp.double_clicked() {
                            actions.push(ParamAction::Cmd(ParamCmd::RenameGroup(gi)));
                        } else if resp.clicked()
                            && let Some(g) = s.groups.get_mut(gi)
                        {
                            g.open = !g.open;
                        }
                        if full {
                            resp.context_menu(|ui| {
                                if let Some(c) = param_menu::group_menu(ui, &mut s, gi, subject) {
                                    actions.push(ParamAction::Cmd(c));
                                }
                            });
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
                            combined_pad(
                                ui,
                                &mut s,
                                pi,
                                partner,
                                &mut self.sel_param,
                                can_edit,
                                subject,
                                full,
                                &mut actions,
                            );
                            continue;
                        }
                        parameter_row(
                            ui,
                            &mut s,
                            pi,
                            &mut self.sel_param,
                            can_edit,
                            subject,
                            full,
                            &mut actions,
                        );
                    }
                }
                // the empty space under the rows takes the palette's own
                // right-click menu (create parameter or folder and so on)
                if full {
                    let rest = ui.available_height().clamp(40.0, list_h.max(40.0));
                    let (_, bg) =
                        ui.allocate_exact_size(vec2(ui.available_width(), rest), Sense::click());
                    bg.context_menu(|ui| {
                        if let Some(c) = param_menu::panel_menu(ui, &mut s, &sc) {
                            actions.push(ParamAction::Cmd(c));
                        }
                    });
                }
            },
        );
        self.session = Some(s);
        actions
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

        // the list takes everything but the footer, so the footer sits at
        // the bottom of the palette however few parameters there are
        let footer_h = 40.0;
        let list_h = (ui.available_height() - footer_h).max(24.0);
        let actions = self.param_list(ui, "params", true, list_h);

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
        ui.add_space(8.0);

        for action in actions {
            match action {
                ParamAction::Select(pi) => {
                    self.sel_param = Some(pi);
                    self.selected = Some(Sel::Parameter(pi));
                }
                ParamAction::Cmd(c) => self.run_param_cmd(c),
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
            if !self.part_collapsed.contains(&i)
                && let Some(kids) = children.get(i)
            {
                for &k in kids.iter().rev() {
                    stack.push((k, depth + 1));
                }
            }
            guard += 1;
            if guard > n * 2 + 16 {
                break;
            }
        }
        // a folder can fold when it holds sub-folders or meshes
        let has_mesh: std::collections::HashSet<u32> =
            s.puppet.model().art_meshes.iter().filter_map(|m| m.parent_part).collect();

        // live stack positions, front-most last, for the right-hand column
        let stack: std::collections::HashMap<usize, usize> =
            s.puppet.drawables().enumerate().map(|(pos, d)| (d.mesh, pos + 1)).collect();
        // which part each mesh belongs to, up the whole chain, for previews
        let part_parent: Vec<Option<u32>> =
            s.puppet.model().parts.iter().map(|p| p.parent_part).collect();
        let mesh_part: Vec<Option<u32>> =
            s.puppet.model().art_meshes.iter().map(|m| m.parent_part).collect();
        let under = |mesh: usize, part: usize| -> bool {
            let mut cur = mesh_part.get(mesh).copied().flatten();
            let mut hops = 0;
            while let Some(p) = cur {
                if p as usize == part {
                    return true;
                }
                cur = part_parent.get(p as usize).copied().flatten();
                hops += 1;
                if hops > 256 {
                    break;
                }
            }
            false
        };
        // a couple of renders per frame keeps the palette responsive
        let mut budget = 2usize;
        let ctx = ui.ctx().clone();
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
                let label_x = rect.min.x + 48.0 + depth as f32 * 14.0;
                let foldable = children.get(i).is_some_and(|k| !k.is_empty())
                    || has_mesh.contains(&(i as u32));
                if foldable {
                    let caret = Rect::from_center_size(
                        pos2(label_x - 8.0, rect.center().y),
                        vec2(12.0, 16.0),
                    );
                    let open_now = !self.part_collapsed.contains(&i);
                    ui.painter().text(
                        caret.center(),
                        Align2::CENTER_CENTER,
                        if open_now { "⏷" } else { "⏵" },
                        FontId::proportional(9.0),
                        theme::pal().text_faint,
                    );
                    let cresp =
                        ui.interact(caret, egui::Id::new(("part-caret", i)), Sense::click());
                    if cresp.clicked() {
                        if open_now {
                            self.part_collapsed.insert(i);
                        } else {
                            self.part_collapsed.remove(&i);
                        }
                    }
                }
                paint_folder(ui, pos2(label_x, rect.center().y));
                // the part's own preview, like the reference palette shows
                let thumb =
                    Rect::from_min_size(pos2(label_x + 14.0, rect.min.y + 1.0), vec2(16.0, 16.0));
                if !s.part_previews.contains_key(&i) && budget > 0 && ui.is_rect_visible(thumb) {
                    budget -= 1;
                    let handle = crate::thumb::render_filtered(
                        &s.puppet,
                        &s.preview_tex,
                        &|m| under(m, i),
                        96,
                    )
                    .map(|(w, h, px)| {
                        let img = egui::ColorImage::from_rgba_unmultiplied([w, h], &px);
                        ctx.load_texture(
                            format!("part-prev-{i}"),
                            img,
                            egui::TextureOptions::LINEAR,
                        )
                    });
                    s.part_previews.insert(i, handle);
                }
                if !s.part_previews.contains_key(&i) {
                    ctx.request_repaint();
                }
                ui.painter().rect_filled(thumb, 2.0, theme::pal().canvas_bg);
                let preview = s.part_previews.get(&i).and_then(|h| h.clone());
                if let Some(h) = &preview {
                    let sz = h.size_vec2();
                    let k = (thumb.width() / sz.x).min(thumb.height() / sz.y);
                    let draw = Rect::from_center_size(thumb.center(), sz * k);
                    ui.painter().image(
                        h.id(),
                        draw,
                        Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                        Color32::WHITE,
                    );
                }
                ui.painter().text(
                    pos2(label_x + 36.0, rect.center().y),
                    Align2::LEFT_CENTER,
                    truncate(&name, 20),
                    FontId::proportional(11.5),
                    theme::pal().text,
                );
                // hovering shows it big, Cubism style
                if let Some(h) = preview
                    && resp.hovered()
                {
                    resp.clone().on_hover_ui(|ui| {
                        let sz = h.size_vec2();
                        let k = (160.0 / sz.x).min(200.0 / sz.y);
                        ui.image((h.id(), sz * k));
                    });
                }
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

                // the part's meshes, nested one level deeper, unless folded
                if self.part_collapsed.contains(&i) {
                    continue;
                }
                let meshes: Vec<(usize, String)> = s
                    .puppet
                    .model()
                    .art_meshes
                    .iter()
                    .enumerate()
                    .filter(|(_, m)| m.parent_part == Some(i as u32))
                    .map(|(mi, _)| (mi, s.mesh_name(mi)))
                    .collect();
                for (mi, mid) in meshes {
                    let x_off = 66.0 + depth as f32 * 14.0;
                    parts_mesh_row(ui, s, &mut self.selected, mi, &mid, x_off, effective, &stack);
                }
            }
            // meshes that belong to no part sit at the top level, so imports
            // need no artificial root folder
            let loose: Vec<(usize, String)> = s
                .puppet
                .model()
                .art_meshes
                .iter()
                .enumerate()
                .filter(|(_, m)| m.parent_part.is_none())
                .map(|(mi, _)| (mi, s.mesh_name(mi)))
                .collect();
            for (mi, mid) in loose {
                parts_mesh_row(ui, s, &mut self.selected, mi, &mid, 30.0, true, &stack);
            }
            // glue objects, as the reference palette lists them
            let glues: Vec<(usize, String, String, String)> = s
                .puppet
                .model()
                .glues
                .iter()
                .enumerate()
                .map(|(gi, g)| {
                    let name = |i: u32| s.mesh_name(i as usize);
                    (gi, g.id.clone(), name(g.art_mesh_a), name(g.art_mesh_b))
                })
                .collect();
            let mut delete: Option<usize> = None;
            for (gi, id, a, b) in glues {
                let (rect, resp) =
                    ui.allocate_exact_size(vec2(ui.available_width(), 18.0), Sense::click());
                if self.selected_glue == Some(gi) {
                    ui.painter().rect_filled(rect, 0.0, theme::pal().select);
                }
                let tag = Rect::from_min_size(
                    pos2(rect.min.x + 22.0, rect.min.y + 3.0),
                    vec2(30.0, 12.0),
                );
                ui.painter().rect_filled(tag, 3.0, Color32::from_rgb(0xE8, 0xC6, 0x3B));
                ui.painter().text(
                    tag.center(),
                    Align2::CENTER_CENTER,
                    "Glue",
                    FontId::proportional(9.0),
                    Color32::BLACK,
                );
                ui.painter().text(
                    pos2(tag.max.x + 6.0, rect.center().y),
                    Align2::LEFT_CENTER,
                    format!("{id}  {} ↔ {}", truncate(&a, 10), truncate(&b, 10)),
                    FontId::proportional(10.5),
                    theme::pal().text,
                );
                if resp.clicked() {
                    self.selected_glue = Some(gi);
                    self.tool = Tool::Glue;
                }
                resp.context_menu(|ui| {
                    if ui.button("Delete glue").clicked() {
                        delete = Some(gi);
                        ui.close();
                    }
                });
            }
            if let Some(gi) = delete
                && s.edit(|m| ayatsuri_core::edit::unbind_glue(m, gi))
            {
                log::info!("glue {gi} deleted");
                self.selected_glue = None;
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
            if !self.def_collapsed.contains(&i)
                && let Some(kids) = children.get(i)
            {
                for &k in kids.iter().rev() {
                    stack.push((k, depth + 1));
                }
            }
            guard += 1;
            if guard > nd * 2 + 16 {
                break;
            }
        }
        let rows: Vec<(usize, usize, String, bool, bool)> = order
            .into_iter()
            .filter_map(|(i, depth)| {
                model.deformers.get(i).map(|d| {
                    let has_kids = children.get(i).is_some_and(|k| !k.is_empty())
                        || model.art_meshes.iter().any(|m| m.parent_deformer == Some(i as u32));
                    (i, depth, d.id.clone(), matches!(d.kind, DeformerKind::Warp(_)), has_kids)
                })
            })
            .collect();

        // which deformer each mesh hangs from, and each deformer's parent,
        // so a row can find everything it moves for its preview
        let def_parent: Vec<Option<u32>> =
            s.puppet.model().deformers.iter().map(|d| d.parent_deformer).collect();
        let mesh_def: Vec<Option<u32>> =
            s.puppet.model().art_meshes.iter().map(|m| m.parent_deformer).collect();
        let moved_by = |mesh: usize, def: usize| -> bool {
            let mut cur = mesh_def.get(mesh).copied().flatten();
            let mut hops = 0;
            while let Some(d) = cur {
                if d as usize == def {
                    return true;
                }
                cur = def_parent.get(d as usize).copied().flatten();
                hops += 1;
                if hops > 256 {
                    break;
                }
            }
            false
        };
        let mut budget = 2usize;
        let ctx = ui.ctx().clone();
        ScrollArea::vertical().id_salt("deformers").show(ui, |ui| {
            for (i, dep, id, is_warp, has_kids) in rows {
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
                // the lock column keeps canvas drags off a deformer
                let locked = self.def_locked.contains(&i);
                let lock = Rect::from_min_size(rect.min + vec2(16.0, 0.0), vec2(16.0, 18.0));
                ui.painter().text(
                    lock.center(),
                    Align2::CENTER_CENTER,
                    if locked {
                        egui_phosphor::regular::LOCK_SIMPLE
                    } else {
                        egui_phosphor::regular::LOCK_SIMPLE_OPEN
                    },
                    FontId::proportional(10.0),
                    if locked { theme::pal().accent } else { theme::pal().text_faint },
                );
                let lresp = ui.interact(lock, egui::Id::new(("def-lock", i)), Sense::click());
                if lresp.clicked() {
                    if locked {
                        self.def_locked.remove(&i);
                    } else {
                        self.def_locked.insert(i);
                    }
                }
                let x = rect.min.x + 48.0 + dep as f32 * 14.0;
                if has_kids {
                    let caret =
                        Rect::from_center_size(pos2(x - 10.0, rect.center().y), vec2(12.0, 14.0));
                    let open_now = !self.def_collapsed.contains(&i);
                    ui.painter().text(
                        caret.center(),
                        Align2::CENTER_CENTER,
                        if open_now { "⏷" } else { "⏵" },
                        FontId::proportional(9.0),
                        theme::pal().text_faint,
                    );
                    let caret_resp =
                        ui.interact(caret, egui::Id::new(("def-caret", i)), Sense::click());
                    if caret_resp.clicked() {
                        if open_now {
                            self.def_collapsed.insert(i);
                        } else {
                            self.def_collapsed.remove(&i);
                        }
                    }
                }
                if is_warp {
                    paint_warp_glyph(ui, pos2(x, rect.center().y));
                } else {
                    paint_rotation_glyph(ui, pos2(x, rect.center().y));
                }
                // a preview of the artwork this deformer moves, Cubism style
                let thumb = Rect::from_min_size(pos2(x + 13.0, rect.min.y + 1.0), vec2(16.0, 16.0));
                if !s.deformer_previews.contains_key(&i) && budget > 0 && ui.is_rect_visible(thumb)
                {
                    budget -= 1;
                    let handle = crate::thumb::render_filtered(
                        &s.puppet,
                        &s.preview_tex,
                        &|m| moved_by(m, i),
                        96,
                    )
                    .map(|(w, h, px)| {
                        let img = egui::ColorImage::from_rgba_unmultiplied([w, h], &px);
                        ctx.load_texture(format!("def-prev-{i}"), img, egui::TextureOptions::LINEAR)
                    });
                    s.deformer_previews.insert(i, handle);
                }
                if !s.deformer_previews.contains_key(&i) {
                    ctx.request_repaint();
                }
                ui.painter().rect_filled(thumb, 2.0, theme::pal().canvas_bg);
                let preview = s.deformer_previews.get(&i).and_then(|h| h.clone());
                if let Some(h) = &preview {
                    let sz = h.size_vec2();
                    let k = (thumb.width() / sz.x).min(thumb.height() / sz.y);
                    ui.painter().image(
                        h.id(),
                        Rect::from_center_size(thumb.center(), sz * k),
                        Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                        Color32::WHITE,
                    );
                }
                ui.painter().text(
                    pos2(x + 34.0, rect.center().y),
                    Align2::LEFT_CENTER,
                    truncate(&id, 22),
                    FontId::proportional(11.5),
                    theme::pal().text,
                );
                if let Some(h) = preview
                    && resp.hovered()
                {
                    resp.clone().on_hover_ui(|ui| {
                        let sz = h.size_vec2();
                        let k = (160.0 / sz.x).min(200.0 / sz.y);
                        ui.image((h.id(), sz * k));
                    });
                }
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

                // this deformer's meshes, nested unless folded away
                if self.def_collapsed.contains(&i) {
                    continue;
                }
                let meshes: Vec<(usize, String)> = s
                    .puppet
                    .model()
                    .art_meshes
                    .iter()
                    .enumerate()
                    .filter(|(_, m)| m.parent_deformer == Some(i as u32))
                    .map(|(mi, _)| (mi, s.mesh_name(mi)))
                    .collect();
                for (mi, mid) in meshes {
                    let (rect, resp) =
                        ui.allocate_exact_size(vec2(ui.available_width(), 15.0), Sense::click());
                    if self.selected == Some(Sel::Mesh(mi)) {
                        ui.painter().rect_filled(rect, 0.0, theme::pal().select);
                    }
                    let x = rect.min.x + 62.0 + dep as f32 * 14.0;
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
        // a glue being edited shows first, the way the reference inspector
        // switches to glue display
        if let Some(gi) = self.selected_glue
            && let Some(g) = s.puppet.model().glues.get(gi).cloned()
        {
            let mesh = |i: u32| s.mesh_name(i as usize);
            let (name_a, name_b) = (mesh(g.art_mesh_a), mesh(g.art_mesh_b));
            egui::Grid::new("glue-props").num_columns(2).spacing([10.0, 4.0]).show(ui, |ui| {
                ui.weak("Glue");
                ui.label(&g.id);
                ui.end_row();
                ui.weak("Mesh A (red)");
                ui.label(&name_a);
                ui.end_row();
                ui.weak("Mesh B (green)");
                ui.label(&name_b);
                ui.end_row();
                ui.weak("Bound pairs");
                ui.label((g.indices.len() / 2).to_string());
                ui.end_row();
                ui.weak("Compatibility");
                let current = g.intensities.first().copied().unwrap_or(1.0);
                let mut v = self.glue_compat.unwrap_or(current);
                let r = ui.add(
                    egui::Slider::new(&mut v, 0.0..=1.0)
                        .custom_formatter(|x, _| format!("{:.0}%", x * 100.0)),
                );
                if r.changed() {
                    self.glue_compat = Some(v);
                }
                if (r.drag_stopped() || r.lost_focus())
                    && let Some(v) = self.glue_compat.take()
                    && s.edit(|m| ayatsuri_core::edit::set_glue_compatibility(m, gi, v))
                {
                    log::debug!("glue {gi}: compatibility {:.0}%", v * 100.0);
                }
                ui.end_row();
            });
            ui.weak("Lower compatibility weakens the glue; key it per parameter for blends.");
            ui.separator();
        }
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
                row(ui, "Name", s.mesh_name(i));
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
            // keep offering the textures until the renderer has them
            if s.gpu_pending.is_some() && s.gpu_uploaded.load(std::sync::atomic::Ordering::Acquire)
            {
                s.gpu_pending = None;
                log::debug!("textures are on the GPU");
            }
            let cb = crate::gpu_view::callback(
                view,
                &s.puppet,
                s.gpu_pending.clone(),
                s.gpu_uploaded.clone(),
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
                add_mesh_shape(&vp, mesh);
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
                    hit_mesh(&s.puppet, ux, uy, &|m| {
                        self.mesh_locked.contains(&m)
                            || !s.mesh_shown.get(m).copied().unwrap_or(true)
                    })
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
            // a click on the selected warp's points or box handles belongs
            // to warp editing, not to picking the mesh underneath
            if resp.clicked()
                && !self.pts.hot
                && let Some(ptr) = resp.interact_pointer_pos()
            {
                let (ux, uy) = unit_of(ptr);
                let alt = ui.ctx().input(|i| i.modifiers.alt);
                let hit = hit_mesh(&s.puppet, ux, uy, &|m| {
                    self.mesh_locked.contains(&m) || !s.mesh_shown.get(m).copied().unwrap_or(true)
                });
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
                if let Some(mi) = hit_mesh(&s.puppet, ux, uy, &|m| {
                    self.mesh_locked.contains(&m) || !s.mesh_shown.get(m).copied().unwrap_or(true)
                }) {
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
            if let Some(mi) = hit_mesh(&s.puppet, ux, uy, &|m| {
                self.mesh_locked.contains(&m) || !s.mesh_shown.get(m).copied().unwrap_or(true)
            }) {
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
        // with a mesh or warp selected the lasso picks its points instead (the
        // point tool handles that); otherwise it selects whole meshes
        let lasso_points = match self.selected {
            Some(Sel::Mesh(_)) => true,
            Some(Sel::Deformer(d)) => s.puppet.warp_grid(d).is_some(),
            _ => false,
        };
        if self.tool == Tool::Lasso && !space && self.mesh_edit.is_none() && lasso_points {
            // a click off the object's points and box lets go of it, so the
            // next lasso selects meshes again
            if resp.clicked() && !self.pts.hot && self.pts.picked.is_empty() {
                self.selected = None;
                self.multi_sel.clear();
            }
        }
        if self.tool == Tool::Lasso && !space && self.mesh_edit.is_none() && !lasso_points {
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
                        let caught = pos.iter().any(|xy| {
                            point_edit::point_in_polygon(to_screen(xy[0], xy[1]), &self.lasso)
                        });
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
        let mut warp_hot = false;
        match self.selected {
            Some(Sel::Mesh(mi)) => {
                if let Some((gpos, gidx)) =
                    s.puppet.mesh_geometry(mi).map(|g| (g.positions.to_vec(), g.indices.to_vec()))
                {
                    let (pos, _) = gpos.as_chunks::<2>();
                    let mut pts: Vec<Pos2> = pos.iter().map(|xy| to_screen(xy[0], xy[1])).collect();
                    // the arrow tool edits the mesh's current keyform: the box
                    // moves, scales and rotates it all, picked points get their
                    // own box, and a brush selection moves by its weights
                    let editable = matches!(self.tool, Tool::Arrow | Tool::Lasso)
                        && !space
                        && self.mesh_edit.is_none()
                        && !self.mesh_locked.contains(&mi);
                    let mut grab_hot = None;
                    let mut hover_pt = None;
                    // the mesh's deform path at this edit level: dragging a
                    // control point bends the mesh along the path, points
                    // inside the width following, the hardness share fully
                    let level = self.edit_level;
                    let mut path_s: Vec<Pos2> = if level >= 2 {
                        self.deform_paths
                            .get(&(mi, level))
                            .map(|l| l.iter().filter_map(|p| pin_pos(*p, &pts, &gidx)).collect())
                            .unwrap_or_default()
                    } else {
                        Vec::new()
                    };
                    let mut near_cp = None;
                    let mut path_busy = false;
                    if editable && !path_s.is_empty() {
                        let inp = point_edit::PtInput::read(ui, &resp);
                        let probe = if inp.drag_started { inp.origin.or(inp.ptr) } else { inp.ptr };
                        near_cp = probe.and_then(|p| nearest_within(&path_s, p, 8.0));
                        if inp.drag_started
                            && self.path_grab.is_none()
                            && let (Some(cp), Some(start)) = (near_cp, probe)
                            && let Some(local) = s.puppet.mesh_local_positions(mi)
                        {
                            let bind = point_edit::bind_path(
                                &path_s,
                                &pts,
                                self.path_width * scale,
                                self.path_hardness,
                            );
                            self.path_grab = Some(PathGrab {
                                mesh: mi,
                                point: cp,
                                start,
                                ptr: start,
                                bind: Some(bind),
                                verts: pts.clone(),
                                local,
                            });
                        }
                        let mut finish: Option<PathFinish> = None;
                        if let Some(gp) = &mut self.path_grab
                            && gp.mesh == mi
                            && let Some(bind) = &gp.bind
                        {
                            path_busy = true;
                            if let Some(p) = inp.ptr {
                                gp.ptr = p;
                            }
                            let mut moved = bind.path.clone();
                            if let Some(cp) = moved.get_mut(gp.point) {
                                *cp += gp.ptr - gp.start;
                            }
                            let offs = point_edit::follow_path(bind, &gp.verts, &moved);
                            for &(vi, d) in &offs {
                                if let (Some(p), Some(v)) = (pts.get_mut(vi), gp.verts.get(vi)) {
                                    *p = *v + d;
                                }
                            }
                            path_s = moved.clone();
                            if inp.drag_stopped || !inp.any_down {
                                let to = moved.get(gp.point).copied().unwrap_or(gp.ptr);
                                finish = Some((gp.point, to, offs, std::mem::take(&mut gp.local)));
                            }
                        }
                        if let Some((cp, to, offs, local)) = finish {
                            self.path_grab = None;
                            path_busy = false;
                            let parent =
                                s.puppet.model().art_meshes.get(mi).and_then(|m| m.parent_deformer);
                            let k_unit = 1.0 / (scale * c.pixels_per_unit);
                            let moves: Vec<(usize, f32, f32)> = offs
                                .iter()
                                .filter(|(_, d)| d.length() > 1e-3)
                                .map(|&(vi, d)| {
                                    let px = local.get(vi * 2).copied().unwrap_or(0.0);
                                    let py = local.get(vi * 2 + 1).copied().unwrap_or(0.0);
                                    let (dx, dy) = parent_delta(
                                        &s.puppet,
                                        parent,
                                        px,
                                        py,
                                        d.x * k_unit,
                                        d.y * k_unit,
                                    );
                                    (vi, dx, dy)
                                })
                                .collect();
                            let values: Vec<f32> = (0..s.puppet.model().parameters.len())
                                .map(|i| s.puppet.value(i))
                                .collect();
                            let target = ayatsuri_core::edit::Target::ArtMesh(mi);
                            match ayatsuri_core::keyform::keyform_at(
                                s.puppet.model(),
                                target,
                                &values,
                            ) {
                                Some(k) => {
                                    if !moves.is_empty()
                                        && s.edit(|m| {
                                            ayatsuri_core::edit::move_vertices_in(
                                                m,
                                                mi,
                                                Some(k),
                                                &moves,
                                            )
                                        })
                                    {
                                        log::debug!(
                                            "deform path bent {} point(s) of mesh {mi} in keyform {k}",
                                            moves.len()
                                        );
                                    }
                                    // the control point stays where it was dropped,
                                    // pinned to the bent mesh
                                    if let Some(pin) = pin_at(&pts, &gidx, to)
                                        && let Some(slot) = self
                                            .deform_paths
                                            .get_mut(&(mi, level))
                                            .and_then(|l| l.get_mut(cp))
                                    {
                                        *slot = pin;
                                    }
                                }
                                None => log::warn!(
                                    "the parameters are between keys: move them onto a key to edit this mesh"
                                ),
                            }
                        }
                        if near_cp.is_some() || path_busy {
                            warp_hot = true;
                            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                        }
                    }
                    if editable && !path_busy && near_cp.is_none() {
                        self.pts.bind((1, mi));
                        self.pts.weights = self.brush_w.get(&(1, mi)).cloned().unwrap_or_default();
                        let inp = point_edit::PtInput::read(ui, &resp);
                        let opts = point_edit::PtOpts {
                            whole_box: true,
                            marquee: true,
                            lasso: self.tool == Tool::Lasso,
                            reach: 7.0,
                            pad: 0.0,
                        };
                        let out = point_edit::point_tool(&mut self.pts, &inp, &mut pts, &opts);
                        warp_hot |= self.pts.hot;
                        grab_hot = out.hover_grab;
                        hover_pt = out.hover;
                        if ui.ctx().input(|i| i.key_pressed(egui::Key::Escape)) {
                            self.pts.picked.clear();
                        }
                        if let Some(moved) = out.commit
                            && let Some(local) = s.puppet.mesh_local_positions(mi)
                        {
                            let parent =
                                s.puppet.model().art_meshes.get(mi).and_then(|m| m.parent_deformer);
                            let k_unit = 1.0 / (scale * c.pixels_per_unit);
                            let moves: Vec<(usize, f32, f32)> = moved
                                .iter()
                                .map(|&(vi, d)| {
                                    let px = local.get(vi * 2).copied().unwrap_or(0.0);
                                    let py = local.get(vi * 2 + 1).copied().unwrap_or(0.0);
                                    let (dx, dy) = parent_delta(
                                        &s.puppet,
                                        parent,
                                        px,
                                        py,
                                        d.x * k_unit,
                                        d.y * k_unit,
                                    );
                                    (vi, dx, dy)
                                })
                                .collect();
                            let values: Vec<f32> = (0..s.puppet.model().parameters.len())
                                .map(|i| s.puppet.value(i))
                                .collect();
                            let target = ayatsuri_core::edit::Target::ArtMesh(mi);
                            match ayatsuri_core::keyform::keyform_at(
                                s.puppet.model(),
                                target,
                                &values,
                            ) {
                                Some(k) => {
                                    if s.edit(|m| {
                                        ayatsuri_core::edit::move_vertices_in(
                                            m,
                                            mi,
                                            Some(k),
                                            &moves,
                                        )
                                    }) {
                                        log::debug!(
                                            "mesh {mi}: {} point(s) moved in keyform {k}",
                                            moves.len()
                                        );
                                    }
                                }
                                None => log::warn!(
                                    "the parameters are between keys: move them onto a key to edit this mesh"
                                ),
                            }
                        }
                        if hover_pt.is_some() {
                            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                        }
                    }
                    let (tris, _) = gidx.as_chunks::<3>();
                    let stroke = Stroke::new(self.disp.mesh_width, self.disp.mesh_color);
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
                    for (vi, p) in pts.iter().enumerate() {
                        let r =
                            if hover_pt == Some(vi) && self.pts.drag.is_none() { 4.0 } else { 1.8 };
                        vp.circle_filled(*p, r, self.disp.mesh_color);
                    }
                    draw_weights(&vp, &pts, self.brush_w.get(&(1, mi)), self.disp.weight_color);
                    if !path_s.is_empty() {
                        draw_path(
                            &vp,
                            &path_s,
                            self.path_width * scale,
                            self.path_hardness,
                            near_cp,
                        );
                    }
                    if editable {
                        point_edit::draw_overlay(&self.pts, &vp, &pts, true, 0.0, grab_hot);
                    }
                }
            }
            Some(Sel::Deformer(di)) => {
                if let Some((grid, rows, cols)) = s.puppet.warp_grid(di) {
                    // Cubism's warp editing: the box around the grid moves,
                    // scales (red dots; Shift keeps the aspect, Alt from the
                    // centre) and rotates (just outside) the whole warp; click,
                    // Shift+click or drag a rectangle to pick grid points and
                    // the box works on those. Edits land in the keyform the
                    // parameters sit on, as one undo step per drag.
                    let stride = cols as usize + 1;
                    let (gp, _) = grid.as_chunks::<2>();
                    let mut screen_pts: Vec<Pos2> =
                        gp.iter().map(|p| to_screen(p[0], p[1])).collect();
                    let mut warp_commit: Option<Vec<(usize, f32, f32)>> = None;
                    let mut warp_commit_screen = false;
                    let editable = matches!(self.tool, Tool::Arrow | Tool::Lasso)
                        && !space
                        && !self.def_locked.contains(&di);
                    let mut grab_hot = None;
                    let mut hover_pt = None;
                    // the Bezier control lattice, fitted to the grid in the
                    // warp's parent space and drawn through the parent
                    let parent =
                        s.puppet.model().deformers.get(di).and_then(|de| de.parent_deformer);
                    let to_scr = |x: f32, y: f32| match parent {
                        Some(p) => {
                            let (mx, my) = s.puppet.deformer_point(p as usize, x, y);
                            to_screen(mx, my)
                        }
                        None => to_screen(x, y),
                    };
                    let bez_mode = editable && self.warp_bezier;
                    let mut bez: Option<(ayatsuri_core::bezier::Lattice, Vec<Pos2>)> = None;
                    let mut bez_hover: Option<usize> = None;
                    let mut bez_busy = false;
                    if bez_mode && let Some(local) = s.puppet.warp_local_grid(di) {
                        let (bw, bh) = if self.edit_level >= 3 {
                            (1, 1)
                        } else {
                            self.bezier_div
                                .get(&di)
                                .copied()
                                .unwrap_or((2.min(cols as usize), 2.min(rows as usize)))
                        };
                        let inp = point_edit::PtInput::read(ui, &resp);
                        let mut lat = match &self.bez_drag {
                            Some(d) if d.deformer == di => d.lattice.clone(),
                            _ => ayatsuri_core::bezier::Lattice::fit(
                                &local,
                                cols as usize,
                                rows as usize,
                                bw,
                                bh,
                            ),
                        };
                        let net_s: Vec<Pos2> = lat.net.iter().map(|&(x, y)| to_scr(x, y)).collect();
                        // a drag decides its grab where the button went down
                        let probe = if inp.drag_started { inp.origin.or(inp.ptr) } else { inp.ptr };
                        bez_hover = probe.and_then(|p| {
                            net_s
                                .iter()
                                .enumerate()
                                .filter(|(i, _)| {
                                    lat.kind(*i) != ayatsuri_core::bezier::NetPoint::Inner
                                })
                                .map(|(i, q)| (i, q.distance(p)))
                                .filter(|(_, d)| *d < 8.0)
                                .min_by(|a, b| a.1.total_cmp(&b.1))
                                .map(|(i, _)| i)
                        });
                        if inp.drag_started
                            && self.bez_drag.is_none()
                            && let (Some(idx), Some(start)) =
                                (bez_hover, inp.origin.or(inp.press).or(inp.ptr))
                        {
                            self.bez_drag = Some(BezierDrag {
                                deformer: di,
                                idx,
                                start,
                                ptr: start,
                                lattice: lat.clone(),
                                local: local.clone(),
                            });
                        }
                        if let Some(d) = &mut self.bez_drag
                            && d.deformer == di
                        {
                            bez_busy = true;
                            if let Some(p) = inp.ptr {
                                d.ptr = p;
                            }
                            // screen motion to the warp's parent space, through
                            // the parent's local jacobian at the grabbed point
                            let k_unit = 1.0 / (scale * c.pixels_per_unit);
                            let at = d.lattice.net.get(d.idx).copied().unwrap_or((0.0, 0.0));
                            let sd = d.ptr - d.start;
                            let (dx, dy) = parent_delta(
                                &s.puppet,
                                parent,
                                at.0,
                                at.1,
                                sd.x * k_unit,
                                sd.y * k_unit,
                            );
                            lat = d.lattice.clone();
                            let mut moving = vec![d.idx];
                            if lat.kind(d.idx) == ayatsuri_core::bezier::NetPoint::Anchor {
                                moving.extend(lat.handles_of(d.idx));
                            }
                            for i in moving {
                                if let Some(q) = lat.net.get_mut(i) {
                                    q.0 += dx;
                                    q.1 += dy;
                                }
                            }
                            lat.derive_inner();
                            let new_grid = lat.to_grid(cols as usize, rows as usize);
                            let (ng, _) = new_grid.as_chunks::<2>();
                            for (sp, &[x, y]) in screen_pts.iter_mut().zip(ng) {
                                *sp = to_scr(x, y);
                            }
                            if inp.drag_stopped || !inp.any_down {
                                let (old, _) = d.local.as_chunks::<2>();
                                let moves: Vec<(usize, f32, f32)> = ng
                                    .iter()
                                    .zip(old)
                                    .enumerate()
                                    .map(|(gi, (&[nx, ny], &[ox, oy]))| (gi, nx - ox, ny - oy))
                                    .filter(|(_, x, y)| x.abs() > 1e-7 || y.abs() > 1e-7)
                                    .collect();
                                self.bez_drag = None;
                                bez_busy = false;
                                warp_commit = Some(moves);
                            }
                        }
                        let net_now: Vec<Pos2> =
                            lat.net.iter().map(|&(x, y)| to_scr(x, y)).collect();
                        bez = Some((lat, net_now));
                    }
                    if bez_hover.is_some() || bez_busy {
                        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                    }
                    if editable && !bez_busy && bez_hover.is_none() {
                        self.pts.bind((2, di));
                        // a brush selection drives division points, not handles
                        self.pts.weights = if self.warp_bezier {
                            Vec::new()
                        } else {
                            self.brush_w.get(&(2, di)).cloned().unwrap_or_default()
                        };
                        let inp = point_edit::PtInput::read(ui, &resp);
                        let reach = if self.warp_bezier { 0.0 } else { 8.0 };
                        // in Bezier mode the box stands off the grid so its dots
                        // don't sit on the outer anchors
                        let opts = point_edit::PtOpts {
                            whole_box: true,
                            marquee: !self.warp_bezier,
                            lasso: self.tool == Tool::Lasso,
                            reach,
                            pad: if self.warp_bezier { 16.0 } else { 0.0 },
                        };
                        let out =
                            point_edit::point_tool(&mut self.pts, &inp, &mut screen_pts, &opts);
                        warp_hot = self.pts.hot;
                        if let Some(m) = out.commit {
                            warp_commit =
                                Some(m.into_iter().map(|(gi, d)| (gi, d.x, d.y)).collect());
                            warp_commit_screen = true;
                        }
                        grab_hot = out.hover_grab;
                        hover_pt = out.hover;
                        if ui.ctx().input(|i| i.key_pressed(egui::Key::Escape)) {
                            self.pts.picked.clear();
                        }
                        if hover_pt.is_some() {
                            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                        }
                    }
                    warp_hot |= bez_hover.is_some() || bez_busy;
                    // land the edit in the keyform the parameters sit on. Box
                    // and point drags hand back screen offsets; Bezier drags
                    // hand back parent-space ones already
                    if let Some(moved) = warp_commit.take() {
                        let k_unit = 1.0 / (scale * c.pixels_per_unit);
                        let moves: Vec<(usize, f32, f32)> = if warp_commit_screen {
                            let local = s.puppet.warp_local_grid(di).unwrap_or_default();
                            moved
                                .iter()
                                .map(|&(gi, sx, sy)| {
                                    let px = local.get(gi * 2).copied().unwrap_or(0.0);
                                    let py = local.get(gi * 2 + 1).copied().unwrap_or(0.0);
                                    let (dx, dy) = parent_delta(
                                        &s.puppet,
                                        parent,
                                        px,
                                        py,
                                        sx * k_unit,
                                        sy * k_unit,
                                    );
                                    (gi, dx, dy)
                                })
                                .collect()
                        } else {
                            moved
                        };
                        let values: Vec<f32> = (0..s.puppet.model().parameters.len())
                            .map(|i| s.puppet.value(i))
                            .collect();
                        let target = ayatsuri_core::edit::Target::Deformer(di);
                        match ayatsuri_core::keyform::keyform_at(s.puppet.model(), target, &values)
                        {
                            Some(k) => {
                                if !moves.is_empty()
                                    && s.edit(|m| {
                                        ayatsuri_core::edit::move_warp_points(
                                            m,
                                            di,
                                            Some(k),
                                            &moves,
                                        )
                                    })
                                {
                                    log::debug!(
                                        "warp {di}: {} grid point(s) moved in keyform {k}",
                                        moves.len()
                                    );
                                }
                            }
                            None => log::warn!(
                                "the parameters are between keys: move them onto a key to edit this warp"
                            ),
                        }
                    }

                    // in Bezier mode the division points are only a readout: grey
                    let grid_col = if bez.is_some() {
                        Color32::from_gray(150).gamma_multiply(0.6)
                    } else {
                        self.disp.warp_color
                    };
                    let stroke = Stroke::new(1.0, grid_col);
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
                            if hover_pt == Some(gi) && self.pts.drag.is_none() {
                                vp.circle(
                                    pa,
                                    4.0,
                                    theme::pal().accent,
                                    Stroke::new(1.0, theme::pal().warp_green),
                                );
                            } else if bez.is_some() {
                                vp.circle_filled(pa, 1.3, grid_col);
                            } else {
                                vp.circle_filled(pa, 2.2, grid_col);
                            }
                        }
                    }
                    draw_weights(
                        &vp,
                        &screen_pts,
                        self.brush_w.get(&(2, di)),
                        self.disp.weight_color,
                    );
                    if let Some((lat, net_s)) = &bez {
                        let green = self.disp.warp_color;
                        for (i, &p) in net_s.iter().enumerate() {
                            if lat.kind(i) != ayatsuri_core::bezier::NetPoint::Anchor {
                                continue;
                            }
                            for h in lat.handles_of(i) {
                                if let Some(&hp) = net_s.get(h) {
                                    vp.line_segment([p, hp], Stroke::new(1.2, green));
                                    let hot = bez_hover == Some(h);
                                    let col = if hot { theme::pal().accent } else { green };
                                    vp.circle_filled(hp, if hot { 4.0 } else { 2.6 }, col);
                                }
                            }
                            let hot = bez_hover == Some(i);
                            vp.circle(
                                p,
                                if hot { 5.5 } else { 4.2 },
                                if hot { theme::pal().accent } else { green },
                                Stroke::new(1.0, Color32::WHITE),
                            );
                        }
                    }
                    if editable && !bez_busy {
                        point_edit::draw_overlay(
                            &self.pts,
                            &vp,
                            &screen_pts,
                            true,
                            if self.warp_bezier { 16.0 } else { 0.0 },
                            grab_hot,
                        );
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
                            && !self.def_locked.contains(&di)
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

                    let stroke = Stroke::new(1.4, self.disp.rotation_color);
                    vp.circle_stroke(o, 26.0, stroke);
                    vp.line_segment([o, tip], stroke);
                    let turning = self.rot_drag.as_ref().is_some_and(|d| d.turning);
                    if near_tip || turning {
                        vp.circle(tip, 4.5, theme::pal().accent, stroke);
                    } else {
                        vp.circle_filled(tip, 3.0, self.disp.rotation_color);
                    }
                    if near_origin || self.rot_drag.as_ref().is_some_and(|d| !d.turning) {
                        vp.circle(o, 4.5, theme::pal().accent, stroke);
                    } else {
                        vp.circle_filled(o, 2.5, self.disp.rotation_color);
                    }
                }
            }
            _ => {}
        }

        self.pts.hot = warp_hot;

        // Brush Selection: paint a soft selection onto the selected meshes' or
        // warp's points. Drag adds weight (Alt takes it away), dabs spaced
        // by distance so the speed of the stroke doesn't matter; B + drag
        // sizes the brush; Esc clears. The arrow tool then moves every
        // painted point by its weight.
        if self.tool == Tool::BrushSelect && !space && self.mesh_edit.is_none() {
            let (ptr, alt, b_down, esc, delta) = ui.ctx().input(|i| {
                (
                    i.pointer.latest_pos(),
                    i.modifiers.alt,
                    i.key_down(egui::Key::B),
                    i.key_pressed(egui::Key::Escape),
                    i.pointer.delta(),
                )
            });
            let mut targets: Vec<((u8, usize), Vec<Pos2>)> = Vec::new();
            if let Some(Sel::Deformer(di)) = self.selected
                && let Some((grid, _, _)) = s.puppet.warp_grid(di)
            {
                let (gp, _) = grid.as_chunks::<2>();
                targets.push(((2, di), gp.iter().map(|p| to_screen(p[0], p[1])).collect()));
            }
            let mut meshes = self.multi_sel.clone();
            if let Some(Sel::Mesh(mi)) = self.selected
                && !meshes.contains(&mi)
            {
                meshes.push(mi);
            }
            for mi in meshes {
                if let Some(g) = s.puppet.mesh_geometry(mi) {
                    let (gp, _) = g.positions.as_chunks::<2>();
                    targets.push(((1, mi), gp.iter().map(|p| to_screen(p[0], p[1])).collect()));
                }
            }
            let r = self.tool_opts.brush_size.clamp(2.0, 400.0);
            if esc {
                self.brush_w.clear();
                log::debug!("brush selection cleared");
            }
            if b_down && resp.dragged() {
                self.tool_opts.brush_size = (self.tool_opts.brush_size + delta.x).clamp(2.0, 400.0);
            } else if let Some(p) = ptr
                && (resp.drag_started() || resp.clicked() || resp.dragged())
            {
                let dab = resp.drag_started()
                    || resp.clicked()
                    || self.brush_last.is_none_or(|l| l.distance(p) >= r * 0.15);
                if dab {
                    self.brush_last = Some(p);
                    let sign = if alt { -1.0 } else { 1.0 };
                    for (key, pts) in &targets {
                        let w = self.brush_w.entry(*key).or_default();
                        if w.len() != pts.len() {
                            w.resize(pts.len(), 0.0);
                        }
                        for (q, wt) in pts.iter().zip(w.iter_mut()) {
                            let d = q.distance(p);
                            if d < r {
                                let t = 1.0 - d / r;
                                let f = t * t * (3.0 - 2.0 * t);
                                *wt = (*wt + sign * self.brush_weight * f).clamp(0.0, 1.0);
                            }
                        }
                    }
                }
            }
            if resp.drag_stopped() {
                self.brush_last = None;
                // an object painted back to nothing has no selection
                self.brush_w.retain(|_, w| w.iter().any(|x| *x > 1e-3));
            }
            for (key, pts) in &targets {
                draw_weights(&vp, pts, self.brush_w.get(key), self.disp.weight_color);
            }
            if let Some(p) = ptr
                && resp.hovered()
            {
                let col = if alt { theme::pal().text_faint } else { self.disp.weight_color };
                vp.circle_stroke(p, r, Stroke::new(1.2, col.gamma_multiply(2.0)));
                ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
            }
        }

        // the Deform Path tool lays a path on the selected mesh: click adds a
        // control point, dragging one moves it, Alt+click deletes it,
        // Backspace removes the last. Placing deforms nothing; the arrow tool
        // drags the points to bend the mesh. Levels 2 and 3 each keep their
        // own path, level 1 hides them, as in Cubism.
        if self.tool == Tool::DeformPath && !space && self.mesh_edit.is_none() {
            let level = self.edit_level;
            let inp = point_edit::PtInput::read(ui, &resp);
            let target = match self.selected {
                Some(Sel::Mesh(mi)) => Some(mi),
                _ => None,
            };
            let geo = target.and_then(|mi| {
                s.puppet.mesh_geometry(mi).map(|g| {
                    let (gp, _) = g.positions.as_chunks::<2>();
                    let pts: Vec<Pos2> = gp.iter().map(|p| to_screen(p[0], p[1])).collect();
                    (pts, g.indices.to_vec())
                })
            });
            match (target, geo) {
                (Some(mi), Some((pts, idx))) if level >= 2 => {
                    let key = (mi, level);
                    let pins = self.deform_paths.get(&key).cloned().unwrap_or_default();
                    let mut path_s: Vec<Pos2> =
                        pins.iter().filter_map(|p| pin_pos(*p, &pts, &idx)).collect();
                    let probe = if inp.drag_started { inp.origin.or(inp.ptr) } else { inp.ptr };
                    let near = probe.and_then(|p| nearest_within(&path_s, p, 8.0));
                    if inp.clicked
                        && let Some(at) = inp.press.or(inp.ptr)
                    {
                        match near {
                            Some(i) if inp.alt => {
                                if let Some(list) = self.deform_paths.get_mut(&key)
                                    && i < list.len()
                                {
                                    list.remove(i);
                                }
                            }
                            None => match pin_at(&pts, &idx, at) {
                                Some(pin) => self.deform_paths.entry(key).or_default().push(pin),
                                None => {
                                    log::warn!("deform path: the mesh needs faces to hold a path")
                                }
                            },
                            _ => {}
                        }
                    }
                    if ui.ctx().input(|i| i.key_pressed(egui::Key::Backspace))
                        && !ui.ctx().egui_wants_keyboard_input()
                        && let Some(list) = self.deform_paths.get_mut(&key)
                    {
                        list.pop();
                    }
                    if inp.drag_started
                        && let (Some(i), Some(start)) = (near, probe)
                    {
                        self.path_grab = Some(PathGrab {
                            mesh: mi,
                            point: i,
                            start,
                            ptr: start,
                            bind: None,
                            verts: Vec::new(),
                            local: Vec::new(),
                        });
                    }
                    if let Some(gp) = &mut self.path_grab
                        && gp.mesh == mi
                        && gp.bind.is_none()
                    {
                        if let Some(p) = inp.ptr {
                            gp.ptr = p;
                        }
                        let moved_to = path_s.get(gp.point).map(|p| *p + (gp.ptr - gp.start));
                        if let (Some(dst), Some(to)) = (path_s.get_mut(gp.point), moved_to) {
                            *dst = to;
                        }
                        if inp.drag_stopped || !inp.any_down {
                            if let Some(to) = moved_to
                                && let Some(pin) = pin_at(&pts, &idx, to)
                                && let Some(slot) = self
                                    .deform_paths
                                    .get_mut(&key)
                                    .and_then(|l| l.get_mut(gp.point))
                            {
                                *slot = pin;
                            }
                            self.path_grab = None;
                        }
                    }
                    let stroke = Stroke::new(1.0, self.disp.mesh_color.gamma_multiply(0.6));
                    for t in idx.as_chunks::<3>().0 {
                        let (Some(&a), Some(&b), Some(&c2)) = (
                            pts.get(t[0] as usize),
                            pts.get(t[1] as usize),
                            pts.get(t[2] as usize),
                        ) else {
                            continue;
                        };
                        vp.line_segment([a, b], stroke);
                        vp.line_segment([b, c2], stroke);
                        vp.line_segment([c2, a], stroke);
                    }
                    draw_path(&vp, &path_s, self.path_width * scale, self.path_hardness, near);
                    if near.is_some() {
                        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                    } else {
                        ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
                    }
                    path_s.clear();
                }
                (Some(_), _) => {}
                (None, _) => {
                    // no mesh yet: a click picks one
                    if inp.clicked
                        && let Some(ptr) = inp.press.or(inp.ptr)
                    {
                        let ux = ((ptr.x - origin.x) / scale - c.origin_x) / c.pixels_per_unit;
                        let uy = ((ptr.y - origin.y) / scale - c.origin_y) / c.pixels_per_unit;
                        if let Some(mi) = hit_mesh(&s.puppet, ux, uy, &|m| {
                            self.mesh_locked.contains(&m)
                                || !s.mesh_shown.get(m).copied().unwrap_or(true)
                        }) {
                            self.selected = Some(Sel::Mesh(mi));
                        }
                    }
                }
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
            AddFree(usize, [usize; 3], [f32; 3], Option<(u16, u16)>, Vec<u16>),
            Connect(usize, u16, u16),
            Cut(usize, u16, u16),
            EraseMany(usize, Vec<usize>),
            Del(usize, usize),
        }
        let mut mesh_op: Option<MeshOp> = None;
        if let Some(mi) = self.mesh_edit {
            vp.rect_filled(view, 0.0, Color32::from_black_alpha(150));
            let ptr = ui.ctx().input(|i| i.pointer.latest_pos());
            let mesh_id = Some(s.mesh_name(mi));
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
                let reach = self.pick_radius.clamp(2.0, 40.0);
                if let Some(ptr) = ptr {
                    for (vi, sp) in screen_pts.iter().enumerate() {
                        let d = sp.distance(ptr);
                        if d < reach && near.is_none_or(|(_, nd)| d < nd) {
                            near = Some((vi, d));
                        }
                    }
                }

                if !space
                    && self.mesh_mode == 1
                    && resp.clicked()
                    && near.is_none()
                    && let Some(ptr) = resp.interact_pointer_pos()
                {
                    // add: inside a face splits it (or drops a loose point
                    // with auto connect off); outside, the point closes a face
                    // onto the nearest open edge; with no faces at all it is
                    // placed against the three nearest points, for planning
                    let (tris, _) = g.indices.as_chunks::<3>();
                    let bary = |c: [usize; 3]| -> Option<[f32; 3]> {
                        let s0 = *screen_pts.get(c[0])?;
                        let s1 = *screen_pts.get(c[1])?;
                        let s2 = *screen_pts.get(c[2])?;
                        let den = (s1.y - s2.y) * (s0.x - s2.x) + (s2.x - s1.x) * (s0.y - s2.y);
                        if den.abs() < 1e-6 {
                            return None;
                        }
                        let w0 =
                            ((s1.y - s2.y) * (ptr.x - s2.x) + (s2.x - s1.x) * (ptr.y - s2.y)) / den;
                        let w1 =
                            ((s2.y - s0.y) * (ptr.x - s2.x) + (s0.x - s2.x) * (ptr.y - s2.y)) / den;
                        Some([w0, w1, 1.0 - w0 - w1])
                    };
                    let mut inside: Option<(usize, [usize; 3], [f32; 3])> = None;
                    let mut closest: Option<([usize; 3], [f32; 3], f32)> = None;
                    for (ti, t) in tris.iter().enumerate() {
                        let c = [t[0] as usize, t[1] as usize, t[2] as usize];
                        let Some(w) = bary(c) else { continue };
                        let worst = w[0].min(w[1]).min(w[2]);
                        if worst >= 0.0 {
                            inside = Some((ti, c, w));
                            break;
                        }
                        if closest.is_none_or(|(_, _, cw)| worst > cw) {
                            closest = Some((c, w, worst));
                        }
                    }
                    if let Some((ti, c, w)) = inside {
                        mesh_op = Some(if self.mesh_autoconnect {
                            MeshOp::Add(mi, ti, w)
                        } else {
                            MeshOp::AddFree(mi, c, w, None, Vec::new())
                        });
                    } else if let Some((c, w, _)) = closest {
                        let connect = if self.mesh_autoconnect {
                            // open edges belong to exactly one face
                            let mut count: std::collections::HashMap<(u16, u16), u32> =
                                std::collections::HashMap::new();
                            for t in tris {
                                for (x, y) in [(t[0], t[1]), (t[1], t[2]), (t[2], t[0])] {
                                    *count.entry((x.min(y), x.max(y))).or_insert(0) += 1;
                                }
                            }
                            let mut best: Option<((u16, u16), f32)> = None;
                            for (&(x, y), &n) in &count {
                                if n != 1 {
                                    continue;
                                }
                                let (Some(&px), Some(&py)) =
                                    (screen_pts.get(x as usize), screen_pts.get(y as usize))
                                else {
                                    continue;
                                };
                                let seg = py - px;
                                let len2 = seg.length_sq().max(1e-6);
                                let k = ((ptr - px).dot(seg) / len2).clamp(0.0, 1.0);
                                let d = ptr.distance(px + seg * k);
                                if best.is_none_or(|(_, bd)| d < bd) {
                                    best = Some(((x, y), d));
                                }
                            }
                            best.map(|(e, _)| e)
                        } else {
                            None
                        };
                        mesh_op = Some(MeshOp::AddFree(mi, c, w, connect, Vec::new()));
                    } else {
                        // no faces: the nearest three points that are not in
                        // a line give the placement
                        let mut order: Vec<usize> = (0..screen_pts.len()).collect();
                        order.sort_by(|&a, &b| {
                            let da = screen_pts.get(a).map_or(f32::MAX, |p| p.distance(ptr));
                            let db = screen_pts.get(b).map_or(f32::MAX, |p| p.distance(ptr));
                            da.total_cmp(&db)
                        });
                        let basis = match (order.first(), order.get(1)) {
                            (Some(&a), Some(&b)) => order
                                .iter()
                                .skip(2)
                                .find_map(|&c| bary([a, b, c]).map(|w| ([a, b, c], w))),
                            _ => None,
                        };
                        match basis {
                            Some((c, w)) => {
                                let links = if self.mesh_autoconnect {
                                    vec![c[0] as u16, c[1] as u16]
                                } else {
                                    Vec::new()
                                };
                                mesh_op = Some(MeshOp::AddFree(mi, c, w, None, links));
                            }
                            None => log::warn!(
                                "add point: the mesh's points all lie on one line; place one off it first"
                            ),
                        }
                    }
                }
                // the erase brush: everything the circle sweeps is marked,
                // and the whole stroke goes in one undo step on release
                if self.mesh_mode == 5 && !space {
                    let r = self.tool_opts.brush_size.clamp(4.0, 400.0);
                    let painting = resp.dragged() || resp.drag_started() || resp.clicked();
                    if painting && let Some(ptr) = resp.interact_pointer_pos() {
                        for (vi, sp) in screen_pts.iter().enumerate() {
                            if sp.distance(ptr) <= r {
                                self.erase_marked.insert(vi);
                            }
                        }
                    }
                    if (resp.drag_stopped() || resp.clicked()) && !self.erase_marked.is_empty() {
                        let marked: Vec<usize> = self.erase_marked.iter().copied().collect();
                        mesh_op = Some(MeshOp::EraseMany(mi, marked));
                        self.erase_marked.clear();
                    }
                    // [ and ] size the brush, like every paint program
                    let (shrink, grow) = ui.ctx().input(|i| {
                        (
                            i.key_pressed(egui::Key::OpenBracket),
                            i.key_pressed(egui::Key::CloseBracket),
                        )
                    });
                    if shrink {
                        self.tool_opts.brush_size = (self.tool_opts.brush_size / 1.25).max(4.0);
                    }
                    if grow {
                        self.tool_opts.brush_size = (self.tool_opts.brush_size * 1.25).min(400.0);
                    }
                    if let Some(ptr) = ptr {
                        vp.circle_stroke(ptr, r, Stroke::new(1.2, theme::pal().rotation_red));
                        ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
                    }
                } else {
                    self.erase_marked.clear();
                }
                // connect and cut both work on a pair: first click picks a
                // point, second click acts; clicking empty space resets
                let pairing = matches!(self.mesh_mode, 3 | 4);
                if !space && pairing && resp.clicked() {
                    match (near, self.edge_first) {
                        (Some((vi, _)), None) => self.edge_first = Some(vi),
                        (Some((vi, _)), Some(first)) if vi == first => self.edge_first = None,
                        (Some((vi, _)), Some(first)) => {
                            mesh_op = Some(if self.mesh_mode == 3 {
                                MeshOp::Connect(mi, first as u16, vi as u16)
                            } else {
                                MeshOp::Cut(mi, first as u16, vi as u16)
                            });
                            // connecting chains: the far point becomes the
                            // next start, so an outline draws click by click
                            self.edge_first = (self.mesh_mode == 3).then_some(vi);
                        }
                        (None, _) => self.edge_first = None,
                    }
                }
                if !pairing {
                    self.edge_first = None;
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
                add_mesh_shape(&vp, mesh);

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
                    let (r, fill) = if self.erase_marked.contains(&vi) {
                        (4.0, theme::pal().rotation_red)
                    } else if hot && self.mesh_mode != 5 {
                        (4.0, theme::pal().accent)
                    } else {
                        (2.6, Color32::WHITE)
                    };
                    vp.circle(*sp, r, fill, Stroke::new(1.0, theme::pal().accent));
                }
                if hover_vi.is_some() && !space && self.mesh_mode != 5 {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                }
                for &(x, y) in self.pending_edges.get(&mi).map(|v| v.as_slice()).unwrap_or(&[]) {
                    if let (Some(&px), Some(&py)) =
                        (screen_pts.get(x as usize), screen_pts.get(y as usize))
                    {
                        vp.line_segment([px, py], Stroke::new(1.4, theme::pal().accent));
                    }
                }
                if let Some(first) = self.edge_first
                    && let Some(&fp) = screen_pts.get(first)
                {
                    vp.circle_stroke(fp, 7.0, Stroke::new(1.5, theme::pal().accent));
                    if let Some(ptr) = ptr {
                        vp.add(egui::Shape::dashed_line(
                            &[fp, ptr],
                            Stroke::new(1.2, theme::pal().accent),
                            5.0,
                            3.0,
                        ));
                    }
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
                MeshOp::AddFree(mi, corners, w, connect, links) => {
                    let n =
                        s.puppet.model().art_meshes.get(mi).map_or(0, |m| m.vertex_count) as u16;
                    let ok = s.edit(|m| {
                        ayatsuri_core::edit::add_free_vertex(m, mi, corners, w, connect).is_some()
                    });
                    // auto connect on a faceless mesh: lines to the two
                    // nearest points, closing a face if those two are joined
                    if ok {
                        for l in links {
                            connect_pair(&mut self.pending_edges, s, mi, n, l);
                        }
                    }
                    ok
                }
                MeshOp::Connect(mi, a, b) => connect_pair(&mut self.pending_edges, s, mi, a, b),
                MeshOp::Cut(mi, a, b) => {
                    let key = (a.min(b), a.max(b));
                    let pending = self.pending_edges.entry(mi).or_default();
                    if let Some(at) = pending.iter().position(|&e| e == key) {
                        pending.remove(at);
                        log::debug!("mesh {mi}: line {a}-{b} removed");
                        true
                    } else if ayatsuri_core::edit::has_edge(s.puppet.model(), mi, a, b) {
                        log::debug!("mesh {mi}: edge {a}-{b} cut");
                        s.edit(|m| ayatsuri_core::edit::remove_edge(m, mi, a, b))
                    } else {
                        log::debug!("mesh {mi}: {a} and {b} were not connected");
                        true
                    }
                }
                MeshOp::EraseMany(mi, marked) => {
                    self.pending_edges.remove(&mi);
                    let n = marked.len();
                    let done = s.edit(|m| ayatsuri_core::edit::delete_vertices(m, mi, &marked));
                    if done {
                        log::debug!("mesh {mi}: erase brush removed {n} point(s)");
                    } else {
                        log::warn!("erase brush: a mesh keeps at least three points");
                    }
                    done
                }
                MeshOp::Del(mi, vi) => {
                    // indices shift on delete, so pending lines can't survive it
                    self.pending_edges.remove(&mi);
                    s.edit(|m| ayatsuri_core::edit::delete_vertex(m, mi, vi))
                }
            };
            if !applied {
                log::warn!("mesh edit had nothing to do");
            }
        }

        // glue: weight-coloured points, a yellow tag per glue, and the weight
        // brush while a glue is being edited. Red is mesh A's priority,
        // green is B's, as in Cubism.
        {
            let sel_meshes: Vec<usize> = match self.selected {
                Some(Sel::Mesh(mi)) => {
                    let mut v = self.multi_sel.clone();
                    if !v.contains(&mi) {
                        v.push(mi);
                    }
                    v
                }
                _ => self.multi_sel.clone(),
            };
            let shown: Vec<usize> = s
                .puppet
                .model()
                .glues
                .iter()
                .enumerate()
                .filter(|(gi, g)| {
                    self.selected_glue == Some(*gi)
                        || self.tool == Tool::Glue
                        || sel_meshes.contains(&(g.art_mesh_a as usize))
                        || sel_meshes.contains(&(g.art_mesh_b as usize))
                })
                .map(|(gi, _)| gi)
                .collect();
            let ptr = ui.ctx().input(|i| i.pointer.latest_pos());
            let shift = ui.ctx().input(|i| i.modifiers.shift);
            for gi in shown {
                let Some(g) = s.puppet.model().glues.get(gi) else { continue };
                let (ga, gb) = (g.art_mesh_a as usize, g.art_mesh_b as usize);
                let (Some(pa), Some(pb)) = (
                    s.puppet.mesh_geometry(ga).map(|x| x.positions.to_vec()),
                    s.puppet.mesh_geometry(gb).map(|x| x.positions.to_vec()),
                ) else {
                    continue;
                };
                let (pairs, _) = g.indices.as_chunks::<2>();
                let (weights, _) = g.weights.as_chunks::<2>();
                let editing = self.selected_glue == Some(gi);
                let mut centre = Vec2::ZERO;
                let mut count = 0.0f32;
                let mut pair_pos: Vec<(Pos2, Pos2)> = Vec::with_capacity(pairs.len());
                for (&[ia, ib], &[w0, w1]) in pairs.iter().zip(weights) {
                    let a = (pa.get(ia as usize * 2), pa.get(ia as usize * 2 + 1));
                    let b = (pb.get(ib as usize * 2), pb.get(ib as usize * 2 + 1));
                    let (Some(&ax), Some(&ay)) = a else { continue };
                    let (Some(&bx), Some(&by)) = b else { continue };
                    let (sa, sb) = (to_screen(ax, ay), to_screen(bx, by));
                    pair_pos.push((sa, sb));
                    centre += sa.to_vec2();
                    count += 1.0;
                    // priority: one minus the pull toward the other side
                    let (pa_, pb_) = (1.0 - w0, 1.0 - w1);
                    let t = pa_ / (pa_ + pb_).max(1e-4);
                    let col =
                        Color32::from_rgb((60.0 + 190.0 * t) as u8, (200.0 - 150.0 * t) as u8, 70);
                    if w0 + w1 < 0.98 {
                        // pulled apart by the A or B brush
                        vp.line_segment([sa, sb], Stroke::new(1.0, col));
                        vp.circle_filled(sb, if editing { 3.0 } else { 2.0 }, col);
                    }
                    vp.circle_filled(sa, if editing { 3.5 } else { 2.4 }, col);
                }
                // the yellow tag, clickable to start editing this glue
                if count > 0.0 {
                    let at = (centre / count).to_pos2() + vec2(0.0, -14.0);
                    let tag = Rect::from_center_size(at, vec2(36.0, 14.0));
                    let tresp = ui.interact(tag, egui::Id::new(("glue-tag", gi)), Sense::click());
                    vp.rect_filled(tag, 3.0, Color32::from_rgb(0xE8, 0xC6, 0x3B));
                    vp.text(
                        tag.center(),
                        Align2::CENTER_CENTER,
                        "Glue",
                        FontId::proportional(10.0),
                        Color32::BLACK,
                    );
                    if editing {
                        vp.rect_stroke(
                            tag,
                            3.0,
                            Stroke::new(1.5, Color32::WHITE),
                            StrokeKind::Outside,
                        );
                    }
                    if tresp.clicked() {
                        self.selected_glue = Some(gi);
                        self.tool = Tool::Glue;
                    }
                    tresp.context_menu(|ui| {
                        if ui.button("Delete glue").clicked() {
                            self.selected_glue = Some(gi);
                            ui.close();
                        }
                    });
                }
                // the weight brush: strength builds while dragging, lands on
                // release as one undo step
                if editing && self.tool == Tool::Glue && !space {
                    let r = self.tool_opts.brush_size.clamp(4.0, 400.0);
                    if let Some(p) = ptr {
                        vp.circle_stroke(
                            p,
                            r,
                            Stroke::new(1.2, Color32::from_rgb(0xE8, 0xC6, 0x3B)),
                        );
                    }
                    if (resp.dragged() || resp.drag_started())
                        && let Some(p) = resp.interact_pointer_pos()
                    {
                        let dt = ui.ctx().input(|i| i.stable_dt).clamp(0.001, 0.1);
                        for (pi, (sa, sb)) in pair_pos.iter().enumerate() {
                            let d = sa.distance(p).min(sb.distance(p));
                            if d <= r {
                                let falloff = 1.0 - d / r;
                                let e = self.glue_stroke.entry(pi).or_insert(0.0);
                                *e = (*e + self.glue_strength * falloff * dt * 3.0).min(1.0);
                            }
                        }
                    }
                    for (pi, k) in &self.glue_stroke {
                        if let Some((sa, _)) = pair_pos.get(*pi) {
                            vp.circle_stroke(*sa, 3.0 + 4.0 * k, Stroke::new(1.0, Color32::WHITE));
                        }
                    }
                    if resp.drag_stopped() && !self.glue_stroke.is_empty() {
                        let hits: Vec<(usize, f32)> = self.glue_stroke.drain().collect();
                        let brush = match self.glue_mode {
                            1 => ayatsuri_core::edit::GlueBrush::OnlyA,
                            2 => ayatsuri_core::edit::GlueBrush::OnlyB,
                            3 => ayatsuri_core::edit::GlueBrush::Remerge,
                            _ => ayatsuri_core::edit::GlueBrush::Balance { favor_a: !shift },
                        };
                        if s.edit(|m| ayatsuri_core::edit::glue_brush(m, gi, &hits, brush)) {
                            log::debug!(
                                "glue {gi}: weight brush on {} pair(s), {brush:?}",
                                hits.len()
                            );
                        }
                    }
                }
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
/// Pin a point to the mesh: the triangle it sits in, else the triangle
/// it is closest to (weights then extrapolate), as barycentric weights.
fn pin_at(pts: &[Pos2], idx: &[u16], p: Pos2) -> Option<PathPin> {
    let mut best: Option<(PathPin, f32)> = None;
    for (tri, t) in idx.as_chunks::<3>().0.iter().enumerate() {
        let (Some(&a), Some(&b), Some(&c)) =
            (pts.get(t[0] as usize), pts.get(t[1] as usize), pts.get(t[2] as usize))
        else {
            continue;
        };
        let den = (b.y - c.y) * (a.x - c.x) + (c.x - b.x) * (a.y - c.y);
        if den.abs() < 1e-6 {
            continue;
        }
        let w0 = ((b.y - c.y) * (p.x - c.x) + (c.x - b.x) * (p.y - c.y)) / den;
        let w1 = ((c.y - a.y) * (p.x - c.x) + (a.x - c.x) * (p.y - c.y)) / den;
        let w = [w0, w1, 1.0 - w0 - w1];
        let worst = w[0].min(w[1]).min(w[2]);
        if best.as_ref().is_none_or(|(_, bw)| worst > *bw) {
            best = Some((PathPin { tri, w }, worst));
        }
    }
    best.map(|(p, _)| p)
}

/// Where a pinned path point sits on the mesh now.
fn pin_pos(pin: PathPin, pts: &[Pos2], idx: &[u16]) -> Option<Pos2> {
    let t = idx.get(pin.tri * 3..pin.tri * 3 + 3)?;
    let mut out = Vec2::ZERO;
    for (&v, &w) in t.iter().zip(&pin.w) {
        out += pts.get(v as usize)?.to_vec2() * w;
    }
    Some(out.to_pos2())
}

fn nearest_within(pts: &[Pos2], p: Pos2, reach: f32) -> Option<usize> {
    pts.iter()
        .enumerate()
        .map(|(i, q)| (i, q.distance(p)))
        .filter(|(_, d)| *d < reach)
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(i, _)| i)
}

/// A deform path: the line, control points, and around each point the red
/// width circle with the blue hardness circle inside, as in Cubism.
fn draw_path(vp: &egui::Painter, pts: &[Pos2], width: f32, hardness: f32, hot: Option<usize>) {
    if pts.len() > 1 {
        vp.add(egui::Shape::line(pts.to_vec(), Stroke::new(1.6, theme::pal().warp_green)));
    }
    for (i, &p) in pts.iter().enumerate() {
        vp.circle_stroke(p, width, Stroke::new(1.0, theme::pal().rotation_red.gamma_multiply(0.5)));
        vp.circle_stroke(
            p,
            width * hardness.clamp(0.0, 1.0),
            Stroke::new(1.0, Color32::from_rgb(0x4E, 0x8F, 0xD9).gamma_multiply(0.6)),
        );
        let r = if hot == Some(i) { 5.0 } else { 3.6 };
        vp.circle(p, r, theme::pal().warp_green, Stroke::new(1.0, Color32::WHITE));
    }
}

/// A brush selection's weights: each painted point in the weight colour,
/// as strong as its weight.
fn draw_weights(vp: &egui::Painter, pts: &[Pos2], w: Option<&Vec<f32>>, col: Color32) {
    let Some(w) = w else { return };
    for (p, &wt) in pts.iter().zip(w) {
        if wt > 1e-3 {
            vp.circle_filled(*p, 3.0 + 3.0 * wt, col.gamma_multiply(wt.clamp(0.0, 1.0)));
        }
    }
}

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
/// Every shortcut, with Cubism's defaults (docs.live2d.com shortcut list):
/// (category, action id, label, modifiers, key). Rebindable in Settings.
pub(crate) const KEY_ACTIONS: &[(&str, &str, &str, egui::Modifiers, egui::Key)] = {
    use egui::Key as K;
    use egui::Modifiers as M;
    const C: M = M::COMMAND;
    const CS: M = M::COMMAND.plus(M::SHIFT);
    const CA: M = M::COMMAND.plus(M::ALT);
    const CAS: M = M::COMMAND.plus(M::ALT).plus(M::SHIFT);
    const N: M = M::NONE;
    &[
        ("File", "new-model", "New model", C, K::N),
        ("File", "open", "Open", C, K::O),
        ("File", "save", "Save", C, K::S),
        ("File", "save-as", "Save as", CS, K::S),
        ("File", "close", "Close", C, K::W),
        ("File", "export-image", "Export image", C, K::I),
        ("File", "export-moc3", "Export as moc3", CA, K::S),
        ("File", "shortcuts", "Keyboard shortcuts", CAS, K::K),
        ("Edit", "undo", "Undo", C, K::Z),
        ("Edit", "redo", "Redo", CS, K::Z),
        ("Edit", "redo-alt", "Redo (alternate)", C, K::Y),
        ("Edit", "select-parent", "Select deformer for parent", C, K::D),
        ("Modeling", "mesh-edit", "Edit mesh", C, K::E),
        ("Modeling", "auto-mesh", "Automatic mesh generator", CS, K::A),
        ("Modeling", "auto-connect", "Auto connect edges", C, K::R),
        ("Modeling", "glue-bind", "Bind glue", C, K::G),
        ("Modeling", "glue-unbind", "Unbind glue", CS, K::G),
        ("Modeling", "reset-params", "Reset parameters to default", C, K::Num1),
        ("Modeling", "four-corners", "Generate four corner forms", C, K::Num4),
        ("Modeling", "lock-selected", "Lock/unlock selected", C, K::Num2),
        ("Modeling", "toggle-visible", "Show/hide selected", C, K::Num3),
        ("Modeling", "hide-unselected", "Hide unselected", CS, K::Num3),
        ("Modeling", "show-all", "Show all", CA, K::Num3),
        ("Tools", "tool-arrow", "Arrow tool", N, K::A),
        ("Tools", "tool-lasso", "Lasso tool", N, K::L),
        ("Tools", "tool-brush", "Brush selection tool", N, K::B),
        ("Tools", "tool-path", "Deform path tool", N, K::P),
        ("Tools", "level-1", "Edit level 1", N, K::Num1),
        ("Tools", "level-2", "Edit level 2", N, K::Num2),
        ("Tools", "level-3", "Edit level 3", N, K::Num3),
        ("View", "display-all", "Display all (fit)", C, K::Num0),
    ]
};

pub(crate) fn format_shortcut(sc: &egui::KeyboardShortcut) -> String {
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

/// Join two points of a mesh: a face closes when the pair shares a
/// neighbour (by face edges or drawn lines); otherwise the line is kept as
/// drawn until one does. True when anything changed.
fn connect_pair(
    pending: &mut std::collections::HashMap<usize, Vec<(u16, u16)>>,
    s: &mut Session,
    mi: usize,
    a: u16,
    b: u16,
) -> bool {
    let key = (a.min(b), a.max(b));
    if a == b
        || ayatsuri_core::edit::has_edge(s.puppet.model(), mi, a, b)
        || pending.get(&mi).is_some_and(|p| p.contains(&key))
    {
        log::debug!("mesh {mi}: {a} and {b} are already connected");
        return false;
    }
    let lines = pending.get(&mi).cloned().unwrap_or_default();
    let closed = s.edit(|m| ayatsuri_core::edit::add_edge(m, mi, a, b, &lines));
    let list = pending.entry(mi).or_default();
    if closed {
        // lines a face now carries are no longer pending
        list.retain(|&(x, y)| !ayatsuri_core::edit::has_edge(s.puppet.model(), mi, x, y));
        log::debug!("mesh {mi}: connecting {a}-{b} closed a face");
    } else {
        list.push(key);
        log::debug!("mesh {mi}: line {a}-{b} drawn, fills once it closes a triangle");
    }
    true
}

/// Hand egui a mesh only when it has something to draw: egui-wgpu panics
/// slicing an empty index buffer, and it does so while rendering, outside
/// the frame guard. A mesh whose faces were all cut is exactly that case.
fn add_mesh_shape(painter: &egui::Painter, mesh: egui::Mesh) {
    if mesh.indices.is_empty() || mesh.vertices.is_empty() {
        return;
    }
    // indices past the vertex list would be the next panic; drop the mesh
    let n = mesh.vertices.len() as u32;
    if mesh.indices.iter().any(|&i| i >= n) {
        log::warn!("skipped drawing a mesh with out-of-range indices");
        return;
    }
    painter.add(egui::Shape::mesh(mesh));
}

/// One mesh row in the Parts palette: eye, dot, name, live stack position.
#[allow(clippy::too_many_arguments)]
fn parts_mesh_row(
    ui: &mut Ui,
    s: &mut Session,
    selected: &mut Option<Sel>,
    mi: usize,
    mid: &str,
    x_off: f32,
    parent_effective: bool,
    stack: &std::collections::HashMap<usize, usize>,
) {
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 16.0), Sense::click());
    if *selected == Some(Sel::Mesh(mi)) {
        ui.painter().rect_filled(rect, 0.0, theme::pal().select);
    }
    let mesh_on = s.mesh_shown.get(mi).copied().unwrap_or(true);
    let eye = Rect::from_min_size(rect.min, vec2(18.0, 16.0));
    paint_eye(ui, eye, mesh_on, mesh_on && parent_effective);
    let eresp = ui.interact(eye, egui::Id::new(("mesh-eye", mi)), Sense::click());
    if eresp.clicked() {
        let now = !mesh_on;
        if let Some(f) = s.mesh_shown.get_mut(mi) {
            *f = now;
        }
        s.puppet.set_mesh_visible(mi, now);
        s.dirty = true;
    }
    let x = rect.min.x + x_off;
    ui.painter().circle_filled(pos2(x, rect.center().y), 2.0, theme::pal().text_faint);
    // names come first: the stack column only shows once a name has room
    // for a dozen glyphs beside it (about 6px a glyph)
    let glyph = 6.2;
    let with_stack = (rect.max.x - 44.0 - (x + 8.0)).max(0.0);
    let show_stack = with_stack / glyph >= 12.0;
    let room = if show_stack { with_stack } else { (rect.max.x - 6.0 - (x + 8.0)).max(0.0) };
    ui.painter().text(
        pos2(x + 8.0, rect.center().y),
        Align2::LEFT_CENTER,
        truncate(mid, ((room / glyph) as usize).max(3)),
        FontId::proportional(10.5),
        theme::pal().text,
    );
    if show_stack {
        // the live stack position, what actually decides overlap
        let pos_label = stack.get(&mi).map(|p| format!("#{p}")).unwrap_or_else(|| "–".into());
        ui.painter().text(
            rect.right_center() - vec2(6.0, 0.0),
            Align2::RIGHT_CENTER,
            pos_label,
            FontId::proportional(10.0),
            theme::pal().text_faint,
        );
    }
    if resp.clicked() {
        *selected = Some(Sel::Mesh(mi));
    }
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

/// The front-most mesh under a model-space point. Locked meshes, hidden
/// ones and fully transparent ones can't be picked: you can't select what
/// you can't see.
fn hit_mesh(
    puppet: &ayatsuri_core::Puppet,
    ux: f32,
    uy: f32,
    skip: &dyn Fn(usize) -> bool,
) -> Option<usize> {
    let order: Vec<_> = puppet.drawables().filter(|d| d.opacity > 0.004).map(|d| d.mesh).collect();
    for &mi in order.iter().rev() {
        if skip(mi) {
            continue;
        }
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
    Cmd(ParamCmd),
}

fn parameter_row(
    ui: &mut Ui,
    s: &mut Session,
    pi: usize,
    sel_param: &mut Option<usize>,
    can_edit: bool,
    subject: Option<param_menu::Moving>,
    full: bool,
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
        // Cubism's "link parameters below": joins this row with the next
        // one into a two-axis pad
        let (lr, lresp) = ui.allocate_exact_size(
            vec2(14.0, 16.0),
            if full { Sense::click() } else { Sense::hover() },
        );
        let lcol = if lresp.hovered() { theme::pal().text } else { theme::pal().text_faint };
        if full {
            ui.painter().text(
                lr.center(),
                Align2::CENTER_CENTER,
                egui_phosphor::regular::LINK_SIMPLE,
                FontId::proportional(11.0),
                lcol.gamma_multiply(if lresp.hovered() { 1.0 } else { 0.6 }),
            );
            if lresp.on_hover_text("Link with the parameter below (2D pad)").clicked() {
                actions.push(ParamAction::Cmd(ParamCmd::Combine(pi)));
            }
        }
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
        if full {
            nresp.context_menu(|ui| {
                if let Some(c) = param_menu::name_menu(ui, s, pi, subject) {
                    actions.push(ParamAction::Cmd(c));
                }
            });
        }
        let slider_w = (ui.available_width() - 68.0).max(40.0);
        let (changed, slider_resp) = param_slider(ui, slider_w, &mut v, min, max, default, &keys);
        if slider_resp.is_pointer_button_down_on() {
            s.params_held.push(pi);
        }
        if changed {
            s.puppet.set_value(pi, v);
            s.dirty = true;
        }
        if slider_resp.drag_stopped() {
            log::debug!("parameter {display} dragged to {v:.3}");
        }
        if full {
            slider_resp.context_menu(|ui| {
                if let Some(c) = param_menu::slider_menu(ui, pi, v, can_edit) {
                    actions.push(ParamAction::Cmd(c));
                }
            });
        }
        let dec = decimals.clamp(1, 4) as usize;
        let step = 0.1f32.powi(dec as i32);
        let mut dv = v;
        let vresp = ui.add_sized(
            [52.0, 16.0],
            egui::DragValue::new(&mut dv)
                .speed(step)
                .range(min..=max)
                .min_decimals(dec)
                .max_decimals(dec),
        );
        if vresp.is_pointer_button_down_on() || vresp.has_focus() {
            s.params_held.push(pi);
        }
        if vresp.changed() {
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
    can_edit: bool,
    subject: Option<param_menu::Moving>,
    menus: bool,
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
    let slider_w = (avail - 14.0 - name_w - value_w - 18.0).max(60.0);
    let side = 96.0_f32.min(slider_w);
    let (full, _) = ui.allocate_exact_size(vec2(avail, side + 20.0), Sense::hover());
    // the bottom 10 px stay empty so the next row never crowds the pad
    let block = Rect::from_min_max(full.min, pos2(full.max.x, full.max.y - 10.0));
    let cy = block.center().y;

    // names align with the pad's top and bottom edges, like Cubism
    let edge = (side - 6.0) * 0.5 - 8.0;
    for (p, label, dy) in [(pa, &name_a, -edge), (pb, &name_b, edge)] {
        let nr = Rect::from_min_size(pos2(block.min.x + 14.0, cy + dy - 8.0), vec2(name_w, 16.0));
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
        if menus {
            nresp.context_menu(|ui| {
                if let Some(c) = param_menu::name_menu(ui, s, p, subject) {
                    actions.push(ParamAction::Cmd(c));
                }
            });
        }
    }

    // the pad, centered in the slider column
    // the unlink button sits where the link button was, between the names
    let ur = Rect::from_center_size(pos2(block.min.x + 7.0, cy), vec2(14.0, 16.0));
    let uresp = ui.interact(ur, egui::Id::new(("pad-unlink", pa)), Sense::click());
    ui.painter().text(
        ur.center(),
        Align2::CENTER_CENTER,
        egui_phosphor::regular::LINK_SIMPLE_BREAK,
        FontId::proportional(11.0),
        if uresp.hovered() { theme::pal().text } else { theme::pal().text_faint },
    );
    if menus && uresp.on_hover_text("Unlink into two sliders").clicked() {
        actions.push(ParamAction::Cmd(ParamCmd::Uncombine(pa)));
    }
    let col_left = block.min.x + 14.0 + name_w + 6.0;
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
    if resp.is_pointer_button_down_on() {
        s.params_held.push(pa);
        s.params_held.push(pb);
    }
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
        // the slider popup for each of the pad's two parameters
        ui.menu_button(truncate(&name_a, 18), |ui| {
            if let Some(c) = param_menu::slider_menu(ui, pa, va, can_edit) {
                actions.push(ParamAction::Cmd(c));
            }
        });
        ui.menu_button(truncate(&name_b, 18), |ui| {
            if let Some(c) = param_menu::slider_menu(ui, pb, vb, can_edit) {
                actions.push(ParamAction::Cmd(c));
            }
        });
        ui.separator();
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
        let vresp = ui.put(
            vr,
            egui::DragValue::new(&mut dv).speed(step).min_decimals(digits).max_decimals(digits),
        );
        if vresp.is_pointer_button_down_on() || vresp.has_focus() {
            s.params_held.push(p);
        }
        if vresp.changed() {
            s.puppet.set_value(p, dv);
            s.dirty = true;
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) struct WashKey {
    pub(crate) w: u32,
    pub(crate) h: u32,
    pub(crate) base: [u8; 4],
    pub(crate) accent: [u8; 4],
}

/// The header wash as an opaque image: the accent laid over the header
/// colour at 6% on the left, easing to nothing on the right, with triangular
/// noise of about one shade so no stripes form.
pub(crate) fn dithered_wash(k: &WashKey) -> egui::ColorImage {
    let [br, bg, bb, _] = k.base;
    let [ar, ag, ab, _] = k.accent;
    let (w, h) = (k.w as usize, k.h as usize);
    let mut px = Vec::with_capacity(w.saturating_mul(h));
    for y in 0..h {
        for x in 0..w {
            let t = x as f32 / w.max(2).saturating_sub(1) as f32;
            let a = 0.06 * (1.0 - t * t * (3.0 - 2.0 * t));
            // two hashed uniforms summed give triangular noise in -1..1
            let n = noise(x as u32, y as u32, 0) + noise(x as u32, y as u32, 1) - 1.0;
            let mix = |b: u8, c: u8| {
                (f32::from(b) + (f32::from(c) - f32::from(b)) * a + n).round().clamp(0.0, 255.0)
                    as u8
            };
            px.push(Color32::from_rgb(mix(br, ar), mix(bg, ag), mix(bb, ab)));
        }
    }
    egui::ColorImage { size: [w, h], source_size: vec2(w as f32, h as f32), pixels: px }
}

/// A cheap integer hash to 0..1, stable per pixel so the grain never crawls.
fn noise(x: u32, y: u32, salt: u32) -> f32 {
    let mut v =
        x.wrapping_mul(0x8da6_b343) ^ y.wrapping_mul(0xd816_3841) ^ salt.wrapping_mul(0xcb1a_b31f);
    v ^= v >> 13;
    v = v.wrapping_mul(0x5bd1_e995);
    v ^= v >> 15;
    (v & 0xffff) as f32 / 65535.0
}

/// What a home card's context menu asked for, applied after the card loop.
enum HomeCmd {
    Open(PathBuf),
    OpenNewWindow(PathBuf),
    ShowInFolder(PathBuf),
    CopyPath(PathBuf),
    Rename(PathBuf),
    Duplicate(PathBuf),
    Remove(PathBuf),
    RemoveMissing,
    ClearAll,
}

fn home_card_menu(ui: &mut Ui, path: &Path, exists: bool, cmd: &mut Option<HomeCmd>) {
    let p = || path.to_path_buf();
    let mut pick = |ui: &mut Ui, label: &str, enabled: bool, c: HomeCmd| {
        if ui.add_enabled(enabled, egui::Button::new(label)).clicked() {
            *cmd = Some(c);
            ui.close();
        }
    };
    pick(ui, "Open", exists, HomeCmd::Open(p()));
    pick(ui, "Open in New Window", exists, HomeCmd::OpenNewWindow(p()));
    ui.separator();
    pick(ui, "Show in Folder", exists, HomeCmd::ShowInFolder(p()));
    pick(ui, "Copy Path", true, HomeCmd::CopyPath(p()));
    ui.separator();
    pick(ui, "Rename…", exists, HomeCmd::Rename(p()));
    pick(ui, "Duplicate", exists, HomeCmd::Duplicate(p()));
    ui.separator();
    pick(ui, "Remove from Recent", true, HomeCmd::Remove(p()));
    pick(ui, "Remove Missing Files", true, HomeCmd::RemoveMissing);
    pick(ui, "Clear Recent", true, HomeCmd::ClearAll);
}

/// Everything after the first dot of the file name, dot included, so
/// `ariu.model3.json` keeps `.model3.json` whole.
fn full_extension(path: &Path) -> String {
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    name.find('.').filter(|&i| i > 0).map(|i| name[i..].to_string()).unwrap_or_default()
}

/// The file name without its full extension, as the cards show it.
fn card_name(path: &Path) -> String {
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let ext = full_extension(path);
    name.strip_suffix(ext.as_str()).unwrap_or(&name).to_string()
}

fn rename_file(path: &Path, stem: &str) -> std::io::Result<PathBuf> {
    use std::io::{Error, ErrorKind};
    if stem.is_empty() || stem.contains(['/', '\\']) || stem == "." || stem == ".." {
        return Err(Error::new(ErrorKind::InvalidInput, "that is not a valid file name"));
    }
    let target = path.with_file_name(format!("{stem}{}", full_extension(path)));
    if target == path {
        return Ok(target);
    }
    if target.exists() {
        return Err(Error::new(ErrorKind::AlreadyExists, "a file with that name already exists"));
    }
    let old_thumb = crate::thumb::cache_path(path);
    std::fs::rename(path, &target)?;
    // a rename keeps size and mtime, so the cached preview carries over
    if let (Some(from), Some(to)) = (old_thumb, crate::thumb::cache_path(&target)) {
        let _ = std::fs::rename(from, to);
    }
    Ok(target)
}

/// `name copy.ext`, then `name copy 2.ext` and so on, never overwriting.
fn duplicate_file(path: &Path) -> std::io::Result<PathBuf> {
    let (stem, ext) = (card_name(path), full_extension(path));
    let target = (1..1000)
        .map(|n| {
            let suffix = if n == 1 { " copy".to_string() } else { format!(" copy {n}") };
            path.with_file_name(format!("{stem}{suffix}{ext}"))
        })
        .find(|t| !t.exists())
        .ok_or_else(|| std::io::Error::other("too many copies"))?;
    std::fs::copy(path, &target)?;
    Ok(target)
}

/// Open the system file manager with the file selected where the platform
/// allows it, else on its folder.
fn show_in_folder(path: &Path) -> std::io::Result<()> {
    use std::process::Command;
    if cfg!(target_os = "macos") {
        return Command::new("open").arg("-R").arg(path).spawn().map(|_| ());
    }
    if cfg!(target_os = "windows") {
        let mut arg = std::ffi::OsString::from("/select,");
        arg.push(path);
        return Command::new("explorer").arg(arg).spawn().map(|_| ());
    }
    // the freedesktop file manager interface selects the file (Dolphin,
    // Nautilus, Nemo, Thunar...); plain xdg-open on the folder otherwise
    let uri = format!("file://{}", path.display());
    let selected = Command::new("dbus-send")
        .args([
            "--session",
            "--print-reply",
            "--dest=org.freedesktop.FileManager1",
            "/org/freedesktop/FileManager1",
            "org.freedesktop.FileManager1.ShowItems",
        ])
        .arg(format!("array:string:{uri}"))
        .arg("string:")
        .output()
        .is_ok_and(|o| o.status.success());
    if selected {
        return Ok(());
    }
    let folder = path.parent().unwrap_or(path);
    Command::new("xdg-open").arg(folder).spawn().map(|_| ())
}

#[cfg(test)]
mod home_file_tests {
    use super::*;

    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("ayatsuri-home-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn extensions_stay_whole() {
        assert_eq!(full_extension(Path::new("/m/ariu.model3.json")), ".model3.json");
        assert_eq!(card_name(Path::new("/m/ariu.model3.json")), "ariu");
        assert_eq!(full_extension(Path::new("/m/rig.aom")), ".aom");
        assert_eq!(card_name(Path::new("/m/.hidden")), ".hidden");
        assert_eq!(card_name(Path::new("/m/plain")), "plain");
    }

    #[test]
    fn rename_keeps_extension_and_refuses_clobber() {
        let dir = scratch("rename");
        let a = dir.join("a.model3.json");
        std::fs::write(&a, b"{}").unwrap();
        std::fs::write(dir.join("taken.model3.json"), b"{}").unwrap();
        let b = rename_file(&a, "b").unwrap();
        assert_eq!(b, dir.join("b.model3.json"));
        assert!(b.exists() && !a.exists());
        assert!(rename_file(&b, "taken").is_err());
        assert!(rename_file(&b, "../escape").is_err());
        assert!(rename_file(&b, "").is_err());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn duplicate_never_overwrites() {
        let dir = scratch("dup");
        let a = dir.join("rig.aom");
        std::fs::write(&a, b"x").unwrap();
        assert_eq!(duplicate_file(&a).unwrap(), dir.join("rig copy.aom"));
        assert_eq!(duplicate_file(&a).unwrap(), dir.join("rig copy 2.aom"));
        let _ = std::fs::remove_dir_all(dir);
    }
}
