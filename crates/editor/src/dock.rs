//! Docking: a tree of splits and tab groups, the way FilmCraft does its
//! Premiere-style workspaces. Gutters drag to resize. A tab drags out of its
//! group and drops onto another: the centre joins as a tab, an edge splits.
//! The tree serializes, so workspaces can be saved once persistence lands.

use egui::{
    Align2, Color32, CursorIcon, Id, Pos2, Rect, Sense, Stroke, StrokeKind, Ui, pos2, vec2,
};
use serde::{Deserialize, Serialize};

/// Every panel the editor can show.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PanelKind {
    Parts,
    Deformers,
    Parameters,
    Inspector,
    ToolDetails,
    Viewport,
    Timeline,
    Textures,
    Log,
}

impl PanelKind {
    pub const ALL: [PanelKind; 9] = [
        PanelKind::Parts,
        PanelKind::Deformers,
        PanelKind::Parameters,
        PanelKind::Inspector,
        PanelKind::ToolDetails,
        PanelKind::Viewport,
        PanelKind::Timeline,
        PanelKind::Textures,
        PanelKind::Log,
    ];

    /// Palette names as Cubism Editor spells them.
    pub fn title(self) -> &'static str {
        match self {
            PanelKind::Parts => "Part",
            PanelKind::Deformers => "Deformer",
            PanelKind::Parameters => "Parameter",
            PanelKind::Inspector => "Properties",
            PanelKind::ToolDetails => "Tool Details",
            PanelKind::Viewport => "View",
            PanelKind::Timeline => "Timeline",
            PanelKind::Textures => "Texture",
            PanelKind::Log => "Log",
        }
    }
}

/// How a split divides its space.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum SplitSize {
    /// First child takes this fraction of the space.
    Ratio(f32),
    /// First child is this many points.
    FixedA(f32),
    /// Second child is this many points.
    FixedB(f32),
}

/// The layout tree. Leaves are tab groups; interior nodes are splits.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum DockNode {
    Split { vertical: bool, size: SplitSize, a: Box<DockNode>, b: Box<DockNode> },
    Tabs { panels: Vec<PanelKind>, active: usize },
}

fn tabs(panels: &[PanelKind], active: usize) -> DockNode {
    DockNode::Tabs { panels: panels.to_vec(), active }
}

fn hsplit(size: SplitSize, a: DockNode, b: DockNode) -> DockNode {
    DockNode::Split { vertical: false, size, a: Box::new(a), b: Box::new(b) }
}

fn vsplit(size: SplitSize, a: DockNode, b: DockNode) -> DockNode {
    DockNode::Split { vertical: true, size, a: Box::new(a), b: Box::new(b) }
}

/// The Modeling workspace, laid out the way Cubism Editor's is: Part and
/// Deformer palettes left, Inspector and Parameter in the next column, the
/// View filling the rest.
pub fn default_workspace() -> DockNode {
    use PanelKind::*;
    use SplitSize::*;
    hsplit(
        FixedA(250.0),
        vsplit(Ratio(0.5), tabs(&[Parts], 0), tabs(&[Deformers, Log], 0)),
        hsplit(
            FixedA(320.0),
            vsplit(
                FixedA(240.0),
                tabs(&[ToolDetails, Inspector, Textures], 0),
                tabs(&[Parameters], 0),
            ),
            tabs(&[Viewport], 0),
        ),
    )
}

/// The Animation workspace: the View on top, the Timeline across the bottom.
pub fn animation_workspace() -> DockNode {
    use PanelKind::*;
    use SplitSize::*;
    vsplit(
        FixedB(260.0),
        hsplit(
            FixedA(250.0),
            vsplit(Ratio(0.5), tabs(&[Parts], 0), tabs(&[Deformers, Log], 0)),
            tabs(&[Viewport], 0),
        ),
        tabs(&[Timeline], 0),
    )
}

impl DockNode {
    pub fn panels(&self, out: &mut Vec<PanelKind>) {
        match self {
            DockNode::Split { a, b, .. } => {
                a.panels(out);
                b.panels(out);
            }
            DockNode::Tabs { panels, .. } => out.extend(panels.iter().copied()),
        }
    }

    pub fn contains(&self, p: PanelKind) -> bool {
        let mut v = Vec::new();
        self.panels(&mut v);
        v.contains(&p)
    }

    /// Make `p` the active tab of its group. Returns false if not present.
    pub fn activate(&mut self, p: PanelKind) -> bool {
        match self {
            DockNode::Split { a, b, .. } => a.activate(p) || b.activate(p),
            DockNode::Tabs { panels, active } => {
                if let Some(i) = panels.iter().position(|x| *x == p) {
                    *active = i;
                    true
                } else {
                    false
                }
            }
        }
    }

    /// Remove a panel's tab; a group left empty collapses its split.
    pub fn close(&mut self, p: PanelKind) {
        if let DockNode::Split { a, b, .. } = self {
            a.close(p);
            b.close(p);
            let empty =
                |n: &DockNode| matches!(n, DockNode::Tabs { panels, .. } if panels.is_empty());
            if empty(a) {
                *self = (**b).clone();
            } else if empty(b) {
                *self = (**a).clone();
            }
        } else if let DockNode::Tabs { panels, active } = self {
            panels.retain(|x| *x != p);
            *active = (*active).min(panels.len().saturating_sub(1));
        }
    }

    /// Add a panel as a tab next to `near`, or into the first group.
    pub fn open_near(&mut self, p: PanelKind, near: PanelKind) {
        if self.contains(p) {
            self.activate(p);
            return;
        }
        fn add(n: &mut DockNode, p: PanelKind, near: PanelKind) -> bool {
            match n {
                DockNode::Split { a, b, .. } => add(a, p, near) || add(b, p, near),
                DockNode::Tabs { panels, active } => {
                    if panels.contains(&near) {
                        panels.push(p);
                        *active = panels.len().saturating_sub(1);
                        true
                    } else {
                        false
                    }
                }
            }
        }
        fn first(n: &mut DockNode, p: PanelKind) -> bool {
            match n {
                DockNode::Split { a, .. } => first(a, p),
                DockNode::Tabs { panels, active } => {
                    panels.push(p);
                    *active = panels.len().saturating_sub(1);
                    true
                }
            }
        }
        if !add(self, p, near) {
            first(self, p);
        }
    }

    /// The tab group containing `anchor`, mutably.
    fn group_of(&mut self, anchor: PanelKind) -> Option<&mut DockNode> {
        match self {
            DockNode::Split { a, b, .. } => a.group_of(anchor).or_else(|| b.group_of(anchor)),
            DockNode::Tabs { panels, .. } => {
                if panels.contains(&anchor) {
                    Some(self)
                } else {
                    None
                }
            }
        }
    }
}

/// Where on a group a dragged tab is about to land.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DropZone {
    Center,
    Left,
    Right,
    Top,
    Bottom,
}

/// Re-dock `p`: out of wherever it is, onto the group holding `anchor`.
/// Centre joins the group as a tab; an edge splits the group in two.
pub fn apply_move(root: &mut DockNode, p: PanelKind, anchor: PanelKind, zone: DropZone) {
    if p == anchor {
        return;
    }
    root.close(p);
    let Some(group) = root.group_of(anchor) else {
        root.open_near(p, anchor);
        return;
    };
    match zone {
        DropZone::Center => {
            if let DockNode::Tabs { panels, active } = group {
                panels.push(p);
                *active = panels.len().saturating_sub(1);
            }
        }
        zone => {
            let old = group.clone();
            let new = tabs(&[p], 0);
            *group = match zone {
                DropZone::Left => hsplit(SplitSize::Ratio(0.5), new, old),
                DropZone::Right => hsplit(SplitSize::Ratio(0.5), old, new),
                DropZone::Top => vsplit(SplitSize::Ratio(0.5), new, old),
                DropZone::Bottom | DropZone::Center => vsplit(SplitSize::Ratio(0.5), old, new),
            };
        }
    }
}

/// One laid-out tab group.
pub struct Group {
    pub path: String,
    pub rect: Rect,
    pub content: Rect,
    pub panels: Vec<PanelKind>,
    pub active: usize,
}

/// Dock chrome metrics; colours come from the live palette.
pub mod theme {
    pub const GAP: f32 = 4.0;
    pub const TAB_H: f32 = 22.0;
    pub const RADIUS: f32 = 3.0;
}

/// A split's first-child size in `avail` points, never panicking and never
/// starving either side below 40 points when there is room.
fn split_first(size: SplitSize, avail: f32) -> f32 {
    let want = match size {
        SplitSize::Ratio(r) => avail * r,
        SplitSize::FixedA(px) => px,
        SplitSize::FixedB(px) => avail - px,
    };
    let lo = 40.0_f32.min(avail * 0.5);
    let hi = (avail - 40.0).max(lo);
    want.clamp(lo, hi)
}

/// Lay the tree out into group rects, drawing and handling the resize gutters.
pub fn layout(ui: &mut Ui, node: &mut DockNode, rect: Rect, path: &str, out: &mut Vec<Group>) {
    match node {
        DockNode::Tabs { panels, active } => {
            let content = Rect::from_min_max(pos2(rect.min.x, rect.min.y + theme::TAB_H), rect.max);
            out.push(Group {
                path: path.to_string(),
                rect,
                content,
                panels: panels.clone(),
                active: *active,
            });
        }
        DockNode::Split { vertical, size, a, b } => {
            let g = theme::GAP;
            let total = if *vertical { rect.height() } else { rect.width() };
            let avail = (total - g).max(0.0);
            let first = split_first(*size, avail);
            let (ra, gutter, rb) = if *vertical {
                (
                    Rect::from_min_max(rect.min, pos2(rect.max.x, rect.min.y + first)),
                    Rect::from_min_max(
                        pos2(rect.min.x, rect.min.y + first),
                        pos2(rect.max.x, rect.min.y + first + g),
                    ),
                    Rect::from_min_max(pos2(rect.min.x, rect.min.y + first + g), rect.max),
                )
            } else {
                (
                    Rect::from_min_max(rect.min, pos2(rect.min.x + first, rect.max.y)),
                    Rect::from_min_max(
                        pos2(rect.min.x + first, rect.min.y),
                        pos2(rect.min.x + first + g, rect.max.y),
                    ),
                    Rect::from_min_max(pos2(rect.min.x + first + g, rect.min.y), rect.max),
                )
            };
            let hit = gutter.expand2(if *vertical { vec2(0.0, 3.0) } else { vec2(3.0, 0.0) });
            let id = Id::new(("dock-gutter", path.to_string()));
            let resp = ui.interact(hit, id, Sense::drag());
            if resp.hovered() || resp.dragged() {
                ui.ctx().set_cursor_icon(if *vertical {
                    CursorIcon::ResizeVertical
                } else {
                    CursorIcon::ResizeHorizontal
                });
            }
            if resp.dragged() {
                let d = if *vertical { resp.drag_delta().y } else { resp.drag_delta().x };
                let nf = (first + d).clamp(40.0, (avail - 40.0).max(40.0));
                *size = match *size {
                    SplitSize::Ratio(_) => SplitSize::Ratio(nf / avail.max(1.0)),
                    SplitSize::FixedA(_) => SplitSize::FixedA(nf),
                    SplitSize::FixedB(_) => SplitSize::FixedB(avail - nf),
                };
            }
            if resp.dragged() || resp.hovered() {
                ui.painter().rect_filled(
                    gutter,
                    0.0,
                    crate::theme::pal().accent.gamma_multiply(if resp.dragged() {
                        0.9
                    } else {
                        0.4
                    }),
                );
            }
            layout(ui, a, ra, &format!("{path}a"), out);
            layout(ui, b, rb, &format!("{path}b"), out);
        }
    }
}

/// What the chrome asks the editor to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DockAction {
    Activate(PanelKind),
    Close(PanelKind),
}

/// A tab mid-drag: the panel and where it came from.
#[derive(Clone, Copy, Debug, PartialEq)]
struct DragTab {
    panel: PanelKind,
}

fn drag_id() -> Id {
    Id::new("dock-drag-tab")
}

pub fn dragged_tab(ui: &Ui) -> Option<PanelKind> {
    ui.ctx().data(|d| d.get_temp::<DragTab>(drag_id())).map(|t| t.panel)
}

/// Draw one group's frame and tab strip. Starts tab drags; returns clicks.
pub fn draw_group_chrome(ui: &mut Ui, g: &Group) -> Vec<DockAction> {
    let mut actions = Vec::new();
    let painter = ui.painter().clone();
    painter.rect_filled(g.rect, theme::RADIUS, crate::theme::pal().panel_bg);

    let strip = Rect::from_min_size(g.rect.min, vec2(g.rect.width(), theme::TAB_H));
    let mut x = strip.min.x + 10.0;
    let text_y = strip.min.y + theme::TAB_H * 0.55;
    for (i, p) in g.panels.iter().enumerate() {
        let is_active = i == g.active;
        let color =
            if is_active { crate::theme::pal().text } else { crate::theme::pal().text_faint };
        let galley = painter.layout_no_wrap(
            crate::i18n::tr(p.title()).to_string(),
            egui::FontId::proportional(12.0),
            color,
        );
        let w = galley.size().x + 16.0;
        let tab_rect = Rect::from_min_size(pos2(x, strip.min.y), vec2(w, theme::TAB_H));
        let resp = ui.interact(
            tab_rect,
            Id::new(("dock-tab", g.path.clone(), *p)),
            Sense::click_and_drag(),
        );
        let color = if is_active || resp.hovered() {
            crate::theme::pal().text
        } else {
            crate::theme::pal().text_faint
        };
        painter.galley_with_override_text_color(
            pos2(tab_rect.min.x + 8.0, text_y - galley.size().y / 2.0),
            galley,
            color,
        );
        if is_active {
            let uy = strip.min.y + theme::TAB_H - 4.0;
            painter.line_segment(
                [pos2(tab_rect.min.x + 8.0, uy), pos2(tab_rect.max.x - 8.0, uy)],
                Stroke::new(1.0, crate::theme::pal().text),
            );
        }
        if resp.clicked() || resp.drag_started() {
            actions.push(DockAction::Activate(*p));
        }
        if resp.drag_started() {
            ui.ctx().data_mut(|d| d.insert_temp(drag_id(), DragTab { panel: *p }));
        }
        if resp.middle_clicked() {
            actions.push(DockAction::Close(*p));
        }
        x += w + 4.0;
    }
    actions
}

/// The drop zone under `pos` for a group, by which fifth of the content rect
/// the pointer is in.
fn zone_at(content: Rect, pos: Pos2) -> DropZone {
    let fx = ((pos.x - content.min.x) / content.width().max(1.0)).clamp(0.0, 1.0);
    let fy = ((pos.y - content.min.y) / content.height().max(1.0)).clamp(0.0, 1.0);
    if fx < 0.25 {
        DropZone::Left
    } else if fx > 0.75 {
        DropZone::Right
    } else if fy < 0.25 {
        DropZone::Top
    } else if fy > 0.75 {
        DropZone::Bottom
    } else {
        DropZone::Center
    }
}

fn zone_rect(content: Rect, zone: DropZone) -> Rect {
    let half = |r: Rect, z: DropZone| match z {
        DropZone::Left => Rect::from_min_max(r.min, pos2(r.center().x, r.max.y)),
        DropZone::Right => Rect::from_min_max(pos2(r.center().x, r.min.y), r.max),
        DropZone::Top => Rect::from_min_max(r.min, pos2(r.max.x, r.center().y)),
        DropZone::Bottom => Rect::from_min_max(pos2(r.min.x, r.center().y), r.max),
        DropZone::Center => r.shrink(r.width().min(r.height()) * 0.18),
    };
    half(content, zone)
}

/// While a tab is being dragged: ghost label at the pointer, drop preview on
/// the hovered group, and the actual move on release. Call after every group
/// has drawn, so the overlay paints on top.
pub fn drag_overlay(ui: &mut Ui, root: &mut DockNode, groups: &[Group]) {
    let Some(p) = dragged_tab(ui) else { return };
    let pointer = ui.input(|i| i.pointer.latest_pos());
    let released = ui.input(|i| i.pointer.any_released());
    let painter = ui.painter().clone();

    if let Some(pos) = pointer {
        painter.text(
            pos + vec2(14.0, 2.0),
            Align2::LEFT_CENTER,
            crate::i18n::tr(p.title()),
            egui::FontId::proportional(12.0),
            crate::theme::pal().text,
        );
        let target = groups.iter().find(|g| g.rect.contains(pos));
        if let Some(g) = target {
            let zone = zone_at(g.content, pos);
            let r = zone_rect(g.content, zone);
            painter.rect_filled(r, theme::RADIUS, crate::theme::pal().accent.gamma_multiply(0.25));
            painter.rect_stroke(
                r,
                theme::RADIUS,
                Stroke::new(1.0, crate::theme::pal().accent),
                StrokeKind::Inside,
            );
            if released {
                let anchor = g.panels.iter().copied().find(|x| *x != p);
                if let Some(anchor) = anchor {
                    apply_move(root, p, anchor, zone);
                }
            }
        }
    }
    if released {
        ui.ctx().data_mut(|d| d.remove::<DragTab>(drag_id()));
    }
    ui.ctx().request_repaint();
}

/// Focus outline for a group (the viewport while a model is loaded, later the
/// focused panel the way FilmCraft outlines it).
pub fn focus_outline(ui: &Ui, rect: Rect) {
    ui.painter().rect_stroke(
        rect,
        theme::RADIUS,
        Stroke::new(1.0, Color32::from_rgb(0x4c, 0x8b, 0xf5)),
        StrokeKind::Inside,
    );
}
