// Stroke mesh-mapping in Mesh Edit (experimental), after the Cubism manual:
// drag to draw a stroke and the mesh is built along it; click the line to
// add a control point, Alt+click one to delete it, drag one to move it,
// Shift+drag to draw another stroke, drag away from the path to redraw it.
// Width handles (orange circles): Ctrl+click the line adds one, Ctrl+Alt+
// click deletes one, Ctrl+drag sets the width. Shift+E or the button at the
// canvas's top left confirms; Delete cancels. A new line that would redraw
// an unconfirmed stroke asks first, so a slip does not throw the stroke
// away.
//
// Strokes are kept in canvas pixels; the canvas borrows the session while
// it draws, so this hands back what to do and the palette applies it.

use ayatsuri_core::stroke_mesh::{self, P, Settings, Stroke};
use egui::{Align2, Color32, FontId, Painter, Pos2, Rect, Response, Sense, Stroke as Line, Ui};
use egui::{pos2, vec2};

use super::EditorUi;
use crate::i18n::tr;
use crate::theme;

/// The Mesh-mapping settings in Tool details.
#[derive(Default)]
pub(super) struct StrokeOpts {
    pub(super) settings: Settings,
    /// Keep the existing mesh under the stroke instead of replacing it.
    pub(super) maintain: bool,
}

/// The strokes being drawn on one mesh, not yet confirmed.
pub(super) struct StrokeEdit {
    pub(super) mesh: usize,
    pub(super) strokes: Vec<Stroke>,
    drag: Option<Drag>,
    /// A new line that would redraw stroke `.0`, waiting for the rigger to
    /// say whether to replace it.
    ask: Option<(usize, Stroke)>,
}

enum Drag {
    /// A stroke being drawn: its samples, and the stroke it redraws.
    Draw {
        samples: Vec<P>,
        replaces: Option<usize>,
    },
    Point {
        stroke: usize,
        point: usize,
    },
    Width {
        stroke: usize,
        handle: usize,
    },
}

/// What the canvas asks for.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum StrokeAct {
    Confirm,
}

/// Canvas pixels to the screen and back: screen = origin + px * scale.
#[derive(Clone, Copy)]
pub(super) struct View {
    pub(super) origin: Pos2,
    pub(super) scale: f32,
    /// The canvas area, for the confirm button.
    pub(super) rect: Rect,
}

impl View {
    fn screen(self, p: P) -> Pos2 {
        pos2(self.origin.x + p[0] * self.scale, self.origin.y + p[1] * self.scale)
    }
    fn px(self, p: Pos2) -> P {
        let k = self.scale.max(1e-6);
        [(p.x - self.origin.x) / k, (p.y - self.origin.y) / k]
    }
}

/// Screen points within this of a control point or line pick it.
const REACH: f32 = 7.0;

fn dist(a: P, b: P) -> f32 {
    (a[0] - b[0]).hypot(a[1] - b[1])
}

/// One frame of the tool: input on `resp`, the strokes and their mesh drawn
/// with `vp`. `mods` are (alt, ctrl, shift).
pub(super) fn stroke_tool(
    ui: &Ui,
    vp: &Painter,
    resp: &Response,
    state: &mut Option<StrokeEdit>,
    mesh: usize,
    opts: &StrokeOpts,
    view: View,
    mods: (bool, bool, bool),
    // the mesh as shown now: screen points and faces, for the preview of
    // what Maintain current mesh off removes
    existing: (&[Pos2], &[u16]),
) -> Option<StrokeAct> {
    let (alt, ctrl, shift) = mods;
    if state.as_ref().is_none_or(|e| e.mesh != mesh) {
        *state = Some(StrokeEdit { mesh, strokes: Vec::new(), drag: None, ask: None });
    }
    let edit = state.as_mut()?;
    let set = &opts.settings;
    let ptr = ui.ctx().input(|i| i.pointer.latest_pos());

    // a drag only starts once the pointer has gone past the click distance,
    // so what it takes hold of is what was under the press
    let press = ui.ctx().input(|i| i.pointer.press_origin());
    let probe = if resp.drag_started() { press.or(ptr) } else { ptr };
    // what is under the pointer: a control point, a width handle, the line
    let mut on_point: Option<(usize, usize)> = None;
    let mut on_handle: Option<(usize, usize)> = None;
    let mut on_line: Option<(usize, f32)> = None;
    if let Some(p) = probe {
        let px = view.px(p);
        let mut best = REACH;
        for (si, s) in edit.strokes.iter().enumerate() {
            for (pi, &c) in s.points.iter().enumerate() {
                let d = view.screen(c).distance(p);
                if d < best {
                    best = d;
                    on_point = Some((si, pi));
                }
            }
        }
        let mut best = REACH;
        for (si, s) in edit.strokes.iter().enumerate() {
            for (hi, &(t, k)) in s.widths.iter().enumerate() {
                let Some((at, _)) = stroke_mesh::point_on(s, t) else { continue };
                let c = view.screen(at);
                let rim = (c.distance(p) - set.width * k * view.scale).abs();
                let d = rim.min(c.distance(p));
                if d < best {
                    best = d;
                    on_handle = Some((si, hi));
                }
            }
        }
        let mut best = REACH / view.scale.max(1e-6);
        for (si, s) in edit.strokes.iter().enumerate() {
            if let Some((t, d)) = stroke_mesh::nearest_on(s, px)
                && d < best
            {
                best = d;
                on_line = Some((si, t));
            }
        }
    }

    // the replace question takes the canvas until it is answered
    if let Some((i, new)) = edit.ask.take() {
        let mut answer: Option<u8> = None;
        let at = view.rect.center() - vec2(150.0, 40.0);
        egui::Area::new(egui::Id::new("stroke-replace"))
            .order(egui::Order::Foreground)
            .fixed_pos(at)
            .show(ui.ctx(), |ui| {
                egui::Frame::popup(ui.style()).show(ui, |ui| {
                    ui.set_width(300.0);
                    ui.label(tr(
                        "The new line would replace the stroke you have not confirmed yet.",
                    ));
                    ui.horizontal(|ui| {
                        if ui.button(tr("Replace")).clicked() {
                            answer = Some(0);
                        }
                        if ui.button(tr("Keep both")).clicked() {
                            answer = Some(1);
                        }
                        if ui.button(tr("Discard the new line")).clicked() {
                            answer = Some(2);
                        }
                    });
                });
            });
        let esc = ui.ctx().input(|i| i.key_pressed(egui::Key::Escape));
        match (answer, esc) {
            (Some(0), _) => {
                if let Some(old) = edit.strokes.get_mut(i) {
                    *old = new;
                }
                log::debug!("stroke mesh: stroke {i} redrawn");
            }
            (Some(1), _) => edit.strokes.push(new),
            (Some(_), _) | (None, true) => log::debug!("stroke mesh: new line discarded"),
            (None, false) => edit.ask = Some((i, new)),
        }
        // nothing else on the canvas this frame
        edit.drag = None;
        draw(vp, edit, set, view, existing, opts.maintain);
        return None;
    }

    // drags: a handle with Ctrl, a control point, else a new stroke
    if resp.drag_started()
        && let Some(p) = probe
    {
        let px = view.px(p);
        edit.drag = if ctrl {
            on_handle.map(|(stroke, handle)| Drag::Width { stroke, handle })
        } else if let Some((stroke, point)) = on_point.filter(|_| !alt) {
            Some(Drag::Point { stroke, point })
        } else if !alt {
            // away from the path redraws the nearest stroke; Shift adds one
            let replaces = (!shift && !edit.strokes.is_empty())
                .then(|| {
                    edit.strokes
                        .iter()
                        .enumerate()
                        .filter_map(|(i, s)| stroke_mesh::nearest_on(s, px).map(|(_, d)| (i, d)))
                        .min_by(|a, b| a.1.total_cmp(&b.1))
                        .map(|(i, _)| i)
                })
                .flatten();
            Some(Drag::Draw { samples: vec![px], replaces })
        } else {
            None
        };
    }
    if resp.dragged()
        && let Some(p) = resp.interact_pointer_pos()
    {
        let px = view.px(p);
        match &mut edit.drag {
            Some(Drag::Draw { samples, .. }) => {
                if samples.last().is_none_or(|&l| dist(l, px) * view.scale > 2.0) {
                    samples.push(px);
                }
            }
            Some(Drag::Point { stroke, point }) => {
                if let Some(c) =
                    edit.strokes.get_mut(*stroke).and_then(|s| s.points.get_mut(*point))
                {
                    *c = px;
                }
            }
            Some(Drag::Width { stroke, handle }) => {
                if let Some(s) = edit.strokes.get_mut(*stroke) {
                    let t = s.widths.get(*handle).map_or(0.0, |h| h.0);
                    if let Some((at, _)) = stroke_mesh::point_on(s, t)
                        && let Some(h) = s.widths.get_mut(*handle)
                    {
                        h.1 = (dist(at, px) / set.width.max(1e-3)).min(20.0);
                    }
                }
            }
            None => {}
        }
    }
    if resp.drag_stopped()
        && let Some(Drag::Draw { samples, replaces }) = edit.drag.take()
    {
        let points = stroke_mesh::simplify(&samples, 1.5 / view.scale.max(1e-6));
        let long = samples.windows(2).map(|w| match w {
            [a, b] => dist(*a, *b),
            _ => 0.0,
        });
        if points.len() >= 2 && long.sum::<f32>() * view.scale > 6.0 {
            let stroke = Stroke::new(points);
            // redrawing a stroke throws it away: ask first
            match replaces.filter(|&i| i < edit.strokes.len()) {
                Some(i) => edit.ask = Some((i, stroke)),
                None => edit.strokes.push(stroke),
            }
        }
    }
    if !resp.dragged() && !resp.drag_started() {
        edit.drag = None;
    }

    // clicks: points and handles in and out
    if resp.clicked() {
        if ctrl && alt {
            if let Some((si, hi)) = on_handle
                && let Some(s) = edit.strokes.get_mut(si)
                && hi < s.widths.len()
            {
                s.widths.remove(hi);
            }
        } else if ctrl {
            if let Some((si, t)) = on_line
                && let Some(s) = edit.strokes.get_mut(si)
            {
                let k = s.scale_at(t);
                s.widths.push((t, k));
                s.widths.sort_by(|a, b| a.0.total_cmp(&b.0));
            }
        } else if alt {
            if let Some((si, pi)) = on_point
                && let Some(s) = edit.strokes.get_mut(si)
            {
                if s.points.len() > 2 {
                    s.points.remove(pi);
                } else {
                    edit.strokes.remove(si);
                }
            }
        } else if on_point.is_none()
            && let Some((si, t)) = on_line
            && let Some(s) = edit.strokes.get_mut(si)
            && let Some((at, _)) = stroke_mesh::point_on(s, t)
        {
            let at_index = stroke_mesh::insert_at(s, t);
            s.points.insert(at_index.min(s.points.len()), at);
        }
    }

    let typing = ui.ctx().egui_wants_keyboard_input();
    let (confirm_key, cancel_key) = ui.ctx().input(|i| {
        (i.modifiers.shift && i.key_pressed(egui::Key::E), i.key_pressed(egui::Key::Delete))
    });
    if !typing && cancel_key {
        edit.strokes.clear();
        edit.drag = None;
    }

    draw(vp, edit, set, view, existing, opts.maintain);
    if on_point.is_some() || (ctrl && on_handle.is_some()) {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    } else {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
    }

    // Confirm stroke being edited, at the canvas's top left
    let mut confirm = !typing && confirm_key && !edit.strokes.is_empty();
    if !edit.strokes.is_empty() {
        let text = tr("Confirm stroke being edited (Click here [Shift+E])");
        let font = FontId::proportional(12.0);
        let galley = vp.layout_no_wrap(text.to_string(), font.clone(), theme::pal().text);
        let r =
            Rect::from_min_size(view.rect.min + vec2(10.0, 44.0), galley.size() + vec2(16.0, 10.0));
        let hit = ui.interact(r, egui::Id::new("stroke-confirm"), Sense::click());
        let fill = if hit.hovered() { theme::pal().header } else { theme::pal().chrome };
        vp.rect_filled(r, 4.0, fill);
        vp.rect_stroke(r, 4.0, Line::new(1.0, theme::pal().accent), egui::StrokeKind::Outside);
        vp.text(r.center(), Align2::CENTER_CENTER, text, font, theme::pal().accent);
        confirm |= hit.clicked();
    }
    confirm.then_some(StrokeAct::Confirm)
}

/// The strokes, the mesh they make, and (with Maintain current mesh off)
/// the points and lines of the mesh now that confirming removes.
fn draw(
    vp: &Painter,
    edit: &StrokeEdit,
    set: &Settings,
    view: View,
    existing: (&[Pos2], &[u16]),
    maintain: bool,
) {
    let red = theme::pal().rotation_red;
    let (pts, tris) = stroke_mesh::build_all(&edit.strokes, set);
    if !maintain && !tris.is_empty() {
        let (scr_old, idx) = existing;
        let old_px: Vec<P> = scr_old.iter().map(|&p| view.px(p)).collect();
        let old_tris: Vec<[u16; 3]> = idx.as_chunks::<3>().0.to_vec();
        let (keep, kept) = stroke_mesh::keep_existing(&old_px, &old_tris, &pts, &tris, false);
        let gone = Line::new(1.0, Color32::from_white_alpha(70));
        for t in old_tris.iter().filter(|t| !kept.contains(t)) {
            for (a, b) in [(t[0], t[1]), (t[1], t[2]), (t[2], t[0])] {
                if let (Some(&pa), Some(&pb)) = (scr_old.get(a as usize), scr_old.get(b as usize)) {
                    vp.add(egui::Shape::dashed_line(&[pa, pb], gone, 2.0, 3.0));
                }
            }
        }
        for (p, k) in scr_old.iter().zip(&keep) {
            if !k {
                let d = 3.5;
                let x = Line::new(1.5, red);
                vp.line_segment([*p - vec2(d, d), *p + vec2(d, d)], x);
                vp.line_segment([*p - vec2(d, -d), *p + vec2(d, -d)], x);
            }
        }
    }
    let scr: Vec<Pos2> = pts.iter().map(|&p| view.screen(p)).collect();
    let dash = Line::new(1.0, red);
    for t in &tris {
        for (a, b) in [(t[0], t[1]), (t[1], t[2]), (t[2], t[0])] {
            if let (Some(&pa), Some(&pb)) = (scr.get(a as usize), scr.get(b as usize)) {
                vp.add(egui::Shape::dashed_line(&[pa, pb], dash, 4.0, 3.0));
            }
        }
    }
    for p in &scr {
        vp.rect_filled(Rect::from_center_size(*p, vec2(4.0, 4.0)), 0.0, red);
    }
    let green = theme::pal().warp_green;
    for s in &edit.strokes {
        let line: Vec<Pos2> =
            stroke_mesh::curve(&s.points).iter().map(|&p| view.screen(p)).collect();
        vp.add(egui::Shape::line(line, Line::new(2.0, green)));
        for &(t, k) in &s.widths {
            if let Some((at, _)) = stroke_mesh::point_on(s, t) {
                let r = (set.width * k * view.scale).max(3.0);
                vp.circle_stroke(view.screen(at), r, Line::new(1.5, theme::ACCENT));
            }
        }
        for &c in &s.points {
            vp.circle(view.screen(c), 3.5, Color32::WHITE, Line::new(1.5, Color32::BLACK));
        }
    }
    if let Some(Drag::Draw { samples, .. }) = &edit.drag {
        let line: Vec<Pos2> = samples.iter().map(|&p| view.screen(p)).collect();
        vp.add(egui::Shape::line(line, Line::new(1.5, theme::pal().accent)));
    }
}

/// Where a point of the art's display lands on the texture: barycentric in
/// the triangle of the mesh as shown that holds it, else carried on from
/// the nearest one.
fn uv_at(positions: &[f32], uvs: &[f32], indices: &[u16], p: P) -> Option<[f32; 2]> {
    let get = |v: &[f32], i: u16| -> Option<P> {
        let i = i as usize * 2;
        Some([*v.get(i)?, *v.get(i + 1)?])
    };
    let mut best: Option<([f32; 3], [u16; 3], f32)> = None;
    for t in indices.as_chunks::<3>().0 {
        let [a, b, c] = *t;
        let (Some(pa), Some(pb), Some(pc)) =
            (get(positions, a), get(positions, b), get(positions, c))
        else {
            continue;
        };
        let den = (pb[1] - pc[1]) * (pa[0] - pc[0]) + (pc[0] - pb[0]) * (pa[1] - pc[1]);
        if den.abs() < 1e-9 {
            continue;
        }
        let w0 = ((pb[1] - pc[1]) * (p[0] - pc[0]) + (pc[0] - pb[0]) * (p[1] - pc[1])) / den;
        let w1 = ((pc[1] - pa[1]) * (p[0] - pc[0]) + (pa[0] - pc[0]) * (p[1] - pc[1])) / den;
        let w = [w0, w1, 1.0 - w0 - w1];
        let worst = w0.min(w1).min(w[2]);
        if best.is_none_or(|b| worst > b.2) {
            best = Some((w, [a, b, c], worst));
        }
        if worst >= 0.0 {
            break;
        }
    }
    let (w, ix, _) = best?;
    let mut uv = [0.0f32; 2];
    for (k, i) in w.iter().zip(ix) {
        let q = get(uvs, i)?;
        uv[0] += k * q[0];
        uv[1] += k * q[1];
    }
    Some([uv[0].clamp(0.0, 1.0), uv[1].clamp(0.0, 1.0)])
}

impl EditorUi {
    /// Confirm the strokes on their mesh: the stroke mesh joins what stays
    /// of the mesh (all of it with Maintain current mesh), as one undo step.
    pub(super) fn apply_stroke_mesh(&mut self) {
        let Some(edit) = self.stroke_edit.take() else { return };
        let mi = edit.mesh;
        let Some(art) = self.mesh_art.as_ref().filter(|a| a.mesh == mi) else {
            log::warn!("stroke mesh: open the mesh in Mesh Edit first");
            return;
        };
        let reference = art.reference.clone();
        let Some(s) = &mut self.session else { return };
        let c = s.puppet.model().canvas;
        let ppu = c.pixels_per_unit.max(1e-6);
        let to_px = |x: f32, y: f32| -> P { [c.origin_x + x * ppu, c.origin_y + y * ppu] };
        let to_model = |p: P| -> P { [(p[0] - c.origin_x) / ppu, (p[1] - c.origin_y) / ppu] };

        let (new_pts, new_tris) =
            stroke_mesh::build_all(&edit.strokes, &self.tool_opts.stroke.settings);
        // 1 across gives points only, which join the mesh as they are
        if new_pts.is_empty() {
            log::warn!("stroke mesh: the stroke is too short to make a mesh");
            return;
        }
        let mut new_uvs = Vec::with_capacity(new_pts.len() * 2);
        for &p in &new_pts {
            let Some(uv) = uv_at(&art.positions, &art.uvs, &art.indices, to_model(p)) else {
                log::warn!(
                    "stroke mesh: the mesh had no faces on entering Mesh Edit to place it on"
                );
                return;
            };
            new_uvs.extend(uv);
        }
        // the mesh as it is now, in canvas pixels too
        let Some(g) = s.puppet.mesh_geometry(mi) else { return };
        let old_pts: Vec<P> =
            g.positions.as_chunks::<2>().0.iter().map(|p| to_px(p[0], p[1])).collect();
        let old_tris: Vec<[u16; 3]> = g.indices.as_chunks::<3>().0.to_vec();
        let old_uvs = g.uvs.to_vec();
        let (keep, kept_tris) = stroke_mesh::keep_existing(
            &old_pts,
            &old_tris,
            &new_pts,
            &new_tris,
            self.tool_opts.stroke.maintain,
        );
        let mut uvs: Vec<f32> = Vec::new();
        let mut remap = vec![u16::MAX; old_pts.len()];
        for (i, k) in keep.iter().enumerate() {
            if *k
                && let (Some(u), Some(v)) = (old_uvs.get(i * 2), old_uvs.get(i * 2 + 1))
                && let Some(r) = remap.get_mut(i)
            {
                *r = (uvs.len() / 2) as u16;
                uvs.extend([*u, *v]);
            }
        }
        let mut idx: Vec<u16> = Vec::new();
        for t in &kept_tris {
            let m = t.map(|i| remap.get(i as usize).copied().unwrap_or(u16::MAX));
            if m.iter().all(|&i| i != u16::MAX) {
                idx.extend(m);
            }
        }
        let base = (uvs.len() / 2) as u16;
        uvs.extend(new_uvs);
        idx.extend(new_tris.iter().flat_map(|t| t.map(|i| i + base)));
        let (verts, faces) = (uvs.len() / 2, idx.len() / 3);
        if s.edit(|m| ayatsuri_core::edit::remesh_from(m, mi, Some(&reference), &uvs, &idx)) {
            self.pending_edges.remove(&mi);
            log::info!(
                "stroke mesh: {} now has {verts} vertices, {faces} triangles",
                s.mesh_name(mi)
            );
        } else {
            log::warn!("stroke mesh: the mesh could not be rebuilt");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::Session;
    use ayatsuri_format::Model;
    use ayatsuri_format::model::ArtMesh;

    /// A unit square of two faces whose texture maps one to one.
    fn editor() -> EditorUi {
        let mut model = Model::new();
        model.canvas.pixels_per_unit = 100.0;
        model.art_meshes.push(ArtMesh {
            id: "ArtMesh0".into(),
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
            clip_group: None,
            clip_inherit: false,
            binding: Default::default(),
            opacities: vec![1.0],
            draw_orders: vec![500.0],
            positions: vec![0.0, 0.0, 1.0, 0.0, 1.0, 1.0, 0.0, 1.0],
            blend_shapes: Vec::new(),
            multiply: Vec::new(),
            screen: Vec::new(),
        });
        let ctx = egui::Context::default();
        let s = Session::create(&ctx, "t".into(), String::new(), model).unwrap();
        let mut ed = EditorUi::new();
        let m = s.puppet.model().art_meshes[0].clone();
        ed.mesh_art = Some(super::super::MeshArt {
            mesh: 0,
            positions: m.positions.clone(),
            uvs: m.uvs.clone(),
            indices: m.indices.clone(),
            reference: m,
        });
        ed.session = Some(s);
        ed.mesh_edit = Some(0);
        ed
    }

    #[test]
    fn a_confirmed_stroke_joins_the_mesh_on_the_art_and_undoes() {
        let mut ed = editor();
        let c = ed.session.as_ref().unwrap().puppet.model().canvas;
        let px = |x: f32, y: f32| {
            [c.origin_x + x * c.pixels_per_unit, c.origin_y + y * c.pixels_per_unit]
        };
        ed.tool_opts.stroke.settings = Settings {
            width: 0.1 * c.pixels_per_unit,
            density: 0.2 * c.pixels_per_unit,
            across: 2,
            invert: false,
        };
        ed.stroke_edit = Some(StrokeEdit {
            mesh: 0,
            strokes: vec![Stroke::new(vec![px(0.1, 0.5), px(0.9, 0.5)])],
            drag: None,
            ask: None,
        });
        ed.apply_stroke_mesh();
        let s = ed.session.as_mut().unwrap();
        let m = &s.puppet.model().art_meshes[0];
        // the square's corners stay (the stroke covers none); 5 stations on
        // one side and 6 half-stations on the other zigzag between them
        assert_eq!(m.vertex_count, 4 + 11);
        assert_eq!(m.indices.len() / 3, 2 + 9);
        // the first station sits on the art where it was drawn
        assert!(
            (m.uvs[10] - 0.1).abs() < 1e-4 && (m.uvs[11] - 0.6).abs() < 1e-4,
            "{:?}",
            &m.uvs[10..12]
        );
        assert!((m.positions[10] - 0.1).abs() < 1e-4 && (m.positions[11] - 0.6).abs() < 1e-4);
        s.undo();
        assert_eq!(s.puppet.model().art_meshes[0].vertex_count, 4);
        assert!(ed.stroke_edit.is_none(), "confirm uses the strokes up");
    }

    #[test]
    fn without_maintain_the_stroke_takes_what_it_covers() {
        let mut ed = editor();
        let c = ed.session.as_ref().unwrap().puppet.model().canvas;
        let px = |x: f32, y: f32| {
            [c.origin_x + x * c.pixels_per_unit, c.origin_y + y * c.pixels_per_unit]
        };
        // a wide stroke over the bottom edge covers corners 0 and 1
        ed.tool_opts.stroke.settings = Settings {
            width: 0.2 * c.pixels_per_unit,
            density: 0.5 * c.pixels_per_unit,
            across: 2,
            invert: false,
        };
        ed.stroke_edit = Some(StrokeEdit {
            mesh: 0,
            strokes: vec![Stroke::new(vec![px(-0.1, 0.0), px(1.1, 0.0)])],
            drag: None,
            ask: None,
        });
        ed.apply_stroke_mesh();
        let n = ed.session.as_ref().unwrap().puppet.model().art_meshes[0].vertex_count;
        assert_eq!(n, 2 + 3 + 4, "corners 2 and 3 stay; 3 stations and 4 half-stations join them");
    }
}
