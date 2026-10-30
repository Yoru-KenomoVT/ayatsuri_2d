use std::path::{Path, PathBuf};

use ayatsuri_core::edit::Child;
use ayatsuri_core::palette::{Drop as PDrop, Row as PRow};
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
mod skin_dialog;
mod stroke_tool;
mod temp_deform;
mod tool_details;
mod weight_limits;
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
    /// Modeling > Temporary Deform Tool, bound to the selection.
    TempDeform,
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
            Tool::TempDeform => egui_phosphor::regular::BOUNDING_BOX,
        }
    }
    fn label(self) -> &'static str {
        match self {
            Tool::Arrow => "Arrow",
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
            Tool::TempDeform => "Temporary Deform Tool",
        }
    }
}

/// A deformer picked on the canvas (Shift+Alt+click, or the right-click
/// tree): alone, or with `add` put into the pick or taken out of it.
/// The selection it changes, borrowed field by field
/// so the canvas can pick while it holds the session.
struct CanvasPick<'a> {
    model: &'a ayatsuri_format::Model,
    selected: &'a mut Option<Sel>,
    pick: &'a mut Vec<usize>,
    anchor: &'a mut Option<usize>,
    meshes: &'a mut Vec<usize>,
    parts: &'a mut Vec<usize>,
}

fn canvas_def_pick(c: CanvasPick, d: usize, add: bool) {
    let mut pick = def_group(*c.selected, c.pick);
    if !add {
        pick.clear();
    }
    match pick.iter().position(|&x| x == d) {
        Some(at) if add => {
            pick.remove(at);
        }
        Some(_) => {}
        None => pick.push(d),
    }
    let pick = top_level_pick(c.model, &pick);
    *c.selected = if pick.contains(&d) {
        Some(Sel::Deformer(d))
    } else {
        pick.last().copied().map(Sel::Deformer)
    };
    *c.anchor = Some(d);
    *c.pick = pick;
    c.meshes.clear();
    c.parts.clear();
}

/// Several deformers are picked at the top level only, as in Cubism: one
/// inside another picked deformer is left out of the pick, so a rigged
/// child taken in by mistake never gets the group's moves or keys.
fn top_level_pick(model: &ayatsuri_format::Model, pick: &[usize]) -> Vec<usize> {
    if pick.len() < 2 {
        return pick.to_vec();
    }
    let kept = outermost(model, pick);
    if kept.len() < pick.len() {
        log::info!(
            "{} deformer(s) inside another picked one left out of the pick",
            pick.len() - kept.len()
        );
    }
    kept
}

/// The deformers in `picked` with no other picked deformer above them.
fn outermost(model: &ayatsuri_format::Model, picked: &[usize]) -> Vec<usize> {
    picked
        .iter()
        .copied()
        .filter(|&d| {
            let mut cur = model.deformers.get(d).and_then(|x| x.parent_deformer);
            let mut hops = 0;
            while let Some(p) = cur.map(|p| p as usize) {
                if picked.contains(&p) || hops > 256 {
                    return hops > 256;
                }
                hops += 1;
                cur = model.deformers.get(p).and_then(|x| x.parent_deformer);
            }
            true
        })
        .collect()
}

/// (min x, min y, max x, max y) on the canvas.
type Extent = (f32, f32, f32, f32);

/// An object's extent on the canvas now.
fn world_extent(puppet: &ayatsuri_core::Puppet, t: ayatsuri_core::edit::Target) -> Option<Extent> {
    use ayatsuri_core::edit::Target;
    let pts: Vec<f32> = match t {
        Target::ArtMesh(i) => puppet.mesh_geometry(i)?.positions.to_vec(),
        Target::Deformer(d) => match puppet.warp_grid(d) {
            Some((g, _, _)) => g.to_vec(),
            None => {
                let o = puppet.rotation_state(d)?.origin;
                vec![o.x, o.y]
            }
        },
        Target::Part(_) => return None,
    };
    let (xy, _) = pts.as_chunks::<2>();
    xy.iter().fold(None, |acc, p| {
        Some(match acc {
            None => (p[0], p[1], p[0], p[1]),
            Some((a, b, c, d)) => (a.min(p[0]), b.min(p[1]), c.max(p[0]), d.max(p[1])),
        })
    })
}

/// An object's parent deformer and its middle in that parent's space, for
/// turning a canvas move into the object's own.
fn local_centre(
    puppet: &ayatsuri_core::Puppet,
    t: ayatsuri_core::edit::Target,
) -> Option<(Option<u32>, (f32, f32))> {
    use ayatsuri_core::edit::Target;
    let model = puppet.model();
    let mean = |v: &[f32]| {
        let (xy, _) = v.as_chunks::<2>();
        let n = xy.len().max(1) as f32;
        xy.iter().fold((0.0, 0.0), |(x, y), p| (x + p[0] / n, y + p[1] / n))
    };
    match t {
        Target::ArtMesh(i) => {
            Some((model.art_meshes.get(i)?.parent_deformer, mean(&puppet.mesh_local_positions(i)?)))
        }
        Target::Deformer(d) => {
            let parent = model.deformers.get(d)?.parent_deformer;
            match puppet.warp_local_grid(d) {
                Some(g) => Some((parent, mean(&g))),
                None => Some((parent, puppet.rotation_local_origin(d)?)),
            }
        }
        Target::Part(_) => None,
    }
}

/// What the canvas menu needs to know, read before it is drawn.
struct MenuCtx {
    mesh_edit: Option<usize>,
    picked_points: usize,
    /// The selected deformer and whether it is a warp.
    deformer: Option<(usize, bool)>,
    /// The mesh under the click (else the selected one).
    mesh: Option<usize>,
    shown: bool,
    locked: bool,
    sc: Vec<(&'static str, String)>,
}

/// The canvas menu's modeling items: Mesh Edit's when a mesh is open in
/// it, else the selected deformer's, else those of the mesh clicked on.
fn canvas_menu_items(ui: &mut Ui, mc: &MenuCtx, out: &mut Option<CanvasCmd>) {
    let sc =
        |a: &str| mc.sc.iter().find(|(k, _)| *k == a).map(|(_, v)| v.clone()).unwrap_or_default();
    if let Some(mi) = mc.mesh_edit {
        let n = mc.picked_points;
        menu_item(ui, out, n >= 2, "Merge Vertices", sc("merge-vertices"), CanvasCmd::MergePoints);
        menu_item(ui, out, n >= 1, "Delete Points", String::new(), CanvasCmd::DeletePoints);
        ui.separator();
        menu_item(
            ui,
            out,
            true,
            "Auto Connect",
            sc("auto-connect"),
            CanvasCmd::Mesh(MeshAction::AutoConnect),
        );
        menu_item(
            ui,
            out,
            true,
            "Divide into 4",
            String::new(),
            CanvasCmd::Mesh(MeshAction::Divide),
        );
        menu_item(ui, out, true, "Reset Mesh", String::new(), CanvasCmd::Mesh(MeshAction::Reset));
        menu_flip(ui, out, mi);
        ui.separator();
        menu_item(ui, out, true, "Finish Mesh Edit", sc("mesh-edit"), CanvasCmd::FinishMeshEdit);
        ui.separator();
        return;
    }
    if let Some((d, warp)) = mc.deformer {
        menu_item(ui, out, true, "Rename…", sc("rename"), CanvasCmd::Def(DefCmd::Rename(d)));
        ui.menu_button(crate::i18n::tr("Flip"), |ui| {
            for (label, h, alone) in [
                ("Flip Horizontally", true, false),
                ("Flip Vertically", false, false),
                ("Flip Warp Only Horizontally", true, true),
                ("Flip Warp Only Vertically", false, true),
            ] {
                if warp || !alone {
                    menu_item(
                        ui,
                        out,
                        true,
                        label,
                        String::new(),
                        CanvasCmd::Def(DefCmd::Flip(d, h, alone)),
                    );
                }
            }
        });
        if warp {
            menu_item(
                ui,
                out,
                true,
                "Conversion Divisions…",
                String::new(),
                CanvasCmd::Def(DefCmd::Division(d)),
            );
        }
        menu_item(ui, out, true, "Delete", String::new(), CanvasCmd::Def(DefCmd::Delete(d)));
        ui.separator();
        return;
    }
    let Some(mi) = mc.mesh else { return };
    menu_item(ui, out, true, "Edit Mesh", sc("mesh-edit"), CanvasCmd::EditMesh(mi));
    menu_item(ui, out, true, "Rename…", sc("rename"), CanvasCmd::RenameMesh(mi));
    menu_item(ui, out, true, "Clip by…", String::new(), CanvasCmd::ClipBy(mi));
    menu_item(ui, out, true, "Copy ID", sc("copy-ids"), CanvasCmd::CopyIds(mi));
    ui.separator();
    menu_item(
        ui,
        out,
        true,
        "Create Warp Deformer…",
        String::new(),
        CanvasCmd::CreateAround(mi, true),
    );
    menu_item(
        ui,
        out,
        true,
        "Create Rotation Deformer…",
        String::new(),
        CanvasCmd::CreateAround(mi, false),
    );
    menu_item(
        ui,
        out,
        true,
        "Select Parent Deformer",
        sc("select-parent"),
        CanvasCmd::SelectParent(mi),
    );
    menu_flip(ui, out, mi);
    ui.separator();
    let shown = if mc.shown { "Hide" } else { "Show" };
    menu_item(ui, out, true, shown, sc("toggle-visible"), CanvasCmd::ToggleShown(mi));
    let lock = if mc.locked { "Unlock" } else { "Lock" };
    menu_item(ui, out, true, lock, sc("lock-selected"), CanvasCmd::ToggleLock(mi));
    menu_item(ui, out, true, "Delete Art Mesh", String::new(), CanvasCmd::DeleteMeshes(mi));
    ui.separator();
}

/// One canvas menu entry with its shortcut on the right.
fn menu_item(
    ui: &mut Ui,
    out: &mut Option<CanvasCmd>,
    on: bool,
    label: &'static str,
    shortcut: String,
    cmd: CanvasCmd,
) {
    if ui
        .add_enabled(on, egui::Button::new(crate::i18n::tr(label)).shortcut_text(shortcut))
        .clicked()
    {
        *out = Some(cmd);
        ui.close();
    }
}

/// The Flip submenu for an art mesh (with the pick it is in).
fn menu_flip(ui: &mut Ui, out: &mut Option<CanvasCmd>, mi: usize) {
    ui.menu_button(crate::i18n::tr("Flip"), |ui| {
        for (label, h) in [("Flip Horizontally", true), ("Flip Vertically", false)] {
            menu_item(ui, out, true, label, String::new(), CanvasCmd::Flip(mi, h));
        }
    });
}

/// The deformers above an art mesh, root first (a loop is cut short).
fn deformer_chain(model: &ayatsuri_format::Model, mesh: usize) -> Vec<usize> {
    let mut out = Vec::new();
    let mut cur = model.art_meshes.get(mesh).and_then(|m| m.parent_deformer);
    while let Some(d) = cur.map(|d| d as usize) {
        if out.contains(&d) {
            break;
        }
        out.push(d);
        cur = model.deformers.get(d).and_then(|x| x.parent_deformer);
    }
    out.reverse();
    out
}

/// The deformers a deformer action works on: the palette pick when the
/// selected deformer is in it, else the selected deformer alone.
fn def_group(selected: Option<Sel>, pick: &[usize]) -> Vec<usize> {
    match selected {
        Some(Sel::Deformer(d)) if pick.contains(&d) => pick.to_vec(),
        Some(Sel::Deformer(d)) => vec![d],
        _ => Vec::new(),
    }
}

/// Ctrl+click toggles `row` in the pick, Shift+click picks the run from
/// the anchor in `order`, a plain click picks just `row`.
fn pick_in(
    pick: &mut Vec<usize>,
    anchor: &mut Option<usize>,
    row: usize,
    shift: bool,
    ctrl: bool,
    order: &[usize],
) {
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

/// The shortcut action that picks a tool, for its toolbar tip.
fn tool_key(t: Tool) -> Option<&'static str> {
    Some(match t {
        Tool::Arrow => "tool-arrow",
        Tool::Lasso => "tool-lasso",
        Tool::BrushSelect => "tool-brush",
        Tool::DeformPath => "tool-path",
        Tool::DeformBrush => "tool-deform-brush",
        Tool::Glue => "tool-glue",
        Tool::ArtPath => "tool-artpath",
        Tool::MeshManual => "mesh-edit",
        Tool::MeshAuto => "tool-auto-mesh",
        Tool::WarpCreate => "tool-warp",
        Tool::RotationCreate => "tool-rotation",
        Tool::TextureAtlas => "tool-atlas",
        Tool::TempDeform => return None,
    })
}

/// Per-tool options shown in the Tool Details palette.
struct ToolOptions {
    brush_size: f32,
    /// The Automatic mesh generator's settings as they stand.
    auto: crate::prefs::MeshPreset,
    /// The name being typed for a new preset (Add…), while it is open.
    auto_new_name: Option<String>,
    warp_cols: u32,
    warp_rows: u32,
    /// Stroke mesh-mapping's settings.
    stroke: stroke_tool::StrokeOpts,
}

impl Default for ToolOptions {
    fn default() -> Self {
        Self {
            brush_size: 40.0,
            auto: crate::prefs::MeshPreset::default(),
            auto_new_name: None,
            warp_cols: 5,
            warp_rows: 5,
            stroke: stroke_tool::StrokeOpts::default(),
        }
    }
}

#[derive(Clone, PartialEq, Eq)]
enum Workspace {
    Modeling,
    Animation,
    Custom(usize),
}

/// The editor workspace: a dockable tree of palettes around one loaded
/// puppet, laid out and styled after Cubism Editor's Modeling workspace.
pub struct EditorUi {
    layout: DockNode,
    workspace: Workspace,
    pub(crate) session: Option<Session>,
    selected: Option<Sel>,
    /// The last click picked a parameter: Delete leaves the selected mesh
    /// alone until the next click elsewhere.
    param_focus: bool,
    log_view: log::Level,
    pending_import: Option<PathBuf>,
    /// View zoom relative to fit-to-panel, and pan in screen points.
    view_zoom: f32,
    view_pan: Vec2,
    pub(crate) prefs: Prefs,
    tool: Tool,
    pub(crate) show_about: bool,
    /// What's new: this version's changelog, once per version and from Help.
    pub(crate) show_whats_new: bool,
    pub(crate) update_win: crate::whats_new::UpdateWindow,
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
    skin_dlg: Option<skin_dialog::SkinDialog>,
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
    /// Windows, dialogs and modes open last frame, one bit each (see
    /// `log_state_changes`), so the log says what the user opened.
    logged_open: u64,
    /// The window title last sent: sending one every frame would keep an
    /// idle editor drawing, as each window command asks for a frame.
    sent_title: String,
    heartbeat: f32,
    /// Timeline state: playhead seconds, transport, auto-key, and the time
    /// the tracks were last written into the puppet.
    anim_time: f32,
    anim_playing: bool,
    /// Playback as it was last frame, and every parameter's value from
    /// when it started: stopping puts the model back the way it was.
    anim_was_playing: bool,
    play_pose: Vec<f32>,
    anim_autokey: bool,
    /// Space went down with a drag, so letting it go is not a play tap.
    space_dragged: bool,
    /// The Expression palette, and the pose under its preview: each
    /// parameter the expression touches with the value it had before.
    expressions: crate::expressions::Expressions,
    expr_base: Vec<(usize, f32)>,
    expr_shown: Option<usize>,
    /// The sound output, opened when a composition first plays, and the
    /// composition time its players were started for last frame.
    pub(crate) audio_out: Option<crate::audio::Out>,
    pub(crate) audio_at: Option<f32>,
    /// Output devices as Settings last listed them.
    pub(crate) audio_devices: Option<Vec<crate::audio::Device>>,
    /// The Audio Mixer palette's state.
    pub(crate) mixer: crate::mixer::Mixer,
    anim_applied: f32,
    timeline: crate::timeline::Timeline,
    new_anim: Option<AnimNew>,
    /// The Animation details dialog's working copy.
    anim_details: Option<AnimDetails>,
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
    /// Cubism's New Part dialog, while it is open.
    new_part: Option<NewPartDlg>,
    /// Model Template: applying or saving a parameter template.
    template_dlg: Option<TemplateDlg>,
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
    /// Change the draw order of all ArtMeshes, while open: Auto setup, and
    /// the Manual setup interval.
    draw_spread: Option<(bool, f32)>,
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
    /// Stroke mesh-mapping's strokes, not yet confirmed.
    stroke_edit: Option<stroke_tool::StrokeEdit>,
    /// Confirm asked for on the canvas: applied once it lets go.
    stroke_confirm: bool,
    /// The mesh Mesh Edit was on last frame, to notice it closing.
    mesh_edit_was: Option<usize>,
    /// The Mesh Edit dev hook has run, and has logged its vertices.
    dev_mesh_done: bool,
    dev_mesh_logged: bool,
    /// A mesh or warp point drag being shown live on the model.
    live_drag: Option<LiveDrag>,
    /// The same for a box over several objects (meshes or deformers).
    live_group: LiveGroup,
    /// A rotation being turned live: the deformer and its angles as they were.
    rot_live: Option<(usize, Vec<f32>)>,
    /// Rotation handles made longer with Alt+drag, by deformer: display only.
    rot_handle: std::collections::HashMap<usize, f32>,
    /// Set by "Reset Toolbar Position": put the floating bar back next frame.
    toolbar_reset: bool,
    /// The Rename dialog of a deformer or an art mesh: which, the name and
    /// ID being typed, and whether the last ID was refused.
    def_rename: Option<(Sel, String, String, bool)>,
    /// The Conversion Divisions dialog: the warp and the columns and rows
    /// being set.
    def_division: Option<(usize, u32, u32)>,
    /// Tool Details' Conversion division while it differs from the warp's:
    /// the warp and the columns and rows typed, applied with Apply.
    td_division: Option<(usize, u32, u32)>,
    /// Undo depth at the start of the frame; see `on_crash`.
    frame_undo_depth: usize,
    /// When the last frame began, and the real time since the one before:
    /// playback and physics step by it, not by egui's predicted frame time,
    /// so uneven frames cannot make them fall behind the clock.
    frame_at: Option<std::time::Instant>,
    frame_dt: f32,
    /// The frame limiter's beat: whether the last frame asked for another
    /// animation frame, and when the last animation frame started.
    anim_wanted: std::cell::Cell<bool>,
    anim_slot: Option<std::time::Instant>,
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
    /// The key Motion Mirroring last wrote to (parameter, key) and the
    /// selection it was for; its red mark goes when the selection changes.
    mirror_mark: Option<(usize, f32, Vec<ayatsuri_core::edit::Target>)>,
    deformer_dialog: Option<deformer_dialog::DeformerDialog>,
    /// Extra meshes picked by the lasso, beyond `selected`.
    multi_sel: Vec<usize>,
    /// Parts picked in the Parts palette (Ctrl toggles, Shift a range), and
    /// the rows each Shift range starts from.
    part_pick: Vec<usize>,
    /// Cubism's Prohibit drag and drop for the Parts palette.
    parts_dnd_off: bool,
    /// Show > Highlight Deformer's Child Elements.
    highlight_children: bool,
    /// View > Highlight Overlapping Polygons.
    highlight_overlap: bool,
    /// Cubism's Texture Atlas View (T): meshes draw every pixel their uvs
    /// cover on the atlas, as the export will, instead of only their own
    /// art, so one grown over a neighbour shows it.
    atlas_view: bool,
    part_anchor: Option<usize>,
    /// Deformers picked together in the Deformer palette (Ctrl or Shift
    /// click); see `def_group` for when it counts.
    def_pick: Vec<usize>,
    def_anchor: Option<usize>,
    /// The Deformer palette's last plain or Ctrl clicked row, deformer
    /// (true) or mesh, for a Shift+click range across both kinds of row.
    def_row_anchor: Option<(bool, usize)>,
    /// The Parts palette row under the pointer, outlined on the canvas.
    palette_hover: Option<Sel>,
    /// The Parts and Deformer palettes' search boxes.
    part_search: String,
    def_search: String,
    mesh_anchor: Option<usize>,
    /// Picking clipping masks on the canvas for this mesh: each click on
    /// another mesh adds it (Shift keeps picking), Esc ends.
    clip_pick: Option<usize>,
    /// The rest of the pick `clip_pick` was started on: every mesh in it
    /// gets the same masks.
    clip_also: Vec<usize>,
    /// A deformer creation requested from inside a borrow (canvas, tool pane).
    pending_create: Option<(bool, Option<(f32, f32)>)>,
    /// A guide line asked for from a menu, placed once the canvas is known.
    pending_guide: Option<GuideCmd>,
    /// Where the canvas was last right-clicked, for "add a guide here".
    ctx_click: Option<Pos2>,
    /// The meshes drawn where the canvas was right-clicked, front first,
    /// for its Select Layer menu.
    ctx_layers: Vec<usize>,
    /// The deformers above the front-most mesh under the right-click, root
    /// first, for the tree at the top of the canvas menu.
    ctx_tree: Vec<usize>,
    /// An action picked on the canvas menu, run at the start of the next
    /// frame (the canvas holds the session while the menu is drawn).
    canvas_cmd: Option<CanvasCmd>,
    /// The right-click came with Ctrl: Cubism's ArtMesh pop-up list, the
    /// meshes under the pointer and nothing else.
    ctx_mesh_list: bool,
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
    /// The edge tool's first pick, and in Add vertex the point the next
    /// one draws its line from (Cubism's chain; Esc ends it).
    edge_first: Option<usize>,
    /// Lines drawn with the edge tool that have not closed a face yet,
    /// per mesh; they turn into faces as soon as a triangle closes.
    pending_edges: std::collections::HashMap<usize, Vec<(u16, u16)>>,
    /// The model as it was when mesh editing began, for Cancel.
    mesh_snapshot: Option<Model>,
    /// Deform paths per (mesh, edit level 2 or 3), as in Cubism.
    deform_paths: std::collections::HashMap<(usize, u8), Vec<PathPin>>,
    path_grab: Option<PathGrab>,
    /// Where a press away from the path's points went down: a pen tap
    /// wanders a few pixels and reads as a drag, so a short one still
    /// places a point there.
    path_press: Option<Pos2>,
    art_draft: Option<ayatsuri_format::model::ArtPath>,
    art_grab: Option<ArtGrab>,
    art_edit: Option<(usize, ayatsuri_format::model::ArtPath)>,
    art_style: ayatsuri_format::model::ArtPath,
    art_finish: bool,
    art_pick: Option<u8>,
    /// The Inspector's eyedropper: which colour waits for a click on the
    /// art, and the colour a click took, for the Inspector to set.
    color_pick: Option<ayatsuri_core::edit::ColorKind>,
    color_picked: Option<(ayatsuri_core::edit::ColorKind, [u8; 3])>,
    clip_with: Option<usize>,
    clip_search: String,
    /// The Clip by field while it is being typed in: the mesh and the text.
    clip_ids: Option<(usize, String)>,
    /// The name typed for a new clip group.
    clip_group_name: String,
    dbrush: Option<DeformStroke>,
    tdeform: Option<temp_deform::TempDeform>,
    tdeform_cols: usize,
    tdeform_rows: usize,
    /// Physics was on when the Temporary Deform Tool started: it pauses
    /// while the tool is bound and comes back when it is let go.
    tdeform_paused_physics: bool,
    /// Why the Temporary Deform Tool would not start, shown in a window.
    tdeform_error: Option<&'static str>,
    dbrush_hardness: f32,
    dbrush_strength: f32,
    /// The pen tablet's pressure, read each frame.
    pub(crate) pen: crate::pen::Pen,
    show_guides: bool,
    /// Mirror edit: point moves repeat, flipped, on the point across the
    /// vertical axis (a vertical guide line nearest the centre, else the
    /// canvas centre).
    mirror_edit: bool,
    guide_drag: Option<usize>,
    clip_invert: bool,
    art_pending: Option<(usize, ayatsuri_format::model::ArtPath)>,
    pick_pages: std::collections::HashMap<usize, (usize, u32, u32, Vec<u8>)>,
    /// Each texture page's alpha, a quarter size or less, so a click only
    /// picks a mesh where its art is drawn.
    pick_alpha: AlphaPages,
    /// Where the last plain click picked, so a second click on the same
    /// spot goes to the mesh under the one it picked.
    last_pick: Option<Pos2>,
    /// A short notice over the canvas (why a click did nothing), and when
    /// it goes; the time is set the first frame it shows.
    notice: Option<(String, Option<f64>)>,
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
    /// The mesh itself as it was then: new points are placed on it, so
    /// they land on the art where clicked however few points are left.
    reference: ayatsuri_format::model::ArtMesh,
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
    /// Mirror Edit: the point across the axis, and how far it has moved
    /// (parent space), mirroring this one.
    partner: Option<(usize, f32, f32)>,
}

/// A Mesh Edit point move: the vertex, how far (parent space), and the
/// texture coordinate it slides to when known.
type PointMove = (usize, f32, f32, Option<(f32, f32)>);

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
    /// Ctrl when the drag began: the deformer changes, what it holds
    /// stays (Cubism's table); without it the centre carries its art.
    keep: bool,
    /// Alt on the handle: only the handle's length changes.
    handle_only: bool,
    /// The centre dragged without Ctrl: the keyform it moves in and its
    /// stored origin, for the live preview.
    origin_base: Option<(usize, f32, f32)>,
    /// The round scale handle is held: how far from the centre the press
    /// was, the factor so far, and (without Ctrl) the keyform and stored
    /// scale for the live preview.
    scaling: bool,
    scale_from: f32,
    factor: f32,
    scale_base: Option<(usize, f32)>,
}

impl RotDrag {
    /// The turn so far; Shift holds it to 45 degree steps.
    fn turn(&self, shift: bool) -> f32 {
        if shift { (self.degrees / 45.0).round() * 45.0 } else { self.degrees }
    }
}

/// The rotation deformer's round scale handle: a ring this far from its
/// centre, in screen pixels.
const ROT_RING: f32 = 26.0;

/// What the canvas's right-click menu can do.
enum CanvasCmd {
    // in Mesh Edit
    MergePoints,
    DeletePoints,
    Mesh(MeshAction),
    FinishMeshEdit,
    // on an art mesh, or the selection it is in
    EditMesh(usize),
    RenameMesh(usize),
    ClipBy(usize),
    CopyIds(usize),
    CreateAround(usize, bool),
    SelectParent(usize),
    Flip(usize, bool),
    ToggleShown(usize),
    ToggleLock(usize),
    DeleteMeshes(usize),
    // on the selected deformer
    Def(DefCmd),
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
    /// Shift held: where it went down, and the axis the stroke is held to
    /// (true for X) once it has moved far enough to tell.
    lock: Option<(Pos2, Option<bool>)>,
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

/// Ask for the next animation frame straight away; under an FPS cap the
/// frame limiter at the top of that frame holds it to its slot.
fn pace_anim(wanted: &std::cell::Cell<bool>, ctx: &egui::Context) {
    wanted.set(true);
    ctx.request_repaint();
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

/// One object under a box over several: a mesh or warp keyform, or a
/// rotation deformer's origin (one point).
#[derive(Clone, Copy, PartialEq)]
enum GroupObj {
    Live(LiveTarget),
    Origin(usize),
}

/// A rotation deformer origin under a live group preview: deformer,
/// keyform, the stored origin and the blended local one.
type LiveOrigin = (usize, usize, (f32, f32), (f32, f32));

/// A live preview for a box over several objects: while the box is dragged
/// each object shows its points where the box has them, and everything is
/// put back before the edit lands (or the drag ends).
#[derive(Default)]
struct LiveGroup {
    live: Vec<LiveDrag>,
    origins: Vec<LiveOrigin>,
}

impl LiveGroup {
    /// Put every object back; true when something was showing.
    fn restore(&mut self, puppet: &mut ayatsuri_core::Puppet) -> bool {
        let any = !self.live.is_empty() || !self.origins.is_empty();
        for l in self.live.drain(..) {
            l.restore(puppet);
        }
        for (d, k, (x, y), _) in self.origins.drain(..) {
            if let Some((ox, oy)) = puppet.rotation_origin_mut(d, k) {
                (*ox, *oy) = (x, y);
            }
        }
        any
    }

    /// Show a drag: `from` is where the dragged points started, `pts` where
    /// they are now, `owner` whose point each is. With no drag (or a
    /// marquee) it puts everything back. True when the puppet changed.
    fn update(
        &mut self,
        puppet: &mut ayatsuri_core::Puppet,
        from: Option<&[(usize, Pos2)]>,
        pts: &[Pos2],
        owner: &[(GroupObj, usize)],
        k_unit: f32,
    ) -> bool {
        let Some(from) = from else { return self.restore(puppet) };
        if self.live.is_empty() && self.origins.is_empty() {
            let values: Vec<f32> =
                (0..puppet.model().parameters.len()).map(|i| puppet.value(i)).collect();
            let mut seen: Vec<GroupObj> = Vec::new();
            for &(obj, _) in owner {
                if seen.contains(&obj) {
                    continue;
                }
                seen.push(obj);
                match obj {
                    GroupObj::Live(t) => {
                        if let Some(l) = LiveDrag::start(puppet, t) {
                            self.live.push(l);
                        }
                    }
                    GroupObj::Origin(d) => {
                        let target = ayatsuri_core::edit::Target::Deformer(d);
                        let k = ayatsuri_core::keyform::keyform_at(puppet.model(), target, &values);
                        let local = puppet.rotation_local_origin(d);
                        if let (Some(k), Some(local)) = (k, local)
                            && let Some((x, y)) = puppet.rotation_origin_mut(d, k)
                        {
                            let stored = (*x, *y);
                            self.origins.push((d, k, stored, local));
                        }
                    }
                }
            }
        }
        let mut per: Vec<(GroupObj, Vec<(usize, Vec2)>)> = Vec::new();
        for &(i, p0) in from {
            let (Some(&(obj, vi)), Some(&p)) = (owner.get(i), pts.get(i)) else { continue };
            match per.iter_mut().find(|(o, _)| *o == obj) {
                Some((_, v)) => v.push((vi, p - p0)),
                None => per.push((obj, vec![(vi, p - p0)])),
            }
        }
        let shifts_of = |obj: GroupObj| {
            per.iter().find(|(o, _)| *o == obj).map(|(_, v)| v.as_slice()).unwrap_or(&[])
        };
        for l in &self.live {
            l.show(puppet, shifts_of(GroupObj::Live(l.target)), k_unit);
        }
        for &(d, k, (x, y), (lx, ly)) in &self.origins {
            let shift = shifts_of(GroupObj::Origin(d)).first().map(|s| s.1).unwrap_or(Vec2::ZERO);
            let parent = puppet.model().deformers.get(d).and_then(|x| x.parent_deformer);
            let (dx, dy) = parent_delta(puppet, parent, lx, ly, shift.x * k_unit, shift.y * k_unit);
            if let Some((ox, oy)) = puppet.rotation_origin_mut(d, k) {
                (*ox, *oy) = (x + dx, y + dy);
            }
        }
        true
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

/// A deform path point as skinning takes it: a triangle and the weights
/// of its corners.
type SkinPin = (usize, [f32; 3]);

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
/// Model Template's dialog: pick a template to apply (the built-in ones
/// first, then those saved in the templates folder), or name one to save
/// from this model. `after_import` words it as the start of a new model.
enum TemplateDlg {
    Apply { list: Vec<crate::templates::Template>, pick: usize, after_import: bool },
    Save { name: String, description: String },
}

impl TemplateDlg {
    fn apply(after_import: bool) -> TemplateDlg {
        let mut list = crate::templates::built_in();
        list.extend(crate::templates::saved().into_iter().map(|(_, t)| t));
        TemplateDlg::Apply { list, pick: 0, after_import }
    }
}

/// Cubism's New Part dialog as it is filled in.
struct NewPartDlg {
    name: String,
    id: String,
    order: f32,
    /// Put selected objects into children.
    take: bool,
}

enum MeshRowCmd {
    /// A click on the row; the palette applies Ctrl / Shift picking.
    Click(usize),
    Delete(usize),
    ClipBy(usize),
    /// Group into New Part: this mesh, or the pick it is in.
    Group(usize),
    Rename(usize),
    /// Flip this mesh (with the pick it is in), horizontally or not.
    Flip(usize, bool),
}

enum DefCmd {
    /// Create a warp (true) or rotation deformer around this object.
    Create(bool, Sel),
    CreateForSelection(bool),
    Rename(usize),
    RenameMesh(usize),
    /// Change this warp's conversion divisions.
    Division(usize),
    /// Flip this deformer (with the pick it is in): horizontally or not,
    /// and the warp alone (what it holds staying put) or with its content.
    Flip(usize, bool, bool),
    /// Flip this art mesh (with the pick it is in), horizontally or not.
    FlipMesh(usize, bool),
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
    /// Save As for the Cubism Editor project.
    ExportCmo3(Box<ExportDlg>),
    WorkspaceExport,
    WorkspaceImport,
    ImportMotion,
    /// A sound file added to the current composition as a layer.
    ImportAudio,
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
    /// Every point and face gone, to draw the mesh from scratch.
    Clear,
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

/// A menu button whose popup holds a text box: clicks inside (typing in
/// it) keep it open, a click outside or `ui.close()` shuts it.
fn search_menu<R>(ui: &mut Ui, label: &str, add: impl FnOnce(&mut Ui) -> R) -> egui::Response {
    use egui::containers::menu::{MenuButton, MenuConfig};
    MenuButton::new(label)
        .config(MenuConfig::new().close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside))
        .ui(ui, add)
        .0
}

/// Layers that played `old` play `new` from now on, so a renamed scene
/// or composition keeps its place in every composition.
fn rename_source(s: &mut crate::session::Session, old: &str, new: &str, comp: bool) {
    for c in &mut s.compositions {
        for l in &mut c.layers {
            let hit = match &l.source {
                ayatsuri_format::LayerSource::Scene(n) => !comp && n == old,
                ayatsuri_format::LayerSource::Composition(n) => comp && n == old,
                ayatsuri_format::LayerSource::Audio(_) => false,
            };
            if hit {
                l.source = if comp {
                    ayatsuri_format::LayerSource::Composition(new.to_string())
                } else {
                    ayatsuri_format::LayerSource::Scene(new.to_string())
                };
            }
        }
    }
}

/// The Animation details dialog: the current scene's or composition's
/// name, rate, length and loop, and frames to open up or take out.
struct AnimDetails {
    name: String,
    fps: f32,
    duration: f32,
    looped: bool,
    frames: i64,
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
    /// Drives blend shapes instead of keyforms (Cubism 5.0).
    blend_shape: bool,
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
        let mut prefs = Prefs::load();
        // tests never touch the saved preferences
        let whats_new = !cfg!(test) && crate::whats_new::first_start(&mut prefs);
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
            param_focus: false,
            log_view: log::Level::Info,
            pending_import,
            view_zoom: 1.0,
            view_pan: Vec2::ZERO,
            prefs,
            tool: Tool::Arrow,
            show_about: false,
            show_whats_new: whats_new,
            update_win: crate::whats_new::UpdateWindow::default(),
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
            skin_dlg: None,
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
            logged_open: 0,
            sent_title: String::new(),
            heartbeat: 0.0,
            anim_time: 0.0,
            anim_playing: false,
            space_dragged: false,
            expressions: crate::expressions::Expressions::default(),
            expr_base: Vec::new(),
            expr_shown: None,
            audio_out: None,
            audio_at: None,
            audio_devices: None,
            mixer: crate::mixer::Mixer::default(),
            anim_autokey: true,
            anim_applied: f32::NAN,
            anim_was_playing: false,
            play_pose: Vec::new(),
            timeline: crate::timeline::Timeline::default(),
            new_anim: None,
            anim_details: None,
            export_dlg: None,
            def_collapsed: std::collections::HashSet::new(),
            def_locked: std::collections::HashSet::new(),
            mesh_locked: std::collections::HashSet::new(),

            part_collapsed: std::collections::HashSet::new(),
            crash: None,
            last_autosave: std::time::Instant::now(),
            new_param: None,
            new_folder: None,
            new_part: None,
            template_dlg: None,
            folder_from: Vec::new(),
            new_model: None,
            show_model_info: false,
            gpu: false,
            theme_applied: false,
            logo: None,
            home_wash: None,
            show_ws_save: false,
            draw_spread: None,
            ws_name: String::new(),
            show_ws_manage: false,
            settings_tab: 0,
            settings_pos: None,
            frame_undo_depth: 0,
            frame_at: None,
            frame_dt: 1.0 / 60.0,
            anim_wanted: std::cell::Cell::new(false),
            anim_slot: None,
            def_rename: None,
            def_division: None,
            td_division: None,
            toolbar_reset: false,
            live_drag: None,
            live_group: LiveGroup::default(),
            dev_mesh_done: false,
            dev_def_done: false,
            dev_def_logged: false,
            mesh_art: None,
            stroke_edit: None,
            stroke_confirm: false,
            mesh_edit_was: None,
            dev_mesh_logged: false,
            rot_live: None,
            rot_handle: Default::default(),
            api: Default::default(),
            recovery: Default::default(),
            recovering: None,
            tool_opts: ToolOptions::default(),
            lasso: Vec::new(),
            sel_param: None,
            param_edit: None,
            pmenu: param_menu::ParamMenuState::default(),
            mirror_mark: None,
            deformer_dialog: None,
            multi_sel: Vec::new(),
            part_pick: Vec::new(),
            parts_dnd_off: false,
            highlight_children: false,
            highlight_overlap: false,
            atlas_view: false,
            part_anchor: None,
            def_pick: Vec::new(),
            def_anchor: None,
            def_row_anchor: None,
            palette_hover: None,
            part_search: String::new(),
            def_search: String::new(),
            mesh_anchor: None,
            clip_pick: None,
            clip_also: Vec::new(),
            pending_create: None,
            pending_guide: None,
            ctx_click: None,
            ctx_layers: Vec::new(),
            ctx_tree: Vec::new(),
            canvas_cmd: None,
            ctx_mesh_list: false,
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
            path_press: None,
            art_draft: None,
            art_grab: None,
            art_edit: None,
            art_style: ayatsuri_format::model::ArtPath::default(),
            art_finish: false,
            art_pick: None,
            color_pick: None,
            color_picked: None,
            clip_with: None,
            clip_search: String::new(),
            clip_ids: None,
            clip_group_name: String::new(),
            clip_invert: false,
            dbrush: None,
            tdeform: None,
            tdeform_cols: 3,
            tdeform_rows: 3,
            tdeform_paused_physics: false,
            tdeform_error: None,
            dbrush_hardness: 0.3,
            dbrush_strength: 0.7,
            pen: Default::default(),
            show_guides: true,
            mirror_edit: false,
            guide_drag: None,
            art_pending: None,
            pick_pages: std::collections::HashMap::new(),
            pick_alpha: AlphaPages::default(),
            last_pick: None,
            notice: None,
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

    /// Before egui reads this frame's input: a pen taken through the
    /// Wayland tablet protocol joins it as pointer events (see `pen`).
    pub fn pen_input(&mut self, ctx: &egui::Context, raw: &mut egui::RawInput) {
        self.pen.feed(ctx, raw);
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
            if let Err(e) = std::fs::remove_file(&p) {
                log::warn!("could not remove the crash copy {}: {e}", p.display());
            }
        }
        log::info!("closing normally");
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
            if let Err(e) = std::process::Command::new(opener).arg(&dir).spawn() {
                log::warn!("could not open the log folder with {opener}: {e}");
            }
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
                let from_art = path.extension().is_some_and(|e| {
                    matches!(e.to_string_lossy().to_lowercase().as_str(), "psd" | "kra" | "clip")
                });
                if self.recovering.as_ref() == Some(&path) {
                    // recovered work is unsaved work: no file of its own, not
                    // a recent; its autosave takes over from the copy
                    self.recovering = None;
                    s.aom_path = None;
                    s.modified = true;
                    if let Err(e) = std::fs::remove_file(&path) {
                        log::warn!(
                            "recovered, but the recovery copy {} stays: {e}",
                            path.display()
                        );
                    }
                    log::info!("recovered {}", s.name);
                } else {
                    self.prefs.remember(path);
                }
                {
                    let m = s.puppet.model();
                    log::info!(
                        "opened {}: {} meshes, {} deformers, {} parameters, {} parts, {} texture page(s), {} animation(s)",
                        s.name,
                        m.art_meshes.len(),
                        m.deformers.len(),
                        m.parameters.len(),
                        m.parts.len(),
                        s.texture_pngs.len(),
                        s.animations.len()
                    );
                }
                // a model straight from artwork has no parameters yet:
                // offer a template to start from
                let fresh =
                    s.puppet.model().parameters.is_empty() && s.aom_path.is_none() && from_art;
                self.session = Some(s);
                self.selected = None;
                self.view_zoom = 1.0;
                self.view_pan = Vec2::ZERO;
                if fresh {
                    self.template_dlg = Some(TemplateDlg::apply(true));
                }
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

    /// Log the windows, dialogs and modes that opened or closed since last
    /// frame. One comparison a frame and a line only on a change, so the
    /// log says what the user did without a line per frame.
    fn log_state_changes(&mut self) {
        let states: [(&str, bool); 28] = [
            ("Settings", self.show_settings),
            ("API window", self.show_api_window),
            ("Model Statistics", self.show_stats),
            ("Timelapse", self.timelapse_view.is_some()),
            ("Render", self.render_win.is_some()),
            ("Texture Atlas", self.atlas_win.is_some()),
            ("Skinning", self.skin_dlg.is_some()),
            ("Physics Settings", self.show_physics),
            ("New Animation", self.new_anim.is_some()),
            ("Animation Details", self.anim_details.is_some()),
            ("Export Model", self.export_dlg.is_some()),
            ("New Parameter", self.new_param.is_some()),
            ("New Folder", self.new_folder.is_some()),
            ("New Part", self.new_part.is_some()),
            ("Model Template", self.template_dlg.is_some()),
            ("New Project", self.new_model.is_some()),
            ("Project", self.show_model_info),
            ("Save Workspace", self.show_ws_save),
            ("Manage Workspaces", self.show_ws_manage),
            ("Guide Line Settings", self.show_guide_settings),
            ("New Deformer", self.deformer_dialog.is_some()),
            ("Edit Parameter", self.param_edit.is_some()),
            ("Mesh Edit", self.mesh_edit.is_some()),
            ("Physics", self.physics_on),
            ("Follow", self.follow_cursor),
            ("Temporary Deform Tool", self.tdeform.is_some()),
            ("Playlist playback", self.phys_page.pl_playing),
            ("Mirror Edit", self.mirror_edit),
        ];
        let mut now = 0u64;
        for (i, &(_, on)) in states.iter().enumerate() {
            if on {
                now |= 1 << i;
            }
        }
        let changed = now ^ self.logged_open;
        if changed == 0 {
            return;
        }
        // the last six are modes turned on and off, the rest windows
        let modes = states.len() - 6;
        for (i, &(name, on)) in states.iter().enumerate() {
            if changed & (1 << i) == 0 {
                continue;
            }
            match (i >= modes, on) {
                (true, true) => log::info!("{name}: on"),
                (true, false) => log::info!("{name}: off"),
                (false, true) => log::info!("{name}: opened"),
                (false, false) => log::debug!("{name}: closed"),
            }
        }
        self.logged_open = now;
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
                log::info!("saved {}", path.display());
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
            if let PickFor::ExportMoc3(dlg) | PickFor::ExportCmo3(dlg) = why {
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
            (PickFor::ExportCmo3(dlg), Some(path)) => self.export_cmo3_to(&dlg, &path),
            (PickFor::ExportCmo3(dlg), None) => self.export_dlg = Some(*dlg),
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
            (PickFor::ImportAudio, Some(path)) => self.import_audio(&path),
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

    /// Add a sound file as a layer of the current composition, at the
    /// playhead; with no composition open, a new one named after the file,
    /// as long as the sound, takes it.
    fn import_audio(&mut self, path: &Path) {
        let Some(s) = &mut self.session else { return };
        let rel = s.audio_rel(path);
        if let Err(e) = s.load_audio(&rel) {
            log::error!("import audio {}: {e}", path.display());
            return;
        }
        let len = s.audio.get(&rel).map_or(1.0, |c| c.len()).max(1.0 / 30.0);
        let stem = path.file_stem().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default();
        let ci = match s.current_comp.filter(|ci| *ci < s.compositions.len()) {
            Some(ci) => ci,
            None => {
                let fps = s.current_anim.and_then(|i| s.animations.get(i)).map_or(30.0, |a| a.fps);
                s.compositions.push(ayatsuri_format::Composition {
                    name: if stem.is_empty() { "comp".into() } else { stem.clone() },
                    fps,
                    duration: len,
                    looped: false,
                    ..ayatsuri_format::Composition::default()
                });
                let ci = s.compositions.len() - 1;
                s.current_comp = Some(ci);
                self.anim_time = 0.0;
                ci
            }
        };
        let at = self.anim_time.max(0.0);
        if let Some(c) = s.compositions.get(ci).cloned() {
            self.timeline.remember_comp(ci, &c, None);
        }
        if let Some(c) = s.compositions.get_mut(ci) {
            c.layers.insert(
                0,
                ayatsuri_format::Layer {
                    source: ayatsuri_format::LayerSource::Audio(rel.clone()),
                    start: at.min(c.duration),
                    out_point: len,
                    ..ayatsuri_format::Layer::default()
                },
            );
            if c.duration < at + len {
                c.duration = at + len;
            }
        }
        s.modified = true;
        self.anim_applied = f32::NAN;
        if !self.layout.contains(PanelKind::Timeline) {
            self.layout.open_near(PanelKind::Timeline, PanelKind::Viewport);
        }
        log::info!("audio {rel} added to the composition");
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
        match &w {
            Workspace::Modeling => log::info!("workspace: Modeling"),
            Workspace::Animation => log::info!("workspace: Animation"),
            Workspace::Custom(i) => match self.prefs.custom_workspaces.get(*i) {
                Some((name, _)) => log::info!("workspace: {name}"),
                None => log::warn!("workspace {i} is gone; using the default layout"),
            },
        }
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
        // the Animation workspace opens on the first composition, the
        // whole piece, and falls back to the first scene when there is none
        if w == Workspace::Animation
            && let Some(s) = &mut self.session
            && s.current_comp.is_none()
            && !s.compositions.is_empty()
        {
            s.current_comp = Some(0);
            self.anim_time = 0.0;
            self.anim_applied = f32::NAN;
        }
        self.workspace = w;
        self.last_layout = self.layout.clone();
    }

    fn reset_workspace(&mut self) {
        log::info!("workspace layout reset");
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

    /// Hand the GPU the full texture pages residency let go this frame.
    /// The app calls it after every frame, painted or not, so a minimised
    /// or idle window gives its video memory back.
    pub fn apply_gpu_drops(&mut self, render: Option<&egui_wgpu::RenderState>) {
        let Some(s) = &mut self.session else { return };
        if s.gpu_drop.is_empty() {
            return;
        }
        let Some(render) = render else { return };
        let pages = std::mem::take(&mut s.gpu_drop);
        let mut r = render.renderer.write();
        if let Some(gpu) = r.callback_resources.get_mut::<ayatsuri_render::GpuRenderer>() {
            gpu.drop_full_pages(&pages);
            log::debug!("{} full texture page(s) given back", pages.len());
        }
    }

    pub fn ui(&mut self, ui: &mut Ui) {
        self.limit_frame();
        // the real time since the last frame; after a pause (the editor was
        // asleep) one ordinary frame, so play and physics pick up smoothly
        let now = std::time::Instant::now();
        let gap = self.frame_at.map_or(0.0, |t| now.duration_since(t).as_secs_f32());
        self.frame_dt = if gap > 0.0 && gap <= 0.1 { gap } else { 1.0 / 60.0 };
        self.frame_at = Some(now);
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
        if std::mem::take(&mut self.stroke_confirm) {
            self.apply_stroke_mesh();
        }
        if self.mesh_edit.is_some() {
            self.mesh_edit_was = self.mesh_edit;
        } else if self.mesh_edit_was.take().is_some() && self.prefs.atlas_repack_auto {
            self.repack_if_outgrown(&ui.ctx().clone());
        }
        self.def_division_window(&ui.ctx().clone());
        self.whats_new_window(&ui.ctx().clone());
        self.temp_deform_error_window(&ui.ctx().clone());
        self.notice_area(&ui.ctx().clone());
        self.blend_edit_label(&ui.ctx().clone());
        self.update_window(&ui.ctx().clone());
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
        self.log_state_changes();
        if self.tool != self.logged_tool {
            log::info!("tool: {}", self.tool.label());
            self.logged_tool = self.tool;
        }

        // the title carries the unsaved marker, like every other editor
        {
            let title = match &self.session {
                Some(s) if s.modified => format!("Ayatsuri2D · {} *", s.name),
                Some(s) => format!("Ayatsuri2D · {}", s.name),
                None => "Ayatsuri2D".into(),
            };
            if title != self.sent_title {
                ui.ctx().send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
                self.sent_title = title;
            }
        }
        // closing with unsaved work stops at a dialog first
        if ui.ctx().input(|i| i.viewport().close_requested())
            && !self.force_close
            && self.session.as_ref().is_some_and(|s| s.modified)
        {
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.confirm_close = Some(1);
        }
        self.drop_stale_selection();
        if let Some(c) = self.canvas_cmd.take() {
            self.run_canvas_cmd(&ui.ctx().clone(), c);
        }
        self.pen.read(ui.ctx());
        crate::pen::apply_click_options(ui.ctx(), self.prefs.pen_mode);
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
            // with the Timeline focused its actions go first, so B keys the
            // work area there and picks the brush tool anywhere else
            let anim_first = self.workspace == Workspace::Animation && self.timeline.focused;
            bound.sort_by_key(|(id, sc)| {
                (std::cmp::Reverse(weight(sc.modifiers)), !(anim_first && id.starts_with("anim-")))
            });
            for (id, sc) in bound {
                if typing && weight(sc.modifiers) == 0 {
                    continue;
                }
                // S keys the parameter in Animation; Solo is a modeling view
                if matches!(id, "solo" | "highlight-children")
                    && self.workspace == Workspace::Animation
                {
                    continue;
                }
                if ui.ctx().input_mut(|i| i.consume_shortcut(&sc)) {
                    fired.push(id);
                }
            }
            // a tap on Space plays or pauses, as in After Effects; held with
            // a drag it pans the view instead, so the tap counts on release
            let (space_up, any_down) =
                ui.ctx().input(|i| (i.key_released(egui::Key::Space), i.pointer.any_down()));
            if space_up {
                if !self.space_dragged
                    && !any_down
                    && !typing
                    && self.workspace == Workspace::Animation
                {
                    fired.push("anim-play");
                }
                self.space_dragged = false;
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
        let timeline_has_keys = self.workspace == Workspace::Animation && self.timeline.focused;
        // a press anywhere ends the parameter list's hold on Delete; a
        // click on a parameter takes it again when it is released
        if ui.ctx().input(|i| i.pointer.primary_pressed()) {
            self.param_focus = false;
        }
        if let Some(gi) = self.selected_glue
            && self.mesh_edit.is_none()
            && !timeline_has_keys
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
            && !timeline_has_keys
            && self.tool == Tool::Arrow
            && self.pts.picked.is_empty()
            && !ui.ctx().egui_wants_keyboard_input()
            && !self.param_focus
            && let Some(Sel::Mesh(mi)) = self.selected
            && ui.ctx().input(|i| i.key_pressed(egui::Key::Delete) && !i.modifiers.command)
        {
            let meshes = self.mesh_pick_with(mi);
            self.delete_meshes(meshes);
        } else if self.selected_glue.is_none()
            && self.mesh_edit.is_none()
            && !timeline_has_keys
            && self.tool == Tool::Arrow
            && !ui.ctx().egui_wants_keyboard_input()
            && !self.param_focus
            && let Some(Sel::Part(p)) = self.selected
            && ui.ctx().input(|i| {
                (i.key_pressed(egui::Key::Delete) || i.key_pressed(egui::Key::Backspace))
                    && !i.modifiers.command
            })
        {
            let parts = if self.part_pick.contains(&p) { self.part_pick.clone() } else { vec![p] };
            self.delete_parts(parts, false);
        }
        // Cut, Copy and Paste for whole art meshes; Mesh Edit has its own,
        // for points. Ctrl+Shift+C stays Copy Form.
        if self.mesh_edit.is_none() && !timeline_has_keys && !ui.ctx().egui_wants_keyboard_input() {
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
                            egui::Button::new(crate::i18n::tr("New Animation…")).shortcut_text(self.sc("new-animation")),
                        )
                        .clicked()
                    {
                        ui.close();
                        self.open_new_anim(false);
                    }
                    if ui
                        .add_enabled(
                            self.session.is_some(),
                            egui::Button::new(crate::i18n::tr("New Composition…")).shortcut_text(self.sc("new-composition")),
                        )
                        .on_hover_text(crate::i18n::tr(
                            "Scenes and other compositions laid out as layers over one clock.",
                        ))
                        .clicked()
                    {
                        ui.close();
                        self.open_new_anim(true);
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
                            egui::Button::new(crate::i18n::tr("Export Animation…")).shortcut_text(self.sc("export-animation")),
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
                            egui::Button::new(crate::i18n::tr("Export Timelapse…")).shortcut_text(self.sc("export-timelapse")),
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
                            egui::Button::new(crate::i18n::tr("Import Motion…")).shortcut_text(self.sc("import-motion")),
                        )
                        .on_hover_text(crate::i18n::tr("Add a .motion3.json as an animation"))
                        .clicked()
                    {
                        ui.close();
                        self.pick_motion();
                    }
                    if ui
                        .add_enabled(
                            self.session.is_some(),
                            egui::Button::new(crate::i18n::tr("Import Audio…")).shortcut_text(self.sc("import-audio")),
                        )
                        .on_hover_text(crate::i18n::tr(
                            "Add a WAV, MP3, FLAC or Ogg file as a layer of the current composition (a new one when there is none)",
                        ))
                        .clicked()
                    {
                        ui.close();
                        self.pick_audio();
                    }
                    if ui
                        .add_enabled(
                            self.session.is_some(),
                            egui::Button::new(crate::i18n::tr("Import Artwork…")).shortcut_text(self.sc("import-artwork")),
                        )
                        .on_hover_text(crate::i18n::tr(
                            "Add a Photoshop, Krita or Clip Studio file's layers to this model \
                             as new parts and meshes",
                        ))
                        .clicked()
                    {
                        ui.close();
                        self.pick_artwork(false);
                    }
                    if ui
                        .add_enabled(
                            self.session.is_some(),
                            egui::Button::new(crate::i18n::tr("Re-import Artwork…")).shortcut_text(self.sc("reimport-artwork")),
                        )
                        .on_hover_text(crate::i18n::tr(
                            "Swap in updated art from the file this model was made from: layers \
                             match meshes by folder and name, and the rigging stays as it is",
                        ))
                        .clicked()
                    {
                        ui.close();
                        self.pick_artwork(true);
                    }
                    ui.separator();
                    if ui
                        .add_enabled(
                            self.session.is_some(),
                            egui::Button::new(crate::i18n::tr("Project…")).shortcut_text(self.sc("project-info")),
                        )
                        .clicked()
                    {
                        self.show_model_info = true;
                        ui.close();
                    }
                    if ui
                        .add_enabled(
                            self.session.is_some(),
                            egui::Button::new(crate::i18n::tr("Model Statistics…")).shortcut_text(self.sc("model-stats")),
                        )
                        .clicked()
                    {
                        self.show_stats = true;
                        ui.close();
                    }
                    ui.separator();
                    if ui.add(egui::Button::new(crate::i18n::tr("Environment Settings…")).shortcut_text(self.sc("settings"))).clicked() {
                        self.show_settings = true;
                        ui.close();
                    }
                    if ui.add(egui::Button::new(crate::i18n::tr("Keyboard Shortcuts…")).shortcut_text(self.sc("shortcuts"))).clicked() {
                        self.show_settings = true;
                        self.settings_tab = crate::windows::settings::TAB_KEYBOARD;
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
                    if ui.add(egui::Button::new(crate::i18n::tr("Exit")).shortcut_text(self.sc("exit"))).clicked() {
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
                        .add_enabled(can_undo, egui::Button::new(crate::i18n::tr("Undo")).shortcut_text(self.sc("undo")))
                        .clicked()
                    {
                        ui.close();
                        if let Some(s) = &mut self.session {
                            s.undo();
                        }
                    }
                    if ui
                        .add_enabled(can_redo, egui::Button::new(crate::i18n::tr("Redo")).shortcut_text(self.sc("redo")))
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
                    if ui.add_enabled(has, egui::Button::new(crate::i18n::tr("Zoom In")).shortcut_text(self.sc("zoom-in"))).clicked()
                    {
                        self.view_zoom = (self.view_zoom * 1.25).clamp(ZOOM_MIN, ZOOM_MAX);
                        ui.close();
                    }
                    if ui.add_enabled(has, egui::Button::new(crate::i18n::tr("Zoom Out")).shortcut_text(self.sc("zoom-out"))).clicked()
                    {
                        self.view_zoom = (self.view_zoom / 1.25).clamp(ZOOM_MIN, ZOOM_MAX);
                        ui.close();
                    }
                    if ui.add_enabled(has, egui::Button::new(crate::i18n::tr("Fit")).shortcut_text(self.sc("display-all"))).clicked() {
                        self.view_zoom = 1.0;
                        self.view_pan = Vec2::ZERO;
                        ui.close();
                    }
                    ui.separator();
                    let solo_on = self.session.as_ref().is_some_and(|s| s.solo.is_some());
                    if ui
                        .add_enabled(has, egui::Button::selectable(solo_on, crate::i18n::tr("Solo")))
                        .on_hover_text(crate::i18n::tr(
                            "Show the selected objects in front and keep the rest from being \
                             picked (S)",
                        ))
                        .clicked()
                    {
                        self.toggle_solo();
                        ui.close();
                    }
                    ui.checkbox(
                        &mut self.highlight_children,
                        crate::i18n::tr("Highlight Deformer's Child Elements"),
                    )
                    .on_hover_text(crate::i18n::tr(
                        "Outline every art mesh under the selected deformer (Ctrl+Shift+D)",
                    ));
                    ui.checkbox(
                        &mut self.highlight_overlap,
                        crate::i18n::tr("Highlight Overlapping Polygons"),
                    )
                    .on_hover_text(crate::i18n::tr(
                        "Mark triangles that cover others of the same mesh, in meshes with an \
                         advanced blend mode: they blend twice there and flicker as it moves",
                    ));
                    ui.add(
                        egui::Checkbox::new(
                            &mut self.atlas_view,
                            crate::i18n::tr("Texture Atlas View"),
                        ),
                    )
                    .on_hover_text(crate::i18n::tr(
                        "Show every pixel each mesh covers on the texture atlas, as the export will: a mesh grown over its neighbour shows that neighbour (T)",
                    ));
                    ui.checkbox(&mut self.mirror_edit, crate::i18n::tr("Mirror Edit"))
                        .on_hover_text(crate::i18n::tr(
                            "Moving a point also moves its mirror across the vertical axis: \
                             the vertical guide line nearest the centre, or the canvas centre",
                        ));
                    ui.menu_button(crate::i18n::tr("Guide Line"), |ui| {
                        ui.checkbox(&mut self.show_guides, crate::i18n::tr("Show Guide Lines"));
                        if ui
                            .add_enabled(has, egui::Button::new(crate::i18n::tr("Add Guide Lines in the Center")).shortcut_text(self.sc("guides-center")))
                            .clicked()
                        {
                            self.pending_guide = Some(GuideCmd::Center);
                            ui.close();
                        }
                        if ui
                            .add_enabled(
                                has,
                                egui::Button::new(crate::i18n::tr(
                                    "Add a Guide Line at the Selected Vertex",
                                )).shortcut_text(self.sc("guide-at-vertex")),
                            )
                            .clicked()
                        {
                            self.pending_guide = Some(GuideCmd::AtVertex);
                            ui.close();
                        }
                        if ui
                            .add_enabled(has, egui::Button::new(crate::i18n::tr("Guide Line settings…")).shortcut_text(self.sc("guide-settings")))
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
                        if ui
                            .checkbox(&mut self.prefs.smooth_drag, crate::i18n::tr("Smooth movement"))
                            .on_hover_text(crate::i18n::tr(
                                "Points and objects follow the pointer softly while dragged; they land where you let go",
                            ))
                            .changed()
                        {
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
                        if ui.add_enabled(any, egui::Button::new(crate::i18n::tr("Clear guide lines")).shortcut_text(self.sc("guides-clear"))).clicked() {
                            if let Some(s) = &mut self.session {
                                s.guides.clear();
                                log::info!("guide lines cleared");
                                s.modified = true;
                            }
                            ui.close();
                        }
                    });
                });
                ui.menu_button(crate::i18n::tr("Modeling"), |ui| {
                    if ui.add(egui::Button::new(crate::i18n::tr("Modeling Workspace")).shortcut_text(self.sc("workspace-modeling"))).clicked() {
                        self.set_workspace(Workspace::Modeling);
                        ui.close();
                    }
                    ui.separator();
                    let has_sel = !self.selection_meshes().is_empty();
                    ui.menu_button(crate::i18n::tr("Deformer"), |ui| {
                        if ui
                            .add_enabled(
                                has_sel,
                                egui::Button::new(crate::i18n::tr("Create Warp Deformer")).shortcut_text(self.sc("create-warp")),
                            )
                            .clicked()
                        {
                            self.pending_create = Some((true, None));
                            ui.close();
                        }
                        if ui
                            .add_enabled(
                                has_sel,
                                egui::Button::new(crate::i18n::tr("Create Rotation Deformer")).shortcut_text(self.sc("create-rotation")),
                            )
                            .clicked()
                        {
                            self.pending_create = Some((false, None));
                            ui.close();
                        }
                    });
                    let skin_ready = self.skin_path().is_some();
                    let mesh_picked = matches!(self.selected, Some(Sel::Mesh(_)));
                    ui.menu_button(crate::i18n::tr("Skinning"), |ui| {
                        if ui
                            .add_enabled(mesh_picked, egui::Button::new(crate::i18n::tr("Skinning…")).shortcut_text(self.sc("skinning")))
                            .on_disabled_hover_text(crate::i18n::tr(
                                "Select the mesh, with rotation deformers placed along it",
                            ))
                            .clicked()
                        {
                            ui.close();
                            self.open_skin_dialog(false);
                        }
                        if ui.add(egui::Button::new(crate::i18n::tr("Generate Parameters for Rotation Deformers…")).shortcut_text(self.sc("skin-params")))
                            .clicked()
                        {
                            ui.close();
                            self.open_skin_dialog(true);
                        }
                        ui.separator();
                        if ui
                            .add_enabled(
                                skin_ready,
                                egui::Button::new(crate::i18n::tr("Use Deform Path for Skinning")).shortcut_text(self.sc("skin-path")),
                            )
                            .on_disabled_hover_text(crate::i18n::tr(
                                "Select a mesh and draw a deform path on it from root to tip (Deform Path tool, edit level 2 or 3)",
                            ))
                            .clicked()
                        {
                            ui.close();
                            self.apply_skin();
                        }
                    });
                    let can_keys = self.edit_target().is_some() && self.sel_param.is_some();
                    ui.menu_button(crate::i18n::tr("Parameter"), |ui| {
                        if ui
                            .add_enabled(
                                can_keys,
                                egui::Button::new(crate::i18n::tr("Add Keys at Min and Max")).shortcut_text(self.sc("keys-min-max")),
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
                                )).shortcut_text(self.sc("keys-min-default-max")),
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
                                egui::Button::new(crate::i18n::tr("Delete Nearest Key")).shortcut_text(self.sc("key-delete-nearest")),
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
                                egui::Button::new(crate::i18n::tr("Motion Mirroring…"))
                                    .shortcut_text(self.sc("motion-mirror")),
                            )
                            .on_disabled_hover_text(crate::i18n::tr("Select a parameter first"))
                            .clicked()
                            && let Some(pi) = self.sel_param
                        {
                            ui.close();
                            self.run_param_cmd(ParamCmd::Mirror(pi));
                        }
                        if ui
                            .add_enabled(
                                self.session.is_some() && self.sel_param.is_some(),
                                egui::Button::new(crate::i18n::tr("Extended Interpolation…")).shortcut_text(self.sc("extended-interpolation")),
                            )
                            .on_disabled_hover_text(crate::i18n::tr("Select a parameter first"))
                            .clicked()
                            && let Some(pi) = self.sel_param
                        {
                            ui.close();
                            self.run_param_cmd(ParamCmd::Curve(pi));
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
                                egui::Button::new(crate::i18n::tr("Auto Mesh Generation")).shortcut_text(self.sc("auto-mesh")),
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
                        ui.separator();
                        if ui
                            .add_enabled(
                                self.session.is_some(),
                                egui::Button::new(crate::i18n::tr(
                                    "Change the draw order of all ArtMeshes…",
                                )),
                            )
                            .clicked()
                        {
                            ui.close();
                            self.draw_spread = Some((true, 10.0));
                        }
                    });
                    if ui
                        .add_enabled(
                            self.session.is_some(),
                            egui::Button::new(crate::i18n::tr("Texture Atlas…")).shortcut_text(self.sc("texture-atlas")),
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
                                egui::Button::new(crate::i18n::tr("Create ArtPath")).shortcut_text(self.sc("artpath-create")),
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
                    ui.menu_button(crate::i18n::tr("Edit Form"), |ui| {
                        let flippable = self.edit_target().is_some();
                        for (label, horizontal, id) in
                            [("Flip Horizontally", true, "flip-h"), ("Flip Vertically", false, "flip-v")]
                        {
                            if ui
                                .add_enabled(
                                    flippable,
                                    egui::Button::new(crate::i18n::tr(label))
                                        .shortcut_text(self.sc(id)),
                                )
                                .clicked()
                            {
                                ui.close();
                                self.flip_selection(horizontal);
                            }
                        }
                        // the warp alone, what it holds staying put (Ctrl)
                        let warp_sel = self.session.as_ref().is_some_and(|s| {
                            def_group(self.selected, &self.def_pick).iter().any(|&d| {
                                s.puppet.warp_grid(d).is_some()
                            })
                        });
                        for (label, horizontal, id) in [
                            ("Flip Warp Only Horizontally", true, "flip-h-only"),
                            ("Flip Warp Only Vertically", false, "flip-v-only"),
                        ] {
                            if ui
                                .add_enabled(
                                    warp_sel,
                                    egui::Button::new(crate::i18n::tr(label))
                                        .shortcut_text(self.sc(id)),
                                )
                                .on_hover_text(crate::i18n::tr(
                                    "Mirror the warp's grid while the art and deformers inside stay where they are",
                                ))
                                .clicked()
                            {
                                ui.close();
                                self.flip_warps_only(horizontal);
                            }
                        }
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
                    });
                    let can_temp = self.can_temp_deform();
                    ui.menu_button(crate::i18n::tr("Temporary Deform Tool"), |ui| {
                        for m in temp_deform::Method::ALL {
                            if m == temp_deform::Method::Warp {
                                ui.separator();
                            }
                            let id = match m {
                                temp_deform::Method::Zoom => "temp-zoom",
                                temp_deform::Method::Rotation => "temp-rotation",
                                temp_deform::Method::Free => "temp-free",
                                temp_deform::Method::Perspective => "temp-perspective",
                                temp_deform::Method::Distort => "temp-distort",
                                temp_deform::Method::Warp => "temp-warp",
                                temp_deform::Method::Path => "temp-path",
                            };
                            if ui
                                .add_enabled(
                                    can_temp,
                                    egui::Button::new(crate::i18n::tr(m.label()))
                                        .shortcut_text(self.sc(id)),
                                )
                                .clicked()
                            {
                                ui.close();
                                self.start_temp_deform(m);
                            }
                        }
                        ui.separator();
                        if ui
                            .add_enabled(
                                self.tdeform.is_some(),
                                egui::Button::new(crate::i18n::tr(
                                    "Deselect the Temporary Deform Tool",
                                ))
                                .shortcut_text(self.sc("temp-deselect")),
                            )
                            .clicked()
                        {
                            ui.close();
                            self.end_temp_deform();
                        }
                    });
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
                            egui::Button::new(crate::i18n::tr("Physics Settings…")).shortcut_text(self.sc("physics")),
                        )
                        .clicked()
                    {
                        self.show_physics = true;
                        ui.close();
                    }
                    ui.separator();
                    ui.add_enabled_ui(self.session.is_some(), |ui| {
                        ui.menu_button(crate::i18n::tr("Model Template"), |ui| {
                            if ui.add(egui::Button::new(crate::i18n::tr("Apply Parameter Template…")).shortcut_text(self.sc("template-apply"))).clicked() {
                                self.template_dlg = Some(TemplateDlg::apply(false));
                                ui.close();
                            }
                            if ui.add(egui::Button::new(crate::i18n::tr("Save Parameters as Template…")).shortcut_text(self.sc("template-save")))
                                .clicked()
                            {
                                self.template_dlg = Some(TemplateDlg::Save {
                                    name: String::new(),
                                    description: String::new(),
                                });
                                ui.close();
                            }
                        });
                    });
                });
                ui.menu_button(crate::i18n::tr("Animation"), |ui| {
                    if ui.add(egui::Button::new(crate::i18n::tr("Animation Workspace")).shortcut_text(self.sc("workspace-animation"))).clicked() {
                        self.set_workspace(Workspace::Animation);
                        ui.close();
                    }
                    ui.separator();
                    if ui
                        .add_enabled(
                            self.session.is_some(),
                            egui::Button::new(crate::i18n::tr("New Animation…")).shortcut_text(self.sc("new-animation")),
                        )
                        .clicked()
                    {
                        ui.close();
                        self.open_new_anim(false);
                    }
                    if ui
                        .add_enabled(
                            self.session.is_some(),
                            egui::Button::new(crate::i18n::tr("New Composition…")).shortcut_text(self.sc("new-composition")),
                        )
                        .on_hover_text(crate::i18n::tr(
                            "Scenes and other compositions laid out as layers over one clock.",
                        ))
                        .clicked()
                    {
                        ui.close();
                        self.open_new_anim(true);
                    }
                    let has_anim = self.session.as_ref().is_some_and(|s| s.current_anim.is_some());
                    let current = self.session.as_ref().and_then(|s| {
                        if let Some(c) = s.current_comp.and_then(|i| s.compositions.get(i)) {
                            Some((c.name.clone(), c.fps, c.duration, c.looped))
                        } else {
                            s.current_anim
                                .and_then(|i| s.animations.get(i))
                                .map(|a| (a.name.clone(), a.fps, a.duration, a.looped))
                        }
                    });
                    if ui
                        .add_enabled(
                            current.is_some(),
                            egui::Button::new(crate::i18n::tr("Edit Animation Details…")).shortcut_text(self.sc("animation-details")),
                        )
                        .on_hover_text(crate::i18n::tr(
                            "Name, frame rate, length in seconds and loop; insert or remove frames at the playhead.",
                        ))
                        .clicked()
                    {
                        ui.close();
                        self.open_anim_details();
                    }
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
                        .add_enabled(self.session.is_some(), egui::Button::new(crate::i18n::tr("Timelapse…")).shortcut_text(self.sc("timelapse")))
                        .clicked()
                    {
                        self.open_timelapse();
                        ui.close();
                    }
                    if ui.add(egui::Button::new(crate::i18n::tr("Settings…")).shortcut_text(self.sc("settings"))).clicked() {
                        self.show_settings = true;
                        ui.close();
                    }
                    ui.separator();
                    ui.menu_button(crate::i18n::tr("Workspace"), |ui| {
                        if ui.add(egui::Button::new(crate::i18n::tr("Save as New Workspace…")).shortcut_text(self.sc("workspace-save"))).clicked() {
                            self.show_ws_save = true;
                            ui.close();
                        }
                        if ui.add(egui::Button::new(crate::i18n::tr("Manage Workspaces…")).shortcut_text(self.sc("workspace-manage"))).clicked() {
                            self.show_ws_manage = true;
                            ui.close();
                        }
                        ui.separator();
                        if ui.add(egui::Button::new(crate::i18n::tr("Export Current Layout…")).shortcut_text(self.sc("layout-export"))).clicked() {
                            ui.close();
                            self.pick_layout(true);
                        }
                        if ui.add(egui::Button::new(crate::i18n::tr("Import Layout…")).shortcut_text(self.sc("layout-import"))).clicked() {
                            ui.close();
                            self.pick_layout(false);
                        }
                    });
                    if ui.add(egui::Button::new(crate::i18n::tr("Reset Current Layout")).shortcut_text(self.sc("layout-reset"))).clicked() {
                        self.reset_workspace();
                        ui.close();
                    }
                });
                ui.menu_button(crate::i18n::tr("API"), |ui| self.api_menu(ui));
                ui.menu_button(crate::i18n::tr("Help"), |ui| {
                    if ui.button(crate::i18n::tr("What's New")).clicked() {
                        self.show_whats_new = true;
                        ui.close();
                    }
                    if ui.button(crate::i18n::tr("Check for Updates")).clicked() {
                        self.check_updates();
                        ui.close();
                    }
                    if ui.button(crate::i18n::tr("Join the Discord")).clicked() {
                        ui.ctx().open_url(egui::OpenUrl::new_tab(crate::whats_new::DISCORD));
                        ui.close();
                    }
                    if ui.button(crate::i18n::tr("Support Ayatsuri2D (optional)")).clicked() {
                        ui.ctx().open_url(egui::OpenUrl::new_tab(crate::windows::about::SUPPORT));
                        ui.close();
                    }
                    ui.separator();
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
                // the workspace dropdown: a flat menu button like the menu
                // bar's File and Edit, its entries menu items, the active
                // one in the accent colour
                let mut pick: Option<Workspace> = None;
                let mut entries: Vec<(Workspace, String)> = vec![
                    (Workspace::Modeling, crate::i18n::tr("Modeling").to_string()),
                    (Workspace::Animation, crate::i18n::tr("Animation").to_string()),
                ];
                for (i, (name, _)) in self.prefs.custom_workspaces.iter().enumerate() {
                    entries.push((Workspace::Custom(i), name.clone()));
                }
                let label = entries
                    .iter()
                    .find(|(w, _)| *w == self.workspace)
                    .map(|(_, n)| n.clone())
                    .unwrap_or_else(|| crate::i18n::tr("Custom").to_string());
                let title = format!("{label}  {}", egui_phosphor::regular::CARET_DOWN);
                ui.menu_button(title, |ui| {
                    for (w, name) in &entries {
                        let active = self.workspace == *w;
                        let text = if active {
                            egui::RichText::new(name).color(theme::pal().accent)
                        } else {
                            egui::RichText::new(name)
                        };
                        if ui.button(text).clicked() {
                            pick = Some(w.clone());
                        }
                    }
                    ui.separator();
                    if ui.button(crate::i18n::tr("Save as New Workspace…")).clicked() {
                        self.show_ws_save = true;
                    }
                    if ui.button(crate::i18n::tr("Manage Workspaces…")).clicked() {
                        self.show_ws_manage = true;
                    }
                    if pick.is_some() || self.show_ws_save || self.show_ws_manage {
                        ui.close();
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
                            physics_to_rest(s);
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
            // play remembers the pose; stop (from anywhere: the transport, a
            // key, the menu, the end of a clip) puts it back, so the model
            // does not stay on the last frame with its eyes shut
            if self.anim_playing && !self.anim_was_playing {
                self.play_pose =
                    (0..s.puppet.model().parameters.len()).map(|i| s.puppet.value(i)).collect();
                log::debug!("playback: started, pose of {} parameters kept", self.play_pose.len());
            } else if !self.anim_playing && self.anim_was_playing {
                for (i, &v) in self.play_pose.iter().enumerate() {
                    s.puppet.set_value(i, v);
                }
                // the clip must not write the stopped frame back over it
                self.anim_applied = self.anim_time;
                s.dirty = true;
                log::debug!("playback: stopped, pose restored");
            }
            self.anim_was_playing = self.anim_playing;
            let mut posed = false;
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
                let dt = self.frame_dt;
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
                    // paced like physics, so the FPS cap holds during playback
                    pace_anim(&self.anim_wanted, ui.ctx());
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
                    posed = true;
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

            // a composition's sound layers play along: started at the
            // playhead when play begins or jumps, stopped with it
            let want = comp
                .as_ref()
                .filter(|_| self.anim_playing)
                .map(|c| crate::audio::comp_cues(c, &s.audio, self.anim_time));
            match want {
                Some((cues, live)) => {
                    let dt = self.frame_dt;
                    let jumped =
                        self.audio_at.is_none_or(|at| (self.anim_time - (at + dt)).abs() > 0.12);
                    if jumped {
                        if self.audio_out.as_ref().is_none_or(|o| !o.alive()) {
                            self.audio_out = crate::audio::Out::open(&self.prefs.audio());
                        }
                        if let Some(out) = &mut self.audio_out {
                            out.start(cues);
                        }
                    }
                    if let Some(out) = &self.audio_out {
                        out.sync(&live, &self.mixer.solo);
                    }
                    self.audio_at = Some(self.anim_time);
                }
                None => {
                    // playback stopped: close the output, so no thread keeps
                    // streaming silence; the next play opens it again
                    if self.audio_at.take().is_some()
                        && let Some(mut out) = self.audio_out.take()
                    {
                        out.stop();
                    }
                }
            }

            // the Expression palette's preview lies over the pose: the base
            // is what the scene (or the hand) set, kept while nothing moves
            // so an Add does not pile up frame after frame
            let shown = if self.expressions.preview { self.expressions.sel } else { None };
            if shown != self.expr_shown {
                for (pi, v) in self.expr_base.drain(..) {
                    s.puppet.set_value(pi, v);
                }
                self.expr_shown = shown;
                s.dirty = true;
            }
            if let Some(mut e) = shown.and_then(|i| s.expressions.get(i)).cloned() {
                // the popup's parameter previews too, before it is added
                if let Some(p) = self.expressions.pending(s)
                    && !e.params.iter().any(|ep| ep.id == p.id)
                {
                    e.params.push(p);
                }
                let model = s.puppet.model();
                let ids: Vec<usize> = e
                    .params
                    .iter()
                    .filter_map(|ep| model.parameters.iter().position(|p| p.id == ep.id))
                    .collect();
                let fresh =
                    posed || ids.iter().any(|pi| !self.expr_base.iter().any(|(b, _)| b == pi));
                if fresh {
                    self.expr_base = ids.iter().map(|&pi| (pi, s.puppet.value(pi))).collect();
                }
                for ep in &e.params {
                    let Some(pi) = s.puppet.model().parameters.iter().position(|p| p.id == ep.id)
                    else {
                        continue;
                    };
                    let base =
                        self.expr_base.iter().find(|(b, _)| *b == pi).map_or(0.0, |(_, v)| *v);
                    s.puppet.set_value(pi, crate::expressions::blend(base, ep));
                }
                s.dirty = true;
            }
        }

        // a physics playlist plays while Physics Settings is up: the scenes
        // drive the parameters over the rest pose, the rig runs on top
        if self.show_physics
            && self.phys_page.pl_playing
            && let Some(s) = &mut self.session
            && let Some(pl) = s.playlists.get(self.phys_page.pl_sel).cloned()
        {
            let fps = s.animations.first().map_or(30.0, |a| a.fps).max(1.0);
            let scope =
                ayatsuri_core::comp::Scope { scenes: &s.animations, comps: &s.compositions };
            let comp = ayatsuri_core::comp::playlist_comp(&scope, &pl, fps);
            let dt = self.frame_dt;
            self.phys_page.pl_time += dt;
            if self.phys_page.pl_time > comp.duration {
                if self.phys_page.pl_loop && comp.duration > 0.0 {
                    self.phys_page.pl_time %= comp.duration;
                } else {
                    self.phys_page.pl_time = 0.0;
                    self.phys_page.pl_playing = false;
                }
            }
            let blends = ayatsuri_core::comp::sample(&scope, &comp, self.phys_page.pl_time);
            for ((target, id), b) in blends {
                if target != ayatsuri_format::CurveTarget::Parameter {
                    continue;
                }
                if let Some((pi, d)) = s
                    .puppet
                    .model()
                    .parameters
                    .iter()
                    .enumerate()
                    .find(|(_, p)| p.id == id)
                    .map(|(pi, p)| (pi, p.default))
                {
                    s.puppet.set_value(pi, b.over(d));
                }
            }
            s.dirty = true;
            ui.ctx().request_repaint();
        }

        // cursor following and the pendulum rig both drive parameters, so
        // they run just ahead of the per-frame evaluate
        if let Some(s) = &mut self.session {
            let dt = self.frame_dt;
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
        self.part_new_window(ui);
        self.template_window(ui);
        self.model_new_window(ui);
        self.model_info_window(ui);
        self.new_anim_window(ui);
        self.anim_details_window(ui);
        self.export_window(ui);
        self.render_window(ui);
        self.atlas_window(ui);
        self.skin_dialog(ui);
        self.export_overwrite_window(ui);
        self.close_confirm_window(ui);
        self.workspace_windows(ui);
        self.param_edit_window(ui);
        self.param_menu_windows(ui);
    }

    /// Cubism's Edit Mesh dialog, shown while mesh edit mode is live: the
    /// manual modes, the auto mesher, and the way out.
    fn notice_area(&mut self, ctx: &egui::Context) {
        let Some((text, until)) = &mut self.notice else { return };
        let now = ctx.input(|i| i.time);
        let until = *until.get_or_insert(now + 3.0);
        if now > until {
            self.notice = None;
            return;
        }
        egui::Area::new(egui::Id::new("canvas-notice"))
            .anchor(Align2::CENTER_TOP, vec2(0.0, 110.0))
            .order(egui::Order::Foreground)
            .interactable(false)
            .show(ctx, |ui| {
                egui::Frame::popup(ui.style()).show(ui, |ui| {
                    ui.label(
                        egui::RichText::new(format!("{}  {}", egui_phosphor::regular::INFO, text))
                            .color(theme::pal().text),
                    );
                });
            });
        ctx.request_repaint_after(std::time::Duration::from_secs_f64((until - now).max(0.05)));
    }

    /// Cubism's "Blend Shape is editable: <parameter>": shown while edits to
    /// the selected object land in a blend shape keyform, not a normal one.
    fn blend_edit_label(&self, ctx: &egui::Context) {
        use ayatsuri_core::edit::Target;
        let target = match self.selected {
            Some(Sel::Mesh(i)) => Target::ArtMesh(i),
            Some(Sel::Deformer(i)) => Target::Deformer(i),
            Some(Sel::Part(i)) => Target::Part(i),
            _ => return,
        };
        let Some(s) = &self.session else { return };
        let model = s.puppet.model();
        let values: Vec<f32> = (0..model.parameters.len()).map(|i| s.puppet.value(i)).collect();
        let Some(pi) = ayatsuri_core::keyform::editing_blend_shape(model, target, &values) else {
            return;
        };
        let name = s
            .param_names
            .get(pi)
            .filter(|n| !n.is_empty())
            .or_else(|| model.parameters.get(pi).map(|p| &p.id))
            .cloned()
            .unwrap_or_default();
        egui::Area::new(egui::Id::new("blend-edit-label"))
            .anchor(Align2::CENTER_TOP, vec2(0.0, 74.0))
            .order(egui::Order::Foreground)
            .interactable(false)
            .show(ctx, |ui| {
                egui::Frame::popup(ui.style()).show(ui, |ui| {
                    ui.label(
                        egui::RichText::new(format!(
                            "{}  {}: {name}",
                            egui_phosphor::regular::SHAPES,
                            crate::i18n::tr("Blend Shape is editable")
                        ))
                        .color(theme::pal().accent),
                    );
                });
            });
    }

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
                let why_not = mesh_unfinished(verts, tris);
                if let Some(why) = why_not {
                    ui.colored_label(theme::pal().rotation_red, crate::i18n::tr(why));
                }
                ui.horizontal(|ui| {
                    if ui
                        .add_enabled(
                            why_not.is_none(),
                            egui::Button::new(
                                egui::RichText::new(crate::i18n::tr("Done")).color(Color32::WHITE),
                            ),
                        )
                        .on_disabled_hover_text(crate::i18n::tr(why_not.unwrap_or_default()))
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
        tilt: f32,
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
            let grid = Stroke::new(0.5, theme::pal().border.gamma_multiply(0.5));
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
            let dt = self.frame_dt.clamp(0.001, 0.05);
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
        // the frame's cross through the pivot, turned by the angle inputs as
        // they move (and by the view's own turn), as in Cubism
        if let Some(&(ax, ay)) = drawn.first() {
            let pivot = map(ax, ay);
            let (ts, tc) = (tilt + self.phys_page.prev_rot).to_radians().sin_cos();
            let reach_px = prect.width().hypot(prect.height());
            let cross = Stroke::new(1.0, theme::pal().border);
            for (dx, dy) in [(tc, ts), (-ts, tc)] {
                let d = vec2(dx, dy) * reach_px;
                pp.line_segment([pivot - d, pivot + d], cross);
            }
        }
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
                    ("angle", &{
                        // no "-0.0" at rest
                        let a = -(tilt + self.phys_page.prev_rot);
                        format!("{:.1}", if a.abs() < 0.05 { 0.0 } else { a })
                    }),
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
    /// Cubism's Change the draw order of all ArtMeshes: each mesh its own
    /// draw order in Parts palette order, so a model imported at 500 all
    /// round (as a PSD comes in) keeps its stacking with values to work from.
    fn draw_spread_window(&mut self, ui: &mut Ui) {
        let Some((mut auto, mut interval)) = self.draw_spread else { return };
        let mut open = true;
        let mut apply = false;
        crate::windows::Dialog::new(crate::i18n::tr("Change the draw order of all ArtMeshes"))
            .open(&mut open)
            .show(ui.ctx(), |ui| {
                ui.radio_value(&mut auto, true, crate::i18n::tr("Auto setup"));
                ui.horizontal(|ui| {
                    ui.radio_value(&mut auto, false, crate::i18n::tr("Manual setup"));
                    ui.add_enabled(
                        !auto,
                        egui::DragValue::new(&mut interval).range(1.0..=500.0).speed(1.0),
                    );
                    ui.label(crate::i18n::tr("Interval"));
                });
                ui.weak(crate::i18n::tr(
                    "In Parts palette order, the top row highest. Meshes in a draw order \
                     group keep theirs.",
                ));
                ui.horizontal(|ui| {
                    apply = ui.button(crate::i18n::tr("OK")).clicked();
                    if ui.button(crate::i18n::tr("Cancel")).clicked() {
                        self.draw_spread = None;
                    }
                });
            });
        if self.draw_spread.is_none() {
            return;
        }
        self.draw_spread = (open && !apply).then_some((auto, interval));
        if apply
            && let Some(s) = &mut self.session
            && s.edit(|m| {
                ayatsuri_core::palette::spread_draw_orders(m, (!auto).then_some(interval))
            })
        {
            log::info!(
                "draw orders of all art meshes changed ({})",
                if auto { "auto".to_string() } else { format!("every {interval}") }
            );
        }
    }

    fn workspace_windows(&mut self, ui: &mut Ui) {
        self.draw_spread_window(ui);
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
                    log::info!("workspace saved as {name}", name = self.ws_name.trim());
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
                    let (name, _) = self.prefs.custom_workspaces.remove(i);
                    log::info!("workspace {name} deleted");
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
            PanelKind::Expressions => {
                if let Some(s) = &mut self.session {
                    self.expressions.show(ui, s);
                } else {
                    ui.weak(crate::i18n::tr("No model loaded."));
                }
            }
            PanelKind::Mixer => {
                if let Some(s) = &mut self.session {
                    let mut vol = self.prefs.audio_volume.unwrap_or(100);
                    let route = self
                        .audio_out
                        .as_ref()
                        .map(|o| o.describe.clone())
                        .or_else(|| self.prefs.audio_device.clone())
                        .unwrap_or_else(|| crate::i18n::tr("System default").to_string());
                    let levels =
                        self.audio_out.as_ref().map(|o| o.take_levels()).unwrap_or_default();
                    let changed =
                        self.mixer.show(ui, s, &mut self.timeline, &mut vol, levels, &route);
                    if self.anim_playing {
                        ui.ctx().request_repaint();
                    }
                    if changed.master {
                        self.prefs.audio_volume = Some(vol);
                        self.prefs.save();
                        if let Some(out) = &self.audio_out {
                            out.set_volume(vol as f32 / 100.0);
                        }
                    }
                } else {
                    ui.weak(crate::i18n::tr("No model loaded."));
                }
            }
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
            Some(crate::timeline::Request::ImportAudio) => self.pick_audio(),
            Some(crate::timeline::Request::EditDetails) => self.open_anim_details(),
            None => {}
        }
    }

    /// File > Import Audio: pick a sound file for the current composition.
    fn pick_audio(&mut self) {
        self.pick_async(PickFor::ImportAudio, || {
            rfd::FileDialog::new()
                .add_filter("Audio", &["wav", "mp3", "flac", "ogg", "oga"])
                .set_title(crate::i18n::tr("Import Audio…"))
                .pick_file()
        });
    }

    /// Animation > Edit Animation Details, for the composition on show,
    /// else the scene.
    fn open_anim_details(&mut self) {
        let current = self.session.as_ref().and_then(|s| {
            if let Some(c) = s.current_comp.and_then(|i| s.compositions.get(i)) {
                Some((c.name.clone(), c.fps, c.duration, c.looped))
            } else {
                s.current_anim
                    .and_then(|i| s.animations.get(i))
                    .map(|a| (a.name.clone(), a.fps, a.duration, a.looped))
            }
        });
        if let Some((name, fps, duration, looped)) = current {
            self.anim_details = Some(AnimDetails { name, fps, duration, looped, frames: 1 });
        }
    }

    /// Animation > Edit Animation Details: the current scene's or
    /// composition's name, rate, length and loop, applied as one undo step
    /// of the Timeline; frames open up or come out at the playhead.
    fn anim_details_window(&mut self, ui: &mut Ui) {
        let Some(d) = &mut self.anim_details else { return };
        let mut open = true;
        let mut apply = false;
        let mut ripple = 0i64;
        crate::windows::Dialog::new(crate::i18n::tr("Animation details")).open(&mut open).show(
            ui.ctx(),
            |ui| {
                crate::windows::form(ui, "adetails", |ui| {
                    ui.label(crate::i18n::tr("Name"));
                    ui.text_edit_singleline(&mut d.name);
                    ui.end_row();
                    ui.label(crate::i18n::tr("FPS"));
                    ui.add(egui::DragValue::new(&mut d.fps).range(1.0..=120.0));
                    ui.end_row();
                    ui.label(crate::i18n::tr("Length"));
                    ui.add(
                        egui::DragValue::new(&mut d.duration)
                            .range(0.1..=600.0)
                            .speed(0.05)
                            .suffix(" s"),
                    );
                    ui.end_row();
                    ui.label(crate::i18n::tr("Loop"));
                    ui.checkbox(&mut d.looped, "");
                    ui.end_row();
                    ui.label(crate::i18n::tr("Frames at the playhead"));
                    ui.horizontal(|ui| {
                        ui.add(egui::DragValue::new(&mut d.frames).range(1..=600));
                        if ui
                            .button(crate::i18n::tr("Insert"))
                            .on_hover_text(crate::i18n::tr(
                                "Open up frames at the playhead; later keys move on.",
                            ))
                            .clicked()
                        {
                            ripple = d.frames;
                        }
                        if ui
                            .button(crate::i18n::tr("Remove"))
                            .on_hover_text(crate::i18n::tr(
                                "Take frames out at the playhead; later keys move back.",
                            ))
                            .clicked()
                        {
                            ripple = -d.frames;
                        }
                    });
                    ui.end_row();
                });
                ui.horizontal(|ui| {
                    if ui.button(crate::i18n::tr("Apply")).clicked() {
                        apply = true;
                    }
                });
            },
        );
        if ripple != 0 {
            self.timeline.ripple(ripple);
            if let Some(s) = &self.session {
                // the dialog's length follows the ripple
                let fps = s
                    .current_comp
                    .and_then(|i| s.compositions.get(i))
                    .map(|c| c.fps)
                    .or_else(|| s.current_anim.and_then(|i| s.animations.get(i)).map(|a| a.fps))
                    .unwrap_or(30.0)
                    .max(1.0);
                if let Some(d) = &mut self.anim_details {
                    d.duration = (d.duration + ripple as f32 / fps).max(1.0 / fps);
                }
            }
        }
        if apply && let (Some(d), Some(s)) = (self.anim_details.take(), &mut self.session) {
            let name = if d.name.trim().is_empty() { "motion".to_string() } else { d.name.clone() };
            log::info!("{name}: {} fps, {:.2} s, looped {}", d.fps, d.duration, d.looped);
            if let Some(ci) = s.current_comp {
                if let Some(c) = s.compositions.get(ci).cloned() {
                    self.timeline.remember_comp(ci, &c, None);
                    let old = c.name.clone();
                    if let Some(c) = s.compositions.get_mut(ci) {
                        c.name = name.clone();
                        c.fps = d.fps.max(1.0);
                        c.duration = d.duration.max(1.0 / c.fps);
                        c.looped = d.looped;
                    }
                    if old != name {
                        rename_source(s, &old, &name, true);
                    }
                }
            } else if let Some(ai) = s.current_anim
                && let Some(a) = s.animations.get(ai).cloned()
            {
                self.timeline.remember(ai, &a, None);
                let old = a.name.clone();
                if let Some(a) = s.animations.get_mut(ai) {
                    a.name = name.clone();
                    a.fps = d.fps.max(1.0);
                    a.duration = d.duration.max(1.0 / a.fps);
                    a.looped = d.looped;
                    if let Some((w0, w1)) = a.work_area {
                        a.work_area = Some((w0.min(a.duration), w1.min(a.duration)));
                    }
                }
                if old != name {
                    rename_source(s, &old, &name, false);
                }
            }
            s.modified = true;
            self.anim_applied = f32::NAN;
        } else if !open {
            self.anim_details = None;
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
            } else if targets.is_empty() {
                log::warn!("Delete Keyform: select the objects whose key to delete first");
            } else {
                log::warn!("Delete Keyform: the selected objects have no keys on this parameter");
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
    /// The selected mesh and its deform path at the current edit level
    /// (or the other one), root to tip, when it has two points or more.
    fn skin_path(&self) -> Option<(usize, Vec<SkinPin>)> {
        let Some(Sel::Mesh(mi)) = self.selected else { return None };
        let level = self.edit_level.max(2);
        let pins = [level, 5 - level]
            .iter()
            .filter_map(|&l| self.deform_paths.get(&(mi, l)))
            .find(|p| p.len() >= 2)?;
        Some((mi, pins.iter().map(|p| (p.tri, p.w)).collect()))
    }

    /// Modeling > Skinning > Use Deform Path for Skinning: the selected
    /// mesh becomes a glued chain of pieces on rotation deformers, one per
    /// path point but the tip, with a parameter each (-45 to 45) in a
    /// folder named after the mesh, as Cubism does it.
    fn apply_skin(&mut self) {
        let Some((mi, pins)) = self.skin_path() else { return };
        let Some(s) = &mut self.session else { return };
        let name = s.mesh_name(mi);
        let base: String = name.chars().filter(|c| c.is_ascii_alphanumeric()).collect();
        let base = format!("Param{}Skin", if base.is_empty() { "Mesh".into() } else { base });
        let mut made = None;
        let mut failed = None;
        s.edit(|m| match ayatsuri_core::skin::skin_from_pins(m, mi, &pins, 45.0, &base) {
            Ok(r) => {
                made = Some(r);
                true
            }
            Err(e) => {
                failed = Some(e);
                false
            }
        });
        if let Some(e) = failed {
            log::warn!("skinning {name}: {}", skin_dialog::skin_error(e));
            return;
        }
        let Some(made) = made else { return };
        for (k, _) in made.params.iter().enumerate() {
            s.param_names.push(format!("{name} {}", k + 1));
        }
        s.groups.push(crate::session::ParamGroup {
            name: name.clone(),
            params: made.params.clone(),
            open: true,
            color: None,
        });
        // the path became the deformers
        self.deform_paths.retain(|(m, _), _| *m != mi);
        self.selected = made.deformers.first().map(|&d| Sel::Deformer(d));
        log::info!(
            "skinned {name}: {} pieces on {} rotation deformers, {} parameters",
            made.pieces.len(),
            made.deformers.len(),
            made.params.len()
        );
    }

    /// Cubism's Solo on or off. On, it takes the selection: meshes, every
    /// mesh inside a selected part, and every mesh under a selected
    /// deformer, however deep.
    fn toggle_solo(&mut self) {
        let Some(s) = &self.session else { return };
        if s.solo.is_some() {
            if let Some(s) = &mut self.session {
                s.solo = None;
            }
            log::info!("solo off");
            return;
        }
        let model = s.puppet.model();
        let mut meshes: std::collections::HashSet<usize> =
            self.selection_meshes().into_iter().collect();
        let mut parts = self.part_pick.clone();
        if let Some(Sel::Part(p)) = self.selected {
            parts.push(p);
        }
        let under_deformer = match self.selected {
            Some(Sel::Deformer(d)) => meshes_under_deformer(model, d),
            _ => Vec::new(),
        };
        for (mi, m) in model.art_meshes.iter().enumerate() {
            let in_part = m.parent_part.is_some_and(|q| {
                parts.iter().any(|&p| ayatsuri_core::palette::within(model, q as usize, p))
            });
            let under = under_deformer.contains(&mi);
            if in_part || under {
                meshes.insert(mi);
            }
        }
        if meshes.is_empty() {
            set_notice(&mut self.notice, crate::i18n::tr("Select what to solo first."));
            return;
        }
        let ids: std::collections::HashSet<String> = meshes
            .iter()
            .filter_map(|&mi| model.art_meshes.get(mi).map(|m| m.id.clone()))
            .collect();
        log::info!("solo on: {} mesh(es)", ids.len());
        if let Some(s) = &mut self.session {
            s.solo = Some(ids);
        }
    }

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
    ///
    /// Frames keep a steady beat: each is due one period after the last
    /// one's slot, not after the last frame finished, so the build time and
    /// the wait for the screen do not stack up and the rate holds at the
    /// cap. A frame that runs late starts a new beat instead of rushing.
    fn request_anim(&self, ctx: &egui::Context) {
        pace_anim(&self.anim_wanted, ctx);
    }

    /// The frame limiter: while something animates under an FPS cap, a
    /// frame that comes early sleeps to its slot, one period after the last
    /// one's, so playback and physics run at a steady rate instead of
    /// whenever a timer happens to fire. Frames a person causes (a click, a
    /// drag) and an editor at rest never wait.
    fn limit_frame(&mut self) {
        // the beat lasts while each frame asks for the next
        if !self.anim_wanted.replace(false) || self.prefs.fps_cap == 0 {
            self.anim_slot = None;
            return;
        }
        let period = std::time::Duration::from_secs_f32(1.0 / self.prefs.fps_cap as f32);
        let now = std::time::Instant::now();
        let slot = match self.anim_slot {
            Some(prev) if prev + period > now => {
                std::thread::sleep(prev + period - now);
                prev + period
            }
            // the first frame of a beat, or a late one: a new beat from now
            // rather than rushing to catch up
            _ => now,
        };
        self.anim_slot = Some(slot);
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
            MeshAction::Clear => {
                let Some(s) = &mut self.session else { return };
                let cleared = s.edit(|m| {
                    let mut any = false;
                    for &mi in &targets {
                        let n = m.art_meshes.get(mi).map_or(0, |a| a.vertex_count as usize);
                        let all: Vec<usize> = (0..n).collect();
                        any |= ayatsuri_core::edit::delete_vertices(m, mi, &all);
                    }
                    any
                });
                if cleared {
                    for &mi in &targets {
                        self.pending_edges.remove(&mi);
                    }
                    self.edge_first = None;
                    self.mesh_picked.clear();
                    log::info!(
                        "mesh cleared on {} mesh(es): place points on the art",
                        targets.len()
                    );
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
                let lines: Vec<Vec<(u16, u16)>> = pages
                    .iter()
                    .map(|(mi, _)| self.pending_edges.get(mi).cloned().unwrap_or_default())
                    .collect();
                s.edit(|m| {
                    for ((mi, page), lines) in pages.iter().zip(&lines) {
                        let solid = |u: f32, v: f32| -> bool {
                            let Some(t) = page else { return true };
                            let x =
                                ((u * t.width as f32) as i64).clamp(0, t.width as i64 - 1) as usize;
                            let y = ((v * t.height as f32) as i64).clamp(0, t.height as i64 - 1)
                                as usize;
                            t.rgba.get((y * t.width as usize + x) * 4 + 3).copied().unwrap_or(255)
                                > 8
                        };
                        let opaque: Option<&dyn Fn(f32, f32) -> bool> =
                            protect.then_some(&solid as &dyn Fn(f32, f32) -> bool);
                        added += ayatsuri_core::edit::auto_connect(m, *mi, rebuild, lines, opaque);
                    }
                    added > 0
                });
                // lines that became face edges are done; the rest stay drawn
                for &mi in &targets {
                    if let Some(list) = self.pending_edges.get_mut(&mi) {
                        list.retain(|&(x, y)| {
                            !ayatsuri_core::edit::has_edge(s.puppet.model(), mi, x, y)
                        });
                    }
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

    /// One mesh's new triangulation from its texture's alpha, or why not.
    fn auto_mesh_data(&self, mi: usize) -> Result<(Vec<f32>, Vec<u16>), String> {
        let o = &self.tool_opts.auto;
        let s = self.session.as_ref().ok_or("no model open")?;
        let am = s.puppet.model().art_meshes.get(mi).ok_or("mesh is gone")?;
        let png = s.texture_pngs.get(am.texture as usize).ok_or("mesh has no texture")?;
        let img = image::load_from_memory(png)
            .map_err(|e| format!("texture decode failed: {e}"))?
            .to_rgba8();
        let bbox = |uvs: &[f32]| {
            let b = uvs
                .as_chunks::<2>()
                .0
                .iter()
                .fold((f32::MAX, f32::MAX, f32::MIN, f32::MIN), |(a, b, c, d), &[u, v]| {
                    (a.min(u), b.min(v), c.max(u), d.max(v))
                });
            (b.0 <= b.2 && b.1 <= b.3).then_some(b)
        };
        // the layer's own box, where its art was imported (or last packed),
        // never where its points happen to be now: the old mesh is ignored
        let art_box = |i: usize| {
            s.original_meshes
                .get(i)
                .and_then(|(uvs, _)| bbox(uvs))
                .or_else(|| s.puppet.model().art_meshes.get(i).and_then(|m| bbox(&m.uvs)))
        };
        let (u0, v0, u1, v1) = art_box(mi).ok_or("mesh has no extent")?;
        // Cubism gives margins for a 1024px texture and scales them with it
        let k = img.width().max(img.height()) as f32 / 1024.0;
        let params = ayatsuri_core::automesh::Params {
            outline_spacing: o.interval_outside.clamp(4.0, 512.0),
            inner_spacing: o.interval_inside.clamp(4.0, 512.0),
            margin: (o.margin_outside * k).clamp(0.0, 256.0),
            inner_margin: (o.margin_inside * k).clamp(0.0, 256.0),
            min_margin: (o.min_margin * k).clamp(0.0, 256.0),
            min_points: o.min_points.clamp(3, 64) as usize,
            threshold: o.alpha,
            style: if o.face {
                ayatsuri_core::automesh::Style::Face
            } else {
                ayatsuri_core::automesh::Style::Lattice
            },
        };
        // room for the outline's margin: each side grows over the page's
        // empty space, stopping two pixels short of the other layers' art
        let (pw, ph) = (img.width() as f32, img.height() as f32);
        let others: Vec<(f32, f32, f32, f32)> = s
            .puppet
            .model()
            .art_meshes
            .iter()
            .enumerate()
            .filter(|(i, m)| *i != mi && m.texture == am.texture)
            .filter_map(|(i, _)| art_box(i))
            .filter(|b| *b != (u0, v0, u1, v1))
            .collect();
        let (gu, gv) = (2.0 / pw, 2.0 / ph);
        let (ru, rv) = ((params.margin + 2.0) / pw, (params.margin + 2.0) / ph);
        let mut room = (u0, v0, u1, v1);
        let across_v = |o: &(f32, f32, f32, f32), r: &(f32, f32, f32, f32)| o.1 < r.3 && o.3 > r.1;
        let across_u = |o: &(f32, f32, f32, f32), r: &(f32, f32, f32, f32)| o.0 < r.2 && o.2 > r.0;
        let mut left = (u0 - ru).max(0.0);
        let mut right = (u1 + ru).min(1.0);
        for o in others.iter().filter(|o| across_v(o, &room)) {
            if o.2 <= u0 {
                left = left.max(o.2 + gu);
            }
            if o.0 >= u1 {
                right = right.min(o.0 - gu);
            }
        }
        room.0 = left.min(u0);
        room.2 = right.max(u1);
        let mut top = (v0 - rv).max(0.0);
        let mut bottom = (v1 + rv).min(1.0);
        for o in others.iter().filter(|o| across_u(o, &room)) {
            if o.3 <= v0 {
                top = top.max(o.3 + gv);
            }
            if o.1 >= v1 {
                bottom = bottom.min(o.1 - gv);
            }
        }
        room.1 = top.min(v0);
        room.3 = bottom.max(v1);
        let started = std::time::Instant::now();
        let alpha_at = |x: u32, y: u32| img.get_pixel_checked(x, y).map_or(0, |p| p.0[3]);
        let out = ayatsuri_core::automesh::for_patch(
            &alpha_at,
            (img.width(), img.height()),
            (u0, v0, u1, v1),
            room,
            &params,
        )
        .ok_or("no opaque pixels in this layer's art")?;
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
        // a mesh cleared in Mesh Edit is placed on the mesh as it was then
        let reference = self.mesh_art.as_ref().map(|a| (a.mesh, a.reference.clone()));
        let Some(s) = &mut self.session else { return };
        let (verts, tris) =
            jobs.iter().fold((0, 0), |(v, t), (_, (uv, ix))| (v + uv.len() / 2, t + ix.len() / 3));
        if s.edit(|m| {
            let mut any = false;
            for (mi, (uvs, idx)) in &jobs {
                let r = reference.as_ref().filter(|r| r.0 == *mi).map(|r| &r.1);
                any |= ayatsuri_core::edit::remesh_from(m, *mi, r, uvs, idx);
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
        s.key_colors.retain(|e| e.0 != pi);
        for e in &mut s.key_colors {
            if e.0 > pi {
                e.0 -= 1;
            }
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
                        blend_shape: false,
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
                    ui.label("");
                    let blend = ui
                        .checkbox(&mut new.blend_shape, crate::i18n::tr("Blend Shape"))
                        .on_hover_text(crate::i18n::tr(
                            "Its keyforms add on top of the others instead of multiplying them",
                        ));
                    // the manual's recommended range: base key at the default
                    if blend.changed() && new.blend_shape {
                        (new.min, new.default, new.max) = (0.0, 0.0, 1.0);
                    }
                    ui.end_row();
                });
                ui.add_space(6.0);
                if ui.button(crate::i18n::tr("Create")).clicked() {
                    create = true;
                }
            },
        );
        if create {
            let (name, id, min, default, max, group, blend_shape) = {
                let n = self.new_param.take();
                let Some(n) = n else { return };
                (n.name, n.id, n.min, n.default, n.max, n.group, n.blend_shape)
            };
            let id = if id.is_empty() { name.replace(' ', "") } else { id };
            if let Some(s) = &mut self.session {
                if s.edit(|m| {
                    let made =
                        ayatsuri_core::edit::create_parameter(m, id.clone(), min, default, max);
                    if let Some(p) = made.and_then(|i| m.parameters.get_mut(i)) {
                        p.blend_shape = blend_shape;
                    }
                    made.is_some()
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
        // a selected object stays selected, so its keys can be made on
        // parameter after parameter; with none, the parameter is shown
        if !matches!(self.selected, Some(Sel::Mesh(_) | Sel::Deformer(_) | Sel::Part(_))) {
            self.selected = Some(Sel::Parameter(pi));
        }
        self.param_focus = true;
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

    /// Model Template: apply a parameter template, or save this model's
    /// parameters as one.
    fn template_window(&mut self, ui: &mut Ui) {
        let Some(dlg) = &mut self.template_dlg else { return };
        let mut open = true;
        let mut go = false;
        let mut skip = false;
        crate::windows::Dialog::new(crate::i18n::tr("Model Template")).open(&mut open).show(
            ui.ctx(),
            |ui| {
                ui.set_width(360.0);
                match dlg {
                    TemplateDlg::Apply { list, pick, after_import } => {
                        if *after_import {
                            ui.label(crate::i18n::tr(
                                "The artwork came in without parameters. Start from a template?",
                            ));
                            ui.add_space(4.0);
                        }
                        for (i, t) in list.iter().enumerate() {
                            let label = format!(
                                "{}  ({})",
                                crate::i18n::tr_text(&t.name),
                                crate::i18n::trf(
                                    "{count} parameters",
                                    &[("count", &t.parameters.len())]
                                )
                            );
                            ui.radio_value(pick, i, label);
                        }
                        if let Some(t) = list.get(*pick) {
                            ui.add_space(4.0);
                            ui.weak(crate::i18n::tr_text(&t.description));
                        }
                        ui.add_space(4.0);
                        ui.weak(crate::i18n::tr(
                            "Parameters the model already has (by ID) are left as they are.",
                        ));
                        ui.add_space(6.0);
                        ui.horizontal(|ui| {
                            go = ui.button(crate::i18n::tr("Apply")).clicked();
                            if *after_import {
                                skip = ui.button(crate::i18n::tr("No Template")).clicked();
                            }
                        });
                    }
                    TemplateDlg::Save { name, description } => {
                        crate::windows::form(ui, "tsave", |ui| {
                            ui.label(crate::i18n::tr("Name"));
                            ui.text_edit_singleline(name);
                            ui.end_row();
                            ui.label(crate::i18n::tr("Description"));
                            ui.text_edit_singleline(description);
                            ui.end_row();
                        });
                        ui.weak(crate::i18n::tr(
                            "Saves the parameters, their ranges and folders. Apply it to any model \
                             from Modeling > Model Template.",
                        ));
                        go = ui.button(crate::i18n::tr("Save")).clicked();
                    }
                }
            },
        );
        if !open || skip {
            self.template_dlg = None;
            return;
        }
        if !go {
            return;
        }
        let Some(dlg) = self.template_dlg.take() else { return };
        let Some(s) = &mut self.session else { return };
        match dlg {
            TemplateDlg::Apply { list, pick, .. } => {
                if let Some(t) = list.get(pick) {
                    let n = crate::templates::apply(s, t);
                    log::info!("template {}: {n} parameter(s) added", t.name);
                }
            }
            TemplateDlg::Save { name, description } => {
                let name = if name.trim().is_empty() { s.name.clone() } else { name.trim().into() };
                let t = crate::templates::from_session(s, &name, &description);
                match crate::templates::save(&t) {
                    Ok(p) => log::info!("template saved to {}", p.display()),
                    Err(e) => log::error!("template not saved: {e}"),
                }
            }
        }
    }

    /// Open New Part, with Put selected objects into children set to `take`.
    fn open_new_part(&mut self, take: bool) {
        let Some(s) = &self.session else { return };
        self.new_part = Some(NewPartDlg {
            name: crate::i18n::tr("New Part").to_string(),
            id: ayatsuri_core::palette::free_part_id(s.puppet.model()),
            order: 500.0,
            take,
        });
    }

    /// Cubism's New Part: name, ID, draw order, and whether the selected
    /// objects go in. Without them the part goes inside the selected part
    /// (or beside the selected mesh); with them, where they were.
    fn part_new_window(&mut self, ui: &mut Ui) {
        let Some(dlg) = &mut self.new_part else { return };
        let mut open = true;
        let mut create = false;
        crate::windows::Dialog::new(crate::i18n::tr("New Part")).open(&mut open).show(
            ui.ctx(),
            |ui| {
                ui.set_width(260.0);
                egui::Grid::new("new-part").num_columns(2).show(ui, |ui| {
                    ui.label(crate::i18n::tr("Name"));
                    ui.text_edit_singleline(&mut dlg.name);
                    ui.end_row();
                    ui.label(crate::i18n::tr("Part ID"));
                    ui.text_edit_singleline(&mut dlg.id);
                    ui.end_row();
                    ui.label(crate::i18n::tr("Draw order"));
                    ui.add(egui::DragValue::new(&mut dlg.order).range(0.0..=1000.0).speed(1.0));
                    ui.end_row();
                });
                ui.checkbox(&mut dlg.take, crate::i18n::tr("Put selected objects into children"));
                ui.weak(crate::i18n::tr("IDs take letters, digits and _ only."));
                if ui.button(crate::i18n::tr("Create")).clicked() {
                    create = true;
                }
            },
        );
        if !open {
            self.new_part = None;
            return;
        }
        if !create {
            return;
        }
        let Some(dlg) = self.new_part.take() else { return };
        let Some(s) = &mut self.session else { return };
        let id = dlg.id.trim().to_string();
        if id.is_empty()
            || !id.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
            || s.puppet.model().parts.iter().any(|p| p.id == id)
        {
            set_notice(
                &mut self.notice,
                crate::i18n::tr("That Part ID is empty, taken or has characters IDs can't use."),
            );
            self.new_part = Some(dlg);
            return;
        }
        // what goes in, in palette order, and where the part goes
        let mut children: Vec<PRow> = Vec::new();
        if dlg.take {
            let parts: Vec<usize> = match (self.part_pick.is_empty(), self.selected) {
                (true, Some(Sel::Part(p))) => vec![p],
                _ => self.part_pick.clone(),
            };
            let meshes: Vec<usize> = match (self.multi_sel.is_empty(), self.selected) {
                (true, Some(Sel::Mesh(m))) => vec![m],
                _ => self.multi_sel.clone(),
            };
            for (r, _) in ayatsuri_core::palette::rows(s.puppet.model()) {
                let picked = match r {
                    PRow::Part(p) => parts.contains(&p),
                    PRow::Mesh(m) => meshes.contains(&m),
                };
                if picked {
                    children.push(r);
                }
            }
        }
        let parent = match children.first() {
            Some(&r) => ayatsuri_core::palette::parent(s.puppet.model(), r),
            None => match self.selected {
                Some(Sel::Part(p)) => Some(p),
                Some(Sel::Mesh(m)) => {
                    ayatsuri_core::palette::parent(s.puppet.model(), PRow::Mesh(m))
                }
                _ => None,
            },
        };
        let mut index = None;
        s.edit(|m| {
            index = Some(ayatsuri_core::palette::new_part(m, &id, parent, dlg.order, &children));
            true
        });
        let Some(index) = index else { return };
        let n = s.puppet.model().parts.len();
        if s.part_names.len() < n {
            s.part_names.resize(n, String::new());
        }
        if let Some(slot) = s.part_names.get_mut(index) {
            slot.clone_from(&dlg.name);
        }
        if s.part_shown.len() < n {
            s.part_shown.resize(n, true);
        }
        if let Some(p) = parent {
            self.part_collapsed.remove(&p);
        }
        self.selected = Some(Sel::Part(index));
        self.part_pick.clear();
        self.multi_sel.clear();
        log::info!("new part {id} ({}) holding {} object(s)", dlg.name, children.len());
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
            // evaluate again: without it the meshes and boxes kept the old
            // pose while the warp lattice, read from the values, showed the
            // new one
            s.dirty = true;
            log::info!("parameters reset to defaults");
        }
    }

    fn run_key_action(&mut self, ctx: &egui::Context, id: &str) {
        log::debug!("shortcut: {id}");
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
                self.settings_tab = crate::windows::settings::TAB_KEYBOARD;
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
            "motion-mirror" if have => {
                if let Some(pi) = self.sel_param {
                    self.run_param_cmd(ParamCmd::Mirror(pi));
                }
            }
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
            "tool-deform-brush" | "tool-glue" | "tool-artpath" | "tool-auto-mesh" | "tool-warp"
            | "tool-rotation" | "tool-atlas"
                if self.mesh_edit.is_none() =>
            {
                self.choose_tool(match id {
                    "tool-deform-brush" => Tool::DeformBrush,
                    "tool-glue" => Tool::Glue,
                    "tool-artpath" => Tool::ArtPath,
                    "tool-auto-mesh" => Tool::MeshAuto,
                    "tool-warp" => Tool::WarpCreate,
                    "tool-rotation" => Tool::RotationCreate,
                    _ => Tool::TextureAtlas,
                });
            }
            "level-1" => self.edit_level = 1,
            "level-2" => self.edit_level = 2,
            "level-3" => self.edit_level = 3,
            "display-all" => {
                self.view_zoom = 1.0;
                self.view_pan = Vec2::ZERO;
            }
            "focus" => self.focus_request = true,
            "rename" if have => self.rename_selected(),
            "rename-deformer" if have => {
                let d = match self.selected {
                    Some(Sel::Deformer(d)) => Some(d),
                    Some(Sel::Mesh(m)) => self.session.as_ref().and_then(|s| {
                        s.puppet.model().art_meshes.get(m)?.parent_deformer.map(|d| d as usize)
                    }),
                    _ => None,
                };
                if let Some(d) = d {
                    self.open_def_rename(d);
                }
            }
            "mirror-edit" => self.mirror_edit = !self.mirror_edit,
            "solo" => self.toggle_solo(),
            "highlight-children" => self.highlight_children = !self.highlight_children,
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
            "zoom-in" => self.view_zoom = (self.view_zoom * 1.25).clamp(ZOOM_MIN, ZOOM_MAX),
            "zoom-out" => self.view_zoom = (self.view_zoom / 1.25).clamp(ZOOM_MIN, ZOOM_MAX),
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
            "temp-zoom" => self.start_temp_deform(temp_deform::Method::Zoom),
            "temp-rotation" => self.start_temp_deform(temp_deform::Method::Rotation),
            "temp-free" => self.start_temp_deform(temp_deform::Method::Free),
            "temp-perspective" => self.start_temp_deform(temp_deform::Method::Perspective),
            "temp-distort" => self.start_temp_deform(temp_deform::Method::Distort),
            "temp-warp" => self.start_temp_deform(temp_deform::Method::Warp),
            "temp-path" => self.start_temp_deform(temp_deform::Method::Path),
            "temp-deselect" => self.end_temp_deform(),
            "unlock-all" => {
                self.mesh_locked.clear();
                self.def_locked.clear();
                log::info!("everything unlocked");
            }
            "copy-form" => self.copy_form(),
            "paste-form" => self.paste_form(),
            "new-animation" if have => self.open_new_anim(false),
            "new-composition" if have => self.open_new_anim(true),
            "export-animation" if have => {
                self.open_render(ctx, crate::windows::render::Source::Animation);
            }
            "export-timelapse" if have => {
                self.open_render(ctx, crate::windows::render::Source::Timelapse);
            }
            "import-motion" if have => self.pick_motion(),
            "import-audio" if have => self.pick_audio(),
            "import-artwork" if have => self.pick_artwork(false),
            "reimport-artwork" if have => self.pick_artwork(true),
            "project-info" if have => self.show_model_info = true,
            "model-stats" if have => self.show_stats = true,
            "settings" => self.show_settings = true,
            "exit" => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
            "highlight-overlap" => self.highlight_overlap = !self.highlight_overlap,
            "atlas-view" if have => {
                self.atlas_view = !self.atlas_view;
                log::info!("texture atlas view {}", if self.atlas_view { "on" } else { "off" });
            }
            "guides-show" => self.show_guides = !self.show_guides,
            "guides-center" if have => self.pending_guide = Some(GuideCmd::Center),
            "guide-at-vertex" if have => self.pending_guide = Some(GuideCmd::AtVertex),
            "guide-settings" if have => self.show_guide_settings = true,
            "guide-snap" => {
                self.prefs.guide_snap_off = !self.prefs.guide_snap_off;
                self.prefs.save();
            }
            "smooth-drag" => {
                self.prefs.smooth_drag = !self.prefs.smooth_drag;
                self.prefs.save();
            }
            "rulers" => {
                self.prefs.rulers_off = !self.prefs.rulers_off;
                self.prefs.save();
            }
            "workspace-modeling" => self.set_workspace(Workspace::Modeling),
            "create-warp" if !self.selection_meshes().is_empty() => {
                self.pending_create = Some((true, None));
            }
            "create-rotation" if !self.selection_meshes().is_empty() => {
                self.pending_create = Some((false, None));
            }
            "conversion-divisions" => {
                if let Some(Sel::Deformer(d)) = self.selected {
                    self.open_def_division(d);
                }
            }
            "new-part" if have => self.open_new_part(false),
            "group-into-part" if have => self.open_new_part(true),
            "clip-by" => {
                if let Some(Sel::Mesh(mi)) = self.selected {
                    self.start_clip_pick(mi);
                }
            }
            "copy-ids" => {
                if let Some(s) = &self.session {
                    let model = s.puppet.model();
                    let ids: Vec<&str> = self
                        .selection_meshes()
                        .into_iter()
                        .filter_map(|m| model.art_meshes.get(m))
                        .map(|m| m.id.as_str())
                        .collect();
                    if !ids.is_empty() {
                        ctx.copy_text(ids.join(", "));
                    }
                }
            }
            "skinning" if matches!(self.selected, Some(Sel::Mesh(_))) => {
                self.open_skin_dialog(false);
            }
            "skin-params" if have => self.open_skin_dialog(true),
            "skin-path" if self.skin_path().is_some() => self.apply_skin(),
            "texture-atlas" if have => self.open_atlas(),
            "artpath-create" if have => {
                self.tool = Tool::ArtPath;
                self.art_draft = None;
            }
            "flip-h" if self.edit_target().is_some() => self.flip_selection(true),
            "flip-v" if self.edit_target().is_some() => self.flip_selection(false),
            "flip-h-only" if have => self.flip_warps_only(true),
            "merge-vertices" if have => self.merge_picked_points(),
            "flip-v-only" if have => self.flip_warps_only(false),
            "physics" if have => self.show_physics = true,
            "template-apply" if have => self.template_dlg = Some(TemplateDlg::apply(false)),
            "template-save" if have => {
                self.template_dlg =
                    Some(TemplateDlg::Save { name: String::new(), description: String::new() });
            }
            "param-create" if have => self.run_param_cmd(ParamCmd::Create(None)),
            "param-folder" if have => self.run_param_cmd(ParamCmd::CreateFolder),
            "batch-reflect" if have => self.run_param_cmd(ParamCmd::BatchReflect),
            "weight-limits" if have => self.run_param_cmd(ParamCmd::WeightLimits),
            "delete-empty-params" if have => self.run_param_cmd(ParamCmd::DeleteEmptyParams),
            "delete-empty-groups" if have => self.run_param_cmd(ParamCmd::DeleteEmptyGroups),
            "params-expand" | "params-collapse" => {
                if let Some(s) = &mut self.session {
                    let open = id == "params-expand";
                    s.groups.iter_mut().for_each(|g| g.open = open);
                }
            }
            "keys-min-max"
            | "keys-min-default-max"
            | "key-add-single"
            | "key-delete-nearest"
            | "extended-interpolation"
            | "param-edit"
            | "param-replicate"
            | "param-delete"
            | "param-link"
            | "param-group"
            | "param-reset"
            | "param-select"
            | "param-reflect"
            | "param-adjust"
            | "param-change" => self.selected_param_action(id),
            "workspace-animation" => self.set_workspace(Workspace::Animation),
            "animation-details" if have => self.open_anim_details(),
            "auto-key" => self.anim_autokey = !self.anim_autokey,
            "timelapse" if have => self.open_timelapse(),
            "workspace-save" => self.show_ws_save = true,
            "workspace-manage" => self.show_ws_manage = true,
            "layout-export" => self.pick_layout(true),
            "layout-import" => self.pick_layout(false),
            "layout-reset" => self.reset_workspace(),
            "whats-new" => self.show_whats_new = true,
            "check-updates" => self.check_updates(),
            "anim-first" | "anim-last" | "anim-prev-frame" | "anim-next-frame"
            | "anim-prev-key" | "anim-next-key" | "anim-play"
                if self.workspace == Workspace::Animation =>
            {
                self.timeline_key(id);
            }
            id if id.starts_with("anim-") && self.workspace == Workspace::Animation => {
                self.timeline.act(id);
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

    /// A shortcut on the selected parameter, as its menus would do it.
    fn selected_param_action(&mut self, id: &str) {
        let Some(pi) = self.sel_param else { return };
        let Some(s) = &self.session else { return };
        let Some(meta) = s.puppet.model().parameters.get(pi).cloned() else { return };
        let (value, linked) =
            (s.puppet.value(pi), s.combined.iter().any(|&(a, b)| a == pi || b == pi));
        let can_keys = self.edit_target().is_some();
        let cmd = match id {
            "keys-min-max" if can_keys => {
                self.add_keys_to_selection(vec![meta.min, meta.max]);
                None
            }
            "keys-min-default-max" if can_keys => {
                self.add_keys_to_selection(vec![meta.min, meta.default, meta.max]);
                None
            }
            "key-add-single" if can_keys => Some(ParamCmd::AddSingle(pi, value)),
            "key-delete-nearest" if can_keys => {
                self.delete_key_on_selection();
                None
            }
            "extended-interpolation" => Some(ParamCmd::Curve(pi)),
            "param-edit" => Some(ParamCmd::Edit(pi)),
            "param-replicate" => Some(ParamCmd::Replicate(pi)),
            "param-delete" => Some(ParamCmd::Delete(pi)),
            "param-link" => {
                Some(if linked { ParamCmd::Uncombine(pi) } else { ParamCmd::Combine(pi) })
            }
            "param-group" => Some(ParamCmd::GroupSelected(pi)),
            "param-reset" => {
                if let Some(s) = &mut self.session {
                    s.puppet.set_value(pi, meta.default);
                    s.dirty = true;
                }
                None
            }
            "param-select" => Some(ParamCmd::Select(pi)),
            "param-reflect" if can_keys => Some(ParamCmd::Reflect(pi)),
            "param-adjust" if can_keys => Some(ParamCmd::Adjust(pi)),
            "param-change" if can_keys => Some(ParamCmd::Change(pi)),
            _ => None,
        };
        if let Some(c) = cmd {
            self.run_param_cmd(c);
        }
    }

    /// New Animation or New Composition, with the menus' starting values.
    fn open_new_anim(&mut self, comp: bool) {
        self.new_anim = Some(if comp {
            AnimNew { name: "comp".into(), fps: 30.0, duration: 5.0, looped: false, comp: true }
        } else {
            AnimNew { name: "idle".into(), fps: 30.0, duration: 3.0, looped: true, comp: false }
        });
    }

    fn pick_motion(&mut self) {
        self.pick_async(PickFor::ImportMotion, || {
            rfd::FileDialog::new()
                .add_filter(crate::i18n::tr("Live2D motion"), &["json"])
                .set_title(crate::i18n::tr("Import motion3.json"))
                .pick_files()
                .map(|v| v.into_iter().next())
                .unwrap_or_default()
        });
    }

    /// Import Artwork, or Re-import Artwork over the model's own art.
    fn pick_artwork(&mut self, reimport: bool) {
        if reimport {
            self.pick_async(PickFor::ReimportPaint, || {
                rfd::FileDialog::new()
                    .add_filter(crate::i18n::tr("Artwork"), &["psd", "kra", "clip"])
                    .set_title(crate::i18n::tr("Re-import artwork"))
                    .pick_file()
            });
        } else {
            self.pick_async(PickFor::ImportPaint, || {
                rfd::FileDialog::new()
                    .add_filter(crate::i18n::tr("Artwork"), &["psd", "kra", "clip"])
                    .set_title(crate::i18n::tr("Import artwork into this model"))
                    .pick_file()
            });
        }
    }

    /// Export Current Layout, or Import Layout.
    fn pick_layout(&mut self, export: bool) {
        if export {
            self.pick_async(PickFor::WorkspaceExport, || {
                rfd::FileDialog::new()
                    .add_filter(crate::i18n::tr("Workspace"), &["json"])
                    .set_file_name("workspace.json")
                    .save_file()
            });
        } else {
            self.pick_async(PickFor::WorkspaceImport, || {
                rfd::FileDialog::new()
                    .add_filter(crate::i18n::tr("Workspace"), &["json"])
                    .pick_file()
            });
        }
    }

    fn check_updates(&mut self) {
        self.update.start();
        self.update_win = crate::whats_new::UpdateWindow::default();
        self.prefs.update_dismissed = None;
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
        if show_all {
            log::info!("all meshes shown");
        } else {
            log::info!("meshes hidden but the {} selected", keep.len());
        }
    }

    /// Edit Form > Flip on the selected mesh(es) or warp, one undo step.
    fn flip_selection(&mut self, horizontal: bool) {
        use ayatsuri_core::edit::Target;
        // a selected deformer flips with its pick, never with meshes left
        // picked from before
        let defs = def_group(self.selected, &self.def_pick);
        let targets: Vec<Target> = if defs.is_empty() {
            self.selection_meshes().into_iter().map(Target::ArtMesh).collect()
        } else {
            defs.into_iter().map(Target::Deformer).collect()
        };
        let Some(s) = &mut self.session else { return };
        // several objects flip as one, across the middle of them all: each
        // mirrors across its own centre, then moves to its mirrored place
        let shifts: Vec<(Target, f32, f32)> = if targets.len() > 1 {
            let centres: Vec<(Target, Option<Extent>)> =
                targets.iter().map(|&t| (t, world_extent(&s.puppet, t))).collect();
            let all = centres.iter().filter_map(|(_, e)| *e).fold(None, |acc, e| {
                Some(match acc {
                    None => e,
                    Some((a, b, c, d)) => {
                        (f32::min(a, e.0), f32::min(b, e.1), f32::max(c, e.2), f32::max(d, e.3))
                    }
                })
            });
            match all {
                Some((x0, y0, x1, y1)) => centres
                    .into_iter()
                    .filter_map(|(t, e)| {
                        let (ex0, ey0, ex1, ey1) = e?;
                        let (gx, gy) = ((x0 + x1) / 2.0, (y0 + y1) / 2.0);
                        let (ox, oy) = ((ex0 + ex1) / 2.0, (ey0 + ey1) / 2.0);
                        let (wx, wy) = if horizontal {
                            (2.0 * (gx - ox), 0.0)
                        } else {
                            (0.0, 2.0 * (gy - oy))
                        };
                        let (parent, (px, py)) = local_centre(&s.puppet, t)?;
                        let (dx, dy) = parent_delta(&s.puppet, parent, px, py, wx, wy);
                        Some((t, dx, dy))
                    })
                    .collect(),
                None => Vec::new(),
            }
        } else {
            Vec::new()
        };
        let mut n = 0;
        s.edit(|m| {
            for &t in &targets {
                n += ayatsuri_core::edit::flip_object(m, t, horizontal) as usize;
            }
            for &(t, dx, dy) in &shifts {
                match t {
                    Target::ArtMesh(i) => {
                        let count = m.art_meshes.get(i).map_or(0, |a| a.vertex_count as usize);
                        let moves: Vec<(usize, f32, f32)> =
                            (0..count).map(|v| (v, dx, dy)).collect();
                        ayatsuri_core::edit::move_vertices(m, i, &moves);
                    }
                    Target::Deformer(d) => match m.deformers.get(d).map(|x| &x.kind) {
                        Some(DeformerKind::Warp(w)) => {
                            let moves: Vec<(usize, f32, f32)> =
                                (0..w.grid_points()).map(|p| (p, dx, dy)).collect();
                            ayatsuri_core::edit::move_warp_points(m, d, None, &moves);
                        }
                        Some(DeformerKind::Rotation(_)) => {
                            ayatsuri_core::edit::move_rotation_origin(m, d, dx, dy);
                        }
                        None => {}
                    },
                    Target::Part(_) => {}
                }
            }
            n > 0
        });
        if n == 0 {
            log::warn!("flip works on meshes and deformers");
        } else {
            log::info!(
                "flipped {n} object(s) {}",
                if horizontal { "horizontally" } else { "vertically" }
            );
        }
    }

    /// Flip from a mesh row's menu: the pick it is in, or the mesh alone.
    fn flip_mesh_row(&mut self, mi: usize, horizontal: bool) {
        let pick = self.mesh_pick_with(mi);
        if pick.len() < 2 {
            self.multi_sel = vec![mi];
        }
        self.selected = Some(Sel::Mesh(mi));
        self.def_pick.clear();
        self.flip_selection(horizontal);
    }

    /// Flip only the selected warps (the whole pick), each across its own
    /// centre, while what they hold stays where it is.
    fn flip_warps_only(&mut self, horizontal: bool) {
        let defs = def_group(self.selected, &self.def_pick);
        let Some(s) = &mut self.session else { return };
        let values: Vec<f32> =
            (0..s.puppet.model().parameters.len()).map(|i| s.puppet.value(i)).collect();
        let plan: Vec<(usize, usize)> = defs
            .into_iter()
            .filter(|&d| {
                matches!(
                    s.puppet.model().deformers.get(d).map(|x| &x.kind),
                    Some(DeformerKind::Warp(_))
                )
            })
            .filter_map(|d| {
                let t = ayatsuri_core::edit::Target::Deformer(d);
                Some((d, ayatsuri_core::keyform::keyform_at(s.puppet.model(), t, &values)?))
            })
            .collect();
        if plan.is_empty() {
            log::warn!("select a warp deformer, with the parameters on a key, to flip it alone");
            return;
        }
        let n = plan.len();
        if s.edit(|m| {
            let mut any = false;
            for &(d, k) in &plan {
                any |= ayatsuri_core::edit::flip_warp_keep_children(m, d, k, horizontal);
            }
            any
        }) {
            log::info!(
                "flipped {n} warp(s) alone {}",
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
        let defs = def_group(self.selected, &self.def_pick);
        let all = !defs.is_empty() && defs.iter().all(|d| self.def_locked.contains(d));
        for d in defs {
            if all {
                self.def_locked.remove(&d);
            } else {
                self.def_locked.insert(d);
            }
        }
        log::info!("{} locked mesh(es)", self.mesh_locked.len());
    }

    fn toggle_selected_visibility(&mut self) {
        let meshes = self.selection_meshes();
        let defs = def_group(self.selected, &self.def_pick);
        if let Some(s) = &mut self.session {
            for d in defs {
                if let Some(f) = s.deformer_shown.get_mut(d) {
                    *f = !*f;
                    s.puppet.set_deformer_visible(d, *f);
                    s.dirty = true;
                }
            }
            log::info!("visibility toggled on {} mesh(es)", meshes.len());
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
        let marks = self.key_marks();
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
                        let (rect, resp) = ui.allocate_exact_size(
                            vec2(ui.available_width(), 18.0),
                            if full { Sense::click_and_drag() } else { Sense::click() },
                        );
                        // a folder's colour fills its whole header, as in
                        // Cubism; parameters keep the thin stripe
                        let color = s.groups.get(gi).and_then(|g| g.color);
                        let (fill, ink) = match color {
                            Some([r, g, b]) => {
                                (Color32::from_rgb(r, g, b), crate::color_pick::text_on([r, g, b]))
                            }
                            None => (theme::pal().header, theme::pal().text),
                        };
                        ui.painter().rect_filled(rect, 2.0, fill);
                        let arrow = if open { "⏷" } else { "⏵" };
                        ui.painter().text(
                            rect.left_center() + vec2(6.0, 0.0),
                            Align2::LEFT_CENTER,
                            format!("{arrow} {name}"),
                            FontId::proportional(11.0),
                            ink,
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
                            // drag a folder by its header; a parameter dropped
                            // here goes into it, a folder lands before it
                            let mut drop = None;
                            let here = param_menu::Moving::Group(gi);
                            param_menu::drag_target(
                                ui,
                                &resp,
                                rect,
                                here,
                                param_menu::Row::Folder { group: gi, open },
                                &mut drop,
                            );
                            if let Some(c) = drop {
                                actions.push(ParamAction::Cmd(c));
                            }
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
                            &marks,
                            &mut actions,
                        );
                    }
                }
                // what is being dragged follows the pointer
                if let Some(m) = egui::DragAndDrop::payload::<param_menu::Moving>(ui.ctx())
                    && let Some(at) = ui.ctx().pointer_latest_pos()
                {
                    let label = match *m {
                        param_menu::Moving::Param(p) => s
                            .param_names
                            .get(p)
                            .filter(|n| !n.is_empty())
                            .cloned()
                            .or_else(|| s.puppet.model().parameters.get(p).map(|x| x.id.clone()))
                            .unwrap_or_default(),
                        param_menu::Moving::Group(g) => {
                            s.groups.get(g).map(|g| g.name.clone()).unwrap_or_default()
                        }
                    };
                    egui::Area::new(egui::Id::new("param-drag-ghost"))
                        .order(egui::Order::Tooltip)
                        .interactable(false)
                        .fixed_pos(at + vec2(14.0, 6.0))
                        .show(ui.ctx(), |ui| {
                            egui::Frame::popup(ui.style()).show(ui, |ui| ui.label(label));
                        });
                }
                // the empty space under the rows takes the palette's own
                // right-click menu (create parameter or folder and so on)
                if full {
                    let rest = ui.available_height().clamp(40.0, list_h.max(40.0));
                    let (bg_rect, bg) =
                        ui.allocate_exact_size(vec2(ui.available_width(), rest), Sense::click());
                    bg.context_menu(|ui| {
                        if let Some(c) = param_menu::panel_menu(ui, &mut s, &sc) {
                            actions.push(ParamAction::Cmd(c));
                        }
                    });
                    // the space under the last row is the end of the list,
                    // outside every folder
                    if bg.dnd_hover_payload::<param_menu::Moving>().is_some() {
                        ui.painter().hline(
                            bg_rect.x_range(),
                            bg_rect.top(),
                            Stroke::new(2.0, theme::pal().accent),
                        );
                    }
                    if let Some(m) = bg.dnd_release_payload::<param_menu::Moving>() {
                        actions.push(ParamAction::Cmd(ParamCmd::Drop(*m, param_menu::Place::End)));
                    }
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
                    blend_shape: false,
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
        // Cubism greys the palette out while the temporary deform tool is bound
        if self.tdeform.is_some() {
            ui.disable();
        }
        if self.session.is_none() {
            ui.weak(crate::i18n::tr("No model loaded."));
            return;
        }
        let field =
            search_box(ui, &mut self.part_search, crate::i18n::tr("Search parts and meshes"));
        if std::mem::take(&mut self.focus_search)
            && let Some(f) = field
        {
            f.request_focus();
        }
        let mut take_new: Option<bool> = None;
        let mut delete_parts: Option<(Vec<usize>, bool)> = None;
        ui.horizontal(|ui| {
            let new = ui.small_button(egui_phosphor::regular::FOLDER_PLUS).on_hover_text(
                crate::i18n::tr("New Part. Shift+click puts the selected objects in it"),
            );
            if new.clicked() {
                let picked = self.part_pick.len() + self.multi_sel.len();
                take_new = Some(picked > 1 || ui.input(|i| i.modifiers.shift));
            }
            let lock = if self.parts_dnd_off {
                egui_phosphor::regular::LOCK_SIMPLE
            } else {
                egui_phosphor::regular::HAND_GRABBING
            };
            if ui
                .selectable_label(self.parts_dnd_off, lock)
                .on_hover_text(crate::i18n::tr("Prohibit drag and drop"))
                .clicked()
            {
                self.parts_dnd_off = !self.parts_dnd_off;
            }
        });
        if let Some(take) = take_new {
            self.open_new_part(take);
        }
        let rename_sc = self.sc("rename");
        let Some(s) = &mut self.session else { return };
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
        for (i, part) in s.puppet.model().parts.iter().enumerate() {
            if let Some(p) = part.parent_part
                && let Some(c) = children.get_mut(p as usize)
            {
                c.push(i);
            }
        }
        // the palette's rows as Cubism orders them (front-most first), less
        // what a search hides and what sits in a folded folder
        let mut visible: Vec<(PRow, usize)> = Vec::new();
        let mut folded_at: Option<usize> = None;
        for (row, depth) in ayatsuri_core::palette::rows(s.puppet.model()) {
            if folded_at.is_some_and(|d| depth > d) {
                continue;
            }
            folded_at = None;
            let keep = match row {
                PRow::Part(i) => part_shown_by_search.get(i).copied().unwrap_or(true),
                PRow::Mesh(mi) => mesh_shown_by_search.get(mi).copied().unwrap_or(true),
            };
            if !keep {
                continue;
            }
            if let PRow::Part(i) = row
                && !searching
                && self.part_collapsed.contains(&i)
            {
                folded_at = Some(depth);
            }
            visible.push((row, depth));
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
        let part_rows: Vec<usize> = visible
            .iter()
            .filter_map(|(r, _)| match r {
                PRow::Part(i) => Some(*i),
                PRow::Mesh(_) => None,
            })
            .collect();
        let mesh_rows: Vec<usize> = visible
            .iter()
            .filter_map(|(r, _)| match r {
                PRow::Mesh(mi) => Some(*mi),
                PRow::Part(_) => None,
            })
            .collect();
        let dnd = !self.parts_dnd_off && self.mesh_edit.is_none();
        let mut drop_cmd: Option<(Dragged, PDrop)> = None;
        let mut part_click: Option<usize> = None;
        let mut group_part: Option<usize> = None;
        let mut rename_part: Option<usize> = None;
        let mut part_double = false;
        ScrollArea::vertical().id_salt("parts").show(ui, |ui| {
            for (row, depth) in visible {
                let i = match row {
                    PRow::Part(i) => i,
                    PRow::Mesh(mi) => {
                        let x_off =
                            if depth == 0 { 30.0 } else { 66.0 + (depth - 1) as f32 * 14.0 };
                        let parent =
                            s.puppet.model().art_meshes.get(mi).and_then(|m| m.parent_part);
                        let effective = ancestors_shown(s, parent);
                        let mid = s.mesh_name(mi);
                        let resp = parts_mesh_row(
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
                            dnd,
                            &rename_sc,
                        );
                        if dnd {
                            palette_drop_target(ui, &resp, row, false, &mut drop_cmd);
                        }
                        continue;
                    }
                };
                let Some((id, parent)) =
                    s.puppet.model().parts.get(i).map(|p| (p.id.clone(), p.parent_part))
                else {
                    continue;
                };
                let name = s.part_names.get(i).cloned().unwrap_or(id);
                let shown = s.part_shown.get(i).copied().unwrap_or(true);
                let row_w = ui.available_width();
                let sense = if dnd { Sense::click_and_drag() } else { Sense::click() };
                let (rect, resp) = ui.allocate_exact_size(vec2(row_w, 18.0), sense);
                if dnd && resp.drag_started() {
                    let parts =
                        if self.part_pick.contains(&i) { self.part_pick.clone() } else { vec![i] };
                    resp.dnd_set_drag_payload(Dragged::Parts(parts));
                }
                if self.selected == Some(Sel::Part(i)) || self.part_pick.contains(&i) {
                    paint_picked_row(ui, rect);
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
                // another frame only for a row on screen still waiting for its
                // preview (a few are made per frame); rows scrolled out of
                // view wait until they are shown, or the editor never rests
                if !s.part_previews.contains_key(&i) && ui.is_rect_visible(thumb) {
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
                if resp.double_clicked() {
                    part_double = true;
                } else if resp.clicked() {
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
                    ui.separator();
                    let picked = self.part_pick.contains(&i);
                    let n = if picked { self.part_pick.len() + self.multi_sel.len() } else { 1 };
                    let label = if n > 1 {
                        crate::i18n::trf("Group {count} Objects into New Part…", &[("count", &n)])
                    } else {
                        crate::i18n::tr("Group into New Part…").to_string()
                    };
                    if ui.button(label).clicked() {
                        group_part = Some(i);
                        ui.close();
                    }
                    if ui
                        .add(
                            egui::Button::new(crate::i18n::tr("Rename…")).shortcut_text(&rename_sc),
                        )
                        .clicked()
                    {
                        rename_part = Some(i);
                        ui.close();
                    }
                    ui.separator();
                    if ui
                        .add(egui::Button::new(crate::i18n::tr("Delete")).shortcut_text("Del"))
                        .clicked()
                    {
                        delete_parts = Some((targets.clone(), false));
                        ui.close();
                    }
                    if ui
                        .button(crate::i18n::tr("Delete Part (Child objects will remain)"))
                        .clicked()
                    {
                        delete_parts = Some((targets.clone(), true));
                        ui.close();
                    }
                });

                if dnd {
                    let open = !self.part_collapsed.contains(&i);
                    palette_drop_target(ui, &resp, row, open, &mut drop_cmd);
                }
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
        if let Some((what, to)) = drop_cmd {
            self.palette_move(what, to);
        }
        let (shift, ctrl) = ui.input(|i| (i.modifiers.shift, i.modifiers.command));
        if part_double {
            self.deselect_all();
        } else if let Some(i) = part_click {
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
            Some(MeshRowCmd::Rename(mi)) => self.open_mesh_rename(mi),
            Some(MeshRowCmd::Flip(mi, h)) => self.flip_mesh_row(mi, h),
            Some(MeshRowCmd::Group(mi)) => {
                // a mesh outside the pick groups alone
                if !self.multi_sel.contains(&mi) {
                    self.multi_sel = vec![mi];
                    self.part_pick.clear();
                }
                self.selected = Some(Sel::Mesh(mi));
                self.open_new_part(true);
            }
            None => {}
        }
        if let Some(i) = rename_part {
            self.open_part_rename(i);
        }
        if let Some((parts, keep)) = delete_parts {
            self.delete_parts(parts, keep);
        }
        if let Some(i) = group_part {
            if !self.part_pick.contains(&i) {
                self.part_pick = vec![i];
                self.multi_sel.clear();
            }
            self.selected = Some(Sel::Part(i));
            self.open_new_part(true);
        }
    }

    /// Nothing selected: no part, deformer, mesh or glue, and no pick, so
    /// no key can be made on an object by mistake. A double-click on a
    /// Parts or Deformer row does this (the first click selects the row,
    /// the second lets everything go).
    fn deselect_all(&mut self) {
        self.selected = None;
        self.selected_glue = None;
        self.part_pick.clear();
        self.part_anchor = None;
        self.def_pick.clear();
        self.def_anchor = None;
        self.multi_sel.clear();
        self.mesh_anchor = None;
        log::info!("selection cleared from the palette");
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
        pick_in(pick, anchor, row, shift, ctrl, order);
    }

    /// Shift+click in the Deformer palette when the last clicked row was the
    /// other kind: every row of the clicked kind between the two is picked
    /// (deformers as the outermost of them). False when the anchor is the
    /// same kind, or gone, and the usual range applies.
    fn pick_across(&mut self, deformer: bool, row: usize, all_rows: &[(bool, usize)]) -> bool {
        let Some(anchor) = self.def_row_anchor.filter(|a| a.0 != deformer) else {
            return false;
        };
        let at = |r: (bool, usize)| all_rows.iter().position(|&x| x == r);
        let (Some(a), Some(b)) = (at(anchor), at((deformer, row))) else {
            return false;
        };
        let (lo, hi) = (a.min(b), a.max(b));
        let run: Vec<usize> = all_rows
            .get(lo..=hi)
            .unwrap_or_default()
            .iter()
            .filter(|r| r.0 == deformer)
            .map(|r| r.1)
            .collect();
        if deformer {
            self.def_pick = match &self.session {
                Some(s) => top_level_pick(s.puppet.model(), &run),
                None => run,
            };
            self.def_anchor = Some(row);
            self.selected = if self.def_pick.contains(&row) {
                Some(Sel::Deformer(row))
            } else {
                self.def_pick.last().copied().map(Sel::Deformer)
            };
            self.multi_sel.clear();
            self.part_pick.clear();
        } else {
            self.multi_sel = run;
            self.mesh_anchor = Some(row);
            self.selected = Some(Sel::Mesh(row));
            self.def_pick.clear();
        }
        log::debug!(
            "Deformer palette: Shift+click picked {} {} across both kinds of row",
            if deformer { self.def_pick.len() } else { self.multi_sel.len() },
            if deformer { "deformer(s)" } else { "mesh(es)" }
        );
        true
    }

    /// A deformer row clicked with Ctrl or Shift (or plainly): the pick
    /// starts from the selected deformer, and the selection follows.
    fn pick_def_rows(&mut self, row: usize, shift: bool, ctrl: bool, order: &[usize]) {
        let current = match self.selected {
            Some(Sel::Deformer(d)) => Some(d),
            _ => None,
        };
        if let Some(d) = current {
            if !self.def_pick.contains(&d) {
                self.def_pick = vec![d];
                self.def_anchor = Some(d);
            }
        } else {
            self.def_pick.clear();
            self.def_anchor = None;
        }
        pick_in(&mut self.def_pick, &mut self.def_anchor, row, shift, ctrl, order);
        if let Some(s) = &self.session {
            self.def_pick = top_level_pick(s.puppet.model(), &self.def_pick);
        }
        self.selected = if self.def_pick.contains(&row) {
            Some(Sel::Deformer(row))
        } else {
            self.def_pick.last().copied().map(Sel::Deformer)
        };
        if self.def_pick.len() > 1 {
            self.multi_sel.clear();
            self.part_pick.clear();
        }
    }

    /// Run what was picked on the canvas menu.
    fn run_canvas_cmd(&mut self, ctx: &egui::Context, cmd: CanvasCmd) {
        // an object outside the selection is selected alone first
        let take = |ed: &mut Self, mi: usize| {
            if !ed.selection_meshes().contains(&mi) {
                ed.selected = Some(Sel::Mesh(mi));
                ed.multi_sel = vec![mi];
                ed.def_pick.clear();
            }
        };
        match cmd {
            CanvasCmd::MergePoints => self.merge_picked_points(),
            CanvasCmd::DeletePoints => {
                if let Some(mi) = self.mesh_edit {
                    let points: Vec<usize> = self.mesh_picked.iter().copied().collect();
                    let Some(s) = &mut self.session else { return };
                    if s.edit(|m| ayatsuri_core::edit::delete_vertices(m, mi, &points)) {
                        self.mesh_picked.clear();
                        self.pending_edges.remove(&mi);
                        self.edge_first = None;
                        log::info!("{} point(s) deleted", points.len());
                    }
                }
            }
            CanvasCmd::Mesh(a) => self.run_mesh_action(a),
            CanvasCmd::FinishMeshEdit => self.run_key_action(ctx, "mesh-edit"),
            CanvasCmd::EditMesh(mi) => {
                self.selected = Some(Sel::Mesh(mi));
                self.multi_sel = vec![mi];
                self.run_key_action(ctx, "mesh-edit");
            }
            CanvasCmd::RenameMesh(mi) => self.open_mesh_rename(mi),
            CanvasCmd::ClipBy(mi) => {
                take(self, mi);
                self.start_clip_pick(mi);
            }
            CanvasCmd::CopyIds(mi) => {
                take(self, mi);
                self.run_key_action(ctx, "copy-ids");
            }
            CanvasCmd::CreateAround(mi, warp) => {
                take(self, mi);
                self.pending_create = Some((warp, None));
            }
            CanvasCmd::SelectParent(mi) => {
                self.selected = Some(Sel::Mesh(mi));
                self.select_parent();
            }
            CanvasCmd::Flip(mi, h) => self.flip_mesh_row(mi, h),
            CanvasCmd::ToggleShown(mi) => {
                take(self, mi);
                self.toggle_selected_visibility();
            }
            CanvasCmd::ToggleLock(mi) => {
                take(self, mi);
                self.toggle_selected_lock();
            }
            CanvasCmd::DeleteMeshes(mi) => {
                let meshes = self.mesh_pick_with(mi);
                self.delete_meshes(meshes);
            }
            CanvasCmd::Def(c) => self.run_def_cmd(c),
        }
    }

    /// Merge vertices on the points picked in Mesh Edit.
    fn merge_picked_points(&mut self) {
        let Some(mi) = self.mesh_edit else {
            log::warn!("merge vertices: open the mesh in Mesh Edit and pick the points first");
            return;
        };
        let points: Vec<usize> = self.mesh_picked.iter().copied().collect();
        if points.len() < 2 {
            log::warn!("merge vertices: pick two or more points (Shift+click or a box)");
            return;
        }
        let Some(s) = &mut self.session else { return };
        if s.edit(|m| ayatsuri_core::edit::merge_vertices(m, mi, &points)) {
            self.mesh_picked.clear();
            self.pending_edges.remove(&mi);
            self.edge_first = None;
            log::info!("{} point(s) merged into one", points.len());
        }
    }

    /// Let go of selected objects the model no longer has (an undo or
    /// redo took them away), so nothing acts on an index that is gone.
    fn drop_stale_selection(&mut self) {
        let Some(s) = &self.session else { return };
        let m = s.puppet.model();
        let (meshes, defs, parts, params) =
            (m.art_meshes.len(), m.deformers.len(), m.parts.len(), m.parameters.len());
        let gone = match self.selected {
            Some(Sel::Mesh(i)) => i >= meshes,
            Some(Sel::Deformer(i)) => i >= defs,
            Some(Sel::Part(i)) => i >= parts,
            Some(Sel::Parameter(i)) => i >= params,
            None => false,
        };
        if gone {
            log::debug!("selection: {:?} is gone from the model", self.selected);
            self.selected = None;
        }
        self.multi_sel.retain(|&i| i < meshes);
        self.def_pick.retain(|&i| i < defs);
        self.part_pick.retain(|&i| i < parts);
    }

    /// Start picking `mesh`'s clipping masks on the canvas.
    fn start_clip_pick(&mut self, mesh: usize) {
        // a mesh in a pick clips the whole pick, which stays selected
        let pick = self.mesh_pick_with(mesh);
        if pick.len() < 2 {
            self.multi_sel.clear();
        }
        self.selected = Some(Sel::Mesh(mesh));
        self.tool = Tool::Arrow;
        self.mesh_edit = None;
        self.clip_pick = Some(mesh);
        self.clip_also = pick.into_iter().filter(|&m| m != mesh).collect();
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
        self.remap_mesh_state(&remap);
    }

    /// Delete parts, with what is inside them or (`keep_children`) leaving
    /// it to their parents; meshes picked with them go too.
    fn delete_parts(&mut self, parts: Vec<usize>, keep_children: bool) {
        if self.mesh_edit.is_some() {
            log::warn!("leave Mesh Edit before deleting parts");
            return;
        }
        let Some(s) = &mut self.session else { return };
        let names: Vec<String> = parts
            .iter()
            .map(|&p| {
                s.part_names.get(p).filter(|n| !n.is_empty()).cloned().unwrap_or_else(|| {
                    s.puppet.model().parts.get(p).map(|x| x.id.clone()).unwrap_or_default()
                })
            })
            .collect();
        let meshes = self.multi_sel.clone();
        let ids: Vec<String> = s.puppet.model().art_meshes.iter().map(|m| m.id.clone()).collect();
        if !s.edit(|m| {
            // meshes first: they do not move part numbers
            let any = ayatsuri_core::edit::delete_art_meshes(m, &meshes);
            ayatsuri_core::palette::delete_parts(m, &parts, keep_children) || any
        }) {
            return;
        }
        log::info!(
            "deleted part(s) {}{}",
            names.join(", "),
            if keep_children { ", children kept" } else { "" }
        );
        let now: std::collections::HashMap<&str, usize> = s
            .puppet
            .model()
            .art_meshes
            .iter()
            .enumerate()
            .map(|(i, m)| (m.id.as_str(), i))
            .collect();
        let moved: Vec<Option<usize>> =
            ids.iter().map(|id| now.get(id.as_str()).copied()).collect();
        self.selected = None;
        self.part_pick.clear();
        self.part_anchor = None;
        self.part_collapsed.clear();
        self.def_pick.clear();
        self.def_anchor = None;
        self.def_locked.clear();
        self.def_collapsed.clear();
        self.selected_glue = None;
        self.remap_mesh_state(&|i| moved.get(i).copied().flatten());
    }

    /// Renumber what the editor keeps per mesh after the meshes moved:
    /// `remap` gives each old index its new one, or None when it is gone.
    fn remap_mesh_state(&mut self, remap: &dyn Fn(usize) -> Option<usize>) {
        self.selected = match self.selected {
            Some(Sel::Mesh(m)) => remap(m).map(Sel::Mesh),
            other => other,
        };
        self.multi_sel = self.multi_sel.iter().filter_map(|&m| remap(m)).collect();
        self.mesh_anchor = self.mesh_anchor.and_then(remap);
        self.clip_pick = self.clip_pick.and_then(remap);
        self.clip_also = self.clip_also.iter().filter_map(|&m| remap(m)).collect();
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

    /// A Parts palette drop: the rows move (a dragged mesh brings the pick
    /// it is part of) and everything kept per mesh follows the renumbering.
    fn palette_move(&mut self, what: Dragged, to: PDrop) {
        let rows: Vec<PRow> = match what {
            Dragged::Parts(p) => p.into_iter().map(PRow::Part).collect(),
            Dragged::Mesh(mi) => self.mesh_pick_with(mi).into_iter().map(PRow::Mesh).collect(),
        };
        let Some(s) = &mut self.session else { return };
        let ids: Vec<String> = s.puppet.model().art_meshes.iter().map(|m| m.id.clone()).collect();
        if !s.edit(|m| ayatsuri_core::palette::move_rows(m, &rows, to)) {
            return;
        }
        let now: std::collections::HashMap<String, usize> = s
            .puppet
            .model()
            .art_meshes
            .iter()
            .enumerate()
            .map(|(i, m)| (m.id.clone(), i))
            .collect();
        log::info!("parts palette: moved {} row(s) to {to:?}", rows.len());
        self.remap_mesh_state(&|i| ids.get(i).and_then(|id| now.get(id).copied()));
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
                    let key = tool_key(t).map(|a| self.sc(a)).unwrap_or_default();
                    if key.is_empty() {
                        resp.on_hover_text(crate::i18n::tr(t.label()))
                    } else {
                        resp.on_hover_text(format!("{} ({key})", crate::i18n::tr(t.label())))
                    }
                };
                if resp.clicked() && !locked {
                    self.choose_tool(t);
                }
            }
        }
    }

    /// A tool picked on the toolbar. Warp and Rotation Deformer open their
    /// creation dialog at once, as Cubism's buttons do; the dialog follows
    /// the selection while it is open, so picking what goes inside can come
    /// before or after.
    fn choose_tool(&mut self, t: Tool) {
        self.tool = t;
        if matches!(t, Tool::WarpCreate | Tool::RotationCreate) && self.session.is_some() {
            self.pending_create = Some((t == Tool::WarpCreate, None));
        }
    }

    fn deformers_panel(&mut self, ui: &mut Ui) {
        if self.tdeform.is_some() {
            ui.disable();
        }
        let mut def_cmd: Option<DefCmd> = None;
        let rename_sc = self.sc("rename");
        let picked_defs = def_group(self.selected, &self.def_pick);
        let mut def_rows: Vec<usize> = Vec::new();
        let mut def_click: Option<usize> = None;
        let mut def_mesh_rows: Vec<usize> = Vec::new();
        let mut def_mesh_click: Option<usize> = None;
        // every row top to bottom, deformers (true) and meshes
        let mut all_rows: Vec<(bool, usize)> = Vec::new();
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
        let mut def_double = false;
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
            for (i, dep, _, is_warp, has_kids) in rows {
                let (rect, resp) = ui
                    .allocate_exact_size(vec2(ui.available_width(), 18.0), Sense::click_and_drag());
                if self.selected == Some(Sel::Deformer(i)) || picked_defs.contains(&i) {
                    paint_picked_row(ui, rect);
                }
                def_rows.push(i);
                all_rows.push((true, i));
                // drag a row onto a deformer to make it the parent
                if resp.drag_started() && !self.def_locked.contains(&i) {
                    resp.dnd_set_drag_payload(Child::Deformer(i));
                }
                // as in the Parts palette: the edges drop beside this
                // deformer (under its parent), the middle drops into it
                let row_parent = s.puppet.model().deformers.get(i).and_then(|d| d.parent_deformer);
                let open = has_kids && !self.def_collapsed.contains(&i);
                def_drop_target(ui, &resp, Some(i), row_parent, open, &mut def_cmd);
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
                // another frame only for a row on screen still waiting for its
                // preview (a few are made per frame); rows scrolled out of
                // view wait until they are shown, or the editor never rests
                if !s.deformer_previews.contains_key(&i) && ui.is_rect_visible(thumb) {
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
                    truncate(&s.deformer_name(i), 22),
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
                if resp.double_clicked() {
                    def_double = true;
                } else if resp.clicked() {
                    def_click = Some(i);
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
                    if ui
                        .add(
                            egui::Button::new(crate::i18n::tr("Rename…")).shortcut_text(&rename_sc),
                        )
                        .clicked()
                    {
                        def_cmd = Some(DefCmd::Rename(i));
                        ui.close();
                    }
                    ui.menu_button(crate::i18n::tr("Flip"), |ui| {
                        for (label, h) in [("Flip Horizontally", true), ("Flip Vertically", false)]
                        {
                            if ui.button(crate::i18n::tr(label)).clicked() {
                                def_cmd = Some(DefCmd::Flip(i, h, false));
                                ui.close();
                            }
                        }
                        if is_warp {
                            ui.separator();
                            for (label, h) in [
                                ("Flip Warp Only Horizontally", true),
                                ("Flip Warp Only Vertically", false),
                            ] {
                                if ui.button(crate::i18n::tr(label)).clicked() {
                                    def_cmd = Some(DefCmd::Flip(i, h, true));
                                    ui.close();
                                }
                            }
                        }
                    });
                    if is_warp
                        && ui
                            .button(crate::i18n::tr("Conversion Divisions…"))
                            .on_hover_text(crate::i18n::tr(
                                "Change its columns and rows; its shape is kept",
                            ))
                            .clicked()
                    {
                        def_cmd = Some(DefCmd::Division(i));
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
                    // dropping beside a mesh puts it under the mesh's deformer
                    def_drop_target(ui, &resp, None, Some(i as u32), false, &mut def_cmd);
                    def_mesh_rows.push(mi);
                    all_rows.push((false, mi));
                    if self.selected == Some(Sel::Mesh(mi)) || self.multi_sel.contains(&mi) {
                        paint_picked_row(ui, rect);
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
                        if ui
                            .add(
                                egui::Button::new(crate::i18n::tr("Rename…"))
                                    .shortcut_text(&rename_sc),
                            )
                            .clicked()
                        {
                            def_cmd = Some(DefCmd::RenameMesh(mi));
                            ui.close();
                        }
                        ui.menu_button(crate::i18n::tr("Flip"), |ui| {
                            for (label, h) in
                                [("Flip Horizontally", true), ("Flip Vertically", false)]
                            {
                                if ui.button(crate::i18n::tr(label)).clicked() {
                                    def_cmd = Some(DefCmd::FlipMesh(mi, h));
                                    ui.close();
                                }
                            }
                        });
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
        if def_double {
            self.deselect_all();
        } else if let Some(i) = def_click {
            let (shift, ctrl) = ui.ctx().input(|i| (i.modifiers.shift, i.modifiers.command));
            if !(shift && self.pick_across(true, i, &all_rows)) {
                self.pick_def_rows(i, shift, ctrl, &def_rows);
            }
            if !shift {
                self.def_row_anchor = Some((true, i));
            }
        }
        // what is being dragged follows the pointer
        if let Some(p) = egui::DragAndDrop::payload::<Child>(&ctx)
            && let Some(at) = ctx.pointer_latest_pos()
            && let Some(s) = &self.session
        {
            let lines: Vec<String> = match *p {
                Child::Deformer(d)
                    if def_group(self.selected, &self.def_pick).len() > 1
                        && self.def_pick.contains(&d) =>
                {
                    let n = self.def_pick.len();
                    vec![crate::i18n::trf("{count} deformers", &[("count", &n)])]
                }
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
            if shift && self.pick_across(false, mi, &all_rows) {
                self.after_mesh_pick(mi, ctrl, shift);
            } else {
                self.pick_rows(mi, shift, ctrl, &def_mesh_rows, false);
                self.after_mesh_pick(mi, ctrl, shift);
            }
            if !shift {
                self.def_row_anchor = Some((false, mi));
            }
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
            DefCmd::Rename(i) => self.open_def_rename(i),
            DefCmd::RenameMesh(i) => self.open_mesh_rename(i),
            DefCmd::Division(i) => self.open_def_division(i),
            DefCmd::FlipMesh(mi, h) => self.flip_mesh_row(mi, h),
            DefCmd::Flip(i, horizontal, alone) => {
                // a deformer outside the pick flips by itself
                if !def_group(self.selected, &self.def_pick).contains(&i) {
                    self.selected = Some(Sel::Deformer(i));
                    self.def_pick = vec![i];
                    self.multi_sel.clear();
                }
                if alone {
                    self.flip_warps_only(horizontal);
                } else {
                    self.flip_selection(horizontal);
                }
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
                    Child::Deformer(d) if def_group(self.selected, &self.def_pick).contains(&d) => {
                        def_group(self.selected, &self.def_pick)
                            .into_iter()
                            .map(Child::Deformer)
                            .collect()
                    }
                    other => vec![other],
                };
                // re-parenting can re-sort deformers: the pick follows by id
                let picked_ids: Vec<String> = self.def_pick.iter().filter_map(id_of).collect();
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
                    self.def_pick = picked_ids.iter().filter_map(index_of).collect();
                    if count > 1 {
                        log::info!("re-parented {count} objects under {parent:?}");
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
                // a deformer in the pick deletes the whole pick, last first
                // so the earlier indices hold, as one undo step
                let group = def_group(self.selected, &self.def_pick);
                let mut gone = if group.contains(&i) { group } else { vec![i] };
                gone.sort_unstable_by(|a, b| b.cmp(a));
                let Some(s) = &mut self.session else { return };
                let names: Vec<String> = gone
                    .iter()
                    .filter_map(|&d| s.puppet.model().deformers.get(d).map(|d| d.id.clone()))
                    .collect();
                if s.edit(|m| {
                    let mut any = false;
                    for &d in &gone {
                        any |= ayatsuri_core::edit::delete_deformer(m, d);
                    }
                    any
                }) {
                    log::info!("deleted deformer(s) {}", names.join(", "));
                    // indices past each one moved down
                    let fix = |set: &mut std::collections::HashSet<usize>| {
                        *set = set
                            .iter()
                            .filter(|d| !gone.contains(d))
                            .map(|&d| d - gone.iter().filter(|&&g| g < d).count())
                            .collect();
                    };
                    fix(&mut self.def_collapsed);
                    fix(&mut self.def_locked);
                    self.def_pick.clear();
                    self.def_anchor = None;
                    if matches!(self.selected, Some(Sel::Deformer(_))) {
                        self.selected = None;
                    }
                }
            }
        }
    }

    /// The meshes that reach past their own art on the atlas (Mesh Edit
    /// grew them), whose exported texture would show a neighbour's pixels
    /// until the atlas is laid out again.
    pub(crate) fn outgrown_meshes(&self) -> Vec<usize> {
        let Some(s) = &self.session else { return Vec::new() };
        let sizes = s.page_sizes();
        let art = s.art_boxes();
        s.puppet
            .model()
            .art_meshes
            .iter()
            .enumerate()
            .filter(|(i, m)| {
                let Some(b) = art.get(*i).filter(|b| b[0] > -1.0) else { return false };
                let Some(&(pw, ph)) = sizes.get(m.texture as usize) else { return false };
                // half a pixel past the art counts
                let (tu, tv) = (0.5 / pw.max(1) as f32, 0.5 / ph.max(1) as f32);
                m.uvs.as_chunks::<2>().0.iter().any(|uv| {
                    uv[0] < b[0] - tu || uv[0] > b[2] + tu || uv[1] < b[1] - tv || uv[1] > b[3] + tv
                })
            })
            .map(|(i, _)| i)
            .collect()
    }

    /// Lay the atlas out again when a mesh reaches past its own art, so the
    /// pages hold only each mesh's own pixels and no export shows a
    /// neighbour's: one undo step, at the size of the biggest page now.
    pub(crate) fn repack_if_outgrown(&mut self, ctx: &egui::Context) {
        let outgrown = self.outgrown_meshes();
        let Some(s) = &mut self.session else { return };
        let sizes = s.page_sizes();
        let art = s.art_boxes();
        let model = s.puppet.model();
        // layers still on their own textures have no neighbours to show;
        // the atlas waits for the rigger to make it
        if outgrown.is_empty() || s.atlas_pending() {
            return;
        }
        let pieces = crate::atlas::pieces(model, &sizes, &art);
        let page = pieces
            .iter()
            .filter_map(|p| sizes.get(p.page))
            .map(|&(w, h)| w.max(h))
            .max()
            .unwrap_or(4096)
            .clamp(crate::atlas::MIN_PAGE, crate::atlas::MAX_PAGE);
        let names: Vec<String> = outgrown.iter().take(3).map(|&i| s.mesh_name(i)).collect();
        match crate::atlas::auto_layout(&pieces, page, true)
            .and_then(|layout| s.apply_atlas(ctx, &pieces, &layout))
        {
            Ok(()) => log::info!(
                "atlas laid out again: {} mesh(es) outgrew their art ({})",
                outgrown.len(),
                names.join(", ")
            ),
            Err(e) => log::warn!("atlas: could not lay it out again: {e}"),
        }
    }

    /// Open Rename Deformer on a deformer's current name and ID.
    fn open_def_rename(&mut self, i: usize) {
        let Some(s) = &self.session else { return };
        let Some(d) = s.puppet.model().deformers.get(i) else { return };
        let name = s.deformer_names.get(i).cloned().unwrap_or_default();
        self.def_rename = Some((Sel::Deformer(i), name, d.id.clone(), false));
    }

    /// Open Rename Art Mesh on a mesh's current name and ID.
    fn open_mesh_rename(&mut self, i: usize) {
        let Some(s) = &self.session else { return };
        let Some(m) = s.puppet.model().art_meshes.get(i) else { return };
        let name = s.mesh_names.get(i).cloned().unwrap_or_default();
        self.def_rename = Some((Sel::Mesh(i), name, m.id.clone(), false));
    }

    /// Open Rename Part on a part's current name and ID.
    fn open_part_rename(&mut self, i: usize) {
        let Some(s) = &self.session else { return };
        let Some(p) = s.puppet.model().parts.get(i) else { return };
        let name = s.part_names.get(i).cloned().unwrap_or_default();
        self.def_rename = Some((Sel::Part(i), name, p.id.clone(), false));
    }

    /// The rename shortcut: Rename on whatever is selected, a part,
    /// deformer or art mesh, or Edit Parameter on a selected parameter.
    fn rename_selected(&mut self) {
        match self.selected {
            Some(Sel::Part(i)) => self.open_part_rename(i),
            Some(Sel::Deformer(i)) => self.open_def_rename(i),
            Some(Sel::Mesh(i)) => self.open_mesh_rename(i),
            Some(Sel::Parameter(i)) => self.run_param_cmd(ParamCmd::Edit(i)),
            None => {
                if let Some(pi) = self.sel_param {
                    self.run_param_cmd(ParamCmd::Edit(pi));
                }
            }
        }
    }

    /// Rename a part, deformer or art mesh: its name (shown in the palettes,
    /// free text, as in Cubism) and its ID (what exports and the runtime
    /// use: unique and not empty), both editable after it was made.
    fn def_rename_window(&mut self, ctx: &egui::Context) {
        let Some((target, name, id, refused)) = &mut self.def_rename else { return };
        let (target, mut open, mut ok) = (*target, true, false);
        let title = match target {
            Sel::Mesh(_) => crate::i18n::tr("Rename Art Mesh"),
            Sel::Part(_) => crate::i18n::tr("Rename Part"),
            _ => crate::i18n::tr("Rename Deformer"),
        };
        crate::windows::Dialog::new(title).open(&mut open).show(ctx, |ui| {
            crate::windows::form(ui, "def-rename", |ui| {
                ui.label(crate::i18n::tr("Name"));
                let edit = ui.add(egui::TextEdit::singleline(name).desired_width(240.0));
                // the name takes the keyboard when the dialog opens
                if ui.memory(|m| m.focused().is_none()) {
                    edit.request_focus();
                }
                ok |= edit.lost_focus() && ui.input(|k| k.key_pressed(egui::Key::Enter));
                ui.end_row();
                ui.label(crate::i18n::tr("ID"));
                let edit = ui.add(egui::TextEdit::singleline(id).desired_width(240.0));
                ok |= edit.lost_focus() && ui.input(|k| k.key_pressed(egui::Key::Enter));
                ui.end_row();
            });
            if *refused {
                ui.colored_label(
                    theme::pal().rotation_red,
                    crate::i18n::tr("That ID is empty or already used."),
                );
            }
            ui.add_space(6.0);
            ok |= ui.button(crate::i18n::tr("Rename")).clicked();
        });
        if ok {
            let (name, id) = (name.trim().to_string(), id.trim().to_string());
            let Some(s) = &mut self.session else { return };
            let (kind, i, same_id) = match target {
                Sel::Mesh(i) => {
                    ("art mesh", i, s.puppet.model().art_meshes.get(i).is_some_and(|m| m.id == id))
                }
                Sel::Deformer(i) => {
                    ("deformer", i, s.puppet.model().deformers.get(i).is_some_and(|d| d.id == id))
                }
                Sel::Part(i) => {
                    ("part", i, s.puppet.model().parts.get(i).is_some_and(|p| p.id == id))
                }
                _ => {
                    self.def_rename = None;
                    return;
                }
            };
            let renamed = same_id
                || s.edit(|m| match target {
                    Sel::Mesh(i) => ayatsuri_core::edit::rename_art_mesh(m, i, &id),
                    Sel::Part(i) => ayatsuri_core::edit::rename_part(m, i, &id),
                    _ => ayatsuri_core::edit::rename_deformer(m, i, &id),
                });
            if !renamed {
                log::warn!("{kind} {i}: the ID {id:?} is empty or already used");
                if let Some(r) = &mut self.def_rename {
                    r.3 = true;
                }
                return;
            }
            if !same_id {
                log::info!("{kind} {i}: ID changed to {id}");
            }
            let names = match kind {
                "art mesh" => &mut s.mesh_names,
                "part" => &mut s.part_names,
                _ => &mut s.deformer_names,
            };
            if let Some(n) = names.get_mut(i).filter(|n| **n != name) {
                *n = name.clone();
                s.modified = true;
                log::info!("{kind} {i}: renamed to {name:?}");
            }
            self.def_rename = None;
        } else if !open {
            self.def_rename = None;
        }
    }
    /// Open Conversion Divisions on a warp's current columns and rows.
    fn open_def_division(&mut self, i: usize) {
        let Some(s) = &self.session else { return };
        if let Some(DeformerKind::Warp(w)) = s.puppet.model().deformers.get(i).map(|d| &d.kind) {
            self.def_division = Some((i, w.cols, w.rows));
        }
    }

    /// Conversion Divisions: a warp's columns and rows after it was made.
    /// Every keyform's grid is sampled again, so what the warp holds keeps
    /// its shape; a coarser grid loses the bends between the points it
    /// keeps, which the dialog says.
    fn def_division_window(&mut self, ctx: &egui::Context) {
        let Some((i, cols, rows)) = &mut self.def_division else { return };
        let (i, mut open, mut ok) = (*i, true, false);
        let was = self.session.as_ref().and_then(|s| match s.puppet.model().deformers.get(i) {
            Some(d) => match &d.kind {
                DeformerKind::Warp(w) => Some((w.cols, w.rows)),
                DeformerKind::Rotation(_) => None,
            },
            None => None,
        });
        let Some((was_cols, was_rows)) = was else {
            self.def_division = None;
            return;
        };
        crate::windows::Dialog::new(crate::i18n::tr("Conversion Divisions")).open(&mut open).show(
            ctx,
            |ui| {
                crate::windows::form(ui, "def-division", |ui| {
                    ui.label(crate::i18n::tr("Conversion divisions"));
                    ui.horizontal(|ui| {
                        ui.add(egui::DragValue::new(cols).range(1..=64).prefix("W "));
                        ui.add(egui::DragValue::new(rows).range(1..=64).prefix("H "));
                    });
                    ui.end_row();
                });
                if *cols < was_cols || *rows < was_rows {
                    ui.weak(crate::i18n::tr(
                        "Fewer divisions keep the shape only at the points that remain.",
                    ));
                }
                ui.add_space(6.0);
                ok = ui
                    .add_enabled(
                        (*cols, *rows) != (was_cols, was_rows),
                        egui::Button::new(crate::i18n::tr("Apply")),
                    )
                    .clicked();
            },
        );
        if ok {
            let (cols, rows) = (*cols, *rows);
            self.apply_warp_division(i, cols, rows);
            self.def_division = None;
        } else if !open {
            self.def_division = None;
        }
    }

    /// Change warp `i`'s conversion division as one undo step: its grids
    /// are sampled again, and the Bezier handles and picked points, made for
    /// the old grid, are dropped.
    fn apply_warp_division(&mut self, i: usize, cols: u32, rows: u32) {
        let Some(s) = &mut self.session else { return };
        if s.edit(|m| ayatsuri_core::edit::set_warp_division(m, i, cols, rows)) {
            s.bezier_nets.retain(|&(d, _), _| d != i);
            self.bezier_div.remove(&i);
            self.pts = point_edit::PointState::default();
            log::info!("deformer {i}: divisions now {cols}x{rows}");
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
                ui.horizontal(|ui| {
                    ui.add_sized(
                        [74.0, 16.0],
                        egui::Label::new(
                            egui::RichText::new(crate::i18n::tr("ID"))
                                .color(theme::pal().text_faint)
                                .size(11.0),
                        ),
                    );
                    ui.label(egui::RichText::new(&m.id).size(11.5));
                    if ui
                        .small_button(egui_phosphor::regular::COPY)
                        .on_hover_text(crate::i18n::tr(
                            "Copy this ID, to paste into another mesh's Clip by",
                        ))
                        .clicked()
                    {
                        ui.ctx().copy_text(m.id.clone());
                    }
                });
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
                color_rows(
                    ui,
                    s,
                    &targets,
                    self.sel_param,
                    &mut self.color_pick,
                    &mut self.color_picked,
                );
                use ayatsuri_format::model::{Blend, flags};
                let blend = Blend::of(m.blend, m.flags);
                let mut new_blend = blend;
                blend_rows(ui, ("mesh", i), &mut new_blend);
                if !new_blend.legacy_bits().1 {
                    ui.horizontal(|ui| {
                        ui.add_space(78.0);
                        // wraps inside the panel, under the controls
                        ui.add(
                            egui::Label::new(
                                egui::RichText::new(crate::i18n::tr(
                                    "Kept by moc3 5.3; older versions draw the nearest of Normal, Add and Multiply",
                                ))
                                .weak(),
                            )
                            .wrap(),
                        );
                    });
                }
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
                new_flags |= new_blend.legacy_bits().0;
                if double_sided {
                    new_flags |= flags::DOUBLE_SIDED;
                }
                if inverted {
                    new_flags |= flags::INVERTED_MASK;
                }
                // clipping in one row, like Cubism's field: the masks as chips
                // (x takes one off), a field to type or paste IDs into with
                // suggestions, and picking on the canvas
                let mut masks = m.masks.clone();
                let picking = self.clip_pick == Some(i);
                let row = clip_by_row(ui, s, i, &masks, &mut self.clip_ids, picking);
                // with several meshes selected the field edits all of them
                let others: Vec<usize> = if self.multi_sel.contains(&i) {
                    self.multi_sel.iter().copied().filter(|&m| m != i).collect()
                } else {
                    Vec::new()
                };
                if row.toggle_pick {
                    self.clip_pick = if picking { None } else { Some(i) };
                    self.clip_also = others.clone();
                    self.tool = Tool::Arrow;
                }
                if let Some(next) = row.masks {
                    let (added, removed): (Vec<u32>, Vec<u32>) = (
                        next.iter().copied().filter(|x| !masks.contains(x)).collect(),
                        masks.iter().copied().filter(|x| !next.contains(x)).collect(),
                    );
                    masks = next;
                    let changed = s.edit(|model| {
                        let mut changed = ayatsuri_core::edit::set_masks(model, i, &masks);
                        for &o in &others {
                            let Some(mut list) = model.art_meshes.get(o).map(|m| m.masks.clone())
                            else {
                                continue;
                            };
                            list.retain(|x| !removed.contains(x));
                            for &a in &added {
                                if a as usize != o && !list.contains(&a) {
                                    list.push(a);
                                }
                            }
                            changed |= ayatsuri_core::edit::set_masks(model, o, &list);
                        }
                        changed
                    });
                    if changed {
                        log::info!(
                            "{} mesh(es) clip by {} mesh(es)",
                            others.len() + 1,
                            masks.len()
                        );
                    }
                }
                clip_rows(
                    ui,
                    s,
                    i,
                    &mut self.clip_group_name,
                    self.prefs.clip_groups_on(),
                    self.prefs.inherit_clip_on(),
                );

                if new_flags != m.flags || new_blend != blend || (order - base_order).abs() > 0.5 {
                    let changed = s.edit(|model| {
                        let Some(am) = model.art_meshes.get_mut(i) else { return false };
                        am.flags = new_flags;
                        am.blend = new_blend;
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
                let name = s.deformer_names.get(i).cloned().unwrap_or_default();
                let Some(d) = s.puppet.model().deformers.get(i) else { return };
                row(ui, crate::i18n::tr("Name"), if name.is_empty() { "-".into() } else { name });
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
                        row(ui, crate::i18n::tr("Type"), crate::i18n::tr("Warp").into());
                        ui.horizontal(|ui| {
                            ui.add_sized(
                                [74.0, 16.0],
                                egui::Label::new(
                                    egui::RichText::new(crate::i18n::tr("Divisions"))
                                        .color(theme::pal().text_faint)
                                        .size(11.0),
                                ),
                            );
                            if ui
                                .small_button(format!("{} x {}", w.cols, w.rows))
                                .on_hover_text(crate::i18n::tr(
                                    "Change its conversion divisions; its shape is kept",
                                ))
                                .clicked()
                            {
                                self.def_division = Some((i, w.cols, w.rows));
                            }
                        });
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
                color_rows(
                    ui,
                    s,
                    &[ayatsuri_core::edit::Target::Deformer(i)],
                    self.sel_param,
                    &mut self.color_pick,
                    &mut self.color_picked,
                );
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
                             exported. To fade a part by a parameter, turn on Offscreen drawing \
                             and key its Opacity, or put its meshes under a deformer and key \
                             that deformer's Opacity.",
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
                self.offscreen_rows(ui, i);
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
    /// Cubism 5.3's offscreen drawing on a part: the switch, then what the
    /// layer lands with (opacity keyed like any opacity, blend, mask).
    fn offscreen_rows(&mut self, ui: &mut Ui, part: usize) {
        use ayatsuri_core::edit::Target;
        let sel_param = self.sel_param;
        let Some(s) = &mut self.session else { return };
        let Some((current, base_order)) = s
            .puppet
            .model()
            .parts
            .get(part)
            .map(|p| (p.offscreen.clone(), p.draw_orders.first().copied().unwrap_or(500.0)))
        else {
            return;
        };
        ui.separator();
        let mut on = current.is_some();
        ui.horizontal(|ui| {
            ui.add_space(78.0);
            ui.checkbox(&mut on, crate::i18n::tr("Offscreen drawing")).on_hover_text(
                crate::i18n::tr(
                    "Draws the part's contents as one layer first, so its opacity, clipping \
                     and blend mode apply to the group as a whole. Its contents keep together \
                     in the draw order. Exports as moc3 5.3.",
                ),
            );
        });
        if on != current.is_some() && s.edit(|m| ayatsuri_core::edit::set_offscreen(m, part, on)) {
            log::info!("part {part}: offscreen drawing {}", if on { "on" } else { "off" });
            return;
        }
        let Some(o) = current else { return };
        opacity_row(ui, s, &[Target::Part(part)], sel_param);
        // the group sorts as one item at the part's own draw order
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
            ui.add(egui::DragValue::new(&mut order).range(0.0..=1000.0).speed(1))
                .on_hover_text(crate::i18n::tr(
                    "Where the whole group draws among its neighbours; its contents keep their own order inside it",
                ));
        });
        if (order - base_order).abs() > 0.5
            && s.edit(|m| {
                let Some(p) = m.parts.get_mut(part) else { return false };
                let delta = (order - base_order).round();
                for d in &mut p.draw_orders {
                    *d += delta;
                }
                true
            })
        {
            log::info!("part {part}: draw order {order:.0}");
        }
        let mut blend = o.blend;
        blend_rows(ui, ("part", part), &mut blend);
        let mut inverted = o.inverted;
        ui.horizontal(|ui| {
            ui.add_space(78.0);
            ui.checkbox(&mut inverted, crate::i18n::tr("Reverse mask"));
        });
        let mut masks = o.masks.clone();
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
                    egui::RichText::new(format!("{}  ×", truncate(&s.mesh_name(x as usize), 18)))
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
            }
            search_menu(ui, "+", |ui| {
                ui.add(
                    egui::TextEdit::singleline(&mut self.clip_search)
                        .hint_text(crate::i18n::tr("Search meshes"))
                        .desired_width(180.0),
                );
                let needle = self.clip_search.to_lowercase();
                let n = s.puppet.model().art_meshes.len();
                egui::ScrollArea::vertical().max_height(260.0).show(ui, |ui| {
                    let free: Vec<usize> =
                        (0..n).filter(|&mi| !masks.contains(&(mi as u32))).collect();
                    for mi in free {
                        let name = s.mesh_name(mi);
                        if (needle.is_empty() || name.to_lowercase().contains(&needle))
                            && ui.button(name).clicked()
                        {
                            masks.push(mi as u32);
                            ui.close();
                        }
                    }
                });
            })
            .on_hover_text(crate::i18n::tr("Add a clipping mask from the list"));
        });
        if (blend != o.blend || inverted != o.inverted || masks != o.masks)
            && s.edit(|m| {
                let Some(off) = m.parts.get_mut(part).and_then(|p| p.offscreen.as_mut()) else {
                    return false;
                };
                off.blend = blend;
                off.inverted = inverted;
                off.masks = masks;
                true
            })
        {
            log::info!("part {part}: offscreen layer settings changed");
        }
    }

    fn textures_panel(&mut self, ui: &mut Ui) {
        let mut open_atlas = false;
        let Some(s) = &self.session else {
            ui.weak(crate::i18n::tr("No textures."));
            return;
        };
        // only atlas pages meshes draw from: pages holding one paint layer
        // wait for the atlas, and old pages stay only for undo
        let model = s.puppet.model();
        let shown: Vec<usize> = (0..s.textures.len())
            .filter(|&i| !s.layer_pages.get(i).copied().unwrap_or(false))
            .filter(|&i| {
                model.art_meshes.iter().any(|m| m.texture as usize == i)
                    || (!model.art_paths.is_empty() && model.art_path_page == Some(i as u32))
            })
            .collect();
        let pending = s.atlas_pending();
        if pending {
            ui.weak(crate::i18n::tr(
                "The texture atlas has not been made yet: each layer is still on a texture of its own",
            ));
            if ui.button(crate::i18n::tr("Make Texture Atlas…")).clicked() {
                open_atlas = true;
            }
        } else if ui.button(crate::i18n::tr("Edit atlas…")).clicked() {
            open_atlas = true;
        }
        if shown.is_empty() && !pending {
            ui.weak(crate::i18n::tr("No textures."));
        }
        ScrollArea::vertical().id_salt("textures").show(ui, |ui| {
            // numbered as the export will write them
            for (n, &i) in shown.iter().enumerate() {
                let Some(t) = s.textures.get(i) else { continue };
                match t {
                    Some(t) => {
                        ui.weak(format!("{n}: texture_{n:02}"));
                        let size = t.size_vec2();
                        let w = ui.available_width().min(160.0);
                        ui.image((t.id(), vec2(w, w * size.y / size.x.max(1.0))));
                    }
                    None => {
                        ui.weak(crate::i18n::trf("{index}: failed to load", &[("index", &n)]));
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
        // shortcut hints for the canvas menu, read before the session is held
        let menu_sc: Vec<(&'static str, String)> = [
            "mesh-edit",
            "auto-connect",
            "rename",
            "select-parent",
            "copy-ids",
            "toggle-visible",
            "lock-selected",
            "merge-vertices",
        ]
        .iter()
        .map(|a| (*a, self.sc(a)))
        .collect();

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
        if space && resp.dragged() {
            self.space_dragged = true;
        }
        let middle = ui.ctx().input(|i| i.pointer.middle_down());
        if resp.dragged() && (space || middle) {
            self.view_pan += resp.drag_delta();
        }
        // a middle drag only pans: from here on the tools treat it like a
        // Space drag and leave the model alone
        let space = space || middle || resp.dragged_by(egui::PointerButton::Middle);
        if resp.secondary_clicked() {
            self.ctx_click = resp.interact_pointer_pos();
            self.ctx_mesh_list = ui.ctx().input(|i| i.modifiers.command);
            self.ctx_layers = match self.ctx_click {
                Some(p) if self.mesh_edit.is_none() => {
                    let c = s.puppet.model().canvas;
                    let (origin, scale) = view_origin_scale(view, c, self.view_zoom, self.view_pan);
                    let ux = ((p.x - origin.x) / scale - c.origin_x) / c.pixels_per_unit;
                    let uy = ((p.y - origin.y) / scale - c.origin_y) / c.pixels_per_unit;
                    hits_visible(s, &mut self.pick_alpha, ux, uy, &|m| {
                        self.mesh_locked.contains(&m)
                    })
                }
                _ => Vec::new(),
            };
            self.ctx_tree = match self.ctx_layers.first() {
                Some(&mi) if !self.prefs.def_tree_off => deformer_chain(s.puppet.model(), mi),
                _ => Vec::new(),
            };
        }
        // Cubism's tree: the right-clicked mesh's deformers, root first, then
        // the mesh; a click selects one, Shift+click adds it to the pick
        let tree: Vec<(usize, String, bool)> = self
            .ctx_tree
            .iter()
            .map(|&d| {
                let warp = s.puppet.warp_grid(d).is_some();
                (d, s.deformer_name(d).to_string(), warp)
            })
            .collect();
        let tree_mesh = self.ctx_layers.first().copied().filter(|_| !tree.is_empty());
        let mut pick_def: Option<usize> = None;
        let picked_now = def_group(self.selected, &self.def_pick);
        // what the menu's object items work on: in Mesh Edit its points,
        // else the selected deformer, else the mesh under the click
        let mut canvas_cmd: Option<CanvasCmd> = None;
        let menu_ctx = MenuCtx {
            mesh_edit: self.mesh_edit,
            picked_points: self.mesh_picked.len(),
            deformer: match self.selected {
                Some(Sel::Deformer(d)) => Some((d, s.puppet.warp_grid(d).is_some())),
                _ => None,
            },
            mesh: self.ctx_layers.first().copied().or(match self.selected {
                Some(Sel::Mesh(m)) => Some(m),
                _ => None,
            }),
            shown: self
                .ctx_layers
                .first()
                .is_none_or(|&m| s.mesh_shown.get(m).copied().unwrap_or(true)),
            locked: self.ctx_layers.first().is_some_and(|m| self.mesh_locked.contains(m)),
            sc: menu_sc,
        };
        // every layer drawn under the right-click, front first, so a mesh
        // under another one can be picked without the Part tree
        let layers: Vec<(usize, String)> =
            self.ctx_layers.iter().map(|&mi| (mi, s.mesh_name(mi).to_string())).collect();
        let mut pick_layer = None;
        let mut hover_layer = None;
        let mut hover_def: Option<usize> = None;
        let current = self.selected;
        // a row: its picture, a click picks it, a hover outlines it on the
        // canvas so the right one is easy to tell apart
        let mut list = |ui: &mut Ui, s: &mut Session| {
            for (mi, name) in &layers {
                let r = layer_row(ui, s, *mi, name, current == Some(Sel::Mesh(*mi)));
                if r.hovered() {
                    hover_layer = Some(*mi);
                }
                if r.clicked() {
                    pick_layer = Some(*mi);
                    ui.close();
                }
            }
        };
        resp.context_menu(|ui| {
            if self.ctx_mesh_list {
                if layers.is_empty() {
                    ui.weak(crate::i18n::tr("No art mesh here"));
                }
                list(ui, s);
                return;
            }
            for (depth, (d, name, warp)) in tree.iter().enumerate() {
                let r = ui
                    .horizontal(|ui| {
                        ui.add_space(depth as f32 * 12.0);
                        let (glyph, _) = ui.allocate_exact_size(vec2(14.0, 14.0), Sense::hover());
                        if *warp {
                            paint_warp_glyph(ui, glyph.center());
                        } else {
                            paint_rotation_glyph(ui, glyph.center());
                        }
                        ui.add(egui::Button::selectable(picked_now.contains(d), name))
                    })
                    .inner;
                if r.hovered() {
                    hover_def = Some(*d);
                }
                if r.clicked() {
                    pick_def = Some(*d);
                    ui.close();
                }
            }
            if let Some(mi) = tree_mesh {
                ui.horizontal(|ui| {
                    ui.add_space(tree.len() as f32 * 12.0 + 18.0);
                    ui.weak(s.mesh_name(mi));
                });
                ui.separator();
            }
            canvas_menu_items(ui, &menu_ctx, &mut canvas_cmd);
            if !layers.is_empty() {
                ui.menu_button(crate::i18n::tr("Select Layer"), |ui| list(ui, s));
                ui.separator();
            }
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
                if ui.button(crate::i18n::tr("Add Guide Lines in the Center")).clicked() {
                    self.pending_guide = Some(GuideCmd::Center);
                    ui.close();
                }
                if ui.button(crate::i18n::tr("Add a Guide Line at the Selected Vertex")).clicked() {
                    self.pending_guide = Some(GuideCmd::AtVertex);
                    ui.close();
                }
                ui.checkbox(&mut self.show_guides, crate::i18n::tr("Show Guide Lines"));
                ui.checkbox(&mut self.mirror_edit, crate::i18n::tr("Mirror Edit"));
                if ui.button(crate::i18n::tr("Guide Line Settings…")).clicked() {
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
        if let Some(mi) = hover_layer {
            self.palette_hover = Some(Sel::Mesh(mi));
        }
        if let Some(d) = hover_def {
            self.palette_hover = Some(Sel::Deformer(d));
        }
        if let Some(d) = pick_def {
            let shift = ui.ctx().input(|i| i.modifiers.shift);
            canvas_def_pick(
                CanvasPick {
                    model: s.puppet.model(),
                    selected: &mut self.selected,
                    pick: &mut self.def_pick,
                    anchor: &mut self.def_anchor,
                    meshes: &mut self.multi_sel,
                    parts: &mut self.part_pick,
                },
                d,
                shift,
            );
            log::info!("selection: deformer {} from the canvas menu", s.deformer_name(d));
        }
        if canvas_cmd.is_some() {
            self.canvas_cmd = canvas_cmd;
            ui.ctx().request_repaint();
        }
        if let Some(mi) = pick_layer {
            log::info!("selection: {} from the canvas menu", s.mesh_name(mi));
            self.selected = Some(Sel::Mesh(mi));
            self.multi_sel = vec![mi];
        }
        let scroll = ui.ctx().input(|i| if resp.hovered() { i.smooth_scroll_delta.y } else { 0.0 });
        if scroll != 0.0 {
            let pivot = ui.ctx().input(|i| i.pointer.latest_pos()).unwrap_or(view.center());
            (self.view_zoom, self.view_pan) = zoom_about(
                self.view_zoom,
                self.view_pan,
                view.center(),
                pivot,
                (scroll * 0.0015).exp(),
            );
        }

        let c = s.puppet.model().canvas;
        let (cw, ch) = (c.width.max(1.0), c.height.max(1.0));
        let fit = fit_scale(view, c);
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
                self.view_zoom = (want / fit).clamp(ZOOM_MIN, ZOOM_MAX);
                let sc = fit * self.view_zoom;
                self.view_pan = vec2(cw / 2.0 - (x0 + x1) / 2.0, ch / 2.0 - (y0 + y1) / 2.0) * sc;
            }
        }
        let (origin, scale) = view_origin_scale(view, c, self.view_zoom, self.view_pan);
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
        self.pts.smooth = self.prefs.smooth_drag_seconds();
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
        vp.rect_filled(page, 0.0, self.prefs.canvas_page_color().unwrap_or(theme::pal().canvas));

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
            // nothing else wakes an idle editor: come back when the full
            // pages are due to go
            if let Some(wait) = s.residency.idle_wake() {
                ui.ctx().request_repaint_after(wait);
            }
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
                None,
                pos2(origin.x, origin.y),
                scale,
                ui.ctx().pixels_per_point(),
                solo_view(s, &self.prefs).as_ref(),
                // the atlas view leaves the art unclipped, as exported
                &if self.atlas_view { Vec::new() } else { s.art_boxes() },
                // Mesh Edit draws the mesh's art frozen above the scrim; the
                // live one under it would show every edit re-cutting it
                self.mesh_edit,
            );
            vp.add(cb);
        } else {
            // Solo: the others first behind the inactive color, then the
            // soloed meshes in front as they are
            let solo = solo_view(s, &self.prefs);
            let draws: Vec<_> = s.puppet.drawables().collect();
            let soloed = |mi: usize| solo.as_ref().is_some_and(|v| v.meshes.contains(&mi));
            let order = draws
                .iter()
                .filter(|d| !soloed(d.mesh))
                .map(|d| (d, false))
                .chain(draws.iter().filter(|d| soloed(d.mesh)).map(|d| (d, true)));
            for (d, front) in order {
                if d.opacity <= 0.0 || d.indices.is_empty() || self.mesh_edit == Some(d.mesh) {
                    continue;
                }
                let texture = s.textures.get(d.texture as usize).and_then(|t| t.as_ref());
                let k = match (&solo, front) {
                    (Some(v), false) => v.tint,
                    _ => [1.0; 4],
                };
                let a = d.opacity * k[3];
                let tint = Color32::from_rgba_premultiplied(
                    (k[0] * a * 255.0) as u8,
                    (k[1] * a * 255.0) as u8,
                    (k[2] * a * 255.0) as u8,
                    (a * 255.0) as u8,
                );
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

        if solo_badge(ui, view, s, &mut self.prefs) {
            self.prefs.save();
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

        // Highlight Deformer's Child Elements: every mesh under the selected
        // deformer, its faces washed and its edges drawn
        if self.highlight_children
            && let Some(Sel::Deformer(d)) = self.selected
        {
            let fill = theme::pal().accent.gamma_multiply(0.18);
            let stroke = Stroke::new(1.2, theme::pal().accent);
            for mi in meshes_under_deformer(s.puppet.model(), d) {
                let Some(g) = s.puppet.mesh_geometry(mi) else { continue };
                let (pos, _) = g.positions.as_chunks::<2>();
                let mut mesh = egui::Mesh::default();
                for p in pos {
                    mesh.colored_vertex(to_screen(p[0], p[1]), fill);
                }
                mesh.indices.extend(g.indices.iter().map(|&i| i as u32));
                add_mesh_shape(&vp, mesh);
                for &[a, b, cx] in g.indices.as_chunks::<3>().0 {
                    for (x, y) in [(a, b), (b, cx), (cx, a)] {
                        if let (Some(p0), Some(p1)) = (pos.get(x as usize), pos.get(y as usize)) {
                            vp.line_segment(
                                [to_screen(p0[0], p0[1]), to_screen(p1[0], p1[1])],
                                stroke,
                            );
                        }
                    }
                }
            }
        }

        // Highlight Overlapping Polygons: in meshes with an advanced blend,
        // the triangles that cover others of the same mesh, as drawn now
        if self.highlight_overlap {
            let [r, g, b, a] = self.prefs.overlap_color.unwrap_or(OVERLAP_COLOR);
            let fill = Color32::from_rgba_unmultiplied(r, g, b, a);
            let stroke = Stroke::new(1.2, Color32::from_rgb(r, g, b));
            for (mi, am) in s.puppet.model().art_meshes.iter().enumerate() {
                if !ayatsuri_core::overlap::can_flicker(am) {
                    continue;
                }
                let Some(g) = s.puppet.mesh_geometry(mi) else { continue };
                let hits = ayatsuri_core::overlap::overlapping(g.positions, g.indices);
                if hits.is_empty() {
                    continue;
                }
                let (pos, _) = g.positions.as_chunks::<2>();
                let (tris, _) = g.indices.as_chunks::<3>();
                let at = |i: u16| pos.get(i as usize).map(|p| to_screen(p[0], p[1]));
                let mut mesh = egui::Mesh::default();
                for t in hits.iter().filter_map(|&ti| tris.get(ti)) {
                    let (Some(p0), Some(p1), Some(p2)) = (at(t[0]), at(t[1]), at(t[2])) else {
                        continue;
                    };
                    let base = mesh.vertices.len() as u32;
                    for p in [p0, p1, p2] {
                        mesh.colored_vertex(p, fill);
                    }
                    mesh.indices.extend([base, base + 1, base + 2]);
                    vp.add(egui::Shape::closed_line(vec![p0, p1, p2], stroke));
                }
                add_mesh_shape(&vp, mesh);
            }
        }

        // the Select tool: hover previews a mesh, click selects it, empty
        // click deselects, double-click jumps into mesh editing
        // the texture atlas view is on: say so, so it's never mistaken for
        // the model looking wrong
        if self.atlas_view {
            let text = crate::i18n::tr("Texture Atlas View  ·  T to switch back");
            let galley = painter.layout_no_wrap(
                text.to_string(),
                FontId::proportional(11.5),
                theme::pal().text,
            );
            let at = view.left_top() + vec2(10.0, 10.0);
            painter.rect_filled(
                Rect::from_min_size(at - vec2(6.0, 3.0), galley.size() + vec2(12.0, 6.0)),
                4.0,
                theme::pal().chrome.gamma_multiply(0.92),
            );
            painter.galley(at, galley, theme::pal().text);
        }
        // the Inspector's eyedropper: a click on the art takes its colour
        // (and selects nothing); Esc lets go
        let eyedropping = self.color_pick.is_some();
        if let Some(kind) = self.color_pick {
            let ptr = ui.ctx().input(|i| i.pointer.latest_pos()).filter(|p| view.contains(*p));
            let sample = ptr.and_then(|p| {
                let c = s.puppet.model().canvas;
                let (origin, scale) = view_origin_scale(view, c, self.view_zoom, self.view_pan);
                let ux = ((p.x - origin.x) / scale - c.origin_x) / c.pixels_per_unit;
                let uy = ((p.y - origin.y) / scale - c.origin_y) / c.pixels_per_unit;
                color_at(&s.puppet, &s.texture_pngs, &mut self.pick_pages, ux, uy)
            });
            if let (Some(p), Some([r, g, b, _])) = (ptr, sample) {
                painter.circle(
                    p + vec2(18.0, -18.0),
                    9.0,
                    Color32::from_rgb(r, g, b),
                    Stroke::new(1.5, Color32::WHITE),
                );
            }
            ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
            let cancel = ui.ctx().input(|i| i.key_pressed(egui::Key::Escape));
            if resp.clicked() || cancel {
                if let (false, Some([r, g, b, _])) = (cancel, sample) {
                    self.color_picked = Some((kind, [r, g, b]));
                }
                self.color_pick = None;
                self.pick_pages.clear();
            }
        }
        if self.tool == Tool::Arrow && !space && self.mesh_edit.is_none() && !eyedropping {
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
                    hit_visible(s, &mut self.pick_alpha, ux, uy, &|m| self.mesh_locked.contains(&m))
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
            // a click on the selected object's points or box handles belongs
            // to editing, not to picking the mesh underneath; a click on the
            // box body picks, so another mesh inside it is one click away
            let ctrl_now = ui.ctx().input(|i| i.modifiers.command);
            if resp.clicked()
                && (!self.pts.on_handle || ctrl_now)
                && let Some(ptr) = resp.interact_pointer_pos()
            {
                let (ux, uy) = unit_of(ptr);
                let (alt, add) =
                    ui.ctx().input(|i| (i.modifiers.alt, i.modifiers.shift || i.modifiers.command));
                let hits = hits_visible(s, &mut self.pick_alpha, ux, uy, &|m| {
                    self.mesh_locked.contains(&m)
                });
                let hit = if add || self.clip_with.is_some() || self.clip_pick.is_some() {
                    hits.first().copied()
                } else {
                    let cur = match self.selected {
                        Some(Sel::Mesh(m)) => Some(m),
                        _ => None,
                    };
                    cycle_pick(&hits, cur, ptr, self.last_pick)
                };
                self.last_pick = (!add).then_some(ptr);
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
                        // the whole pick takes the mask, in one undo step;
                        // the mask itself is never clipped by itself
                        let targets: Vec<usize> = std::iter::once(target)
                            .chain(self.clip_also.iter().copied())
                            .filter(|&t| t != mi)
                            .collect();
                        let n = targets.len();
                        if s.edit(|m| add_mask_to(m, &targets, mi)) {
                            log::info!("{n} mesh(es) now clip by {}", s.mesh_name(mi));
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
                } else if add && alt {
                    // Shift+Alt+click adds the mesh's deformer to the pick,
                    // or takes it out when it is already in
                    let d = hit.and_then(|mi| {
                        s.puppet.model().art_meshes.get(mi).and_then(|m| m.parent_deformer)
                    });
                    if let Some(d) = d {
                        canvas_def_pick(
                            CanvasPick {
                                model: s.puppet.model(),
                                selected: &mut self.selected,
                                pick: &mut self.def_pick,
                                anchor: &mut self.def_anchor,
                                meshes: &mut self.multi_sel,
                                parts: &mut self.part_pick,
                            },
                            d as usize,
                            true,
                        );
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
                if let Some(mi) =
                    hit_visible(s, &mut self.pick_alpha, ux, uy, &|m| self.mesh_locked.contains(&m))
                {
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

        // the Edit Mesh tool: clicking a mesh jumps straight into edit mode,
        // on the selected mesh when it is drawn there, else on the front one
        // drawn there; a click where nothing is drawn does nothing
        if self.tool == Tool::MeshManual
            && !space
            && self.mesh_edit.is_none()
            && resp.clicked()
            && let Some(ptr) = resp.interact_pointer_pos()
        {
            let ux = ((ptr.x - origin.x) / scale - c.origin_x) / c.pixels_per_unit;
            let uy = ((ptr.y - origin.y) / scale - c.origin_y) / c.pixels_per_unit;
            let hits =
                hits_visible(s, &mut self.pick_alpha, ux, uy, &|m| self.mesh_locked.contains(&m));
            let cur = match self.selected {
                Some(Sel::Mesh(m)) if hits.contains(&m) => Some(m),
                _ => None,
            };
            if let Some(mi) = cur.or(hits.first().copied()) {
                self.selected = Some(Sel::Mesh(mi));
                self.mesh_edit = Some(mi);
                self.mesh_drag = None;
                self.mesh_snapshot = Some(s.puppet.model().clone());
            } else {
                set_notice(&mut self.notice, crate::i18n::tr("Click an art mesh to edit it."));
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
                && let Some(mi) =
                    hit_visible(s, &mut self.pick_alpha, ux, uy, &|m| self.mesh_locked.contains(&m))
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
        let mut group_live = false;
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
                    if self.pts.settling {
                        ui.ctx().request_repaint();
                    }
                    if let (Some(ax), Some(m)) = (mirror_x, &mut out.commit) {
                        point_edit::mirror_moves(m, &pts, ax, mirror_tol);
                    }
                    warp_hot |= self.pts.hot;
                    grab_hot = out.hover_grab;
                    if ui.ctx().input(|i| i.key_pressed(egui::Key::Escape)) {
                        self.pts.picked.clear();
                    }
                    // the art follows the box while it is dragged
                    group_live = true;
                    let from = self
                        .pts
                        .drag
                        .as_ref()
                        .filter(|d| d.grab != point_edit::Grab::Marquee && out.commit.is_none())
                        .map(|d| d.from.clone());
                    let objs: Vec<(GroupObj, usize)> = owner
                        .iter()
                        .map(|&(m, v)| (GroupObj::Live(LiveTarget::Mesh(m)), v))
                        .collect();
                    let k_unit = 1.0 / (scale * c.pixels_per_unit);
                    if self.live_group.update(&mut s.puppet, from.as_deref(), &pts, &objs, k_unit) {
                        s.dirty = true;
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
                            let offs = point_edit::follow_path(bind, &gp.verts, &moved, gp.point);
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
                        if self.pts.settling {
                            ui.ctx().request_repaint();
                        }
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
            Some(Sel::Deformer(_)) if def_group(self.selected, &self.def_pick).len() > 1 => {
                // several deformers picked: one box over every warp's grid
                // and rotation deformer's origin moves, scales and rotates
                // them together, in the keyform the parameters sit on
                // a picked deformer inside another picked one rides along
                // with it, so only the outermost ones take the box (else
                // the inner ones would move twice); any level can be picked
                let picked = def_group(self.selected, &self.def_pick);
                let group: Vec<usize> = outermost(s.puppet.model(), &picked)
                    .into_iter()
                    .filter(|d| !self.def_locked.contains(d))
                    .collect();
                // (deformer, grid point; usize::MAX for a rotation origin)
                let mut owner: Vec<(usize, usize)> = Vec::new();
                let mut pts: Vec<Pos2> = Vec::new();
                let mut grids: Vec<(usize, u32, u32)> = Vec::new();
                for &d in &group {
                    if let Some((g, rows, cols)) = s.puppet.warp_grid(d) {
                        let base = pts.len();
                        for (gi, q) in g.as_chunks::<2>().0.iter().enumerate() {
                            pts.push(to_screen(q[0], q[1]));
                            owner.push((d, gi));
                        }
                        grids.push((base, rows, cols));
                    } else if let Some((x, y)) = s.puppet.rotation_local_origin(d) {
                        let parent =
                            s.puppet.model().deformers.get(d).and_then(|x| x.parent_deformer);
                        let (wx, wy) = match parent {
                            Some(p) => s.puppet.deformer_point(p as usize, x, y),
                            None => (x, y),
                        };
                        pts.push(to_screen(wx, wy));
                        owner.push((d, usize::MAX));
                    }
                }
                let editable =
                    matches!(self.tool, Tool::Arrow | Tool::Lasso) && !space && !pts.is_empty();
                let key =
                    group.iter().fold(0x51ed_usize, |h, d| h.wrapping_mul(31).wrapping_add(*d));
                let mut grab_hot = None;
                if editable {
                    self.pts.bind((4, key));
                    self.pts.weights = Vec::new();
                    let inp = point_edit::PtInput::read(ui, &resp);
                    let opts = point_edit::PtOpts {
                        whole_box: true,
                        marquee: true,
                        lasso: self.tool == Tool::Lasso,
                        reach: 7.0,
                        pad: 0.0,
                    };
                    let mut out = point_edit::point_tool(&mut self.pts, &inp, &mut pts, &opts);
                    if self.pts.settling {
                        ui.ctx().request_repaint();
                    }
                    if let (Some(ax), Some(m)) = (mirror_x, &mut out.commit) {
                        point_edit::mirror_moves(m, &pts, ax, mirror_tol);
                    }
                    warp_hot |= self.pts.hot;
                    grab_hot = out.hover_grab;
                    if ui.ctx().input(|i| i.key_pressed(egui::Key::Escape)) {
                        self.pts.picked.clear();
                    }
                    // the art follows the box while it is dragged
                    // with Ctrl the deformers move without what they hold,
                    // as in Cubism, so the art stays
                    group_live = true;
                    let keep_children = ui.input(|i| i.modifiers.command);
                    let from = self
                        .pts
                        .drag
                        .as_ref()
                        .filter(|d| {
                            d.grab != point_edit::Grab::Marquee
                                && out.commit.is_none()
                                && !keep_children
                        })
                        .map(|d| d.from.clone());
                    let objs: Vec<(GroupObj, usize)> = owner
                        .iter()
                        .map(|&(d, gi)| match gi {
                            usize::MAX => (GroupObj::Origin(d), 0),
                            _ => (GroupObj::Live(LiveTarget::Warp(d)), gi),
                        })
                        .collect();
                    let k_unit = 1.0 / (scale * c.pixels_per_unit);
                    if self.live_group.update(&mut s.puppet, from.as_deref(), &pts, &objs, k_unit) {
                        s.dirty = true;
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
                        let mut pivots: Vec<(usize, PivotMove)> = Vec::new();
                        for &(i, dv) in &moved {
                            let Some(&(d, gi)) = owner.get(i) else { continue };
                            // a rotation's centre moves as its pivot: what it
                            // holds stays
                            if gi == usize::MAX && keep_children {
                                if let Some(pm) =
                                    pivot_move(&s.puppet, d, dv.x * k_unit, dv.y * k_unit)
                                {
                                    pivots.push((d, pm));
                                }
                                continue;
                            }
                            let (px, py) = if gi == usize::MAX {
                                s.puppet.rotation_local_origin(d).unwrap_or((0.0, 0.0))
                            } else {
                                let local = locals.entry(d).or_insert_with(|| {
                                    s.puppet.warp_local_grid(d).unwrap_or_default()
                                });
                                (
                                    local.get(gi * 2).copied().unwrap_or(0.0),
                                    local.get(gi * 2 + 1).copied().unwrap_or(0.0),
                                )
                            };
                            let parent =
                                s.puppet.model().deformers.get(d).and_then(|x| x.parent_deformer);
                            let (dx, dy) = parent_delta(
                                &s.puppet,
                                parent,
                                px,
                                py,
                                dv.x * k_unit,
                                dv.y * k_unit,
                            );
                            per.entry(d).or_default().push((gi, dx, dy));
                        }
                        let mut plan = Vec::new();
                        let mut between = 0usize;
                        for (d, moves) in per {
                            let target = ayatsuri_core::edit::Target::Deformer(d);
                            match ayatsuri_core::keyform::keyform_at(
                                s.puppet.model(),
                                target,
                                &values,
                            ) {
                                Some(k) => plan.push((d, k, moves)),
                                None => between += 1,
                            }
                        }
                        let count = plan.len() + pivots.len();
                        if count > 0
                            && s.edit(|m| {
                                let mut any = false;
                                for (d, (deltas, child)) in &pivots {
                                    any |= ayatsuri_core::edit::move_rotation_pivot(
                                        m, *d, deltas, *child,
                                    );
                                }
                                for (d, k, moves) in &plan {
                                    match moves.first() {
                                        Some(&(usize::MAX, dx, dy)) => {
                                            any |= ayatsuri_core::edit::move_rotation_origin_in(
                                                m, *d, *k, dx, dy,
                                            );
                                        }
                                        _ if keep_children => {
                                            any |=
                                                ayatsuri_core::edit::move_warp_points_keep_children(
                                                    m, *d, *k, moves,
                                                );
                                        }
                                        _ => {
                                            any |= ayatsuri_core::edit::move_warp_points(
                                                m,
                                                *d,
                                                Some(*k),
                                                moves,
                                            );
                                        }
                                    }
                                }
                                any
                            })
                        {
                            log::debug!("{count} deformers moved together");
                        }
                        if between > 0 {
                            log::warn!(
                                "{between} deformer(s) sit between keys and did not move: put the parameters on a key"
                            );
                        }
                    }
                }
                let stroke = Stroke::new(1.0, theme::pal().accent.gamma_multiply(0.8));
                for &(base, rows, cols) in &grids {
                    let stride = cols as usize + 1;
                    let at = |r: usize, c2: usize| pts.get(base + r * stride + c2).copied();
                    for r in 0..=rows as usize {
                        for c2 in 0..=cols as usize {
                            let Some(p) = at(r, c2) else { continue };
                            if let Some(q) = at(r, c2 + 1).filter(|_| c2 < cols as usize) {
                                vp.line_segment([p, q], stroke);
                            }
                            if let Some(q) = at(r + 1, c2).filter(|_| r < rows as usize) {
                                vp.line_segment([p, q], stroke);
                            }
                        }
                    }
                }
                for (p, &(_, gi)) in pts.iter().zip(&owner) {
                    if gi == usize::MAX {
                        vp.circle_stroke(*p, 5.0, Stroke::new(1.5, theme::pal().accent));
                    }
                    vp.circle_filled(*p, 2.0, theme::pal().accent);
                }
                if editable {
                    point_edit::draw_overlay(&self.pts, &vp, &pts, true, 0.0, grab_hot);
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
                            let sd =
                                if inp.shift { crate::point_edit::axis_locked(sd) } else { sd };
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
                        if self.pts.settling {
                            ui.ctx().request_repaint();
                        }
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
                    // Ctrl moves the warp without what it holds, as in
                    // Cubism: only the grid follows the drag, the art stays
                    let keep_children = ui.input(|i| i.modifiers.command);
                    let live_target = (!keep_children
                        && (self.bez_drag.is_some() || point_drag.is_some()))
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
                                        if keep_children {
                                            ayatsuri_core::edit::move_warp_points_keep_children(
                                                m, di, k, &moves,
                                            )
                                        } else {
                                            ayatsuri_core::edit::move_warp_points(
                                                m,
                                                di,
                                                Some(k),
                                                &moves,
                                            )
                                        }
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
                        let shift = ui.input(|i| i.modifiers.shift);
                        if d.turning && !d.handle_only {
                            angle += d.turn(shift);
                        } else if !d.turning
                            && !d.scaling
                            && let Some(ptr) = ptr
                            && resp.dragged()
                        {
                            o = ptr;
                        }
                    }
                    let a = angle.to_radians();
                    let handle = self.rot_handle.get(&di).copied().unwrap_or(26.0);
                    let dir = vec2(a.sin(), -a.cos()) * handle;
                    let tip = o + dir;

                    // a drag only starts once the pointer has gone past the
                    // click distance (wider in pen mode), so what it grabs
                    // is where the press began, not where the pointer is now
                    let press = ui.ctx().input(|i| i.pointer.press_origin());
                    let at = if resp.drag_started() { press.or(ptr) } else { ptr };
                    let near_origin =
                        at.is_some_and(|p| p.distance(o) < 9.0) && self.rot_drag.is_none();
                    let near_tip = at.is_some_and(|p| p.distance(tip) < 9.0)
                        && !near_origin
                        && self.rot_drag.is_none();
                    // the round scale handle: the ring around the centre
                    let near_ring = at.is_some_and(|p| (p.distance(o) - ROT_RING).abs() < 5.0)
                        && !near_origin
                        && !near_tip
                        && self.rot_drag.is_none();
                    if !space {
                        if resp.drag_started()
                            && !self.def_locked.contains(&di)
                            && (near_origin || near_tip || near_ring)
                            && let Some(ptr) = ptr
                            && let Some(local) = s.puppet.rotation_local_origin(di)
                        {
                            let start = press.unwrap_or(ptr);
                            // the way already come before the drag started,
                            // less this frame's step, which the drag adds
                            let ahead = (ptr - start) - resp.drag_delta();
                            let (wx, wy) = (
                                ahead.x / (scale * c.pixels_per_unit),
                                ahead.y / (scale * c.pixels_per_unit),
                            );
                            let parent = s
                                .puppet
                                .model()
                                .deformers
                                .get(di)
                                .and_then(|de| de.parent_deformer);
                            let (dx, dy) = if near_tip {
                                (0.0, 0.0)
                            } else {
                                parent_delta(&s.puppet, parent, local.0, local.1, wx, wy)
                            };
                            self.rot_drag = Some(RotDrag {
                                deformer: di,
                                turning: near_tip,
                                local_origin: local,
                                dx,
                                dy,
                                wx: if near_tip { 0.0 } else { wx },
                                wy: if near_tip { 0.0 } else { wy },
                                degrees: 0.0,
                                last_ptr: start,
                                keep: ui.input(|i| i.modifiers.command),
                                handle_only: near_tip && ui.input(|i| i.modifiers.alt),
                                origin_base: None,
                                scaling: near_ring,
                                scale_from: (start - o).length().max(1.0),
                                factor: 1.0,
                                scale_base: None,
                            });
                        }
                        // live turn: what the rotation holds swings with the
                        // handle; the angles go back before the real edit
                        let turning_now = self
                            .rot_drag
                            .as_ref()
                            .filter(|d| d.turning && d.deformer == di && !d.keep && !d.handle_only);
                        let shift_now = ui.input(|i| i.modifiers.shift);
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
                                        *x = b + d.turn(shift_now);
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
                            if d.scaling {
                                d.factor = ((ptr - o).length() / d.scale_from).clamp(0.05, 20.0);
                                // without Ctrl the art grows with it, live in
                                // the keyform the parameters sit on
                                if !d.keep {
                                    if d.scale_base.is_none() {
                                        let values: Vec<f32> =
                                            (0..s.puppet.model().parameters.len())
                                                .map(|i| s.puppet.value(i))
                                                .collect();
                                        let target = ayatsuri_core::edit::Target::Deformer(di);
                                        if let Some(k) = ayatsuri_core::keyform::keyform_at(
                                            s.puppet.model(),
                                            target,
                                            &values,
                                        ) && let Some(base) = s
                                            .puppet
                                            .rotation_scales_mut(di)
                                            .and_then(|v| v.get(k).copied())
                                        {
                                            d.scale_base = Some((k, base));
                                        }
                                    }
                                    if let Some((k, base)) = d.scale_base
                                        && let Some(v) = s
                                            .puppet
                                            .rotation_scales_mut(di)
                                            .and_then(|v| v.get_mut(k))
                                    {
                                        *v = base * d.factor;
                                        s.dirty = true;
                                    }
                                }
                            } else if d.turning && d.handle_only {
                                self.rot_handle.insert(di, (ptr - o).length().clamp(12.0, 600.0));
                            } else if d.turning {
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
                                // without Ctrl the art comes along: the origin
                                // of the keyform the parameters sit on moves
                                // live, put back before the real edit
                                if !d.keep {
                                    if d.origin_base.is_none() {
                                        let values: Vec<f32> =
                                            (0..s.puppet.model().parameters.len())
                                                .map(|i| s.puppet.value(i))
                                                .collect();
                                        let target = ayatsuri_core::edit::Target::Deformer(di);
                                        if let Some(k) = ayatsuri_core::keyform::keyform_at(
                                            s.puppet.model(),
                                            target,
                                            &values,
                                        ) && let Some((x, y)) =
                                            s.puppet.rotation_origin_mut(di, k)
                                        {
                                            d.origin_base = Some((k, *x, *y));
                                        }
                                    }
                                    if let Some((k, bx, by)) = d.origin_base
                                        && let Some((x, y)) = s.puppet.rotation_origin_mut(di, k)
                                    {
                                        (*x, *y) = (bx + d.dx, by + d.dy);
                                        s.dirty = true;
                                    }
                                }
                            }
                        }
                        if resp.drag_stopped()
                            && let Some(d) = self.rot_drag.take()
                            && d.deformer == di
                        {
                            // the live angles and origin go back first; the
                            // edit below changes the real model once
                            if let Some((rd, base)) = self.rot_live.take()
                                && let Some(a) = s.puppet.rotation_angles_mut(rd)
                                && a.len() == base.len()
                            {
                                a.copy_from_slice(&base);
                            }
                            if let Some((k, bx, by)) = d.origin_base
                                && let Some((x, y)) = s.puppet.rotation_origin_mut(di, k)
                            {
                                (*x, *y) = (bx, by);
                                s.dirty = true;
                            }
                            if let Some((k, base)) = d.scale_base
                                && let Some(v) =
                                    s.puppet.rotation_scales_mut(di).and_then(|v| v.get_mut(k))
                            {
                                *v = base;
                                s.dirty = true;
                            }
                            let deg = d.turn(ui.input(|i| i.modifiers.shift));
                            if d.scaling {
                                let (dfm, f) = (d.deformer, d.factor);
                                if (f - 1.0).abs() > 1e-4 {
                                    let done = match (d.keep, d.scale_base) {
                                        (true, _) => s.edit(|m| {
                                            ayatsuri_core::edit::scale_rotation_keep_children(
                                                m, dfm, f,
                                            )
                                        }),
                                        (false, Some((k, _))) => s.edit(|m| {
                                            ayatsuri_core::edit::scale_rotation_in(m, dfm, k, f)
                                        }),
                                        (false, None) => {
                                            log::warn!(
                                                "the parameters are between keys: move them onto a key to scale this rotation deformer"
                                            );
                                            false
                                        }
                                    };
                                    if done {
                                        log::debug!(
                                            "rotation {dfm} scaled x{f:.3}{}",
                                            if d.keep { " alone" } else { "" }
                                        );
                                    }
                                }
                            } else if d.turning && d.handle_only {
                                // only the handle's length, already shown
                            } else if d.turning && d.keep && deg.abs() > 1e-3 {
                                let dfm = d.deformer;
                                if s.edit(|m| {
                                    ayatsuri_core::edit::rotate_rotation_keep_children(m, dfm, deg)
                                }) {
                                    log::debug!("rotation {dfm} turned {deg:.2} deg alone");
                                }
                            } else if !d.turning
                                && !d.keep
                                && let Some((k, _, _)) = d.origin_base
                                && (d.dx != 0.0 || d.dy != 0.0)
                            {
                                let (dfm, dx, dy) = (d.deformer, d.dx, d.dy);
                                if s.edit(|m| {
                                    ayatsuri_core::edit::move_rotation_origin_in(m, dfm, k, dx, dy)
                                }) {
                                    log::debug!(
                                        "rotation {dfm} moved with its art in keyform {k} ({dx:.3}, {dy:.3})"
                                    );
                                }
                            } else if d.turning && deg.abs() > 1e-3 {
                                let dfm = d.deformer;
                                // with a blend shape parameter on a key, only
                                // that shape keyform turns
                                let values: Vec<f32> = (0..s.puppet.model().parameters.len())
                                    .map(|i| s.puppet.value(i))
                                    .collect();
                                let target = ayatsuri_core::edit::Target::Deformer(dfm);
                                let shaped = ayatsuri_core::keyform::editing_blend_shape(
                                    s.puppet.model(),
                                    target,
                                    &values,
                                )
                                .and_then(|_| {
                                    ayatsuri_core::keyform::keyform_at(
                                        s.puppet.model(),
                                        target,
                                        &values,
                                    )
                                });
                                if s.edit(|m| {
                                    ayatsuri_core::edit::rotate_rotation_at(m, dfm, shaped, deg)
                                }) {
                                    log::debug!("rotation {dfm} turned {deg:.2} deg");
                                }
                            } else if !d.turning && !d.keep && (d.dx != 0.0 || d.dy != 0.0) {
                                log::warn!(
                                    "the parameters are between keys: move them onto a key to move this rotation deformer"
                                );
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
                        } else if near_ring {
                            ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeNwSe);
                        }
                    }

                    let stroke = Stroke::new(1.4, self.disp.rotation_color);
                    let scaling = self.rot_drag.as_ref().filter(|d| d.scaling && d.deformer == di);
                    let ring = ROT_RING * scaling.map_or(1.0, |d| d.factor);
                    if near_ring || scaling.is_some() {
                        vp.circle_stroke(o, ring, Stroke::new(2.2, theme::pal().accent));
                    } else {
                        vp.circle_stroke(o, ring, stroke);
                    }
                    vp.line_segment([o, tip], stroke);
                    let turning = self.rot_drag.as_ref().is_some_and(|d| d.turning);
                    if near_tip || turning {
                        vp.circle(tip, 4.5, theme::pal().accent, stroke);
                    } else {
                        vp.circle_filled(tip, 3.0, self.disp.rotation_color);
                    }
                    if near_origin
                        || self.rot_drag.as_ref().is_some_and(|d| !d.turning && !d.scaling)
                    {
                        vp.circle(o, 4.5, theme::pal().accent, stroke);
                    } else {
                        vp.circle_filled(o, 2.5, self.disp.rotation_color);
                    }
                }
            }
            _ => {}
        }

        // a group preview whose box is gone (selection changed) puts back
        if !group_live && self.live_group.restore(&mut s.puppet) {
            s.dirty = true;
        }
        self.pts.hot = warp_hot;
        if !warp_hot {
            self.pts.on_handle = false;
        }

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
            // a pen's pressure sizes and weights the stroke as set
            let (pen_size, pen_strength) = self.pen.brush(&self.prefs);
            let r = self.tool_opts.brush_size.clamp(4.0, 400.0) * pen_size;
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
                self.dbrush = Some(DeformStroke { parts, last: p, lock: None });
            }
            if !sizing
                && resp.dragged()
                && let (Some(st), Some(mut p)) = (&mut self.dbrush, ptr)
            {
                // Shift holds the stroke to X or Y, whichever it moves along
                // first, from where Shift went down
                if ui.input(|i| i.modifiers.shift) {
                    let (anchor, axis) = st.lock.get_or_insert((st.last, None));
                    let d = p - *anchor;
                    if axis.is_none() && d.length() > 4.0 {
                        *axis = Some(d.x.abs() >= d.y.abs());
                    }
                    match *axis {
                        Some(true) => p.y = anchor.y,
                        Some(false) => p.x = anchor.x,
                        None => p = st.last,
                    }
                } else {
                    st.lock = None;
                }
                let delta = p - st.last;
                let from = st.last;
                st.last = p;
                if delta.length_sq() > 0.0 {
                    let (hard, strength) =
                        (self.dbrush_hardness, self.dbrush_strength * pen_strength);
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
                brush_ring(ui.ctx(), view, p, r, theme::pal().accent);
                if self.dbrush_hardness > 0.02 {
                    brush_ring(
                        ui.ctx(),
                        view,
                        p,
                        r * self.dbrush_hardness,
                        theme::pal().accent.gamma_multiply(0.5),
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
            let (pen_size, pen_strength) = self.pen.brush(&self.prefs);
            let r = self.tool_opts.brush_size.clamp(2.0, 400.0) * pen_size;
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
                                *wt = (*wt + sign * self.brush_weight * pen_strength * f)
                                    .clamp(0.0, 1.0);
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
                brush_ring(ui.ctx(), view, p, r, col.gamma_multiply(2.0));
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
                    if inp.drag_started {
                        self.path_press = if near.is_none() { probe } else { None };
                    }
                    let tap = if inp.clicked {
                        inp.press.or(inp.ptr)
                    } else if inp.drag_stopped
                        && let Some(at) = self.path_press.take()
                    {
                        inp.ptr.filter(|p| p.distance(at) <= 10.0).map(|_| at)
                    } else {
                        None
                    };
                    if let Some(at) = tap {
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
                (Some(_), _) => {
                    // level 1 shows no paths, as in Cubism, so say so rather
                    // than ignore the clicks
                    if resp.hovered()
                        && let Some(p) = inp.ptr
                    {
                        vp.text(
                            p + vec2(14.0, 14.0),
                            Align2::LEFT_TOP,
                            crate::i18n::tr("Deform Path works at edit level 2 or 3"),
                            FontId::proportional(12.0),
                            theme::pal().text,
                        );
                    }
                }
                (None, _) => {
                    // no mesh yet: a click picks one
                    if inp.clicked
                        && let Some(ptr) = inp.press.or(inp.ptr)
                    {
                        let ux = ((ptr.x - origin.x) / scale - c.origin_x) / c.pixels_per_unit;
                        let uy = ((ptr.y - origin.y) / scale - c.origin_y) / c.pixels_per_unit;
                        if let Some(mi) = hit_visible(s, &mut self.pick_alpha, ux, uy, &|m| {
                            self.mesh_locked.contains(&m)
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
                let (dx, dy) = if inp.shift {
                    let d = crate::point_edit::axis_locked(vec2(dx, dy));
                    (d.x, d.y)
                } else {
                    (dx, dy)
                };
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
            /// Several points moved in one undo step (a point and its
            /// mirror): each slides over the art when its uv is known.
            SlideMany(usize, Vec<PointMove>),
            Add(usize, usize, [f32; 3]),
            /// A point placed by weights on three others, optionally
            /// closing a face onto an edge, with lines to `links`; `follow`
            /// makes it the start of the next line.
            AddFree(usize, [usize; 3], [f32; 3], Option<(u16, u16)>, Vec<u16>, bool),
            /// The same, placed on the mesh as it was on entering Mesh Edit.
            AddRef(usize, [usize; 3], [f32; 3], Option<(u16, u16)>, Vec<u16>, bool),
            Connect(usize, u16, u16),
            Cut(usize, u16, u16),
            EraseMany(usize, Vec<usize>),
            Del(usize, usize),
            Paste(usize, Vec<ayatsuri_core::edit::PastedPoint>, Vec<[usize; 3]>),
        }
        let mut mesh_op: Option<MeshOp> = None;
        if self.mesh_edit.is_none() {
            self.mesh_art = None;
            self.stroke_edit = None;
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
                if self.mesh_art.as_ref().is_none_or(|a| a.mesh != mi)
                    && let Some(reference) = s.puppet.model().art_meshes.get(mi).cloned()
                {
                    self.mesh_art = Some(MeshArt {
                        mesh: mi,
                        positions: g.positions.to_vec(),
                        uvs: g.uvs.to_vec(),
                        indices: g.indices.to_vec(),
                        reference,
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

                let (alt, ctrl, shift) =
                    ui.ctx().input(|i| (i.modifiers.alt, i.modifiers.ctrl, i.modifiers.shift));
                // the edge (face edge or drawn line) under the pointer
                let edge_at = |ptr: Pos2| -> Option<(u16, u16)> {
                    let (tris, _) = g.indices.as_chunks::<3>();
                    let lines = self.pending_edges.get(&mi).map(Vec::as_slice).unwrap_or(&[]);
                    tris.iter()
                        .flat_map(|t| [(t[0], t[1]), (t[1], t[2]), (t[2], t[0])])
                        .chain(lines.iter().copied())
                        .filter_map(|(a, b)| {
                            let pa = *screen_pts.get(a as usize)?;
                            let pb = *screen_pts.get(b as usize)?;
                            let seg = pb - pa;
                            let k =
                                ((ptr - pa).dot(seg) / seg.length_sq().max(1e-6)).clamp(0.0, 1.0);
                            Some(((a, b), ptr.distance(pa + seg * k)))
                        })
                        .filter(|&(_, d)| d < 6.0)
                        .min_by(|x, y| x.1.total_cmp(&y.1))
                        .map(|(e, _)| e)
                };
                // whether a new line from a to the pointer would cross an
                // edge or line already there (Cubism leaves the point single)
                let crosses_any = |a: usize, ptr: Pos2| -> bool {
                    let Some(&pa) = screen_pts.get(a) else { return true };
                    let (tris, _) = g.indices.as_chunks::<3>();
                    let lines = self.pending_edges.get(&mi).map(Vec::as_slice).unwrap_or(&[]);
                    let o =
                        |p: Pos2, q: Pos2, r: Pos2| (q - p).x * (r - p).y - (q - p).y * (r - p).x;
                    tris.iter()
                        .flat_map(|t| [(t[0], t[1]), (t[1], t[2]), (t[2], t[0])])
                        .chain(lines.iter().copied())
                        .any(|(x, y)| {
                            if x as usize == a || y as usize == a {
                                return false;
                            }
                            let (Some(&px), Some(&py)) =
                                (screen_pts.get(x as usize), screen_pts.get(y as usize))
                            else {
                                return false;
                            };
                            o(pa, ptr, px) * o(pa, ptr, py) < 0.0
                                && o(px, py, pa) * o(px, py, ptr) < 0.0
                        })
                };
                if self.edge_first.is_some_and(|v| v >= screen_pts.len()) {
                    self.edge_first = None;
                }
                // Alt+click deletes, as in Cubism: the point under the
                // pointer, or else the edge or line
                let editing_lines = matches!(self.mesh_mode, 1 | 3 | 4);
                if !space && editing_lines && alt && resp.clicked() {
                    if let Some((vi, _)) = near {
                        mesh_op = Some(MeshOp::Del(mi, vi));
                    } else if let Some(ptr) = resp.interact_pointer_pos()
                        && let Some((a, b)) = edge_at(ptr)
                    {
                        mesh_op = Some(MeshOp::Cut(mi, a, b));
                    }
                    self.edge_first = None;
                }
                // the cut tool takes a line with one click
                if !space
                    && self.mesh_mode == 4
                    && !alt
                    && resp.clicked()
                    && near.is_none()
                    && let Some(ptr) = resp.interact_pointer_pos()
                    && let Some((a, b)) = edge_at(ptr)
                {
                    mesh_op = Some(MeshOp::Cut(mi, a, b));
                }
                // Add vertex on an existing point: the chain moves to it,
                // drawing a line from the chain's end first
                if !space
                    && self.mesh_mode == 1
                    && !alt
                    && resp.clicked()
                    && let Some((vi, _)) = near
                {
                    match self.edge_first {
                        Some(first) if first == vi => self.edge_first = None,
                        Some(first) if !ctrl => {
                            mesh_op = Some(MeshOp::Connect(mi, first as u16, vi as u16));
                            if !shift {
                                self.edge_first = Some(vi);
                            }
                        }
                        _ => self.edge_first = Some(vi),
                    }
                }
                if !space
                    && self.mesh_mode == 1
                    && !alt
                    && resp.clicked()
                    && near.is_none()
                    && let Some(ptr) = resp.interact_pointer_pos()
                {
                    // the line from the chain's end, as Cubism draws it:
                    // not with Ctrl (a single point) and not across an edge
                    let chain = self
                        .edge_first
                        .filter(|&c| self.mesh_autoconnect && !ctrl && !crosses_any(c, ptr));
                    // add: inside a face splits it (or drops a loose point
                    // with auto connect off); outside, the point closes a face
                    // onto the nearest open edge; with no faces at all it is
                    // placed against the three nearest points. A point drawn
                    // from the chain is always loose, joined by its line only
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
                    let links: Vec<u16> = chain.map(|c| c as u16).into_iter().collect();
                    // a loose point is placed on the art as it was on
                    // entering Mesh Edit: its triangle under the pointer (or
                    // the nearest one), else three of its points
                    let on_art = self.mesh_art.as_ref().filter(|a| a.mesh == mi).and_then(|a| {
                        let pts: Vec<Pos2> = a
                            .positions
                            .as_chunks::<2>()
                            .0
                            .iter()
                            .map(|xy| to_screen(xy[0], xy[1]))
                            .collect();
                        let bary = |c: [usize; 3]| -> Option<[f32; 3]> {
                            let (s0, s1, s2) = (*pts.get(c[0])?, *pts.get(c[1])?, *pts.get(c[2])?);
                            let den = (s1.y - s2.y) * (s0.x - s2.x) + (s2.x - s1.x) * (s0.y - s2.y);
                            if den.abs() < 1e-6 {
                                return None;
                            }
                            let w0 = ((s1.y - s2.y) * (ptr.x - s2.x)
                                + (s2.x - s1.x) * (ptr.y - s2.y))
                                / den;
                            let w1 = ((s2.y - s0.y) * (ptr.x - s2.x)
                                + (s0.x - s2.x) * (ptr.y - s2.y))
                                / den;
                            Some([w0, w1, 1.0 - w0 - w1])
                        };
                        let best = a
                            .indices
                            .as_chunks::<3>()
                            .0
                            .iter()
                            .filter_map(|t| {
                                let c = [t[0] as usize, t[1] as usize, t[2] as usize];
                                bary(c).map(|w| (c, w, w[0].min(w[1]).min(w[2])))
                            })
                            .max_by(|x, y| x.2.total_cmp(&y.2))
                            .map(|(c, w, _)| (c, w));
                        best.or_else(|| {
                            let mut order: Vec<usize> = (0..pts.len()).collect();
                            order.sort_by(|&x, &y| {
                                let d = |i: usize| pts.get(i).map_or(f32::MAX, |p| p.distance(ptr));
                                d(x).total_cmp(&d(y))
                            });
                            let (&p0, &p1) = (order.first()?, order.get(1)?);
                            order
                                .iter()
                                .skip(2)
                                .find_map(|&p2| bary([p0, p1, p2]).map(|w| ([p0, p1, p2], w)))
                        })
                    });
                    if let Some((ti, _, w)) = inside
                        && self.mesh_autoconnect
                        && chain.is_none()
                    {
                        mesh_op = Some(MeshOp::Add(mi, ti, w));
                    } else if let Some((c, w)) = on_art {
                        // outside the faces, auto connect closes a face onto
                        // the nearest open edge (unless drawing a line)
                        let connect =
                            if self.mesh_autoconnect && chain.is_none() && inside.is_none() {
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
                        mesh_op = Some(MeshOp::AddRef(mi, c, w, connect, links, !shift));
                    } else if let Some((c, w, _)) = closest {
                        let connect = if self.mesh_autoconnect && chain.is_none() {
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
                        mesh_op = Some(MeshOp::AddFree(mi, c, w, connect, links, !shift));
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
                                mesh_op = Some(MeshOp::AddFree(mi, c, w, None, links, !shift));
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
                    let r =
                        self.tool_opts.brush_size.clamp(4.0, 400.0) * self.pen.brush(&self.prefs).0;
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
                        brush_ring(ui.ctx(), view, ptr, r, theme::pal().rotation_red);
                        ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
                    }
                } else {
                    self.erase_marked.clear();
                }
                // connect and cut both work on a pair: first click picks a
                // point, second click acts; clicking empty space resets
                let pairing = matches!(self.mesh_mode, 3 | 4);
                if !space && pairing && !alt && resp.clicked() {
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
                if !pairing && self.mesh_mode != 1 {
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
                                .map(|(c, w)| ayatsuri_core::edit::PastedPoint::At(c, w))
                                // a cleared mesh has too few points to place
                                // on: place it on the art as Mesh Edit began,
                                // as a click does
                                .or_else(|| {
                                    let a = self.mesh_art.as_ref().filter(|a| a.mesh == mi)?;
                                    let (rp, _) = a.positions.as_chunks::<2>();
                                    let rp: Vec<Pos2> =
                                        rp.iter().map(|q| pos2(q[0], q[1])).collect();
                                    (!a.indices.is_empty())
                                        .then(|| mesh_weights(&rp, &a.indices, p))
                                        .flatten()
                                        .or_else(|| basis_weights(&rp, p))
                                        .map(|(c, w)| ayatsuri_core::edit::PastedPoint::OnRef(c, w))
                                }),
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
                // points drag in Select and in the point and line tools
                // a drag only starts once the pointer has gone past the click
                // distance (wider in pen mode), so the point it grabs is the
                // one where the press began, and that first stretch counts
                let press = ui.ctx().input(|i| i.pointer.press_origin());
                let near_press = press.and_then(|o| {
                    screen_pts
                        .iter()
                        .enumerate()
                        .map(|(vi, sp)| (vi, sp.distance(o)))
                        .filter(|&(_, d)| d < reach)
                        .min_by(|a, b| a.1.total_cmp(&b.1))
                });
                if !space && !alt && matches!(self.mesh_mode, 0 | 1 | 3 | 4) {
                    if resp.drag_started()
                        && let Some((vi, _)) = near_press.or(near)
                        && let Some(local) = s.puppet.mesh_local_positions(mi)
                    {
                        let m_uvs = s
                            .puppet
                            .model()
                            .art_meshes
                            .get(mi)
                            .map(|m| m.uvs.clone())
                            .unwrap_or_default();
                        let partner = mirror_x
                            .and_then(|ax| {
                                point_edit::mirror_partner(&screen_pts, vi, ax, mirror_tol)
                            })
                            .map(|j| (j, 0.0, 0.0));
                        self.mesh_drag = Some(MeshDrag {
                            vertex: vi,
                            local,
                            dx: 0.0,
                            dy: 0.0,
                            start: screen_pts.clone(),
                            indices: g.indices.to_vec(),
                            model_uvs: m_uvs,
                            partner,
                        });
                    }
                    if resp.dragged()
                        && let Some(d) = &mut self.mesh_drag
                    {
                        // screen motion to model units, then through the
                        // parent deformer's local jacobian to parent space
                        let delta = match (resp.drag_started(), press, ptr) {
                            (true, Some(o), Some(p)) => p - o,
                            _ => resp.drag_delta(),
                        };
                        let mdx = delta.x / (scale * c.pixels_per_unit);
                        let mdy = delta.y / (scale * c.pixels_per_unit);
                        let at = d.vertex * 2;
                        let px = d.local.get(at).copied().unwrap_or(0.0) + d.dx;
                        let py = d.local.get(at + 1).copied().unwrap_or(0.0) + d.dy;
                        let (ldx, ldy) = parent_delta(&s.puppet, parent, px, py, mdx, mdy);
                        d.dx += ldx;
                        d.dy += ldy;
                        // the mirror moves the other way across the axis
                        if let Some((j, pdx, pdy)) = &mut d.partner {
                            let qx = d.local.get(*j * 2).copied().unwrap_or(0.0) + *pdx;
                            let qy = d.local.get(*j * 2 + 1).copied().unwrap_or(0.0) + *pdy;
                            let (ldx, ldy) = parent_delta(&s.puppet, parent, qx, qy, -mdx, mdy);
                            *pdx += ldx;
                            *pdy += ldy;
                        }
                    }
                    if resp.drag_stopped()
                        && let Some(d) = self.mesh_drag.take()
                        && (d.dx != 0.0 || d.dy != 0.0)
                    {
                        // the texture coordinate that was under the drop
                        let uv = ptr
                            .and_then(|p| mesh_weights(&d.start, &d.indices, p))
                            .and_then(|(t, w)| mix_uv(&d.model_uvs, t, w));
                        mesh_op = Some(match (uv, d.partner) {
                            (uv, Some((j, pdx, pdy))) => {
                                // the mirror's drop: its start, moved the
                                // pointer's way flipped across the axis
                                let drop = ptr.zip(d.start.get(d.vertex)).zip(d.start.get(j)).map(
                                    |((p, &a), &b)| pos2(b.x - (p.x - a.x), b.y + (p.y - a.y)),
                                );
                                let puv = drop
                                    .and_then(|p| mesh_weights(&d.start, &d.indices, p))
                                    .and_then(|(t, w)| mix_uv(&d.model_uvs, t, w));
                                MeshOp::SlideMany(
                                    mi,
                                    vec![(d.vertex, d.dx, d.dy, uv), (j, pdx, pdy, puv)],
                                )
                            }
                            (Some(uv), None) => MeshOp::Slide(mi, d.vertex, d.dx, d.dy, uv),
                            (None, None) => MeshOp::Move(mi, d.vertex, d.dx, d.dy),
                        });
                    }
                }
                // Mirror Edit: deleting a point deletes its mirror too
                if let Some(ax) = mirror_x {
                    let with_mirrors = |list: Vec<usize>| -> Vec<usize> {
                        let mut out = list.clone();
                        for &v in &list {
                            if let Some(j) =
                                point_edit::mirror_partner(&screen_pts, v, ax, mirror_tol)
                                && !out.contains(&j)
                            {
                                out.push(j);
                            }
                        }
                        out
                    };
                    mesh_op = match mesh_op {
                        Some(MeshOp::Del(m, v)) => {
                            let all = with_mirrors(vec![v]);
                            Some(if all.len() > 1 {
                                MeshOp::EraseMany(m, all)
                            } else {
                                MeshOp::Del(m, v)
                            })
                        }
                        Some(MeshOp::EraseMany(m, list)) => {
                            Some(MeshOp::EraseMany(m, with_mirrors(list)))
                        }
                        other => other,
                    };
                }

                // live preview: the dragged vertex follows the pointer, and
                // its mirror the other way across the axis
                if let (Some(d), Some(ptr)) = (&self.mesh_drag, ptr) {
                    if let Some((j, _, _)) = d.partner
                        && let (Some(&a), Some(&b)) = (d.start.get(d.vertex), d.start.get(j))
                        && let Some(sp) = screen_pts.get_mut(j)
                    {
                        *sp = pos2(b.x - (ptr.x - a.x), b.y + (ptr.y - a.y));
                    }
                    if let Some(sp) = screen_pts.get_mut(d.vertex) {
                        *sp = ptr;
                    }
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
                // a held point shows the closed hand; one under the pointer
                // the pointing hand
                if self.mesh_drag.is_some() {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
                } else if hover_vi.is_some() && !space && !matches!(self.mesh_mode, 5 | 7) {
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
                // stroke mesh-mapping (experimental), drawn over the mesh:
                // strokes stay put while another tool is used, and go with
                // Mesh Edit
                if self.mesh_mode == 7 && !self.prefs.stroke_mesh_on() {
                    self.mesh_mode = 0;
                }
                if self.mesh_mode == 7 && !space {
                    let sv = stroke_tool::View { origin, scale, rect: view };
                    let act = stroke_tool::stroke_tool(
                        ui,
                        &vp,
                        &resp,
                        &mut self.stroke_edit,
                        mi,
                        &self.tool_opts.stroke,
                        sv,
                        (alt, ctrl, shift),
                        (&screen_pts, g.indices),
                    );
                    if act == Some(stroke_tool::StrokeAct::Confirm) {
                        self.stroke_confirm = true;
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
            let why_not = s
                .puppet
                .model()
                .art_meshes
                .get(mi)
                .and_then(|am| mesh_unfinished(am.vertex_count as usize, am.indices.len() / 3));
            let fill = if why_not.is_some() {
                theme::pal().accent.gamma_multiply(0.35)
            } else {
                theme::pal().accent
            };
            vp.rect_filled(done, 4.0, fill);
            vp.text(
                done.center(),
                Align2::CENTER_CENTER,
                crate::i18n::tr("Done"),
                FontId::proportional(11.0),
                Color32::WHITE,
            );
            let mut esc = ui.ctx().input(|i| i.key_pressed(egui::Key::Escape));
            if esc && self.edge_first.is_some() {
                // the first Esc only lets go of the chain (Cubism's deselect)
                self.edge_first = None;
                esc = false;
            }
            if (dresp.clicked() || esc)
                && let Some(why) = why_not
            {
                set_notice(&mut self.notice, crate::i18n::tr(why));
            } else if dresp.clicked() || esc {
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
                MeshOp::SlideMany(mi, moves) => {
                    log::debug!("mesh {mi}: {} point(s) moved (Mirror Edit)", moves.len());
                    s.edit(|m| {
                        let mut any = false;
                        for &(vi, dx, dy, uv) in &moves {
                            any |= match uv {
                                Some(uv) => {
                                    ayatsuri_core::edit::slide_vertex(m, mi, vi, dx, dy, uv)
                                }
                                None => ayatsuri_core::edit::move_vertex(m, mi, vi, dx, dy),
                            };
                        }
                        any
                    })
                }
                MeshOp::Add(mi, ti, w) => {
                    let n = s.puppet.model().art_meshes.get(mi).map_or(0, |m| m.vertex_count);
                    let ok = s.edit(|m| ayatsuri_core::edit::add_vertex(m, mi, ti, w).is_some());
                    if ok && self.mesh_mode == 1 {
                        self.edge_first = Some(n as usize);
                    }
                    ok
                }
                MeshOp::AddRef(mi, corners, w, connect, links, follow) => {
                    let n =
                        s.puppet.model().art_meshes.get(mi).map_or(0, |m| m.vertex_count) as u16;
                    let reference = self.mesh_art.as_ref().map(|a| a.reference.clone());
                    let ok = reference.is_some_and(|r| {
                        s.edit(|m| {
                            ayatsuri_core::edit::add_vertex_from(m, mi, &r, corners, w, connect)
                                .is_some()
                        })
                    });
                    if ok {
                        for l in links {
                            connect_pair(&mut self.pending_edges, s, mi, n, l);
                        }
                        if follow && self.mesh_mode == 1 {
                            self.edge_first = Some(n as usize);
                        }
                    }
                    ok
                }
                MeshOp::AddFree(mi, corners, w, connect, links, follow) => {
                    let n =
                        s.puppet.model().art_meshes.get(mi).map_or(0, |m| m.vertex_count) as u16;
                    let ok = s.edit(|m| {
                        ayatsuri_core::edit::add_free_vertex(m, mi, corners, w, connect).is_some()
                    });
                    // the chain's line, closing a face if it makes a triangle
                    if ok {
                        for l in links {
                            connect_pair(&mut self.pending_edges, s, mi, n, l);
                        }
                        if follow && self.mesh_mode == 1 {
                            self.edge_first = Some(n as usize);
                        }
                    }
                    ok
                }
                MeshOp::Connect(mi, a, b) => connect_pair(&mut self.pending_edges, s, mi, a, b),
                MeshOp::Cut(mi, a, b) => {
                    let key = (a.min(b), a.max(b));
                    let has_edge = ayatsuri_core::edit::has_edge(s.puppet.model(), mi, a, b);
                    let pending = self.pending_edges.entry(mi).or_default();
                    if let Some(at) = pending.iter().position(|&e| e == key) {
                        pending.remove(at);
                        log::debug!("mesh {mi}: line {a}-{b} removed");
                        // a line drawn over an edge (an undone cut) goes with it
                        !has_edge || s.edit(|m| ayatsuri_core::edit::remove_edge(m, mi, a, b))
                    } else if has_edge {
                        // the faces on the edge go with it; their other sides
                        // stay as lines, as Cubism keeps them
                        let sides: Vec<(u16, u16)> = s
                            .puppet
                            .model()
                            .art_meshes
                            .get(mi)
                            .map(|m| {
                                m.indices
                                    .as_chunks::<3>()
                                    .0
                                    .iter()
                                    .filter(|t| t.contains(&a) && t.contains(&b))
                                    .flat_map(|&[x, y, z]| [(x, y), (y, z), (z, x)])
                                    .map(|(x, y)| (x.min(y), x.max(y)))
                                    .filter(|&e| e != key)
                                    .collect()
                            })
                            .unwrap_or_default();
                        log::debug!("mesh {mi}: edge {a}-{b} cut");
                        let ok = s.edit(|m| ayatsuri_core::edit::remove_edge(m, mi, a, b));
                        if ok {
                            let model = s.puppet.model();
                            for (x, y) in sides {
                                if !ayatsuri_core::edit::has_edge(model, mi, x, y)
                                    && !pending.contains(&(x, y))
                                {
                                    pending.push((x, y));
                                }
                            }
                        }
                        ok
                    } else {
                        log::debug!("mesh {mi}: {a} and {b} were not connected");
                        true
                    }
                }
                MeshOp::EraseMany(mi, marked) => {
                    self.pending_edges.remove(&mi);
                    self.edge_first = None;
                    self.mesh_picked.clear();
                    let n = marked.len();
                    let done = s.edit(|m| ayatsuri_core::edit::delete_vertices(m, mi, &marked));
                    if done {
                        log::debug!("mesh {mi}: erase brush removed {n} point(s)");
                    } else {
                        set_notice(
                            &mut self.notice,
                            crate::i18n::tr("A mesh keeps at least three points."),
                        );
                    }
                    done
                }
                MeshOp::Del(mi, vi) => {
                    // indices shift on delete, so pending lines can't survive it
                    self.pending_edges.remove(&mi);
                    self.edge_first = None;
                    self.mesh_picked.clear();
                    let done = s.edit(|m| ayatsuri_core::edit::delete_vertex(m, mi, vi));
                    if !done {
                        set_notice(
                            &mut self.notice,
                            crate::i18n::tr("A mesh keeps at least three points."),
                        );
                    }
                    done
                }
                MeshOp::Paste(mi, points, faces) => {
                    let reference = self
                        .mesh_art
                        .as_ref()
                        .filter(|a| a.mesh == mi)
                        .map(|a| a.reference.clone());
                    let mut got = None;
                    let done = s.edit(|m| {
                        got = ayatsuri_core::edit::paste_points(
                            m,
                            mi,
                            &points,
                            &faces,
                            reference.as_ref(),
                        );
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
                    let (pen_size, pen_strength) = self.pen.brush(&self.prefs);
                    let r = self.tool_opts.brush_size.clamp(4.0, 400.0) * pen_size;
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
                                *e = (*e + self.glue_strength * pen_strength * falloff * dt * 3.0)
                                    .min(1.0);
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
            if !lines.is_empty() {
                log::info!("{} guide line(s) added", lines.len());
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
                                log::info!("guide line added from the ruler");
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
            crate::windows::Dialog::new(crate::i18n::tr("Guide Line Settings"))
                .open(&mut open)
                .resizable(vec2(460.0, 380.0), vec2(380.0, 240.0))
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

        if self.tdeform.is_some() {
            let xf = temp_deform::ViewXf {
                origin,
                scale,
                cx: c.origin_x,
                cy: c.origin_y,
                ppu: c.pixels_per_unit,
            };
            self.temp_deform_canvas(ui, &resp, view, xf, space);
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
                self.view_zoom = (self.view_zoom / 1.25).clamp(ZOOM_MIN, ZOOM_MAX);
            }
            if ui.small_button("+").clicked() {
                self.view_zoom = (self.view_zoom * 1.25).clamp(ZOOM_MIN, ZOOM_MAX);
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
    // the curve the mesh follows, not straight lines between the points
    if pts.len() > 1 {
        let ctrl: Vec<Vec2> = pts.iter().map(|p| p.to_vec2()).collect();
        let curve: Vec<Pos2> =
            crate::bend_path::Curve::new(&ctrl).pts.iter().map(|p| p.to_pos2()).collect();
        vp.add(egui::Shape::line(curve, Stroke::new(1.6, theme::pal().warp_green)));
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
    const AS: M = M::ALT.plus(M::SHIFT);
    const N: M = M::NONE;
    &[
        ("File", "new-model", "New project", C, K::N),
        ("File", "open", "Open", C, K::O),
        ("File", "save", "Save", C, K::S),
        ("File", "save-as", "Save as", CS, K::S),
        ("File", "close", "Close", C, K::W),
        ("File", "export-image", "Export image", C, K::I),
        ("File", "export-moc3", "Export model", CA, K::S),
        ("File", "shortcuts", "Keyboard shortcuts", CAS, K::K),
        // every menu item gets an entry; Cubism's key when it has one, a few
        // of ours on keys it leaves free, the rest none until you pick one
        ("File", "new-animation", "New Animation…", CS, K::N),
        ("File", "new-composition", "New Composition…", N, K::F35),
        ("File", "export-animation", "Export Animation…", N, K::F35),
        ("File", "export-timelapse", "Export Timelapse…", N, K::F35),
        ("File", "import-motion", "Import Motion…", N, K::F35),
        ("File", "import-audio", "Import Audio…", N, K::F35),
        ("File", "import-artwork", "Import Artwork…", N, K::F35),
        ("File", "reimport-artwork", "Re-import Artwork…", N, K::F35),
        ("File", "project-info", "Project…", N, K::F35),
        ("File", "model-stats", "Model Statistics…", N, K::F35),
        ("File", "settings", "Environment Settings…", C, K::Comma),
        ("File", "exit", "Exit", C, K::Q),
        ("Edit", "undo", "Undo", C, K::Z),
        ("Edit", "redo", "Redo", CS, K::Z),
        ("Edit", "redo-alt", "Redo (alternate)", C, K::Y),
        ("Edit", "select-parent", "Select deformer for parent", C, K::D),
        ("Edit", "rename", "Rename the selected object", N, K::F2),
        (
            "Edit",
            "rename-deformer",
            "Rename the selected deformer, or the selected mesh's",
            M::SHIFT,
            K::F2,
        ),
        ("Modeling", "mesh-edit", "Edit mesh", C, K::E),
        ("Modeling", "auto-mesh", "Automatic mesh generator", CS, K::A),
        ("Modeling", "auto-connect", "Auto connect edges", C, K::R),
        ("Modeling", "glue-bind", "Bind glue", C, K::G),
        ("Modeling", "glue-unbind", "Unbind glue", CS, K::G),
        ("Modeling", "reset-params", "Reset parameters to default", C, K::Num1),
        ("Modeling", "four-corners", "Generate four corner forms", C, K::Num4),
        ("Modeling", "motion-mirror", "Motion Mirroring on the selected parameter", N, K::F35),
        ("Modeling", "lock-selected", "Lock/unlock selected", C, K::Num2),
        ("Modeling", "toggle-visible", "Show/hide selected", C, K::Num3),
        ("Modeling", "hide-unselected", "Hide unselected", CS, K::Num3),
        ("Modeling", "show-all", "Show all", CA, K::Num3),
        ("Tools", "tool-arrow", "Arrow tool", N, K::A),
        ("Tools", "tool-lasso", "Lasso tool", N, K::L),
        ("Tools", "tool-brush", "Brush selection tool", N, K::B),
        ("Tools", "tool-path", "Deform path tool", N, K::P),
        ("Tools", "tool-deform-brush", "Deform Brush", N, K::D),
        ("Tools", "tool-glue", "Glue", N, K::F35),
        ("Tools", "tool-artpath", "ArtPath", N, K::F35),
        ("Tools", "tool-auto-mesh", "Auto Mesh", N, K::F35),
        ("Tools", "tool-warp", "Warp Deformer", N, K::F35),
        ("Tools", "tool-rotation", "Rotation Deformer", N, K::F35),
        ("Tools", "tool-atlas", "Texture Atlas", N, K::F35),
        ("Tools", "level-1", "Edit level 1", N, K::Num1),
        ("Tools", "level-2", "Edit level 2", N, K::Num2),
        ("Tools", "level-3", "Edit level 3", N, K::Num3),
        ("View", "display-all", "Display all (fit)", C, K::Num0),
        ("View", "focus", "Focus on the selected object", N, K::F),
        ("View", "mirror-edit", "Mirror edit", CS, K::M),
        ("View", "solo", "Solo: show only the selected objects", N, K::S),
        ("View", "highlight-children", "Highlight deformer's child elements", CS, K::D),
        ("View", "zoom-in", "Zoom in", C, K::Equals),
        ("View", "zoom-out", "Zoom out", C, K::Minus),
        ("View", "guide-h-mouse", "Horizontal guide line at the mouse", N, K::H),
        ("View", "guide-v-mouse", "Vertical guide line at the mouse", N, K::V),
        ("View", "guide-both-mouse", "Vertical and horizontal guide lines at the mouse", N, K::G),
        ("View", "guides-clear", "Clear guide lines", M::SHIFT, K::G),
        ("View", "highlight-overlap", "Highlight Overlapping Polygons", N, K::F35),
        // Cubism's key for it
        ("View", "atlas-view", "Switch between Source Image and Texture Atlas", N, K::T),
        ("View", "guides-show", "Show Guide Lines", N, K::F35),
        ("View", "guides-center", "Add Guide Lines in the Center", N, K::F35),
        ("View", "guide-at-vertex", "Add a Guide Line at the Selected Vertex", N, K::F35),
        ("View", "guide-settings", "Guide Line settings…", N, K::F35),
        ("View", "guide-snap", "Snap moving vertices to guide lines", N, K::F35),
        ("View", "smooth-drag", "Smooth movement", N, K::F35),
        ("View", "rulers", "Rulers", N, K::F35),
        ("Edit", "select-all", "Select all", C, K::A),
        ("Edit", "search", "Search", C, K::F),
        ("Edit", "copy-form", "Copy form", CS, K::C),
        ("Edit", "paste-form", "Paste form", CS, K::V),
        ("Modeling", "select-children", "Select deformer child elements", CA, K::D),
        ("Modeling", "delete-deformer-content", "Delete deformer and its content", C, K::Delete),
        ("Modeling", "lock-unselected", "Lock unselected", CS, K::Num2),
        ("Modeling", "unlock-all", "Unlock all", CA, K::Num2),
        // the temporary deform tool has no keys until you pick some, as in Cubism
        ("Modeling", "temp-zoom", "Temporary deform: Enlarge/Reduce", N, K::F35),
        ("Modeling", "temp-rotation", "Temporary deform: Rotation", N, K::F35),
        ("Modeling", "temp-free", "Temporary deform: Free Transform", N, K::F35),
        ("Modeling", "temp-perspective", "Temporary deform: Perspective", N, K::F35),
        ("Modeling", "temp-distort", "Temporary deform: Distort", N, K::F35),
        ("Modeling", "temp-warp", "Temporary deform: Temporary Warp", N, K::F35),
        ("Modeling", "temp-path", "Temporary deform: Temporary Path", N, K::F35),
        ("Modeling", "temp-deselect", "Deselect the Temporary Deform Tool", N, K::F35),
        ("Modeling", "workspace-modeling", "Modeling Workspace", N, K::F35),
        ("Modeling", "create-warp", "Create Warp Deformer", N, K::F35),
        ("Modeling", "create-rotation", "Create Rotation Deformer", N, K::F35),
        ("Modeling", "conversion-divisions", "Conversion Divisions…", N, K::F35),
        ("Modeling", "new-part", "New Part", N, K::F35),
        ("Modeling", "group-into-part", "Group into New Part…", N, K::F35),
        ("Modeling", "clip-by", "Clip by…", N, K::F35),
        ("Modeling", "copy-ids", "Copy the selected meshes' IDs", CA, K::C),
        ("Modeling", "skinning", "Skinning…", N, K::F35),
        ("Modeling", "skin-params", "Generate Parameters for Rotation Deformers…", N, K::F35),
        ("Modeling", "skin-path", "Use Deform Path for Skinning", N, K::F35),
        ("Modeling", "texture-atlas", "Texture Atlas…", C, K::T),
        ("Modeling", "artpath-create", "Create ArtPath", N, K::F35),
        ("Modeling", "flip-h", "Flip Horizontally", N, K::F35),
        ("Modeling", "flip-v", "Flip Vertically", N, K::F35),
        ("Modeling", "flip-h-only", "Flip Warp Only Horizontally", N, K::F35),
        ("Modeling", "merge-vertices", "Merge Vertices", N, K::F35),
        ("Modeling", "flip-v-only", "Flip Warp Only Vertically", N, K::F35),
        ("Modeling", "physics", "Physics Settings…", N, K::F35),
        ("Modeling", "template-apply", "Apply Parameter Template…", N, K::F35),
        ("Modeling", "template-save", "Save Parameters as Template…", N, K::F35),
        // these work on the selected parameter
        ("Parameter", "keys-min-max", "Add Keys at Min and Max", N, K::F35),
        ("Parameter", "keys-min-default-max", "Add Keys at Min, Default and Max", N, K::F35),
        ("Parameter", "key-add-single", "Add Single Keyform", N, K::F35),
        ("Parameter", "key-delete-nearest", "Delete Nearest Key", N, K::F35),
        ("Parameter", "extended-interpolation", "Extended Interpolation…", CS, K::I),
        ("Parameter", "param-edit", "Edit Parameter…", N, K::F35),
        ("Parameter", "param-create", "Create Parameter…", N, K::F35),
        ("Parameter", "param-replicate", "Replicate parameters", N, K::F35),
        ("Parameter", "param-delete", "Delete the selected parameter", N, K::F35),
        ("Parameter", "param-link", "Link or unlink the parameter below", N, K::F35),
        ("Parameter", "param-group", "Group into New Folder…", N, K::F35),
        ("Parameter", "param-folder", "Create folder", N, K::F35),
        ("Parameter", "param-reset", "Reset the selected parameter to default", N, K::F35),
        ("Parameter", "param-select", "Select the objects keyed on the parameter", N, K::F35),
        ("Parameter", "param-reflect", "Reflect", N, K::F35),
        ("Parameter", "param-adjust", "Adjust…", N, K::F35),
        ("Parameter", "param-change", "Change…", N, K::F35),
        ("Parameter", "batch-reflect", "Batch Reflect…", N, K::F35),
        ("Parameter", "weight-limits", "Limit Settings for Blend Shape Weights…", N, K::F35),
        ("Parameter", "params-expand", "Expand all", N, K::F35),
        ("Parameter", "params-collapse", "Collapse all", N, K::F35),
        ("Parameter", "delete-empty-params", "Delete empty parameters", N, K::F35),
        ("Parameter", "delete-empty-groups", "Delete empty parameter groups", N, K::F35),
        ("Animation", "workspace-animation", "Animation Workspace", N, K::F35),
        ("Animation", "animation-details", "Edit Animation Details…", N, K::F35),
        ("Animation", "auto-key", "Auto key", N, K::F35),
        ("Animation", "anim-first", "Move to the beginning", N, K::Home),
        ("Animation", "anim-last", "Move to the end", N, K::End),
        ("Animation", "anim-prev-frame", "Previous frame", N, K::PageUp),
        ("Animation", "anim-next-frame", "Next frame", N, K::PageDown),
        ("Animation", "anim-prev-key", "Previous keyframe", N, K::J),
        ("Animation", "anim-next-key", "Next keyframe", N, K::K),
        ("Animation", "anim-play", "Playback", N, K::Enter),
        ("Animation", "anim-add-key", "Key the selected parameter", N, K::S),
        ("Animation", "anim-select-all", "Select every key", CA, K::A),
        (
            "Animation",
            "anim-nudge-left",
            "Nudge the selected keys a frame back",
            M::ALT,
            K::ArrowLeft,
        ),
        (
            "Animation",
            "anim-nudge-right",
            "Nudge the selected keys a frame on",
            M::ALT,
            K::ArrowRight,
        ),
        (
            "Animation",
            "anim-nudge-left-10",
            "Nudge the selected keys ten frames back",
            AS,
            K::ArrowLeft,
        ),
        (
            "Animation",
            "anim-nudge-right-10",
            "Nudge the selected keys ten frames on",
            AS,
            K::ArrowRight,
        ),
        ("Animation", "anim-work-start", "Work area start at the playhead", M::SHIFT, K::B),
        ("Animation", "anim-work-end", "Work area end at the playhead", M::SHIFT, K::N),
        ("Animation", "anim-layer-start", "Layer start at the playhead", N, K::OpenBracket),
        ("Animation", "anim-layer-end", "Layer end at the playhead", N, K::CloseBracket),
        ("Animation", "anim-layer-dup", "Duplicate the layer", M::SHIFT, K::D),
        ("Animation", "anim-layer-split", "Split the layer at the playhead", CS, K::D),
        ("Window", "timelapse", "Timelapse…", N, K::F35),
        ("Window", "workspace-save", "Save as New Workspace…", N, K::F35),
        ("Window", "workspace-manage", "Manage Workspaces…", N, K::F35),
        ("Window", "layout-export", "Export Current Layout…", N, K::F35),
        ("Window", "layout-import", "Import Layout…", N, K::F35),
        ("Window", "layout-reset", "Reset Current Layout", N, K::F35),
        ("Help", "whats-new", "What's New", N, K::F35),
        ("Help", "check-updates", "Check for Updates", N, K::F35),
    ]
};

pub(crate) fn format_shortcut(sc: &egui::KeyboardShortcut) -> String {
    // F35 stands for an action with no key yet
    if sc.logical_key == egui::Key::F35 && sc.modifiers.is_none() {
        return String::new();
    }
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
    // punctuation as its symbol ("[" rather than "OpenBracket"); letters,
    // digits, arrows and named keys keep their names, which every font has
    let key = sc.logical_key;
    let symbol = key.symbol_or_name();
    let punct = symbol.chars().count() == 1 && symbol.chars().all(|c| c.is_ascii_punctuation());
    out.push_str(if punct { symbol } else { key.name() });
    out
}

/// Ctrl, Shift, Alt and the Windows/Command key on their own: never a
/// shortcut's key, only its modifiers.
fn is_modifier(key: egui::Key) -> bool {
    use egui::Key as K;
    matches!(
        key,
        K::ShiftLeft
            | K::ShiftRight
            | K::ControlLeft
            | K::ControlRight
            | K::AltLeft
            | K::AltRight
            | K::SuperLeft
            | K::SuperRight
    )
}

/// What a key press binds to in Settings > Keyboard, or None while only
/// modifiers are down. With Shift held a digit or punctuation key gives
/// its own key (Ctrl+Shift+1, not Ctrl+Shift+!), as the shortcut is
/// matched on.
pub(crate) fn captured_key(
    key: egui::Key,
    physical: Option<egui::Key>,
    modifiers: egui::Modifiers,
) -> Option<(egui::Key, egui::Modifiers)> {
    if is_modifier(key) {
        return None;
    }
    let symbol = key.symbol_or_name();
    let shifted = modifiers.shift
        && symbol.chars().count() == 1
        && symbol.chars().all(|c| c.is_ascii_punctuation());
    let key = match physical {
        Some(p) if shifted && !is_modifier(p) => p,
        _ => key,
    };
    Some((key, modifiers))
}

fn parse_shortcut(text: &str) -> Option<egui::KeyboardShortcut> {
    let mut mods = egui::Modifiers::NONE;
    let mut key = None;
    for part in text.split('+') {
        match part {
            "Ctrl" | "Cmd" => mods = mods.plus(egui::Modifiers::COMMAND),
            "Shift" => mods = mods.plus(egui::Modifiers::SHIFT),
            "Alt" => mods = mods.plus(egui::Modifiers::ALT),
            // "Ctrl++" splits into an empty last part: the Plus key
            "" => key = Some(egui::Key::Plus),
            name => key = egui::Key::from_name(name),
        }
    }
    // a lone modifier saved by an older version falls back to the default
    key.filter(|k| !is_modifier(*k)).map(|k| egui::KeyboardShortcut::new(mods, k))
}

/// Physics off: the parameters it drives go back to rest, so they sit on
/// their keys for editing.
pub(crate) fn physics_to_rest(s: &mut Session) {
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
/// Which keyforms an Inspector edit sets, per object: with a parameter
/// selected that the object is keyed on, every keyform at that parameter's
/// current key (a toggle reads the same at every head angle and physics
/// swing); otherwise the one keyform the parameters sit on. None for an
/// object between keys. The flag says whether any edit spreads.
fn keyform_sets(
    s: &Session,
    targets: &[ayatsuri_core::edit::Target],
    across: Option<usize>,
) -> (Vec<Option<Vec<usize>>>, bool) {
    let values: Vec<f32> =
        (0..s.puppet.model().parameters.len()).map(|p| s.puppet.value(p)).collect();
    let mut spread = false;
    let sets = targets
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
    (sets, spread)
}

/// Properties' Multiply and Screen colour for meshes and deformers, on the
/// keyforms an edit sets (see `keyform_sets`), as in Cubism's Inspector.
/// The picker previews live in place; when it closes, the stored colours
/// go back and one edit commits, so undo sees a single change.
fn color_rows(
    ui: &mut Ui,
    s: &mut Session,
    targets: &[ayatsuri_core::edit::Target],
    across: Option<usize>,
    pick: &mut Option<ayatsuri_core::edit::ColorKind>,
    picked: &mut Option<(ayatsuri_core::edit::ColorKind, [u8; 3])>,
) {
    use ayatsuri_core::edit::{ColorKind, Target};
    let (sets, _) = keyform_sets(s, targets, across);
    let pairs: Vec<(Target, usize)> = targets
        .iter()
        .copied()
        .zip(sets.iter())
        .flat_map(|(t, ks)| ks.iter().flatten().map(move |&k| (t, k)))
        .collect();
    let all_on_keys = !targets.is_empty() && sets.iter().all(Option::is_some);
    for (kind, name, hint) in [
        (
            ColorKind::Multiply,
            "Multiply color",
            "Darkens: white leaves the art as it is. Passed down from a deformer to all under it",
        ),
        (
            ColorKind::Screen,
            "Screen color",
            "Lightens: black leaves the art as it is. Passed down from a deformer to all under it",
        ),
    ] {
        ui.horizontal(|ui| {
            ui.add_sized(
                [74.0, 16.0],
                egui::Label::new(
                    egui::RichText::new(crate::i18n::tr(name))
                        .color(theme::pal().text_faint)
                        .size(11.0),
                ),
            )
            .on_hover_text(crate::i18n::tr(hint));
            let Some(&(t0, k0)) = pairs.first().filter(|_| all_on_keys) else {
                ui.add_enabled(false, egui::Button::new("   ")).on_disabled_hover_text(
                    crate::i18n::tr(
                        "Between keys. Move the parameters onto a key to set the color there.",
                    ),
                );
                return;
            };
            let to8 = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
            let now = ayatsuri_core::edit::color_at(s.puppet.model(), t0, k0, kind);
            let mut rgb = now.map(to8);
            let live_id = egui::Id::new(("color-live", kind == ColorKind::Multiply, pairs.clone()));
            let resp = ui.color_edit_button_srgb(&mut rgb);
            let value = rgb.map(|c| c as f32 / 255.0);
            type Saved = Vec<(Target, Vec<f32>)>;
            if resp.changed() {
                if ui.ctx().data(|d| d.get_temp::<Saved>(live_id)).is_none() {
                    let before: Saved = targets
                        .iter()
                        .filter_map(|&t| Some((t, s.puppet.color_pool_mut(t, kind)?.0.clone())))
                        .collect();
                    ui.ctx().data_mut(|d| d.insert_temp(live_id, before));
                }
                // live: neutral pools filled as needed, numbers set in place
                for &(t, k) in &pairs {
                    if let Some((pool, n)) = s.puppet.color_pool_mut(t, kind) {
                        if pool.len() != n * 3 {
                            pool.resize(n * 3, kind.neutral());
                        }
                        for (c, v) in value.iter().enumerate() {
                            if let Some(slot) = pool.get_mut(k * 3 + c) {
                                *slot = *v;
                            }
                        }
                    }
                }
                s.dirty = true;
            }
            // the eyedropper: take the colour from the art on the canvas
            let picking = *pick == Some(kind);
            if ui
                .add(egui::Button::selectable(picking, egui_phosphor::regular::EYEDROPPER))
                .on_hover_text(crate::i18n::tr(
                    "Pick the colour from the art on the canvas (Esc cancels)",
                ))
                .clicked()
            {
                *pick = if picking { None } else { Some(kind) };
            }
            if let Some((k, [r, g, b])) = *picked
                && k == kind
            {
                *picked = None;
                let set = [r, g, b].map(|c| c as f32 / 255.0);
                if s.edit(|m| {
                    let mut any = false;
                    for &(t, k) in &pairs {
                        any |= ayatsuri_core::edit::set_color(m, t, k, kind, set);
                    }
                    any
                }) {
                    log::info!("{name} of {} keyform(s) picked from the art", pairs.len());
                }
            }
            let reset = ui
                .small_button("↺")
                .on_hover_text(crate::i18n::tr("Back to the color that leaves the art as it is"))
                .clicked();
            // the picker closed (or reset): put the numbers back, one edit
            let open = egui::Popup::is_any_open(ui.ctx());
            let saved: Option<Saved> = ui.ctx().data(|d| d.get_temp(live_id));
            if let Some(before) = saved.filter(|_| !open || reset) {
                ui.ctx().data_mut(|d| d.remove::<Saved>(live_id));
                for (t, pool) in before {
                    if let Some((p, _)) = s.puppet.color_pool_mut(t, kind) {
                        *p = pool;
                    }
                }
                let set = if reset { [kind.neutral(); 3] } else { value };
                if s.edit(|m| {
                    let mut any = false;
                    for &(t, k) in &pairs {
                        any |= ayatsuri_core::edit::set_color(m, t, k, kind, set);
                    }
                    any
                }) {
                    log::info!("{name} of {} keyform(s) set", pairs.len());
                }
                s.dirty = true;
            } else if reset
                && s.edit(|m| {
                    let mut any = false;
                    for &(t, k) in &pairs {
                        any |= ayatsuri_core::edit::set_color(m, t, k, kind, [kind.neutral(); 3]);
                    }
                    any
                })
            {
                log::info!("{name} of {} keyform(s) reset", pairs.len());
            }
        });
    }
}

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
    let (sets, spread) = keyform_sets(s, targets, across);
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

/// A mesh's own art, cropped to it: rendered on first use and kept until
/// the model changes. Parts palette rows and the canvas's mesh lists show it.
fn mesh_preview(ctx: &egui::Context, s: &mut Session, mi: usize) -> Option<egui::TextureHandle> {
    if !s.mesh_previews.contains_key(&mi) {
        let handle = crate::thumb::render_filtered(&s.puppet, &s.preview_tex, &|m| m == mi, 96)
            .map(|(w, h, px)| {
                let img = egui::ColorImage::from_rgba_unmultiplied([w, h], &px);
                ctx.load_texture(format!("mesh-prev-{mi}"), img, egui::TextureOptions::LINEAR)
            });
        s.mesh_previews.insert(mi, handle);
    }
    s.mesh_previews.get(&mi).cloned().flatten()
}

/// The hover preview: the art at up to 160 by 200 points.
fn big_preview(ui: &mut Ui, h: &egui::TextureHandle) {
    let sz = h.size_vec2();
    let k = (160.0 / sz.x).min(200.0 / sz.y);
    ui.image((h.id(), sz * k));
}

/// One mesh in a canvas pick list: a small picture of its art beside its
/// name, the art big on hover. Returns the row's response.
fn layer_row(ui: &mut Ui, s: &mut Session, mi: usize, name: &str, on: bool) -> egui::Response {
    match mesh_preview(ui.ctx(), s, mi) {
        Some(h) => {
            let sz = h.size_vec2();
            let k = (24.0 / sz.x).min(24.0 / sz.y);
            let resp = ui.add(
                egui::Button::image_and_text((h.id(), sz * k), name).selected(on).frame(false),
            );
            resp.on_hover_ui(|ui| big_preview(ui, &h))
        }
        None => ui.add(egui::Button::new(name).selected(on).frame(false)),
    }
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
    drag_ok: bool,
    rename_sc: &str,
) -> egui::Response {
    let (rect, resp) =
        ui.allocate_exact_size(vec2(ui.available_width(), 16.0), Sense::click_and_drag());
    // drag a mesh (or the picked meshes it is one of) onto a row of the
    // Deformer palette to put it under that deformer, or along the Parts
    // palette to move it
    if drag_ok && resp.drag_started() {
        resp.dnd_set_drag_payload(Child::Mesh(mi));
    }
    if resp.dragged() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
    }
    if *selected == Some(Sel::Mesh(mi)) || picked.contains(&mi) {
        paint_picked_row(ui, rect);
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
        if let Some(h) = mesh_preview(ui.ctx(), s, mi) {
            resp.clone().on_hover_ui(|ui| big_preview(ui, &h));
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
        let n = if picked.contains(&mi) { picked.len().max(1) } else { 1 };
        let label = if n > 1 {
            crate::i18n::trf("Group {count} Objects into New Part…", &[("count", &n)])
        } else {
            crate::i18n::tr("Group into New Part…").to_string()
        };
        if ui.button(label).clicked() {
            *cmd = Some(MeshRowCmd::Group(mi));
            ui.close();
        }
        if ui.add(egui::Button::new(crate::i18n::tr("Rename…")).shortcut_text(rename_sc)).clicked()
        {
            *cmd = Some(MeshRowCmd::Rename(mi));
            ui.close();
        }
        ui.menu_button(crate::i18n::tr("Flip"), |ui| {
            for (label, h) in [("Flip Horizontally", true), ("Flip Vertically", false)] {
                if ui.button(crate::i18n::tr(label)).clicked() {
                    *cmd = Some(MeshRowCmd::Flip(mi, h));
                    ui.close();
                }
            }
        });
        let model = s.puppet.model();
        let copy_of: Vec<usize> = if picked.contains(&mi) { picked.to_vec() } else { vec![mi] };
        let ids: Vec<&str> = copy_of
            .iter()
            .filter_map(|&m| model.art_meshes.get(m))
            .map(|m| m.id.as_str())
            .collect();
        let label = if ids.len() > 1 {
            crate::i18n::trf("Copy {count} IDs", &[("count", &ids.len())])
        } else {
            crate::i18n::tr("Copy ID").to_string()
        };
        if ui
            .button(label)
            .on_hover_text(crate::i18n::tr("Paste into another mesh's Clip by to clip by it"))
            .clicked()
        {
            ui.ctx().copy_text(ids.join(", "));
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
    resp
}

/// What a Parts palette drag carries: parts, or (from a mesh row, whose
/// payload is the `Child::Mesh` the Deformer palette also takes) a mesh
/// and the pick it belongs to.
#[derive(Clone, Debug)]
enum Dragged {
    Parts(Vec<usize>),
    Mesh(usize),
}

/// A Deformer palette row as a drop place, the way the Parts palette's
/// are: the top or bottom edge of a row drops beside it, under the row's
/// own parent (deformers have no order among siblings, so above and below
/// are the same), marked with a line; the middle of a deformer row drops
/// into it, marked with a box, and so does the lower half of an open one,
/// whose first child shows just under it. `into` is the row's deformer
/// (None for a mesh row), `parent` the deformer its row sits under.
fn def_drop_target(
    ui: &Ui,
    resp: &egui::Response,
    into: Option<usize>,
    parent: Option<u32>,
    open: bool,
    out: &mut Option<DefCmd>,
) {
    let Some(dragged) = egui::DragAndDrop::payload::<Child>(ui.ctx()).map(|d| *d) else {
        return;
    };
    let rect = resp.rect;
    let Some(ptr) = ui.input(|i| i.pointer.latest_pos()).filter(|p| rect.contains(*p)) else {
        return;
    };
    let f = (ptr.y - rect.min.y) / rect.height().max(1.0);
    let target = match into {
        Some(d) if f >= 0.3 && (open || f <= 0.7) => Some(d),
        _ => None,
    };
    // a deformer dropped on itself goes nowhere
    if target.is_some() && target.map(Child::Deformer) == Some(dragged) {
        return;
    }
    let stroke = Stroke::new(2.0, theme::pal().accent);
    match target {
        Some(_) => {
            ui.painter().rect_stroke(rect.shrink(1.0), 2.0, stroke, StrokeKind::Inside);
        }
        None if f < 0.5 => {
            ui.painter().hline(rect.x_range(), rect.min.y, stroke);
        }
        None => {
            ui.painter().hline(rect.x_range(), rect.max.y, stroke);
        }
    }
    if ui.input(|i| i.pointer.any_released()) {
        let to = target.or(parent.map(|p| p as usize));
        *out = Some(DefCmd::Reparent(dragged, to));
    }
}

/// A Parts palette row as a drop place, as in Cubism: the top of a row
/// drops above it and the bottom below it; the middle of a part row drops
/// into the part, at the bottom of its list, and so does the lower half of
/// an open part, whose first child shows just under it. Marks the place
/// while a drag hovers and reports it on release.
fn palette_drop_target(
    ui: &Ui,
    resp: &egui::Response,
    row: PRow,
    open: bool,
    out: &mut Option<(Dragged, PDrop)>,
) {
    let dragged = match egui::DragAndDrop::payload::<Dragged>(ui.ctx()) {
        Some(d) => (*d).clone(),
        None => match egui::DragAndDrop::payload::<Child>(ui.ctx()).as_deref() {
            Some(Child::Mesh(m)) => Dragged::Mesh(*m),
            _ => return,
        },
    };
    let rect = resp.rect;
    let Some(ptr) = ui.input(|i| i.pointer.latest_pos()).filter(|p| rect.contains(*p)) else {
        return;
    };
    let f = (ptr.y - rect.min.y) / rect.height().max(1.0);
    let to = match row {
        PRow::Mesh(_) if f < 0.5 => PDrop::Above(row),
        PRow::Mesh(_) => PDrop::Below(row),
        PRow::Part(_) if f < 0.3 => PDrop::Above(row),
        PRow::Part(_) if f > 0.7 && !open => PDrop::Below(row),
        PRow::Part(p) => PDrop::Into(p),
    };
    let stroke = Stroke::new(2.0, theme::pal().accent);
    match to {
        PDrop::Above(_) => {
            ui.painter().hline(rect.x_range(), rect.min.y, stroke);
        }
        PDrop::Below(_) => {
            ui.painter().hline(rect.x_range(), rect.max.y, stroke);
        }
        PDrop::Into(_) => {
            ui.painter().rect_stroke(rect.shrink(1.0), 2.0, stroke, StrokeKind::Inside);
        }
    }
    if ui.input(|i| i.pointer.any_released()) {
        *out = Some((dragged, to));
    }
}

fn truncate(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        s.to_string()
    } else {
        s.chars().take(max_chars.saturating_sub(1)).chain(['…']).collect()
    }
}

/// How the sliders show their keys this frame, the way Cubism does: keys
/// every selected object has are green, yellow when only some of them have
/// one, and the key Motion Mirroring last wrote to carries a red glow.
#[derive(Default)]
struct KeyMarks {
    /// How many objects are selected.
    picked: usize,
    /// Per parameter: the selected objects' keys and how many have each.
    keys: Vec<Vec<(f32, usize)>>,
    mirrored: Option<(usize, f32)>,
    /// Colours picked for single keys.
    colors: Vec<(usize, f32, [u8; 3])>,
    /// Per parameter: drives blend shapes (its base key is drawn square).
    blend: Vec<bool>,
}

/// One slider's share of [`KeyMarks`].
#[derive(Default)]
struct KeyLook {
    picked: usize,
    keys: Vec<(f32, usize)>,
    mirrored: Option<f32>,
    colors: Vec<(f32, [u8; 3])>,
    blend: bool,
}

impl KeyMarks {
    fn look(&self, pi: usize) -> KeyLook {
        KeyLook {
            picked: self.picked,
            keys: self.keys.get(pi).cloned().unwrap_or_default(),
            mirrored: self.mirrored.filter(|m| m.0 == pi).map(|m| m.1),
            colors: self.colors.iter().filter(|c| c.0 == pi).map(|c| (c.1, c.2)).collect(),
            blend: self.blend.get(pi).copied().unwrap_or(false),
        }
    }
}

impl EditorUi {
    fn key_marks(&mut self) -> KeyMarks {
        use ayatsuri_core::edit::Target;
        let targets = self.param_targets();
        if self.mirror_mark.as_ref().is_some_and(|m| m.2 != targets) {
            self.mirror_mark = None;
        }
        let Some(s) = &self.session else { return KeyMarks::default() };
        let model = s.puppet.model();
        let mut keys: Vec<Vec<(f32, usize)>> = vec![Vec::new(); model.parameters.len()];
        for t in &targets {
            let binding = match *t {
                Target::ArtMesh(i) => model.art_meshes.get(i).map(|m| &m.binding),
                Target::Deformer(i) => model.deformers.get(i).map(|d| &d.binding),
                Target::Part(i) => model.parts.get(i).map(|p| &p.binding),
            };
            let axes = binding.map(|b| b.axes.as_slice()).unwrap_or_default();
            let shapes = match *t {
                Target::ArtMesh(i) => model.art_meshes.get(i).map(|m| m.blend_shapes.as_slice()),
                Target::Deformer(i) => model.deformers.get(i).map(|d| d.blend_shapes.as_slice()),
                Target::Part(i) => model.parts.get(i).map(|p| p.blend_shapes.as_slice()),
            };
            let held = axes.iter().map(|a| (a.parameter, a.keys.clone())).chain(
                shapes.unwrap_or_default().iter().map(|b| {
                    // a blend shape holds its base key too
                    let mut k = b.keys.clone();
                    k.push(0.0);
                    (b.parameter, k)
                }),
            );
            for (param, axis_keys) in held {
                let Some(list) = keys.get_mut(param as usize) else { continue };
                for k in axis_keys {
                    match list.iter_mut().find(|(e, _)| (*e - k).abs() < 1e-5) {
                        Some((_, n)) => *n += 1,
                        None => list.push((k, 1)),
                    }
                }
            }
        }
        KeyMarks {
            picked: targets.len(),
            keys,
            mirrored: self.mirror_mark.as_ref().map(|m| (m.0, m.1)),
            colors: s.key_colors.clone(),
            blend: model.parameters.iter().map(|p| p.blend_shape).collect(),
        }
    }
}

/// Pair colours for key dots: keys the same distance either side of the
/// default share one, so the pairs Motion Mirroring works on match up.
const PAIR_COLORS: [Color32; 6] = [
    Color32::from_rgb(0x4F, 0xC3, 0xF7),
    Color32::from_rgb(0xBA, 0x68, 0xC8),
    Color32::from_rgb(0xF0, 0x62, 0x92),
    Color32::from_rgb(0x4D, 0xB6, 0xAC),
    Color32::from_rgb(0xFF, 0xB7, 0x4D),
    Color32::from_rgb(0x90, 0xA4, 0xAE),
];

/// Key `k`'s pair colour: white at the default, else by how far from it,
/// nearest first.
fn pair_color(keys: &[f32], default: f32, span: f32, k: f32) -> Color32 {
    let tol = span * 1e-3;
    let d = (k - default).abs();
    if d <= tol {
        return Color32::WHITE;
    }
    let mut dists: Vec<f32> = Vec::new();
    for &e in keys {
        let de = (e - default).abs();
        if de > tol && !dists.iter().any(|&x| (x - de).abs() <= tol) {
            dists.push(de);
        }
    }
    dists.sort_by(f32::total_cmp);
    let i = dists.iter().position(|&x| (x - d).abs() <= tol).unwrap_or(0);
    PAIR_COLORS.get(i % PAIR_COLORS.len()).copied().unwrap_or(Color32::WHITE)
}

/// The Cubism parameter slider: a track, a dot on every key, and the
/// filled handle at the current value. Returns whether the value moved,
/// the response, and whether a right-click stuck the slider onto a key
/// (Cubism's key adhesion; the menu stays shut then).
fn param_slider(
    ui: &mut Ui,
    width: f32,
    value: &mut f32,
    (min, max, default): (f32, f32, f32),
    (keys, between): (&[f32], &[f32]),
    look: &KeyLook,
) -> (bool, egui::Response, bool) {
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
    // the keyframes extended interpolation adds between keys: small hollow
    // marks, under the keys, as they are not keys you edit
    for &k in between {
        painter.circle_stroke(pos2(x_of(k), cy), 1.8, Stroke::new(1.0, theme::pal().text_faint));
    }
    for &k in keys {
        let at = pos2(x_of(k), cy);
        if look.mirrored.is_some_and(|m| (m - k).abs() < 1e-5) {
            // where the mirror wrote: a red glow under the dot
            for (r, a) in [(9.0, 40u8), (7.0, 70), (5.0, 110)] {
                painter.circle_filled(at, r, Color32::from_rgba_unmultiplied(0xE5, 0x39, 0x35, a));
            }
        }
        let held = look.keys.iter().find(|(e, _)| (*e - k).abs() < 1e-5).map_or(0, |e| e.1);
        // the fill tells keys apart: a picked colour, else the key's pair
        // colour (keys mirrored across the default share one)
        let fill = match look.colors.iter().find(|c| (c.0 - k).abs() < 1e-5) {
            Some(&(_, [r, g, b])) => Color32::from_rgb(r, g, b),
            None => pair_color(keys, default, span, k),
        };
        // the outline is Cubism's: green when every selected object has the
        // key, yellow when only some do
        let ring = if held == 0 {
            Stroke::new(1.0, theme::pal().key_ring)
        } else if held < look.picked {
            Stroke::new(1.6, Color32::from_rgb(0xF4, 0xD0, 0x3F))
        } else {
            Stroke::new(1.6, Color32::from_rgb(0x4C, 0xC0, 0x6A))
        };
        let selected = held > 0 && (*value - k).abs() <= span * 1e-4;
        let red = Stroke::new(1.4, Color32::from_rgb(0xE5, 0x39, 0x35));
        if look.blend {
            // a blend shape parameter's keys are squares, as in Cubism, in the
            // key's colour like any other key
            painter.rect(
                Rect::from_center_size(at, vec2(7.2, 7.2)),
                0.0,
                fill,
                ring,
                egui::StrokeKind::Inside,
            );
            // the selected key: the slider sits on it
            if selected {
                painter.rect_stroke(
                    Rect::from_center_size(at, vec2(10.0, 12.0)),
                    0.0,
                    red,
                    egui::StrokeKind::Middle,
                );
            }
        } else {
            painter.circle(at, if held > 0 { 3.3 } else { 3.0 }, fill, ring);
            // the selected key: the slider sits on it
            if selected {
                painter.circle_stroke(at, 4.6, red);
            }
        }
    }
    // a faint tick at the default, the way Cubism hints the rest value
    let dx = x_of(default);
    painter.line_segment(
        [pos2(dx, cy - 4.0), pos2(dx, cy + 4.0)],
        Stroke::new(1.0, theme::pal().track),
    );
    // the handle: a square on a blend shape parameter, as its keys are
    let handle = pos2(x_of(*value), cy);
    if look.blend {
        painter.rect_filled(
            Rect::from_center_size(handle, vec2(7.2, 9.0)),
            0.0,
            theme::pal().accent,
        );
    } else {
        painter.circle_filled(handle, 4.5, theme::pal().accent);
    }

    let mut changed = false;
    // the left button only: a right-click opens the menu and must leave the
    // value where it is (bar the snap onto a key, below)
    if (resp.dragged_by(egui::PointerButton::Primary) || resp.clicked())
        && let Some(p) = resp.interact_pointer_pos()
    {
        let f = ((p.x - rect.min.x - pad) / (rect.width() - pad * 2.0)).clamp(0.0, 1.0);
        *value = min + f * span;
        changed = true;
    }
    // right-click near a key: the slider sticks to it, exactly
    let mut stuck = false;
    if resp.secondary_clicked()
        && let Some(p) = resp.interact_pointer_pos()
        && let Some(k) = keys
            .iter()
            .copied()
            .map(|k| (k, (x_of(k) - p.x).abs()))
            .filter(|&(_, d)| d <= 6.0)
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(k, _)| k)
        && (*value - k).abs() > span * 1e-4
    {
        *value = k;
        changed = true;
        stuck = true;
    }
    (changed, resp, stuck)
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

/// Texture pages' alpha for picking, small enough to keep every page: the
/// largest of each block of texels, so thin strokes still count.
#[derive(Default)]
pub(crate) struct AlphaPages {
    pages: std::collections::HashMap<usize, (usize, u32, u32, Vec<u8>)>,
}

impl AlphaPages {
    const MAX: u32 = 1024;

    fn alpha(&mut self, pngs: &[Vec<u8>], page: usize, u: f32, v: f32) -> Option<f32> {
        let png = pngs.get(page)?;
        let fresh = self.pages.get(&page).is_some_and(|(len, ..)| *len == png.len());
        if !fresh {
            let img = image::load_from_memory(png).ok()?.into_rgba8();
            let (w, h) = img.dimensions();
            let step = w.max(h).div_ceil(Self::MAX).max(1);
            let (sw, sh) = (w.div_ceil(step), h.div_ceil(step));
            let mut small = vec![0u8; (sw * sh) as usize];
            for (x, y, px) in img.enumerate_pixels() {
                let at = ((y / step) * sw + x / step) as usize;
                if let Some(a) = small.get_mut(at) {
                    *a = (*a).max(px.0[3]);
                }
            }
            self.pages.insert(page, (png.len(), sw, sh, small));
        }
        let (_, w, h, px) = self.pages.get(&page)?;
        let x = ((u.clamp(0.0, 1.0) * *w as f32) as u32).min(w.saturating_sub(1));
        let y = ((v.clamp(0.0, 1.0) * *h as f32) as u32).min(h.saturating_sub(1));
        px.get((y * w + x) as usize).map(|&a| a as f32 / 255.0)
    }
}

/// Where a point lands in a mesh: its texture page and UV, or None outside.
fn uv_at(puppet: &ayatsuri_core::Puppet, mi: usize, ux: f32, uy: f32) -> Option<(usize, f32, f32)> {
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
    Some((g.texture as usize, u, v))
}

/// Every mesh drawn at a point, front first: shown with its parts, not
/// skipped, not clipped away there, and with its art drawn there (a
/// transparent margin of a mesh is not a hit). A model without its
/// textures picks by the triangles alone.
fn hits_visible(
    s: &Session,
    alpha: &mut AlphaPages,
    ux: f32,
    uy: f32,
    skip: &dyn Fn(usize) -> bool,
) -> Vec<usize> {
    let order: Vec<(usize, f32, Vec<u32>, bool)> = s
        .puppet
        .drawables()
        .filter(|d| d.opacity > 0.004)
        .map(|d| {
            let inverted = d.flags & ayatsuri_format::model::flags::INVERTED_MASK != 0;
            (d.mesh, d.opacity, d.masks.to_vec(), inverted)
        })
        .collect();
    let mut drawn = |mi: usize| -> Option<f32> {
        let (page, u, v) = uv_at(&s.puppet, mi, ux, uy)?;
        Some(alpha.alpha(&s.texture_pngs, page, u, v).unwrap_or(1.0))
    };
    let mut out = Vec::new();
    for (mi, opacity, masks, inverted) in order.into_iter().rev() {
        if skip(mi) || !mesh_pickable(s, mi) {
            continue;
        }
        let Some(a) = drawn(mi) else { continue };
        if a * opacity < 0.1 {
            continue;
        }
        if !masks.is_empty() {
            let in_mask = masks.iter().any(|&k| drawn(k as usize).is_some_and(|a| a > 0.1));
            if in_mask == inverted {
                continue;
            }
        }
        out.push(mi);
    }
    out
}

/// The front mesh drawn at a point (see hits_visible).
fn hit_visible(
    s: &Session,
    alpha: &mut AlphaPages,
    ux: f32,
    uy: f32,
    skip: &dyn Fn(usize) -> bool,
) -> Option<usize> {
    hits_visible(s, alpha, ux, uy, skip).first().copied()
}

/// A plain click: the front mesh, or, clicked again within a few points of
/// the last pick while the picked mesh is still under the pointer, the
/// mesh under it (round to the front after the back one).
fn cycle_pick(
    hits: &[usize],
    selected: Option<usize>,
    at: Pos2,
    last: Option<Pos2>,
) -> Option<usize> {
    let again = last.is_some_and(|l| l.distance(at) <= 4.0);
    match selected.and_then(|cur| hits.iter().position(|&h| h == cur)) {
        Some(i) if again => hits.get((i + 1) % hits.len()).copied(),
        _ => hits.first().copied(),
    }
}

/// The scale that fits the canvas in the view, at zoom 1.
/// The canvas zoom's range, as a multiple of the size that fits the view.
const ZOOM_MIN: f32 = 0.02;
const ZOOM_MAX: f32 = 400.0;

/// Zoom by `factor` keeping the canvas point under `pivot` where it is:
/// the new zoom and pan. The pan follows the zoom that happened, so at a
/// limit, where the zoom stops, the view stays put instead of flying off.
fn zoom_about(zoom: f32, pan: Vec2, view_center: Pos2, pivot: Pos2, factor: f32) -> (f32, Vec2) {
    let want = (zoom * factor).clamp(ZOOM_MIN, ZOOM_MAX);
    let done = want / zoom.max(1e-6);
    let center = view_center + pan;
    (want, pan + (center - pivot) * (done - 1.0))
}

fn fit_scale(view: Rect, c: ayatsuri_format::model::Canvas) -> f32 {
    let (cw, ch) = (c.width.max(1.0), c.height.max(1.0));
    ((view.width() / cw).min(view.height() / ch) * 0.92).max(1e-4)
}

/// Where the canvas's top-left corner sits on screen, and screen points per
/// canvas pixel, at this zoom and pan.
fn view_origin_scale(
    view: Rect,
    c: ayatsuri_format::model::Canvas,
    zoom: f32,
    pan: Vec2,
) -> (Pos2, f32) {
    let (cw, ch) = (c.width.max(1.0), c.height.max(1.0));
    let scale = fit_scale(view, c) * zoom;
    (view.center() + pan - vec2(cw, ch) * scale * 0.5, scale)
}

/// Say something over the canvas for a few seconds (EditorUi::notice; a
/// free function so code holding the session can still set it).
fn set_notice(slot: &mut Option<(String, Option<f64>)>, text: &str) {
    log::info!("notice: {text}");
    *slot = Some((text.to_string(), None));
}

/// Why a mesh being edited cannot be finished yet: it needs three points and
/// a triangle to be drawn at all. Cancel still puts it back.
fn mesh_unfinished(verts: usize, tris: usize) -> Option<&'static str> {
    if verts < 3 {
        Some("A mesh needs at least three points. Add points, or Cancel to undo the edit.")
    } else if tris == 0 {
        Some(
            "This mesh has no triangles, so nothing would be drawn. Connect points, or Cancel to undo the edit.",
        )
    } else {
        None
    }
}

/// A mesh the canvas may pick: shown, and every part above it shown.
/// Every art mesh under deformer `d`, however deep, walking up each mesh's
/// deformer chain at most once per deformer so a loop cannot spin.
fn meshes_under_deformer(model: &Model, d: usize) -> Vec<usize> {
    (0..model.art_meshes.len())
        .filter(|&mi| {
            let mut at = model.art_meshes.get(mi).and_then(|m| m.parent_deformer);
            for _ in 0..=model.deformers.len() {
                match at {
                    Some(x) if x as usize == d => return true,
                    Some(x) => at = model.deformers.get(x as usize).and_then(|q| q.parent_deformer),
                    None => return false,
                }
            }
            false
        })
        .collect()
}

/// Solo as this frame draws it: the soloed meshes by index and the
/// inactive color, or None when Solo is off.
fn solo_view(s: &Session, prefs: &Prefs) -> Option<crate::gpu_view::Solo> {
    let ids = s.solo.as_ref()?;
    let meshes = s
        .puppet
        .model()
        .art_meshes
        .iter()
        .enumerate()
        .filter(|(_, m)| ids.contains(&m.id))
        .map(|(i, _)| i)
        .collect();
    let [r, g, b, a] = prefs.solo_tint.unwrap_or(SOLO_TINT);
    let f = |x: u8| x as f32 / 255.0;
    Some(crate::gpu_view::Solo { meshes, tint: [f(r), f(g), f(b), f(a)] })
}

/// While Solo is on, a badge in the view's lower left: click it to leave
/// Solo, or set the inactive color beside it. True when the color changed
/// (the caller saves the preferences).
fn solo_badge(ui: &mut Ui, view: Rect, s: &mut Session, prefs: &mut Prefs) -> bool {
    if s.solo.is_none() {
        return false;
    }
    let at = Rect::from_min_size(view.left_bottom() + vec2(12.0, -36.0), vec2(220.0, 24.0));
    let mut changed = false;
    ui.scope_builder(egui::UiBuilder::new().max_rect(at), |ui| {
        ui.horizontal(|ui| {
            let off = ui
                .add(
                    egui::Button::new(
                        // the icon font's X: the bold text font has no ✕ and drew a box
                        egui::RichText::new(format!(
                            "{}  {}",
                            crate::i18n::tr("Solo"),
                            egui_phosphor::regular::X
                        ))
                        .strong(),
                    )
                    .fill(theme::pal().accent),
                )
                .on_hover_text(crate::i18n::tr("Leave Solo (S)"));
            if off.clicked() {
                s.solo = None;
            }
            let mut c = prefs.solo_tint.unwrap_or(SOLO_TINT);
            let mut color = Color32::from_rgba_unmultiplied(c[0], c[1], c[2], c[3]);
            ui.label(crate::i18n::tr("Inactive color"));
            if ui.color_edit_button_srgba(&mut color).changed() {
                c = color.to_srgba_unmultiplied();
                prefs.solo_tint = Some(c);
                changed = true;
            }
        });
    });
    changed
}

/// The inactive color Solo puts over everything else: dimmed and faded.
const SOLO_TINT: [u8; 4] = [90, 90, 100, 110];

/// Highlight Overlapping Polygons' default color: a strong red, half seen.
pub(crate) const OVERLAP_COLOR: [u8; 4] = [255, 64, 96, 120];

/// While Solo is on, only the soloed meshes can be picked (Cubism's Solo
/// keeps the rest from being deformed by mistake).
fn mesh_pickable(s: &Session, mi: usize) -> bool {
    let soloed = s
        .solo
        .as_ref()
        .is_none_or(|ids| s.puppet.model().art_meshes.get(mi).is_some_and(|m| ids.contains(&m.id)));
    soloed
        && s.mesh_shown.get(mi).copied().unwrap_or(true)
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
    marks: &KeyMarks,
    actions: &mut Vec<ParamAction>,
) {
    let Some((min, max, default, decimals)) =
        s.puppet.model().parameters.get(pi).map(|p| (p.min, p.max, p.default, p.decimals))
    else {
        return;
    };
    let display = s.param_names.get(pi).cloned().unwrap_or_default();
    let keys = s.param_keys.get(pi).cloned().unwrap_or_default();
    let between = s.param_curve_points.get(pi).cloned().unwrap_or_default();
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
        let (nr, nresp) = ui.allocate_exact_size(
            vec2(92.0, 16.0),
            if full { Sense::click_and_drag() } else { Sense::click() },
        );
        if *sel_param == Some(pi) || s.param_picked.contains(&pi) {
            ui.painter().rect_filled(nr, 2.0, theme::pal().select);
        }
        // names start clear of where a label colour's stripe goes
        ui.painter().text(
            nr.left_center() + vec2(6.0, 0.0),
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
            // drag the name to reorder; a drop lands before this row
            let mut drop = None;
            let line = Rect::from_min_max(nr.min, pos2(ui.max_rect().max.x, nr.max.y));
            let here = param_menu::Moving::Param(pi);
            param_menu::drag_target(ui, &nresp, line, here, param_menu::Row::Param(pi), &mut drop);
            if let Some(c) = drop {
                actions.push(ParamAction::Cmd(c));
            }
        }
        let slider_w = (ui.available_width() - 68.0).max(40.0);
        let (changed, slider_resp, on_key) = param_slider(
            ui,
            slider_w,
            &mut v,
            (min, max, default),
            (&keys, &between),
            &marks.look(pi),
        );
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
        let span = (max - min).max(1e-6);
        let on = keys.iter().copied().find(|k| (v - k).abs() <= span * 1e-4).map(|k| {
            let now =
                s.key_colors.iter().find(|c| c.0 == pi && (c.1 - k).abs() < 1e-5).map(|c| c.2);
            (k, now)
        });
        if full && !on_key {
            slider_resp.context_menu(|ui| {
                if let Some(c) = param_menu::slider_menu(ui, pi, v, on, can_edit) {
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
        let nresp = ui.interact(
            nr,
            egui::Id::new(("pad-name", pa, p)),
            if menus { Sense::click_and_drag() } else { Sense::click() },
        );
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
            let mut drop = None;
            let here = param_menu::Moving::Param(p);
            param_menu::drag_target(ui, &nresp, nr, here, param_menu::Row::Param(p), &mut drop);
            if let Some(c) = drop {
                actions.push(ParamAction::Cmd(c));
            }
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
    // the left button only: the right-click menu acts on the values as they
    // are, so opening it must not move the dot
    if (resp.dragged_by(egui::PointerButton::Primary) || resp.clicked())
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
            if let Some(c) = param_menu::slider_menu(ui, pa, va, None, can_edit) {
                actions.push(ParamAction::Cmd(c));
            }
        });
        ui.menu_button(truncate(&name_b, 18), |ui| {
            if let Some(c) = param_menu::slider_menu(ui, pb, vb, None, can_edit) {
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

/// A brush's circle under the pointer, drawn above the canvas and the
/// mesh overlay with a dark edge, so it shows over any art.
fn brush_ring(ctx: &egui::Context, view: Rect, at: Pos2, r: f32, color: Color32) {
    let painter = ctx
        .layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::new("brush-ring")))
        .with_clip_rect(view);
    painter.circle_stroke(at, r, Stroke::new(3.0, Color32::from_black_alpha(140)));
    painter.circle_stroke(at, r, Stroke::new(1.4, color));
}

#[cfg(test)]
mod pair_color_tests {
    use super::*;

    #[test]
    fn mirrored_keys_share_a_colour() {
        let keys = [-30.0, -15.0, 0.0, 15.0, 30.0];
        let c = |k| pair_color(&keys, 0.0, 60.0, k);
        assert_eq!(c(0.0), Color32::WHITE);
        assert_eq!(c(-30.0), c(30.0));
        assert_eq!(c(-15.0), c(15.0));
        assert_ne!(c(15.0), c(30.0));
    }
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
                blend_shape: false,
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
            blend_shape: false,
        });
        m.art_meshes.push(ArtMesh {
            id: "M".into(),
            parent_part: None,
            parent_deformer: None,
            visible: true,
            enabled: true,
            texture: 0,
            flags: 0,
            blend: Default::default(),
            vertex_count: 3,
            uvs: vec![0.0; 6],
            indices: vec![0, 1, 2],
            masks: vec![],
            binding: Binding::default(),
            opacities: vec![1.0],
            draw_orders: vec![500.0],
            positions: vec![0.2, 0.2, 0.8, 0.2, 0.5, 0.8],
            blend_shapes: Vec::new(),
            clip_group: None,
            clip_inherit: false,
            multiply: Vec::new(),
            screen: Vec::new(),
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
                blend: Default::default(),
                vertex_count: 3,
                uvs: vec![0.0, 0.0, 1.0, 0.0, 0.0, 1.0],
                indices: vec![0, 1, 2],
                masks: vec![],
                binding: Binding::default(),
                opacities: vec![1.0],
                draw_orders: vec![500.0],
                positions: vec![0.0, 0.0, 1.0, 0.0, 0.0, 1.0],
                blend_shapes: Vec::new(),
                clip_group: None,
                clip_inherit: false,
                multiply: Vec::new(),
                screen: Vec::new(),
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

    #[test]
    fn clip_by_on_a_pick_clips_every_mesh_in_it() {
        let mut ed = editor();
        ed.multi_sel = vec![0, 1];
        ed.selected = Some(Sel::Mesh(0));
        ed.start_clip_pick(0);
        assert_eq!((ed.clip_pick, ed.clip_also.clone()), (Some(0), vec![1]));
        assert_eq!(ed.multi_sel, vec![0, 1]);
        let s = ed.session.as_mut().unwrap();
        assert!(s.edit(|m| add_mask_to(m, &[0, 1], 2)));
        let masks: Vec<Vec<u32>> =
            s.puppet.model().art_meshes.iter().map(|m| m.masks.clone()).collect();
        assert_eq!(masks, vec![vec![2], vec![2], vec![]]);
        // once is enough, and it is one undo step
        assert!(!s.edit(|m| add_mask_to(m, &[0, 1], 2)));
        s.undo();
        assert!(s.puppet.model().art_meshes.iter().all(|m| m.masks.is_empty()));
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

    #[test]
    fn a_deformer_tool_opens_its_dialog_at_once() {
        let mut ed = editor();
        // with nothing selected too: the dialog follows the selection
        ed.choose_tool(Tool::WarpCreate);
        assert_eq!(ed.pending_create, Some((true, None)));
        ed.pending_create = None;
        ed.selected = Some(Sel::Mesh(0));
        ed.choose_tool(Tool::RotationCreate);
        assert!(ed.tool == Tool::RotationCreate);
        assert_eq!(ed.pending_create, Some((false, None)));
    }

    #[test]
    fn done_waits_for_three_points_and_a_triangle() {
        assert!(mesh_unfinished(2, 0).is_some());
        assert!(mesh_unfinished(4, 0).is_some());
        assert_eq!(mesh_unfinished(3, 1), None);
    }

    #[test]
    fn picking_skips_hidden_meshes_and_cycles_on_a_second_click() {
        let mut ed = editor();
        ed.session.as_mut().unwrap().puppet.update();
        let s = ed.session.as_ref().unwrap();
        let mut alpha = AlphaPages::default();
        // three meshes on one spot, B hidden: only A and C are hits
        let hits = hits_visible(s, &mut alpha, 0.2, 0.2, &|_| false);
        assert_eq!(hits.len(), 2);
        assert!(!hits.contains(&1));
        assert!(hits_visible(s, &mut alpha, 5.0, 5.0, &|_| false).is_empty());
        // a first click takes the front; again on the spot, the one under it
        let at = pos2(10.0, 10.0);
        let front = cycle_pick(&hits, None, at, None);
        assert_eq!(front, hits.first().copied());
        let next = cycle_pick(&hits, front, at + vec2(1.0, 0.0), Some(at));
        assert_eq!(next, hits.get(1).copied());
        assert_eq!(cycle_pick(&hits, next, at, Some(at)), front);
        // a click somewhere else starts from the front again
        assert_eq!(cycle_pick(&hits, next, at + vec2(30.0, 0.0), Some(at)), front);
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
            blend_shape: false,
        });
        model.art_meshes.push(ArtMesh {
            id: "ArtMesh0".into(),
            parent_part: None,
            parent_deformer: None,
            visible: true,
            enabled: true,
            texture: 0,
            flags: 0,
            blend: Default::default(),
            vertex_count: 1,
            uvs: vec![0.0, 0.0],
            indices: vec![0, 0, 0],
            masks: vec![],
            binding: Binding {
                curves: Vec::new(),
                axes: vec![BindingAxis { parameter: 0, keys: vec![-1.0, 0.0, 1.0] }],
            },
            opacities: vec![1.0; 3],
            draw_orders: vec![500.0; 3],
            positions: vec![-1.0, 0.0, 0.0, 0.0, 2.0, 0.0],
            blend_shapes: Vec::new(),
            clip_group: None,
            clip_inherit: false,
            multiply: Vec::new(),
            screen: Vec::new(),
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
    fn zooming_at_the_limit_leaves_the_view_where_it_is() {
        let (c, pivot) = (pos2(400.0, 300.0), pos2(700.0, 100.0));
        // below the limit the point under the pointer stays under it
        let (z, pan) = zoom_about(2.0, vec2(10.0, -5.0), c, pivot, 1.5);
        assert_eq!(z, 3.0);
        let under = |z: f32, pan: Vec2| (pivot - (c + pan)) / z;
        let before = under(2.0, vec2(10.0, -5.0));
        assert!((under(z, pan) - before).length() < 1e-3);
        // at the limit nothing moves, however hard the wheel turns
        let (z, pan) = zoom_about(ZOOM_MAX, vec2(10.0, -5.0), c, pivot, 1.5);
        assert_eq!((z, pan), (ZOOM_MAX, vec2(10.0, -5.0)));
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
    fn shortcuts_show_punctuation_as_its_symbol_and_read_back() {
        let sc = |m, k| egui::KeyboardShortcut::new(m, k);
        assert_eq!(format_shortcut(&sc(egui::Modifiers::NONE, egui::Key::OpenBracket)), "[");
        assert_eq!(
            format_shortcut(&sc(egui::Modifiers::COMMAND, egui::Key::CloseBracket)),
            "Ctrl+]"
        );
        assert_eq!(format_shortcut(&sc(egui::Modifiers::SHIFT, egui::Key::B)), "Shift+B");
        assert_eq!(format_shortcut(&sc(egui::Modifiers::NONE, egui::Key::ArrowLeft)), "Left");
        let back = parse_shortcut("Ctrl+]").map(|s| s.logical_key);
        assert_eq!(back, Some(egui::Key::CloseBracket));
    }

    #[test]
    fn a_combination_is_captured_whole_and_reads_back() {
        use egui::{Key as K, Modifiers as M};
        let ctrl_shift = M::COMMAND.plus(M::SHIFT);
        // holding Ctrl, then Shift, waits; S completes Ctrl+Shift+S
        assert_eq!(captured_key(K::ControlLeft, Some(K::ControlLeft), M::COMMAND), None);
        assert_eq!(captured_key(K::ShiftLeft, Some(K::ShiftLeft), ctrl_shift), None);
        let (k, m) = captured_key(K::S, Some(K::S), ctrl_shift).unwrap();
        let text = format_shortcut(&egui::KeyboardShortcut::new(m, k));
        assert_eq!(text, "Ctrl+Shift+S");
        assert_eq!(parse_shortcut(&text), Some(egui::KeyboardShortcut::new(ctrl_shift, K::S)));
        // Shift+1 types "!": the key under the finger is what binds
        assert_eq!(
            captured_key(K::Exclamationmark, Some(K::Num1), M::SHIFT),
            Some((K::Num1, M::SHIFT))
        );
        // Ctrl++ reads back as Plus; a lone modifier saved before does not bind
        assert_eq!(parse_shortcut("Ctrl++").map(|s| s.logical_key), Some(K::Plus));
        assert_eq!(parse_shortcut("Ctrl+ControlLeft"), None);
    }

    #[test]
    fn dragging_places_parameters_and_folders_like_layers() {
        use crate::session::ParamGroup;
        use param_menu::{Moving, Place};
        let mut ed = editor();
        let group = |name: &str, params: Vec<usize>| ParamGroup {
            name: name.into(),
            params,
            open: true,
            color: None,
        };
        let layout = |ed: &EditorUi| -> Vec<(String, Vec<usize>)> {
            let s = ed.session.as_ref().unwrap();
            s.groups.iter().map(|g| (g.name.clone(), g.params.clone())).collect()
        };
        let row = |n: &str, p: &[usize]| (n.to_string(), p.to_vec());
        let s = ed.session.as_mut().unwrap();
        s.groups = vec![group("A", vec![0, 1, 2]), group("B", vec![3])];
        // above and below a row: same folder
        ed.run_param_cmd(ParamCmd::Drop(Moving::Param(2), Place::Before(0)));
        ed.run_param_cmd(ParamCmd::Drop(Moving::Param(0), Place::After(1)));
        assert_eq!(layout(&ed), vec![row("A", &[2, 1, 0]), row("B", &[3])]);
        // above a folder: out of every folder, between them
        ed.run_param_cmd(ParamCmd::Drop(Moving::Param(1), Place::BeforeGroup(1)));
        assert_eq!(layout(&ed), vec![row("A", &[2, 0]), row("", &[1]), row("B", &[3])]);
        // below a collapsed folder: out, and the loose run grows
        ed.run_param_cmd(ParamCmd::Drop(Moving::Param(0), Place::AfterGroup(0)));
        assert_eq!(layout(&ed), vec![row("A", &[2]), row("", &[0, 1]), row("B", &[3])]);
        // into a folder: first inside it; the emptied run goes
        ed.run_param_cmd(ParamCmd::Drop(Moving::Param(0), Place::Into(2)));
        ed.run_param_cmd(ParamCmd::Drop(Moving::Param(1), Place::Into(2)));
        assert_eq!(layout(&ed), vec![row("A", &[2]), row("B", &[1, 0, 3])]);
        // under the last row: the end, outside every folder
        ed.run_param_cmd(ParamCmd::Drop(Moving::Param(3), Place::End));
        assert_eq!(layout(&ed), vec![row("A", &[2]), row("B", &[1, 0]), row("", &[3])]);
        // a folder dropped beside a loose parameter splits the run
        ed.run_param_cmd(ParamCmd::Drop(Moving::Param(2), Place::After(3)));
        ed.run_param_cmd(ParamCmd::Drop(Moving::Group(1), Place::After(3)));
        assert_eq!(
            layout(&ed),
            vec![row("A", &[]), row("", &[3]), row("B", &[1, 0]), row("", &[2])]
        );
        // and folders reorder around each other
        ed.run_param_cmd(ParamCmd::Drop(Moving::Group(2), Place::BeforeGroup(0)));
        assert_eq!(layout(&ed), vec![row("B", &[1, 0]), row("A", &[]), row("", &[3, 2])]);
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
    fn reset_all_evaluates_the_model_again() {
        let mut ed = editor();
        let s = ed.session.as_mut().unwrap();
        s.puppet.set_value(0, 1.0);
        s.dirty = false;
        ed.run_param_cmd(ParamCmd::ResetAll);
        let s = ed.session.as_ref().unwrap();
        assert_eq!(s.puppet.value(0), 0.0);
        assert!(s.dirty);
    }

    #[test]
    fn reflect_turns_the_key_values_around() {
        let mut ed = editor();
        ed.selected = Some(Sel::Mesh(0));
        // keyed x - 1 at -1, x at 0, x + 2 at 1: Reflect swaps the ends
        ed.run_param_cmd(ParamCmd::Reflect(0));
        let m = mesh(&ed);
        assert_eq!(m.binding.axes[0].keys, vec![-1.0, 0.0, 1.0]);
        assert_eq!(m.positions, vec![2.0, 0.0, 0.0, 0.0, -1.0, 0.0]);
        // Batch Reflect does it again for every object on the parameter
        ed.selected = None;
        ed.batch_reflect(&[true]);
        assert_eq!(mesh(&ed).positions, vec![-1.0, 0.0, 0.0, 0.0, 2.0, 0.0]);
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
    fn deformer_rows_pick_with_ctrl_and_shift_as_in_cubism() {
        let order = [4, 2, 7, 1];
        let mut ed = EditorUi::new();
        ed.pick_def_rows(2, false, false, &order);
        assert!(ed.selected == Some(Sel::Deformer(2)) && ed.def_pick == vec![2]);
        // Ctrl adds just the one clicked, and takes it off again
        ed.pick_def_rows(1, false, true, &order);
        assert_eq!(def_group(ed.selected, &ed.def_pick), vec![2, 1]);
        ed.pick_def_rows(1, false, true, &order);
        assert!(ed.selected == Some(Sel::Deformer(2)) && ed.def_pick == vec![2]);
        // Shift takes every row from the last click to this one
        ed.pick_def_rows(2, false, false, &order);
        ed.pick_def_rows(1, true, false, &order);
        assert_eq!(def_group(ed.selected, &ed.def_pick), vec![2, 7, 1]);
        // a plain click starts over
        ed.pick_def_rows(4, false, false, &order);
        assert_eq!(def_group(ed.selected, &ed.def_pick), vec![4]);
        // a mesh selection does not count as a deformer pick
        ed.selected = Some(Sel::Mesh(0));
        assert!(def_group(ed.selected, &ed.def_pick).is_empty());
    }

    #[test]
    fn shift_click_reaches_across_mesh_and_deformer_rows() {
        // deformer 0 holding mesh 5, then deformer 1 holding meshes 6 and 7
        let rows = [(true, 0), (false, 5), (true, 1), (false, 6), (false, 7)];
        let mut ed = EditorUi::new();
        // a mesh clicked, then Shift on a deformer above it: the deformers
        // between them are picked
        ed.def_row_anchor = Some((false, 7));
        assert!(ed.pick_across(true, 0, &rows));
        assert_eq!(def_group(ed.selected, &ed.def_pick), vec![0, 1]);
        assert!(ed.multi_sel.is_empty());
        // and the other way: meshes from a deformer down
        ed.def_row_anchor = Some((true, 0));
        assert!(ed.pick_across(false, 6, &rows));
        assert_eq!(ed.multi_sel, vec![5, 6]);
        assert!(ed.selected == Some(Sel::Mesh(6)) && ed.def_pick.is_empty());
        // the same kind keeps the usual range
        assert!(!ed.pick_across(true, 1, &rows));
    }

    #[test]
    fn only_top_level_deformers_join_a_pick() {
        use ayatsuri_core::edit::{NewDeformer, create_deformer};
        let mut m = Model::new();
        let warp = |m: &mut Model, parent: Option<u32>, id: &str| {
            let kind = NewDeformer::Warp { rows: 1, cols: 1 };
            create_deformer(m, kind, &[], parent, (0.0, 0.0, 1.0, 1.0), None, id.into()).unwrap()
        };
        let a = warp(&mut m, None, "A");
        let b = warp(&mut m, Some(a as u32), "B");
        let c = warp(&mut m, Some(b as u32), "C");
        let d = warp(&mut m, None, "D");
        // siblings at the top, as in Cubism, all move
        assert_eq!(outermost(&m, &[a, d]), vec![a, d]);
        // lower levels can be picked too; inside a picked one they ride along
        assert_eq!(outermost(&m, &[c, a, d]), vec![a, d]);
        assert_eq!(outermost(&m, &[c, b]), vec![b]);
        let ids = |v: Vec<usize>| -> Vec<String> {
            v.into_iter().map(|i| m.deformers[i].id.clone()).collect()
        };
        assert_eq!(ids(outermost(&m, &[c])), vec!["C"]);
        // in the palette: Ctrl+click on a child of a picked deformer is
        // refused, and on its parent drops the child
        let mut ed = EditorUi::new();
        let ctx = egui::Context::default();
        ed.session = Some(Session::create(&ctx, "t".into(), String::new(), m).unwrap());
        let order = [a, b, c, d];
        ed.pick_def_rows(b, false, false, &order);
        ed.pick_def_rows(c, false, true, &order);
        assert_eq!(def_group(ed.selected, &ed.def_pick), vec![b]);
        ed.pick_def_rows(d, false, true, &order);
        ed.pick_def_rows(a, false, true, &order);
        assert_eq!(def_group(ed.selected, &ed.def_pick), vec![d, a]);
        // a Shift run over a branch keeps its top: A and D, not B and C
        ed.pick_def_rows(a, false, false, &order);
        ed.pick_def_rows(d, true, false, &order);
        assert_eq!(def_group(ed.selected, &ed.def_pick), vec![a, d]);
    }

    #[test]
    fn a_picked_group_flips_across_its_middle() {
        use ayatsuri_core::edit::{NewDeformer, create_deformer};
        let mut m = Model::new();
        for (id, x) in [("Left", 0.0), ("Right", 2.0)] {
            let kind = NewDeformer::Warp { rows: 1, cols: 1 };
            create_deformer(&mut m, kind, &[], None, (x, 0.0, x + 1.0, 1.0), None, id.into())
                .unwrap();
        }
        let ctx = egui::Context::default();
        let mut ed = EditorUi::new();
        ed.session = Some(Session::create(&ctx, "t".into(), String::new(), m).unwrap());
        ed.selected = Some(Sel::Deformer(0));
        ed.def_pick = vec![0, 1];
        let span = |ed: &mut EditorUi, d: usize| {
            let s = ed.session.as_mut().unwrap();
            s.puppet.update();
            world_extent(&s.puppet, ayatsuri_core::edit::Target::Deformer(d))
                .map(|e| (e.0, e.2))
                .unwrap()
        };
        let (l, r) = (span(&mut ed, 0), span(&mut ed, 1));
        let mid2 = l.0 + r.1;
        ed.flip_selection(true);
        let near =
            |a: (f32, f32), b: (f32, f32)| (a.0 - b.0).abs() < 1e-4 && (a.1 - b.1).abs() < 1e-4;
        // across the middle of the pair: each lands on the other's side
        let (l2, r2) = (span(&mut ed, 0), span(&mut ed, 1));
        assert!(near(l2, (mid2 - l.1, mid2 - l.0)), "{l:?} -> {l2:?}");
        assert!(near(r2, (mid2 - r.1, mid2 - r.0)), "{r:?} -> {r2:?}");
        assert!(l2.0 > r2.1, "they swapped sides");
    }

    #[test]
    fn default_shortcuts_never_share_a_key() {
        // Modeling and the Timeline split a few keys by workspace
        let mut seen: Vec<(&str, egui::Modifiers, egui::Key)> = Vec::new();
        for &(_, id, _, m, k) in KEY_ACTIONS {
            if k == egui::Key::F35 {
                continue;
            }
            for &(other, om, ok) in &seen {
                let split = id.starts_with("anim-") != other.starts_with("anim-");
                assert!(om != m || ok != k || split, "{id} and {other} share a key");
            }
            seen.push((id, m, k));
        }
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
            blend: Default::default(),
            vertex_count: 4,
            uvs: vec![0.0, 0.0, 1.0, 0.0, 1.0, 1.0, 0.0, 1.0],
            indices: vec![0, 1, 2, 0, 2, 3],
            masks: vec![],
            binding: Binding::default(),
            opacities: vec![1.0],
            draw_orders: vec![500.0],
            positions: vec![0.0, 0.0, 1.0, 0.0, 1.0, 1.0, 0.0, 1.0],
            blend_shapes: Vec::new(),
            clip_group: None,
            clip_inherit: false,
            multiply: Vec::new(),
            screen: Vec::new(),
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

/// The colour blends as the Properties menu lists them: Cubism's order,
/// with the two modes from before 5.3 last.
const COLOR_BLEND_MENU: [ayatsuri_format::model::ColorBlend; 18] = {
    use ayatsuri_format::model::ColorBlend::*;
    [
        Normal,
        Add,
        AddGlow,
        Darken,
        Multiply,
        ColorBurn,
        LinearBurn,
        Lighten,
        Screen,
        ColorDodge,
        Overlay,
        SoftLight,
        HardLight,
        LinearLight,
        Hue,
        Color,
        AddCompatible,
        MultiplyCompatible,
    ]
};

/// What the Clip by row asks for.
struct ClipRow {
    /// The masks after an edit, when they changed.
    masks: Option<Vec<u32>>,
    /// Pick on canvas was clicked.
    toggle_pick: bool,
}

/// Clip by, in one row like Cubism's clipping field: the masks as chips,
/// then a field to type a name or ID into (suggestions drop down, Enter
/// adds the top one or every ID in an `A, B` list), paste IDs or a mesh
/// copied with Ctrl+C into, Backspace on it empty to take the last mask
/// off; then picking on the canvas and copying the masks' IDs.
/// Add `mask` to every target's clipping masks; false when none changed.
fn add_mask_to(model: &mut ayatsuri_format::Model, targets: &[usize], mask: usize) -> bool {
    let mut changed = false;
    for &t in targets {
        let Some(mut list) = model.art_meshes.get(t).map(|m| m.masks.clone()) else { continue };
        if !list.contains(&(mask as u32)) {
            list.push(mask as u32);
            changed |= ayatsuri_core::edit::set_masks(model, t, &list);
        }
    }
    changed
}

fn clip_by_row(
    ui: &mut Ui,
    s: &Session,
    mesh: usize,
    masks: &[u32],
    buf: &mut Option<(usize, String)>,
    picking: bool,
) -> ClipRow {
    let model = s.puppet.model();
    let mut want: Vec<u32> = masks.to_vec();
    let mut toggle_pick = false;
    let mut text = match buf {
        Some((m, t)) if *m == mesh => t.clone(),
        _ => String::new(),
    };
    let field = ui.make_persistent_id(("clip-by", mesh));
    let suggest = field.with("suggest");
    let add = |want: &mut Vec<u32>, mi: usize| {
        if mi != mesh && !want.contains(&(mi as u32)) {
            want.push(mi as u32);
        }
    };
    let index_of = |id: &str| model.art_meshes.iter().position(|m| m.id == id);
    if ui.memory(|m| m.has_focus(field)) {
        // a whole copied mesh would paste as its JSON: it adds its IDs
        let ids = ui.ctx().input_mut(|i| {
            let mut ids = Vec::new();
            i.events.retain(|e| match e {
                egui::Event::Paste(t) => match crate::mesh_clip::ClipFile::from_text(t) {
                    Some(clip) => {
                        ids.extend(clip.meshes.iter().map(|c| c.mesh.id.clone()));
                        false
                    }
                    None => true,
                },
                _ => true,
            });
            ids
        });
        for id in ids {
            match index_of(&id) {
                Some(mi) => add(&mut want, mi),
                None => log::warn!("Clip by: no mesh has the ID {id}"),
            }
        }
    }
    ui.horizontal(|ui| {
        ui.add_sized(
            [74.0, 16.0],
            egui::Label::new(
                egui::RichText::new(crate::i18n::tr("Clip by"))
                    .color(theme::pal().text_faint)
                    .size(11.0),
            ),
        );
        let hint = crate::i18n::tr("Type or paste an ID");
        let r = ui
            .add(
                egui::TextEdit::singleline(&mut text)
                    .id(field)
                    .hint_text(hint)
                    .desired_width((ui.available_width() - 60.0).max(80.0)),
            )
            .on_hover_text(crate::i18n::tr(
                "Type a mesh's name or ID and pick it from the list, or paste IDs separated by \
                 commas (Copy ID on a mesh, or Ctrl+C) and press Enter. Backspace on an empty \
                 field takes the last mask off.",
            ));
        let (enter, esc, back) = ui.input(|i| {
            (
                i.key_pressed(egui::Key::Enter),
                i.key_pressed(egui::Key::Escape),
                i.key_pressed(egui::Key::Backspace),
            )
        });
        if r.has_focus() && back && text.is_empty() && buf.as_ref().is_some_and(|b| b.1.is_empty())
        {
            want.pop();
        }
        // the meshes the text matches, by name or ID; paths first
        let needle = text.trim().to_lowercase();
        let is_path = |mi: usize| ayatsuri_core::edit::art_path_of(model, mi).is_some();
        let mut matches: Vec<usize> = (0..model.art_meshes.len())
            .filter(|&mi| mi != mesh && !want.contains(&(mi as u32)))
            .filter(|&mi| {
                needle.is_empty()
                    || s.mesh_name(mi).to_lowercase().contains(&needle)
                    || model
                        .art_meshes
                        .get(mi)
                        .is_some_and(|m| m.id.to_lowercase().contains(&needle))
            })
            .collect();
        matches.sort_by_key(|&mi| !is_path(mi));
        // a press on the list takes the focus off the field first: that is
        // a pick on its way, not the end of typing, so the list stays and
        // the text waits for the click
        let hover = suggest.with("hover");
        let over_list = ui.ctx().data(|d| d.get_temp::<bool>(hover)).unwrap_or(false);
        ui.ctx().data_mut(|d| d.insert_temp(hover, false));
        if r.lost_focus() && !(over_list && !enter && !esc) {
            if esc {
                text.clear();
            } else if !text.trim().is_empty() {
                let (found, unknown) = crate::mesh_clip::resolve_ids(model, mesh, &text);
                // a whole name or ID (names have spaces), else the top suggestion
                let exact = matches.iter().copied().find(|&mi| {
                    s.mesh_name(mi).to_lowercase() == needle
                        || model.art_meshes.get(mi).is_some_and(|m| m.id.to_lowercase() == needle)
                });
                if let Some(mi) = exact.filter(|_| found.is_empty() || !unknown.is_empty()) {
                    add(&mut want, mi);
                } else if found.is_empty() {
                    if let Some(&mi) = matches.first() {
                        add(&mut want, mi);
                    }
                } else {
                    for mi in found {
                        add(&mut want, mi as usize);
                    }
                    if !unknown.is_empty() {
                        log::warn!("Clip by: no other mesh has the ID {}", unknown.join(", "));
                    }
                }
                text.clear();
            }
            if enter {
                r.request_focus();
            } else {
                egui::Popup::close_id(ui.ctx(), suggest);
            }
        }
        if r.has_focus() {
            egui::Popup::open_id(ui.ctx(), suggest);
        }
        let mut picked = None;
        egui::Popup::from_response(&r)
            .id(suggest)
            .open_memory(None)
            .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
            .show(|ui| {
                ui.set_width(240.0);
                let over = ui.ui_contains_pointer();
                ui.ctx().data_mut(|d| d.insert_temp(hover, over));
                if matches.is_empty() {
                    ui.weak(crate::i18n::tr("No other mesh matches"));
                    return;
                }
                egui::ScrollArea::vertical().max_height(240.0).show(ui, |ui| {
                    let mut heading = None;
                    for &mi in matches.iter().take(60) {
                        let path = is_path(mi);
                        if heading != Some(path) {
                            heading = Some(path);
                            ui.weak(if path {
                                crate::i18n::tr("ArtPaths")
                            } else {
                                crate::i18n::tr("Meshes")
                            });
                        }
                        let name = s.mesh_name(mi);
                        let id = model.art_meshes.get(mi).map_or("", |m| m.id.as_str());
                        // a row like the other dropdowns': the name, its ID
                        // faint at the right when the two differ
                        let row = egui::Button::selectable(false, name.as_str())
                            .right_text(
                                egui::RichText::new(if name == id { "" } else { id })
                                    .color(theme::pal().text_faint)
                                    .size(11.0),
                            )
                            .min_size(vec2(ui.available_width(), 0.0));
                        if ui.add(row).clicked() {
                            picked = Some(mi);
                        }
                    }
                });
            });
        if let Some(mi) = picked {
            add(&mut want, mi);
            text.clear();
            r.request_focus();
        }
        if ui
            .selectable_label(picking, egui_phosphor::regular::CROSSHAIR)
            .on_hover_text(crate::i18n::tr(
                "Pick on canvas: click the mask mesh. Shift+click adds several, Esc ends.",
            ))
            .clicked()
        {
            toggle_pick = true;
        }
        if !want.is_empty()
            && ui
                .small_button(egui_phosphor::regular::COPY)
                .on_hover_text(crate::i18n::tr(
                    "Copy these masks' IDs, to paste into another mesh's Clip by",
                ))
                .clicked()
        {
            let ids: Vec<&str> = want
                .iter()
                .filter_map(|&x| model.art_meshes.get(x as usize))
                .map(|m| m.id.as_str())
                .collect();
            ui.ctx().copy_text(ids.join(", "));
        }
        *buf = (r.has_focus() || !text.is_empty()).then(|| (mesh, text.clone()));
    });
    // the masks as chips under the field (x takes one off)
    if !want.is_empty() {
        ui.horizontal_wrapped(|ui| {
            ui.add_space(78.0);
            let mut drop: Option<usize> = None;
            for (at, &x) in want.iter().enumerate() {
                let name = s.mesh_name(x as usize);
                let id = model.art_meshes.get(x as usize).map_or("", |m| m.id.as_str());
                let chip = egui::Button::new((
                    egui::RichText::new(truncate(&name, 22)).size(12.0),
                    egui::RichText::new(egui_phosphor::regular::X).size(12.5),
                ))
                .corner_radius(10.0)
                .min_size(vec2(0.0, 22.0));
                if ui
                    .add(chip)
                    .on_hover_text(format!(
                        "{id}\n{}",
                        crate::i18n::tr("Stop clipping by this mesh")
                    ))
                    .clicked()
                {
                    drop = Some(at);
                }
            }
            if let Some(at) = drop {
                want.remove(at);
            }
        });
    }
    ClipRow { masks: (want != masks).then_some(want), toggle_pick }
}

/// Below a mesh's Clip by: inheriting its masks' clip, the chain that
/// gives, and the clip group it shares its masks through.
/// Clip groups and inherit are experimental (Settings > Experimental):
/// offered when switched on there, and kept visible on a mesh that already
/// uses one so it can be undone.
fn clip_rows(
    ui: &mut Ui,
    s: &mut Session,
    mesh: usize,
    name_buf: &mut String,
    groups_on: bool,
    inherit_on: bool,
) {
    use ayatsuri_core::clip::{ChainStop, chain_of};
    let model = s.puppet.model();
    let Some(m) = model.art_meshes.get(mesh) else { return };
    let (masks, inherit, group) = (m.masks.clone(), m.clip_inherit, m.clip_group);
    let (chain, stop) = chain_of(model, mesh);
    let groups: Vec<String> = model.clip_groups.iter().map(|g| g.name.clone()).collect();
    let shared =
        group.map_or(0, |g| model.art_meshes.iter().filter(|x| x.clip_group == Some(g)).count());
    let names =
        |set: &[u32]| set.iter().map(|&x| s.mesh_name(x as usize)).collect::<Vec<_>>().join(", ");

    let mut want_inherit = inherit;
    if !masks.is_empty() && (inherit_on || inherit) {
        ui.horizontal(|ui| {
            ui.add_space(78.0);
            let label = if masks.len() == 1 {
                crate::i18n::trf("Inherit {mask}'s clip", &[("mask", &names(&masks))])
            } else {
                crate::i18n::tr("Inherit the masks' clip").into()
            };
            ui.checkbox(&mut want_inherit, label).on_hover_text(crate::i18n::tr(
                "Also clip by whatever the masks are clipped by, so a highlight clipped by \
                 the iris stays inside the eye white. moc3 export cannot store it yet: it \
                 exports clipped by its own masks only, and says so.",
            ));
            ui.label(
                egui::RichText::new(crate::i18n::tr("experimental"))
                    .size(10.0)
                    .color(theme::pal().accent),
            );
        });
        if inherit && chain.len() > 1 {
            let mut line = format!("→ {}", s.mesh_name(mesh));
            for set in &chain {
                line.push_str(" ⊂ ");
                line.push_str(&names(set));
            }
            ui.horizontal_wrapped(|ui| {
                ui.add_space(78.0);
                ui.weak(egui::RichText::new(line).size(10.5));
            });
        }
        if inherit {
            let why = match stop {
                Some(ChainStop::MasksDisagree) => Some(
                    "The masks are clipped differently, so there is no one clip to inherit past here",
                ),
                Some(ChainStop::Loop) => {
                    Some("The masks clip each other in a loop; the chain stops there")
                }
                Some(ChainStop::TooDeep) => Some("Inherited clips stop after eight levels"),
                None if chain.len() < 2 => Some("The masks are not clipped by anything yet"),
                None => None,
            };
            if let Some(why) = why {
                ui.horizontal_wrapped(|ui| {
                    ui.add_space(78.0);
                    ui.weak(egui::RichText::new(crate::i18n::tr(why)).size(10.5));
                });
            }
        }
    }

    let mut want_group = group;
    let mut save_as: Option<String> = None;
    let mut delete_group = false;
    if groups_on || group.is_some() {
        ui.horizontal(|ui| {
            ui.add_sized(
                [74.0, 16.0],
                egui::Label::new(
                    egui::RichText::new(crate::i18n::tr("Clip group"))
                        .color(theme::pal().text_faint)
                        .size(11.0),
                ),
            );
            let current = want_group
                .and_then(|g| groups.get(g as usize).cloned())
                .unwrap_or_else(|| crate::i18n::tr("none").into());
            egui::ComboBox::from_id_salt(("clip-group", mesh)).selected_text(current).show_ui(
                ui,
                |ui| {
                    ui.selectable_value(&mut want_group, None, crate::i18n::tr("none"));
                    for (g, name) in groups.iter().enumerate() {
                        ui.selectable_value(&mut want_group, Some(g as u32), name);
                    }
                },
            );
            if want_group.is_some()
                && ui
                    .small_button(egui_phosphor::regular::TRASH)
                    .on_hover_text(crate::i18n::tr(
                        "Delete this clip group; its meshes keep the masks they have",
                    ))
                    .clicked()
            {
                delete_group = true;
            }
        });
    }
    if group.is_some() && shared > 1 {
        ui.horizontal_wrapped(|ui| {
            ui.add_space(78.0);
            ui.weak(
                egui::RichText::new(crate::i18n::trf(
                    "Changing these masks changes them on all {count} meshes in the group",
                    &[("count", &shared)],
                ))
                .size(10.5),
            );
        });
    }
    if groups_on && group.is_none() && !masks.is_empty() {
        ui.horizontal(|ui| {
            ui.add_space(78.0);
            search_menu(ui, crate::i18n::tr("Save as group…"), |ui| {
                ui.add(
                    egui::TextEdit::singleline(name_buf)
                        .hint_text(crate::i18n::tr("Group name, e.g. Eye L"))
                        .desired_width(160.0),
                );
                let name = name_buf.trim().to_string();
                if ui
                    .add_enabled(!name.is_empty(), egui::Button::new(crate::i18n::tr("Save")))
                    .clicked()
                {
                    save_as = Some(name);
                    ui.close();
                }
            })
            .on_hover_text(crate::i18n::tr(
                "Name these masks so other meshes can clip by them too, and change with them",
            ));
        });
    }

    if want_inherit != inherit
        && s.edit(|m| ayatsuri_core::edit::set_clip_inherit(m, mesh, want_inherit))
    {
        log::info!("{} inherits its masks' clip: {want_inherit}", s.mesh_name(mesh));
    }
    if delete_group {
        if let Some(g) = group
            && s.edit(|m| ayatsuri_core::edit::delete_clip_group(m, g as usize))
        {
            log::info!("clip group {g} deleted");
        }
    } else if want_group != group
        && s.edit(|m| ayatsuri_core::edit::set_clip_group(m, mesh, want_group.map(|g| g as usize)))
    {
        log::info!("{} clip group: {want_group:?}", s.mesh_name(mesh));
    }
    if let Some(name) = save_as
        && s.edit(|m| ayatsuri_core::edit::create_clip_group(m, mesh, name.clone()).is_some())
    {
        log::info!("clip group {name} saved from {}", s.mesh_name(mesh));
        name_buf.clear();
    }
}

/// The Inspector's Blend mode and Alpha blend rows, for a mesh or an
/// offscreen part's layer. The two modes from before 5.3 ignore the alpha
/// blend, so it greys out and stays Over with them.
fn blend_rows(ui: &mut Ui, salt: (&str, usize), blend: &mut ayatsuri_format::model::Blend) {
    use ayatsuri_format::model::{AlphaBlend, ColorBlend};
    let label = |ui: &mut egui::Ui, text: &'static str| {
        ui.add_sized(
            [74.0, 16.0],
            egui::Label::new(
                egui::RichText::new(crate::i18n::tr(text))
                    .color(theme::pal().text_faint)
                    .size(11.0),
            ),
        );
    };
    ui.horizontal(|ui| {
        label(ui, "Blend mode");
        egui::ComboBox::from_id_salt(("blend", salt))
            .selected_text(crate::i18n::trc(color_blend_name(blend.color)))
            .show_ui(ui, |ui| {
                for c in COLOR_BLEND_MENU {
                    ui.selectable_value(&mut blend.color, c, crate::i18n::trc(color_blend_name(c)));
                }
            });
    });
    let compatible =
        matches!(blend.color, ColorBlend::AddCompatible | ColorBlend::MultiplyCompatible);
    ui.horizontal(|ui| {
        label(ui, "Alpha blend");
        ui.add_enabled_ui(!compatible, |ui| {
            egui::ComboBox::from_id_salt(("alpha-blend", salt))
                .selected_text(crate::i18n::trc(alpha_blend_name(blend.alpha)))
                .show_ui(ui, |ui| {
                    for a in AlphaBlend::ALL {
                        ui.selectable_value(
                            &mut blend.alpha,
                            a,
                            crate::i18n::trc(alpha_blend_name(a)),
                        );
                    }
                });
        });
    });
    if compatible {
        blend.alpha = AlphaBlend::Over;
    }
}

fn color_blend_name(c: ayatsuri_format::model::ColorBlend) -> &'static str {
    use ayatsuri_format::model::ColorBlend::*;
    match c {
        Normal => "blend|Normal",
        Add => "blend|Add",
        AddGlow => "blend|Add (Glow)",
        Darken => "blend|Darken",
        Multiply => "blend|Multiply",
        ColorBurn => "blend|Color Burn",
        LinearBurn => "blend|Linear Burn",
        Lighten => "blend|Lighten",
        Screen => "blend|Screen",
        ColorDodge => "blend|Color Dodge",
        Overlay => "blend|Overlay",
        SoftLight => "blend|Soft Light",
        HardLight => "blend|Hard Light",
        LinearLight => "blend|Linear Light",
        Hue => "blend|Hue",
        Color => "blend|Color",
        AddCompatible => "blend|Add (Before 5.3)",
        MultiplyCompatible => "blend|Multiply (Before 5.3)",
    }
}

fn alpha_blend_name(a: ayatsuri_format::model::AlphaBlend) -> &'static str {
    use ayatsuri_format::model::AlphaBlend::*;
    match a {
        Over => "blend|Over",
        Atop => "blend|Atop",
        Out => "blend|Out",
        Conjoint => "blend|Conjoint",
        Disjoint => "blend|Disjoint",
    }
}

/// A selected palette row: the selection fill and an accent bar down its
/// left edge, so what is selected (one row or many) reads at a glance.
fn paint_picked_row(ui: &Ui, rect: Rect) {
    let pal = theme::pal();
    ui.painter().rect_filled(rect, 0.0, pal.select);
    let bar = Rect::from_min_size(rect.min, vec2(3.0, rect.height()));
    ui.painter().rect_filled(bar, 0.0, pal.accent);
}

#[cfg(test)]
mod atlas_leak_tests {
    use super::*;
    use crate::session::Session;
    use ayatsuri_import::{Document, Layer};

    fn layer(name: &str, x: i32, rgb: [u8; 3]) -> Layer {
        Layer {
            name: name.into(),
            groups: vec![],
            x,
            y: 10,
            width: 30,
            height: 30,
            rgba: [rgb[0], rgb[1], rgb[2], 255].repeat(30 * 30),
            visible: true,
            opacity: 1.0,
            clip_to_below: false,
            blend: ayatsuri_import::Blend::NORMAL,
        }
    }

    #[test]
    fn a_mesh_grown_over_its_neighbour_is_repacked_with_only_its_own_art() {
        let ctx = egui::Context::default();
        let doc = Document {
            width: 200,
            height: 100,
            layers: vec![layer("Red", 10, [255, 0, 0]), layer("Blue", 60, [0, 0, 255])],
            folders: Vec::new(),
        };
        let art = ayatsuri_import::from_layers(&doc).unwrap();
        let mut s = Session::create(&ctx, "t".into(), String::new(), Model::new()).unwrap();
        s.add_imported(&ctx, art, std::path::Path::new("a.psd")).unwrap();
        // the layers come in on pages of their own; the atlas is made first
        assert!(s.atlas_pending());
        let sizes = s.page_sizes();
        let pieces = crate::atlas::pieces(s.puppet.model(), &sizes, &s.art_boxes());
        let layout = crate::atlas::auto_layout(&pieces, 256, false).unwrap();
        s.apply_atlas(&ctx, &pieces, &layout).unwrap();
        assert!(!s.atlas_pending());
        let red = (0..2).find(|&i| s.mesh_name(i) == "Red").unwrap();
        let blue = 1 - red;
        // the red mesh's right edge moves out over the blue layer's art
        let blue_u1 =
            s.puppet.model().art_meshes[blue].uvs.chunks(2).map(|p| p[0]).fold(0.0f32, f32::max);
        assert!(s.edit(|m| {
            let r = &mut m.art_meshes[red];
            let right = r.uvs.chunks(2).map(|p| p[0]).fold(0.0f32, f32::max);
            for uv in r.uvs.chunks_mut(2) {
                if (uv[0] - right).abs() < 1e-6 {
                    uv[0] = blue_u1;
                }
            }
            true
        }));
        let mut ed = EditorUi::new();
        ed.session = Some(s);
        // the export sees the outgrown mesh, and nothing after the layout
        assert_eq!(ed.outgrown_meshes(), vec![red]);
        ed.repack_if_outgrown(&ctx);
        assert!(ed.outgrown_meshes().is_empty());
        let s = ed.session.as_ref().unwrap();
        let m = &s.puppet.model().art_meshes[red];
        let page = image::load_from_memory(&s.texture_pngs[m.texture as usize]).unwrap().to_rgba8();
        let (w, h) = (page.width() as f32, page.height() as f32);
        let (mut u0, mut v0, mut u1, mut v1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
        for uv in m.uvs.chunks(2) {
            (u0, v0, u1, v1) = (u0.min(uv[0]), v0.min(uv[1]), u1.max(uv[0]), v1.max(uv[1]));
        }
        let (mut reds, mut blues) = (0, 0);
        for y in (v0 * h) as u32..(v1 * h) as u32 {
            for x in (u0 * w) as u32..(u1 * w) as u32 {
                let p = page.get_pixel(x, y);
                if p[3] > 0 && p[2] > 128 {
                    blues += 1;
                }
                if p[3] > 0 && p[0] > 128 {
                    reds += 1;
                }
            }
        }
        assert!(reds > 0, "its own art came along");
        assert_eq!(blues, 0, "the neighbour's did not");
    }
}
