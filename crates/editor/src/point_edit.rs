// The point editing shared by mesh edit, form editing and warp grids: pick
// points (click, Shift+click, marquee), then move, scale or rotate them with
// Cubism's bounding box. The box shows red dots on its corners and edges;
// drag a dot to scale (Shift keeps the aspect, Alt scales from the centre),
// drag inside to move, drag just outside the box to rotate (Shift snaps to
// 15 degrees). Everything here is screen space; callers turn the final
// offsets into model edits.

use egui::{Color32, Painter, Pos2, Rect, Stroke, StrokeKind, Vec2, pos2, vec2};

/// What a drag grabbed.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Grab {
    /// One point (and the rest of the selection with it).
    Point,
    Move,
    Scale {
        hx: i8,
        hy: i8,
    },
    Rotate,
    Marquee,
}

/// A drag in flight: the points it moves with their start positions.
pub struct PtDrag {
    pub grab: Grab,
    pub start: Pos2,
    pub ptr: Pos2,
    pub from: Vec<(usize, Pos2)>,
    pub bbox: Rect,
}

/// How far outside the box the rotate band reaches, in screen px.
const BAND: f32 = 16.0;
const DOT: f32 = 6.0;

pub fn bbox(pts: impl Iterator<Item = Pos2>) -> Option<Rect> {
    let mut r: Option<Rect> = None;
    for p in pts {
        r = Some(match r {
            None => Rect::from_min_max(p, p),
            Some(r) => r.union(Rect::from_min_max(p, p)),
        });
    }
    // a flat selection still needs a box to grab
    r.map(|r| {
        Rect::from_center_size(r.center(), vec2(r.width().max(12.0), r.height().max(12.0)))
            .expand(4.0)
    })
}

pub fn handles(r: Rect) -> [(Pos2, i8, i8); 8] {
    let (x0, x1, y0, y1, cx, cy) = (r.min.x, r.max.x, r.min.y, r.max.y, r.center().x, r.center().y);
    [
        (pos2(x0, y0), -1, -1),
        (pos2(cx, y0), 0, -1),
        (pos2(x1, y0), 1, -1),
        (pos2(x1, cy), 1, 0),
        (pos2(x1, y1), 1, 1),
        (pos2(cx, y1), 0, 1),
        (pos2(x0, y1), -1, 1),
        (pos2(x0, cy), -1, 0),
    ]
}

/// What the pointer would grab on the box, ignoring points.
pub fn hit_box(r: Rect, p: Pos2) -> Option<Grab> {
    if let Some((_, hx, hy)) = handles(r).into_iter().find(|(h, ..)| h.distance(p) <= DOT + 2.0) {
        return Some(Grab::Scale { hx, hy });
    }
    if r.contains(p) {
        return Some(Grab::Move);
    }
    if r.expand(BAND).contains(p) {
        return Some(Grab::Rotate);
    }
    None
}

/// The affine map a drag applies: p' = a * (p - o) + o + t.
#[derive(Clone, Copy)]
pub struct Xform {
    o: Pos2,
    m: [f32; 4],
    t: Vec2,
}

impl Xform {
    pub fn apply(&self, p: Pos2) -> Pos2 {
        let d = p - self.o;
        self.o + vec2(self.m[0] * d.x + self.m[1] * d.y, self.m[2] * d.x + self.m[3] * d.y) + self.t
    }
}

pub fn xform(d: &PtDrag, shift: bool, alt: bool) -> Xform {
    let id = Xform { o: d.bbox.center(), m: [1.0, 0.0, 0.0, 1.0], t: Vec2::ZERO };
    match d.grab {
        Grab::Point | Grab::Move => Xform { t: d.ptr - d.start, ..id },
        Grab::Marquee => id,
        Grab::Scale { hx, hy } => {
            let r = d.bbox;
            let at = |h: i8, lo: f32, mid: f32, hi: f32| match h {
                -1 => lo,
                1 => hi,
                _ => mid,
            };
            let handle = pos2(
                at(hx, r.min.x, r.center().x, r.max.x),
                at(hy, r.min.y, r.center().y, r.max.y),
            );
            let anchor = if alt {
                r.center()
            } else {
                pos2(
                    at(-hx, r.min.x, r.center().x, r.max.x),
                    at(-hy, r.min.y, r.center().y, r.max.y),
                )
            };
            let ratio = |h: i8, p: f32, hp: f32, a: f32| {
                if h == 0 || (hp - a).abs() < 1e-3 { 1.0 } else { (p - a) / (hp - a) }
            };
            let mut sx = ratio(hx, d.ptr.x, handle.x, anchor.x);
            let mut sy = ratio(hy, d.ptr.y, handle.y, anchor.y);
            if shift {
                // keep the aspect: the axis pulled further wins
                let s = match (hx, hy) {
                    (0, _) => sy,
                    (_, 0) => sx,
                    _ => {
                        if sx.abs() > sy.abs() {
                            sx
                        } else {
                            sy
                        }
                    }
                };
                sx = s;
                sy = s;
            }
            Xform { o: anchor, m: [sx, 0.0, 0.0, sy], t: Vec2::ZERO }
        }
        Grab::Rotate => {
            let c = d.bbox.center();
            let (a0, a1) = (d.start - c, d.ptr - c);
            let mut ang = a1.y.atan2(a1.x) - a0.y.atan2(a0.x);
            if shift {
                let step = 15f32.to_radians();
                ang = (ang / step).round() * step;
            }
            let (s, co) = ang.sin_cos();
            Xform { o: c, m: [co, -s, s, co], t: Vec2::ZERO }
        }
    }
}

pub fn draw_box(p: &Painter, r: Rect, hot: Option<Grab>) {
    let red = Color32::from_rgb(0xE5, 0x48, 0x48);
    let line = if hot == Some(Grab::Rotate) { red } else { Color32::from_gray(0xd8) };
    p.rect_stroke(r, 0.0, Stroke::new(1.0, line), StrokeKind::Middle);
    for (h, hx, hy) in handles(r) {
        let hover = hot == Some(Grab::Scale { hx, hy });
        p.circle(h, if hover { DOT } else { DOT - 2.0 }, red, Stroke::new(1.0, Color32::WHITE));
    }
}

/// Selection and drag state for whichever object's points are on screen.
/// `owner` is (kind, index): 0 mesh in mesh edit, 1 mesh form, 2 warp.
#[derive(Default)]
pub struct PointState {
    pub owner: Option<(u8, usize)>,
    pub picked: std::collections::BTreeSet<usize>,
    pub drag: Option<PtDrag>,
    /// The pointer is over something this tool would take (a point or the
    /// box), so canvas clicks should not reselect objects underneath.
    pub hot: bool,
}

pub struct PtOpts {
    /// Show the box around every point when nothing is picked (form
    /// editing transforms the whole object, like Cubism).
    pub whole_box: bool,
    /// Dragging empty space draws a selection rectangle.
    pub marquee: bool,
    pub reach: f32,
}

pub struct PtOut {
    /// Final screen offsets when a drag lands.
    pub commit: Option<Vec<(usize, Vec2)>>,
    pub hover: Option<usize>,
    pub hover_grab: Option<Grab>,
}

impl PointState {
    /// Point selection belongs to one object; switching objects drops it.
    pub fn bind(&mut self, owner: (u8, usize)) {
        if self.owner != Some(owner) {
            self.owner = Some(owner);
            self.picked.clear();
            self.drag = None;
        }
    }

    pub fn clear(&mut self) {
        self.owner = None;
        self.picked.clear();
        self.drag = None;
        self.hot = false;
    }

    /// The box the tool shows right now, if any.
    pub fn active_box(&self, pts: &[Pos2], whole: bool) -> Option<Rect> {
        if self.picked.len() >= 2 {
            bbox(self.picked.iter().filter_map(|&i| pts.get(i).copied()))
        } else if self.picked.is_empty() && whole && pts.len() >= 2 {
            bbox(pts.iter().copied())
        } else {
            None
        }
    }
}

/// Input for the point tool: picks, drags, and the live preview (written
/// into `pts`). Call before drawing the object so the preview shows.
pub fn point_tool(
    st: &mut PointState,
    resp: &egui::Response,
    input: &egui::InputState,
    pts: &mut [Pos2],
    o: &PtOpts,
) -> PtOut {
    let ptr = input.pointer.latest_pos();
    let (shift, alt) = (input.modifiers.shift, input.modifiers.alt);
    let near = ptr.and_then(|p| {
        pts.iter()
            .enumerate()
            .map(|(i, q)| (i, q.distance(p)))
            .filter(|(_, d)| *d < o.reach)
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(i, _)| i)
    });
    let boxed = st.active_box(pts, o.whole_box);
    let box_grab = match (boxed, ptr) {
        (Some(r), Some(p)) => hit_box(r, p),
        _ => None,
    };
    // a scale dot beats a point under it; a point beats the box body
    let hover_grab = match (box_grab, near) {
        (Some(g @ Grab::Scale { .. }), _) => Some(g),
        (_, Some(_)) => Some(Grab::Point),
        (g, None) => g,
    };
    st.hot = hover_grab.is_some() || st.drag.is_some();

    if resp.drag_started()
        && let Some(start) = resp.interact_pointer_pos().or(ptr)
    {
        let grab = match hover_grab {
            Some(g) => g,
            None if o.marquee => Grab::Marquee,
            None => Grab::Move,
        };
        // grabbing an unpicked point makes it the selection
        if grab == Grab::Point
            && let Some(i) = near
            && !st.picked.contains(&i)
        {
            if !shift {
                st.picked.clear();
            }
            st.picked.insert(i);
        }
        let movers: Vec<usize> = if st.picked.is_empty() {
            if grab == Grab::Marquee || boxed.is_none() {
                Vec::new()
            } else {
                (0..pts.len()).collect()
            }
        } else {
            st.picked.iter().copied().collect()
        };
        if grab == Grab::Marquee || !movers.is_empty() {
            let bbox = boxed
                .or_else(|| bbox(movers.iter().filter_map(|&i| pts.get(i).copied())))
                .unwrap_or(Rect::from_min_max(start, start));
            st.drag = Some(PtDrag {
                grab,
                start,
                ptr: start,
                from: movers.iter().filter_map(|&i| pts.get(i).map(|p| (i, *p))).collect(),
                bbox,
            });
        }
    }
    if let Some(d) = &mut st.drag
        && let Some(p) = ptr
    {
        d.ptr = p;
    }

    let mut commit = None;
    if let Some(d) = &st.drag {
        if d.grab != Grab::Marquee {
            let x = xform(d, shift, alt);
            for &(i, p0) in &d.from {
                if let Some(p) = pts.get_mut(i) {
                    *p = x.apply(p0);
                }
            }
        }
        if resp.drag_stopped() || !input.pointer.any_down() {
            if d.grab == Grab::Marquee {
                let r = Rect::from_two_pos(d.start, d.ptr);
                if !shift {
                    st.picked.clear();
                }
                for (i, p) in pts.iter().enumerate() {
                    if r.contains(*p) {
                        st.picked.insert(i);
                    }
                }
            } else {
                let x = xform(d, shift, alt);
                let moved: Vec<(usize, Vec2)> = d
                    .from
                    .iter()
                    .map(|&(i, p0)| (i, x.apply(p0) - p0))
                    .filter(|(_, v)| v.length() > 1e-3)
                    .collect();
                if !moved.is_empty() {
                    commit = Some(moved);
                }
            }
            st.drag = None;
        }
    } else if resp.clicked() {
        match near {
            Some(i) if shift => {
                if !st.picked.remove(&i) {
                    st.picked.insert(i);
                }
            }
            Some(i) => {
                st.picked.clear();
                st.picked.insert(i);
            }
            None if box_grab.is_none() => st.picked.clear(),
            None => {}
        }
    }
    PtOut { commit, hover: near, hover_grab }
}

/// The tool's own drawing: picked points, the box, the marquee.
pub fn draw_overlay(st: &PointState, vp: &Painter, pts: &[Pos2], whole: bool, hot: Option<Grab>) {
    let accent = crate::theme::pal().accent;
    for &i in &st.picked {
        if let Some(&p) = pts.get(i) {
            vp.circle(p, 4.2, accent, Stroke::new(1.0, Color32::WHITE));
        }
    }
    match &st.drag {
        Some(d) if d.grab == Grab::Marquee => {
            let r = Rect::from_two_pos(d.start, d.ptr);
            vp.rect_filled(r, 0.0, accent.gamma_multiply(0.08));
            vp.rect_stroke(r, 0.0, Stroke::new(1.0, accent), StrokeKind::Middle);
        }
        Some(_) => {
            if let Some(r) = bbox(
                st.drag
                    .iter()
                    .flat_map(|d| d.from.iter())
                    .filter_map(|&(i, _)| pts.get(i).copied()),
            ) {
                draw_box(vp, r, st.drag.as_ref().map(|d| d.grab));
            }
        }
        None => {
            if let Some(r) = st.active_box(pts, whole) {
                draw_box(vp, r, hot);
            }
        }
    }
}

/// Deform path follow: a vertex near the path keeps its place relative to
/// the nearest segment, so moving a control point drags it along like a
/// bone. Built once when the grab starts.
pub struct PathBind {
    /// vertex, segment, position along it (0..1), offset in the segment's
    /// frame (along, across), weight.
    pub ties: Vec<(usize, usize, f32, Vec2, f32)>,
    pub path: Vec<Pos2>,
}

/// Tie every vertex within `width` of the path to its nearest segment. The
/// inner `hardness` share of the width follows fully; the rest fades out.
pub fn bind_path(path: &[Pos2], verts: &[Pos2], width: f32, hardness: f32) -> PathBind {
    let mut ties = Vec::new();
    let inner = width * hardness.clamp(0.0, 1.0);
    for (vi, &v) in verts.iter().enumerate() {
        let mut best: Option<(usize, f32, f32)> = None;
        if path.len() == 1
            && let Some(&p0) = path.first()
        {
            best = Some((0, 0.0, p0.distance(v)));
        }
        for j in 0..path.len().saturating_sub(1) {
            let (Some(&a), Some(&b)) = (path.get(j), path.get(j + 1)) else { continue };
            let seg = b - a;
            let t = ((v - a).dot(seg) / seg.length_sq().max(1e-6)).clamp(0.0, 1.0);
            let dist = v.distance(a + seg * t);
            if best.is_none_or(|(_, _, bd)| dist < bd) {
                best = Some((j, t, dist));
            }
        }
        let Some((j, t, dist)) = best else { continue };
        if dist > width {
            continue;
        }
        let w = if dist <= inner {
            1.0
        } else {
            let x = 1.0 - (dist - inner) / (width - inner).max(1e-3);
            x * x * (3.0 - 2.0 * x)
        };
        let (a, b) = (path.get(j).copied().unwrap_or(v), path.get(j + 1).copied());
        let (u, n) = frame(a, b);
        let at = b.map_or(a, |b| a + (b - a) * t);
        let off = v - at;
        ties.push((vi, j, t, vec2(off.dot(u), off.dot(n)), w));
    }
    PathBind { ties, path: path.to_vec() }
}

fn frame(a: Pos2, b: Option<Pos2>) -> (Vec2, Vec2) {
    let u = b.map(|b| b - a).filter(|d| d.length() > 1e-4).map_or(Vec2::X, |d| d.normalized());
    (u, vec2(-u.y, u.x))
}

/// Where each tied vertex goes now that the path is `moved` (same length).
/// Returns (vertex, screen offset) pairs.
pub fn follow_path(bind: &PathBind, verts: &[Pos2], moved: &[Pos2]) -> Vec<(usize, Vec2)> {
    let mut out = Vec::with_capacity(bind.ties.len());
    for &(vi, j, t, off, w) in &bind.ties {
        let Some(&v) = verts.get(vi) else { continue };
        let (Some(&a), Some(&a0)) = (moved.get(j), bind.path.get(j)) else { continue };
        let (b, b0) = (moved.get(j + 1).copied(), bind.path.get(j + 1).copied());
        let (u, n) = frame(a, b);
        // segments stretch: the along offset scales with the segment
        let stretch = match (b, b0) {
            (Some(b), Some(b0)) => (b - a).length() / (b0 - a0).length().max(1e-3),
            _ => 1.0,
        };
        let at = b.map_or(a, |b| a + (b - a) * t);
        let target = at + u * (off.x * stretch) + n * off.y;
        out.push((vi, (target - v) * w));
    }
    out
}

/// Least-squares affine map from screen points to their uvs (u and v each
/// a x + b y + c). Mesh edit uses it so moved points pick up the
/// texture under their new spot instead of stretching it.
pub fn fit_uv(pts: &[Pos2], uvs: &[f32]) -> Option<[f32; 6]> {
    // normal equations over [x, y, 1], kept as plain sums
    let (mut sxx, mut sxy, mut syy, mut sx, mut sy, mut n) = (0.0f64, 0.0, 0.0, 0.0, 0.0, 0.0);
    let (mut sxu, mut syu, mut su, mut sxv, mut syv, mut sv) = (0.0f64, 0.0, 0.0, 0.0, 0.0, 0.0);
    let (pairs, _) = uvs.as_chunks::<2>();
    for (p, &[u, v]) in pts.iter().zip(pairs) {
        let (x, y) = (p.x as f64, p.y as f64);
        let (u, v) = (u as f64, v as f64);
        sxx += x * x;
        sxy += x * y;
        syy += y * y;
        sx += x;
        sy += y;
        n += 1.0;
        sxu += x * u;
        syu += y * u;
        su += u;
        sxv += x * v;
        syv += y * v;
        sv += v;
    }
    if n < 3.0 {
        return None;
    }
    let det3 = |m: [f64; 9]| {
        let [a, b, c, d, e, f, g, h, i] = m;
        a * (e * i - f * h) - b * (d * i - f * g) + c * (d * h - e * g)
    };
    let m = [sxx, sxy, sx, sxy, syy, sy, sx, sy, n];
    let d = det3(m);
    if d.abs() < 1e-9 {
        return None;
    }
    // Cramer's rule: swap each column for the right-hand side in turn
    let solve = |r: [f64; 3]| -> [f32; 3] {
        let [r0, r1, r2] = r;
        let [a, b, c, dd, e, f, g, h, i] = m;
        [
            (det3([r0, b, c, r1, e, f, r2, h, i]) / d) as f32,
            (det3([a, r0, c, dd, r1, f, g, r2, i]) / d) as f32,
            (det3([a, b, r0, dd, e, r1, g, h, r2]) / d) as f32,
        ]
    };
    let [a, b, c] = solve([sxu, syu, su]);
    let [e, f, g] = solve([sxv, syv, sv]);
    Some([a, b, c, e, f, g])
}

pub fn uv_at(fit: &[f32; 6], p: Pos2) -> (f32, f32) {
    (fit[0] * p.x + fit[1] * p.y + fit[2], fit[3] * p.x + fit[4] * p.y + fit[5])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn drag(grab: Grab, start: Pos2, ptr: Pos2) -> PtDrag {
        PtDrag {
            grab,
            start,
            ptr,
            from: Vec::new(),
            bbox: Rect::from_min_max(pos2(0.0, 0.0), pos2(10.0, 10.0)),
        }
    }

    #[test]
    fn scale_from_opposite_corner() {
        let d = drag(Grab::Scale { hx: 1, hy: 1 }, pos2(10.0, 10.0), pos2(20.0, 20.0));
        let x = xform(&d, false, false);
        // the top-left corner stays, the bottom-right follows the pointer
        assert!(x.apply(pos2(0.0, 0.0)).distance(pos2(0.0, 0.0)) < 1e-4);
        assert!(x.apply(pos2(10.0, 10.0)).distance(pos2(20.0, 20.0)) < 1e-4);
        // alt scales around the centre instead
        let x = xform(&d, false, true);
        assert!(x.apply(pos2(5.0, 5.0)).distance(pos2(5.0, 5.0)) < 1e-4);
    }

    #[test]
    fn uv_fit_recovers_the_map() {
        let pts = [pos2(0.0, 0.0), pos2(100.0, 0.0), pos2(0.0, 50.0), pos2(30.0, 20.0)];
        let uvs: Vec<f32> = pts.iter().flat_map(|p| [p.x / 200.0 + 0.1, p.y / 100.0]).collect();
        let f = fit_uv(&pts, &uvs).unwrap();
        let (u, v) = uv_at(&f, pos2(50.0, 50.0));
        assert!((u - 0.35).abs() < 1e-4 && (v - 0.5).abs() < 1e-4);
    }

    #[test]
    fn rotate_quarter_turn() {
        let d = drag(Grab::Rotate, pos2(20.0, 5.0), pos2(5.0, 20.0));
        let x = xform(&d, false, false);
        assert!(x.apply(pos2(10.0, 5.0)).distance(pos2(5.0, 10.0)) < 1e-3);
    }

    #[test]
    fn path_carries_vertices() {
        let path = [pos2(0.0, 0.0), pos2(10.0, 0.0)];
        let verts = [pos2(10.0, 2.0), pos2(100.0, 100.0)];
        let bind = bind_path(&path, &verts, 5.0, 1.0);
        assert_eq!(bind.ties.len(), 1);
        // swing the end point down: the vertex near it swings too
        let moved = [pos2(0.0, 0.0), pos2(0.0, 10.0)];
        let off = follow_path(&bind, &verts, &moved);
        let (vi, d) = off[0];
        let now = verts[vi] + d;
        assert!(now.distance(pos2(-2.0, 10.0)) < 1e-3, "{now:?}");
    }
}
