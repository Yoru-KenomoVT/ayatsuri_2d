use std::path::{Path, PathBuf};

use ayatsuri_core::edit::Child;
use ayatsuri_format::Model;
use ayatsuri_format::model::DeformerKind;
use egui::{
    Align2, CentralPanel, Color32, FontId, Frame, Panel, Pos2, Rect, ScrollArea, Sense, Stroke,
    StrokeKind, Ui, UiBuilder, Vec2, pos2, vec2,
};

use crate::dock::{self, DockAction, DockNode, PanelKind};
use crate::point_edit;

mod api_bar;
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
    /// The layout inside: Cubism's lattice, or a face's rings and hubs.
    auto_style: ayatsuri_core::automesh::Style,
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
            auto_style: ayatsuri_core::automesh::Style::Lattice,
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
            Workspace::Modeling => crate::i18n::tr("Modeling").into(),
            Workspace::Animation => crate::i18n::tr("Animation").into(),
            Workspace::Custom(i) => prefs
                .custom_workspaces
                .get(*i)
                .map(|(n, _)| n.clone())
                .unwrap_or_else(|| crate::i18n::tr("Custom").into()),
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
    /// The start-up offer to download ffmpeg when there is none.
    ffmpeg_prompt: crate::ffmpeg::Prompt,
    pub(crate) about: crate::windows::about::AboutState,
    /// What's known about newer releases (launcher's check, About's check).
    pub(crate) update: crate::update::UpdateState,
    /// An export waiting on "overwrite the model already in that folder?":
    /// the options, the folder, and the model3.json names found there.
    pub(crate) export_overwrite: Option<(ExportDlg, PathBuf, Vec<String>)>,
    pub(crate) show_settings: bool,
    pub(crate) show_api_window: bool,
    /// The keyboard settings' search text.
    pub(crate) key_filter: String,
    /// A file picker open on its own thread, and a model loading on one.
    picking: Option<std::sync::mpsc::Receiver<BgJob>>,
    /// Any other native file dialog in flight, and what it is for.
    dialog_pick: Option<(PickFor, std::sync::mpsc::Receiver<Option<PathBuf>>)>,
    loading: Option<(PathBuf, std::sync::mpsc::Receiver<BgJob>)>,

    /// Discord Rich Presence while enabled, and the app id it runs as.
    rpc: Option<crate::discord::Presence>,
    rpc_id: String,

    dev_physics_done: bool,
    /// One-time startup work (fullscreen) has run.
    startup_done: bool,

    pub(crate) show_stats: bool,
    /// Window > Timelapse, while open.
    timelapse_view: Option<crate::timelapse::View>,
    /// File > Export Animation / Export Timelapse, while open.
    pub(crate) render_win: Option<crate::windows::render::RenderWin>,
    /// Modeling > Texture Atlas, while open.
    pub(crate) atlas_win: Option<crate::windows::atlas::AtlasWin>,
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
    pub(crate) physics_on: bool,
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
    timeline: crate::timeline::Timeline,
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
    /// Parameters the New Folder dialog files into the folder it makes.
    folder_from: Vec<usize>,
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
    /// The deformer dev hook has run, and has logged its grid.
    dev_def_done: bool,
    dev_def_logged: bool,
    /// The art Mesh Edit shows, frozen on entry.
    mesh_art: Option<MeshArt>,
    /// The Mesh Edit dev hook has run, and has logged its vertices.
    dev_mesh_done: bool,
    dev_mesh_logged: bool,
    /// A mesh or warp point drag being shown live on the model.
    live_drag: Option<LiveDrag>,
    /// A rotation being turned live: the deformer and its angles as they were.
    rot_live: Option<(usize, Vec<f32>)>,
    /// Set by "Reset Toolbar Position": put the floating bar back next frame.
    toolbar_reset: bool,
    /// The Rename Deformer dialog: which deformer and the name typed.
    def_rename: Option<(usize, String)>,
    /// Undo depth at the start of the frame; see `on_crash`.
    frame_undo_depth: usize,
    /// The Ayatsuri API server and its editor-side state.
    pub(crate) api: api_bar::ApiState,
    pub(crate) recovery: crate::windows::recovery::RecoveryState,
    /// A recovery file being opened: it loads as unsaved work.
    pub(crate) recovering: Option<PathBuf>,
    tool_opts: ToolOptions,
    lasso: Vec<Pos2>,
    /// The parameter rows' own selection, independent of object selection.
    sel_param: Option<usize>,
    param_edit: Option<ParamEdit>,
    pmenu: param_menu::ParamMenuState,
    deformer_dialog: Option<deformer_dialog::DeformerDialog>,
    /// Extra meshes picked by the lasso, beyond `selected`.
    multi_sel: Vec<usize>,
    /// Parts picked in the Parts palette (Ctrl toggles, Shift a range), and
    /// the rows each Shift range starts from.
    part_pick: Vec<usize>,
    part_anchor: Option<usize>,
    /// The Parts palette row under the pointer, outlined on the canvas.
    palette_hover: Option<Sel>,
    /// The Parts and Deformer palettes' search boxes.
    part_search: String,
    def_search: String,
    mesh_anchor: Option<usize>,
    /// Picking clipping masks on the canvas for this mesh: each click on
    /// another mesh adds it (Shift keeps picking), Esc ends.
    clip_pick: Option<usize>,
    /// A deformer creation requested from inside a borrow (canvas, tool pane).
    pending_create: Option<(bool, Option<(f32, f32)>)>,
    /// A guide line asked for from a menu, placed once the canvas is known.
    pending_guide: Option<GuideCmd>,
    /// Where the canvas was last right-clicked, for "add a guide here".
    ctx_click: Option<Pos2>,
    show_guide_settings: bool,
    /// F: frame the selection once the canvas size is known.
    focus_request: bool,
    /// Ctrl+F: put the cursor in the Parts search box.
    focus_search: bool,
    /// Copy Form: (1 mesh / 2 warp, local positions).
    form_clip: Option<(u8, Vec<f32>)>,
    /// Mesh edit mode: the mesh whose vertices are being edited, if any.
    mesh_edit: Option<usize>,
    mesh_drag: Option<MeshDrag>,
    /// 0 move, 1 add vertices, 2 erase vertices, 3 connect points, 4 cut
    /// edges, 5 erase brush, 6 select points (lasso, copy and paste).
    mesh_mode: u8,
    /// Points the erase brush has swept over during the current stroke.
    erase_marked: std::collections::BTreeSet<usize>,
    /// Mesh Edit's picked points (Select points mode), on `mesh_picked_on`.
    mesh_picked: std::collections::BTreeSet<usize>,
    mesh_picked_on: Option<usize>,
    /// The Select points lasso being drawn, in screen points.
    mesh_lasso: Vec<Pos2>,
    /// Points copied in Mesh Edit, ready to paste into any mesh.
    point_clip: Option<PointClip>,
    /// Edit > Copy (true) or Paste (false) asked for from the menu.
    point_clip_cmd: Option<bool>,
    /// Art meshes copied or cut, as they were then, ready to paste; the
    /// same goes on the system clipboard as text.
    mesh_clip: Option<crate::mesh_clip::ClipFile>,
    /// Edit > Cut, Copy or Paste asked for from the menu, outside Mesh Edit.
    mesh_clip_cmd: Option<ClipCmd>,
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
    art_draft: Option<ayatsuri_format::model::ArtPath>,
    art_grab: Option<ArtGrab>,
    art_edit: Option<(usize, ayatsuri_format::model::ArtPath)>,
    art_style: ayatsuri_format::model::ArtPath,
    art_finish: bool,
    art_pick: Option<u8>,
    clip_with: Option<usize>,
    clip_search: String,
    dbrush: Option<DeformStroke>,
    dbrush_hardness: f32,
    dbrush_strength: f32,
    show_guides: bool,
    /// Mirror edit: point moves repeat, flipped, on the point across the
    /// vertical axis (a vertical guide line nearest the centre, else the
    /// canvas centre).
    mirror_edit: bool,
    guide_drag: Option<usize>,
    clip_invert: bool,
    art_pending: Option<(usize, ayatsuri_format::model::ArtPath)>,
    pick_pages: std::collections::HashMap<usize, (usize, u32, u32, Vec<u8>)>,
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

/// Edit > Cut, Copy or Paste, from the menu, for art meshes.
#[derive(Clone, Copy, PartialEq, Eq)]
enum ClipCmd {
    Cut,
    Copy,
    Paste,
}

/// One vertex being dragged in mesh edit mode. The parent-space offset is
/// banked as the pointer moves and applied as a single edit on release.
/// Points copied in Mesh Edit: where they were on the canvas (model units,
/// as drawn) and the faces among them, by index into `points`.
struct PointClip {
    points: Vec<Pos2>,
    faces: Vec<[usize; 3]>,
    from: String,
}

/// Three points of `pts` around `p` that are not in a line, with `p`'s
/// weights on them: how a point is placed on a mesh with no faces.
fn basis_weights(pts: &[Pos2], p: Pos2) -> Option<([usize; 3], [f32; 3])> {
    let mut order: Vec<usize> = (0..pts.len()).collect();
    order.sort_by(|&a, &b| {
        let da = pts.get(a).map_or(f32::MAX, |q| q.distance(p));
        let db = pts.get(b).map_or(f32::MAX, |q| q.distance(p));
        da.total_cmp(&db)
    });
    let (&a, &b) = (order.first()?, order.get(1)?);
    order.iter().skip(2).find_map(|&c| {
        let (s0, s1, s2) = (*pts.get(a)?, *pts.get(b)?, *pts.get(c)?);
        let den = (s1.y - s2.y) * (s0.x - s2.x) + (s2.x - s1.x) * (s0.y - s2.y);
        if den.abs() < 1e-9 {
            return None;
        }
        let w0 = ((s1.y - s2.y) * (p.x - s2.x) + (s2.x - s1.x) * (p.y - s2.y)) / den;
        let w1 = ((s2.y - s0.y) * (p.x - s2.x) + (s0.x - s2.x) * (p.y - s2.y)) / den;
        Some(([a, b, c], [w0, w1, 1.0 - w0 - w1]))
    })
}

/// Mesh Edit's art, captured on entry and drawn unchanged until it ends.
struct MeshArt {
    mesh: usize,
    /// Canvas-space positions, so it follows zoom and pan.
    positions: Vec<f32>,
    uvs: Vec<f32>,
    indices: Vec<u16>,
}

struct MeshDrag {
    vertex: usize,
    /// Blended parent-space positions captured when the drag began.
    local: Vec<f32>,
    dx: f32,
    dy: f32,
    /// The mesh on screen when the drag began, its triangles, and its
    /// texture coordinates as the model stores them: the art under the drop
    /// point is read off this, so the vertex slides over the picture.
    start: Vec<Pos2>,
    indices: Vec<u16>,
    model_uvs: Vec<f32>,
}

/// Where `p` falls on a mesh as (triangle corners, weights): inside the
/// triangle holding it, else extended from the nearest one, so a point
/// dragged past the edge keeps reading the art beyond it.
fn mesh_weights(pts: &[Pos2], indices: &[u16], p: Pos2) -> Option<([usize; 3], [f32; 3])> {
    let (tris, _) = indices.as_chunks::<3>();
    let mut best: Option<(f32, [usize; 3], [f32; 3])> = None;
    for &[a, b, c] in tris {
        let (a, b, c) = (a as usize, b as usize, c as usize);
        let (Some(&pa), Some(&pb), Some(&pc)) = (pts.get(a), pts.get(b), pts.get(c)) else {
            continue;
        };
        let d = (pb.x - pa.x) * (pc.y - pa.y) - (pc.x - pa.x) * (pb.y - pa.y);
        if d.abs() < 1e-9 {
            continue;
        }
        let wb = ((p.x - pa.x) * (pc.y - pa.y) - (pc.x - pa.x) * (p.y - pa.y)) / d;
        let wc = ((pb.x - pa.x) * (p.y - pa.y) - (p.x - pa.x) * (pb.y - pa.y)) / d;
        let wa = 1.0 - wb - wc;
        // 0 inside; outside, how far past the edges the weights reach
        let out = (-wa).max(0.0) + (-wb).max(0.0) + (-wc).max(0.0);
        if best.as_ref().is_none_or(|(o, _, _)| out < *o) {
            best = Some((out, [a, b, c], [wa, wb, wc]));
        }
        if out == 0.0 {
            break;
        }
    }
    best.map(|(_, t, w)| (t, w))
}

/// Blend a vertex pool's (u, v) pairs with triangle weights.
fn mix_uv(uvs: &[f32], t: [usize; 3], w: [f32; 3]) -> Option<(f32, f32)> {
    let mut out = (0.0, 0.0);
    for (i, wi) in t.iter().zip(w) {
        out.0 += uvs.get(i * 2)? * wi;
        out.1 += uvs.get(i * 2 + 1)? * wi;
    }
    Some(out)
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
    /// The origin drag so far in canvas units, for the pivot move.
    wx: f32,
    wy: f32,
    degrees: f32,
    last_ptr: Pos2,
}

/// What a live drag is rewriting.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum LiveTarget {
    Mesh(usize),
    Warp(usize),
}

type BrushPlan = Vec<(LiveTarget, usize, Vec<(usize, f32, f32)>)>;

struct DeformStroke {
    parts: Vec<(LiveDrag, Vec<Pos2>, Vec<Vec2>)>,
    last: Pos2,
}

/// The brush's pull at `t` (distance over radius): full inside the hard
/// core, then a smooth fade to nothing at the edge.
fn brush_falloff(t: f32, hardness: f32) -> f32 {
    if t >= 1.0 {
        0.0
    } else if t <= hardness {
        1.0
    } else {
        let u = (1.0 - t) / (1.0 - hardness).max(1e-4);
        u * u * (3.0 - 2.0 * u)
    }
}

/// A point dropped near a guide lands on it (canvas pixels).
#[derive(Clone, Copy)]
enum GuideCmd {
    /// At a screen point: vertical, horizontal, or both (None).
    At(Option<bool>, Pos2),
    /// The same from a key, only when the mouse is over the canvas.
    Mouse(Option<bool>, Pos2),
    Center,
    AtVertex,
}

fn snap_to_guides(guides: &[ayatsuri_format::GuideLine], q: [f32; 2], tolerance: f32) -> [f32; 2] {
    let mut out = q;
    for g in guides {
        if g.vertical && (q[0] - g.at).abs() < tolerance {
            out[0] = g.at;
        }
        if !g.vertical && (q[1] - g.at).abs() < tolerance {
            out[1] = g.at;
        }
    }
    out
}

/// A point drag shown live on the model: the keyform being edited, its
/// numbers as they were, and the object's local positions at the grab.
/// Each frame the keyform is rewritten from these, so the art (and
/// everything a warp holds) follows the pointer without an undo step per
/// frame; on release they go back and one real edit lands.
struct LiveDrag {
    target: LiveTarget,
    keyform: usize,
    base: Vec<f32>,
    local: Vec<f32>,
    parent: Option<u32>,
}

impl LiveDrag {
    fn start(puppet: &mut ayatsuri_core::Puppet, target: LiveTarget) -> Option<Self> {
        let values: Vec<f32> =
            (0..puppet.model().parameters.len()).map(|i| puppet.value(i)).collect();
        let (edit_target, local, parent) = match target {
            LiveTarget::Mesh(m) => (
                ayatsuri_core::edit::Target::ArtMesh(m),
                puppet.mesh_local_positions(m)?,
                puppet.model().art_meshes.get(m)?.parent_deformer,
            ),
            LiveTarget::Warp(d) => (
                ayatsuri_core::edit::Target::Deformer(d),
                puppet.warp_local_grid(d)?,
                puppet.model().deformers.get(d)?.parent_deformer,
            ),
        };
        let keyform = ayatsuri_core::keyform::keyform_at(puppet.model(), edit_target, &values)?;
        let mut me = Self { target, keyform, base: Vec::new(), local, parent };
        me.base = me.form(puppet)?.to_vec();
        Some(me)
    }

    fn form<'a>(&self, puppet: &'a mut ayatsuri_core::Puppet) -> Option<&'a mut [f32]> {
        match self.target {
            LiveTarget::Mesh(m) => puppet.mesh_keyform_mut(m, self.keyform),
            LiveTarget::Warp(d) => puppet.warp_keyform_mut(d, self.keyform),
        }
    }

    /// Write the keyform as the drag has it: `shifts` are screen moves.
    fn show(&self, puppet: &mut ayatsuri_core::Puppet, shifts: &[(usize, Vec2)], k_unit: f32) {
        let moves: Vec<(usize, f32, f32)> = shifts
            .iter()
            .map(|&(i, d)| {
                let px = self.local.get(i * 2).copied().unwrap_or(0.0);
                let py = self.local.get(i * 2 + 1).copied().unwrap_or(0.0);
                let (dx, dy) =
                    parent_delta(puppet, self.parent, px, py, d.x * k_unit, d.y * k_unit);
                (i, dx, dy)
            })
            .collect();
        if let Some(form) = self.form(puppet) {
            form.copy_from_slice(&self.base);
            for (i, dx, dy) in moves {
                if let Some(x) = form.get_mut(i * 2) {
                    *x += dx;
                }
                if let Some(y) = form.get_mut(i * 2 + 1) {
                    *y += dy;
                }
            }
        }
    }

    /// Write the keyform from new local positions (a Bezier drag's grid).
    fn show_local(&self, puppet: &mut ayatsuri_core::Puppet, new_local: &[f32]) {
        let (base, local) = (self.base.clone(), self.local.clone());
        if let Some(form) = self.form(puppet)
            && form.len() == new_local.len()
        {
            for (i, f) in form.iter_mut().enumerate() {
                let (b, o, n) = (
                    base.get(i).copied().unwrap_or(0.0),
                    local.get(i).copied().unwrap_or(0.0),
                    new_local.get(i).copied().unwrap_or(0.0),
                );
                *f = b + (n - o);
            }
        }
    }

    fn restore(&self, puppet: &mut ayatsuri_core::Puppet) {
        let base = self.base.clone();
        if let Some(form) = self.form(puppet)
            && form.len() == base.len()
        {
            form.copy_from_slice(&base);
        }
    }
}

/// Keep `live` on `target` (starting it, or switching from another
/// object), or put back whatever it was showing when nothing is dragged.
fn live_for(
    live: &mut Option<LiveDrag>,
    puppet: &mut ayatsuri_core::Puppet,
    target: Option<LiveTarget>,
) {
    let same = live.as_ref().map(|l| l.target) == target;
    if !same {
        if let Some(old) = live.take() {
            old.restore(puppet);
        }
        *live = target.and_then(|t| LiveDrag::start(puppet, t));
    }
}

/// A deform path control point, pinned to a triangle of its mesh so the
/// path rides along as the mesh deforms between poses.
#[derive(Clone, Copy)]
struct PathPin {
    tri: usize,
    w: [f32; 3],
}

/// Results coming back from background threads.
/// A Deformer palette menu choice, run once the palette is drawn.
/// What a Parts palette mesh row's menu asked for.
enum MeshRowCmd {
    /// A click on the row; the palette applies Ctrl / Shift picking.
    Click(usize),
    Delete(usize),
    ClipBy(usize),
}

enum DefCmd {
    /// Create a warp (true) or rotation deformer around this object.
    Create(bool, Sel),
    CreateForSelection(bool),
    Rename(usize),
    Delete(usize),
    /// Delete this art mesh (with the rest of the pick when it is in it).
    DeleteMesh(usize),
    /// Pick this art mesh's clipping masks on the canvas.
    ClipBy(usize),
    /// A mesh's deform path at an edit level: open it with the path tool,
    /// or remove it.
    EditPath(usize, u8),
    DeletePath(usize, u8),
    /// Move this object under that deformer (None: under no deformer).
    Reparent(Child, Option<usize>),
}

/// What a native file dialog running on its own thread was opened for;
/// the frame loop finishes the job when the path comes back.
pub(crate) enum PickFor {
    /// Save As; `then_close` carries an Unsaved Changes close (0 the
    /// model, 1 the window) to finish once the save lands.
    SaveAs {
        then_close: Option<u8>,
    },
    /// Width in pixels, JPEG rather than PNG, transparent background.
    ExportImage(u32, bool, bool),
    RenderFolder,
    ExportMoc3(Box<ExportDlg>),
    WorkspaceExport,
    WorkspaceImport,
    ImportMotion,
    /// A .psd or .kra added to the open model.
    ImportPaint,
    /// An updated .psd or .kra whose art replaces the matching meshes' art.
    ReimportPaint,
}

enum BgJob {
    Picked(Option<PathBuf>),
    Loaded(PathBuf, Box<Result<Session, String>>),
}

/// A finished path bend: the control point, where it was dropped, each
/// vertex's screen offset, and the parent-space positions at the grab.
type PathFinish = (usize, Pos2, Vec<(usize, Vec2)>, Vec<f32>);

#[derive(Clone, Copy)]
struct ArtGrab {
    anchor: usize,
    part: u8,
    start: [f32; 2],
    orig: [f32; 6],
}

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

/// The New Animation (or New Composition) dialog's working copy.
struct AnimNew {
    name: String,
    fps: f32,
    duration: f32,
    looped: bool,
    /// A composition of layers rather than a scene of curves.
    comp: bool,
}

/// The Export as moc3 dialog's working copy.
#[derive(Clone)]
pub(crate) struct ExportDlg {
    pub(crate) name: String,
    pub(crate) version: usize,
    pub(crate) physics: bool,
    pub(crate) motions: bool,
    pub(crate) expressions: bool,
    /// 0 moc3, 1 cmo3, 2 image.
    pub(crate) tab: usize,
    pub(crate) image_width: u32,
    pub(crate) jpeg: bool,
    pub(crate) transparent: bool,
}

/// The New Model dialog's working copy.
struct ModelNew {
    name: String,
    author: String,
    width: f32,
    height: f32,
    ppu: f32,
    std_params: bool,
    /// Record a timelapse beside the project, from the Settings default.
    timelapse: bool,
}

impl Default for ModelNew {
    fn default() -> Self {
        Self {
            name: "untitled".into(),
            author: String::new(),
            width: 2048.0,
            height: 2048.0,
            ppu: 1024.0,
            timelapse: true,
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
        let check_updates = !prefs.update_check_off;
        let ask_ffmpeg = !prefs.ffmpeg_prompt_off;
        crate::i18n::set_language(prefs.language.as_deref().unwrap_or("system"));
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
            // tests never reach the network
            update: crate::update::UpdateState::new(check_updates && !cfg!(test)),
            ffmpeg_prompt: crate::ffmpeg::Prompt::new(ask_ffmpeg && !cfg!(test)),
            export_overwrite: None,
            show_settings: false,
            show_api_window: false,
            key_filter: String::new(),
            picking: None,
            dialog_pick: None,
            loading: None,

            rpc: None,
            rpc_id: String::new(),

            dev_physics_done: false,
            startup_done: false,

            show_stats: false,
            timelapse_view: None,
            render_win: None,
            atlas_win: None,
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
            timeline: crate::timeline::Timeline::default(),
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
            folder_from: Vec::new(),
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
            frame_undo_depth: 0,
            def_rename: None,
            toolbar_reset: false,
            live_drag: None,
            dev_mesh_done: false,
            dev_def_done: false,
            dev_def_logged: false,
            mesh_art: None,
            dev_mesh_logged: false,
            rot_live: None,
            api: Default::default(),
            recovery: Default::default(),
            recovering: None,
            tool_opts: ToolOptions::default(),
            lasso: Vec::new(),
            sel_param: None,
            param_edit: None,
            pmenu: param_menu::ParamMenuState::default(),
            deformer_dialog: None,
            multi_sel: Vec::new(),
            part_pick: Vec::new(),
            part_anchor: None,
            palette_hover: None,
            part_search: String::new(),
            def_search: String::new(),
            mesh_anchor: None,
            clip_pick: None,
            pending_create: None,
            pending_guide: None,
            ctx_click: None,
            show_guide_settings: false,
            focus_request: false,
            focus_search: false,
            form_clip: None,
            mesh_edit: None,
            mesh_drag: None,
            mesh_mode: 0,
            mesh_picked: std::collections::BTreeSet::new(),
            mesh_picked_on: None,
            mesh_lasso: Vec::new(),
            point_clip: None,
            point_clip_cmd: None,
            mesh_clip: None,
            mesh_clip_cmd: None,
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
            art_draft: None,
            art_grab: None,
            art_edit: None,
            art_style: ayatsuri_format::model::ArtPath::default(),
            art_finish: false,
            art_pick: None,
            clip_with: None,
            clip_search: String::new(),
            clip_invert: false,
            dbrush: None,
            dbrush_hardness: 0.3,
            dbrush_strength: 0.7,
            show_guides: true,
            mirror_edit: false,
            guide_drag: None,
            art_pending: None,
            pick_pages: std::collections::HashMap::new(),
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
    /// The app is closing normally: anything unsaved was saved or knowingly
    /// discarded at the Unsaved Changes prompt, so the safety copies go.
    pub fn clean_exit(&mut self) {
        if let Some(s) = &self.session {
            s.discard_autosave();
        }
        for p in crate::take_crash_copies() {
            let _ = std::fs::remove_file(p);
        }
        crate::update::finish(&self.update);
    }

    /// Last line of defence: the app is going down past every guard (eframe
    /// itself panicked). Keep any unsaved work in a recovery file the
    /// relaunch can open.
    pub fn emergency_save(&mut self) {
        if let Some(s) = &self.session
            && s.modified
            && let Some(p) = s.write_recovery()
        {
            log::error!("unsaved work kept in {}", p.display());
            crate::remember_recovery(p);
        }
    }

    pub fn on_crash(&mut self) {
        let info = logging::take_crash();
        // stop whatever was mid-flight; it is the likeliest culprit
        self.mesh_drag = None;
        self.pts.clear();
        self.rot_drag = None;
        self.anim_playing = false;
        // undo what the crashed frame did to the model before saving a copy,
        // so the recovery holds the last good state; the rollback itself is
        // guarded, a second failure is logged and the model left as it is
        let depth = self.frame_undo_depth;
        if let Some(s) = &mut self.session {
            let rolled = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                s.crash_rollback(depth);
            }));
            if rolled.is_err() {
                log::error!("the rollback after the crash failed too; keeping the model as is");
            }
        }
        let recovery = self.session.as_ref().and_then(|s| s.write_recovery());
        if let Some(p) = &recovery {
            crate::remember_recovery(p.clone());
            crate::note_crash_copy(p.clone());
        }
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
                .unwrap_or_else(|| crate::i18n::tr("unknown error").into()),
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
        crate::windows::Dialog::new(crate::i18n::tr("Something went wrong")).show(ui.ctx(), |ui| {
                ui.set_width(440.0);
                ui.label(
                    egui::RichText::new(format!("{} {}", egui_phosphor::regular::WARNING, c.message))
                        .color(theme::pal().accent)
                        .strong(),
                );
                if !c.location.is_empty() {
                    ui.weak(crate::i18n::trf("at {location}", &[("location", &c.location)]));
                }
                ui.add_space(6.0);
                ui.label(crate::i18n::tr("The editor caught the error and kept running. The step that failed was undone where possible."));
                match &c.recovery {
                    Some(p) => {
                        ui.label(crate::i18n::trf(
                            "Your model was saved to {path}",
                            &[("path", &p.display())],
                        ));
                    }
                    None => {
                        ui.weak(crate::i18n::tr("No model was open, nothing needed rescuing."));
                    }
                }
                if let Some(p) = logging::log_path() {
                    ui.weak(crate::i18n::trf(
                        "Full details, with a backtrace, are in {path}",
                        &[("path", &p.display())],
                    ));
                }
                if c.count > 1 {
                    ui.weak(crate::i18n::trf(
                        "This has happened {count} times this session.",
                        &[("count", &c.count)],
                    ));
                }
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button(crate::i18n::tr("Copy details")).clicked() {
                        copy = true;
                    }
                    if ui.button(crate::i18n::tr("Open log folder")).clicked() {
                        open_dir = true;
                    }
                    if ui.button(egui::RichText::new(crate::i18n::tr("Keep working")).strong()).clicked() {
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
    pub(crate) fn import(&mut self, ctx: &egui::Context, path: PathBuf) {
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
                .add_filter(
                    crate::i18n::tr("All supported"),
                    &["aom", "moc3", "cmo3", "json", "psd", "kra", "clip"],
                )
                .add_filter(
                    crate::i18n::tr("Ayatsuri / Live2D model"),
                    &["aom", "moc3", "cmo3", "json"],
                )
                .add_filter(
                    crate::i18n::tr("Artwork (Photoshop, Krita, Clip Studio)"),
                    &["psd", "kra", "clip"],
                )
                .set_title(crate::i18n::tr("Open model or artwork"))
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
        let answered = match &self.dialog_pick {
            Some((_, rx)) => match rx.try_recv() {
                Ok(path) => Some(path),
                Err(std::sync::mpsc::TryRecvError::Disconnected) => Some(None),
                Err(std::sync::mpsc::TryRecvError::Empty) => {
                    // keep drawing while the dialog is up, so the answer is
                    // picked up promptly and the window never looks hung
                    ctx.request_repaint_after(std::time::Duration::from_millis(150));
                    None
                }
            },
            None => None,
        };
        if let Some(path) = answered
            && let Some((why, _)) = self.dialog_pick.take()
        {
            self.finish_pick(ctx, why, path);
        }
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
            Some(Some((path, Ok(mut s)))) => {
                self.loading = None;
                if self.recovering.as_ref() == Some(&path) {
                    // recovered work is unsaved work: no file of its own, not
                    // a recent; its autosave takes over from the copy
                    self.recovering = None;
                    s.aom_path = None;
                    s.modified = true;
                    let _ = std::fs::remove_file(&path);
                    log::info!("recovered {}", s.name);
                } else {
                    self.prefs.remember(path);
                }
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
                            ui.label(crate::i18n::trf("Opening {name}…", &[("name", &name)]));
                        });
                    },
                );
            });
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(100));
    }

    fn save_session(&mut self, save_as: bool) {
        self.save_session_then(save_as, None);
    }

    /// Save to the known path, or ask for one on the picker thread first.
    /// `then_close` finishes an Unsaved Changes close after a picked save.
    fn save_session_then(&mut self, save_as: bool, then_close: Option<u8>) {
        let Some(s) = &self.session else { return };
        match s.aom_path.clone().filter(|_| !save_as) {
            Some(path) => {
                self.save_session_to(path);
            }
            None => {
                let file = format!("{}.aom", s.name);
                self.pick_async(PickFor::SaveAs { then_close }, move || {
                    rfd::FileDialog::new()
                        .add_filter(crate::i18n::tr("Ayatsuri Open Model"), &["aom"])
                        .set_file_name(file)
                        .set_title(crate::i18n::tr("Save model"))
                        .save_file()
                });
            }
        }
    }

    /// True when the model was written.
    fn save_session_to(&mut self, path: PathBuf) -> bool {
        let Some(s) = &mut self.session else { return false };
        match s.save_aom(&path) {
            Ok(()) => {
                // saved, so the safety copy is stale
                s.discard_autosave();
                self.prefs.remember(path);
                true
            }
            Err(e) => {
                log::error!("save failed: {e}");
                false
            }
        }
    }

    /// Run a native file dialog on its own thread. The system picker blocks
    /// until it closes, and on the UI thread the desktop calls the window
    /// "not responding"; this way the editor keeps drawing underneath.
    pub(crate) fn pick_async(
        &mut self,
        why: PickFor,
        dialog: impl FnOnce() -> Option<PathBuf> + Send + 'static,
    ) {
        if self.dialog_pick.is_some() || self.picking.is_some() {
            log::info!("a file dialog is already open");
            if let PickFor::ExportMoc3(dlg) = why {
                self.export_dlg = Some(*dlg);
            }
            return;
        }
        let (tx, rx) = std::sync::mpsc::channel();
        let spawned = std::thread::Builder::new().name("file-dialog".into()).spawn(move || {
            let _ = tx.send(dialog());
        });
        match spawned {
            Ok(_) => self.dialog_pick = Some((why, rx)),
            Err(e) => log::error!("could not open the file dialog: {e}"),
        }
    }

    /// A background file dialog answered: finish what it was opened for.
    fn finish_pick(&mut self, ctx: &egui::Context, why: PickFor, path: Option<PathBuf>) {
        match (why, path) {
            (PickFor::SaveAs { then_close }, Some(path)) => {
                if self.save_session_to(path)
                    && let Some(action) = then_close
                {
                    self.finish_close(ctx, action);
                }
            }
            // cancelled out of an Unsaved Changes save: ask again
            (PickFor::SaveAs { then_close: Some(action) }, None) => {
                self.confirm_close = Some(action);
            }
            (PickFor::ExportImage(w, j, t), Some(path)) => self.export_image_to(path, w, j, t),
            (PickFor::RenderFolder, Some(dir)) => {
                if let Some(w) = &mut self.render_win {
                    w.set_folder(dir.clone());
                }
                self.prefs.render.folder = Some(dir);
                self.prefs.save();
            }
            (PickFor::ExportMoc3(dlg), Some(dir)) => self.export_moc3_to(&dlg, &dir),
            // a cancelled folder pick leaves the export dialog as it was
            (PickFor::ExportMoc3(dlg), None) => self.export_dlg = Some(*dlg),
            (PickFor::WorkspaceExport, Some(path)) => {
                match serde_json::to_vec_pretty(&self.layout) {
                    Ok(bytes) => match std::fs::write(&path, bytes) {
                        Ok(()) => log::info!("workspace exported to {}", path.display()),
                        Err(e) => log::error!("workspace export: {e}"),
                    },
                    Err(e) => log::error!("workspace export: {e}"),
                }
            }
            (PickFor::ImportMotion, Some(path)) => self.import_motion(&path),
            (PickFor::ReimportPaint, Some(path)) => {
                if self.mesh_edit.is_some() {
                    log::warn!("leave Mesh Edit before re-importing artwork");
                } else if let Some(s) = &mut self.session {
                    match s.reimport_paint(ctx, &path) {
                        Ok(r) => {
                            log::info!("re-imported {}: new art on {} mesh(es)", r.file, r.updated);
                            if !r.new_layers.is_empty() {
                                log::warn!(
                                    "{} layer(s) match no mesh (use Import PSD to add them): {}",
                                    r.new_layers.len(),
                                    r.new_layers.join(", ")
                                );
                            }
                            if !r.missing.is_empty() {
                                log::warn!(
                                    "{} mesh(es) found no layer and kept their old art: {}",
                                    r.missing.len(),
                                    r.missing.join(", ")
                                );
                            }
                            self.mesh_art = None;
                        }
                        Err(e) => log::error!("re-import artwork: {e}"),
                    }
                }
            }
            (PickFor::ImportPaint, Some(path)) => {
                if self.mesh_edit.is_some() {
                    log::warn!("leave Mesh Edit before importing artwork");
                } else if let Some(s) = &mut self.session {
                    match s.import_paint(ctx, &path) {
                        Ok(file) => {
                            log::info!("imported artwork from {file}");
                            self.mesh_drag = None;
                            self.mesh_art = None;
                        }
                        Err(e) => log::error!("import artwork: {e}"),
                    }
                }
            }
            (PickFor::WorkspaceImport, Some(path)) => {
                match std::fs::read(&path)
                    .map_err(|e| e.to_string())
                    .and_then(|b| serde_json::from_slice::<DockNode>(&b).map_err(|e| e.to_string()))
                {
                    Ok(layout) => {
                        self.layout = layout;
                        self.save_layout();
                        log::info!("workspace imported from {}", path.display());
                    }
                    Err(e) => log::error!("workspace import: {e}"),
                }
            }
            _ => {}
        }
    }

    /// Add a motion3.json to the open model as a new scene.
    fn import_motion(&mut self, path: &Path) {
        let Some(s) = &mut self.session else { return };
        let name = crate::session::motion_name(path);
        match std::fs::read(path)
            .map_err(|e| e.to_string())
            .and_then(|b| ayatsuri_import::from_motion3(&b, &name).map_err(|e| e.to_string()))
        {
            Ok(a) => {
                log::info!("imported motion {} ({} curves)", a.name, a.curves.len());
                s.animations.push(a);
                s.current_anim = Some(s.animations.len().saturating_sub(1));
                s.current_comp = None;
                s.modified = true;
                self.anim_time = 0.0;
                self.anim_applied = f32::NAN;
            }
            Err(e) => log::error!("import motion {}: {e}", path.display()),
        }
    }

    /// Close the model (0) or the window (1) after Unsaved Changes.
    fn finish_close(&mut self, ctx: &egui::Context, action: u8) {
        self.confirm_close = None;
        if action == 1 {
            self.force_close = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        } else {
            self.close_session();
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
        // where undo stood when this frame began, for the crash rollback
        self.frame_undo_depth = self.session.as_ref().map_or(0, |s| s.undo_depth());
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
        // apps driving parameters through the API go first, so what they
        // hold is kept off physics below
        self.api_frame();
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
            // developer hook for screenshots: AYATSURI_SHOW_DIALOG=new
            match std::env::var("AYATSURI_SHOW_DIALOG").as_deref() {
                Ok("new") => self.new_model = Some(ModelNew::default()),
                Ok("api") => self.show_api_window = true,
                _ => {}
            }
            // developer hook for screenshots: AYATSURI_SHOW_ABOUT=credits
            if let Some(tab) = std::env::var_os("AYATSURI_SHOW_ABOUT") {
                self.show_about = true;
                self.about.open_tab(&tab.to_string_lossy());
            }
        }
        // developer hook for testing deformer drags: AYATSURI_DEV_DEFORMER=<i>
        // selects that deformer once a model is loaded
        if let Some(s) = &self.session
            && !self.dev_def_done
            && let Some(di) =
                std::env::var("AYATSURI_DEV_DEFORMER").ok().and_then(|v| v.parse::<usize>().ok())
            && di < s.puppet.model().deformers.len()
        {
            self.dev_def_done = true;
            self.selected = Some(Sel::Deformer(di));
        }
        // developer hook for testing Mesh Edit: AYATSURI_DEV_MESHEDIT=<mesh>
        // opens that mesh in Mesh Edit once a model is loaded
        if let Some(s) = &self.session
            && !self.dev_mesh_done
            && let Some(mi) =
                std::env::var("AYATSURI_DEV_MESHEDIT").ok().and_then(|v| v.parse::<usize>().ok())
            && mi < s.puppet.model().art_meshes.len()
        {
            self.dev_mesh_done = true;
            self.selected = Some(Sel::Mesh(mi));
            self.mesh_edit = Some(mi);
            self.mesh_snapshot = Some(s.puppet.model().clone());
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
        self.recovery_window(ui);
        self.api_prompt_window(&ui.ctx().clone());
        self.api_window(ui);
        self.def_rename_window(&ui.ctx().clone());
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
            if let Some(c) = self.prefs.accent_color() {
                theme::set_accent(&ctx, Some(c));
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
        self.update.poll();
        if self.update.checking() {
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(250));
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
        } else if self.selected_glue.is_none()
            && self.mesh_edit.is_none()
            && self.tool == Tool::Arrow
            && self.pts.picked.is_empty()
            && !ui.ctx().egui_wants_keyboard_input()
            && let Some(Sel::Mesh(mi)) = self.selected
            && ui.ctx().input(|i| i.key_pressed(egui::Key::Delete) && !i.modifiers.command)
        {
            let meshes = self.mesh_pick_with(mi);
            self.delete_meshes(meshes);
        }
        // Cut, Copy and Paste for whole art meshes; Mesh Edit has its own,
        // for points. Ctrl+Shift+C stays Copy Form.
        if self.mesh_edit.is_none() && !ui.ctx().egui_wants_keyboard_input() {
            let keys = ui.ctx().input(|i| {
                let plain = i.modifiers.command && !i.modifiers.shift && !i.modifiers.alt;
                let ev = |f: fn(&egui::Event) -> bool| plain && i.events.iter().any(f);
                if ev(|e| matches!(e, egui::Event::Cut)) || (plain && i.key_pressed(egui::Key::X)) {
                    Some(ClipCmd::Cut)
                } else if ev(|e| matches!(e, egui::Event::Copy))
                    || (plain && i.key_pressed(egui::Key::C))
                {
                    Some(ClipCmd::Copy)
                } else {
                    None
                }
            });
            // egui turns Ctrl+V into the clipboard's text, so a paste is
            // ours when the text is (from this project or another)
            let pasted = ui.ctx().input(|i| {
                i.events.iter().find_map(|e| match e {
                    egui::Event::Paste(t) => crate::mesh_clip::ClipFile::from_text(t),
                    _ => None,
                })
            });
            let ctx = ui.ctx().clone();
            match self.mesh_clip_cmd.take().or(keys) {
                Some(ClipCmd::Cut) => self.copy_meshes(&ctx, true),
                Some(ClipCmd::Copy) => self.copy_meshes(&ctx, false),
                Some(ClipCmd::Paste) => {
                    if let Some(clip) = self.mesh_clip.clone() {
                        self.paste_meshes(&ctx, &clip);
                    }
                }
                None => {
                    if let Some(clip) = pasted {
                        self.paste_meshes(&ctx, &clip);
                    }
                }
            }
        }

        let dropped: Vec<PathBuf> = ui
            .ctx()
            .input(|i| i.raw.dropped_files.iter().map(|f| f.path().to_path_buf()).collect());
        for path in dropped {
            self.import(&ui.ctx().clone(), path);
        }

        Panel::top("menu").show(ui, |ui| {
            egui::MenuBar::new().ui(ui, |ui| {
                ui.menu_button(crate::i18n::tr("File"), |ui| {
                    if ui
                        .add(
                            egui::Button::new(crate::i18n::tr("New…"))
                                .shortcut_text(self.sc("new-model")),
                        )
                        .clicked()
                    {
                        ui.close();
                        self.new_model = Some(ModelNew::default());
                    }
                    if ui
                        .add_enabled(
                            self.session.is_some(),
                            egui::Button::new(crate::i18n::tr("New Animation…")),
                        )
                        .clicked()
                    {
                        ui.close();
                        self.new_anim = Some(AnimNew {
                            name: "idle".into(),
                            fps: 30.0,
                            duration: 3.0,
                            looped: true, comp: false,
                        });
                    }
                    if ui
                        .add_enabled(
                            self.session.is_some(),
                            egui::Button::new(crate::i18n::tr("New Composition…")),
                        )
                        .on_hover_text(crate::i18n::tr(
                            "Scenes and other compositions laid out as layers over one clock.",
                        ))
                        .clicked()
                    {
                        ui.close();
                        self.new_anim = Some(AnimNew {
                            name: "comp".into(),
                            fps: 30.0,
                            duration: 5.0,
                            looped: false,
                            comp: true,
                        });
                    }
                    if ui
                        .add(
                            egui::Button::new(crate::i18n::tr("Open…"))
                                .shortcut_text(self.sc("open")),
                        )
                        .clicked()
                    {
                        ui.close();
                        self.import_dialog(&ui.ctx().clone());
                    }
                    ui.menu_button(crate::i18n::tr("Open Recent"), |ui| {
                        let recents = self.prefs.recents.clone();
                        if recents.is_empty() {
                            ui.weak(crate::i18n::tr("empty"));
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
                            egui::Button::new(crate::i18n::tr("Save"))
                                .shortcut_text(self.sc("save")),
                        )
                        .clicked()
                    {
                        ui.close();
                        self.save_session(false);
                    }
                    if ui
                        .add_enabled(
                            self.session.is_some(),
                            egui::Button::new(crate::i18n::tr("Save As…"))
                                .shortcut_text(self.sc("save-as")),
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
                            egui::Button::new(crate::i18n::tr("Export Model…"))
                                .shortcut_text(self.sc("export-moc3")),
                        )
                        .on_hover_text(crate::i18n::tr(
                            "moc3 for VTube Studio and other Cubism-compatible apps, a Cubism \
                             project, or a still image",
                        ))
                        .clicked()
                    {
                        ui.close();
                        self.open_export(0);
                    }
                    if ui
                        .add_enabled(
                            self.session.is_some(),
                            egui::Button::new(crate::i18n::tr("Export Animation…")),
                        )
                        .on_hover_text(crate::i18n::tr(
                            "Render a scene, with an expression on top if you like, to GIF, \
                             a PNG sequence, MP4, WebM or MOV",
                        ))
                        .clicked()
                    {
                        ui.close();
                        self.open_render(&ui.ctx().clone(), crate::windows::render::Source::Animation);
                    }
                    if ui
                        .add_enabled(
                            self.session.is_some(),
                            egui::Button::new(crate::i18n::tr("Export Timelapse…")),
                        )
                        .on_hover_text(crate::i18n::tr(
                            "Render the project's timelapse: every step of the rig, held on the \
                             finished model at the end",
                        ))
                        .clicked()
                    {
                        ui.close();
                        self.open_render(&ui.ctx().clone(), crate::windows::render::Source::Timelapse);
                    }
                    if ui
                        .add_enabled(
                            self.session.is_some(),
                            egui::Button::new(crate::i18n::tr("Import Motion…")),
                        )
                        .on_hover_text(crate::i18n::tr("Add a .motion3.json as a scene"))
                        .clicked()
                    {
                        ui.close();
                        self.pick_async(PickFor::ImportMotion, || {
                            rfd::FileDialog::new()
                                .add_filter(crate::i18n::tr("Live2D motion"), &["json"])
                                .set_title(crate::i18n::tr("Import motion3.json"))
                                .pick_files()
                                .map(|v| v.into_iter().next())
                                .unwrap_or_default()
                        });
                    }
                    if ui
                        .add_enabled(
                            self.session.is_some(),
                            egui::Button::new(crate::i18n::tr("Import Artwork…")),
                        )
                        .on_hover_text(crate::i18n::tr(
                            "Add a Photoshop, Krita or Clip Studio file's layers to this model \
                             as new parts and meshes",
                        ))
                        .clicked()
                    {
                        ui.close();
                        self.pick_async(PickFor::ImportPaint, || {
                            rfd::FileDialog::new()
                                .add_filter(crate::i18n::tr("Artwork"), &["psd", "kra", "clip"])
                                .set_title(crate::i18n::tr("Import artwork into this model"))
                                .pick_file()
                        });
                    }
                    if ui
                        .add_enabled(
                            self.session.is_some(),
                            egui::Button::new(crate::i18n::tr("Re-import Artwork…")),
                        )
                        .on_hover_text(crate::i18n::tr(
                            "Swap in updated art from the file this model was made from: layers \
                             match meshes by folder and name, and the rigging stays as it is",
                        ))
                        .clicked()
                    {
                        ui.close();
                        self.pick_async(PickFor::ReimportPaint, || {
                            rfd::FileDialog::new()
                                .add_filter(crate::i18n::tr("Artwork"), &["psd", "kra", "clip"])
                                .set_title(crate::i18n::tr("Re-import artwork"))
                                .pick_file()
                        });
                    }
                    ui.separator();
                    if ui
                        .add_enabled(
                            self.session.is_some(),
                            egui::Button::new(crate::i18n::tr("Project…")),
                        )
                        .clicked()
                    {
                        self.show_model_info = true;
                        ui.close();
                    }
                    if ui
                        .add_enabled(
                            self.session.is_some(),
                            egui::Button::new(crate::i18n::tr("Model Statistics…")),
                        )
                        .clicked()
                    {
                        self.show_stats = true;
                        ui.close();
                    }
                    ui.separator();
                    if ui.button(crate::i18n::tr("Environment Settings…")).clicked() {
                        self.show_settings = true;
                        ui.close();
                    }
                    if ui.button(crate::i18n::tr("Keyboard Shortcuts…")).clicked() {
                        self.show_settings = true;
                        self.settings_tab = 3;
                        ui.close();
                    }
                    ui.separator();
                    if ui
                        .add_enabled(
                            self.session.is_some(),
                            egui::Button::new(crate::i18n::tr("Close"))
                                .shortcut_text(self.sc("close")),
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
                    if ui.button(crate::i18n::tr("Exit")).clicked() {
                        ui.close();
                        ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                });
                ui.menu_button(crate::i18n::tr("Edit"), |ui| {
                    let (can_undo, can_redo) = self
                        .session
                        .as_ref()
                        .map(|s| (s.can_undo(), s.can_redo()))
                        .unwrap_or((false, false));
                    if ui
                        .add_enabled(can_undo, egui::Button::new(crate::i18n::tr("Undo")))
                        .clicked()
                    {
                        ui.close();
                        if let Some(s) = &mut self.session {
                            s.undo();
                        }
                    }
                    if ui
                        .add_enabled(can_redo, egui::Button::new(crate::i18n::tr("Redo")))
                        .clicked()
                    {
                        ui.close();
                        if let Some(s) = &mut self.session {
                            s.redo();
                        }
                    }
                    ui.separator();
                    // in Mesh Edit these copy and paste the picked points,
                    // elsewhere the selected art meshes
                    let editing = self.mesh_edit.is_some();
                    let meshes = !self.selected_meshes().is_empty();
                    if ui
                        .add_enabled(
                            !editing && meshes,
                            egui::Button::new(crate::i18n::tr("Cut")).shortcut_text("Ctrl+X"),
                        )
                        .clicked()
                    {
                        self.mesh_clip_cmd = Some(ClipCmd::Cut);
                        ui.close();
                    }
                    let can_copy = if editing { !self.mesh_picked.is_empty() } else { meshes };
                    if ui
                        .add_enabled(
                            can_copy,
                            egui::Button::new(crate::i18n::tr("Copy")).shortcut_text("Ctrl+C"),
                        )
                        .clicked()
                    {
                        if editing {
                            self.point_clip_cmd = Some(true);
                        } else {
                            self.mesh_clip_cmd = Some(ClipCmd::Copy);
                        }
                        ui.close();
                    }
                    let can_paste =
                        if editing { self.point_clip.is_some() } else { self.mesh_clip.is_some() };
                    if ui
                        .add_enabled(
                            can_paste && self.session.is_some(),
                            egui::Button::new(crate::i18n::tr("Paste")).shortcut_text("Ctrl+V"),
                        )
                        .clicked()
                    {
                        if editing {
                            self.point_clip_cmd = Some(false);
                        } else {
                            self.mesh_clip_cmd = Some(ClipCmd::Paste);
                        }
                        ui.close();
                    }
                });
                ui.menu_button(crate::i18n::tr("View"), |ui| {
                    let has = self.session.is_some();
                    if ui.add_enabled(has, egui::Button::new(crate::i18n::tr("Zoom In"))).clicked()
                    {
                        self.view_zoom = (self.view_zoom * 1.25).clamp(0.02, 40.0);
                        ui.close();
                    }
                    if ui.add_enabled(has, egui::Button::new(crate::i18n::tr("Zoom Out"))).clicked()
                    {
                        self.view_zoom = (self.view_zoom / 1.25).clamp(0.02, 40.0);
                        ui.close();
                    }
                    if ui.add_enabled(has, egui::Button::new(crate::i18n::tr("Fit"))).clicked() {
                        self.view_zoom = 1.0;
                        self.view_pan = Vec2::ZERO;
                        ui.close();
                    }
                    ui.separator();
                    ui.checkbox(&mut self.mirror_edit, crate::i18n::tr("Mirror Edit"))
                        .on_hover_text(crate::i18n::tr(
                            "Moving a point also moves its mirror across the vertical axis: \
                             the vertical guide line nearest the centre, or the canvas centre",
                        ));
                    ui.menu_button(crate::i18n::tr("Guide Line"), |ui| {
                        ui.checkbox(&mut self.show_guides, crate::i18n::tr("Show Guide Lines"));
                        if ui
                            .add_enabled(has, egui::Button::new(crate::i18n::tr("Add guide lines in the center")))
                            .clicked()
                        {
                            self.pending_guide = Some(GuideCmd::Center);
                            ui.close();
                        }
                        if ui
                            .add_enabled(
                                has,
                                egui::Button::new(crate::i18n::tr(
                                    "Add a guide line to the coordinates of the selected vertex",
                                )),
                            )
                            .clicked()
                        {
                            self.pending_guide = Some(GuideCmd::AtVertex);
                            ui.close();
                        }
                        if ui
                            .add_enabled(has, egui::Button::new(crate::i18n::tr("Guide Line settings…")))
                            .clicked()
                        {
                            self.show_guide_settings = true;
                            ui.close();
                        }
                        ui.separator();
                        let mut snap = !self.prefs.guide_snap_off;
                        if ui
                            .checkbox(&mut snap, crate::i18n::tr("Snap moving vertices to guide lines"))
                            .changed()
                        {
                            self.prefs.guide_snap_off = !snap;
                            self.prefs.save();
                        }
                        let mut rulers = !self.prefs.rulers_off;
                        if ui
                            .checkbox(&mut rulers, crate::i18n::tr("Rulers"))
                            .on_hover_text(crate::i18n::tr(
                                "The rulers along the canvas edges; drag out of them for a guide line",
                            ))
                            .changed()
                        {
                            self.prefs.rulers_off = !rulers;
                            self.prefs.save();
                        }
                        let any = self.session.as_ref().is_some_and(|s| !s.guides.is_empty());
                        if ui.add_enabled(any, egui::Button::new(crate::i18n::tr("Clear guide lines"))).clicked() {
                            if let Some(s) = &mut self.session {
                                s.guides.clear();
                                s.modified = true;
                            }
                            ui.close();
                        }
                    });
                });
                ui.menu_button(crate::i18n::tr("Modeling"), |ui| {
                    if ui.button(crate::i18n::tr("Modeling Workspace")).clicked() {
                        self.set_workspace(Workspace::Modeling);
                        ui.close();
                    }
                    ui.separator();
                    let has_sel = !self.selection_meshes().is_empty();
                    ui.menu_button(crate::i18n::tr("Deformer"), |ui| {
                        if ui
                            .add_enabled(
                                has_sel,
                                egui::Button::new(crate::i18n::tr("Create Warp Deformer")),
                            )
                            .clicked()
                        {
                            self.pending_create = Some((true, None));
                            ui.close();
                        }
                        if ui
                            .add_enabled(
                                has_sel,
                                egui::Button::new(crate::i18n::tr("Create Rotation Deformer")),
                            )
                            .clicked()
                        {
                            self.pending_create = Some((false, None));
                            ui.close();
                        }
                    });
                    let can_keys = self.edit_target().is_some() && self.sel_param.is_some();
                    ui.menu_button(crate::i18n::tr("Parameter"), |ui| {
                        if ui
                            .add_enabled(
                                can_keys,
                                egui::Button::new(crate::i18n::tr("Add Keys at Min and Max")),
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
                                self.add_keys_to_selection(vec![meta.min, meta.max]);
                            }
                        }
                        if ui
                            .add_enabled(
                                can_keys,
                                egui::Button::new(crate::i18n::tr(
                                    "Add Keys at Min, Default and Max",
                                )),
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
                            .add_enabled(
                                can_keys,
                                egui::Button::new(crate::i18n::tr("Delete Nearest Key")),
                            )
                            .clicked()
                        {
                            ui.close();
                            self.delete_key_on_selection();
                        }
                        ui.separator();
                        if ui
                            .add_enabled(
                                self.session.is_some(),
                                egui::Button::new(crate::i18n::tr("Reset to Default"))
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
                                egui::Button::new(crate::i18n::tr(
                                    "Automatic Generation of Four Corner Forms…",
                                ))
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
                                egui::Button::new(crate::i18n::tr("Motion Mirroring…")),
                            )
                            .on_disabled_hover_text(crate::i18n::tr("Select a parameter first"))
                            .clicked()
                            && let Some(pi) = self.sel_param
                        {
                            ui.close();
                            self.run_param_cmd(ParamCmd::Mirror(pi));
                        }
                    });
                    let mesh_sel = matches!(self.selected, Some(Sel::Mesh(_)));
                    ui.menu_button(crate::i18n::tr("ArtMesh"), |ui| {
                        if ui
                            .add_enabled(
                                mesh_sel,
                                egui::Button::new(crate::i18n::tr("Edit Mesh"))
                                    .shortcut_text(self.sc("mesh-edit")),
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
                            .add_enabled(
                                mesh_sel,
                                egui::Button::new(crate::i18n::tr("Auto Mesh Generation")),
                            )
                            .clicked()
                        {
                            ui.close();
                            if let Some(Sel::Mesh(mi)) = self.selected {
                                self.auto_mesh(mi);
                            }
                        }
                        let _ = ui.add_enabled(
                            false,
                            egui::Button::new(crate::i18n::tr("Convert to ArtPath")),
                        );
                    });
                    if ui
                        .add_enabled(
                            self.session.is_some(),
                            egui::Button::new(crate::i18n::tr("Texture Atlas…")),
                        )
                        .on_hover_text(crate::i18n::tr(
                            "Move and pack the meshes' texture pieces across pages; the \
                             model draws the same",
                        ))
                        .clicked()
                    {
                        ui.close();
                        self.open_atlas();
                    }
                    ui.menu_button(crate::i18n::tr("ArtPath"), |ui| {
                        if ui
                            .add_enabled(
                                self.session.is_some(),
                                egui::Button::new(crate::i18n::tr("Create ArtPath")),
                            )
                            .clicked()
                        {
                            self.tool = Tool::ArtPath;
                            self.art_draft = None;
                            ui.close();
                        }
                    });
                    ui.menu_button(crate::i18n::tr("Glue"), |ui| {
                        if ui
                            .add_enabled(
                                self.session.is_some(),
                                egui::Button::new(crate::i18n::tr("Bind Glue"))
                                    .shortcut_text(self.sc("glue-bind")),
                            )
                            .clicked()
                        {
                            ui.close();
                            self.run_mesh_action(MeshAction::Bind);
                        }
                        if ui
                            .add_enabled(
                                self.session.is_some(),
                                egui::Button::new(crate::i18n::tr("Unbind Glue"))
                                    .shortcut_text(self.sc("glue-unbind")),
                            )
                            .clicked()
                        {
                            ui.close();
                            self.run_mesh_action(MeshAction::Unbind);
                        }
                    });
                    ui.menu_button(crate::i18n::tr("Skinning"), |ui| {
                        let _ = ui.add_enabled(
                            false,
                            egui::Button::new(crate::i18n::tr("Apply Skinning…")),
                        );
                    });
                    ui.menu_button(crate::i18n::tr("Edit Form"), |ui| {
                        let flippable = self.edit_target().is_some();
                        for (label, horizontal) in
                            [("Flip Horizontally", true), ("Flip Vertically", false)]
                        {
                            if ui
                                .add_enabled(flippable, egui::Button::new(crate::i18n::tr(label)))
                                .clicked()
                            {
                                ui.close();
                                self.flip_selection(horizontal);
                            }
                        }
                        if ui
                            .add_enabled(
                                self.session.is_some(),
                                egui::Button::new(crate::i18n::tr(
                                    "Automatic Generation of Four Corner Forms…",
                                )),
                            )
                            .clicked()
                        {
                            ui.close();
                            self.run_param_cmd(ParamCmd::FourCorners);
                        }
                    });
                    let _ = ui.add_enabled(
                        false,
                        egui::Button::new(crate::i18n::tr("Temporary Deform Tool")),
                    );
                    ui.separator();
                    if ui
                        .add_enabled(
                            !self.selection_meshes().is_empty(),
                            egui::Button::new(crate::i18n::tr("Show/Hide Selected"))
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
                            egui::Button::new(crate::i18n::tr("Lock/Unlock Selected"))
                                .shortcut_text(self.sc("lock-selected")),
                        )
                        .clicked()
                    {
                        ui.close();
                        self.toggle_selected_lock();
                    }
                    ui.separator();
                    ui.menu_button(crate::i18n::tr("Edit Level"), |ui| {
                        for lvl in 1..=3u8 {
                            if ui
                                .selectable_label(
                                    self.edit_level == lvl,
                                    crate::i18n::trf("Level {level}", &[("level", &lvl)]),
                                )
                                .clicked()
                            {
                                self.edit_level = lvl;
                                ui.close();
                            }
                        }
                    });
                    ui.separator();
                    if ui
                        .add_enabled(
                            self.session.is_some(),
                            egui::Button::new(crate::i18n::tr("Physics Settings…")),
                        )
                        .clicked()
                    {
                        self.show_physics = true;
                        ui.close();
                    }
                    ui.separator();
                    let _ =
                        ui.add_enabled(false, egui::Button::new(crate::i18n::tr("Texture Atlas…")));
                    let _ = ui
                        .add_enabled(false, egui::Button::new(crate::i18n::tr("Model Template…")));
                });
                ui.menu_button(crate::i18n::tr("Animation"), |ui| {
                    if ui.button(crate::i18n::tr("Animation Workspace")).clicked() {
                        self.set_workspace(Workspace::Animation);
                        ui.close();
                    }
                    ui.separator();
                    if ui
                        .add_enabled(
                            self.session.is_some(),
                            egui::Button::new(crate::i18n::tr("New Animation…")),
                        )
                        .clicked()
                    {
                        ui.close();
                        self.new_anim = Some(AnimNew {
                            name: "idle".into(),
                            fps: 30.0,
                            duration: 3.0,
                            looped: true, comp: false,
                        });
                    }
                    if ui
                        .add_enabled(
                            self.session.is_some(),
                            egui::Button::new(crate::i18n::tr("New Composition…")),
                        )
                        .on_hover_text(crate::i18n::tr(
                            "Scenes and other compositions laid out as layers over one clock.",
                        ))
                        .clicked()
                    {
                        ui.close();
                        self.new_anim = Some(AnimNew {
                            name: "comp".into(),
                            fps: 30.0,
                            duration: 5.0,
                            looped: false,
                            comp: true,
                        });
                    }
                    let has_anim = self.session.as_ref().is_some_and(|s| s.current_anim.is_some());
                    if ui
                        .add_enabled(
                            has_anim,
                            egui::Button::new(if self.anim_playing {
                                crate::i18n::tr("Pause")
                            } else {
                                crate::i18n::tr("Play")
                            }),
                        )
                        .clicked()
                    {
                        ui.close();
                        self.anim_playing = !self.anim_playing;
                    }
                    ui.checkbox(&mut self.anim_autokey, crate::i18n::tr("Auto key"));
                });
                ui.menu_button(crate::i18n::tr("Window"), |ui| {
                    // one checkbox per palette; the menu stays open so several
                    // can be toggled in one go
                    for p in PanelKind::ALL {
                        let was = self.layout.contains(p);
                        let mut open = was;
                        if ui.checkbox(&mut open, crate::i18n::tr(p.title())).changed() {
                            if was {
                                self.layout.close(p);
                            } else {
                                self.layout.open_near(p, PanelKind::Viewport);
                            }
                            self.save_layout();
                        }
                    }
                    ui.separator();
                    if ui
                        .add_enabled(self.session.is_some(), egui::Button::new(crate::i18n::tr("Timelapse…")))
                        .clicked()
                    {
                        self.open_timelapse();
                        ui.close();
                    }
                    if ui.button(crate::i18n::tr("Settings…")).clicked() {
                        self.show_settings = true;
                        ui.close();
                    }
                    ui.separator();
                    ui.menu_button(crate::i18n::tr("Workspace"), |ui| {
                        if ui.button(crate::i18n::tr("Save as New Workspace…")).clicked() {
                            self.show_ws_save = true;
                            ui.close();
                        }
                        if ui.button(crate::i18n::tr("Manage Workspaces…")).clicked() {
                            self.show_ws_manage = true;
                            ui.close();
                        }
                        ui.separator();
                        if ui.button(crate::i18n::tr("Export Current Layout…")).clicked() {
                            ui.close();
                            self.pick_async(PickFor::WorkspaceExport, || {
                                rfd::FileDialog::new()
                                    .add_filter(crate::i18n::tr("Workspace"), &["json"])
                                    .set_file_name("workspace.json")
                                    .save_file()
                            });
                        }
                        if ui.button(crate::i18n::tr("Import Layout…")).clicked() {
                            ui.close();
                            self.pick_async(PickFor::WorkspaceImport, || {
                                rfd::FileDialog::new()
                                    .add_filter(crate::i18n::tr("Workspace"), &["json"])
                                    .pick_file()
                            });
                        }
                    });
                    if ui.button(crate::i18n::tr("Reset Current Layout")).clicked() {
                        self.reset_workspace();
                        ui.close();
                    }
                });
                ui.menu_button(crate::i18n::tr("API"), |ui| self.api_menu(ui));
                ui.menu_button(crate::i18n::tr("Help"), |ui| {
                    if ui.button(crate::i18n::tr("About Ayatsuri2D")).clicked() {
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
                            .selectable_label(
                                self.workspace == Workspace::Modeling,
                                crate::i18n::tr("Modeling"),
                            )
                            .clicked()
                        {
                            pick = Some(Workspace::Modeling);
                        }
                        if ui
                            .selectable_label(
                                self.workspace == Workspace::Animation,
                                crate::i18n::tr("Animation"),
                            )
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
                        if ui.button(crate::i18n::tr("Save as new workspace…")).clicked() {
                            self.show_ws_save = true;
                        }
                        if ui.button(crate::i18n::tr("Manage workspaces…")).clicked() {
                            self.show_ws_manage = true;
                        }
                    });
                if let Some(w) = pick {
                    self.set_workspace(w);
                }
                ui.separator();
                ui.weak(crate::i18n::tr("Edit Level:"));
                for lvl in 1..=3u8 {
                    if ui.selectable_label(self.edit_level == lvl, lvl.to_string()).clicked() {
                        self.edit_level = lvl;
                    }
                }
                ui.separator();
                let has_physics = self.session.as_ref().is_some_and(|s| s.puppet.has_physics());
                if ui
                    .add_enabled_ui(has_physics, |ui| {
                        ui.selectable_label(
                            self.physics_on && has_physics,
                            crate::i18n::tr("Physics"),
                        )
                        .on_hover_text(crate::i18n::tr("Tick the pendulum rig (physics3)"))
                    })
                    .inner
                    .clicked()
                {
                    self.physics_on = !self.physics_on;
                    log::debug!("physics {}", if self.physics_on { "on" } else { "off" });
                    if let Some(s) = &mut self.session {
                        if self.physics_on {
                            s.puppet.reset_physics();
                        } else {
                            // off: the parameters physics drives go back to
                            // rest, so they sit on their keys for editing
                            let outputs: Vec<String> = s
                                .puppet
                                .model()
                                .physics
                                .iter()
                                .flat_map(|rig| rig.groups.iter())
                                .flat_map(|g| g.outputs.iter().map(|o| o.param.clone()))
                                .collect();
                            let rest: Vec<(usize, f32)> = s
                                .puppet
                                .model()
                                .parameters
                                .iter()
                                .enumerate()
                                .filter(|(_, p)| outputs.contains(&p.id))
                                .map(|(i, p)| (i, p.default))
                                .collect();
                            for (i, v) in &rest {
                                s.puppet.set_value(*i, *v);
                            }
                            s.dirty = true;
                            log::debug!("{} physics parameter(s) back to rest", rest.len());
                        }
                    }
                }
                if ui
                    .selectable_label(self.follow_cursor, crate::i18n::tr("Follow"))
                    .on_hover_text(crate::i18n::tr(
                        "Drive the standard face parameters with the cursor",
                    ))
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
            let mut area = egui::Area::new(egui::Id::new("tools-float"))
                .default_pos(default_pos)
                .movable(true);
            if std::mem::take(&mut self.toolbar_reset) {
                area = area.current_pos(default_pos);
            }
            let card_id = egui::Id::new("tools-float-card");
            let card = ui.ctx().data(|d| d.get_temp::<Rect>(card_id));
            let area = area.show(ui.ctx(), |ui| {
                // the whole card answers a right-click, under the buttons
                // (registered first, so they still win where they are)
                if let Some(card) = card {
                    ui.interact(card, card_id, Sense::click())
                        .context_menu(|ui| self.toolbar_menu(ui));
                }
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
            ui.ctx().data_mut(|d| d.insert_temp(card_id, area.response.rect));
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
        // layer on top of it the way the reference player stacks them; a
        // composition drives them through its layers' blends over the rest
        // pose instead
        if let Some(s) = &mut self.session {
            let comp = s.current_comp.and_then(|ci| s.compositions.get(ci).cloned());
            let scene = match comp {
                Some(_) => None,
                None => {
                    s.current_anim.and_then(|ai| s.animations.get(ai).cloned().map(|a| (ai, a)))
                }
            };
            let clock = comp
                .as_ref()
                .map(|c| (c.duration, c.looped, c.work_area))
                .or_else(|| scene.as_ref().map(|(_, a)| (a.duration, a.looped, a.work_area)));
            if let Some((duration, looped, work_area)) = clock {
                let dt = ui.ctx().input(|i| i.stable_dt).clamp(0.0, 0.1);
                if self.anim_playing {
                    // play runs inside the work area when there is one
                    let (w0, w1) = work_area.unwrap_or((0.0, duration));
                    let (w0, w1) = (w0.clamp(0.0, duration), w1.clamp(w0, duration));
                    if self.anim_time < w0 {
                        self.anim_time = w0;
                    }
                    self.anim_time += dt;
                    if self.anim_time > w1 {
                        if looped && w1 > w0 {
                            self.anim_time = w0 + (self.anim_time - w0) % (w1 - w0);
                        } else {
                            self.anim_time = w1;
                            self.anim_playing = false;
                        }
                    }
                    ui.ctx().request_repaint();
                }
                let moved = self.anim_time != self.anim_applied;
                if moved || self.anim_playing {
                    if let Some(c) = &comp {
                        let scope = ayatsuri_core::comp::Scope {
                            scenes: &s.animations,
                            comps: &s.compositions,
                        };
                        for ((target, id), b) in
                            ayatsuri_core::comp::sample(&scope, c, self.anim_time)
                        {
                            match target {
                                ayatsuri_format::CurveTarget::Parameter => {
                                    if let Some((pi, p)) = s
                                        .puppet
                                        .model()
                                        .parameters
                                        .iter()
                                        .enumerate()
                                        .find(|(_, p)| p.id == id)
                                    {
                                        let v = b.over(p.default);
                                        s.puppet.set_value(pi, v);
                                    }
                                }
                                ayatsuri_format::CurveTarget::PartOpacity => {
                                    if let Some(i) =
                                        s.puppet.model().parts.iter().position(|p| p.id == id)
                                    {
                                        let shown = s.part_shown.get(i).copied().unwrap_or(true);
                                        let v = b.over(1.0);
                                        s.puppet.set_part_opacity(i, if shown { v } else { 0.0 });
                                    }
                                }
                                ayatsuri_format::CurveTarget::Model => {}
                            }
                        }
                    } else if let Some((_, anim)) = &scene {
                        for c in &anim.curves {
                            let Some(v) = ayatsuri_core::anim::sample(anim, c, self.anim_time)
                            else {
                                continue;
                            };
                            match c.target {
                                ayatsuri_format::CurveTarget::Parameter => {
                                    if let Some(pi) = s
                                        .puppet
                                        .model()
                                        .parameters
                                        .iter()
                                        .position(|p| p.id == c.id)
                                    {
                                        s.puppet.set_value(pi, v);
                                    }
                                }
                                // animated part opacity sits under the editor's own
                                // show/hide: a hidden part stays hidden
                                ayatsuri_format::CurveTarget::PartOpacity => {
                                    if let Some(i) =
                                        s.puppet.model().parts.iter().position(|p| p.id == c.id)
                                    {
                                        let shown = s.part_shown.get(i).copied().unwrap_or(true);
                                        s.puppet.set_part_opacity(i, if shown { v } else { 0.0 });
                                    }
                                }
                                // model opacity, eye blink and lip sync are player
                                // effects; the editor plays the parameters they drive
                                ayatsuri_format::CurveTarget::Model => {}
                            }
                        }
                    }
                    self.anim_applied = self.anim_time;
                    s.dirty = true;
                } else if self.anim_autokey
                    && !self.follow_cursor
                    && let Some((ai, anim)) = &scene
                {
                    let ai = *ai;
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
                        let reference = ayatsuri_core::anim::param_curve(anim, &p.id)
                            .and_then(|c| ayatsuri_core::anim::sample(anim, c, self.anim_time))
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
                        self.timeline.remember(ai, a, None);
                        let fps_a = a.fps;
                        for (id, v) in &keyed {
                            if let Some(c) = ayatsuri_core::anim::curve_mut(
                                a,
                                ayatsuri_format::CurveTarget::Parameter,
                                id,
                            ) {
                                ayatsuri_core::anim::set_key(
                                    c,
                                    self.anim_time,
                                    *v,
                                    fps_a,
                                    ayatsuri_format::Interp::Auto,
                                );
                            }
                        }
                        s.modified = true;
                        log::debug!(
                            "auto key: {} parameter(s) at {:.3}s",
                            keyed.len(),
                            self.anim_time
                        );
                    }
                }
            }
        }

        // cursor following and the pendulum rig both drive parameters, so
        // they run just ahead of the per-frame evaluate
        if let Some(s) = &mut self.session {
            let dt = ui.ctx().input(|i| i.stable_dt).clamp(0.0, 0.1);
            let mut following = false;
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
                let before = self.follow;
                following = (target - self.follow).length() > 1e-3;
                self.follow =
                    if following { self.follow + (target - self.follow) * ease } else { target };
                let f = self.follow;
                let changed = (f - before).length() > 1e-5;
                // the published standard parameter set every tracker expects
                for (id, v) in [
                    ("ParamAngleX", f.x * 30.0),
                    ("ParamAngleY", -f.y * 30.0),
                    ("ParamBodyAngleX", f.x * 10.0),
                    ("ParamBodyAngleY", -f.y * 10.0),
                    ("ParamEyeBallX", f.x),
                    ("ParamEyeBallY", -f.y),
                ] {
                    if changed
                        && let Some(i) = s.puppet.model().parameters.iter().position(|p| p.id == id)
                    {
                        s.puppet.set_value(i, v);
                        s.dirty = true;
                    }
                }
            }
            let physics_moved =
                self.physics_on && s.puppet.has_physics() && s.puppet.step_physics(dt);
            let animating = following || physics_moved;
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
        let mut never = false;
        self.ffmpeg_prompt.show(ui.ctx(), &mut never);
        if never {
            self.prefs.ffmpeg_prompt_off = true;
            self.prefs.save();
        }
        self.settings_window(ui);
        self.stats_window(ui);
        if let Some(s) = &mut self.session {
            s.timelapse_on = s.timelapse_pref.unwrap_or(!self.prefs.timelapse_off);
        }
        if let (Some(v), Some(s)) = (&mut self.timelapse_view, &self.session)
            && !v.show(ui.ctx(), s)
        {
            self.timelapse_view = None;
        }
        self.mesh_edit_window(ui);
        self.param_new_window(ui);
        self.folder_new_window(ui);
        self.model_new_window(ui);
        self.model_info_window(ui);
        self.new_anim_window(ui);
        self.export_window(ui);
        self.render_window(ui);
        self.atlas_window(ui);
        self.export_overwrite_window(ui);
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
        let mut dialog_action: Option<MeshAction> = None;
        crate::windows::Dialog::new(crate::i18n::tr("Edit Mesh"))
            .width(290.0)
            // beside the canvas, not over the mesh being edited
            .at(ui.ctx().content_rect().right_top() + vec2(-200.0, 220.0))
            .show(ui.ctx(), |ui| {
                ui.label(truncate(&id, 28));
                ui.weak(crate::i18n::trf(
                    "{verts} vertices · {tris} triangles",
                    &[("verts", &verts), ("tris", &tris)],
                ));
                // the same controls as Tool Details' mesh section, so both match
                egui::ScrollArea::vertical()
                    .id_salt("edit-mesh-popup")
                    .max_height(460.0)
                    .show(ui, |ui| self.td_mesh(ui, &mut dialog_action));
                ui.separator();
                ui.horizontal(|ui| {
                    if ui
                        .button(egui::RichText::new(crate::i18n::tr("Done")).color(Color32::WHITE))
                        .clicked()
                    {
                        done = true;
                    }
                    if ui.button(crate::i18n::tr("Cancel")).clicked() {
                        cancel = true;
                    }
                });
                ui.weak(crate::i18n::tr("Esc finishes too; Ctrl+Z steps back"));
            });
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
        if ui.button(crate::i18n::tr("Reset Parameter to Default")).clicked() {
            self.reset_parameters();
        }
        ui.separator();
        // the Parameter palette's own list, folders and pads included, but
        // only for moving values: no link buttons, menus or add/delete
        let h = ui.available_height().max(40.0);
        let actions = self.param_list(ui, "phys-params", false, h);
        for a in actions {
            if let ParamAction::Pick(pi, _) = a {
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
        live_chain: Option<Vec<(f32, f32)>>,
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
        // at rest the preview mirrors the running physics for this group, so
        // moving its input parameters swings it; a drag or Swing hands it to
        // the private sim until that settles
        let drawn: Vec<(f32, f32)> = match live_chain.filter(|c| c.len() == nodes.len()) {
            Some(chain) if !live => {
                ui.ctx().request_repaint();
                chain
            }
            _ => self.phys_prev.iter().map(|p| (p.0, p.1)).collect(),
        };
        let stroke = Stroke::new(1.6, theme::pal().rotation_red);
        let mut prev: Option<Pos2> = None;
        for (i, &(x, y)) in drawn.iter().enumerate() {
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
            crate::i18n::trf(
                "Scale: {scale}\nAngle: {angle}",
                &[
                    ("scale", &format!("{:.1}", self.phys_zoom * 10.0)),
                    ("angle", &format!("{:.1}", -self.phys_page.prev_rot)),
                ],
            ),
            FontId::proportional(9.5),
            ink,
        );
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 3.0;
            if ui.small_button("1:1").on_hover_text(crate::i18n::tr("Full scale")).clicked() {
                self.phys_zoom = 1.0;
            }
            if ui
                .small_button(egui_phosphor::regular::CORNERS_OUT)
                .on_hover_text(crate::i18n::tr("All: fit the whole pendulum"))
                .clicked()
            {
                self.phys_zoom = 1.0;
                self.phys_page.prev_rot = 0.0;
            }
            if ui
                .small_button(egui_phosphor::regular::ARROW_COUNTER_CLOCKWISE)
                .on_hover_text(crate::i18n::tr("Rotate left 45°"))
                .clicked()
            {
                self.phys_page.prev_rot -= 45.0;
            }
            if ui
                .small_button(egui_phosphor::regular::ARROW_CLOCKWISE)
                .on_hover_text(crate::i18n::tr("Rotate right 45°"))
                .clicked()
            {
                self.phys_page.prev_rot += 45.0;
            }
            if ui
                .small_button(crate::i18n::tr("Swing"))
                .on_hover_text(crate::i18n::tr("Give the pendulum a push"))
                .clicked()
            {
                self.phys_prev_drive = reach * 0.4;
                self.phys_prev_until = ui.input(|i| i.time) + 4.0;
            }
        });
    }
    fn workspace_windows(&mut self, ui: &mut Ui) {
        let mut open = self.show_ws_save;
        let mut saved = false;
        crate::windows::Dialog::new(crate::i18n::tr("Save Workspace")).open(&mut open).show(
            ui.ctx(),
            |ui| {
                ui.label(crate::i18n::tr("Name"));
                ui.text_edit_singleline(&mut self.ws_name);
                if ui.button(crate::i18n::tr("Save")).clicked() && !self.ws_name.trim().is_empty() {
                    let name = self.ws_name.trim().to_string();
                    self.prefs.custom_workspaces.push((name, self.layout.clone()));
                    let idx = self.prefs.custom_workspaces.len() - 1;
                    self.workspace = Workspace::Custom(idx);
                    self.prefs.save();
                    self.ws_name.clear();
                    saved = true;
                }
            },
        );
        self.show_ws_save = open && !saved;

        let mut open = self.show_ws_manage;
        crate::windows::Dialog::new(crate::i18n::tr("Workspaces")).open(&mut open).show(
            ui.ctx(),
            |ui| {
                let mut delete: Option<usize> = None;
                if self.prefs.custom_workspaces.is_empty() {
                    ui.weak(crate::i18n::tr("No custom workspaces yet."));
                }
                for (i, (name, _)) in self.prefs.custom_workspaces.iter().enumerate() {
                    ui.horizontal(|ui| {
                        ui.label(name);
                        if ui.small_button(crate::i18n::tr("Delete")).clicked() {
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
            crate::i18n::trf(
                "Ayatsuri2D {version} · open 2D puppet studio",
                &[("version", &env!("CARGO_PKG_VERSION"))],
            ),
            FontId::proportional(12.5),
            theme::pal().text_faint,
        );

        let body = Rect::from_min_max(pos2(rect.min.x + 36.0, band.max.y + 18.0), rect.max);
        let mut ui = ui.new_child(UiBuilder::new().max_rect(body));
        // a newer release: a full-width strip above everything, text on the
        // left, Later and the update buttons on the right; Later holds until the next
        if let Some(latest) = self.update.available().cloned()
            && self.prefs.update_dismissed.as_deref() != Some(latest.version.as_str())
        {
            let width = (ui.available_width() - 36.0).max(200.0);
            egui::Frame::new()
                .fill(theme::pal().accent.gamma_multiply(0.18))
                .stroke(Stroke::new(1.0, theme::pal().accent.gamma_multiply(0.6)))
                .corner_radius(6.0)
                .inner_margin(egui::Margin::symmetric(14, 8))
                .show(&mut ui, |ui| {
                    ui.set_width(width - 28.0);
                    ui.horizontal(|ui| {
                        ui.label(
                            egui::RichText::new(egui_phosphor::regular::ARROW_CIRCLE_UP)
                                .size(16.0)
                                .color(theme::pal().accent),
                        );
                        ui.label(crate::i18n::trf(
                            "Ayatsuri2D {version} is available (you have {current}).",
                            &[("version", &latest.version), ("current", &crate::update::current())],
                        ));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui
                                .button(crate::i18n::tr("Later"))
                                .on_hover_text(crate::i18n::tr("Hide this until the next release"))
                                .clicked()
                            {
                                self.prefs.update_dismissed = Some(latest.version.clone());
                                self.prefs.save();
                            }
                            crate::update::actions(ui, &mut self.update, &latest);
                        });
                    });
                    crate::update::notes(ui, &latest);
                });
            ui.add_space(14.0);
        }
        ui.horizontal(|ui| {
            if ui.add_sized([150.0, 34.0], egui::Button::new(crate::i18n::tr("Open…"))).clicked()
            {
                self.import_dialog(&ui.ctx().clone());
            }
            if ui
                .add_sized([150.0, 34.0], egui::Button::new(crate::i18n::tr("New project…")))
                .clicked()
            {
                self.new_model = Some(ModelNew::default());
            }
            ui.add_space(10.0);
            ui.weak(crate::i18n::tr("or drop a .aom / .moc3 / .model3.json anywhere"));
        });
        if let Some(note) = &self.home_note {
            ui.add_space(6.0);
            ui.colored_label(theme::pal().rotation_red, note);
        }
        ui.add_space(16.0);

        if self.prefs.recents.is_empty() {
            ui.weak(crate::i18n::tr("Models you open appear here."));
            return;
        }
        ui.label(egui::RichText::new(crate::i18n::tr("Recent")).size(13.0).strong());
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
                            crate::i18n::tr("File not found"),
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
                    self.home_note = Some(crate::i18n::trf(
                        "Could not open a new window: {error}",
                        &[("error", &e)],
                    ));
                }
            }
            HomeCmd::ShowInFolder(p) => {
                if let Err(e) = show_in_folder(&p) {
                    self.home_note = Some(crate::i18n::trf(
                        "Could not open the folder: {error}",
                        &[("error", &e)],
                    ));
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
                Err(e) => {
                    self.home_note =
                        Some(crate::i18n::trf("Could not duplicate: {error}", &[("error", &e)]))
                }
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
        crate::windows::Dialog::new(crate::i18n::tr("Rename")).show(ctx, |ui| {
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
                if ui.button(crate::i18n::tr("Rename")).clicked() {
                    commit = true;
                }
                if ui.button(crate::i18n::tr("Cancel")).clicked() {
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
                Err(e) => {
                    self.home_note =
                        Some(crate::i18n::trf("Could not rename: {error}", &[("error", &e)]))
                }
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
                // the whole bar, blank space included, takes the right-click
                ui.interact(bar, ui.id().with("tools-docked-bar"), Sense::click())
                    .context_menu(|ui| self.toolbar_menu(ui));
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
                        egui::RichText::new(crate::i18n::tr(self.tool.label()))
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
                    ui.weak(crate::i18n::tr("Show:"));
                    for (name, lv) in [
                        ("Error", log::Level::Error),
                        ("Warn", log::Level::Warn),
                        ("Info", log::Level::Info),
                        ("Debug", log::Level::Debug),
                    ] {
                        if ui.selectable_label(self.log_view == lv, crate::i18n::tr(name)).clicked()
                        {
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
            ui.weak(crate::i18n::tr("No model loaded."));
            return;
        };
        let mut tp = crate::timeline::Transport {
            time: &mut self.anim_time,
            playing: &mut self.anim_playing,
            autokey: &mut self.anim_autokey,
            applied: &mut self.anim_applied,
            sel_param: &mut self.sel_param,
        };
        match self.timeline.show(ui, s, &mut tp) {
            Some(crate::timeline::Request::NewScene) => {
                self.new_anim = Some(AnimNew {
                    name: "idle".into(),
                    fps: 30.0,
                    duration: 3.0,
                    looped: true,
                    comp: false,
                });
            }
            Some(crate::timeline::Request::NewComp) => {
                self.new_anim = Some(AnimNew {
                    name: "comp".into(),
                    fps: 30.0,
                    duration: 5.0,
                    looped: false,
                    comp: true,
                });
            }
            None => {}
        }
        // K keys the selected parameter row by hand
        if ui.ctx().input(|i| i.key_pressed(egui::Key::K))
            && !ui.ctx().egui_wants_keyboard_input()
            && let Some(pi) = self.sel_param
            && let Some(ai) = s.current_anim
            && let Some(id) = s.puppet.model().parameters.get(pi).map(|p| p.id.clone())
        {
            let v = s.puppet.value(pi);
            if let Some(a) = s.animations.get_mut(ai) {
                self.timeline.remember(ai, a, None);
                let fps_a = a.fps;
                if let Some(c) =
                    ayatsuri_core::anim::curve_mut(a, ayatsuri_format::CurveTarget::Parameter, &id)
                {
                    ayatsuri_core::anim::set_key(
                        c,
                        self.anim_time,
                        v,
                        fps_a,
                        ayatsuri_format::Interp::Auto,
                    );
                }
                s.modified = true;
            }
        }
    }

    fn new_anim_window(&mut self, ui: &mut Ui) {
        let Some(new) = &mut self.new_anim else { return };
        let mut open = true;
        let mut create = false;
        let title = if new.comp { "New Composition" } else { "New Animation" };
        crate::windows::Dialog::new(crate::i18n::tr(title)).open(&mut open).show(ui.ctx(), |ui| {
            crate::windows::form(ui, "anew", |ui| {
                ui.label(crate::i18n::tr("Name"));
                ui.text_edit_singleline(&mut new.name);
                ui.end_row();
                ui.label(crate::i18n::tr("FPS"));
                ui.add(egui::DragValue::new(&mut new.fps).range(1.0..=120.0));
                ui.end_row();
                ui.label(crate::i18n::tr("Length"));
                ui.add(
                    egui::DragValue::new(&mut new.duration)
                        .range(0.1..=600.0)
                        .speed(0.05)
                        .suffix(" s"),
                );
                ui.end_row();
                ui.label(crate::i18n::tr("Loop"));
                ui.checkbox(&mut new.looped, "");
                ui.end_row();
            });
            ui.weak(crate::i18n::tr(if new.comp {
                "Saved inside the .aom; exported baked as motions/<name>.motion3.json."
            } else {
                "Saved inside the .aom; exported as motions/<name>.motion3.json."
            }));
            if ui.button(crate::i18n::tr("Create")).clicked() {
                create = true;
            }
        });
        if create {
            if let (Some(n), Some(s)) = (self.new_anim.take(), &mut self.session) {
                let name = if n.name.trim().is_empty() {
                    if n.comp { "comp".into() } else { "motion".into() }
                } else {
                    n.name
                };
                if n.comp {
                    s.compositions.push(ayatsuri_format::Composition {
                        name: name.clone(),
                        fps: n.fps,
                        duration: n.duration,
                        looped: n.looped,
                        ..ayatsuri_format::Composition::default()
                    });
                    s.current_comp = Some(s.compositions.len() - 1);
                } else {
                    s.animations.push(ayatsuri_format::Animation {
                        name: name.clone(),
                        fps: n.fps,
                        duration: n.duration,
                        looped: n.looped,
                        ..ayatsuri_format::Animation::default()
                    });
                    s.current_anim = Some(s.animations.len() - 1);
                    s.current_comp = None;
                }
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

    /// Add keys on the selected parameter to every selected object (the
    /// whole mesh pick, as Cubism does), as one undo step.
    fn add_keys_to_selection(&mut self, values: Vec<f32>) {
        let targets = self.param_targets();
        if targets.is_empty() {
            log::warn!("select an object before adding keys");
            return;
        }
        let Some(param) = self.sel_param else {
            log::warn!("select a parameter before adding keys");
            return;
        };
        if let Some(s) = &mut self.session {
            let pid =
                s.puppet.model().parameters.get(param).map(|p| p.id.clone()).unwrap_or_default();
            let mut n = 0;
            s.edit(|m| {
                for &t in &targets {
                    n += ayatsuri_core::edit::add_keys(m, t, param, &values) as usize;
                }
                n > 0
            });
            if n > 0 {
                log::info!("added keys {values:?} on {pid} for {n} object(s)");
            }
        }
    }

    /// Delete the key at the parameter's value from every selected object.
    fn delete_key_on_selection(&mut self) {
        let targets = self.param_targets();
        let Some(param) = self.sel_param else { return };
        if let Some(s) = &mut self.session {
            let at = s.puppet.value(param);
            let mut n = 0;
            s.edit(|m| {
                for &t in &targets {
                    n += ayatsuri_core::edit::delete_key(m, t, param, at) as usize;
                }
                n > 0
            });
            if n > 0 {
                log::info!("deleted the key near {at:.3} on {n} object(s)");
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

    /// Auto mesh presets: outline spacing, inner spacing, margin. The first
    /// three are Cubism's. Face standard follows a face mesh a rigger built
    /// by hand: the inside only a little sparser than the outline (about
    /// 1.3 times, where Standard doubles it), so a warp turning the head
    /// bends the cheeks smoothly instead of folding them between sparse
    /// inner points.
    const AUTO_PRESETS: [(&'static str, f32, f32, f32, ayatsuri_core::automesh::Style); 4] = [
        ("Standard", 24.0, 48.0, 8.0, ayatsuri_core::automesh::Style::Lattice),
        ("Deform", 14.0, 28.0, 6.0, ayatsuri_core::automesh::Style::Lattice),
        ("Light", 40.0, 80.0, 10.0, ayatsuri_core::automesh::Style::Lattice),
        ("Face standard", 36.0, 46.0, 6.0, ayatsuri_core::automesh::Style::Face),
    ];

    /// One mesh's new triangulation from its texture's alpha, or why not.
    fn auto_mesh_data(&self, mi: usize) -> Result<(Vec<f32>, Vec<u16>), String> {
        let o = &self.tool_opts;
        let params = ayatsuri_core::automesh::Params {
            outline_spacing: o.auto_spacing.clamp(4.0, 256.0),
            inner_spacing: o.auto_inner.clamp(4.0, 512.0),
            margin: o.auto_margin.clamp(0.0, 128.0),
            threshold: 8,
            style: o.auto_style,
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

    /// Delete a parameter; one still in use asks first whether to unbind it
    /// everywhere (see `delete_parameter_unbinding`).
    fn delete_parameter(&mut self, pi: usize) {
        let Some(s) = &mut self.session else { return };
        let (objects, physics) = ayatsuri_core::edit::param_users(s.puppet.model(), pi);
        if objects + physics > 0 {
            self.pmenu.delete_ask = Some((pi, objects, physics));
            return;
        }
        if !s.edit(|m| ayatsuri_core::edit::delete_parameter(m, pi)) {
            log::warn!("parameter could not be deleted");
            return;
        }
        self.after_parameter_deleted(pi);
    }

    /// Unbind a parameter from everything that uses it, then delete it, as
    /// one undo step.
    fn delete_parameter_unbinding(&mut self, pi: usize) {
        let Some(s) = &mut self.session else { return };
        let done = s.edit(|m| {
            ayatsuri_core::edit::unbind_parameter_everywhere(m, pi);
            ayatsuri_core::edit::delete_parameter(m, pi)
        });
        if !done {
            log::warn!("parameter could not be deleted");
            return;
        }
        self.after_parameter_deleted(pi);
    }

    /// The palette's own bookkeeping once a parameter is gone.
    fn after_parameter_deleted(&mut self, pi: usize) {
        let Some(s) = &mut self.session else { return };
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
        crate::windows::Dialog::new(crate::i18n::tr("New Project")).open(&mut open).show(
            ui.ctx(),
            |ui| {
                ui.set_width(300.0);
                crate::windows::form(ui, "mnew", |ui| {
                    ui.label(crate::i18n::tr("Name"));
                    ui.text_edit_singleline(&mut new.name);
                    ui.end_row();
                    ui.label(crate::i18n::tr("Author"));
                    ui.text_edit_singleline(&mut new.author);
                    ui.end_row();
                    ui.label(crate::i18n::tr("Canvas width"));
                    ui.add(
                        egui::DragValue::new(&mut new.width).range(64.0..=16384.0).suffix(" px"),
                    );
                    ui.end_row();
                    ui.label(crate::i18n::tr("Canvas height"));
                    ui.add(
                        egui::DragValue::new(&mut new.height).range(64.0..=16384.0).suffix(" px"),
                    );
                    ui.end_row();
                    ui.label(crate::i18n::tr("Pixels per unit"));
                    ui.add(egui::DragValue::new(&mut new.ppu).range(1.0..=8192.0));
                    ui.end_row();
                });
                ui.checkbox(
                    &mut new.std_params,
                    crate::i18n::tr("Start with the standard parameter set"),
                );
                ui.weak(crate::i18n::tr(
                    "Angle, eyes, mouth, body and breath, the ids trackers expect.",
                ));
                ui.checkbox(&mut new.timelapse, crate::i18n::tr("Record timelapse"));
                ui.add_space(6.0);
                if ui.button(crate::i18n::tr("Create")).clicked() {
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
            let record = new.timelapse;
            match Session::create(&ui.ctx().clone(), name, new.author, model) {
                Ok(mut s) => {
                    s.timelapse_pref = Some(record);
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
                    .map(|g| {
                        if g.name.is_empty() {
                            crate::i18n::tr("Parameters").into()
                        } else {
                            g.name.clone()
                        }
                    })
                    .collect()
            })
            .unwrap_or_default();
        crate::windows::Dialog::new(crate::i18n::tr("New Parameter")).open(&mut open).show(
            ui.ctx(),
            |ui| {
                ui.set_width(280.0);
                crate::windows::form(ui, "pnew", |ui| {
                    ui.label(crate::i18n::tr("Name"));
                    ui.text_edit_singleline(&mut new.name);
                    ui.end_row();
                    ui.label(crate::i18n::tr("ID"));
                    ui.text_edit_singleline(&mut new.id);
                    ui.end_row();
                    ui.label(crate::i18n::tr("Minimum"));
                    ui.add(egui::DragValue::new(&mut new.min).speed(0.1));
                    ui.end_row();
                    ui.label(crate::i18n::tr("Default"));
                    ui.add(egui::DragValue::new(&mut new.default).speed(0.1));
                    ui.end_row();
                    ui.label(crate::i18n::tr("Maximum"));
                    ui.add(egui::DragValue::new(&mut new.max).speed(0.1));
                    ui.end_row();
                    ui.label(crate::i18n::tr("Folder"));
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
                if ui.button(crate::i18n::tr("Create")).clicked() {
                    create = true;
                }
            },
        );
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

    /// A click on a parameter name: plain picks just it, Ctrl (Cmd on
    /// macOS) toggles it in the pick, Shift picks the run from the last
    /// clicked parameter in palette order.
    fn pick_param(&mut self, pi: usize, mods: egui::Modifiers) {
        let anchor = self.sel_param;
        if let Some(s) = &mut self.session {
            if mods.command {
                if let Some(at) = s.param_picked.iter().position(|&p| p == pi) {
                    s.param_picked.remove(at);
                } else {
                    // a first Ctrl+click keeps the row that was selected
                    if s.param_picked.is_empty()
                        && let Some(a) = anchor
                    {
                        s.param_picked.push(a);
                    }
                    if !s.param_picked.contains(&pi) {
                        s.param_picked.push(pi);
                    }
                }
            } else if mods.shift
                && let Some(a) = anchor
            {
                let order: Vec<usize> =
                    s.groups.iter().flat_map(|g| g.params.iter().copied()).collect();
                let (ia, ib) =
                    (order.iter().position(|&p| p == a), order.iter().position(|&p| p == pi));
                if let (Some(ia), Some(ib)) = (ia, ib) {
                    let (lo, hi) = (ia.min(ib), ia.max(ib));
                    s.param_picked = order.get(lo..=hi).map(<[usize]>::to_vec).unwrap_or_default();
                }
            } else {
                s.param_picked = vec![pi];
            }
        }
        // Shift keeps its anchor so the range can be stretched again
        if !mods.shift {
            self.sel_param = Some(pi);
        }
        self.selected = Some(Sel::Parameter(pi));
    }

    /// The parameters a menu command on `pi` covers: the whole pick, in
    /// palette order, when `pi` is in it, else just `pi`.
    pub(super) fn picked_with(&self, pi: usize) -> Vec<usize> {
        let Some(s) = &self.session else { return vec![pi] };
        if !s.param_picked.contains(&pi) {
            return vec![pi];
        }
        let mut out: Vec<usize> = s
            .groups
            .iter()
            .flat_map(|g| g.params.iter().copied())
            .filter(|p| s.param_picked.contains(p))
            .collect();
        // anything picked but somehow unfiled still comes along
        for &p in &s.param_picked {
            if !out.contains(&p) {
                out.push(p);
            }
        }
        out
    }

    fn folder_new_window(&mut self, ui: &mut Ui) {
        let Some(name) = &mut self.new_folder else { return };
        let mut open = true;
        let mut create = false;
        crate::windows::Dialog::new(crate::i18n::tr("New Folder")).open(&mut open).show(
            ui.ctx(),
            |ui| {
                ui.set_width(220.0);
                ui.text_edit_singleline(name);
                if ui.button(crate::i18n::tr("Create")).clicked() {
                    create = true;
                }
            },
        );
        if create {
            let from = std::mem::take(&mut self.folder_from);
            if let (Some(s), Some(name)) = (&mut self.session, self.new_folder.take()) {
                let name = if name.is_empty() { "Folder".into() } else { name };
                s.groups.push(crate::session::ParamGroup {
                    name,
                    params: Vec::new(),
                    open: true,
                    color: None,
                });
                // "Group into New Folder": the picked parameters move in
                let gi = s.groups.len().saturating_sub(1);
                param_menu::move_params_into(s, &from, gi);
            }
        } else if !open {
            self.new_folder = None;
            self.folder_from.clear();
        }
    }

    fn close_session(&mut self) {
        // closing means the work was saved or knowingly dropped
        if let Some(s) = &self.session {
            s.discard_autosave();
        }
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
        crate::windows::Dialog::new(crate::i18n::tr("Unsaved Changes")).show(ui.ctx(), |ui| {
            ui.set_width(300.0);
            ui.label(crate::i18n::trf(
                "Save changes to {name} before closing?",
                &[("name", &name)],
            ));
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                if ui.button(egui::RichText::new(crate::i18n::tr("Save")).strong()).clicked() {
                    choice = Some(1);
                }
                if ui.button(crate::i18n::tr("Don't Save")).clicked() {
                    choice = Some(2);
                }
                if ui.button(crate::i18n::tr("Cancel")).clicked() {
                    choice = Some(0);
                }
            });
        });
        match choice {
            Some(0) => self.confirm_close = None,
            Some(1) => {
                let known = self.session.as_ref().and_then(|s| s.aom_path.clone());
                match known {
                    // a known path saves now; close only if it worked
                    Some(path) => {
                        if self.save_session_to(path) {
                            self.finish_close(ui.ctx(), action);
                        }
                    }
                    // otherwise the picker asks first and the close follows
                    // the save; the dialog steps aside meanwhile
                    None => {
                        self.confirm_close = None;
                        self.save_session_then(true, Some(action));
                    }
                }
            }
            Some(2) => self.finish_close(ui.ctx(), action),
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
            "export-image" if have => self.open_export(2),
            "export-moc3" if have => self.open_export(0),
            "shortcuts" => {
                self.show_settings = true;
                self.settings_tab = 3;
            }
            // in the Animation workspace the Timeline's key edits come
            // back first; the model's own history is behind them
            "undo" => {
                if let Some(s) = &mut self.session {
                    if self.workspace == Workspace::Animation && self.timeline.undo(s) {
                        self.anim_applied = f32::NAN;
                    } else {
                        s.undo();
                    }
                }
            }
            "redo" | "redo-alt" => {
                if let Some(s) = &mut self.session {
                    if self.workspace == Workspace::Animation && self.timeline.redo(s) {
                        self.anim_applied = f32::NAN;
                    } else {
                        s.redo();
                    }
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
            // B also sizes a brush (B + drag): with a brush already in hand it
            // must not switch tools
            "tool-brush"
                if matches!(self.tool, Tool::DeformBrush | Tool::Glue)
                    || self.mesh_edit.is_some() => {}
            "tool-brush" => self.tool = Tool::BrushSelect,
            "tool-path" => self.tool = Tool::DeformPath,
            "level-1" => self.edit_level = 1,
            "level-2" => self.edit_level = 2,
            "level-3" => self.edit_level = 3,
            "display-all" => {
                self.view_zoom = 1.0;
                self.view_pan = Vec2::ZERO;
            }
            "focus" => self.focus_request = true,
            "mirror-edit" => self.mirror_edit = !self.mirror_edit,
            "guides-clear" => {
                if let Some(s) = &mut self.session
                    && !s.guides.is_empty()
                {
                    s.guides.clear();
                    s.modified = true;
                    log::info!("guide lines cleared");
                }
            }
            "guide-h-mouse" | "guide-v-mouse" | "guide-both-mouse" if have => {
                let which = match id {
                    "guide-h-mouse" => Some(false),
                    "guide-v-mouse" => Some(true),
                    _ => None,
                };
                if let Some(p) = ctx.pointer_latest_pos() {
                    self.pending_guide = Some(GuideCmd::Mouse(which, p));
                }
            }
            "zoom-in" => self.view_zoom = (self.view_zoom * 1.25).clamp(0.02, 40.0),
            "zoom-out" => self.view_zoom = (self.view_zoom / 1.25).clamp(0.02, 40.0),
            "search" => self.focus_search = true,
            "select-all" if have => {
                if let Some(s) = &self.session {
                    self.multi_sel = (0..s.puppet.model().art_meshes.len())
                        .filter(|&m| mesh_pickable(s, m) && !self.mesh_locked.contains(&m))
                        .collect();
                    self.selected = self.multi_sel.last().copied().map(Sel::Mesh);
                    log::info!("selected all {} meshes", self.multi_sel.len());
                }
            }
            "select-children" => {
                if let Some(Sel::Deformer(d)) = self.selected {
                    self.select_children(d);
                }
            }
            "delete-deformer-content" => {
                if let Some(Sel::Deformer(d)) = self.selected {
                    self.delete_deformer_with_content(d);
                }
            }
            "lock-unselected" => {
                let keep = self.selection_meshes();
                if let Some(s) = &self.session {
                    for m in 0..s.puppet.model().art_meshes.len() {
                        if !keep.contains(&m) {
                            self.mesh_locked.insert(m);
                        }
                    }
                }
                log::info!("{} locked mesh(es)", self.mesh_locked.len());
            }
            "unlock-all" => {
                self.mesh_locked.clear();
                self.def_locked.clear();
                log::info!("everything unlocked");
            }
            "copy-form" => self.copy_form(),
            "paste-form" => self.paste_form(),
            "anim-first" | "anim-last" | "anim-prev-frame" | "anim-next-frame"
            | "anim-prev-key" | "anim-next-key" | "anim-play"
                if self.workspace == Workspace::Animation =>
            {
                self.timeline_key(id);
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
    /// Window > Timelapse: the open model's steps, once it has a file.
    fn open_timelapse(&mut self) {
        let Some(s) = &self.session else { return };
        let Some(path) = s.aom_path.clone() else {
            log::info!(
                "save the model first: its timelapse is kept and written with the first save"
            );
            return;
        };
        match crate::timelapse::View::open(&path) {
            Ok(v) => self.timelapse_view = Some(v),
            Err(e) => log::warn!("timelapse: {e}"),
        }
    }

    /// The deformer's own meshes as the selection.
    fn select_children(&mut self, d: usize) {
        let Some(s) = &self.session else { return };
        self.multi_sel = s
            .puppet
            .model()
            .art_meshes
            .iter()
            .enumerate()
            .filter(|(mi, m)| m.parent_deformer == Some(d as u32) && mesh_pickable(s, *mi))
            .map(|(mi, _)| mi)
            .collect();
        self.selected = self.multi_sel.last().copied().map(Sel::Mesh);
    }

    /// Delete a deformer with everything under it, deformers and meshes,
    /// as one undo step.
    fn delete_deformer_with_content(&mut self, d: usize) {
        let Some(s) = &mut self.session else { return };
        let model = s.puppet.model();
        let mut under = vec![d];
        let mut grew = true;
        while grew {
            grew = false;
            for (i, def) in model.deformers.iter().enumerate() {
                if let Some(p) = def.parent_deformer
                    && under.contains(&(p as usize))
                    && !under.contains(&i)
                {
                    under.push(i);
                    grew = true;
                }
            }
        }
        let meshes: Vec<usize> = model
            .art_meshes
            .iter()
            .enumerate()
            .filter(|(_, m)| m.parent_deformer.is_some_and(|p| under.contains(&(p as usize))))
            .map(|(mi, _)| mi)
            .collect();
        let ids: Vec<String> = under
            .iter()
            .rev()
            .filter_map(|&i| model.deformers.get(i).map(|x| x.id.clone()))
            .collect();
        let (nd, nm) = (ids.len(), meshes.len());
        if s.edit(|m| {
            let mut any = ayatsuri_core::edit::delete_art_meshes(m, &meshes);
            for id in &ids {
                if let Some(i) = m.deformers.iter().position(|x| &x.id == id) {
                    any |= ayatsuri_core::edit::delete_deformer(m, i);
                }
            }
            any
        }) {
            self.selected = None;
            self.multi_sel.clear();
            self.def_locked.clear();
            self.def_collapsed.clear();
            log::info!("deleted {nd} deformer(s) and {nm} mesh(es) under them");
        }
    }

    /// Copy Form: the selected mesh's or warp's shape as it stands now.
    fn copy_form(&mut self) {
        let Some(s) = &self.session else { return };
        self.form_clip = match self.selected {
            Some(Sel::Mesh(mi)) => s.puppet.mesh_local_positions(mi).map(|l| (1, l)),
            Some(Sel::Deformer(d)) => s.puppet.warp_local_grid(d).map(|l| (2, l)),
            _ => None,
        };
        if let Some((_, l)) = &self.form_clip {
            log::info!("form copied ({} points)", l.len() / 2);
        }
    }

    /// Paste Form: the copied shape into the keyform the parameters sit on,
    /// for an object with the same number of points.
    fn paste_form(&mut self) {
        let (Some(s), Some((kind, from))) = (&mut self.session, &self.form_clip) else { return };
        let (target, now) = match (self.selected, kind) {
            (Some(Sel::Mesh(mi)), 1) => {
                (ayatsuri_core::edit::Target::ArtMesh(mi), s.puppet.mesh_local_positions(mi))
            }
            (Some(Sel::Deformer(d)), 2) => {
                (ayatsuri_core::edit::Target::Deformer(d), s.puppet.warp_local_grid(d))
            }
            _ => {
                log::warn!("paste form: select an object of the kind the form came from");
                return;
            }
        };
        let Some(now) = now.filter(|n| n.len() == from.len()) else {
            log::warn!("paste form: the object has a different number of points");
            return;
        };
        let values: Vec<f32> =
            (0..s.puppet.model().parameters.len()).map(|i| s.puppet.value(i)).collect();
        let Some(k) = ayatsuri_core::keyform::keyform_at(s.puppet.model(), target, &values) else {
            log::warn!("the parameters are between keys: move them onto a key to paste a form");
            return;
        };
        let moves: Vec<(usize, f32, f32)> = from
            .chunks(2)
            .zip(now.chunks(2))
            .enumerate()
            .filter_map(|(i, (a, b))| match (a, b) {
                ([ax, ay], [bx, by]) => Some((i, ax - bx, ay - by)),
                _ => None,
            })
            .collect();
        if s.edit(|m| match target {
            ayatsuri_core::edit::Target::ArtMesh(mi) => {
                ayatsuri_core::edit::move_vertices_in(m, mi, Some(k), &moves)
            }
            ayatsuri_core::edit::Target::Deformer(d) => {
                ayatsuri_core::edit::move_warp_points(m, d, Some(k), &moves)
            }
            _ => false,
        }) {
            log::info!("form pasted into keyform {k}");
        }
    }

    /// Timeline keys: first/last frame, a frame back or on, the previous or
    /// next key of any curve, play/pause.
    fn timeline_key(&mut self, id: &str) {
        let Some(s) = &self.session else { return };
        // in a composition the layers' ends stand in for keys
        let (fps, duration, mut keys): (f32, f32, Vec<f32>) =
            if let Some(c) = s.current_comp.and_then(|i| s.compositions.get(i)) {
                let ends = c
                    .layers
                    .iter()
                    .flat_map(|l| [l.start, ayatsuri_core::comp::layer_end(l)])
                    .collect();
                (c.fps, c.duration, ends)
            } else if let Some(anim) = s.current_anim.and_then(|i| s.animations.get(i)) {
                let ks = anim.curves.iter().flat_map(|c| c.keys.iter().map(|k| k.time)).collect();
                (anim.fps, anim.duration, ks)
            } else {
                return;
            };
        let frame = 1.0 / fps.max(1.0);
        let t = self.anim_time;
        keys.sort_by(f32::total_cmp);
        self.anim_time = match id {
            "anim-first" => 0.0,
            "anim-last" => duration,
            "anim-prev-frame" => (t - frame).max(0.0),
            "anim-next-frame" => (t + frame).min(duration),
            "anim-prev-key" => keys.iter().rev().copied().find(|k| *k < t - 1e-4).unwrap_or(t),
            "anim-next-key" => keys.iter().copied().find(|k| *k > t + 1e-4).unwrap_or(t),
            _ => t,
        };
        if id == "anim-play" {
            self.anim_playing = !self.anim_playing;
        }
    }

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
    /// File > Export Model: the window on the tab asked for, settings kept
    /// from the last time it was open.
    pub(crate) fn open_export(&mut self, tab: usize) {
        let Some(s) = &self.session else { return };
        let mut dlg = self.export_dlg.take().unwrap_or_else(|| ExportDlg {
            name: s.name.clone(),
            version: ayatsuri_import::ExportVersion::default_index(),
            physics: true,
            motions: true,
            expressions: true,
            tab: 0,
            image_width: s.puppet.model().canvas.width.max(1.0) as u32,
            jpeg: false,
            transparent: true,
        });
        dlg.tab = tab;
        self.export_dlg = Some(dlg);
    }

    /// The Image tab's Export: pick the file, then render at the size asked for.
    pub(crate) fn export_image(&mut self, width: u32, jpeg: bool, transparent: bool) {
        let Some(s) = &self.session else { return };
        let (ext, label) = if jpeg { ("jpg", "JPEG image") } else { ("png", "PNG image") };
        let file = format!("{}.{ext}", s.name);
        let start = self.prefs.export_folder.clone().filter(|d| d.is_dir());
        self.pick_async(PickFor::ExportImage(width, jpeg, transparent), move || {
            let mut d = rfd::FileDialog::new().add_filter(crate::i18n::tr(label), &[ext]);
            if let Some(s) = start {
                d = d.set_directory(s);
            }
            d.set_file_name(file).save_file()
        });
    }

    fn export_image_to(&mut self, path: PathBuf, width: u32, jpeg: bool, transparent: bool) {
        if let Some(dir) = path.parent() {
            self.prefs.export_folder = Some(dir.to_path_buf());
            self.prefs.save();
        }
        let Some(s) = &self.session else { return };
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
        let width = (width.clamp(16, 16384)) as usize;
        let (w, h, mut rgba) = crate::thumb::render(&s.puppet, &tex, width);
        // JPEG has no alpha, and an opaque PNG was asked for: over white
        if jpeg || !transparent {
            for px in rgba.as_chunks_mut::<4>().0 {
                let a = px[3] as u32;
                for c in px.iter_mut().take(3) {
                    *c = ((*c as u32 * a + 255 * (255 - a)) / 255) as u8;
                }
                px[3] = 255;
            }
        }
        let Some(img) = image::RgbaImage::from_raw(w as u32, h as u32, rgba) else {
            log::error!("image export: buffer size mismatch");
            return;
        };
        let result = if jpeg {
            image::DynamicImage::ImageRgba8(img)
                .to_rgb8()
                .save_with_format(&path, image::ImageFormat::Jpeg)
        } else {
            img.save(&path)
        };
        match result {
            Ok(()) => log::info!("image exported to {}", path.display()),
            Err(e) => log::error!("image export: {e}"),
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
            ui.weak(crate::i18n::tr("No model loaded."));
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
            if ui
                .button(chev)
                .on_hover_text(crate::i18n::tr("Collapse or expand all groups"))
                .clicked()
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
                .on_hover_text(crate::i18n::tr("Add keys at min and max (selected object)"));
            let three = ui
                .add_enabled(have_target, egui::Button::new("•••+"))
                .on_hover_text(crate::i18n::tr("Add keys at min, default and max"));
            let one = ui
                .add_enabled(have_target, egui::Button::new("•+"))
                .on_hover_text(crate::i18n::tr("Add a key at the current value"));
            let minus = ui
                .add_enabled(have_target, egui::Button::new("•−"))
                .on_hover_text(crate::i18n::tr("Delete the key nearest the current value"));
            let unbind = ui
                .add_enabled(have_target, egui::Button::new(egui_phosphor::regular::LINK_BREAK))
                .on_hover_text(crate::i18n::tr("Remove the whole binding"));
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
                None => ui.weak(crate::i18n::tr("no object selected")),
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
            if ui
                .button(egui_phosphor::regular::PLUS)
                .on_hover_text(crate::i18n::tr("Add a parameter"))
                .clicked()
            {
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
                .on_hover_text(crate::i18n::tr("Add a folder"))
                .clicked()
            {
                self.new_folder = Some(String::new());
            }
            let deletable = self.sel_param.is_some();
            if ui
                .add_enabled(deletable, egui::Button::new(egui_phosphor::regular::TRASH))
                .on_hover_text(crate::i18n::tr("Delete the selected parameter (must be unused)"))
                .clicked()
                && let Some(pi) = self.sel_param
            {
                self.delete_parameter(pi);
            }
        });
        ui.add_space(8.0);

        for action in actions {
            match action {
                ParamAction::Pick(pi, mods) => self.pick_param(pi, mods),
                ParamAction::Cmd(c) => self.run_param_cmd(c),
            }
        }
    }

    fn param_edit_window(&mut self, ui: &mut Ui) {
        let Some(edit) = &mut self.param_edit else { return };
        let mut open = true;
        let mut apply = false;
        crate::windows::Dialog::new(crate::i18n::tr("Edit Parameter")).open(&mut open).show(
            ui.ctx(),
            |ui| {
                ui.set_width(280.0);
                crate::windows::form(ui, "pedit", |ui| {
                    ui.label(crate::i18n::tr("Name"));
                    ui.text_edit_singleline(&mut edit.name);
                    ui.end_row();
                    ui.label(crate::i18n::tr("ID"));
                    ui.text_edit_singleline(&mut edit.id);
                    ui.end_row();
                    ui.label(crate::i18n::tr("Minimum"));
                    ui.add(egui::DragValue::new(&mut edit.min).speed(0.1));
                    ui.end_row();
                    ui.label(crate::i18n::tr("Default"));
                    ui.add(egui::DragValue::new(&mut edit.default).speed(0.1));
                    ui.end_row();
                    ui.label(crate::i18n::tr("Maximum"));
                    ui.add(egui::DragValue::new(&mut edit.max).speed(0.1));
                    ui.end_row();
                    ui.label(crate::i18n::tr("Repeat"));
                    ui.checkbox(&mut edit.repeat, "");
                    ui.end_row();
                });
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    if ui.button(crate::i18n::tr("OK")).clicked() {
                        apply = true;
                    }
                });
            },
        );
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
            ui.weak(crate::i18n::tr("No model loaded."));
            return;
        };
        let field =
            search_box(ui, &mut self.part_search, crate::i18n::tr("Search parts and meshes"));
        if std::mem::take(&mut self.focus_search)
            && let Some(f) = field
        {
            f.request_focus();
        }
        let query = self.part_search.trim().to_lowercase();
        let searching = !query.is_empty();
        let (part_shown_by_search, mesh_shown_by_search) = if searching {
            let model = s.puppet.model();
            let parents: Vec<Option<u32>> = model.parts.iter().map(|p| p.parent_part).collect();
            let hits: Vec<bool> = model
                .parts
                .iter()
                .enumerate()
                .map(|(i, p)| {
                    let name = s.part_names.get(i).cloned().unwrap_or_default();
                    matches_search(&query, &[&p.id, &name])
                })
                .collect();
            let leaf_parent: Vec<Option<u32>> =
                model.art_meshes.iter().map(|m| m.parent_part).collect();
            let leaf_hit: Vec<bool> = model
                .art_meshes
                .iter()
                .enumerate()
                .map(|(mi, m)| matches_search(&query, &[&m.id, &s.mesh_name(mi)]))
                .collect();
            search_keep(&parents, &hits, &leaf_parent, &leaf_hit)
        } else {
            (
                vec![true; s.puppet.model().parts.len()],
                vec![true; s.puppet.model().art_meshes.len()],
            )
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
            if !part_shown_by_search.get(i).copied().unwrap_or(true) {
                continue;
            }
            order.push((i, depth));
            // a search opens every folded folder so its matches show
            if (searching || !self.part_collapsed.contains(&i))
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
        let mut mesh_cmd: Option<MeshRowCmd> = None;
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
        let part_rows: Vec<usize> = order.iter().map(|o| o.0).collect();
        let mut mesh_rows: Vec<usize> = Vec::new();
        let mut part_click: Option<usize> = None;
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
                if self.selected == Some(Sel::Part(i)) || self.part_pick.contains(&i) {
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
                    part_click = Some(i);
                }
                if resp.hovered() {
                    self.palette_hover = Some(Sel::Part(i));
                }
                // a menu on a picked part acts on the whole pick
                let targets: Vec<usize> =
                    if self.part_pick.contains(&i) { self.part_pick.clone() } else { vec![i] };
                resp.context_menu(|ui| {
                    let label = match (shown, targets.len()) {
                        (true, 1) => crate::i18n::tr("Hide").to_string(),
                        (false, 1) => crate::i18n::tr("Show").to_string(),
                        (true, n) => crate::i18n::trf("Hide {count} Parts", &[("count", &n)]),
                        (false, n) => crate::i18n::trf("Show {count} Parts", &[("count", &n)]),
                    };
                    if ui.button(label).clicked() {
                        for &t in &targets {
                            if let Some(f) = s.part_shown.get_mut(t) {
                                *f = !shown;
                            }
                            s.puppet.set_part_opacity(t, if shown { 0.0 } else { 1.0 });
                        }
                        s.dirty = true;
                        ui.close();
                    }
                    if ui.button(crate::i18n::tr("Select")).clicked() {
                        self.selected = Some(Sel::Part(i));
                        ui.close();
                    }
                });

                // the part's meshes, nested one level deeper, unless folded
                if self.part_collapsed.contains(&i) && !searching {
                    continue;
                }
                let meshes: Vec<(usize, String)> = s
                    .puppet
                    .model()
                    .art_meshes
                    .iter()
                    .enumerate()
                    .filter(|(mi, m)| {
                        m.parent_part == Some(i as u32)
                            && mesh_shown_by_search.get(*mi).copied().unwrap_or(true)
                    })
                    .map(|(mi, _)| (mi, s.mesh_name(mi)))
                    .collect();
                for (mi, mid) in meshes {
                    let x_off = 66.0 + depth as f32 * 14.0;
                    mesh_rows.push(mi);
                    parts_mesh_row(
                        ui,
                        s,
                        &mut self.selected,
                        mi,
                        &mid,
                        x_off,
                        effective,
                        &stack,
                        &mut mesh_cmd,
                        &self.multi_sel,
                        &mut self.palette_hover,
                    );
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
                .filter(|(mi, m)| {
                    m.parent_part.is_none()
                        && mesh_shown_by_search.get(*mi).copied().unwrap_or(true)
                })
                .map(|(mi, _)| (mi, s.mesh_name(mi)))
                .collect();
            for (mi, mid) in loose {
                mesh_rows.push(mi);
                parts_mesh_row(
                    ui,
                    s,
                    &mut self.selected,
                    mi,
                    &mid,
                    30.0,
                    true,
                    &stack,
                    &mut mesh_cmd,
                    &self.multi_sel,
                    &mut self.palette_hover,
                );
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
                    crate::i18n::tr("Glue"),
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
                    if ui.button(crate::i18n::tr("Delete glue")).clicked() {
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
        let (shift, ctrl) = ui.input(|i| (i.modifiers.shift, i.modifiers.command));
        if let Some(i) = part_click {
            self.pick_rows(i, shift, ctrl, &part_rows, true);
            self.selected = Some(Sel::Part(i));
            if !ctrl && !shift {
                self.multi_sel.clear();
            }
        }
        match mesh_cmd {
            Some(MeshRowCmd::Click(mi)) => {
                self.pick_rows(mi, shift, ctrl, &mesh_rows, false);
                self.after_mesh_pick(mi, ctrl, shift);
                if !ctrl && !shift {
                    self.part_pick.clear();
                }
            }
            Some(MeshRowCmd::Delete(mi)) => {
                let meshes = self.mesh_pick_with(mi);
                self.delete_meshes(meshes);
            }
            Some(MeshRowCmd::ClipBy(mi)) => self.start_clip_pick(mi),
            None => {}
        }
    }

    /// After a mesh row was clicked (and `pick_rows` updated the pick): the
    /// main selection follows. A Ctrl+click that took the mesh out leaves
    /// the last one still picked selected, or nothing.
    fn after_mesh_pick(&mut self, mi: usize, ctrl: bool, shift: bool) {
        if !ctrl && !shift {
            self.multi_sel = vec![mi];
            self.selected = Some(Sel::Mesh(mi));
        } else if self.multi_sel.contains(&mi) {
            self.selected = Some(Sel::Mesh(mi));
        } else {
            self.selected = self.multi_sel.last().copied().map(Sel::Mesh);
        }
    }

    /// The Parts palette's picking, for part rows (`parts`) or mesh rows:
    /// a plain click picks one row, Ctrl+click toggles one, Shift+click
    /// picks every row from the last plain or Ctrl click, in palette order.
    fn pick_rows(&mut self, row: usize, shift: bool, ctrl: bool, order: &[usize], parts: bool) {
        let (pick, anchor) = if parts {
            (&mut self.part_pick, &mut self.part_anchor)
        } else {
            // the mesh pick starts from what the canvas had selected
            if let Some(Sel::Mesh(m)) = self.selected {
                if self.multi_sel.is_empty() {
                    self.multi_sel.push(m);
                }
                if self.mesh_anchor.is_none() {
                    self.mesh_anchor = Some(m);
                }
            }
            (&mut self.multi_sel, &mut self.mesh_anchor)
        };
        let at = |r: usize| order.iter().position(|&x| x == r);
        match (shift, ctrl) {
            (true, _) => match (anchor.and_then(at), at(row)) {
                (Some(a), Some(b)) => {
                    let (lo, hi) = (a.min(b), a.max(b));
                    *pick = order.get(lo..=hi).map(<[usize]>::to_vec).unwrap_or_default();
                }
                _ => {
                    *pick = vec![row];
                    *anchor = Some(row);
                }
            },
            (false, true) => {
                match pick.iter().position(|&x| x == row) {
                    Some(p) => {
                        pick.remove(p);
                    }
                    None => pick.push(row),
                }
                *anchor = Some(row);
            }
            (false, false) => {
                *pick = vec![row];
                *anchor = Some(row);
            }
        }
    }

    /// Start picking `mesh`'s clipping masks on the canvas.
    fn start_clip_pick(&mut self, mesh: usize) {
        self.selected = Some(Sel::Mesh(mesh));
        self.multi_sel.clear();
        self.tool = Tool::Arrow;
        self.mesh_edit = None;
        self.clip_pick = Some(mesh);
    }

    /// The meshes a mesh menu acts on: the whole pick when `mesh` is in it,
    /// else just `mesh`.
    fn mesh_pick_with(&self, mesh: usize) -> Vec<usize> {
        let mut pick = self.multi_sel.clone();
        if let Some(Sel::Mesh(m)) = self.selected {
            pick.push(m);
        }
        if pick.contains(&mesh) {
            pick.sort_unstable();
            pick.dedup();
            pick
        } else {
            vec![mesh]
        }
    }

    /// The art meshes selected now: the clicked one and the lasso's.
    fn selected_meshes(&self) -> Vec<usize> {
        let mut out = self.multi_sel.clone();
        if let Some(Sel::Mesh(m)) = self.selected {
            out.push(m);
        }
        out.sort_unstable();
        out.dedup();
        out
    }

    /// Copy the selected art meshes as they are now, every keyform with
    /// them, here and onto the system clipboard (so Ctrl+V works, and they
    /// paste into another project); Cut deletes them after, as one undo step.
    fn copy_meshes(&mut self, ctx: &egui::Context, cut: bool) {
        let meshes = self.selected_meshes();
        let Some(s) = &self.session else { return };
        if meshes.is_empty() {
            return;
        }
        let clip = crate::mesh_clip::copy(s, &meshes);
        ctx.copy_text(clip.to_text());
        log::info!("{} {} art mesh(es)", if cut { "cut" } else { "copied" }, clip.meshes.len());
        self.mesh_clip = Some(clip);
        if cut {
            self.delete_meshes(meshes);
        }
    }

    /// Paste art meshes, as one undo step, and select them: beside their
    /// originals in the project they came from, with their artwork as a new
    /// page in another.
    fn paste_meshes(&mut self, ctx: &egui::Context, clip: &crate::mesh_clip::ClipFile) {
        let Some(s) = &mut self.session else { return };
        match crate::mesh_clip::paste(s, ctx, clip) {
            Ok(added) => {
                for (i, name) in &added {
                    if let Some(n) = s.mesh_names.get_mut(*i) {
                        n.clone_from(name);
                    }
                }
                log::info!("pasted {} art mesh(es)", added.len());
                self.selected = added.first().map(|&(i, _)| Sel::Mesh(i));
                self.multi_sel = added.iter().skip(1).map(|&(i, _)| i).collect();
            }
            Err(e) => log::warn!("paste: {e}"),
        }
    }

    /// Delete art meshes as one undo step, and renumber everything the
    /// editor keeps per mesh so nothing points at the wrong one after.
    fn delete_meshes(&mut self, meshes: Vec<usize>) {
        if self.mesh_edit.is_some() {
            log::warn!("leave Mesh Edit before deleting art meshes");
            return;
        }
        let Some(s) = &mut self.session else { return };
        let names: Vec<String> = meshes.iter().map(|&m| s.mesh_name(m)).collect();
        if !s.edit(|m| ayatsuri_core::edit::delete_art_meshes(m, &meshes)) {
            return;
        }
        log::info!("deleted art mesh(es): {}", names.join(", "));
        let mut gone = meshes;
        gone.sort_unstable();
        // a kept index moves down by how many deleted ones came before it
        let remap = |i: usize| -> Option<usize> {
            if gone.binary_search(&i).is_ok() {
                None
            } else {
                Some(i - gone.partition_point(|&g| g < i))
            }
        };
        self.selected = None;
        self.multi_sel.clear();
        self.selected_glue = None;
        self.mesh_drag = None;
        self.edge_first = None;
        self.pending_edges =
            self.pending_edges.drain().filter_map(|(m, e)| Some((remap(m)?, e))).collect();
        self.deform_paths =
            self.deform_paths.drain().filter_map(|((m, l), p)| Some(((remap(m)?, l), p))).collect();
        self.brush_w = self
            .brush_w
            .drain()
            .filter_map(|((kind, i), w)| match kind {
                1 => Some(((kind, remap(i)?), w)),
                _ => Some(((kind, i), w)),
            })
            .collect();
    }
    /// The toolbar's right-click menu, wherever on the bar it opens.
    fn toolbar_menu(&mut self, ui: &mut Ui) {
        let docked = self.prefs.toolbar_docked;
        if ui
            .button(if docked {
                crate::i18n::tr("Float Toolbar")
            } else {
                crate::i18n::tr("Dock Toolbar")
            })
            .clicked()
        {
            self.prefs.toolbar_docked = !docked;
            self.prefs.save();
            ui.close();
        }
        if !docked && ui.button(crate::i18n::tr("Reset Toolbar Position")).clicked() {
            self.prefs.toolbar_pos = None;
            self.toolbar_reset = true;
            self.prefs.save();
            ui.close();
        }
    }

    fn tool_strip(&mut self, ui: &mut Ui) {
        // Mesh Edit brings its own tools in its popup: the strip greys out
        // and points there, so nobody reaches for a tool that does nothing
        let locked = self.mesh_edit.is_some();
        // fixed-width name slot first, so switching tools never resizes the bar
        if !self.prefs.toolbar_docked {
            let (nr, _) = ui.allocate_exact_size(vec2(96.0, 26.0), Sense::hover());
            ui.painter().text(
                nr.left_center() + vec2(2.0, 0.0),
                Align2::LEFT_CENTER,
                crate::i18n::tr(self.tool.label()),
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
                } else if resp.hovered() && !locked {
                    ui.painter().rect_filled(rect, 5.0, theme::pal().header);
                }
                let icon_color = match (locked, active) {
                    (true, _) => theme::pal().text_faint.gamma_multiply(0.6),
                    (false, true) => theme::pal().accent,
                    (false, false) => theme::pal().text,
                };
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
                resp.context_menu(|ui| self.toolbar_menu(ui));
                let resp = if locked {
                    resp.on_hover_text(crate::i18n::tr(
                        "Mesh Edit has its own tools, in its popup; Done or Esc leaves Mesh Edit",
                    ))
                } else {
                    resp.on_hover_text(crate::i18n::tr(t.label()))
                };
                if resp.clicked() && !locked {
                    self.tool = t;
                }
            }
        }
    }

    fn deformers_panel(&mut self, ui: &mut Ui) {
        let mut def_cmd: Option<DefCmd> = None;
        let mut def_mesh_rows: Vec<usize> = Vec::new();
        let mut def_mesh_click: Option<usize> = None;
        let Some(s) = &mut self.session else {
            ui.weak(crate::i18n::tr("No model loaded."));
            return;
        };
        search_box(ui, &mut self.def_search, crate::i18n::tr("Search deformers and meshes"));
        let query = self.def_search.trim().to_lowercase();
        let searching = !query.is_empty();
        // hierarchy: deformers under their parents, meshes under deformers
        let model = s.puppet.model();
        let nd = model.deformers.len();
        let (def_shown, mesh_shown_by_search) = if searching {
            let parents: Vec<Option<u32>> =
                model.deformers.iter().map(|d| d.parent_deformer).collect();
            let hits: Vec<bool> =
                model.deformers.iter().map(|d| matches_search(&query, &[&d.id])).collect();
            let leaf_parent: Vec<Option<u32>> =
                model.art_meshes.iter().map(|m| m.parent_deformer).collect();
            let leaf_hit: Vec<bool> = model
                .art_meshes
                .iter()
                .enumerate()
                .map(|(mi, m)| matches_search(&query, &[&m.id, &s.mesh_name(mi)]))
                .collect();
            search_keep(&parents, &hits, &leaf_parent, &leaf_hit)
        } else {
            (vec![true; nd], vec![true; model.art_meshes.len()])
        };
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
            if !def_shown.get(i).copied().unwrap_or(true) {
                continue;
            }
            order.push((i, depth));
            // a search opens every folded row so its matches show
            if (searching || !self.def_collapsed.contains(&i))
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
            if egui::DragAndDrop::has_payload_of_type::<Child>(ui.ctx())
                && let Some(p) = ui.ctx().pointer_latest_pos()
            {
                let clip = ui.clip_rect();
                let edge = 28.0;
                let speed = if p.y < clip.min.y + edge && p.y > clip.min.y - edge {
                    (clip.min.y + edge - p.y) / edge * 14.0
                } else if p.y > clip.max.y - edge && p.y < clip.max.y + edge {
                    -(p.y - (clip.max.y - edge)) / edge * 14.0
                } else {
                    0.0
                };
                if speed != 0.0 && p.x >= clip.min.x && p.x <= clip.max.x {
                    ui.scroll_with_delta(vec2(0.0, speed));
                    ui.ctx().request_repaint();
                }
            }
            for (i, dep, id, is_warp, has_kids) in rows {
                let (rect, resp) = ui
                    .allocate_exact_size(vec2(ui.available_width(), 18.0), Sense::click_and_drag());
                if self.selected == Some(Sel::Deformer(i)) {
                    ui.painter().rect_filled(rect, 0.0, theme::pal().select);
                }
                // drag a row onto a deformer to make it the parent
                if resp.drag_started() && !self.def_locked.contains(&i) {
                    resp.dnd_set_drag_payload(Child::Deformer(i));
                }
                if let Some(p) = resp.dnd_hover_payload::<Child>()
                    && *p != Child::Deformer(i)
                {
                    ui.painter().rect_stroke(
                        rect.shrink(1.0),
                        3.0,
                        Stroke::new(1.5, theme::pal().accent),
                        StrokeKind::Inside,
                    );
                }
                if let Some(p) = resp.dnd_release_payload::<Child>() {
                    def_cmd = Some(DefCmd::Reparent(*p, Some(i)));
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
                    if ui
                        .button(crate::i18n::tr("Create Warp Deformer…"))
                        .on_hover_text(crate::i18n::tr("Around this deformer"))
                        .clicked()
                    {
                        def_cmd = Some(DefCmd::Create(true, Sel::Deformer(i)));
                        ui.close();
                    }
                    if ui
                        .button(crate::i18n::tr("Create Rotation Deformer…"))
                        .on_hover_text(crate::i18n::tr("Around this deformer"))
                        .clicked()
                    {
                        def_cmd = Some(DefCmd::Create(false, Sel::Deformer(i)));
                        ui.close();
                    }
                    ui.separator();
                    if ui.button(crate::i18n::tr("Rename…")).clicked() {
                        def_cmd = Some(DefCmd::Rename(i));
                        ui.close();
                    }
                    if ui
                        .button(crate::i18n::tr("Delete"))
                        .on_hover_text(crate::i18n::tr(
                            "Its meshes and deformers move up to its parent, unchanged",
                        ))
                        .clicked()
                    {
                        def_cmd = Some(DefCmd::Delete(i));
                        ui.close();
                    }
                    ui.separator();
                    if ui.button(crate::i18n::tr("Select children")).clicked() {
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
                if self.def_collapsed.contains(&i) && !searching {
                    continue;
                }
                let meshes: Vec<(usize, String)> = s
                    .puppet
                    .model()
                    .art_meshes
                    .iter()
                    .enumerate()
                    .filter(|(mi, m)| {
                        m.parent_deformer == Some(i as u32)
                            && mesh_shown_by_search.get(*mi).copied().unwrap_or(true)
                    })
                    .map(|(mi, _)| (mi, s.mesh_name(mi)))
                    .collect();
                for (mi, mid) in meshes {
                    let (rect, resp) = ui.allocate_exact_size(
                        vec2(ui.available_width(), 15.0),
                        Sense::click_and_drag(),
                    );
                    if resp.drag_started() {
                        resp.dnd_set_drag_payload(Child::Mesh(mi));
                    }
                    // dropping on a mesh row means its deformer
                    if let Some(p) = resp.dnd_release_payload::<Child>() {
                        def_cmd = Some(DefCmd::Reparent(*p, Some(i)));
                    }
                    def_mesh_rows.push(mi);
                    if self.selected == Some(Sel::Mesh(mi)) || self.multi_sel.contains(&mi) {
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
                        def_mesh_click = Some(mi);
                    }
                    resp.context_menu(|ui| {
                        if ui.button(crate::i18n::tr("Create Warp Deformer…")).clicked() {
                            def_cmd = Some(DefCmd::Create(true, Sel::Mesh(mi)));
                            ui.close();
                        }
                        if ui.button(crate::i18n::tr("Create Rotation Deformer…")).clicked() {
                            def_cmd = Some(DefCmd::Create(false, Sel::Mesh(mi)));
                            ui.close();
                        }
                        ui.separator();
                        if ui.button(crate::i18n::tr("Clip by…")).clicked() {
                            def_cmd = Some(DefCmd::ClipBy(mi));
                            ui.close();
                        }
                        if ui.button(crate::i18n::tr("Delete Art Mesh")).clicked() {
                            def_cmd = Some(DefCmd::DeleteMesh(mi));
                            ui.close();
                        }
                    });
                    // the mesh's deform paths, one row per edit level
                    let mut levels: Vec<(u8, usize)> = self
                        .deform_paths
                        .iter()
                        .filter(|((m, _), pins)| *m == mi && !pins.is_empty())
                        .map(|((_, l), pins)| (*l, pins.len()))
                        .collect();
                    levels.sort_unstable();
                    for (level, pins) in levels {
                        let (prect, presp) = ui
                            .allocate_exact_size(vec2(ui.available_width(), 15.0), Sense::click());
                        let on = self.tool == Tool::DeformPath
                            && self.edit_level == level
                            && self.selected == Some(Sel::Mesh(mi));
                        if on {
                            ui.painter().rect_filled(prect, 0.0, theme::pal().select);
                        }
                        let px = prect.min.x + 76.0 + dep as f32 * 14.0;
                        ui.painter().text(
                            pos2(px, prect.center().y),
                            Align2::LEFT_CENTER,
                            egui_phosphor::regular::PATH,
                            FontId::proportional(11.0),
                            theme::pal().accent,
                        );
                        ui.painter().text(
                            pos2(px + 14.0, prect.center().y),
                            Align2::LEFT_CENTER,
                            crate::i18n::trf(
                                "Deform path · level {level} · {pins} pins",
                                &[("level", &level), ("pins", &pins)],
                            ),
                            FontId::proportional(10.0),
                            theme::pal().text_faint,
                        );
                        if presp.clicked() {
                            def_cmd = Some(DefCmd::EditPath(mi, level));
                        }
                        presp.context_menu(|ui| {
                            if ui.button(crate::i18n::tr("Edit Deform Path")).clicked() {
                                def_cmd = Some(DefCmd::EditPath(mi, level));
                                ui.close();
                            }
                            if ui.button(crate::i18n::tr("Delete Deform Path")).clicked() {
                                def_cmd = Some(DefCmd::DeletePath(mi, level));
                                ui.close();
                            }
                        });
                    }
                }
            }
            // the empty space under the rows: create around the selection
            let rest = ui.available_height().max(40.0);
            let (bg_rect, bg) =
                ui.allocate_exact_size(vec2(ui.available_width(), rest), Sense::click());
            // dropped on the empty space: out of every deformer
            if bg.dnd_hover_payload::<Child>().is_some() {
                ui.painter().rect_stroke(
                    bg_rect.shrink(2.0),
                    3.0,
                    Stroke::new(1.0, theme::pal().accent.gamma_multiply(0.6)),
                    StrokeKind::Inside,
                );
            }
            if let Some(p) = bg.dnd_release_payload::<Child>() {
                def_cmd = Some(DefCmd::Reparent(*p, None));
            }
            bg.context_menu(|ui| {
                if ui.button(crate::i18n::tr("Create Warp Deformer…")).clicked() {
                    def_cmd = Some(DefCmd::CreateForSelection(true));
                    ui.close();
                }
                if ui.button(crate::i18n::tr("Create Rotation Deformer…")).clicked() {
                    def_cmd = Some(DefCmd::CreateForSelection(false));
                    ui.close();
                }
            });
        });
        // what is being dragged follows the pointer
        if let Some(p) = egui::DragAndDrop::payload::<Child>(&ctx)
            && let Some(at) = ctx.pointer_latest_pos()
            && let Some(s) = &self.session
        {
            let lines: Vec<String> = match *p {
                Child::Deformer(d) => vec![
                    s.puppet.model().deformers.get(d).map(|d| d.id.clone()).unwrap_or_default(),
                ],
                Child::Mesh(m) if self.multi_sel.len() > 1 && self.multi_sel.contains(&m) => {
                    let n = self.multi_sel.len();
                    let mut v = vec![crate::i18n::trf("{count} meshes", &[("count", &n)])];
                    v.extend(
                        self.multi_sel.iter().take(4).map(|&mi| format!("  {}", s.mesh_name(mi))),
                    );
                    if n > 4 {
                        v.push(crate::i18n::trf("  and {count} more", &[("count", &(n - 4))]));
                    }
                    v
                }
                Child::Mesh(m) => vec![s.mesh_name(m)],
            };
            let painter = ctx
                .layer_painter(egui::LayerId::new(egui::Order::Tooltip, egui::Id::new("def-drag")));
            let galley = painter.layout_no_wrap(
                lines.join("\n"),
                FontId::proportional(11.0),
                theme::pal().accent,
            );
            let card = Rect::from_min_size(at + vec2(14.0, -6.0), galley.size() + vec2(12.0, 8.0));
            painter.rect_filled(card, 4.0, theme::pal().chrome.gamma_multiply(0.95));
            painter.rect_stroke(
                card,
                4.0,
                Stroke::new(1.0, theme::pal().accent.gamma_multiply(0.6)),
                StrokeKind::Inside,
            );
            painter.galley(card.min + vec2(6.0, 4.0), galley, theme::pal().accent);
        }
        if let Some(cmd) = def_cmd {
            self.run_def_cmd(cmd);
        }
        if let Some(mi) = def_mesh_click {
            let (shift, ctrl) = ui.ctx().input(|i| (i.modifiers.shift, i.modifiers.command));
            self.pick_rows(mi, shift, ctrl, &def_mesh_rows, false);
            self.after_mesh_pick(mi, ctrl, shift);
        }
    }

    fn run_def_cmd(&mut self, cmd: DefCmd) {
        match cmd {
            DefCmd::Create(warp, target) => {
                let keep = matches!(target, Sel::Mesh(mi) if self.multi_sel.contains(&mi));
                self.selected = Some(target);
                if !keep {
                    self.multi_sel = match target {
                        Sel::Mesh(mi) => vec![mi],
                        _ => Vec::new(),
                    };
                }
                self.pending_create = Some((warp, None));
            }
            DefCmd::CreateForSelection(warp) => self.pending_create = Some((warp, None)),
            DefCmd::Rename(i) => {
                let id = self
                    .session
                    .as_ref()
                    .and_then(|s| s.puppet.model().deformers.get(i))
                    .map(|d| d.id.clone())
                    .unwrap_or_default();
                self.def_rename = Some((i, id));
            }
            DefCmd::Reparent(child, parent) => {
                let Some(s) = &mut self.session else { return };
                // the palette's per-row state is by index, and a re-parent
                // can re-sort deformers; carry it over by id
                let ids: Vec<String> =
                    s.puppet.model().deformers.iter().map(|d| d.id.clone()).collect();
                let id_of = |i: &usize| ids.get(*i).cloned();
                let locked: Vec<String> = self.def_locked.iter().filter_map(id_of).collect();
                let folded: Vec<String> = self.def_collapsed.iter().filter_map(id_of).collect();
                let picked = match self.selected {
                    Some(Sel::Deformer(d)) => ids.get(d).cloned(),
                    _ => None,
                };
                let group: Vec<Child> = match child {
                    Child::Mesh(mi) if self.multi_sel.len() > 1 && self.multi_sel.contains(&mi) => {
                        self.multi_sel.iter().map(|&m| Child::Mesh(m)).collect()
                    }
                    other => vec![other],
                };
                let count = group.len();
                if s.edit(|m| {
                    let mut any = false;
                    for c in &group {
                        any |= ayatsuri_core::edit::set_parent(m, *c, parent).is_some();
                    }
                    any
                }) {
                    let now: Vec<String> =
                        s.puppet.model().deformers.iter().map(|d| d.id.clone()).collect();
                    let index_of = |id: &String| now.iter().position(|n| n == id);
                    self.def_locked = locked.iter().filter_map(index_of).collect();
                    self.def_collapsed = folded.iter().filter_map(index_of).collect();
                    if let Some(id) = picked {
                        self.selected = index_of(&id).map(Sel::Deformer);
                    }
                    if count > 1 {
                        log::info!("re-parented {count} meshes under {parent:?}");
                    } else {
                        log::info!("re-parented {child:?} under {parent:?}");
                    }
                } else {
                    log::warn!("can't move a deformer under itself or its own children");
                }
            }
            DefCmd::ClipBy(mi) => self.start_clip_pick(mi),
            DefCmd::EditPath(mi, level) => {
                self.selected = Some(Sel::Mesh(mi));
                self.multi_sel = vec![mi];
                self.edit_level = level;
                self.tool = Tool::DeformPath;
            }
            DefCmd::DeletePath(mi, level) => {
                if self.deform_paths.remove(&(mi, level)).is_some() {
                    self.path_grab = None;
                    let name = self.session.as_ref().map(|s| s.mesh_name(mi)).unwrap_or_default();
                    log::info!("deleted the level {level} deform path on {name}");
                }
            }
            DefCmd::DeleteMesh(mi) => {
                let meshes = self.mesh_pick_with(mi);
                self.delete_meshes(meshes);
            }
            DefCmd::Delete(i) => {
                let Some(s) = &mut self.session else { return };
                let name =
                    s.puppet.model().deformers.get(i).map(|d| d.id.clone()).unwrap_or_default();
                if s.edit(|m| ayatsuri_core::edit::delete_deformer(m, i)) {
                    log::info!("deleted deformer {name}");
                    // indices past it moved down one
                    let fix = |set: &mut std::collections::HashSet<usize>| {
                        *set = set
                            .iter()
                            .filter(|&&d| d != i)
                            .map(|&d| if d > i { d - 1 } else { d })
                            .collect();
                    };
                    fix(&mut self.def_collapsed);
                    fix(&mut self.def_locked);
                    if matches!(self.selected, Some(Sel::Deformer(_))) {
                        self.selected = None;
                    }
                }
            }
        }
    }

    fn def_rename_window(&mut self, ctx: &egui::Context) {
        let Some((i, name)) = &mut self.def_rename else { return };
        let (i, mut open, mut ok) = (*i, true, false);
        crate::windows::Dialog::new(crate::i18n::tr("Rename Deformer")).open(&mut open).show(
            ctx,
            |ui| {
                let edit = ui.add(egui::TextEdit::singleline(name).desired_width(240.0));
                if !edit.has_focus() && !edit.lost_focus() {
                    edit.request_focus();
                }
                ok |= edit.lost_focus() && ui.input(|k| k.key_pressed(egui::Key::Enter));
                ui.add_space(6.0);
                ok |= ui.button(crate::i18n::tr("Rename")).clicked();
            },
        );
        if ok {
            let name = name.clone();
            if let Some(s) = &mut self.session {
                if s.edit(|m| ayatsuri_core::edit::rename_deformer(m, i, &name)) {
                    log::info!("deformer {i} renamed to {}", name.trim());
                } else {
                    log::warn!("rename refused: the name is empty or already used");
                }
            }
            self.def_rename = None;
        } else if !open {
            self.def_rename = None;
        }
    }
    fn inspector_panel(&mut self, ui: &mut Ui) {
        ScrollArea::vertical().id_salt("props").show(ui, |ui| self.inspector_contents(ui));
    }

    fn inspector_contents(&mut self, ui: &mut Ui) {
        let Some(s) = &mut self.session else {
            ui.weak(crate::i18n::tr("Nothing selected."));
            return;
        };
        // a glue being edited shows first, the way the reference inspector
        // switches to glue display
        if let Some(gi) = self.selected_glue
            && let Some(g) = s.puppet.model().glues.get(gi).cloned()
        {
            let mesh = |i: u32| s.mesh_name(i as usize);
            let (name_a, name_b) = (mesh(g.art_mesh_a), mesh(g.art_mesh_b));
            crate::windows::form(ui, "glue-props", |ui| {
                ui.weak(crate::i18n::tr("Glue"));
                ui.label(&g.id);
                ui.end_row();
                ui.weak(crate::i18n::tr("Mesh A (red)"));
                ui.label(&name_a);
                ui.end_row();
                ui.weak(crate::i18n::tr("Mesh B (green)"));
                ui.label(&name_b);
                ui.end_row();
                ui.weak(crate::i18n::tr("Bound pairs"));
                ui.label((g.indices.len() / 2).to_string());
                ui.end_row();
                ui.weak(crate::i18n::tr("Compatibility"));
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
            ui.weak(crate::i18n::tr(
                "Lower compatibility weakens the glue; key it per parameter for blends.",
            ));
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
                row(ui, crate::i18n::tr("Name"), s.mesh_name(i));
                row(ui, crate::i18n::tr("ID"), m.id.clone());
                row(
                    ui,
                    crate::i18n::tr("Part"),
                    m.parent_part
                        .and_then(|p| s.part_names.get(p as usize).cloned())
                        .unwrap_or_else(|| "-".into()),
                );
                row(
                    ui,
                    crate::i18n::tr("Deformer"),
                    m.parent_deformer
                        .and_then(|d| s.puppet.model().deformers.get(d as usize))
                        .map_or("-".into(), |d| d.id.clone()),
                );
                row(ui, crate::i18n::tr("Texture"), format!("{}", m.texture));
                row(ui, crate::i18n::tr("Vertices"), format!("{}", m.vertex_count));
                let stack = s.puppet.drawables().position(|d| d.mesh == i);
                row(
                    ui,
                    crate::i18n::tr("Render #"),
                    stack.map_or(crate::i18n::tr("hidden").into(), |n| format!("{n}")),
                );
                ui.separator();

                // the editable half, like Cubism's inspector
                // with several meshes picked, opacity sets them all
                let mut picked: Vec<usize> = self.multi_sel.clone();
                if !picked.contains(&i) {
                    picked = vec![i];
                }
                picked.sort_unstable();
                picked.dedup();
                if picked.len() > 1 {
                    ui.weak(crate::i18n::trf(
                        "{count} meshes selected: opacity and keys apply to all",
                        &[("count", &picked.len())],
                    ));
                }
                let targets: Vec<ayatsuri_core::edit::Target> =
                    picked.iter().map(|&m| ayatsuri_core::edit::Target::ArtMesh(m)).collect();
                opacity_row(ui, s, &targets, self.sel_param);
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
                            egui::RichText::new(crate::i18n::tr("Blend mode"))
                                .color(theme::pal().text_faint)
                                .size(11.0),
                        ),
                    );
                    egui::ComboBox::from_id_salt(("blend", i))
                        .selected_text(crate::i18n::tr(
                            ["Normal", "Additive", "Multiply"]
                                .get(new_blend.min(2))
                                .copied()
                                .unwrap_or("Normal"),
                        ))
                        .show_ui(ui, |ui| {
                            for (bi, name) in ["Normal", "Additive", "Multiply"].iter().enumerate()
                            {
                                ui.selectable_value(&mut new_blend, bi, crate::i18n::tr(name));
                            }
                        });
                });
                let mut double_sided = m.flags & flags::DOUBLE_SIDED != 0;
                let mut inverted = m.flags & flags::INVERTED_MASK != 0;
                ui.horizontal(|ui| {
                    ui.add_space(78.0);
                    ui.checkbox(&mut double_sided, crate::i18n::tr("Double-sided"));
                });
                ui.horizontal(|ui| {
                    ui.add_space(78.0);
                    ui.checkbox(&mut inverted, crate::i18n::tr("Invert mask"));
                });
                let base_order = m.draw_orders.first().copied().unwrap_or(500.0);
                let mut order = base_order;
                ui.horizontal(|ui| {
                    ui.add_sized(
                        [74.0, 16.0],
                        egui::Label::new(
                            egui::RichText::new(crate::i18n::tr("Draw order"))
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
                // clipping: the masks as chips (x takes one off), pick more on
                // the canvas or from the list
                let mut masks = m.masks.clone();
                let mut changed_masks = false;
                ui.horizontal_wrapped(|ui| {
                    ui.add_sized(
                        [74.0, 16.0],
                        egui::Label::new(
                            egui::RichText::new(crate::i18n::tr("Clip by"))
                                .color(theme::pal().text_faint)
                                .size(11.0),
                        ),
                    );
                    if masks.is_empty() {
                        ui.weak(crate::i18n::tr("none"));
                    }
                    let mut drop: Option<usize> = None;
                    for (at, &x) in masks.iter().enumerate() {
                        let chip = egui::Button::new(
                            egui::RichText::new(format!(
                                "{}  ×",
                                truncate(&s.mesh_name(x as usize), 18)
                            ))
                            .size(10.5),
                        )
                        .corner_radius(8.0)
                        .small();
                        if ui
                            .add(chip)
                            .on_hover_text(crate::i18n::tr("Stop clipping by this mesh"))
                            .clicked()
                        {
                            drop = Some(at);
                        }
                    }
                    if let Some(at) = drop {
                        masks.remove(at);
                        changed_masks = true;
                    }
                });
                ui.horizontal(|ui| {
                    ui.add_space(78.0);
                    let picking = self.clip_pick == Some(i);
                    if ui
                        .selectable_label(picking, crate::i18n::tr("Pick on canvas"))
                        .on_hover_text(crate::i18n::tr("Click the mask mesh on the canvas. Shift+click adds several, Esc ends."))
                        .clicked()
                    {
                        self.clip_pick = if picking { None } else { Some(i) };
                        self.tool = Tool::Arrow;
                    }
                    ui.menu_button("+", |ui| {
                        ui.add(
                            egui::TextEdit::singleline(&mut self.clip_search)
                                .hint_text(crate::i18n::tr("Search meshes"))
                                .desired_width(180.0),
                        );
                        let needle = self.clip_search.to_lowercase();
                        let model = s.puppet.model();
                        let is_path = |mi: usize| ayatsuri_core::edit::art_path_of(model, mi).is_some();
                        let mut order: Vec<usize> = (0..model.art_meshes.len())
                            .filter(|&mi| mi != i && !masks.contains(&(mi as u32)))
                            .filter(|&mi| {
                                needle.is_empty() || s.mesh_name(mi).to_lowercase().contains(&needle)
                            })
                            .collect();
                        order.sort_by_key(|&mi| !is_path(mi));
                        egui::ScrollArea::vertical().max_height(260.0).show(ui, |ui| {
                            let mut heading = None;
                            for mi in order {
                                let path = is_path(mi);
                                if heading != Some(path) {
                                    heading = Some(path);
                                    ui.weak(if path {
                                        crate::i18n::tr("ArtPaths")
                                    } else {
                                        crate::i18n::tr("Meshes")
                                    });
                                }
                                if ui.button(s.mesh_name(mi)).clicked() {
                                    masks.push(mi as u32);
                                    changed_masks = true;
                                    ui.close();
                                }
                            }
                        });
                    })
                    .response
                    .on_hover_text(crate::i18n::tr("Add a clipping mask from the list"));
                });
                if changed_masks && s.edit(|model| ayatsuri_core::edit::set_masks(model, i, &masks))
                {
                    log::info!("{} clips by {} mesh(es)", s.mesh_name(i), masks.len());
                }

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
                row(ui, crate::i18n::tr("ID"), d.id.clone());
                row(
                    ui,
                    crate::i18n::tr("Part"),
                    d.parent_part
                        .and_then(|p| s.part_names.get(p as usize).cloned())
                        .unwrap_or_else(|| "-".into()),
                );
                row(
                    ui,
                    crate::i18n::tr("Deformer"),
                    d.parent_deformer
                        .and_then(|p| s.puppet.model().deformers.get(p as usize))
                        .map_or("-".into(), |p| p.id.clone()),
                );
                match &d.kind {
                    DeformerKind::Warp(w) => {
                        row(
                            ui,
                            crate::i18n::tr("Type"),
                            crate::i18n::trf(
                                "Warp {cols}x{rows}",
                                &[("cols", &w.cols), ("rows", &w.rows)],
                            ),
                        );
                        row(ui, crate::i18n::tr("Keyforms"), format!("{}", w.opacities.len()));
                    }
                    DeformerKind::Rotation(r) => {
                        row(ui, crate::i18n::tr("Type"), crate::i18n::tr("Rotation").into());
                        row(ui, crate::i18n::tr("Base angle"), format!("{:.1}", r.base_angle));
                        row(ui, crate::i18n::tr("Keyforms"), format!("{}", r.opacities.len()));
                    }
                }
                let kids = s
                    .puppet
                    .model()
                    .art_meshes
                    .iter()
                    .filter(|m| m.parent_deformer == Some(i as u32))
                    .count();
                row(
                    ui,
                    crate::i18n::tr("Children"),
                    crate::i18n::trf("{count} meshes", &[("count", &kids)]),
                );
                ui.separator();
                opacity_row(ui, s, &[ayatsuri_core::edit::Target::Deformer(i)], self.sel_param);
            }
            Some(Sel::Part(i)) => {
                let Some(p) = s.puppet.model().parts.get(i) else { return };
                row(ui, crate::i18n::tr("Name"), s.part_names.get(i).cloned().unwrap_or_default());
                row(ui, crate::i18n::tr("ID"), p.id.clone());
                row(
                    ui,
                    crate::i18n::tr("Parent"),
                    p.parent_part
                        .and_then(|q| s.part_names.get(q as usize).cloned())
                        .unwrap_or_else(|| "-".into()),
                );
                ui.separator();
                let mut opacity = s.puppet.part_user_opacity(i) * 100.0;
                ui.horizontal(|ui| {
                    ui.add_sized(
                        [74.0, 16.0],
                        egui::Label::new(
                            egui::RichText::new(crate::i18n::tr("View opacity"))
                                .color(theme::pal().text_faint)
                                .size(11.0),
                        ),
                    );
                    let slider =
                        egui::Slider::new(&mut opacity, 0.0..=100.0).suffix("%").fixed_decimals(0);
                    if ui
                        .add(slider)
                        .on_hover_text(crate::i18n::tr(
                            "Only changes what you see while editing; it is not saved or \
                             exported, because moc3 parts have no opacity. To fade a part by a \
                             parameter, put its meshes under a deformer and key that \
                             deformer's Opacity.",
                        ))
                        .changed()
                    {
                        s.puppet.set_part_opacity(i, opacity / 100.0);
                        if let Some(f) = s.part_shown.get_mut(i) {
                            *f = opacity > 0.0;
                        }
                        s.dirty = true;
                    }
                });
            }
            Some(Sel::Parameter(i)) => {
                let Some(p) = s.puppet.model().parameters.get(i) else { return };
                row(ui, crate::i18n::tr("Name"), s.param_names.get(i).cloned().unwrap_or_default());
                row(ui, crate::i18n::tr("ID"), p.id.clone());
                row(
                    ui,
                    crate::i18n::tr("Range"),
                    crate::i18n::trf("{from} to {to}", &[("from", &p.min), ("to", &p.max)]),
                );
                row(ui, crate::i18n::tr("Default"), format!("{}", p.default));
                row(
                    ui,
                    crate::i18n::tr("Keys"),
                    format!("{}", s.param_keys.get(i).map_or(0, Vec::len)),
                );
                row(
                    ui,
                    crate::i18n::tr("Repeat"),
                    if p.repeat {
                        crate::i18n::tr("yes").into()
                    } else {
                        crate::i18n::tr("no").into()
                    },
                );
            }
            None => {
                if self.multi_sel.len() > 1 {
                    ui.weak(crate::i18n::trf(
                        "{count} meshes selected",
                        &[("count", &self.multi_sel.len())],
                    ));
                } else {
                    ui.weak(crate::i18n::tr("Nothing selected."));
                }
            }
        }
    }
    fn textures_panel(&mut self, ui: &mut Ui) {
        let mut open_atlas = false;
        let Some(s) = &self.session else {
            ui.weak(crate::i18n::tr("No textures."));
            return;
        };
        if ui.button(crate::i18n::tr("Edit atlas…")).clicked() {
            open_atlas = true;
        }
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
                        ui.weak(crate::i18n::trf("{index}: failed to load", &[("index", &i)]));
                    }
                }
            }
        });
        if open_atlas {
            self.open_atlas();
        }
    }

    // ---- View ----------------------------------------------------------------

    /// The view area: gray surround, the white canvas page, the puppet, and
    /// the zoom strip along the bottom, Cubism-style.
    fn viewport(&mut self, ui: &mut Ui, content: Rect) {
        let painter = ui.painter_at(content);
        let view_bg = self.prefs.view_bg_color().unwrap_or(theme::pal().canvas_bg);
        painter.rect_filled(content, 0.0, view_bg);

        let strip_h = 26.0;
        let view = Rect::from_min_max(content.min, pos2(content.max.x, content.max.y - strip_h));

        let Some(s) = &mut self.session else {
            painter.text(
                view.center(),
                Align2::CENTER_CENTER,
                crate::i18n::tr("操り2D · drop a .moc3 or .model3.json here"),
                FontId::proportional(13.0),
                theme::pal().text_faint,
            );
            return;
        };

        // space + drag pans, with the grab cursor; middle drag always pans
        let space = ui.ctx().input(|i| i.key_down(egui::Key::Space));
        let resp = ui.interact(view, egui::Id::new("view-nav"), Sense::click_and_drag());
        let cursor_before = ui.ctx().output(|o| o.cursor_icon);
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
        if resp.secondary_clicked() {
            self.ctx_click = resp.interact_pointer_pos();
        }
        resp.context_menu(|ui| {
            ui.menu_button(crate::i18n::tr("Guide Line"), |ui| {
                let at = self.ctx_click;
                for (label, which) in [
                    (crate::i18n::tr("Add Horizontal Line"), Some(false)),
                    (crate::i18n::tr("Add Vertical Line"), Some(true)),
                    (crate::i18n::tr("Add Vertical and Horizontal Lines"), None),
                ] {
                    if ui.add_enabled(at.is_some(), egui::Button::new(label)).clicked()
                        && let Some(p) = at
                    {
                        self.pending_guide = Some(GuideCmd::At(which, p));
                        ui.close();
                    }
                }
                if ui.button(crate::i18n::tr("Add guide lines in the center")).clicked() {
                    self.pending_guide = Some(GuideCmd::Center);
                    ui.close();
                }
                if ui
                    .button(crate::i18n::tr(
                        "Add a guide line to the coordinates of the selected vertex",
                    ))
                    .clicked()
                {
                    self.pending_guide = Some(GuideCmd::AtVertex);
                    ui.close();
                }
                ui.checkbox(&mut self.show_guides, crate::i18n::tr("Show Guide Lines"));
                ui.checkbox(&mut self.mirror_edit, crate::i18n::tr("Mirror Edit"));
                if ui.button(crate::i18n::tr("Open guide line settings")).clicked() {
                    self.show_guide_settings = true;
                    ui.close();
                }
            });
            ui.separator();
            if ui.button(crate::i18n::tr("Fit")).clicked() {
                self.view_zoom = 1.0;
                self.view_pan = Vec2::ZERO;
                ui.close();
            }
            if ui.button(crate::i18n::tr("Reset view")).clicked() {
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
        if std::mem::take(&mut self.focus_request) {
            let mut pts: Vec<[f32; 2]> = Vec::new();
            let mut add = |pos: &[f32]| {
                for p in pos.chunks(2) {
                    if let [x, y] = p {
                        pts.push([
                            x * c.pixels_per_unit + c.origin_x,
                            y * c.pixels_per_unit + c.origin_y,
                        ]);
                    }
                }
            };
            match self.selected {
                Some(Sel::Deformer(d)) => {
                    if let Some((g, _, _)) = s.puppet.warp_grid(d) {
                        add(g);
                    }
                }
                _ => {
                    let picked = if !self.multi_sel.is_empty() {
                        self.multi_sel.clone()
                    } else if let Some(Sel::Mesh(m)) = self.selected {
                        vec![m]
                    } else {
                        Vec::new()
                    };
                    for mi in picked {
                        if let Some(g) = s.puppet.mesh_geometry(mi) {
                            add(g.positions);
                        }
                    }
                }
            }
            if !pts.is_empty() {
                let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
                for [x, y] in &pts {
                    x0 = x0.min(*x);
                    y0 = y0.min(*y);
                    x1 = x1.max(*x);
                    y1 = y1.max(*y);
                }
                let (w, h) = ((x1 - x0).max(1.0), (y1 - y0).max(1.0));
                let want = (view.width() * 0.7 / w).min(view.height() * 0.7 / h);
                self.view_zoom = (want / fit).clamp(0.02, 40.0);
                let sc = fit * self.view_zoom;
                self.view_pan = vec2(cw / 2.0 - (x0 + x1) / 2.0, ch / 2.0 - (y0 + y1) / 2.0) * sc;
            }
        }
        let scale = fit * self.view_zoom;
        let origin = view.center() + self.view_pan - vec2(cw, ch) * scale * 0.5;
        // the mirror axis on screen, when Mirror Edit is on
        let mirror_x = self.mirror_edit.then(|| {
            let at = s
                .guides
                .iter()
                .filter(|g| g.vertical)
                .map(|g| g.at)
                .min_by(|a, b| (a - cw * 0.5).abs().total_cmp(&(b - cw * 0.5).abs()))
                .unwrap_or(cw * 0.5);
            origin.x + at * scale
        });
        let mirror_tol = (3.0 * scale).max(2.0);

        // the canvas page
        let page = Rect::from_min_size(pos2(origin.x, origin.y), vec2(cw * scale, ch * scale));
        let vp = ui.painter_at(view);
        // a hidden mesh can't stay picked
        if self.multi_sel.iter().any(|&m| !mesh_pickable(s, m)) {
            self.multi_sel.retain(|&m| mesh_pickable(s, m));
        }
        if let Some(Sel::Mesh(m)) = self.selected
            && !mesh_pickable(s, m)
        {
            self.selected = self.multi_sel.last().copied().map(Sel::Mesh);
        }
        self.pts.snap = if self.show_guides && !self.prefs.guide_snap_off {
            s.guides
                .iter()
                .map(|g| {
                    (
                        g.vertical,
                        if g.vertical { origin.x + g.at * scale } else { origin.y + g.at * scale },
                    )
                })
                .collect()
        } else {
            Vec::new()
        };
        vp.rect_filled(page, 0.0, theme::pal().canvas);
        vp.rect_stroke(page, 0.0, Stroke::new(1.0, Color32::from_gray(0x8c)), StrokeKind::Outside);

        let to_screen = |x: f32, y: f32| -> Pos2 {
            let px = c.origin_x + x * c.pixels_per_unit;
            let py = c.origin_y + y * c.pixels_per_unit;
            pos2(origin.x + px * scale, origin.y + py * scale)
        };

        if let Some(mi) = self.mesh_edit
            && ayatsuri_core::edit::art_path_of(s.puppet.model(), mi).is_some()
        {
            self.mesh_edit = None;
            self.mesh_snapshot = None;
            self.tool = Tool::ArtPath;
            self.selected = Some(Sel::Mesh(mi));
            self.art_draft = None;
            log::info!(
                "{} is an ArtPath: its points are edited with the ArtPath tool",
                s.mesh_name(mi)
            );
        }
        if self.mesh_edit.is_none() {
            s.drop_full_textures();
        }
        if self.gpu {
            // keep offering the textures until the renderer has them
            if s.gpu_pending.is_some() && s.gpu_uploaded.load(std::sync::atomic::Ordering::Acquire)
            {
                s.gpu_pending = None;
                log::debug!("textures are on the GPU");
            }
            // which pages this frame draws, and whether the small proxy is too
            // coarse for them at this zoom; residency decides what to load or drop
            let ppp = ui.ctx().pixels_per_point();
            let mut used: Vec<(usize, bool)> = Vec::new();
            for d in s.puppet.drawables() {
                if d.opacity <= 0.0 || d.indices.is_empty() {
                    continue;
                }
                let page = d.texture as usize;
                if used.iter().any(|(p, _)| *p == page) {
                    continue;
                }
                let proxy_w = s.preview_tex.get(page).map_or(1, |t| t.width.max(1)) as f32;
                let full_w = s.residency_page_width(page) as f32;
                let need = scale * ppp * (full_w / proxy_w) > 1.0;
                used.push((page, need));
            }
            let idle = ui.ctx().input(|i| {
                let v = i.viewport();
                v.minimized == Some(true) || v.focused == Some(false)
            });
            let budget_mb = match self.prefs.texture_budget_mb {
                0 => 512,
                mb => mb,
            };
            // the Settings budget rules, except after an out-of-memory, when the
            // halved budget stands until the model is reopened
            let pref_budget = budget_mb as usize * (1 << 20);
            s.residency.budget = if s.residency.oom_count == 0 {
                pref_budget
            } else {
                s.residency.budget.min(pref_budget)
            };
            if crate::gpu_out_of_memory() {
                let gone = s.residency.on_out_of_memory();
                s.gpu_drop.extend(gone);
                crate::gpu_memory_ok();
                log::warn!(
                    "video memory ran out; full texture pages dropped and the budget is now {} MB",
                    s.residency.budget >> 20
                );
            }
            let plan = s.residency.tick(&used, idle, &s.texture_pngs);
            if let Some((i, data)) = plan.upload {
                let mut pages = s.gpu_pages.take().map(|p| (*p).clone()).unwrap_or_default();
                pages.retain(|(k, _)| *k != i);
                pages.push((i, data));
                s.gpu_pages = Some(std::sync::Arc::new(pages));
                s.gpu_pages_uploaded.store(false, std::sync::atomic::Ordering::Release);
            }
            s.gpu_drop.extend(plan.drop);
            if s.residency.oom_count > 0 {
                let note = crate::i18n::tr(
                    "Video memory ran out: textures beyond the budget draw at lower detail. Close other models or windows to give it back.",
                );
                let at = view.left_top() + vec2(12.0, 12.0);
                let galley = vp.layout(
                    note.to_string(),
                    FontId::proportional(13.0),
                    Color32::WHITE,
                    view.width() - 48.0,
                );
                let back = Rect::from_min_size(at, galley.size() + vec2(16.0, 10.0));
                vp.rect_filled(back, 6.0, theme::pal().rotation_red.gamma_multiply(0.9));
                vp.galley(at + vec2(8.0, 5.0), galley, Color32::WHITE);
            }
            if s.gpu_pages.is_some()
                && s.gpu_pages_uploaded.load(std::sync::atomic::Ordering::Acquire)
            {
                s.gpu_pages = None;
            }
            let cb = crate::gpu_view::callback(
                view,
                &s.puppet,
                s.gpu_pending.clone(),
                s.gpu_uploaded.clone(),
                s.gpu_pages.clone(),
                s.gpu_pages_uploaded.clone(),
                if s.gpu_drop.is_empty() { None } else { Some(std::mem::take(&mut s.gpu_drop)) },
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

        // a Parts palette row under the pointer: outline its meshes here, a
        // group's whole contents or one mesh (taken, so it lasts one frame)
        if let Some(h) = self.palette_hover.take() {
            let model = s.puppet.model();
            let under = |mi: usize, part: usize| {
                let mut at = model.art_meshes.get(mi).and_then(|m| m.parent_part);
                let mut hops = 0;
                while let Some(p) = at {
                    if p as usize == part {
                        return true;
                    }
                    at = model.parts.get(p as usize).and_then(|x| x.parent_part);
                    hops += 1;
                    if hops > model.parts.len() {
                        break;
                    }
                }
                false
            };
            let meshes: Vec<usize> = match h {
                Sel::Mesh(mi) => vec![mi],
                Sel::Part(p) => (0..model.art_meshes.len()).filter(|&mi| under(mi, p)).collect(),
                _ => Vec::new(),
            };
            let stroke = Stroke::new(1.0, theme::pal().accent.gamma_multiply(0.7));
            for mi in meshes {
                let Some(g) = s.puppet.mesh_geometry(mi) else { continue };
                let (pos, _) = g.positions.as_chunks::<2>();
                let (tris, _) = g.indices.as_chunks::<3>();
                for &[a, b, cx] in tris {
                    let (Some(p0), Some(p1), Some(p2)) =
                        (pos.get(a as usize), pos.get(b as usize), pos.get(cx as usize))
                    else {
                        continue;
                    };
                    let (s0, s1, s2) =
                        (to_screen(p0[0], p0[1]), to_screen(p1[0], p1[1]), to_screen(p2[0], p2[1]));
                    vp.line_segment([s0, s1], stroke);
                    vp.line_segment([s1, s2], stroke);
                    vp.line_segment([s2, s0], stroke);
                }
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
            let ctrl_now = ui.ctx().input(|i| i.modifiers.command);
            if resp.clicked()
                && (!self.pts.hot || ctrl_now)
                && let Some(ptr) = resp.interact_pointer_pos()
            {
                let (ux, uy) = unit_of(ptr);
                let (alt, add) =
                    ui.ctx().input(|i| (i.modifiers.alt, i.modifiers.shift || i.modifiers.command));
                let hit = hit_mesh(&s.puppet, ux, uy, &|m| {
                    self.mesh_locked.contains(&m) || !s.mesh_shown.get(m).copied().unwrap_or(true)
                });
                if let Some(mask) = self.clip_with {
                    let hit = hit_mesh(&s.puppet, ux, uy, &|m| {
                        m == mask
                            || self.mesh_locked.contains(&m)
                            || !s.mesh_shown.get(m).copied().unwrap_or(true)
                    });
                    if let Some(mi) = hit
                        && let Some(mut list) =
                            s.puppet.model().art_meshes.get(mi).map(|m| m.masks.clone())
                    {
                        use ayatsuri_format::model::flags::INVERTED_MASK;
                        let on = !list.contains(&(mask as u32));
                        if on {
                            list.push(mask as u32);
                        } else {
                            list.retain(|&x| x != mask as u32);
                        }
                        let invert = self.clip_invert != alt;
                        let path = ayatsuri_core::edit::art_path_of(s.puppet.model(), mask)
                            .filter(|p| on && !p.mask_only)
                            .cloned()
                            .map(|p| ayatsuri_format::model::ArtPath { mask_only: true, ..p });
                        let ctx = ui.ctx().clone();
                        let done = s.art_path_edit(&ctx, |m, _| {
                            let mut changed = ayatsuri_core::edit::set_masks(m, mi, &list);
                            if let Some(t) = m.art_meshes.get_mut(mi) {
                                let flags = match (on, invert, list.is_empty()) {
                                    (true, true, _) => t.flags | INVERTED_MASK,
                                    (true, false, _) | (false, _, true) => t.flags & !INVERTED_MASK,
                                    (false, _, false) => t.flags,
                                };
                                changed |= flags != t.flags;
                                t.flags = flags;
                            }
                            if let Some(p) = path {
                                changed |= ayatsuri_core::edit::update_art_path(m, mask, p);
                            }
                            changed
                        });
                        if done {
                            if on {
                                log::info!(
                                    "{} now clips by {}{}",
                                    s.mesh_name(mi),
                                    s.mesh_name(mask),
                                    if invert { " (inverted)" } else { "" }
                                );
                            } else {
                                log::info!(
                                    "{} no longer clips by {}",
                                    s.mesh_name(mi),
                                    s.mesh_name(mask)
                                );
                            }
                        }
                    }
                } else if let Some(target) = self.clip_pick {
                    let c = s.puppet.model().canvas;
                    let hit = art_path_at(
                        s.puppet.model(),
                        ux * c.pixels_per_unit + c.origin_x,
                        uy * c.pixels_per_unit + c.origin_y,
                        6.0 / scale,
                    )
                    .filter(|&a| a != target && mesh_pickable(s, a))
                    .or(hit);
                    if let Some(mi) = hit
                        && mi != target
                    {
                        let mut masks =
                            s.puppet.model().art_meshes.get(target).map(|m| m.masks.clone());
                        if let Some(list) = masks.as_mut()
                            && !list.contains(&(mi as u32))
                        {
                            list.push(mi as u32);
                            if s.edit(|m| ayatsuri_core::edit::set_masks(m, target, list)) {
                                log::info!(
                                    "{} now clips by {}",
                                    s.mesh_name(target),
                                    s.mesh_name(mi)
                                );
                            }
                        }
                    }
                    if !add {
                        self.clip_pick = None;
                    }
                } else if add && !alt {
                    // Shift (or Ctrl) + click adds a mesh to the selection or
                    // takes it out; on empty canvas it keeps what is there
                    if let Some(mi) = hit {
                        if self.multi_sel.is_empty()
                            && let Some(Sel::Mesh(cur)) = self.selected
                        {
                            self.multi_sel.push(cur);
                        }
                        if let Some(at) = self.multi_sel.iter().position(|&m| m == mi) {
                            self.multi_sel.remove(at);
                            self.selected = self.multi_sel.last().copied().map(Sel::Mesh);
                        } else {
                            self.multi_sel.push(mi);
                            self.selected = Some(Sel::Mesh(mi));
                        }
                    }
                } else if alt {
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

        // picking clipping masks: say so on the canvas, Esc or another tool ends it
        if let Some(mask) = self.clip_with {
            let gone = s.puppet.model().art_meshes.get(mask).is_none();
            if gone
                || self.tool != Tool::Arrow
                || self.mesh_edit.is_some()
                || ui.ctx().input(|i| i.key_pressed(egui::Key::Escape))
            {
                self.clip_with = None;
            } else {
                let text = crate::i18n::trf(
                    "Click the meshes {mask} should clip  ·  Alt+click clips inverted  ·  click again to take it off  ·  Esc to finish",
                    &[("mask", &s.mesh_name(mask))],
                );
                let at = content.center_top() + vec2(0.0, 14.0);
                let galley = vp.layout_no_wrap(text, FontId::proportional(12.0), Color32::WHITE);
                let bg =
                    Rect::from_center_size(at + vec2(0.0, galley.size().y / 2.0), galley.size())
                        .expand2(vec2(10.0, 5.0));
                vp.rect_filled(bg, 4.0, theme::pal().accent.gamma_multiply(0.9));
                vp.galley(bg.min + vec2(10.0, 5.0), galley, Color32::WHITE);
                for (mi, m) in s.puppet.model().art_meshes.iter().enumerate() {
                    if !m.masks.contains(&(mask as u32)) {
                        continue;
                    }
                    let Some(g) = s.puppet.mesh_geometry(mi) else { continue };
                    let (pos, _) = g.positions.as_chunks::<2>();
                    let stroke = Stroke::new(1.0, theme::pal().accent);
                    for t in g.indices.as_chunks::<3>().0 {
                        let (Some(a), Some(b), Some(c2)) = (
                            pos.get(t[0] as usize),
                            pos.get(t[1] as usize),
                            pos.get(t[2] as usize),
                        ) else {
                            continue;
                        };
                        let (a, b, c2) =
                            (to_screen(a[0], a[1]), to_screen(b[0], b[1]), to_screen(c2[0], c2[1]));
                        vp.line_segment([a, b], stroke);
                        vp.line_segment([b, c2], stroke);
                        vp.line_segment([c2, a], stroke);
                    }
                }
            }
        }
        if let Some(target) = self.clip_pick {
            let gone = s.puppet.model().art_meshes.get(target).is_none();
            if gone
                || self.tool != Tool::Arrow
                || self.mesh_edit.is_some()
                || ui.ctx().input(|i| i.key_pressed(egui::Key::Escape))
            {
                self.clip_pick = None;
            } else {
                let text = crate::i18n::trf(
                    "Click the mesh that clips {mesh}  ·  Shift+click for more  ·  Esc to finish",
                    &[("mesh", &s.mesh_name(target))],
                );
                let at = content.center_top() + vec2(0.0, 14.0);
                let galley = vp.layout_no_wrap(text, FontId::proportional(12.0), Color32::WHITE);
                let bg =
                    Rect::from_center_size(at + vec2(0.0, galley.size().y / 2.0), galley.size())
                        .expand2(vec2(10.0, 5.0));
                vp.rect_filled(bg, 4.0, theme::pal().accent.gamma_multiply(0.9));
                vp.galley(bg.min + vec2(10.0, 5.0), galley, Color32::WHITE);
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
            // nothing selected: the deformer holds the mesh under the click,
            // instead of quietly coming out empty and moving nothing
            // (the selection read by hand: the session is borrowed here)
            let nothing = self.multi_sel.is_empty()
                && !matches!(self.selected, Some(Sel::Mesh(_)) | Some(Sel::Deformer(_)));
            if nothing
                && let Some(mi) = hit_mesh(&s.puppet, ux, uy, &|m| {
                    self.mesh_locked.contains(&m) || !s.mesh_shown.get(m).copied().unwrap_or(true)
                })
            {
                self.selected = Some(Sel::Mesh(mi));
                self.multi_sel = vec![mi];
            }
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
                        if !mesh_pickable(s, mi) || self.mesh_locked.contains(&mi) {
                            continue;
                        }
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
            Some(Sel::Mesh(_)) if self.multi_sel.len() > 1 => {
                let group: Vec<usize> = self
                    .multi_sel
                    .iter()
                    .copied()
                    .filter(|m| !self.mesh_locked.contains(m))
                    .collect();
                let any_weight = group.iter().any(|m| self.brush_w.contains_key(&(1, *m)));
                let mut owner: Vec<(usize, usize)> = Vec::new();
                let mut pts: Vec<Pos2> = Vec::new();
                let mut weights: Vec<f32> = Vec::new();
                let mut wires: Vec<(Vec<u16>, usize)> = Vec::new();
                for &gm in &group {
                    let Some(g) = s.puppet.mesh_geometry(gm) else { continue };
                    let base = pts.len();
                    let w = self.brush_w.get(&(1, gm));
                    for (vi, xy) in g.positions.as_chunks::<2>().0.iter().enumerate() {
                        pts.push(to_screen(xy[0], xy[1]));
                        owner.push((gm, vi));
                        if any_weight {
                            weights.push(w.and_then(|w| w.get(vi)).copied().unwrap_or(0.0));
                        }
                    }
                    wires.push((g.indices.to_vec(), base));
                }
                let editable = matches!(self.tool, Tool::Arrow | Tool::Lasso)
                    && !space
                    && self.mesh_edit.is_none()
                    && !pts.is_empty();
                let key =
                    group.iter().fold(0x9e37_usize, |h, m| h.wrapping_mul(31).wrapping_add(*m));
                let mut grab_hot = None;
                if editable {
                    self.pts.bind((3, key));
                    self.pts.weights = weights;
                    let inp = point_edit::PtInput::read(ui, &resp);
                    let opts = point_edit::PtOpts {
                        whole_box: true,
                        marquee: true,
                        lasso: self.tool == Tool::Lasso,
                        reach: 7.0,
                        pad: 0.0,
                    };
                    let mut out = point_edit::point_tool(&mut self.pts, &inp, &mut pts, &opts);
                    if let (Some(ax), Some(m)) = (mirror_x, &mut out.commit) {
                        point_edit::mirror_moves(m, &pts, ax, mirror_tol);
                    }
                    warp_hot |= self.pts.hot;
                    grab_hot = out.hover_grab;
                    if ui.ctx().input(|i| i.key_pressed(egui::Key::Escape)) {
                        self.pts.picked.clear();
                    }
                    if let Some(moved) = out.commit {
                        let k_unit = 1.0 / (scale * c.pixels_per_unit);
                        let values: Vec<f32> = (0..s.puppet.model().parameters.len())
                            .map(|i| s.puppet.value(i))
                            .collect();
                        let mut per: std::collections::BTreeMap<usize, Vec<(usize, f32, f32)>> =
                            std::collections::BTreeMap::new();
                        let mut locals: std::collections::HashMap<usize, Vec<f32>> =
                            std::collections::HashMap::new();
                        for &(i, d) in &moved {
                            let Some(&(gm, vi)) = owner.get(i) else { continue };
                            let local = locals.entry(gm).or_insert_with(|| {
                                s.puppet.mesh_local_positions(gm).unwrap_or_default()
                            });
                            let px = local.get(vi * 2).copied().unwrap_or(0.0);
                            let py = local.get(vi * 2 + 1).copied().unwrap_or(0.0);
                            let parent =
                                s.puppet.model().art_meshes.get(gm).and_then(|m| m.parent_deformer);
                            let (dx, dy) =
                                parent_delta(&s.puppet, parent, px, py, d.x * k_unit, d.y * k_unit);
                            per.entry(gm).or_default().push((vi, dx, dy));
                        }
                        let mut plan = Vec::new();
                        let mut between = 0usize;
                        for (gm, moves) in per {
                            let target = ayatsuri_core::edit::Target::ArtMesh(gm);
                            match ayatsuri_core::keyform::keyform_at(
                                s.puppet.model(),
                                target,
                                &values,
                            ) {
                                Some(k) => plan.push((gm, k, moves)),
                                None => between += 1,
                            }
                        }
                        let count = plan.len();
                        if !plan.is_empty()
                            && s.edit(|m| {
                                let mut any = false;
                                for (gm, k, moves) in &plan {
                                    any |= ayatsuri_core::edit::move_vertices_in(
                                        m,
                                        *gm,
                                        Some(*k),
                                        moves,
                                    );
                                }
                                any
                            })
                        {
                            log::debug!("{count} meshes moved together");
                        }
                        if between > 0 {
                            log::warn!(
                                "{between} mesh(es) sit between keys and did not move: put the parameters on a key"
                            );
                        }
                    }
                }
                let stroke = Stroke::new(self.disp.mesh_width, self.disp.mesh_color);
                for (idx, base) in &wires {
                    for t in idx.as_chunks::<3>().0 {
                        let (Some(&a), Some(&b), Some(&c2)) = (
                            pts.get(base + t[0] as usize),
                            pts.get(base + t[1] as usize),
                            pts.get(base + t[2] as usize),
                        ) else {
                            continue;
                        };
                        vp.line_segment([a, b], stroke);
                        vp.line_segment([b, c2], stroke);
                        vp.line_segment([c2, a], stroke);
                    }
                }
                for p in &pts {
                    vp.circle_filled(*p, 1.8, self.disp.mesh_color);
                }
                if editable {
                    point_edit::draw_overlay(&self.pts, &vp, &pts, true, 0.0, grab_hot);
                }
            }
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
                        let mut out = point_edit::point_tool(&mut self.pts, &inp, &mut pts, &opts);
                        if let (Some(ax), Some(m)) = (mirror_x, &mut out.commit) {
                            point_edit::mirror_moves(m, &pts, ax, mirror_tol);
                        }
                        warp_hot |= self.pts.hot;
                        grab_hot = out.hover_grab;
                        hover_pt = out.hover;
                        if ui.ctx().input(|i| i.key_pressed(egui::Key::Escape)) {
                            self.pts.picked.clear();
                        }
                        // live preview: the art follows the pointer, not just
                        // the dots; put back before anything is committed
                        let dragging =
                            self.pts.drag.as_ref().filter(|d| d.grab != point_edit::Grab::Marquee);
                        let was_live = self.live_drag.is_some();
                        live_for(
                            &mut self.live_drag,
                            &mut s.puppet,
                            dragging.map(|_| LiveTarget::Mesh(mi)),
                        );
                        if let (Some(d), Some(l)) = (dragging, &self.live_drag) {
                            let k_unit = 1.0 / (scale * c.pixels_per_unit);
                            let shifts: Vec<(usize, Vec2)> = d
                                .from
                                .iter()
                                .filter_map(|&(i, p0)| Some((i, *pts.get(i)? - p0)))
                                .collect();
                            l.show(&mut s.puppet, &shifts, k_unit);
                        }
                        if was_live || self.live_drag.is_some() {
                            s.dirty = true;
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
                    let mut bez_grid: Option<Vec<f32>> = None;
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
                        let values: Vec<f32> = (0..s.puppet.model().parameters.len())
                            .map(|i| s.puppet.value(i))
                            .collect();
                        let keyform = ayatsuri_core::keyform::keyform_at(
                            s.puppet.model(),
                            ayatsuri_core::edit::Target::Deformer(di),
                            &values,
                        );
                        let kept = keyform
                            .and_then(|k| s.bezier_nets.get(&(di, k)))
                            .filter(|l| l.bw == bw && l.bh == bh)
                            .filter(|l| {
                                let made = l.to_grid(cols as usize, rows as usize);
                                let extent =
                                    local.iter().fold(0.0f32, |a, v| a.max(v.abs())).max(1e-6);
                                made.len() == local.len()
                                    && made
                                        .iter()
                                        .zip(&local)
                                        .all(|(a, b)| (a - b).abs() <= extent * 1e-4)
                            })
                            .cloned();
                        let mut lat = match &self.bez_drag {
                            Some(d) if d.deformer == di => d.lattice.clone(),
                            _ => kept.unwrap_or_else(|| {
                                ayatsuri_core::bezier::Lattice::fit(
                                    &local,
                                    cols as usize,
                                    rows as usize,
                                    bw,
                                    bh,
                                )
                            }),
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
                            bez_grid = Some(new_grid.clone());
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
                                if let Some(k) = keyform {
                                    if s.bezier_nets.len() > 256 {
                                        s.bezier_nets.clear();
                                    }
                                    s.bezier_nets.insert((di, k), lat.clone());
                                    s.modified = true;
                                }
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
                        let mut out =
                            point_edit::point_tool(&mut self.pts, &inp, &mut screen_pts, &opts);
                        if let (Some(ax), Some(m)) = (mirror_x, &mut out.commit) {
                            point_edit::mirror_moves(m, &screen_pts, ax, mirror_tol);
                        }
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
                    // live preview: whatever the warp holds follows the drag;
                    // put back before the real edit below
                    let point_drag = self
                        .pts
                        .drag
                        .as_ref()
                        .filter(|d| d.grab != point_edit::Grab::Marquee)
                        .map(|d| d.from.clone());
                    let live_target = (self.bez_drag.is_some() || point_drag.is_some())
                        .then_some(LiveTarget::Warp(di));
                    if std::env::var_os("AYATSURI_DEV_DEFORMER").is_some() {
                        if !self.dev_def_logged {
                            self.dev_def_logged = true;
                            let ppp = ui.ctx().pixels_per_point();
                            for (gi, sp) in screen_pts.iter().enumerate() {
                                log::info!(
                                    "dev warp point {gi} at {:.0} {:.0}",
                                    sp.x * ppp,
                                    sp.y * ppp
                                );
                            }
                        }
                        if live_target.is_some() {
                            log::debug!(
                                "dev warp live: bez_drag {} point_drag {} bez_grid {} live {}",
                                self.bez_drag.is_some(),
                                point_drag.is_some(),
                                bez_grid.as_ref().map_or(0, Vec::len),
                                self.live_drag.as_ref().map_or("none".to_string(), |l| format!(
                                    "{:?} base {}",
                                    l.target,
                                    l.base.len()
                                ))
                            );
                        }
                    }
                    let was_live = self.live_drag.is_some();
                    live_for(&mut self.live_drag, &mut s.puppet, live_target);
                    if let Some(l) = &self.live_drag {
                        match (&bez_grid, &point_drag) {
                            (Some(grid), _) if self.bez_drag.is_some() => {
                                l.show_local(&mut s.puppet, grid);
                            }
                            (_, Some(from)) => {
                                let k_unit = 1.0 / (scale * c.pixels_per_unit);
                                let shifts: Vec<(usize, Vec2)> = from
                                    .iter()
                                    .filter_map(|&(i, p0)| Some((i, *screen_pts.get(i)? - p0)))
                                    .collect();
                                l.show(&mut s.puppet, &shifts, k_unit);
                            }
                            _ => {}
                        }
                    }
                    if was_live || self.live_drag.is_some() {
                        s.dirty = true;
                    }
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
                        let old_local = s.puppet.warp_local_grid(di).unwrap_or_default();
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
                                    if warp_commit_screen
                                        && let Some(l) = s.bezier_nets.get(&(di, k)).cloned()
                                        && let Some(new_local) = s.puppet.warp_local_grid(di)
                                    {
                                        let extent = old_local
                                            .iter()
                                            .fold(0.0f32, |a, v| a.max(v.abs()))
                                            .max(1e-6);
                                        let tol = extent * 1e-4;
                                        let made = l.to_grid(cols as usize, rows as usize);
                                        let was_theirs = made.len() == old_local.len()
                                            && made
                                                .iter()
                                                .zip(&old_local)
                                                .all(|(a, b)| (a - b).abs() <= tol);
                                        match ayatsuri_core::bezier::affine_between(
                                            &old_local, &new_local, tol,
                                        ) {
                                            Some(a) if was_theirs => {
                                                s.bezier_nets.insert((di, k), l.mapped(a));
                                            }
                                            _ => {
                                                s.bezier_nets.remove(&(di, k));
                                            }
                                        }
                                    }
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
                                wx: 0.0,
                                wy: 0.0,
                                degrees: 0.0,
                                last_ptr: ptr,
                            });
                        }
                        // live turn: what the rotation holds swings with the
                        // handle; the angles go back before the real edit
                        let turning_now =
                            self.rot_drag.as_ref().filter(|d| d.turning && d.deformer == di);
                        match (turning_now, &self.rot_live) {
                            (Some(d), _) => {
                                if self.rot_live.as_ref().is_none_or(|(rd, _)| *rd != di)
                                    && let Some(a) = s.puppet.rotation_angles_mut(di)
                                {
                                    self.rot_live = Some((di, a.to_vec()));
                                }
                                if let Some((_, base)) = &self.rot_live
                                    && let Some(a) = s.puppet.rotation_angles_mut(di)
                                    && a.len() == base.len()
                                {
                                    for (x, b) in a.iter_mut().zip(base) {
                                        *x = b + d.degrees;
                                    }
                                    s.dirty = true;
                                }
                            }
                            (None, Some(_)) => {
                                if let Some((rd, base)) = self.rot_live.take()
                                    && let Some(a) = s.puppet.rotation_angles_mut(rd)
                                    && a.len() == base.len()
                                {
                                    a.copy_from_slice(&base);
                                    s.dirty = true;
                                }
                            }
                            (None, None) => {}
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
                                d.wx += mdx;
                                d.wy += mdy;
                            }
                        }
                        if resp.drag_stopped()
                            && let Some(d) = self.rot_drag.take()
                            && d.deformer == di
                        {
                            // the live angles go back first; the edit below
                            // turns the real model once
                            if let Some((rd, base)) = self.rot_live.take()
                                && let Some(a) = s.puppet.rotation_angles_mut(rd)
                                && a.len() == base.len()
                            {
                                a.copy_from_slice(&base);
                            }
                            if d.turning && d.degrees.abs() > 1e-3 {
                                let (deg, dfm) = (d.degrees, d.deformer);
                                if s.edit(|m| ayatsuri_core::edit::rotate_rotation(m, dfm, deg)) {
                                    log::debug!("rotation {dfm} turned {deg:.2} deg");
                                }
                            } else if !d.turning && (d.wx != 0.0 || d.wy != 0.0) {
                                // the centre is the pivot: it moves, the mesh stays
                                match pivot_move(&s.puppet, d.deformer, d.wx, d.wy) {
                                    Some((deltas, child)) => {
                                        let dfm = d.deformer;
                                        if s.edit(|m| {
                                            ayatsuri_core::edit::move_rotation_pivot(
                                                m, dfm, &deltas, child,
                                            )
                                        }) {
                                            log::debug!(
                                                "rotation {dfm} pivot moved ({:.3}, {:.3})",
                                                d.wx,
                                                d.wy
                                            );
                                        }
                                    }
                                    None => log::warn!(
                                        "the parameters are between keys: move them onto a key to move this pivot"
                                    ),
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

        if self.tool != Tool::DeformBrush
            && let Some(st) = self.dbrush.take()
        {
            for (live, _, _) in &st.parts {
                live.restore(&mut s.puppet);
            }
            s.dirty = true;
        }
        if self.tool == Tool::DeformBrush && !space && self.mesh_edit.is_none() {
            let sizing = size_brush(ui, &resp, &mut self.tool_opts.brush_size, 4.0);
            let r = self.tool_opts.brush_size.clamp(4.0, 400.0);
            let (ptr, any_down) =
                ui.ctx().input(|i| (i.pointer.latest_pos(), i.pointer.any_down()));
            let k_unit = 1.0 / (scale * c.pixels_per_unit);
            let targets: Vec<LiveTarget> = match self.selected {
                Some(Sel::Deformer(d)) if s.puppet.warp_grid(d).is_some() => {
                    vec![LiveTarget::Warp(d)]
                }
                _ => {
                    let mut v = self.multi_sel.clone();
                    if let Some(Sel::Mesh(m)) = self.selected
                        && !v.contains(&m)
                    {
                        v.push(m);
                    }
                    v.into_iter()
                        .filter(|m| !self.mesh_locked.contains(m))
                        .map(LiveTarget::Mesh)
                        .collect()
                }
            };
            let off_key: Vec<LiveTarget> = {
                let values: Vec<f32> =
                    (0..s.puppet.model().parameters.len()).map(|i| s.puppet.value(i)).collect();
                targets
                    .iter()
                    .copied()
                    .filter(|t| {
                        let target = match t {
                            LiveTarget::Mesh(m) => ayatsuri_core::edit::Target::ArtMesh(*m),
                            LiveTarget::Warp(d) => ayatsuri_core::edit::Target::Deformer(*d),
                        };
                        ayatsuri_core::keyform::keyform_at(s.puppet.model(), target, &values)
                            .is_none()
                    })
                    .collect()
            };
            if !off_key.is_empty() {
                let mut snap = false;
                egui::Area::new(egui::Id::new("dbrush-off-key"))
                    .fixed_pos(view.center_top() + vec2(-170.0, 22.0))
                    .order(egui::Order::Foreground)
                    .show(ui.ctx(), |ui| {
                        egui::Frame::popup(ui.style()).show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.label(crate::i18n::tr(
                                    "The parameters are between keys: edits land on a key",
                                ));
                                snap = ui.button(crate::i18n::tr("Snap to nearest keys")).clicked();
                            });
                        });
                    });
                if snap {
                    for t in &off_key {
                        let model = s.puppet.model();
                        let binding = match t {
                            LiveTarget::Mesh(m) => {
                                model.art_meshes.get(*m).map(|a| a.binding.clone())
                            }
                            LiveTarget::Warp(d) => {
                                model.deformers.get(*d).map(|x| x.binding.clone())
                            }
                        };
                        for axis in binding.map(|b| b.axes).unwrap_or_default() {
                            let p = axis.parameter as usize;
                            let v = s.puppet.value(p);
                            if let Some(k) = axis
                                .keys
                                .iter()
                                .copied()
                                .min_by(|a, b| (a - v).abs().total_cmp(&(b - v).abs()))
                            {
                                s.puppet.set_value(p, k);
                            }
                        }
                    }
                    s.dirty = true;
                    log::info!("parameters snapped to the nearest keys");
                }
            }
            if resp.drag_started()
                && !sizing
                && let Some(p) = ptr
            {
                if targets.is_empty() {
                    log::info!("Deform Brush: select meshes or a warp deformer first");
                }
                let mut parts = Vec::new();
                let mut between = 0usize;
                for t in targets {
                    let screen: Vec<Pos2> = match t {
                        LiveTarget::Mesh(m) => s
                            .puppet
                            .mesh_geometry(m)
                            .map(|g| {
                                g.positions
                                    .as_chunks::<2>()
                                    .0
                                    .iter()
                                    .map(|q| to_screen(q[0], q[1]))
                                    .collect()
                            })
                            .unwrap_or_default(),
                        LiveTarget::Warp(d) => s
                            .puppet
                            .warp_grid(d)
                            .map(|(g, _, _)| {
                                g.as_chunks::<2>().0.iter().map(|q| to_screen(q[0], q[1])).collect()
                            })
                            .unwrap_or_default(),
                    };
                    match LiveDrag::start(&mut s.puppet, t) {
                        Some(live) => {
                            let n = screen.len();
                            parts.push((live, screen, vec![Vec2::ZERO; n]));
                        }
                        None => between += 1,
                    }
                }
                if between > 0 {
                    log::warn!(
                        "Deform Brush: {between} object(s) sit between keys; put the parameters on a key to brush them"
                    );
                }
                self.dbrush = Some(DeformStroke { parts, last: p });
            }
            if !sizing
                && resp.dragged()
                && let (Some(st), Some(p)) = (&mut self.dbrush, ptr)
            {
                let delta = p - st.last;
                let from = st.last;
                st.last = p;
                if delta.length_sq() > 0.0 {
                    let (hard, strength) = (self.dbrush_hardness, self.dbrush_strength);
                    for (live, start, shifts) in &mut st.parts {
                        for (j, sh) in shifts.iter_mut().enumerate() {
                            let Some(&p0) = start.get(j) else { continue };
                            let t = (p0 + *sh).distance(from) / r;
                            let w = brush_falloff(t, hard) * strength;
                            if w > 0.0 {
                                *sh += delta * w;
                            }
                        }
                        let moved: Vec<(usize, Vec2)> = shifts
                            .iter()
                            .copied()
                            .enumerate()
                            .filter(|(_, d)| d.length_sq() > 0.0)
                            .collect();
                        live.show(&mut s.puppet, &moved, k_unit);
                    }
                    s.dirty = true;
                }
            }
            if (resp.drag_stopped() || !any_down)
                && let Some(st) = self.dbrush.take()
            {
                for (live, _, _) in &st.parts {
                    live.restore(&mut s.puppet);
                }
                let plan: BrushPlan = st
                    .parts
                    .iter()
                    .map(|(live, _, shifts)| {
                        let moves: Vec<(usize, f32, f32)> = shifts
                            .iter()
                            .enumerate()
                            .filter(|(_, d)| d.length_sq() > 1e-6)
                            .map(|(i, d)| {
                                let px = live.local.get(i * 2).copied().unwrap_or(0.0);
                                let py = live.local.get(i * 2 + 1).copied().unwrap_or(0.0);
                                let (dx, dy) = parent_delta(
                                    &s.puppet,
                                    live.parent,
                                    px,
                                    py,
                                    d.x * k_unit,
                                    d.y * k_unit,
                                );
                                (i, dx, dy)
                            })
                            .collect();
                        (live.target, live.keyform, moves)
                    })
                    .filter(|(_, _, moves)| !moves.is_empty())
                    .collect();
                let points: usize = plan.iter().map(|(_, _, m)| m.len()).sum();
                if !plan.is_empty()
                    && s.edit(|m| {
                        let mut any = false;
                        for (target, k, moves) in &plan {
                            any |= match target {
                                LiveTarget::Mesh(mi) => {
                                    ayatsuri_core::edit::move_vertices_in(m, *mi, Some(*k), moves)
                                }
                                LiveTarget::Warp(d) => {
                                    ayatsuri_core::edit::move_warp_points(m, *d, Some(*k), moves)
                                }
                            };
                        }
                        any
                    })
                {
                    log::debug!("Deform Brush moved {points} point(s)");
                }
                s.dirty = true;
            }
            if let Some(p) = ptr
                && view.contains(p)
            {
                vp.circle_stroke(p, r, Stroke::new(1.2, theme::pal().accent));
                if self.dbrush_hardness > 0.02 {
                    vp.circle_stroke(
                        p,
                        r * self.dbrush_hardness,
                        Stroke::new(1.0, theme::pal().accent.gamma_multiply(0.5)),
                    );
                }
                ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
            }
        }

        // Brush Selection: paint a soft selection onto the selected meshes' or
        // warp's points. Drag adds weight (Alt takes it away), dabs spaced
        // by distance so the speed of the stroke doesn't matter; B + drag
        // sizes the brush; Esc clears. The arrow tool then moves every
        // painted point by its weight.
        if self.tool == Tool::BrushSelect && !space && self.mesh_edit.is_none() {
            let b_down = size_brush(ui, &resp, &mut self.tool_opts.brush_size, 2.0);
            let (ptr, alt, esc) = ui.ctx().input(|i| {
                (i.pointer.latest_pos(), i.modifiers.alt, i.key_pressed(egui::Key::Escape))
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
            // painting over meshes selects them, as Cubism's brush does, so
            // the same stroke picks the objects and weights their points
            let painting = !alt
                && !b_down
                && (resp.drag_started() || resp.clicked() || resp.dragged())
                && self.brush_last.is_none_or(|l| ptr.is_some_and(|p| l.distance(p) >= r * 0.15));
            if painting
                && let Some(p) = ptr
                && !matches!(self.selected, Some(Sel::Deformer(_)))
            {
                let mut picked = Vec::new();
                for mi in 0..s.puppet.model().art_meshes.len() {
                    let hidden = self.mesh_locked.contains(&mi)
                        || !mesh_pickable(s, mi)
                        || targets.iter().any(|(k, _)| *k == (1, mi));
                    if hidden {
                        continue;
                    }
                    let Some(g) = s.puppet.mesh_geometry(mi) else { continue };
                    let (gp, _) = g.positions.as_chunks::<2>();
                    let pts: Vec<Pos2> = gp.iter().map(|q| to_screen(q[0], q[1])).collect();
                    if pts.iter().any(|q| q.distance(p) < r) {
                        picked.push(mi);
                        targets.push(((1, mi), pts));
                    }
                }
                if !picked.is_empty() {
                    if self.multi_sel.is_empty()
                        && let Some(Sel::Mesh(cur)) = self.selected
                    {
                        self.multi_sel.push(cur);
                    }
                    for &mi in &picked {
                        if !self.multi_sel.contains(&mi) {
                            self.multi_sel.push(mi);
                        }
                    }
                    self.selected = self.multi_sel.last().copied().map(Sel::Mesh);
                }
            }
            if b_down {
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

        if self.tool == Tool::ArtPath && !space && self.mesh_edit.is_none() {
            use ayatsuri_format::model::ArtPath;
            let guides = if self.show_guides { s.guides.clone() } else { Vec::new() };
            let snap = |q: [f32; 2]| snap_to_guides(&guides, q, 6.0 / scale);
            let inp = point_edit::PtInput::read(ui, &resp);
            let to_px = |p: Pos2| [(p.x - origin.x) / scale, (p.y - origin.y) / scale];
            let on_screen = |q: [f32; 2]| pos2(origin.x + q[0] * scale, origin.y + q[1] * scale);
            let editing = match self.selected {
                Some(Sel::Mesh(mi))
                    if self.art_draft.is_none()
                        && ayatsuri_core::edit::art_path_of(s.puppet.model(), mi).is_some() =>
                {
                    Some(mi)
                }
                _ => None,
            };
            let mut path: ArtPath = match editing {
                Some(mi) => match &self.art_edit {
                    Some((m, p)) if *m == mi => p.clone(),
                    _ => ayatsuri_core::edit::art_path_of(s.puppet.model(), mi)
                        .cloned()
                        .unwrap_or_default(),
                },
                None => self.art_draft.clone().unwrap_or_else(|| ArtPath {
                    anchors: Vec::new(),
                    mesh: String::new(),
                    ..self.art_style.clone()
                }),
            };
            let probe = if inp.drag_started { inp.origin.or(inp.ptr) } else { inp.ptr };
            let hit = probe.and_then(|p| {
                let mut best: Option<((usize, u8), f32)> = None;
                for (i, a) in path.anchors.iter().enumerate() {
                    let [x, y, ix, iy, ox, oy] = *a;
                    for (k, hx, hy) in [(0u8, 0.0, 0.0), (1, ix, iy), (2, ox, oy)] {
                        if k > 0 && hx == 0.0 && hy == 0.0 {
                            continue;
                        }
                        let d = on_screen([x + hx, y + hy]).distance(p);
                        if d <= 8.0 && best.is_none_or(|(_, bd)| d < bd) {
                            best = Some(((i, k), d));
                        }
                    }
                }
                best.map(|(h, _)| h)
            });
            let mut picking = self.art_pick;
            let mut pick_commit = false;
            if let Some(slot) = picking {
                let sample = inp.ptr.and_then(|p| {
                    let [px, py] = to_px(p);
                    let ux = (px - c.origin_x) / c.pixels_per_unit;
                    let uy = (py - c.origin_y) / c.pixels_per_unit;
                    color_at(&s.puppet, &s.texture_pngs, &mut self.pick_pages, ux, uy)
                });
                if let (Some(p), Some(col)) = (inp.ptr, sample) {
                    let [r, g, b, _] = col;
                    vp.circle(
                        p + vec2(18.0, -18.0),
                        9.0,
                        Color32::from_rgb(r, g, b),
                        Stroke::new(1.5, Color32::WHITE),
                    );
                }
                let cancel = ui.ctx().input(|i| i.key_pressed(egui::Key::Escape));
                if inp.clicked || cancel {
                    if let (false, Some(col)) = (cancel, sample) {
                        match slot {
                            0 => path.stroke_color = col,
                            _ => path.fill_color = col,
                        }
                        self.art_style.stroke_color = path.stroke_color;
                        self.art_style.fill_color = path.fill_color;
                        pick_commit = editing.is_some();
                    }
                    self.art_pick = None;
                    self.pick_pages.clear();
                }
                ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
            }
            let typing = ui.ctx().egui_wants_keyboard_input() || picking.is_some();
            let (enter, esc, back) = ui.ctx().input(|i| {
                (
                    i.key_pressed(egui::Key::Enter),
                    i.key_pressed(egui::Key::Escape),
                    i.key_pressed(egui::Key::Backspace),
                )
            });
            let mut finish = std::mem::take(&mut self.art_finish) && path.anchors.len() >= 2;
            let mut commit = pick_commit;
            if inp.drag_started
                && picking.is_none()
                && let Some(at) = probe
            {
                match hit {
                    Some((i, k)) => {
                        self.art_grab = path.anchors.get(i).map(|a| ArtGrab {
                            anchor: i,
                            part: k,
                            start: to_px(at),
                            orig: *a,
                        });
                    }
                    None if editing.is_none() => {
                        let q = snap(to_px(at));
                        let a = [q[0], q[1], 0.0, 0.0, 0.0, 0.0];
                        path.anchors.push(a);
                        self.art_grab = Some(ArtGrab {
                            anchor: path.anchors.len() - 1,
                            part: 2,
                            start: q,
                            orig: a,
                        });
                    }
                    None => {}
                }
            }
            if let Some(g) = self.art_grab
                && let Some(p) = inp.ptr
            {
                let now = to_px(p);
                let (dx, dy) = (now[0] - g.start[0], now[1] - g.start[1]);
                let [x, y, ix, iy, ox, oy] = g.orig;
                let moved = match g.part {
                    0 => {
                        let q = snap([x + dx, y + dy]);
                        [q[0], q[1], ix, iy, ox, oy]
                    }
                    1 if inp.alt => [x, y, ix + dx, iy + dy, ox, oy],
                    1 => [x, y, ix + dx, iy + dy, -(ix + dx), -(iy + dy)],
                    _ if inp.alt => [x, y, ix, iy, ox + dx, oy + dy],
                    _ => [x, y, -(ox + dx), -(oy + dy), ox + dx, oy + dy],
                };
                if let Some(slot) = path.anchors.get_mut(g.anchor) {
                    *slot = moved;
                }
            }
            if self.art_grab.is_some() && (inp.drag_stopped || !inp.any_down) {
                self.art_grab = None;
                commit = editing.is_some();
            }
            if inp.clicked
                && picking.is_none()
                && let Some(at) = inp.press.or(inp.ptr)
            {
                match hit {
                    Some((i, 0)) if inp.alt => {
                        if i < path.anchors.len() && (editing.is_none() || path.anchors.len() > 2) {
                            path.anchors.remove(i);
                            commit = editing.is_some();
                        }
                    }
                    Some((0, 0)) if editing.is_none() && path.anchors.len() >= 3 => {
                        path.closed = true;
                        finish = true;
                    }
                    Some(_) => {}
                    None if editing.is_none() => {
                        let q = snap(to_px(at));
                        path.anchors.push([q[0], q[1], 0.0, 0.0, 0.0, 0.0]);
                    }
                    None => {}
                }
            }
            if !typing {
                match editing {
                    None => {
                        if enter && path.anchors.len() >= 2 {
                            finish = true;
                        }
                        if esc {
                            path.anchors.clear();
                        }
                        if back {
                            path.anchors.pop();
                        }
                    }
                    Some(_) if esc => self.selected = None,
                    Some(_) => {}
                }
            }
            if finish {
                let part = match self.selected {
                    Some(Sel::Part(p)) => Some(p as u32),
                    Some(Sel::Mesh(mi)) => {
                        s.puppet.model().art_meshes.get(mi).and_then(|m| m.parent_part)
                    }
                    _ => None,
                };
                let draft = path.clone();
                let mut made = None;
                if s.art_path_edit(ui.ctx(), |m, page| {
                    made = ayatsuri_core::edit::create_art_path(m, draft, part, page);
                    made.is_some()
                }) {
                    if let Some(mi) = made {
                        log::info!("ArtPath drawn as mesh {mi}");
                        self.selected = Some(Sel::Mesh(mi));
                    }
                } else {
                    log::warn!(
                        "ArtPath: nothing to draw (it needs a stroke, or fill on a closed path)"
                    );
                }
                path.anchors.clear();
                path.closed = false;
            }
            if commit && let Some(mi) = editing {
                let next = path.clone();
                if s.art_path_edit(ui.ctx(), |m, _| {
                    ayatsuri_core::edit::update_art_path(m, mi, next)
                }) {
                    log::info!("ArtPath {mi} redrawn");
                }
            }
            match editing {
                Some(mi) if self.art_grab.is_some() => self.art_edit = Some((mi, path.clone())),
                Some(_) => self.art_edit = None,
                None if path.anchors.is_empty() => self.art_draft = None,
                None => self.art_draft = Some(path.clone()),
            }

            let samples = ayatsuri_core::artpath::sample(&path.anchors, path.closed, path.step);
            let scr: Vec<Pos2> = samples.iter().map(|q| on_screen(*q)).collect();
            let rgba = |c: [u8; 4]| Color32::from_rgba_unmultiplied(c[0], c[1], c[2], c[3]);
            let live = editing.is_none() || self.art_grab.is_some();
            if live && path.fill && path.closed && scr.len() >= 3 {
                let mut mesh = egui::Mesh::default();
                for p in &scr {
                    mesh.colored_vertex(*p, rgba(path.fill_color));
                }
                for [a, b, c2] in ayatsuri_core::artpath::triangulate(&samples) {
                    mesh.add_triangle(a as u32, b as u32, c2 as u32);
                }
                vp.add(egui::Shape::mesh(mesh));
            }
            let mut outline = scr.clone();
            if path.closed
                && let Some(first) = scr.first()
            {
                outline.push(*first);
            }
            if live && path.stroke && outline.len() >= 2 {
                vp.add(egui::Shape::line(
                    outline.clone(),
                    Stroke::new((path.width * scale).max(1.0), rgba(path.stroke_color)),
                ));
            }
            let green = theme::pal().warp_green;
            if outline.len() >= 2 {
                vp.add(egui::Shape::line(outline, Stroke::new(1.0, green)));
            }
            if editing.is_none()
                && self.art_grab.is_none()
                && let (Some(last), Some(p)) = (scr.last(), inp.ptr)
            {
                vp.line_segment([*last, p], Stroke::new(1.0, green.gamma_multiply(0.5)));
            }
            for (i, a) in path.anchors.iter().enumerate() {
                let [x, y, ix, iy, ox, oy] = *a;
                let at = on_screen([x, y]);
                for (hx, hy) in [(ix, iy), (ox, oy)] {
                    if hx != 0.0 || hy != 0.0 {
                        let h = on_screen([x + hx, y + hy]);
                        vp.line_segment([at, h], Stroke::new(1.0, Color32::from_gray(190)));
                        vp.circle(h, 3.5, Color32::WHITE, Stroke::new(1.0, green));
                    }
                }
                let hot = hit.map(|(j, _)| j) == Some(i);
                let r = if hot { 5.5 } else { 4.0 };
                let fill = if i == 0 && editing.is_none() { theme::pal().accent } else { green };
                let sq = Rect::from_center_size(at, vec2(r * 2.0, r * 2.0));
                vp.rect_filled(sq, 1.0, fill);
                vp.rect_stroke(sq, 1.0, Stroke::new(1.0, Color32::WHITE), StrokeKind::Middle);
            }
            if picking.take().is_none() {
                ui.ctx().set_cursor_icon(if hit.is_some() {
                    egui::CursorIcon::PointingHand
                } else {
                    egui::CursorIcon::Crosshair
                });
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
            /// Mesh Edit's move: the vertex slides over the art (new uv).
            Slide(usize, usize, f32, f32, (f32, f32)),
            Add(usize, usize, [f32; 3]),
            AddFree(usize, [usize; 3], [f32; 3], Option<(u16, u16)>, Vec<u16>),
            Connect(usize, u16, u16),
            Cut(usize, u16, u16),
            EraseMany(usize, Vec<usize>),
            Del(usize, usize),
            Paste(usize, Vec<ayatsuri_core::edit::PastedPoint>, Vec<[usize; 3]>),
        }
        let mut mesh_op: Option<MeshOp> = None;
        if self.mesh_edit.is_none() {
            self.mesh_art = None;
        }
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
                // the art is shown as it was on entering Mesh Edit and stays
                // put; only the nodes and lines move over it (Cubism's way)
                if self.mesh_art.as_ref().is_none_or(|a| a.mesh != mi) {
                    self.mesh_art = Some(MeshArt {
                        mesh: mi,
                        positions: g.positions.to_vec(),
                        uvs: g.uvs.to_vec(),
                        indices: g.indices.to_vec(),
                    });
                }

                // dev: where the vertices are in window points, for scripted drags
                if !self.dev_mesh_logged && std::env::var_os("AYATSURI_DEV_MESHEDIT").is_some() {
                    self.dev_mesh_logged = true;
                    let ppp = ui.ctx().pixels_per_point();
                    for (vi, sp) in screen_pts.iter().enumerate().take(64) {
                        log::info!(
                            "dev mesh edit vertex {vi} at {:.0} {:.0}",
                            sp.x * ppp,
                            sp.y * ppp
                        );
                    }
                }
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
                    let sizing = size_brush(ui, &resp, &mut self.tool_opts.brush_size, 4.0);
                    let r = self.tool_opts.brush_size.clamp(4.0, 400.0);
                    let painting =
                        !sizing && (resp.dragged() || resp.drag_started() || resp.clicked());
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
                // Select points: the picks belong to this mesh only
                if self.mesh_picked_on != Some(mi) {
                    self.mesh_picked.clear();
                    self.mesh_lasso.clear();
                    self.mesh_picked_on = Some(mi);
                }
                self.mesh_picked.retain(|&v| v < screen_pts.len());
                if !space && self.mesh_mode == 6 {
                    let add = ui.ctx().input(|i| i.modifiers.shift);
                    if resp.drag_started()
                        && let Some(p) = resp.interact_pointer_pos()
                    {
                        self.mesh_lasso = vec![p];
                    }
                    if resp.dragged()
                        && let Some(p) = resp.interact_pointer_pos()
                        && self.mesh_lasso.last().is_none_or(|q| q.distance(p) > 2.0)
                    {
                        self.mesh_lasso.push(p);
                    }
                    if resp.drag_stopped() {
                        let lasso = std::mem::take(&mut self.mesh_lasso);
                        if !add {
                            self.mesh_picked.clear();
                        }
                        if lasso.len() >= 3 {
                            for (vi, sp) in screen_pts.iter().enumerate() {
                                if crate::point_edit::point_in_polygon(*sp, &lasso) {
                                    self.mesh_picked.insert(vi);
                                }
                            }
                        }
                    }
                    if resp.clicked() {
                        match near {
                            Some((vi, _)) if add => {
                                if !self.mesh_picked.remove(&vi) {
                                    self.mesh_picked.insert(vi);
                                }
                            }
                            Some((vi, _)) => {
                                self.mesh_picked.clear();
                                self.mesh_picked.insert(vi);
                            }
                            None if !add => self.mesh_picked.clear(),
                            None => {}
                        }
                    }
                    if self.mesh_lasso.len() > 1 {
                        let mut ring = self.mesh_lasso.clone();
                        if let Some(&first) = ring.first() {
                            ring.push(first);
                        }
                        vp.add(egui::Shape::dashed_line(
                            &ring,
                            Stroke::new(1.2, theme::pal().accent),
                            5.0,
                            3.0,
                        ));
                    }
                    if ptr.is_some() && near.is_none() {
                        ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
                    }
                } else {
                    self.mesh_lasso.clear();
                }
                // copy, paste and delete for picked points, in any mode
                let typing = ui.ctx().egui_wants_keyboard_input();
                let (copy_key, paste_key, delete_key) = ui.ctx().input(|i| {
                    let cmd = i.modifiers.command;
                    (
                        i.events.iter().any(|e| matches!(e, egui::Event::Copy))
                            || (cmd && i.key_pressed(egui::Key::C)),
                        i.events.iter().any(|e| matches!(e, egui::Event::Paste(_)))
                            || (cmd && i.key_pressed(egui::Key::V)),
                        i.key_pressed(egui::Key::Delete),
                    )
                });
                let menu = self.point_clip_cmd.take();
                let copy = menu == Some(true) || (!typing && copy_key);
                let paste = menu == Some(false) || (!typing && paste_key);
                let (model_pts, _) = g.positions.as_chunks::<2>();
                let model_pts: Vec<Pos2> = model_pts.iter().map(|p| pos2(p[0], p[1])).collect();
                if copy {
                    if self.mesh_picked.is_empty() {
                        log::warn!("copy points: pick some with Select points first");
                    } else {
                        let picked: Vec<usize> = self.mesh_picked.iter().copied().collect();
                        let slot = |v: u16| picked.iter().position(|&p| p == v as usize);
                        let faces: Vec<[usize; 3]> = g
                            .indices
                            .as_chunks::<3>()
                            .0
                            .iter()
                            .filter_map(|t| Some([slot(t[0])?, slot(t[1])?, slot(t[2])?]))
                            .collect();
                        let points: Vec<Pos2> =
                            picked.iter().filter_map(|&v| model_pts.get(v).copied()).collect();
                        log::info!(
                            "copied {} point(s) and {} face(s) from {}",
                            points.len(),
                            faces.len(),
                            s.mesh_name(mi)
                        );
                        // something on the system clipboard, or egui never
                        // reports the Ctrl+V that pastes them
                        ui.ctx().copy_text(format!("Ayatsuri2D points: {}", points.len()));
                        self.point_clip = Some(PointClip { points, faces, from: s.mesh_name(mi) });
                    }
                }
                if paste && let Some(clip) = &self.point_clip {
                    // a pasted point on an existing one (within 3 screen px)
                    // merges into it, so the meshes line up for glue
                    let merge = 3.0 / (scale * c.pixels_per_unit).max(1e-6);
                    let mut placed = Vec::with_capacity(clip.points.len());
                    for &p in &clip.points {
                        let nearest = model_pts
                            .iter()
                            .enumerate()
                            .map(|(v, q)| (v, q.distance(p)))
                            .min_by(|a, b| a.1.total_cmp(&b.1));
                        let point = match nearest {
                            Some((v, d)) if d <= merge => {
                                Some(ayatsuri_core::edit::PastedPoint::Existing(v))
                            }
                            _ => (!g.indices.is_empty())
                                .then(|| mesh_weights(&model_pts, g.indices, p))
                                .flatten()
                                .or_else(|| basis_weights(&model_pts, p))
                                .map(|(c, w)| ayatsuri_core::edit::PastedPoint::At(c, w)),
                        };
                        match point {
                            Some(pp) => placed.push(pp),
                            None => break,
                        }
                    }
                    if placed.len() == clip.points.len() {
                        mesh_op = Some(MeshOp::Paste(mi, placed, clip.faces.clone()));
                    } else {
                        log::warn!(
                            "paste points: this mesh needs three points not in a line to place them"
                        );
                    }
                }
                if !typing && delete_key && !self.mesh_picked.is_empty() && mesh_op.is_none() {
                    mesh_op =
                        Some(MeshOp::EraseMany(mi, self.mesh_picked.iter().copied().collect()));
                }
                if !space && self.mesh_mode == 0 {
                    if resp.drag_started()
                        && let Some((vi, _)) = near
                        && let Some(local) = s.puppet.mesh_local_positions(mi)
                    {
                        let m_uvs = s
                            .puppet
                            .model()
                            .art_meshes
                            .get(mi)
                            .map(|m| m.uvs.clone())
                            .unwrap_or_default();
                        self.mesh_drag = Some(MeshDrag {
                            vertex: vi,
                            local,
                            dx: 0.0,
                            dy: 0.0,
                            start: screen_pts.clone(),
                            indices: g.indices.to_vec(),
                            model_uvs: m_uvs,
                        });
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
                        // the texture coordinate that was under the drop
                        let uv = ptr
                            .and_then(|p| mesh_weights(&d.start, &d.indices, p))
                            .and_then(|(t, w)| mix_uv(&d.model_uvs, t, w));
                        mesh_op = Some(match uv {
                            Some(uv) => MeshOp::Slide(mi, d.vertex, d.dx, d.dy, uv),
                            None => MeshOp::Move(mi, d.vertex, d.dx, d.dy),
                        });
                    }
                }

                // live preview: the dragged vertex follows the pointer
                if let (Some(d), Some(ptr)) = (&self.mesh_drag, ptr)
                    && let Some(sp) = screen_pts.get_mut(d.vertex)
                {
                    *sp = ptr;
                }

                // the art above the scrim, frozen as it was on entry: moving,
                // adding or erasing nodes never re-cuts or stretches it
                if let Some(art) = &self.mesh_art {
                    let mut mesh = match tex_id {
                        Some(id) => egui::Mesh::with_texture(id),
                        None => egui::Mesh::default(),
                    };
                    let (pos_pairs, _) = art.positions.as_chunks::<2>();
                    let (uv_pairs, _) = art.uvs.as_chunks::<2>();
                    for (i, p) in pos_pairs.iter().enumerate() {
                        let uv = match (tex_id, uv_pairs.get(i)) {
                            (Some(_), Some(&[u, v])) => pos2(u, v),
                            _ => egui::epaint::WHITE_UV,
                        };
                        mesh.vertices.push(egui::epaint::Vertex {
                            pos: to_screen(p[0], p[1]),
                            uv,
                            color: Color32::WHITE,
                        });
                    }
                    mesh.indices.extend(art.indices.iter().map(|&i| i as u32));
                    add_mesh_shape(&vp, mesh);
                }

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
                    } else if self.mesh_picked.contains(&vi) {
                        (4.2, theme::pal().accent)
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
                crate::i18n::trf(
                    "Editing {name}",
                    &[(
                        "name",
                        &truncate(mesh_id.as_deref().unwrap_or(crate::i18n::tr("mesh")), 24),
                    )],
                ),
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
                crate::i18n::tr("Done"),
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
                MeshOp::Slide(mi, vi, dx, dy, uv) => {
                    s.edit(|m| ayatsuri_core::edit::slide_vertex(m, mi, vi, dx, dy, uv))
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
                    self.mesh_picked.clear();
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
                    self.mesh_picked.clear();
                    s.edit(|m| ayatsuri_core::edit::delete_vertex(m, mi, vi))
                }
                MeshOp::Paste(mi, points, faces) => {
                    let mut got = None;
                    let done = s.edit(|m| {
                        got = ayatsuri_core::edit::paste_points(m, mi, &points, &faces);
                        got.is_some()
                    });
                    if let Some(verts) = got {
                        let merged = points
                            .iter()
                            .filter(|p| matches!(p, ayatsuri_core::edit::PastedPoint::Existing(_)))
                            .count();
                        log::info!(
                            "pasted {} point(s) into {} ({merged} merged with points already there)",
                            verts.len(),
                            s.mesh_name(mi)
                        );
                        // the pasted points come out picked, ready to move
                        self.mesh_picked = verts.into_iter().collect();
                        self.mesh_picked_on = Some(mi);
                    }
                    done
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
                        crate::i18n::tr("Glue"),
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
                        if ui.button(crate::i18n::tr("Delete glue")).clicked() {
                            self.selected_glue = Some(gi);
                            ui.close();
                        }
                    });
                }
                // the weight brush: strength builds while dragging, lands on
                // release as one undo step
                if editing && self.tool == Tool::Glue && !space {
                    let sizing = size_brush(ui, &resp, &mut self.tool_opts.brush_size, 4.0);
                    let r = self.tool_opts.brush_size.clamp(4.0, 400.0);
                    if let Some(p) = ptr {
                        vp.circle_stroke(
                            p,
                            r,
                            Stroke::new(1.2, Color32::from_rgb(0xE8, 0xC6, 0x3B)),
                        );
                    }
                    if !sizing
                        && (resp.dragged() || resp.drag_started())
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

        if let Some(cmd) = self.pending_guide.take() {
            let px = |p: Pos2| ((p.x - origin.x) / scale, (p.y - origin.y) / scale);
            let mut lines: Vec<(bool, f32)> = Vec::new();
            match cmd {
                GuideCmd::Mouse(_, p) if !view.contains(p) => {}
                GuideCmd::At(which, p) | GuideCmd::Mouse(which, p) => {
                    let (x, y) = px(p);
                    if which != Some(false) {
                        lines.push((true, x));
                    }
                    if which != Some(true) {
                        lines.push((false, y));
                    }
                }
                GuideCmd::Center => {
                    lines.push((true, cw / 2.0));
                    lines.push((false, ch / 2.0));
                }
                GuideCmd::AtVertex => {
                    let at = |pos: &[f32], v: usize| {
                        let x = pos.get(v * 2).copied()?;
                        let y = pos.get(v * 2 + 1).copied()?;
                        Some((
                            x * c.pixels_per_unit + c.origin_x,
                            y * c.pixels_per_unit + c.origin_y,
                        ))
                    };
                    let first = self.pts.picked.iter().next().copied();
                    let found = match (self.pts.owner, first) {
                        (Some((1, mi)), Some(v)) => {
                            s.puppet.mesh_geometry(mi).and_then(|g| at(g.positions, v))
                        }
                        (Some((2, di)), Some(v)) => {
                            s.puppet.warp_grid(di).and_then(|(g, _, _)| at(g, v))
                        }
                        _ => None,
                    };
                    match found {
                        Some((x, y)) => {
                            lines.push((true, x));
                            lines.push((false, y));
                        }
                        None => log::info!("pick a vertex first (click a point of a mesh or warp)"),
                    }
                }
            }
            for (vertical, at) in lines {
                s.guides.push(ayatsuri_format::GuideLine::new(vertical, at.round()));
                s.modified = true;
            }
            self.show_guides = true;
        }
        if self.show_guides {
            const RULER: f32 = 14.0;
            if let Some(ax) = mirror_x
                && view.min.x <= ax
                && ax <= view.max.x
            {
                // the mirror axis: dashed, in the accent, with its name at the top
                let col = theme::pal().accent;
                let mut y = view.min.y;
                while y < view.max.y {
                    vp.line_segment(
                        [pos2(ax, y), pos2(ax, (y + 6.0).min(view.max.y))],
                        Stroke::new(1.0, col),
                    );
                    y += 10.0;
                }
                vp.text(
                    pos2(ax + 6.0, view.min.y + 6.0),
                    Align2::LEFT_TOP,
                    crate::i18n::tr("Mirror"),
                    FontId::proportional(11.0),
                    col,
                );
            }
            let rulers = !self.prefs.rulers_off;
            let top = Rect::from_min_max(view.min, pos2(view.max.x, view.min.y + RULER));
            let left = Rect::from_min_max(
                pos2(view.min.x, view.min.y + RULER),
                pos2(view.min.x + RULER, view.max.y),
            );
            let canvas_x = |x: f32| (x - origin.x) / scale;
            let canvas_y = |y: f32| (y - origin.y) / scale;
            let screen_x = |x: f32| origin.x + x * scale;
            let screen_y = |y: f32| origin.y + y * scale;
            let snapping = self.pts.drag.is_some();
            for (gi, g) in s.guides.iter().enumerate() {
                let line = if g.vertical {
                    [pos2(screen_x(g.at), view.min.y), pos2(screen_x(g.at), view.max.y)]
                } else {
                    [pos2(view.min.x, screen_y(g.at)), pos2(view.max.x, screen_y(g.at))]
                };
                let stroke = if snapping && self.pts.snapped.contains(&gi) {
                    Stroke::new(2.0, Color32::from_rgb(40, 220, 90))
                } else {
                    let [r, gg, b, a] = g.color;
                    Stroke::new(1.0, Color32::from_rgba_unmultiplied(r, gg, b, a))
                };
                vp.line_segment(line, stroke);
            }
            if rulers {
                let chrome = theme::pal().chrome;
                vp.rect_filled(top, 0.0, chrome);
                vp.rect_filled(left, 0.0, chrome);
                let step = [10.0, 20.0, 50.0, 100.0, 200.0, 500.0, 1000.0, 2000.0, 5000.0]
                    .into_iter()
                    .find(|st| st * scale >= 60.0)
                    .unwrap_or(10000.0);
                let faint = theme::pal().text_faint;
                let mut x = (canvas_x(top.min.x) / step).floor() * step;
                while screen_x(x) <= top.max.x {
                    let sx = screen_x(x);
                    if sx >= left.max.x {
                        vp.line_segment(
                            [pos2(sx, top.max.y - 5.0), pos2(sx, top.max.y)],
                            Stroke::new(1.0, faint),
                        );
                        vp.text(
                            pos2(sx + 2.0, top.min.y + 1.0),
                            Align2::LEFT_TOP,
                            format!("{x:.0}"),
                            FontId::proportional(8.5),
                            faint,
                        );
                    }
                    x += step;
                }
                let mut y = (canvas_y(left.min.y) / step).floor() * step;
                while screen_y(y) <= left.max.y {
                    let sy = screen_y(y);
                    if sy >= left.min.y {
                        vp.line_segment(
                            [pos2(left.max.x - 5.0, sy), pos2(left.max.x, sy)],
                            Stroke::new(1.0, faint),
                        );
                    }
                    y += step;
                }
                for g in &s.guides {
                    let tip = if g.vertical {
                        pos2(screen_x(g.at), top.max.y)
                    } else {
                        pos2(left.max.x, screen_y(g.at))
                    };
                    let pts = if g.vertical {
                        vec![tip, tip + vec2(-5.0, -8.0), tip + vec2(5.0, -8.0)]
                    } else {
                        vec![tip, tip + vec2(-8.0, -5.0), tip + vec2(-8.0, 5.0)]
                    };
                    let [r, gg, b, a] = g.color;
                    vp.add(egui::Shape::convex_polygon(
                        pts,
                        Color32::from_rgba_unmultiplied(r, gg, b, a),
                        Stroke::NONE,
                    ));
                }
                let top_r =
                    ui.interact(top, egui::Id::new("guide-ruler-top"), Sense::click_and_drag());
                let left_r =
                    ui.interact(left, egui::Id::new("guide-ruler-left"), Sense::click_and_drag());
                let marker_at = |guides: &[ayatsuri_format::GuideLine],
                                 p: Pos2,
                                 on_top: bool|
                 -> Option<usize> {
                    guides.iter().position(|g| {
                        if on_top {
                            g.vertical && (screen_x(g.at) - p.x).abs() <= 5.0
                        } else {
                            !g.vertical && (screen_y(g.at) - p.y).abs() <= 5.0
                        }
                    })
                };
                for (r, on_top) in [(&top_r, true), (&left_r, false)] {
                    let at = r.interact_pointer_pos().or(r.hover_pos());
                    if r.drag_started()
                        && let Some(p) = at
                    {
                        self.guide_drag = match marker_at(&s.guides, p, on_top) {
                            Some(i) => Some(i),
                            None => {
                                s.guides.push(ayatsuri_format::GuideLine::new(
                                    !on_top,
                                    if on_top { canvas_y(p.y) } else { canvas_x(p.x) },
                                ));
                                Some(s.guides.len() - 1)
                            }
                        };
                    }
                    if r.secondary_clicked()
                        && let Some(p) = at
                        && let Some(i) = marker_at(&s.guides, p, on_top)
                    {
                        s.guides.remove(i);
                        s.modified = true;
                    }
                    if r.hovered() && at.is_some_and(|p| marker_at(&s.guides, p, on_top).is_some())
                    {
                        ui.ctx().set_cursor_icon(if on_top {
                            egui::CursorIcon::ResizeHorizontal
                        } else {
                            egui::CursorIcon::ResizeVertical
                        });
                    }
                }
            }
            if let Some(i) = self.guide_drag {
                let (ptr, down) =
                    ui.ctx().input(|i| (i.pointer.latest_pos(), i.pointer.any_down()));
                if let (Some(p), Some(g)) = (ptr, s.guides.get_mut(i)) {
                    g.at = if g.vertical { canvas_x(p.x) } else { canvas_y(p.y) }.round();
                    s.modified = true;
                }
                if !down {
                    if ptr.is_none_or(|p| !view.contains(p) || top.contains(p) || left.contains(p))
                        && i < s.guides.len()
                    {
                        s.guides.remove(i);
                    }
                    self.guide_drag = None;
                }
            }
        }
        if self.show_guide_settings {
            let mut open = true;
            egui::Window::new(crate::i18n::tr("Guide Line Settings"))
                .collapsible(false)
                .open(&mut open)
                .default_width(460.0)
                .show(ui.ctx(), |ui| {
                    let mut changed = false;
                    let mut remove = None;
                    ui.horizontal(|ui| {
                        if ui.button(crate::i18n::tr("New Guide Line")).clicked() {
                            s.guides
                                .push(ayatsuri_format::GuideLine::new(false, (ch / 2.0).round()));
                            changed = true;
                        }
                        ui.separator();
                        ui.label(crate::i18n::tr("Change color of all guide lines"));
                        let mut all = s
                            .guides
                            .first()
                            .map_or(ayatsuri_format::GuideLine::DEFAULT_COLOR, |g| g.color);
                        if ui.color_edit_button_srgba_unmultiplied(&mut all).changed() {
                            for g in &mut s.guides {
                                g.color = all;
                            }
                            changed = true;
                        }
                    });
                    ui.add_space(4.0);
                    egui::ScrollArea::vertical().max_height(320.0).show(ui, |ui| {
                        egui::Grid::new("guide-lines").striped(true).show(ui, |ui| {
                            for h in ["No.", "Type", "Position", "Color", "Memo", ""] {
                                ui.weak(crate::i18n::tr(h));
                            }
                            ui.end_row();
                            for (i, g) in s.guides.iter_mut().enumerate() {
                                ui.label((i + 1).to_string());
                                egui::ComboBox::from_id_salt(("guide-type", i))
                                    .selected_text(if g.vertical {
                                        crate::i18n::tr("Vertical")
                                    } else {
                                        crate::i18n::tr("Horizontal")
                                    })
                                    .show_ui(ui, |ui| {
                                        changed |= ui
                                            .selectable_value(
                                                &mut g.vertical,
                                                false,
                                                crate::i18n::tr("Horizontal"),
                                            )
                                            .changed();
                                        changed |= ui
                                            .selectable_value(
                                                &mut g.vertical,
                                                true,
                                                crate::i18n::tr("Vertical"),
                                            )
                                            .changed();
                                    });
                                changed |= ui
                                    .add(egui::DragValue::new(&mut g.at).speed(1.0).suffix(" px"))
                                    .changed();
                                changed |=
                                    ui.color_edit_button_srgba_unmultiplied(&mut g.color).changed();
                                changed |= ui
                                    .add(
                                        egui::TextEdit::singleline(&mut g.memo)
                                            .desired_width(140.0),
                                    )
                                    .changed();
                                if ui
                                    .small_button("×")
                                    .on_hover_text(crate::i18n::tr("Delete"))
                                    .clicked()
                                {
                                    remove = Some(i);
                                }
                                ui.end_row();
                            }
                        });
                    });
                    if let Some(i) = remove {
                        s.guides.remove(i);
                        changed = true;
                    }
                    if changed {
                        s.modified = true;
                    }
                });
            self.show_guide_settings = open;
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
            if ui.small_button(crate::i18n::tr("Fit")).clicked() {
                self.view_zoom = 1.0;
                self.view_pan = Vec2::ZERO;
            }
        });
        // a window over the canvas (Settings, say) keeps its own cursor:
        // tool cursors are only for the canvas itself
        if !resp.hovered() && !resp.dragged() {
            ui.ctx().set_cursor_icon(cursor_before);
        }
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

/// What a pivot drag changes: each keyform's origin shift (parent space),
/// and the one offset every child takes (the deformer's local space).
type PivotMove = (Vec<(f32, f32)>, (f32, f32));

/// A rotation deformer pivot drag of (drag_x, drag_y) canvas units, worked
/// out at the current pose: the offset every child takes so nothing moves
/// now, and each keyform's origin shift that keeps the pivot on the same
/// spot of the mesh in that keyform. A keyform is one stored pose of the
/// deformer (its angle, origin and scale at one parameter value). None when
/// the parameters sit between keyforms, where there is no single one to
/// anchor to.
fn pivot_move(
    puppet: &ayatsuri_core::Puppet,
    deformer: usize,
    drag_x: f32,
    drag_y: f32,
) -> Option<PivotMove> {
    use ayatsuri_format::model::DeformerKind;
    let model = puppet.model();
    let def = model.deformers.get(deformer)?;
    let DeformerKind::Rotation(rot) = &def.kind else { return None };
    let values: Vec<f32> = (0..model.parameters.len()).map(|i| puppet.value(i)).collect();
    // the keyform the parameters sit on right now
    let current = ayatsuri_core::keyform::keyform_at(
        model,
        ayatsuri_core::edit::Target::Deformer(deformer),
        &values,
    )?;
    let state = puppet.rotation_state(deformer)?;
    // children satisfy world = origin + M * child, M = scale * turn(angle) *
    // flip; the child offset that cancels the drag is -(M^-1 * drag)
    let (sin, cos) = state.angle.to_radians().sin_cos();
    let flip_x = if state.reflect_x { -1.0 } else { 1.0 };
    let flip_y = if state.reflect_y { -1.0 } else { 1.0 };
    let scale = if state.scale.abs() > 1e-6 { state.scale } else { 1.0 };
    let child = (
        -(flip_x * (cos * drag_x + sin * drag_y)) / scale,
        -(flip_y * (-sin * drag_x + cos * drag_y)) / scale,
    );
    // every other keyform's origin moves by the drag, turned and scaled by
    // how that keyform differs from the current one
    let current_angle = rot.angles.get(current).copied()?;
    let current_scale = rot.scales.get(current).copied().filter(|v| v.abs() > 1e-6).unwrap_or(1.0);
    let parent = def.parent_deformer;
    let mut deltas = Vec::with_capacity(rot.angles.len());
    for keyform in 0..rot.angles.len() {
        let angle = rot.angles.get(keyform).copied().unwrap_or(current_angle);
        let turn = (angle - current_angle).to_radians();
        let scale_ratio = rot.scales.get(keyform).copied().unwrap_or(1.0) / current_scale;
        let (turn_sin, turn_cos) = turn.sin_cos();
        let shift_x = scale_ratio * (turn_cos * drag_x - turn_sin * drag_y);
        let shift_y = scale_ratio * (turn_sin * drag_x + turn_cos * drag_y);
        let origin_x = rot.origins_x.get(keyform).copied().unwrap_or(0.0);
        let origin_y = rot.origins_y.get(keyform).copied().unwrap_or(0.0);
        deltas.push(parent_delta(puppet, parent, origin_x, origin_y, shift_x, shift_y));
    }
    Some((deltas, child))
}

/// A move of (mdx, mdy) canvas units, as a move in `parent`'s own space for
/// a point at (px, py) there. Exact, not a straight-line guess: warps bend
/// space, so one linear step lands a dragged point short of (or past) the
/// pointer and it visibly jumps on release. Newton steps on the parent's map
/// until the point lands where the pointer let go.
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
    let map = |x: f32, y: f32| puppet.deformer_point(pd, x, y);
    let (sx, sy) = map(px, py);
    let (tx, ty) = (sx + mdx, sy + mdy);
    let (mut x, mut y) = (px, py);
    for _ in 0..16 {
        let (fx, fy) = map(x, y);
        let (ex, ey) = (tx - fx, ty - fy);
        if ex.abs() + ey.abs() < 1e-6 {
            break;
        }
        let e = 1e-3;
        let (ax, ay) = map(x + e, y);
        let (bx, by) = map(x, y + e);
        let (j00, j10, j01, j11) = ((ax - fx) / e, (ay - fy) / e, (bx - fx) / e, (by - fy) / e);
        let det = j00 * j11 - j01 * j10;
        if det.abs() < 1e-9 || !det.is_finite() {
            break;
        }
        x += (j11 * ex - j01 * ey) / det;
        y += (j00 * ey - j10 * ex) / det;
    }
    (x - px, y - py)
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
        ("View", "focus", "Focus on the selected object", N, K::F),
        ("View", "mirror-edit", "Mirror edit", CS, K::M),
        ("View", "zoom-in", "Zoom in", C, K::Equals),
        ("View", "zoom-out", "Zoom out", C, K::Minus),
        ("View", "guide-h-mouse", "Horizontal guide line at the mouse", N, K::H),
        ("View", "guide-v-mouse", "Vertical guide line at the mouse", N, K::V),
        ("View", "guide-both-mouse", "Vertical and horizontal guide lines at the mouse", N, K::G),
        ("View", "guides-clear", "Clear guide lines", M::SHIFT, K::G),
        ("Edit", "select-all", "Select all", C, K::A),
        ("Edit", "search", "Search", C, K::F),
        ("Edit", "copy-form", "Copy form", CS, K::C),
        ("Edit", "paste-form", "Paste form", CS, K::V),
        ("Modeling", "select-children", "Select deformer child elements", CA, K::D),
        ("Modeling", "delete-deformer-content", "Delete deformer and its content", C, K::Delete),
        ("Modeling", "lock-unselected", "Lock unselected", CS, K::Num2),
        ("Modeling", "unlock-all", "Unlock all", CA, K::Num2),
        ("Animation", "anim-first", "Move to the beginning", N, K::Home),
        ("Animation", "anim-last", "Move to the end", N, K::End),
        ("Animation", "anim-prev-frame", "Previous frame", N, K::PageUp),
        ("Animation", "anim-next-frame", "Next frame", N, K::PageDown),
        ("Animation", "anim-prev-key", "Previous keyframe", N, K::J),
        ("Animation", "anim-next-key", "Next keyframe", N, K::K),
        ("Animation", "anim-play", "Playback", N, K::Enter),
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
/// Properties' Opacity slider for a mesh or deformer, on the keyform the
/// parameters sit on (as in Cubism: move the parameters onto a key, then
/// set it). Dragging previews live in place; letting go puts the stored
/// number back and commits one edit, so undo sees a single change.
fn opacity_row(
    ui: &mut Ui,
    s: &mut Session,
    targets: &[ayatsuri_core::edit::Target],
    across: Option<usize>,
) {
    let values: Vec<f32> =
        (0..s.puppet.model().parameters.len()).map(|p| s.puppet.value(p)).collect();
    // which keyforms an edit sets, per object: with a parameter selected that
    // the object is keyed on, every keyform at that parameter's current key
    // (a toggle reads the same at every head angle and physics swing);
    // otherwise the one keyform the parameters sit on
    let mut spread = false;
    let sets: Vec<Option<Vec<usize>>> = targets
        .iter()
        .map(|&t| {
            let model = s.puppet.model();
            if let Some(p) = across
                && let Some(all) = ayatsuri_core::keyform::keyforms_on_key(model, t, p, &values)
            {
                spread |= all.len() > 1;
                return Some(all);
            }
            ayatsuri_core::keyform::keyform_at(model, t, &values).map(|k| vec![k])
        })
        .collect();
    let first = targets
        .first()
        .copied()
        .zip(sets.first().cloned().flatten().and_then(|v| v.first().copied()));
    let stored = first.and_then(|(t, k)| s.puppet.opacity_mut(t, k).map(|o| *o));
    let all_on_keys = !targets.is_empty() && sets.iter().all(Option::is_some);
    ui.horizontal(|ui| {
        let label = if targets.len() > 1 {
            crate::i18n::trf("Opacity ({count})", &[("count", &targets.len())])
        } else {
            crate::i18n::tr("Opacity").into()
        };
        ui.add_sized(
            [74.0, 16.0],
            egui::Label::new(egui::RichText::new(label).color(theme::pal().text_faint).size(11.0)),
        );
        let (true, Some(now)) = (all_on_keys, stored) else {
            ui.add_enabled(false, egui::Slider::new(&mut 100.0f32, 0.0..=100.0).suffix("%"))
                .on_disabled_hover_text(crate::i18n::tr(
                    "Between keys. Move the parameters onto a key to set opacity there \
                     (every selected mesh must be on one).",
                ));
            return;
        };
        let pairs: Vec<(ayatsuri_core::edit::Target, usize)> = targets
            .iter()
            .copied()
            .zip(sets.iter())
            .flat_map(|(t, ks)| ks.iter().flatten().map(move |&k| (t, k)))
            .collect();
        let live_id = egui::Id::new(("opacity-live", pairs.clone()));
        let mut percent = now * 100.0;
        let resp =
            ui.add(egui::Slider::new(&mut percent, 0.0..=100.0).suffix("%").fixed_decimals(0));
        if resp.drag_started() {
            let before: Vec<f32> = pairs
                .iter()
                .map(|&(t, k)| s.puppet.opacity_mut(t, k).map_or(1.0, |o| *o))
                .collect();
            ui.ctx().data_mut(|d| d.insert_temp(live_id, before));
        }
        let original: Option<Vec<f32>> = ui.ctx().data(|d| d.get_temp(live_id));
        let set_all = |m: &mut ayatsuri_format::Model, value: f32| {
            let mut any = false;
            for &(t, k) in &pairs {
                any |= ayatsuri_core::edit::set_opacity(m, t, k, value);
            }
            any
        };
        if resp.changed() {
            let value = (percent / 100.0).clamp(0.0, 1.0);
            if original.is_some() {
                // live: the numbers change in place while dragging
                for &(t, k) in &pairs {
                    if let Some(slot) = s.puppet.opacity_mut(t, k) {
                        *slot = value;
                    }
                }
                s.dirty = true;
            } else if s.edit(|m| set_all(m, value)) {
                log::debug!("opacity of {} object(s) set to {value:.2}", pairs.len());
            }
        }
        if resp.drag_stopped()
            && let Some(before) = original
        {
            ui.ctx().data_mut(|d| d.remove::<Vec<f32>>(live_id));
            let value = (percent / 100.0).clamp(0.0, 1.0);
            // put the stored numbers back, then one real edit
            for (&(t, k), b) in pairs.iter().zip(&before) {
                if let Some(slot) = s.puppet.opacity_mut(t, k) {
                    *slot = *b;
                }
            }
            if s.edit(|m| set_all(m, value)) {
                log::info!("opacity of {} object(s) set to {:.0}%", pairs.len(), value * 100.0);
            }
            s.dirty = true;
        }
    });
    if spread && let Some(p) = across {
        let name = s.param_names.get(p).cloned().unwrap_or_default();
        let at = s.puppet.value(p);
        ui.weak(crate::i18n::trf(
            "Sets {name} = {value} at every other key (head angles, physics…)",
            &[("name", &name), ("value", &format!("{at:.2}"))],
        ));
    }
}

/// A palette search box, with a clear button once something is typed.
fn search_box(ui: &mut Ui, text: &mut String, hint: &str) -> Option<egui::Response> {
    let mut field = None;
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new(egui_phosphor::regular::MAGNIFYING_GLASS)
                .color(theme::pal().text_faint),
        );
        let w = (ui.available_width() - 24.0).max(40.0);
        field = Some(ui.add(egui::TextEdit::singleline(text).hint_text(hint).desired_width(w)));
        if !text.is_empty()
            && ui
                .small_button(egui_phosphor::regular::X)
                .on_hover_text(crate::i18n::tr("Clear the search"))
                .clicked()
        {
            text.clear();
        }
    });
    field
}

/// What a palette search keeps: the rows that match, everything above them
/// (so they show in place), and everything inside a row that matches.
/// `parent[i]` is each node's parent node; `hits[i]` whether node i matches;
/// `leaf_parent[m]` and `leaf_hit[m]` the same for the meshes hanging off
/// the nodes. Returns (node shown, mesh shown).
fn search_keep(
    parent: &[Option<u32>],
    hits: &[bool],
    leaf_parent: &[Option<u32>],
    leaf_hit: &[bool],
) -> (Vec<bool>, Vec<bool>) {
    let n = parent.len();
    let up = |start: Option<u32>, keep: &mut Vec<bool>| {
        let mut at = start;
        let mut hops = 0;
        while let Some(p) = at {
            let Some(k) = keep.get_mut(p as usize) else { break };
            *k = true;
            at = parent.get(p as usize).copied().flatten();
            hops += 1;
            if hops > n {
                break;
            }
        }
    };
    // a match keeps itself and its ancestors
    let mut keep = vec![false; n];
    for i in 0..n {
        if hits.get(i).copied().unwrap_or(false) {
            up(Some(i as u32), &mut keep);
        }
    }
    for (m, hit) in leaf_hit.iter().enumerate() {
        if *hit {
            up(leaf_parent.get(m).copied().flatten(), &mut keep);
        }
    }
    // a match also shows everything inside it
    let inside = |i: usize| {
        let mut at = Some(i as u32);
        let mut hops = 0;
        while let Some(p) = at {
            if hits.get(p as usize).copied().unwrap_or(false) {
                return true;
            }
            at = parent.get(p as usize).copied().flatten();
            hops += 1;
            if hops > n {
                break;
            }
        }
        false
    };
    let nodes: Vec<bool> =
        (0..n).map(|i| keep.get(i).copied().unwrap_or(false) || inside(i)).collect();
    let leaves: Vec<bool> = leaf_hit
        .iter()
        .enumerate()
        .map(|(m, hit)| {
            *hit || leaf_parent.get(m).copied().flatten().is_some_and(|p| inside(p as usize))
        })
        .collect();
    (nodes, leaves)
}

fn matches_search(query: &str, names: &[&str]) -> bool {
    names.iter().any(|n| n.to_lowercase().contains(query))
}

fn parts_mesh_row(
    ui: &mut Ui,
    s: &mut Session,
    selected: &mut Option<Sel>,
    mi: usize,
    mid: &str,
    x_off: f32,
    parent_effective: bool,
    stack: &std::collections::HashMap<usize, usize>,
    cmd: &mut Option<MeshRowCmd>,
    picked: &[usize],
    hover: &mut Option<Sel>,
) {
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 16.0), Sense::click());
    if *selected == Some(Sel::Mesh(mi)) || picked.contains(&mi) {
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
        *cmd = Some(MeshRowCmd::Click(mi));
    }
    // hovering shows the mesh's own art big, as part rows do; rendered on
    // first hover and kept until the model changes
    if resp.hovered() {
        *hover = Some(Sel::Mesh(mi));
        if !s.mesh_previews.contains_key(&mi) {
            let ctx = ui.ctx().clone();
            let handle = crate::thumb::render_filtered(&s.puppet, &s.preview_tex, &|m| m == mi, 96)
                .map(|(w, h, px)| {
                    let img = egui::ColorImage::from_rgba_unmultiplied([w, h], &px);
                    ctx.load_texture(format!("mesh-prev-{mi}"), img, egui::TextureOptions::LINEAR)
                });
            s.mesh_previews.insert(mi, handle);
        }
        if let Some(h) = s.mesh_previews.get(&mi).cloned().flatten() {
            resp.clone().on_hover_ui(|ui| {
                let sz = h.size_vec2();
                let k = (160.0 / sz.x).min(200.0 / sz.y);
                ui.image((h.id(), sz * k));
            });
        }
    }
    resp.context_menu(|ui| {
        if ui
            .button(if mesh_on { crate::i18n::tr("Hide") } else { crate::i18n::tr("Show") })
            .clicked()
        {
            if let Some(f) = s.mesh_shown.get_mut(mi) {
                *f = !mesh_on;
            }
            s.puppet.set_mesh_visible(mi, !mesh_on);
            s.dirty = true;
            ui.close();
        }
        if ui.button(crate::i18n::tr("Select")).clicked() {
            *selected = Some(Sel::Mesh(mi));
            ui.close();
        }
        if ui
            .button(crate::i18n::tr("Clip by…"))
            .on_hover_text(crate::i18n::tr("Pick this mesh's clipping masks on the canvas"))
            .clicked()
        {
            *cmd = Some(MeshRowCmd::ClipBy(mi));
            ui.close();
        }
        ui.separator();
        if ui.button(crate::i18n::tr("Delete Art Mesh")).clicked() {
            *cmd = Some(MeshRowCmd::Delete(mi));
            ui.close();
        }
    });
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
/// Every brush sizes the same way: hold B and drag (right or up grows it),
/// or press [ and ]. True while B is held, so that drag sizes the brush
/// instead of painting with it.
fn size_brush(ui: &Ui, resp: &egui::Response, size: &mut f32, min: f32) -> bool {
    if ui.ctx().egui_wants_keyboard_input() {
        return false;
    }
    let (b, delta, shrink, grow) = ui.ctx().input(|i| {
        (
            i.key_down(egui::Key::B),
            i.pointer.delta(),
            i.key_pressed(egui::Key::OpenBracket),
            i.key_pressed(egui::Key::CloseBracket),
        )
    });
    if b && (resp.dragged() || resp.drag_started()) {
        *size = (*size + delta.x - delta.y).clamp(min, 400.0);
    }
    if shrink {
        *size = (*size / 1.25).max(min);
    }
    if grow {
        *size = (*size * 1.25).min(400.0);
    }
    b
}

/// The ArtPath under a canvas point (pixels): inside a closed outline, or
/// within `slack` of an open one's stroke. Its mesh index.
fn art_path_at(model: &Model, px: f32, py: f32, slack: f32) -> Option<usize> {
    for path in model.art_paths.iter().rev() {
        let Some(mi) = model.art_meshes.iter().position(|m| m.id == path.mesh) else { continue };
        let pts = ayatsuri_core::artpath::sample(&path.anchors, path.closed, path.step);
        let n = pts.len();
        if path.closed && n >= 3 {
            let mut inside = false;
            for i in 0..n {
                let (Some(a), Some(b)) = (pts.get(i), pts.get((i + 1) % n)) else { continue };
                if (a[1] > py) != (b[1] > py)
                    && px < a[0] + (py - a[1]) / (b[1] - a[1]) * (b[0] - a[0])
                {
                    inside = !inside;
                }
            }
            if inside {
                return Some(mi);
            }
        }
        let reach = path.width * 0.5 + slack;
        let near = pts.windows(2).any(|w| {
            let [a, b] = w else { return false };
            let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
            let len2 = (dx * dx + dy * dy).max(1e-9);
            let t = (((px - a[0]) * dx + (py - a[1]) * dy) / len2).clamp(0.0, 1.0);
            let (qx, qy) = (a[0] + t * dx, a[1] + t * dy);
            (px - qx).powi(2) + (py - qy).powi(2) <= reach * reach
        });
        if near {
            return Some(mi);
        }
    }
    None
}

/// The colour the model shows at a point (model units): the top mesh there
/// whose art is solid enough, read from its full-size texture page. A mesh
/// clipped away at that point (outside its mask) doesn't count.
fn color_at(
    puppet: &ayatsuri_core::Puppet,
    pngs: &[Vec<u8>],
    cache: &mut std::collections::HashMap<usize, (usize, u32, u32, Vec<u8>)>,
    ux: f32,
    uy: f32,
) -> Option<[u8; 4]> {
    let mut texel = |mi: usize| -> Option<([u8; 4], f32)> {
        let g = puppet.mesh_geometry(mi)?;
        let (pos, _) = g.positions.as_chunks::<2>();
        let (uv, _) = g.uvs.as_chunks::<2>();
        let (tris, _) = g.indices.as_chunks::<3>();
        let [a, b, c] = *tris.iter().find(|&&[a, b, c]| {
            match (pos.get(a as usize), pos.get(b as usize), pos.get(c as usize)) {
                (Some(p0), Some(p1), Some(p2)) => point_in_triangle(ux, uy, p0, p1, p2),
                _ => false,
            }
        })?;
        let (p0, p1, p2) = (pos.get(a as usize)?, pos.get(b as usize)?, pos.get(c as usize)?);
        let det = (p1[1] - p2[1]) * (p0[0] - p2[0]) + (p2[0] - p1[0]) * (p0[1] - p2[1]);
        if det.abs() < 1e-12 {
            return None;
        }
        let w0 = ((p1[1] - p2[1]) * (ux - p2[0]) + (p2[0] - p1[0]) * (uy - p2[1])) / det;
        let w1 = ((p2[1] - p0[1]) * (ux - p2[0]) + (p0[0] - p2[0]) * (uy - p2[1])) / det;
        let w2 = 1.0 - w0 - w1;
        let (t0, t1, t2) = (uv.get(a as usize)?, uv.get(b as usize)?, uv.get(c as usize)?);
        let u = w0 * t0[0] + w1 * t1[0] + w2 * t2[0];
        let v = w0 * t0[1] + w1 * t1[1] + w2 * t2[1];
        let page = g.texture as usize;
        let png = pngs.get(page)?;
        let fresh = cache.get(&page).is_some_and(|(len, ..)| *len == png.len());
        if !fresh {
            if cache.len() >= 2 {
                cache.clear();
            }
            let img = image::load_from_memory(png).ok()?.into_rgba8();
            cache.insert(page, (png.len(), img.width(), img.height(), img.into_raw()));
        }
        let (_, w, h, px) = cache.get(&page)?;
        let x = ((u.clamp(0.0, 1.0) * *w as f32) as u32).min(w.saturating_sub(1));
        let y = ((v.clamp(0.0, 1.0) * *h as f32) as u32).min(h.saturating_sub(1));
        let at = ((y * w + x) * 4) as usize;
        let &[r, g, b, a] = px.get(at..at + 4)? else { return None };
        Some(([r, g, b, 255], a as f32 / 255.0))
    };
    let order: Vec<(usize, f32, Vec<u32>, bool)> = puppet
        .drawables()
        .filter(|d| d.opacity > 0.05)
        .map(|d| {
            let inverted = d.flags & ayatsuri_format::model::flags::INVERTED_MASK != 0;
            (d.mesh, d.opacity, d.masks.to_vec(), inverted)
        })
        .collect();
    for (mi, opacity, masks, inverted) in order.into_iter().rev() {
        let Some((rgb, alpha)) = texel(mi) else { continue };
        if alpha * opacity < 0.5 {
            continue;
        }
        if !masks.is_empty() {
            let in_mask = masks.iter().any(|&k| texel(k as usize).is_some_and(|(_, a)| a > 0.5));
            if in_mask == inverted {
                continue;
            }
        }
        return Some(rgb);
    }
    None
}

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

/// A mesh the canvas may pick: shown, and every part above it shown.
fn mesh_pickable(s: &Session, mi: usize) -> bool {
    s.mesh_shown.get(mi).copied().unwrap_or(true)
        && ancestors_shown(s, s.puppet.model().art_meshes.get(mi).and_then(|m| m.parent_part))
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
    /// A click with its modifiers: Ctrl toggles, Shift picks a range.
    Pick(usize, egui::Modifiers),
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
            if lresp
                .on_hover_text(crate::i18n::tr("Link with the parameter below (2D pad)"))
                .clicked()
            {
                actions.push(ParamAction::Cmd(ParamCmd::Combine(pi)));
            }
        }
        let (nr, nresp) = ui.allocate_exact_size(vec2(92.0, 16.0), Sense::click());
        if *sel_param == Some(pi) || s.param_picked.contains(&pi) {
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
            actions.push(ParamAction::Pick(pi, ui.input(|i| i.modifiers)));
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
        // the value box sits flush with the row's right edge at a fixed
        // width, exactly where a pad row puts its boxes, so every value in
        // the palette lines up in one column
        let (rest, _) = ui.allocate_exact_size(vec2(ui.available_width(), 16.0), Sense::hover());
        let vr = Rect::from_min_size(pos2(rest.max.x - 52.0, rest.min.y), vec2(52.0, 16.0));
        let vresp = ui.put(
            vr,
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
        if *sel_param == Some(p) || s.param_picked.contains(&p) {
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
            actions.push(ParamAction::Pick(p, ui.input(|i| i.modifiers)));
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
    if menus && uresp.on_hover_text(crate::i18n::tr("Unlink into two sliders")).clicked() {
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
        if ui.button(crate::i18n::tr("Reset both")).clicked() {
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
    let mut pick = |ui: &mut Ui, label: &'static str, enabled: bool, c: HomeCmd| {
        if ui.add_enabled(enabled, egui::Button::new(crate::i18n::tr(label))).clicked() {
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
        return Err(Error::new(
            ErrorKind::InvalidInput,
            crate::i18n::tr("that is not a valid file name"),
        ));
    }
    let target = path.with_file_name(format!("{stem}{}", full_extension(path)));
    if target == path {
        return Ok(target);
    }
    if target.exists() {
        return Err(Error::new(
            ErrorKind::AlreadyExists,
            crate::i18n::tr("a file with that name already exists"),
        ));
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
        .ok_or_else(|| std::io::Error::other(crate::i18n::tr("too many copies")))?;
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

#[cfg(test)]
mod param_pick_tests {
    use super::*;

    /// An editor holding a blank model with parameters 0..5 in one folder.
    fn editor() -> EditorUi {
        let mut model = Model::new();
        for i in 0..5 {
            model.parameters.push(ayatsuri_format::model::Parameter {
                id: format!("P{i}"),
                min: 0.0,
                max: 1.0,
                default: 0.0,
                repeat: false,
                decimals: 3,
            });
        }
        let ctx = egui::Context::default();
        let mut s = Session::create(&ctx, "t".into(), String::new(), model).unwrap();
        s.groups = vec![crate::session::ParamGroup {
            name: String::new(),
            params: (0..5).collect(),
            open: true,
            color: None,
        }];
        let mut ed = EditorUi::new();
        ed.session = Some(s);
        ed
    }

    fn picked(ed: &EditorUi) -> Vec<usize> {
        ed.session.as_ref().unwrap().param_picked.clone()
    }

    #[test]
    fn click_ctrl_and_shift_pick() {
        let mut ed = editor();
        ed.pick_param(1, egui::Modifiers::NONE);
        assert_eq!(picked(&ed), vec![1]);
        ed.pick_param(3, egui::Modifiers::COMMAND);
        assert_eq!(picked(&ed), vec![1, 3]);
        ed.pick_param(1, egui::Modifiers::COMMAND);
        assert_eq!(picked(&ed), vec![3]);
        // Shift runs from the last plain or Ctrl click (1, even though that
        // click unpicked it), in palette order, like a file manager
        ed.pick_param(0, egui::Modifiers::SHIFT);
        assert_eq!(picked(&ed), vec![0, 1]);
        ed.pick_param(4, egui::Modifiers::SHIFT);
        assert_eq!(picked(&ed), vec![1, 2, 3, 4]);
        ed.pick_param(2, egui::Modifiers::NONE);
        assert_eq!(picked(&ed), vec![2]);
    }

    #[test]
    fn picked_params_move_together_in_order() {
        let mut ed = editor();
        ed.session.as_mut().unwrap().groups.push(crate::session::ParamGroup {
            name: "Face".into(),
            params: Vec::new(),
            open: false,
            color: None,
        });
        ed.pick_param(3, egui::Modifiers::NONE);
        ed.pick_param(1, egui::Modifiers::COMMAND);
        // a menu on a picked row covers the whole pick, palette order
        assert_eq!(ed.picked_with(1), vec![1, 3]);
        // on an unpicked row only that row
        assert_eq!(ed.picked_with(0), vec![0]);
        ed.run_param_cmd(ParamCmd::MoveSelectedTo(1, 1));
        let s = ed.session.as_ref().unwrap();
        assert_eq!(s.groups[0].params, vec![0, 2, 4]);
        assert_eq!(s.groups[1].params, vec![1, 3]);
        assert!(s.groups[1].open);
    }
}

#[cfg(test)]
mod drag_tests {
    use super::*;
    use ayatsuri_format::model::{ArtMesh, Binding, Canvas, Parameter};

    /// A mesh under a warp whose grid is bent hard, so its map is far from
    /// linear.
    fn bent_rig() -> ayatsuri_core::Puppet {
        let mut m = Model::new();
        m.canvas = Canvas { pixels_per_unit: 1.0, ..Canvas::default() };
        m.parameters.push(Parameter {
            id: "P".into(),
            min: 0.0,
            max: 1.0,
            default: 0.0,
            repeat: false,
            decimals: 3,
        });
        m.art_meshes.push(ArtMesh {
            id: "M".into(),
            parent_part: None,
            parent_deformer: None,
            visible: true,
            enabled: true,
            texture: 0,
            flags: 0,
            vertex_count: 3,
            uvs: vec![0.0; 6],
            indices: vec![0, 1, 2],
            masks: vec![],
            binding: Binding::default(),
            opacities: vec![1.0],
            draw_orders: vec![500.0],
            positions: vec![0.2, 0.2, 0.8, 0.2, 0.5, 0.8],
        });
        let w = ayatsuri_core::edit::create_deformer(
            &mut m,
            ayatsuri_core::edit::NewDeformer::Warp { rows: 2, cols: 2 },
            &[Child::Mesh(0)],
            None,
            (0.0, 0.0, 1.0, 1.0),
            None,
            "W".into(),
        )
        .unwrap();
        // bend the grid: pull the centre point far up and right
        if let DeformerKind::Warp(g) = &mut m.deformers[w].kind {
            g.positions[8] += 0.35;
            g.positions[9] -= 0.3;
        }
        let mut p = ayatsuri_core::Puppet::new(m);
        p.update();
        p
    }

    #[test]
    fn a_live_warp_drag_moves_the_mesh_before_release() {
        let mut puppet = bent_rig();
        let before = puppet.drawables().next().map(|d| d.positions.to_vec()).unwrap();
        let live = LiveDrag::start(&mut puppet, LiveTarget::Warp(0)).expect("live drag starts");
        // the whole grid shifted right, as a Bezier drag would hand it over
        let grid: Vec<f32> = live.local.chunks(2).flat_map(|p| [p[0] + 0.1, p[1]]).collect();
        live.show_local(&mut puppet, &grid);
        puppet.update();
        let during = puppet.drawables().next().map(|d| d.positions.to_vec()).unwrap();
        assert!((during[0] - before[0] - 0.1).abs() < 1e-3, "{before:?} -> {during:?}");
        live.restore(&mut puppet);
        puppet.update();
        let after = puppet.drawables().next().map(|d| d.positions.to_vec()).unwrap();
        assert_eq!(after, before);
    }

    #[test]
    fn dragged_points_land_exactly_under_the_pointer() {
        let puppet = bent_rig();
        let parent = Some(0u32);
        // a vertex position in the warp's space and a long drag across the bend
        let (px, py) = (0.3, 0.35);
        let (mdx, mdy) = (0.45, 0.4);
        let (dx, dy) = parent_delta(&puppet, parent, px, py, mdx, mdy);
        let (sx, sy) = puppet.deformer_point(0, px, py);
        let (fx, fy) = puppet.deformer_point(0, px + dx, py + dy);
        let miss = ((fx - (sx + mdx)).powi(2) + (fy - (sy + mdy)).powi(2)).sqrt();
        assert!(miss < 1e-4, "landed {miss} away from the pointer");
        // what the old single linear step did: noticeably short or past
        let e = 1e-3;
        let (ax, ay) = puppet.deformer_point(0, px + e, py);
        let (bx, by) = puppet.deformer_point(0, px, py + e);
        let (j00, j10, j01, j11) = ((ax - sx) / e, (ay - sy) / e, (bx - sx) / e, (by - sy) / e);
        let det = j00 * j11 - j01 * j10;
        let (lx, ly) = ((j11 * mdx - j01 * mdy) / det, (j00 * mdy - j10 * mdx) / det);
        let (gx, gy) = puppet.deformer_point(0, px + lx, py + ly);
        let old_miss = ((gx - (sx + mdx)).powi(2) + (gy - (sy + mdy)).powi(2)).sqrt();
        assert!(old_miss > 0.01, "the bend should defeat one linear step, missed by {old_miss}");
        // no parent: canvas moves are moves
        assert_eq!(parent_delta(&puppet, None, px, py, mdx, mdy), (mdx, mdy));
    }
}

#[cfg(test)]
mod mesh_delete_tests {
    use super::*;
    use ayatsuri_format::model::{ArtMesh, Binding};

    /// An editor holding three named meshes, A, B and C.
    fn editor() -> EditorUi {
        let mut model = Model::new();
        for i in 0..3 {
            model.art_meshes.push(ArtMesh {
                id: format!("ArtMesh{i}"),
                parent_part: None,
                parent_deformer: None,
                visible: true,
                enabled: true,
                texture: 0,
                flags: 0,
                vertex_count: 3,
                uvs: vec![0.0, 0.0, 1.0, 0.0, 0.0, 1.0],
                indices: vec![0, 1, 2],
                masks: vec![],
                binding: Binding::default(),
                opacities: vec![1.0],
                draw_orders: vec![500.0],
                positions: vec![0.0, 0.0, 1.0, 0.0, 0.0, 1.0],
            });
        }
        let ctx = egui::Context::default();
        let mut s = Session::create(&ctx, "t".into(), String::new(), model).unwrap();
        s.mesh_names = vec!["A".into(), "B".into(), "C".into()];
        s.mesh_shown = vec![true, false, true];
        let mut ed = EditorUi::new();
        ed.session = Some(s);
        ed
    }

    fn names(ed: &EditorUi) -> Vec<String> {
        let s = ed.session.as_ref().unwrap();
        (0..s.puppet.model().art_meshes.len()).map(|i| s.mesh_name(i)).collect()
    }

    #[test]
    fn names_and_visibility_follow_their_meshes_through_undo() {
        let mut ed = editor();
        ed.deform_paths.insert((2, 2), Vec::new());
        ed.selected = Some(Sel::Mesh(0));
        ed.delete_meshes(vec![0]);
        assert_eq!(names(&ed), ["B", "C"]);
        assert_eq!(ed.session.as_ref().unwrap().mesh_shown, vec![false, true]);
        assert!(ed.deform_paths.contains_key(&(1, 2)));
        assert!(ed.selected.is_none());
        ed.session.as_mut().unwrap().undo();
        assert_eq!(names(&ed), ["A", "B", "C"]);
        assert_eq!(ed.session.as_ref().unwrap().mesh_shown, vec![true, false, true]);
        ed.session.as_mut().unwrap().redo();
        assert_eq!(names(&ed), ["B", "C"]);
    }

    #[test]
    fn a_menu_on_a_picked_mesh_takes_the_whole_pick() {
        let mut ed = editor();
        ed.selected = Some(Sel::Mesh(2));
        ed.multi_sel = vec![0];
        assert_eq!(ed.mesh_pick_with(0), vec![0, 2]);
        assert_eq!(ed.mesh_pick_with(1), vec![1]);
        let pick = ed.mesh_pick_with(2);
        ed.delete_meshes(pick);
        assert_eq!(names(&ed), ["B"]);
    }
}

#[cfg(test)]
mod key_cmd_tests {
    use super::*;
    use ayatsuri_format::model::{ArtMesh, Binding, BindingAxis, Parameter};

    /// One mesh keyed on P at -1, 0 and 1, its vertex at x = key value.
    fn editor() -> EditorUi {
        let mut model = Model::new();
        model.parameters.push(Parameter {
            id: "P".into(),
            min: -1.0,
            max: 1.0,
            default: 0.0,
            repeat: false,
            decimals: 3,
        });
        model.art_meshes.push(ArtMesh {
            id: "ArtMesh0".into(),
            parent_part: None,
            parent_deformer: None,
            visible: true,
            enabled: true,
            texture: 0,
            flags: 0,
            vertex_count: 1,
            uvs: vec![0.0, 0.0],
            indices: vec![0, 0, 0],
            masks: vec![],
            binding: Binding {
                axes: vec![BindingAxis { parameter: 0, keys: vec![-1.0, 0.0, 1.0] }],
            },
            opacities: vec![1.0; 3],
            draw_orders: vec![500.0; 3],
            positions: vec![-1.0, 0.0, 0.0, 0.0, 2.0, 0.0],
        });
        let ctx = egui::Context::default();
        let s = Session::create(&ctx, "t".into(), String::new(), model).unwrap();
        let mut ed = EditorUi::new();
        ed.session = Some(s);
        ed.selected = Some(Sel::Mesh(0));
        ed
    }

    fn mesh(ed: &EditorUi) -> ArtMesh {
        ed.session.as_ref().unwrap().puppet.model().art_meshes[0].clone()
    }

    #[test]
    fn add_keys_goes_on_every_selected_mesh() {
        let mut ed = editor();
        // a second, unkeyed mesh beside the first
        if let Some(s) = ed.session.as_mut() {
            s.edit(|m| {
                let mut other = m.art_meshes[0].clone();
                other.id = "ArtMesh1".into();
                other.binding = Default::default();
                other.positions.truncate(2);
                other.opacities = vec![1.0];
                other.draw_orders = vec![500.0];
                m.art_meshes.push(other);
                true
            });
        }
        ed.selected = Some(Sel::Mesh(1));
        ed.multi_sel = vec![0, 1];
        ed.sel_param = Some(0);
        ed.add_keys_to_selection(vec![-1.0, 1.0]);
        let s = ed.session.as_ref().unwrap();
        let m = s.puppet.model();
        assert_eq!(m.art_meshes[1].binding.axes.len(), 1, "the second mesh got keys too");
        assert_eq!(m.art_meshes[1].opacities.len(), 2);
    }

    #[test]
    fn deleting_a_used_parameter_asks_then_unbinds_and_deletes() {
        let mut ed = editor();
        ed.delete_parameter(0);
        // still used by the mesh: nothing deleted yet, the question is up
        assert_eq!(ed.pmenu.delete_ask, Some((0, 1, 0)));
        assert_eq!(ed.session.as_ref().unwrap().puppet.model().parameters.len(), 1);
        ed.pmenu.delete_ask = None;
        ed.delete_parameter_unbinding(0);
        let s = ed.session.as_mut().unwrap();
        assert!(s.puppet.model().parameters.is_empty());
        assert!(s.puppet.model().art_meshes[0].binding.axes.is_empty());
        // one undo brings the parameter and the keys back
        s.undo();
        assert_eq!(s.puppet.model().parameters.len(), 1);
        assert_eq!(s.puppet.model().art_meshes[0].binding.axes.len(), 1);
    }

    #[test]
    fn delete_keyform_removes_the_key_at_the_value() {
        let mut ed = editor();
        ed.session.as_mut().unwrap().puppet.set_value(0, 1.0);
        ed.run_param_cmd(ParamCmd::DeleteKey(0));
        let m = mesh(&ed);
        assert_eq!(m.binding.axes[0].keys, vec![-1.0, 0.0]);
        assert_eq!(m.positions, vec![-1.0, 0.0, 0.0, 0.0]);
    }

    #[test]
    fn reflect_asks_then_mirrors_into_the_opposite_key() {
        let mut ed = editor();
        ed.session.as_mut().unwrap().puppet.set_value(0, 0.9);
        ed.run_param_cmd(ParamCmd::Reflect(0));
        // it only asks: the key at 1 would overwrite the one at -1
        assert_eq!(ed.pmenu.reflect, Some((0, 1.0, Some(-1.0))));
        assert_eq!(mesh(&ed).positions, vec![-1.0, 0.0, 0.0, 0.0, 2.0, 0.0]);
        // confirmed: the move away from the default (x + 2) comes back
        // mirrored (x - 2) on the opposite key
        ed.reflect_into_opposite(0, 1.0);
        assert_eq!(mesh(&ed).positions, vec![-2.0, 0.0, 0.0, 0.0, 2.0, 0.0]);
    }

    #[test]
    fn reflect_on_the_default_key_has_nothing_to_overwrite() {
        let mut ed = editor();
        ed.run_param_cmd(ParamCmd::Reflect(0));
        assert_eq!(ed.pmenu.reflect, Some((0, 0.0, None)));
    }
}

#[cfg(test)]
mod point_paste_tests {
    use super::*;

    #[test]
    fn basis_weights_skip_points_in_a_line() {
        // the two nearest and the third are in a line; the fourth is not
        let pts = [pos2(0.0, 0.0), pos2(1.0, 0.0), pos2(2.0, 0.0), pos2(0.0, 3.0)];
        let (c, w) = basis_weights(&pts, pos2(0.5, 0.5)).unwrap();
        assert_eq!(c, [0, 1, 3]);
        let x: f32 = c.iter().zip(w).map(|(&i, wi)| pts[i].x * wi).sum();
        let y: f32 = c.iter().zip(w).map(|(&i, wi)| pts[i].y * wi).sum();
        assert!((x - 0.5).abs() < 1e-5 && (y - 0.5).abs() < 1e-5, "{x},{y}");
        assert!(basis_weights(&pts[..3], pos2(0.5, 0.5)).is_none());
    }
}

#[cfg(test)]
mod palette_pick_tests {
    use super::*;

    #[test]
    fn click_ctrl_and_shift_pick_parts_in_palette_order() {
        let mut ed = EditorUi::new();
        // palette order differs from index order on purpose
        let order = [4, 1, 3, 0, 2];
        ed.pick_rows(1, false, false, &order, true);
        assert_eq!(ed.part_pick, vec![1]);
        ed.pick_rows(0, true, false, &order, true);
        assert_eq!(ed.part_pick, vec![1, 3, 0]);
        // Ctrl toggles one off and moves the anchor there
        ed.pick_rows(3, false, true, &order, true);
        assert_eq!(ed.part_pick, vec![1, 0]);
        ed.pick_rows(4, true, false, &order, true);
        assert_eq!(ed.part_pick, vec![4, 1, 3]);
        ed.pick_rows(2, false, false, &order, true);
        assert_eq!(ed.part_pick, vec![2]);
    }

    #[test]
    fn mesh_rows_extend_what_the_canvas_selected() {
        let mut ed = EditorUi::new();
        ed.selected = Some(Sel::Mesh(5));
        ed.pick_rows(7, false, true, &[5, 6, 7], false);
        assert_eq!(ed.multi_sel, vec![5, 7]);
        // Shift from a canvas selection runs from that mesh
        let mut ed = EditorUi::new();
        ed.selected = Some(Sel::Mesh(6));
        ed.pick_rows(8, true, false, &[5, 6, 7, 8], false);
        assert_eq!(ed.multi_sel, vec![6, 7, 8]);
    }
}

#[cfg(test)]
mod palette_search_tests {
    use super::*;

    #[test]
    fn a_search_keeps_matches_their_ancestors_and_whats_inside_a_match() {
        // 0 Head > 1 Face > 2 Eyes ; 3 Body
        let parents = [None, Some(0), Some(1), None];
        // meshes: iris (in Eyes), mouth (in Face), shirt (in Body)
        let leaf_parent = [Some(2), Some(1), Some(3)];
        // searching "iris": only the iris and the folders above it
        let (nodes, leaves) = search_keep(
            &parents,
            &[false, false, false, false],
            &leaf_parent,
            &[true, false, false],
        );
        assert_eq!(nodes, vec![true, true, true, false]);
        assert_eq!(leaves, vec![true, false, false]);
        // searching "face": the Face folder shows all it holds
        let (nodes, leaves) = search_keep(
            &parents,
            &[false, true, false, false],
            &leaf_parent,
            &[false, false, false],
        );
        assert_eq!(nodes, vec![true, true, true, false]);
        assert_eq!(leaves, vec![true, true, false]);
        assert!(matches_search("ey", &["Eye L", "ArtMesh3"]));
        assert!(!matches_search("ey", &["Mouth"]));
    }
}

#[cfg(test)]
mod eyedropper_tests {
    use super::*;
    use ayatsuri_format::model::{ArtMesh, Binding, Canvas};

    #[test]
    fn the_eyedropper_reads_the_texel_under_the_point() {
        let mut m = Model::new();
        m.canvas = Canvas { pixels_per_unit: 1.0, ..Canvas::default() };
        m.art_meshes.push(ArtMesh {
            id: "Q".into(),
            parent_part: None,
            parent_deformer: None,
            visible: true,
            enabled: true,
            texture: 0,
            flags: 0,
            vertex_count: 4,
            uvs: vec![0.0, 0.0, 1.0, 0.0, 1.0, 1.0, 0.0, 1.0],
            indices: vec![0, 1, 2, 0, 2, 3],
            masks: vec![],
            binding: Binding::default(),
            opacities: vec![1.0],
            draw_orders: vec![500.0],
            positions: vec![0.0, 0.0, 1.0, 0.0, 1.0, 1.0, 0.0, 1.0],
        });
        let mut puppet = ayatsuri_core::Puppet::new(m);
        puppet.update();
        let px = vec![255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 255, 255];
        let mut png = Vec::new();
        image::RgbaImage::from_raw(2, 2, px)
            .unwrap()
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        let pages = vec![png];
        let mut cache = std::collections::HashMap::new();
        let at = |x, y, cache: &mut _| color_at(&puppet, &pages, cache, x, y);
        assert_eq!(at(0.25, 0.25, &mut cache), Some([255, 0, 0, 255]));
        assert_eq!(at(0.75, 0.25, &mut cache), Some([0, 255, 0, 255]));
        assert_eq!(at(0.25, 0.75, &mut cache), Some([0, 0, 255, 255]));
        assert_eq!(at(2.0, 2.0, &mut cache), None);
    }
}
