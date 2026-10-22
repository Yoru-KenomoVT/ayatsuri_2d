// Point editing on the canvas (warp grids first): pick
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
    /// How much of the transform each point in `from` takes: 1 for picked
    /// points, the painted weight under a brush selection.
    pub weight: Vec<f32>,
    pub bbox: Rect,
    /// The lasso's outline, when the pick is freehand instead of a box.
    pub lasso: Option<Vec<Pos2>>,
}

impl PtDrag {
    /// Where point `k` of `from` sits under the transform `x`.
    fn place(&self, x: &Xform, k: usize) -> Option<(usize, Pos2)> {
        let &(i, p0) = self.from.get(k)?;
        let w = self.weight.get(k).copied().unwrap_or(1.0);
        Some((i, p0 + (x.apply(p0) - p0) * w))
    }
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

/// Shift-drag: a move goes straight, horizontally, vertically or at 45
/// degrees, whichever is nearest the pointer.
pub fn straight(d: Vec2) -> Vec2 {
    if d.length_sq() < 1e-12 {
        return d;
    }
    let step = std::f32::consts::FRAC_PI_4;
    let a = (d.y.atan2(d.x) / step).round() * step;
    let dir = Vec2::new(a.cos(), a.sin());
    dir * d.dot(dir)
}

pub fn xform(d: &PtDrag, shift: bool, alt: bool) -> Xform {
    let id = Xform { o: d.bbox.center(), m: [1.0, 0.0, 0.0, 1.0], t: Vec2::ZERO };
    match d.grab {
        Grab::Point | Grab::Move => {
            let t = d.ptr - d.start;
            Xform { t: if shift { straight(t) } else { t }, ..id }
        }
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
    /// A brush selection's per-point weights; empty when there is none.
    /// While present, every weighted point moves by its weight and the box
    /// wraps them.
    pub weights: Vec<f32>,
    /// Guide lines in screen space, (vertical, x or y): moving points snap
    /// to them. Empty when snapping is off.
    pub snap: Vec<(bool, f32)>,
    /// The guides a moving point sits on right now (indices into `snap`).
    pub snapped: Vec<usize>,
}

/// How close, in screen points, a moving point comes to a guide to land on it.
const SNAP: f32 = 7.0;

pub struct PtOpts {
    /// Show the box around every point when nothing is picked (form
    /// editing transforms the whole object, like Cubism).
    pub whole_box: bool,
    /// Dragging empty space draws a selection rectangle.
    pub marquee: bool,
    /// ...or a freehand lasso, for the lasso tool.
    pub lasso: bool,
    pub reach: f32,
    /// Extra room around the box, so its dots clear points on the edge.
    pub pad: f32,
}

pub struct PtOut {
    /// Final screen offsets when a drag lands.
    pub commit: Option<Vec<(usize, Vec2)>>,
    pub hover: Option<usize>,
    pub hover_grab: Option<Grab>,
}

/// Mirror edit: for every moved point, the point that sits where it would
/// reflect across the vertical axis `axis_x` (within `tol` screen points)
/// moves the mirrored way, unless it is moving already. Points on the
/// axis have no partner and are left as they are.
pub fn mirror_moves(moves: &mut Vec<(usize, Vec2)>, pts: &[Pos2], axis_x: f32, tol: f32) {
    let given: Vec<(usize, Vec2)> = moves.clone();
    for (i, d) in given {
        let Some(p) = pts.get(i) else { continue };
        if (p.x - axis_x).abs() <= tol {
            continue;
        }
        let want = pos2(2.0 * axis_x - p.x, p.y);
        let partner = pts
            .iter()
            .enumerate()
            .filter(|(j, q)| *j != i && q.distance(want) <= tol)
            .min_by(|a, b| a.1.distance(want).total_cmp(&b.1.distance(want)))
            .map(|(j, _)| j);
        if let Some(j) = partner
            && !moves.iter().any(|(k, _)| *k == j)
        {
            moves.push((j, Vec2::new(-d.x, d.y)));
        }
    }
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
        self.weights.clear();
    }

    fn weighted(&self) -> Vec<usize> {
        self.weights.iter().enumerate().filter(|(_, w)| **w > 1e-3).map(|(i, _)| i).collect()
    }

    /// The box the tool shows right now, if any.
    pub fn active_box(&self, pts: &[Pos2], whole: bool, pad: f32) -> Option<Rect> {
        let r = if !self.weights.is_empty() {
            bbox(self.weighted().into_iter().filter_map(|i| pts.get(i).copied()))
        } else if self.picked.len() >= 2 {
            bbox(self.picked.iter().filter_map(|&i| pts.get(i).copied()))
        } else if self.picked.is_empty() && whole && pts.len() >= 2 {
            bbox(pts.iter().copied())
        } else {
            None
        };
        r.map(|r| r.expand(pad))
    }
}

/// This frame's pointer state, copied out of egui first. The tool must not
/// run inside a `ctx.input` closure: Response methods take the same lock
/// again, and a nested read can deadlock the UI thread.
#[derive(Clone, Copy, Default)]
pub struct PtInput {
    pub ptr: Option<Pos2>,
    pub press: Option<Pos2>,
    /// Where the button went down. A drag only starts once the pointer has
    /// travelled a few pixels, so what it grabs is decided here, not at
    /// the pointer's position by then.
    pub origin: Option<Pos2>,
    pub shift: bool,
    pub alt: bool,
    pub any_down: bool,
    pub drag_started: bool,
    pub drag_stopped: bool,
    pub clicked: bool,
}

impl PtInput {
    pub fn read(ui: &egui::Ui, resp: &egui::Response) -> Self {
        let (ptr, origin, shift, alt, any_down) = ui.ctx().input(|i| {
            (
                i.pointer.latest_pos(),
                i.pointer.press_origin(),
                i.modifiers.shift,
                i.modifiers.alt,
                i.pointer.any_down(),
            )
        });
        Self {
            ptr,
            origin,
            press: resp.interact_pointer_pos(),
            shift,
            alt,
            any_down,
            // only the left button edits; a middle drag pans the view
            drag_started: resp.drag_started_by(egui::PointerButton::Primary),
            drag_stopped: resp.drag_stopped(),
            clicked: resp.clicked(),
        }
    }
}

/// Input for the point tool: picks, drags, and the live preview (written
/// into `pts`). Call before drawing the object so the preview shows.
pub fn point_tool(st: &mut PointState, inp: &PtInput, pts: &mut [Pos2], o: &PtOpts) -> PtOut {
    let ptr = inp.ptr;
    let (shift, alt) = (inp.shift, inp.alt);
    // hits are judged where the button went down when a drag begins
    let at = if inp.drag_started { inp.origin.or(ptr) } else { ptr };
    let weighted = !st.weights.is_empty();
    let reach = if weighted { 0.0 } else { o.reach };
    let near = at.and_then(|p| {
        pts.iter()
            .enumerate()
            .map(|(i, q)| (i, q.distance(p)))
            .filter(|(_, d)| *d < reach)
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(i, _)| i)
    });
    let boxed = st.active_box(pts, o.whole_box, o.pad);
    let box_grab = match (boxed, at) {
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

    if inp.drag_started
        && let Some(start) = inp.origin.or(inp.press).or(ptr)
    {
        let grab = match hover_grab {
            Some(g) => g,
            // with a brush selection a drag anywhere moves it
            None if o.marquee && !weighted => Grab::Marquee,
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
        let movers: Vec<usize> = if weighted {
            st.weighted()
        } else if st.picked.is_empty() {
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
                weight: movers
                    .iter()
                    .filter(|&&i| pts.get(i).is_some())
                    .map(
                        |&i| if weighted { st.weights.get(i).copied().unwrap_or(0.0) } else { 1.0 },
                    )
                    .collect(),
                bbox,
                lasso: (grab == Grab::Marquee && o.lasso).then(|| vec![start]),
            });
        }
    }
    st.snapped.clear();
    if let Some(d) = &mut st.drag
        && let Some(p) = ptr
    {
        d.ptr = p;
        if matches!(d.grab, Grab::Point | Grab::Move) && !st.snap.is_empty() {
            let raw = p - d.start;
            let mut fix_x: Option<(f32, usize)> = None;
            let mut fix_y: Option<(f32, usize)> = None;
            for (k, &(_, p0)) in d.from.iter().enumerate().take(4000) {
                if d.weight.get(k).is_some_and(|w| *w < 0.999) {
                    continue;
                }
                let q = p0 + raw;
                for (gi, &(vertical, at)) in st.snap.iter().enumerate() {
                    let off = if vertical { at - q.x } else { at - q.y };
                    let best = if vertical { &mut fix_x } else { &mut fix_y };
                    if off.abs() <= SNAP && best.is_none_or(|(b, _)| off.abs() < b.abs()) {
                        *best = Some((off, gi));
                    }
                }
            }
            let mut t = raw;
            if let Some((off, gi)) = fix_x {
                t.x += off;
                st.snapped.push(gi);
            }
            if let Some((off, gi)) = fix_y {
                t.y += off;
                st.snapped.push(gi);
            }
            d.ptr = d.start + t;
        }
        if let Some(path) = &mut d.lasso
            && path.last().is_none_or(|l| l.distance(p) > 2.0)
        {
            path.push(p);
        }
    }

    let mut commit = None;
    if let Some(d) = &st.drag {
        if d.grab != Grab::Marquee {
            let x = xform(d, shift, alt);
            for k in 0..d.from.len() {
                if let Some((i, q)) = d.place(&x, k)
                    && let Some(p) = pts.get_mut(i)
                {
                    *p = q;
                }
            }
        }
        if inp.drag_stopped || !inp.any_down {
            if d.grab == Grab::Marquee {
                let r = Rect::from_two_pos(d.start, d.ptr);
                if !shift {
                    st.picked.clear();
                }
                for (i, p) in pts.iter().enumerate() {
                    let inside = match &d.lasso {
                        Some(path) => path.len() > 2 && point_in_polygon(*p, path),
                        None => r.contains(*p),
                    };
                    if inside {
                        st.picked.insert(i);
                    }
                }
            } else {
                let x = xform(d, shift, alt);
                let moved: Vec<(usize, Vec2)> = (0..d.from.len())
                    .filter_map(|k| {
                        let (i, q) = d.place(&x, k)?;
                        let p0 = d.from.get(k)?.1;
                        Some((i, q - p0))
                    })
                    .filter(|(_, v)| v.length() > 1e-3)
                    .collect();
                if !moved.is_empty() {
                    commit = Some(moved);
                }
            }
            st.drag = None;
        }
    } else if inp.clicked {
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
pub fn draw_overlay(
    st: &PointState,
    vp: &Painter,
    pts: &[Pos2],
    whole: bool,
    pad: f32,
    hot: Option<Grab>,
) {
    let accent = crate::theme::pal().accent;
    // picked points, and a lone point being dragged, show red; a single
    // point needs no transform box around it
    let red = Color32::from_rgb(0xE5, 0x48, 0x48);
    for &i in &st.picked {
        if let Some(&p) = pts.get(i) {
            vp.circle(p, 4.2, red, Stroke::new(1.0, Color32::WHITE));
        }
    }
    match &st.drag {
        Some(d) if d.grab == Grab::Marquee => match &d.lasso {
            Some(path) => {
                if path.len() > 1 {
                    vp.add(egui::Shape::line(path.clone(), Stroke::new(1.0, accent)));
                }
                if let (Some(&a), Some(&b)) = (path.first(), path.last()) {
                    vp.add(egui::Shape::dashed_line(&[b, a], Stroke::new(1.0, accent), 4.0, 3.0));
                }
            }
            None => {
                let r = Rect::from_two_pos(d.start, d.ptr);
                vp.rect_filled(r, 0.0, accent.gamma_multiply(0.08));
                vp.rect_stroke(r, 0.0, Stroke::new(1.0, accent), StrokeKind::Middle);
            }
        },
        Some(d) if d.from.len() < 2 => {
            for &(i, _) in &d.from {
                if let Some(&p) = pts.get(i) {
                    vp.circle(p, 4.6, red, Stroke::new(1.0, Color32::WHITE));
                }
            }
        }
        Some(d) => {
            if let Some(r) = bbox(d.from.iter().filter_map(|&(i, _)| pts.get(i).copied())) {
                draw_box(vp, r.expand(pad), Some(d.grab));
            }
        }
        None => {
            if let Some(r) = st.active_box(pts, whole, pad) {
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

/// Even-odd test: is `p` inside the closed outline `poly`?
pub fn point_in_polygon(p: Pos2, poly: &[Pos2]) -> bool {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn drag(grab: Grab, start: Pos2, ptr: Pos2) -> PtDrag {
        PtDrag {
            grab,
            start,
            ptr,
            from: Vec::new(),
            weight: Vec::new(),
            lasso: None,
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
    fn rotate_quarter_turn() {
        let d = drag(Grab::Rotate, pos2(20.0, 5.0), pos2(5.0, 20.0));
        let x = xform(&d, false, false);
        assert!(x.apply(pos2(10.0, 5.0)).distance(pos2(5.0, 10.0)) < 1e-3);
    }

    #[test]
    fn brush_weights_scale_the_move() {
        let mut st = PointState { weights: vec![1.0, 0.5, 0.0], ..PointState::default() };
        let mut pts = [pos2(0.0, 0.0), pos2(10.0, 0.0), pos2(20.0, 0.0)];
        let opts = PtOpts { whole_box: true, marquee: true, lasso: false, reach: 8.0, pad: 0.0 };
        // press away from every point, drag 10 px down
        let start = PtInput {
            ptr: Some(pos2(5.0, 40.0)),
            origin: Some(pos2(5.0, 40.0)),
            any_down: true,
            drag_started: true,
            ..PtInput::default()
        };
        point_tool(&mut st, &start, &mut pts, &opts);
        let end = PtInput {
            ptr: Some(pos2(5.0, 50.0)),
            any_down: false,
            drag_stopped: true,
            ..PtInput::default()
        };
        let out = point_tool(&mut st, &end, &mut pts, &opts);
        let moved = out.commit.unwrap();
        let by = |i: usize| moved.iter().find(|m| m.0 == i).map(|m| m.1.y).unwrap_or(0.0);
        assert!((by(0) - 10.0).abs() < 1e-4);
        assert!((by(1) - 5.0).abs() < 1e-4);
        assert_eq!(by(2), 0.0);
    }

    #[test]
    fn mirror_moves_find_the_partner_across_the_axis() {
        // four points: a pair mirrored about x = 100, one on the axis, one alone
        let pts = [pos2(60.0, 50.0), pos2(140.0, 50.0), pos2(100.0, 80.0), pos2(30.0, 90.0)];
        let mut moves =
            vec![(0, Vec2::new(5.0, -2.0)), (2, Vec2::new(1.0, 1.0)), (3, Vec2::new(2.0, 0.0))];
        mirror_moves(&mut moves, &pts, 100.0, 3.0);
        assert_eq!(moves.len(), 4);
        assert_eq!(moves[3], (1, Vec2::new(-5.0, -2.0)));
        // a partner already moving is not pushed twice
        let mut both = vec![(0, Vec2::new(5.0, 0.0)), (1, Vec2::new(1.0, 0.0))];
        mirror_moves(&mut both, &pts, 100.0, 3.0);
        assert_eq!(both.len(), 2);
    }

    #[test]
    fn a_dragged_point_lands_on_a_nearby_guide() {
        // a vertical guide at x = 50
        let mut st = PointState { snap: vec![(true, 50.0)], ..PointState::default() };
        let mut pts = [pos2(0.0, 0.0), pos2(100.0, 100.0)];
        let opts = PtOpts { whole_box: false, marquee: true, lasso: false, reach: 8.0, pad: 0.0 };
        let start = PtInput {
            ptr: Some(pos2(0.0, 0.0)),
            origin: Some(pos2(0.0, 0.0)),
            any_down: true,
            drag_started: true,
            ..PtInput::default()
        };
        point_tool(&mut st, &start, &mut pts, &opts);
        // let go 4 px short of the guide
        let end = PtInput {
            ptr: Some(pos2(46.0, 20.0)),
            any_down: false,
            drag_stopped: true,
            ..PtInput::default()
        };
        let out = point_tool(&mut st, &end, &mut pts, &opts);
        let moved = out.commit.unwrap();
        let d = moved.iter().find(|m| m.0 == 0).map(|m| m.1).unwrap();
        assert!((d.x - 50.0).abs() < 1e-4, "snapped onto the guide: {d:?}");
        assert!((d.y - 20.0).abs() < 1e-4, "no horizontal guide, y as dragged");
    }

    #[test]
    fn lasso_picks_points_inside_the_outline() {
        let mut st = PointState::default();
        let mut pts = [pos2(10.0, 10.0), pos2(50.0, 50.0), pos2(90.0, 10.0)];
        let opts = PtOpts { whole_box: false, marquee: true, lasso: true, reach: 4.0, pad: 0.0 };
        let step = |st: &mut PointState, pts: &mut [Pos2], at: Pos2, first: bool, last: bool| {
            let inp = PtInput {
                ptr: Some(at),
                origin: Some(pos2(0.0, 0.0)),
                any_down: !last,
                drag_started: first,
                drag_stopped: last,
                ..PtInput::default()
            };
            point_tool(st, &inp, pts, &opts);
        };
        // a loop around the first two points only
        step(&mut st, &mut pts, pos2(0.0, 0.0), true, false);
        for p in [pos2(70.0, 0.0), pos2(70.0, 70.0), pos2(0.0, 70.0)] {
            step(&mut st, &mut pts, p, false, false);
        }
        step(&mut st, &mut pts, pos2(0.0, 1.0), false, true);
        assert_eq!(st.picked.iter().copied().collect::<Vec<_>>(), vec![0, 1]);
    }

    #[test]
    fn path_carries_vertices() {
        let path = [pos2(0.0, 0.0), pos2(10.0, 0.0)];
        let verts = [pos2(10.0, 2.0), pos2(100.0, 100.0)];
        let bind = bind_path(&path, &verts, 5.0, 1.0);
        assert_eq!(bind.ties.len(), 1);
        // swing the end point down: the vertex near it swings with it
        let moved = [pos2(0.0, 0.0), pos2(0.0, 10.0)];
        let off = follow_path(&bind, &verts, &moved);
        let (vi, d) = off[0];
        assert!((verts[vi] + d).distance(pos2(-2.0, 10.0)) < 1e-3);
    }
}
