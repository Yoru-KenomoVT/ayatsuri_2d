// Modeling > Temporary Deform Tool, as Cubism has it: throwaway control
// points around the selected meshes or warp deformer that bend them and
// are gone once the tool is released. Seven ways to transform: Enlarge/
// Reduce, Rotation, Free Transform, Perspective and Distort on a box,
// Temporary Warp on a grid, and Temporary Path on a path you click out.
//
// Every drag lands on the keyform the parameters sit on as one undo step.
// Ctrl+drag moves a control point only (the art stays put), Shift+drag
// moves straight (horizontally, vertically or at 45 degrees), and the
// anchor in the middle is the centre for Rotation and Enlarge/Reduce.

use egui::{Align2, Color32, FontId, Pos2, Rect, Response, Stroke, Ui, Vec2, pos2, vec2};

use super::{EditorUi, LiveDrag, LiveTarget, Sel, Tool, parent_delta};
use crate::i18n::tr;
use crate::session::Session;
use crate::theme;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Method {
    Zoom,
    Rotation,
    Free,
    Perspective,
    Distort,
    Warp,
    Path,
}

impl Method {
    pub(super) const ALL: [Method; 7] = [
        Method::Zoom,
        Method::Rotation,
        Method::Free,
        Method::Perspective,
        Method::Distort,
        Method::Warp,
        Method::Path,
    ];

    pub(super) fn label(self) -> &'static str {
        match self {
            Method::Zoom => "Enlarge/Reduce",
            Method::Rotation => "Rotation",
            Method::Free => "Free Transform",
            Method::Perspective => "Perspective",
            Method::Distort => "Distort",
            Method::Warp => "Temporary Warp Deformation",
            Method::Path => "Temporary Path Deformation",
        }
    }
}

type V = Vec2;

/// Point `i` of a list, or the origin past its end.
fn at(v: &[V], i: usize) -> V {
    v.get(i).copied().unwrap_or(V::ZERO)
}

fn bump(v: &mut [V], i: usize, d: V) {
    if let Some(p) = v.get_mut(i) {
        *p += d;
    }
}

/// What the bound points ride on.
#[derive(Clone, Debug)]
enum Frame {
    /// Corners top-left, top-right, bottom-right, bottom-left, as bound and
    /// as they are now, and the anchor.
    Quad { rest: [V; 4], now: [V; 4], anchor: V },
    /// (cols + 1) x (rows + 1) points row by row, and each bound point's
    /// place in the grid (cell units).
    Grid { cols: usize, rows: usize, now: Vec<V>, uv: Vec<V> },
    /// Control points as bound and now, and each bound point's place.
    Path { rest: Vec<V>, now: Vec<V>, bind: Vec<PathBind> },
}

/// A point held by a path: the sample span, how far along it, and its
/// offset in the path's local frame there.
#[derive(Clone, Copy, Debug)]
struct PathBind {
    k: usize,
    t: f32,
    a: f32,
    b: f32,
}

/// Which control a drag holds.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Grab {
    Corner(usize),
    Edge(usize),
    Anchor,
    Stalk,
    Point(usize),
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Mode {
    Deform,
    /// Ctrl: move the control only.
    Control,
}

struct Drag {
    grab: Grab,
    mode: Mode,
    from: V,
    frame: Frame,
    /// Bound positions when the drag began.
    start: Vec<V>,
    live: Vec<Option<LiveDrag>>,
}

pub(super) struct TempDeform {
    pub(super) method: Method,
    pub(super) cols: usize,
    pub(super) rows: usize,
    /// The tool that was active before, back once this is released.
    pub(super) prev_tool: Tool,
    targets: Vec<(LiveTarget, usize)>,
    /// Bound world positions, every target's points one after another.
    base: Vec<V>,
    frame: Frame,
    values: Vec<f32>,
    drag: Option<Drag>,
}

/// Screen and model units, for one frame of the view.
#[derive(Clone, Copy)]
pub(super) struct ViewXf {
    pub(super) origin: Pos2,
    pub(super) scale: f32,
    pub(super) cx: f32,
    pub(super) cy: f32,
    pub(super) ppu: f32,
}

impl ViewXf {
    fn screen(&self, p: V) -> Pos2 {
        pos2(
            self.origin.x + (self.cx + p.x * self.ppu) * self.scale,
            self.origin.y + (self.cy + p.y * self.ppu) * self.scale,
        )
    }
    fn world(&self, p: Pos2) -> V {
        vec2(
            ((p.x - self.origin.x) / self.scale - self.cx) / self.ppu,
            ((p.y - self.origin.y) / self.scale - self.cy) / self.ppu,
        )
    }
    /// Screen pixels per model unit.
    fn px(&self) -> f32 {
        self.scale * self.ppu
    }
}

// ---- the maths -------------------------------------------------------

/// A grid point at `uv` (cell units); outside the grid the edge cells
/// carry on.
fn grid_eval(now: &[V], cols: usize, rows: usize, uv: V) -> V {
    let i = (uv.x.floor() as isize).clamp(0, cols as isize - 1) as usize;
    let j = (uv.y.floor() as isize).clamp(0, rows as isize - 1) as usize;
    let (fu, fv) = (uv.x - i as f32, uv.y - j as f32);
    let at = |c: usize, r: usize| now.get(r * (cols + 1) + c).copied().unwrap_or(V::ZERO);
    let (p00, p10, p01, p11) = (at(i, j), at(i + 1, j), at(i, j + 1), at(i + 1, j + 1));
    p00 * (1.0 - fu) * (1.0 - fv) + p10 * fu * (1.0 - fv) + p01 * (1.0 - fu) * fv + p11 * fu * fv
}

/// A point bound in box `rest` carried to box `now` (corners clockwise
/// from the top left), bilinearly, as Cubism's Perspective and Distort
/// move the art: rows and columns stay evenly spaced, and the box's
/// middle goes to the average of its corners.
fn quad_carry(rest: &[V; 4], now: &[V; 4], p: V) -> V {
    let grid = |q: &[V; 4]| [q[0], q[1], q[3], q[2]];
    let uv = grid_find(&grid(rest), 1, 1, p, vec2(0.5, 0.5));
    grid_eval(&grid(now), 1, 1, uv)
}

/// The grid place of world point `p`, by Newton from `guess`.
fn grid_find(now: &[V], cols: usize, rows: usize, p: V, guess: V) -> V {
    let mut uv = guess;
    for _ in 0..24 {
        let f = grid_eval(now, cols, rows, uv);
        let e = p - f;
        if e.length_sq() < 1e-12 {
            break;
        }
        let h = 1e-3;
        let du = (grid_eval(now, cols, rows, uv + vec2(h, 0.0)) - f) / h;
        let dv = (grid_eval(now, cols, rows, uv + vec2(0.0, h)) - f) / h;
        let det = du.x * dv.y - dv.x * du.y;
        if det.abs() < 1e-12 || !det.is_finite() {
            break;
        }
        let step = vec2((e.x * dv.y - dv.x * e.y) / det, (du.x * e.y - e.x * du.y) / det);
        uv += step;
    }
    uv
}

/// The path through its control points as a smooth curve (Catmull-Rom),
/// sampled the same way for any points so rest and now line up.
const SPAN: usize = 12;

fn path_samples(pts: &[V]) -> Vec<V> {
    if pts.len() < 2 {
        return pts.to_vec();
    }
    let n = pts.len();
    let pick = |i: isize| at(pts, i.clamp(0, n as isize - 1) as usize);
    let mut out = Vec::with_capacity((n - 1) * SPAN + 1);
    for i in 0..n - 1 {
        let (p0, p1, p2, p3) =
            (pick(i as isize - 1), pick(i as isize), pick(i as isize + 1), pick(i as isize + 2));
        for s in 0..SPAN {
            let t = s as f32 / SPAN as f32;
            let (t2, t3) = (t * t, t * t * t);
            out.push(
                ((p1 * 2.0)
                    + (p2 - p0) * t
                    + (p0 * 2.0 - p1 * 5.0 + p2 * 4.0 - p3) * t2
                    + (p1 * 3.0 - p0 - p2 * 3.0 + p3) * t3)
                    * 0.5,
            );
        }
    }
    out.push(at(pts, n - 1));
    out
}

/// Unit tangents at each sample.
fn tangents(s: &[V]) -> Vec<V> {
    let n = s.len();
    (0..n)
        .map(|i| {
            let d = at(s, (i + 1).min(n - 1)) - at(s, i.saturating_sub(1));
            d.normalized()
        })
        .collect()
}

/// The point and frame at span `k`, fraction `t`.
fn path_frame(s: &[V], tg: &[V], k: usize, t: f32) -> (V, V, V) {
    let p = at(s, k) + (at(s, k + 1) - at(s, k)) * t;
    let tan = (at(tg, k) * (1.0 - t) + at(tg, k + 1) * t).normalized();
    (p, tan, vec2(-tan.y, tan.x))
}

fn path_bind(rest: &[V], p: V) -> PathBind {
    let s = path_samples(rest);
    if s.len() < 2 {
        let d = s.first().map_or(V::ZERO, |&o| p - o);
        return PathBind { k: 0, t: 0.0, a: d.x, b: d.y };
    }
    let mut best = (f32::MAX, 0, 0.0);
    for k in 0..s.len() - 1 {
        let (a, b) = (at(&s, k), at(&s, k + 1));
        let ab = b - a;
        let t = if ab.length_sq() > 0.0 {
            ((p - a).dot(ab) / ab.length_sq()).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let d = (a + ab * t - p).length_sq();
        if d < best.0 {
            best = (d, k, t);
        }
    }
    let tg = tangents(&s);
    let (o, tan, nor) = path_frame(&s, &tg, best.1, best.2);
    PathBind { k: best.1, t: best.2, a: (p - o).dot(tan), b: (p - o).dot(nor) }
}

fn path_eval(s: &[V], tg: &[V], b: &PathBind) -> V {
    if s.len() < 2 {
        return s.first().map_or(V::ZERO, |&o| o + vec2(b.a, b.b));
    }
    let k = b.k.min(s.len() - 2);
    let (o, tan, nor) = path_frame(s, tg, k, b.t);
    o + tan * b.a + nor * b.b
}

impl Frame {
    fn new(method: Method, pts: &[V], cols: usize, rows: usize) -> Self {
        let (mut lo, mut hi) = (vec2(f32::MAX, f32::MAX), vec2(f32::MIN, f32::MIN));
        for p in pts {
            lo = lo.min(*p);
            hi = hi.max(*p);
        }
        if pts.is_empty() {
            (lo, hi) = (V::ZERO, V::ZERO);
        }
        // a flat selection still gets a box with some height
        let pad = ((hi - lo).max_elem() * 0.03).max(1e-3);
        let (lo, hi) = (lo - vec2(pad, pad), hi + vec2(pad, pad));
        match method {
            Method::Warp => {
                let size = hi - lo;
                let now = (0..=rows)
                    .flat_map(|r| {
                        (0..=cols).map(move |c| {
                            lo + vec2(
                                size.x * c as f32 / cols as f32,
                                size.y * r as f32 / rows as f32,
                            )
                        })
                    })
                    .collect();
                let uv = pts
                    .iter()
                    .map(|p| {
                        let q = (*p - lo) / size;
                        vec2(q.x * cols as f32, q.y * rows as f32)
                    })
                    .collect();
                Frame::Grid { cols, rows, now, uv }
            }
            Method::Path => Frame::Path { rest: Vec::new(), now: Vec::new(), bind: Vec::new() },
            _ => {
                let q = [lo, vec2(hi.x, lo.y), hi, vec2(lo.x, hi.y)];
                Frame::Quad { rest: q, now: q, anchor: (lo + hi) * 0.5 }
            }
        }
    }

    /// Where the bound points are now.
    fn map(&self, base: &[V]) -> Vec<V> {
        match self {
            Frame::Quad { rest, now, .. } => {
                base.iter().map(|&p| quad_carry(rest, now, p)).collect()
            }
            Frame::Grid { cols, rows, now, uv } => {
                uv.iter().map(|&q| grid_eval(now, *cols, *rows, q)).collect()
            }
            Frame::Path { rest, now, bind } => {
                if rest.is_empty() {
                    return base.to_vec();
                }
                let s = path_samples(now);
                let tg = if s.len() >= 2 { tangents(&s) } else { Vec::new() };
                bind.iter().map(|b| path_eval(&s, &tg, b)).collect()
            }
        }
    }

    /// Make `held` the bound positions with the controls where they are,
    /// so nothing moves until the next drag.
    fn rebind(&mut self, held: &[V]) {
        match self {
            Frame::Quad { rest, now, .. } => *rest = *now,
            Frame::Grid { cols, rows, now, uv } => {
                for (q, p) in uv.iter_mut().zip(held) {
                    *q = grid_find(now, *cols, *rows, *p, *q);
                }
            }
            Frame::Path { rest, now, bind } => {
                *rest = now.clone();
                *bind = if rest.is_empty() {
                    Vec::new()
                } else {
                    held.iter().map(|&p| path_bind(rest, p)).collect()
                };
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
/// The box after dragging `grab` from `from` to `to` (world units).
fn quad_drag(
    method: Method,
    q0: &[V; 4],
    a0: V,
    grab: Grab,
    mode: Mode,
    from: V,
    to: V,
    snap: bool,
) -> ([V; 4], V) {
    let d = to - from;
    let mut q = *q0;
    match grab {
        Grab::Anchor => return (q, a0 + d),
        Grab::Corner(i) if mode == Mode::Control => bump(&mut q, i, d),
        Grab::Edge(i) if mode == Mode::Control => {
            bump(&mut q, i, d);
            bump(&mut q, (i + 1) % 4, d);
        }
        Grab::Corner(_) | Grab::Stalk if method == Method::Rotation => {
            let (u, v) = (from - a0, to - a0);
            let mut th = v.y.atan2(v.x) - u.y.atan2(u.x);
            if snap {
                let step = 15f32.to_radians();
                th = (th / step).round() * step;
            }
            let (s, c) = th.sin_cos();
            q = q0.map(|p| {
                let r = p - a0;
                a0 + vec2(r.x * c - r.y * s, r.x * s + r.y * c)
            });
        }
        Grab::Corner(i) => match method {
            Method::Zoom => {
                let u = at(q0, i) - a0;
                if u.length_sq() > 1e-12 {
                    let s = (to - a0).dot(u) / u.length_sq();
                    q = q0.map(|p| a0 + (p - a0) * s);
                }
            }
            Method::Free => {
                // scale along the box's own sides, the opposite corner fixed
                let o = (i + 2) % 4;
                let qo = at(q0, o);
                let (e1, e2) = (at(q0, (o + 1) % 4) - qo, at(q0, (o + 3) % 4) - qo);
                let det = e1.x * e2.y - e2.x * e1.y;
                if det.abs() > 1e-12 {
                    let coef = |p: V| {
                        let r = p - qo;
                        ((r.x * e2.y - e2.x * r.y) / det, (e1.x * r.y - r.x * e1.y) / det)
                    };
                    let (a0c, b0c) = coef(at(q0, i));
                    let (a1, b1) = coef(at(q0, i) + d);
                    let sa = if a0c.abs() > 1e-6 { a1 / a0c } else { 1.0 };
                    let sb = if b0c.abs() > 1e-6 { b1 / b0c } else { 1.0 };
                    q = q0.map(|p| {
                        let (a, b) = coef(p);
                        qo + e1 * (a * sa) + e2 * (b * sb)
                    });
                }
            }
            Method::Perspective => {
                let h = (q0[1] - q0[0]).normalized();
                let v = (q0[3] - q0[0]).normalized();
                let (dh, dv) = (d.dot(h), d.dot(v));
                // as Cubism's: the corner keeps to one of its sides, the
                // way the drag mostly goes, and the corner across that side
                // mirrors it, so the box tapers on that side only
                if dh.abs() >= dv.abs() {
                    bump(&mut q, i, h * dh);
                    bump(&mut q, i ^ 1, -(h * dh));
                } else {
                    bump(&mut q, i, v * dv);
                    bump(&mut q, 3 - i, -(v * dv));
                }
            }
            _ => bump(&mut q, i, d),
        },
        Grab::Edge(i) => match method {
            Method::Zoom => {
                let m = (at(q0, i) + at(q0, (i + 1) % 4)) * 0.5;
                let u = m - a0;
                if u.length_sq() > 1e-12 {
                    let s = (to - a0).dot(u) / u.length_sq();
                    q = q0.map(|p| p + u * ((s - 1.0) * (p - a0).dot(u) / u.length_sq()));
                }
            }
            Method::Perspective => {
                // as Cubism's: the side slides along itself, a shear, and
                // the opposite side stays
                let e = (at(q0, (i + 1) % 4) - at(q0, i)).normalized();
                let along = e * d.dot(e);
                bump(&mut q, i, along);
                bump(&mut q, (i + 1) % 4, along);
            }
            _ => {
                bump(&mut q, i, d);
                bump(&mut q, (i + 1) % 4, d);
            }
        },
        Grab::Stalk | Grab::Point(_) => {}
    }
    (q, a0)
}

fn handles(method: Method) -> (bool, bool) {
    // (edge handles, rotation stalk)
    match method {
        Method::Rotation => (false, true),
        _ => (true, false),
    }
}

fn stalk(q: &[V; 4], px: f32) -> V {
    let top = (q[0] + q[1]) * 0.5;
    let up = (top - (q[2] + q[3]) * 0.5).normalized();
    top + up * (28.0 / px)
}

// ---- the tool --------------------------------------------------------

impl TempDeform {
    fn bind(
        s: &mut Session,
        method: Method,
        targets: &[LiveTarget],
        cols: usize,
        rows: usize,
        prev_tool: Tool,
    ) -> Result<Self, &'static str> {
        let mut kept = Vec::new();
        let mut base = Vec::new();
        let mut between = 0;
        for &t in targets {
            if LiveDrag::start(&mut s.puppet, t).is_none() {
                between += 1;
                continue;
            }
            let pts = world_points(s, t);
            kept.push((t, pts.len()));
            base.extend(pts);
        }
        if kept.is_empty() {
            return Err(if between > 0 {
                "The selection sits between keys. Move the parameters onto a key (a \
                 dot on the slider) first, then start the tool again."
            } else {
                "Select one or more meshes, or a warp deformer, first."
            });
        }
        if between > 0 {
            log::warn!(
                "Temporary Deform Tool: {between} object(s) sit between keys and were left out"
            );
        }
        let frame = Frame::new(method, &base, cols, rows);
        let values = (0..s.puppet.model().parameters.len()).map(|i| s.puppet.value(i)).collect();
        Ok(Self { method, cols, rows, prev_tool, targets: kept, base, frame, values, drag: None })
    }

    /// Start again on the same objects with another method or grid size.
    fn rebuild(&mut self, s: &mut Session) {
        if let Some(d) = self.drag.take() {
            for l in d.live.iter().flatten() {
                l.restore(&mut s.puppet);
            }
        }
        let targets: Vec<LiveTarget> = self.targets.iter().map(|t| t.0).collect();
        match Self::bind(s, self.method, &targets, self.cols, self.rows, self.prev_tool) {
            Ok(me) => *self = me,
            Err(e) => log::warn!("Temporary Deform Tool: {e}"),
        }
    }

    fn current(&self, s: &Session) -> Vec<V> {
        self.targets.iter().flat_map(|&(t, _)| world_points(s, t)).collect()
    }

    fn hit(&self, ptr: Pos2, xf: &ViewXf) -> Option<Grab> {
        let near = |p: V| xf.screen(p).distance(ptr) <= 10.0;
        match &self.frame {
            Frame::Quad { now, anchor, .. } => {
                let (edges, stalk_on) = handles(self.method);
                if near(*anchor) {
                    return Some(Grab::Anchor);
                }
                if let Some(i) = (0..4).find(|&i| near(at(now, i))) {
                    return Some(Grab::Corner(i));
                }
                if edges
                    && let Some(i) =
                        (0..4).find(|&i| near((at(now, i) + at(now, (i + 1) % 4)) * 0.5))
                {
                    return Some(Grab::Edge(i));
                }
                if stalk_on && near(stalk(now, xf.px())) {
                    return Some(Grab::Stalk);
                }
                None
            }
            Frame::Grid { now, .. } | Frame::Path { now, .. } => {
                now.iter().position(|&p| near(p)).map(Grab::Point)
            }
        }
    }

    fn drag_to(&mut self, to: V, shift: bool) {
        let Some(d) = &self.drag else { return };
        // Shift: turning snaps the angle, anything else moves straight
        let turning = matches!(d.grab, Grab::Stalk)
            || matches!(d.grab, Grab::Corner(_)) && self.method == Method::Rotation;
        let snap = shift && turning;
        let to =
            if shift && !turning { d.from + crate::point_edit::straight(to - d.from) } else { to };
        let delta = to - d.from;
        match (&mut self.frame, &d.frame) {
            (Frame::Quad { now, anchor, rest }, Frame::Quad { now: q0, anchor: a0, .. }) => {
                let (q, mut a) = quad_drag(self.method, q0, *a0, d.grab, d.mode, d.from, to, snap);
                // the anchor rides with the box as Cubism's does, except
                // where it is the pivot (Rotation, Enlarge/Reduce)
                if d.mode == Mode::Deform
                    && d.grab != Grab::Anchor
                    && matches!(self.method, Method::Free | Method::Perspective | Method::Distort)
                {
                    a = quad_carry(q0, &q, *a0);
                }
                *now = q;
                *anchor = a;
                if d.mode == Mode::Control || d.grab == Grab::Anchor {
                    // the art stays: the box is bound where it now is
                    *rest = q;
                }
            }
            (Frame::Grid { now, .. }, Frame::Grid { now: g0, .. })
            | (Frame::Path { now, .. }, Frame::Path { now: g0, .. }) => {
                let Grab::Point(k) = d.grab else { return };
                if let (Some(p), Some(&p0)) = (now.get_mut(k), g0.get(k)) {
                    *p = p0 + delta;
                }
            }
            _ => {}
        }
    }

    /// Bound world positions now, without the drag's control-only moves.
    fn mapped(&self) -> Vec<V> {
        if let Some(d) = &self.drag
            && d.mode == Mode::Control
        {
            return d.start.clone();
        }
        self.frame.map(&self.base)
    }
}

fn world_points(s: &Session, t: LiveTarget) -> Vec<V> {
    let flat: Option<Vec<f32>> = match t {
        LiveTarget::Mesh(m) => s.puppet.mesh_geometry(m).map(|g| g.positions.to_vec()),
        LiveTarget::Warp(d) => s.puppet.warp_grid(d).map(|(g, _, _)| g.to_vec()),
    };
    flat.unwrap_or_default().as_chunks::<2>().0.iter().map(|q| vec2(q[0], q[1])).collect()
}

/// Shifts per target from `start` to `now` (world units).
fn shifts(targets: &[(LiveTarget, usize)], start: &[V], now: &[V]) -> Vec<Vec<(usize, Vec2)>> {
    let mut at = 0;
    targets
        .iter()
        .map(|&(_, n)| {
            let out = (0..n)
                .filter_map(|i| {
                    let d = *now.get(at + i)? - *start.get(at + i)?;
                    (d.length_sq() > 1e-12).then_some((i, d))
                })
                .collect();
            at += n;
            out
        })
        .collect()
}

impl EditorUi {
    /// The objects the tool works on: the selected warp deformer, else the
    /// selected meshes that are not locked.
    fn temp_deform_targets(&self) -> Vec<LiveTarget> {
        let Some(s) = &self.session else { return Vec::new() };
        match self.selected {
            Some(Sel::Deformer(d)) if s.puppet.warp_grid(d).is_some() => vec![LiveTarget::Warp(d)],
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
        }
    }

    pub(super) fn can_temp_deform(&self) -> bool {
        self.mesh_edit.is_none() && !self.temp_deform_targets().is_empty()
    }

    /// Modeling > Temporary Deform Tool > a method: bind it to the selection.
    pub(super) fn start_temp_deform(&mut self, method: Method) {
        let targets = self.temp_deform_targets();
        let (cols, rows) = (self.tdeform_cols, self.tdeform_rows);
        let prev = match &self.tdeform {
            Some(t) => t.prev_tool,
            None => self.tool,
        };
        let Some(s) = &mut self.session else { return };
        // physics moves its parameters off their keys every frame, which
        // would leave nothing to bind and let go of the tool at once
        if self.physics_on && s.puppet.has_physics() && self.tdeform.is_none() {
            self.physics_on = false;
            super::physics_to_rest(s);
            self.tdeform_paused_physics = true;
            log::info!("Temporary Deform Tool: physics paused while it is bound");
        }
        if let Some(mut old) = self.tdeform.take()
            && let Some(d) = old.drag.take()
        {
            for l in d.live.iter().flatten() {
                l.restore(&mut s.puppet);
            }
        }
        match TempDeform::bind(s, method, &targets, cols, rows, prev) {
            Ok(t) => {
                log::info!("Temporary Deform Tool: {method:?} on {} object(s)", targets.len());
                self.tdeform = Some(t);
                self.tool = Tool::TempDeform;
            }
            Err(e) => {
                log::warn!("Temporary Deform Tool: {e}");
                self.tdeform_error = Some(e);
                self.resume_paused_physics();
            }
        }
    }

    /// Physics comes back on if the tool paused it.
    fn resume_paused_physics(&mut self) {
        if std::mem::take(&mut self.tdeform_paused_physics) {
            self.physics_on = true;
            if let Some(s) = &mut self.session {
                s.puppet.reset_physics();
            }
        }
    }

    /// The reason the tool would not start, until it is read.
    pub(crate) fn temp_deform_error_window(&mut self, ctx: &egui::Context) {
        let Some(e) = self.tdeform_error else { return };
        let mut open = true;
        crate::windows::Dialog::new(tr("Temporary Deform Tool")).width(360.0).open(&mut open).show(
            ctx,
            |ui| {
                ui.label(tr(e));
                ui.add_space(8.0);
                if ui.button(tr("OK")).clicked() {
                    self.tdeform_error = None;
                }
            },
        );
        if !open {
            self.tdeform_error = None;
        }
    }

    /// Release the tool (Deselect the Temporary Deform Tool).
    pub(super) fn end_temp_deform(&mut self) {
        self.resume_paused_physics();
        let Some(mut t) = self.tdeform.take() else { return };
        if let (Some(s), Some(d)) = (&mut self.session, t.drag.take()) {
            for l in d.live.iter().flatten() {
                l.restore(&mut s.puppet);
            }
            s.dirty = true;
        }
        if self.tool == Tool::TempDeform {
            self.tool = t.prev_tool;
        }
    }

    pub(super) fn set_temp_method(&mut self, method: Method) {
        let (Some(t), Some(s)) = (&mut self.tdeform, &mut self.session) else { return };
        if t.method != method {
            log::info!("Temporary Deform Tool: {method:?}");
            t.method = method;
            t.rebuild(s);
        }
    }

    pub(super) fn set_temp_grid(&mut self, cols: usize, rows: usize) {
        self.tdeform_cols = cols;
        self.tdeform_rows = rows;
        let (Some(t), Some(s)) = (&mut self.tdeform, &mut self.session) else { return };
        if t.method == Method::Warp && (t.cols, t.rows) != (cols, rows) {
            t.cols = cols;
            t.rows = rows;
            t.rebuild(s);
        }
    }

    /// The tool on the canvas: controls drawn over the art, drags, the
    /// green check in the corner. Called from the viewport when bound.
    pub(super) fn temp_deform_canvas(
        &mut self,
        ui: &mut Ui,
        resp: &Response,
        view: Rect,
        xf: ViewXf,
        space: bool,
    ) {
        if self.tool != Tool::TempDeform || self.mesh_edit.is_some() {
            self.end_temp_deform();
            return;
        }
        let Some(s) = &mut self.session else {
            self.tdeform = None;
            return;
        };
        let Some(td) = &mut self.tdeform else { return };

        // the green check, top left of the view
        let mut done = ui.ctx().input(|i| i.key_pressed(egui::Key::Escape)) && td.drag.is_none();
        egui::Area::new(egui::Id::new("temp-deform-check"))
            .fixed_pos(view.left_top() + vec2(10.0, 10.0))
            .order(egui::Order::Foreground)
            .show(ui.ctx(), |ui| {
                egui::Frame::popup(ui.style()).show(ui, |ui| {
                    ui.horizontal(|ui| {
                        let check = egui::Button::new(
                            egui::RichText::new(egui_phosphor::regular::CHECK)
                                .color(Color32::WHITE)
                                .size(14.0),
                        )
                        .fill(theme::pal().warp_green);
                        if ui
                            .add(check)
                            .on_hover_text(tr("Deselect the Temporary Deform Tool"))
                            .clicked()
                        {
                            done = true;
                        }
                        ui.label(tr("Temporary Deform Tool"));
                        ui.weak(tr(td.method.label()));
                    });
                });
            });
        if done {
            self.end_temp_deform();
            return;
        }

        // the parameters or the model changed under it: let go
        let values: Vec<f32> =
            (0..s.puppet.model().parameters.len()).map(|i| s.puppet.value(i)).collect();
        if td.drag.is_none() {
            let now = td.current(s);
            let mapped = td.mapped();
            let tol = 0.75 / xf.px();
            let moved = now.len() != mapped.len()
                || now.iter().zip(&mapped).any(|(a, b)| (*a - *b).length() > tol);
            if values != td.values {
                log::info!("Temporary Deform Tool released: the parameters changed");
                self.end_temp_deform();
                return;
            }
            if moved {
                // undo or redo moved the art on this key: stay, with a
                // fresh box around where it is now
                let before = td.targets.len();
                td.rebuild(s);
                if td.targets.len() != before || td.current(s).len() != td.base.len() {
                    log::info!("Temporary Deform Tool released: the objects changed");
                    self.end_temp_deform();
                    return;
                }
                log::debug!(
                    "Temporary Deform Tool: the art moved (undo or redo), box fitted again"
                );
            }
        }

        let (ptr, mods, any_down) =
            ui.ctx().input(|i| (i.pointer.latest_pos(), i.modifiers, i.pointer.any_down()));

        // Temporary Path: clicks add, insert and delete points
        if !space
            && resp.clicked()
            && let Some(p) = resp.interact_pointer_pos()
            && let Frame::Path { now, .. } = &td.frame
        {
            let held = td.mapped();
            let w = xf.world(p);
            let hit = now.iter().position(|&q| xf.screen(q).distance(p) <= 7.0);
            let mut pts = now.clone();
            match hit {
                Some(k) if mods.alt => {
                    pts.remove(k);
                }
                Some(_) => {}
                None => {
                    // on the curve: insert into that span, else add at the end
                    let samples = path_samples(&pts);
                    let on = samples
                        .windows(2)
                        .enumerate()
                        .map(|(i, w2)| {
                            let (a, b) = (xf.screen(at(w2, 0)), xf.screen(at(w2, 1)));
                            let ab = b - a;
                            let t = if ab.length_sq() > 0.0 {
                                ((p - a).dot(ab) / ab.length_sq()).clamp(0.0, 1.0)
                            } else {
                                0.0
                            };
                            (i, (a + ab * t).distance(p))
                        })
                        .filter(|&(_, dist)| dist <= 6.0)
                        .min_by(|a, b| a.1.total_cmp(&b.1));
                    match on {
                        Some((i, _)) => pts.insert(i / SPAN + 1, w),
                        None => pts.push(w),
                    }
                }
            }
            if let Frame::Path { now, .. } = &mut td.frame {
                *now = pts;
            }
            td.base = held.clone();
            td.frame.rebind(&held);
        }

        // a drag only starts once the pointer has gone past the click
        // distance, so the handle is the one where the press began, and
        // the drag counts from there
        let press = ui.ctx().input(|i| i.pointer.press_origin());
        if !space
            && resp.drag_started()
            && let Some(p) = press.or_else(|| resp.interact_pointer_pos())
            && let Some(grab) = td.hit(p, &xf)
            && !(mods.alt && matches!(td.frame, Frame::Path { .. }))
        {
            let mode = if mods.command { Mode::Control } else { Mode::Deform };
            let start = td.mapped();
            let deforms = mode != Mode::Control && grab != Grab::Anchor;
            let live = if deforms {
                td.targets.iter().map(|&(t, _)| LiveDrag::start(&mut s.puppet, t)).collect()
            } else {
                Vec::new()
            };
            td.drag =
                Some(Drag { grab, mode, from: xf.world(p), frame: td.frame.clone(), start, live });
        }

        if td.drag.is_some()
            && resp.dragged()
            && let Some(p) = ptr
        {
            td.drag_to(xf.world(p), mods.shift);
            let now = td.frame.map(&td.base);
            if let Some(d) = &td.drag
                && !d.live.is_empty()
            {
                for (live, sh) in d.live.iter().zip(shifts(&td.targets, &d.start, &now)) {
                    if let Some(live) = live {
                        live.show(&mut s.puppet, &sh, 1.0);
                    }
                }
                s.dirty = true;
            }
        }

        if (resp.drag_stopped() || !any_down)
            && let Some(d) = td.drag.take()
        {
            let now = td.frame.map(&td.base);
            if d.live.is_empty() {
                // a control-only move: the art stays where it was
                td.base = d.start.clone();
                td.frame.rebind(&d.start);
            } else {
                for l in d.live.iter().flatten() {
                    l.restore(&mut s.puppet);
                }
                let plan: super::BrushPlan = d
                    .live
                    .iter()
                    .zip(shifts(&td.targets, &d.start, &now))
                    .filter_map(|(live, sh)| {
                        let live = live.as_ref()?;
                        let moves: Vec<(usize, f32, f32)> = sh
                            .iter()
                            .map(|&(i, v)| {
                                let px = live.local.get(i * 2).copied().unwrap_or(0.0);
                                let py = live.local.get(i * 2 + 1).copied().unwrap_or(0.0);
                                let (dx, dy) =
                                    parent_delta(&s.puppet, live.parent, px, py, v.x, v.y);
                                (i, dx, dy)
                            })
                            .collect();
                        (!moves.is_empty()).then_some((live.target, live.keyform, moves))
                    })
                    .collect();
                if !plan.is_empty() {
                    s.edit(|m| {
                        let mut any = false;
                        for (target, k, moves) in &plan {
                            any |= match target {
                                LiveTarget::Mesh(mi) => {
                                    ayatsuri_core::edit::move_vertices_in(m, *mi, Some(*k), moves)
                                }
                                LiveTarget::Warp(w) => {
                                    ayatsuri_core::edit::move_warp_points(m, *w, Some(*k), moves)
                                }
                            };
                        }
                        any
                    });
                }
                // bind to where the art really landed, so the next drag
                // starts from the model and not from rounding
                s.puppet.update();
                let landed = td.current(s);
                td.base = landed.clone();
                td.frame.rebind(&landed);
                if let Frame::Quad { rest, now, .. } = &mut td.frame {
                    *rest = *now;
                }
            }
            s.dirty = true;
        }

        // the controls
        let vp = ui.painter_at(view);
        let line = Stroke::new(1.2, theme::pal().accent);
        let red = Color32::from_rgb(0xE8, 0x3A, 0x3A);
        let hover = ptr.filter(|p| view.contains(*p)).and_then(|p| td.hit(p, &xf));
        // the box's handles are squares, as Cubism draws them; grid and
        // path points stay round
        let dot = |p: V, g: Grab| {
            let hot = hover == Some(g) || td.drag.as_ref().is_some_and(|d| d.grab == g);
            let at = xf.screen(p);
            if matches!(g, Grab::Corner(_) | Grab::Edge(_) | Grab::Stalk) {
                let r = if hot { 6.0 } else { 4.5 };
                let sq = Rect::from_center_size(at, vec2(r * 2.0, r * 2.0));
                vp.rect_filled(sq, 0.0, red);
                vp.rect_stroke(
                    sq,
                    0.0,
                    Stroke::new(1.0, Color32::from_black_alpha(200)),
                    egui::StrokeKind::Outside,
                );
            } else {
                let r = if hot { 5.0 } else { 3.6 };
                vp.circle_filled(at, r, red);
                vp.circle_stroke(at, r, Stroke::new(1.0, Color32::WHITE));
            }
        };
        if td.drag.is_some() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
        } else if hover.is_some() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        match &td.frame {
            Frame::Quad { now, anchor, .. } => {
                for i in 0..4 {
                    vp.line_segment([xf.screen(at(now, i)), xf.screen(at(now, (i + 1) % 4))], line);
                }
                let (edges, stalk_on) = handles(td.method);
                if stalk_on {
                    let top = (now[0] + now[1]) * 0.5;
                    let st = stalk(now, xf.px());
                    vp.line_segment([xf.screen(top), xf.screen(st)], line);
                    dot(st, Grab::Stalk);
                }
                for i in 0..4 {
                    dot(at(now, i), Grab::Corner(i));
                    if edges {
                        dot((at(now, i) + at(now, (i + 1) % 4)) * 0.5, Grab::Edge(i));
                    }
                }
                let a = xf.screen(*anchor);
                let r = if hover == Some(Grab::Anchor) { 7.0 } else { 5.5 };
                vp.circle_stroke(a, r, Stroke::new(1.5, red));
                vp.line_segment(
                    [a - vec2(r + 3.0, 0.0), a + vec2(r + 3.0, 0.0)],
                    Stroke::new(1.0, red),
                );
                vp.line_segment(
                    [a - vec2(0.0, r + 3.0), a + vec2(0.0, r + 3.0)],
                    Stroke::new(1.0, red),
                );
            }
            Frame::Grid { cols, rows, now, .. } => {
                let pt = |c: usize, r: usize| xf.screen(at(now, r * (cols + 1) + c));
                for r in 0..=*rows {
                    for c in 0..=*cols {
                        if c < *cols {
                            vp.line_segment([pt(c, r), pt(c + 1, r)], line);
                        }
                        if r < *rows {
                            vp.line_segment([pt(c, r), pt(c, r + 1)], line);
                        }
                    }
                }
                for (k, p) in now.iter().enumerate() {
                    dot(*p, Grab::Point(k));
                }
            }
            Frame::Path { now, .. } => {
                let pts: Vec<Pos2> = path_samples(now).iter().map(|&p| xf.screen(p)).collect();
                if pts.len() >= 2 {
                    vp.add(egui::Shape::line(pts, Stroke::new(2.0, theme::pal().accent)));
                }
                for (k, p) in now.iter().enumerate() {
                    dot(*p, Grab::Point(k));
                }
                if now.is_empty() {
                    vp.text(
                        view.center_top() + vec2(0.0, 44.0),
                        Align2::CENTER_TOP,
                        tr("Click to add control points along the art"),
                        FontId::proportional(12.0),
                        theme::pal().text,
                    );
                }
            }
        }
        if td.drag.is_some() || hover.is_some() {
            ui.ctx().request_repaint();
        }
    }

    /// The Tool Details palette while the tool is bound.
    pub(super) fn td_temp_deform(&mut self, ui: &mut Ui) {
        super::tool_details::section(
            ui,
            tr("Transform method"),
            "Drag a control point to deform. Ctrl+drag moves the control point only. Shift+drag moves straight: horizontally, vertically or at 45°. Drag the anchor to move the centre of Rotation and Enlarge/Reduce.\n\nTemporary Path: click to add a control point, click on the path to insert one, Alt+click deletes.\n\nEach drag lands on the keyform the parameters sit on, as one undo step.",
        );
        let current = self.tdeform.as_ref().map(|t| t.method);
        let mut pick = None;
        for m in Method::ALL {
            if ui.radio(current == Some(m), tr(m.label())).clicked() {
                pick = Some(m);
            }
        }
        if let Some(m) = pick {
            self.set_temp_method(m);
        }
        if current == Some(Method::Warp) {
            super::tool_details::section(ui, tr("Number of warp divisions"), "");
            let (mut c, mut r) = (self.tdeform_cols, self.tdeform_rows);
            ui.horizontal(|ui| {
                ui.label(tr("Columns"));
                ui.add(egui::DragValue::new(&mut c).range(1..=20));
                ui.label(tr("Rows"));
                ui.add(egui::DragValue::new(&mut r).range(1..=20));
            });
            if (c, r) != (self.tdeform_cols, self.tdeform_rows) {
                self.set_temp_grid(c, r);
            }
        }
        ui.add_space(8.0);
        if ui.button(tr("Unbind")).clicked() {
            self.end_temp_deform();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: V, b: V) -> bool {
        (a - b).length() < 1e-3
    }

    #[test]
    fn shift_moves_straight() {
        use crate::point_edit::straight;
        // nearly flat goes flat, nearly diagonal goes diagonal
        assert!(close(straight(vec2(10.0, 1.0)), vec2(10.0, 0.0)));
        assert!(close(straight(vec2(1.0, -9.0)), vec2(0.0, -9.0)));
        let d = straight(vec2(5.0, 4.6));
        assert!((d.x - d.y).abs() < 1e-4 && d.x > 4.0);
    }

    #[test]
    fn zoom_scales_about_the_anchor() {
        let q = [vec2(-1.0, -1.0), vec2(1.0, -1.0), vec2(1.0, 1.0), vec2(-1.0, 1.0)];
        let (n, a) = quad_drag(
            Method::Zoom,
            &q,
            V::ZERO,
            Grab::Corner(2),
            Mode::Deform,
            vec2(1.0, 1.0),
            vec2(2.0, 2.0),
            false,
        );
        assert_eq!(a, V::ZERO);
        assert!(close(n[0], vec2(-2.0, -2.0)));
        assert!(close(n[2], vec2(2.0, 2.0)));
    }

    #[test]
    fn rotation_turns_around_the_anchor() {
        let q = [vec2(-1.0, -1.0), vec2(1.0, -1.0), vec2(1.0, 1.0), vec2(-1.0, 1.0)];
        let (n, _) = quad_drag(
            Method::Rotation,
            &q,
            V::ZERO,
            Grab::Corner(1),
            Mode::Deform,
            vec2(1.0, -1.0),
            vec2(1.0, 1.0),
            false,
        );
        // a quarter turn clockwise on screen (y down)
        assert!(close(n[1], vec2(1.0, 1.0)));
        assert!(close(n[0], vec2(1.0, -1.0)));
    }

    #[test]
    fn perspective_moves_the_partner_the_other_way() {
        let q = [vec2(0.0, 0.0), vec2(4.0, 0.0), vec2(4.0, 2.0), vec2(0.0, 2.0)];
        let (n, _) = quad_drag(
            Method::Perspective,
            &q,
            vec2(2.0, 1.0),
            Grab::Corner(0),
            Mode::Deform,
            vec2(0.0, 0.0),
            vec2(1.0, 0.0),
            false,
        );
        assert!(close(n[0], vec2(1.0, 0.0)));
        assert!(close(n[1], vec2(3.0, 0.0)));
        assert!(close(n[2], q[2]) && close(n[3], q[3]));
    }

    #[test]
    fn perspective_side_handle_slides_that_side_along_itself() {
        let q = [vec2(0.0, 0.0), vec2(4.0, 0.0), vec2(4.0, 2.0), vec2(0.0, 2.0)];
        let (n, _) = quad_drag(
            Method::Perspective,
            &q,
            vec2(2.0, 1.0),
            Grab::Edge(0),
            Mode::Deform,
            vec2(2.0, 0.0),
            vec2(3.0, 0.7),
            false,
        );
        // the top slides right; the drag's downward part does nothing
        assert!(close(n[0], vec2(1.0, 0.0)));
        assert!(close(n[1], vec2(5.0, 0.0)));
        assert!(close(n[2], q[2]) && close(n[3], q[3]));
    }

    #[test]
    fn perspective_corner_keeps_to_the_side_the_drag_mostly_goes() {
        let q = [vec2(0.0, 0.0), vec2(4.0, 0.0), vec2(4.0, 2.0), vec2(0.0, 2.0)];
        let drag = |to: V| {
            quad_drag(
                Method::Perspective,
                &q,
                vec2(2.0, 1.0),
                Grab::Corner(0),
                Mode::Deform,
                vec2(0.0, 0.0),
                to,
                false,
            )
            .0
        };
        // mostly sideways: along the top, the top partner mirrors
        let n = drag(vec2(1.0, 0.4));
        assert!(close(n[0], vec2(1.0, 0.0)) && close(n[1], vec2(3.0, 0.0)));
        assert!(close(n[2], q[2]) && close(n[3], q[3]));
        // mostly down: along the side, the side partner mirrors
        let n = drag(vec2(0.2, 0.5));
        assert!(close(n[0], vec2(0.0, 0.5)) && close(n[3], vec2(0.0, 1.5)));
        assert!(close(n[1], q[1]) && close(n[2], q[2]));
    }

    #[test]
    fn the_box_carries_art_bilinearly() {
        let rest = [vec2(0.0, 0.0), vec2(4.0, 0.0), vec2(4.0, 2.0), vec2(0.0, 2.0)];
        // the top narrowed to a quarter of its width
        let now = [vec2(1.5, 0.0), vec2(2.5, 0.0), vec2(4.0, 2.0), vec2(0.0, 2.0)];
        // the middle goes to the corners' average, not a vanishing point
        assert!(close(quad_carry(&rest, &now, vec2(2.0, 1.0)), vec2(2.0, 1.0)));
        // halfway down the left side stays halfway down the new side
        assert!(close(quad_carry(&rest, &now, vec2(0.0, 1.0)), vec2(0.75, 1.0)));
        // corners go to corners
        for i in 0..4 {
            assert!(close(quad_carry(&rest, &now, rest[i]), now[i]), "corner {i}");
        }
        // an unchanged box leaves points where they are
        assert!(close(quad_carry(&rest, &rest, vec2(3.1, 0.7)), vec2(3.1, 0.7)));
    }

    #[test]
    fn free_transform_keeps_the_opposite_corner() {
        let q = [vec2(0.0, 0.0), vec2(4.0, 0.0), vec2(4.0, 2.0), vec2(0.0, 2.0)];
        let (n, _) = quad_drag(
            Method::Free,
            &q,
            vec2(2.0, 1.0),
            Grab::Corner(2),
            Mode::Deform,
            vec2(4.0, 2.0),
            vec2(8.0, 3.0),
            false,
        );
        assert!(close(n[0], q[0]));
        assert!(close(n[1], vec2(8.0, 0.0)));
        assert!(close(n[2], vec2(8.0, 3.0)));
        assert!(close(n[3], vec2(0.0, 3.0)));
    }

    #[test]
    fn control_only_moves_leave_the_art() {
        let pts = vec![vec2(0.0, 0.0), vec2(3.0, 1.0), vec2(1.0, 2.0)];
        for method in [Method::Distort, Method::Warp] {
            let mut f = Frame::new(method, &pts, 3, 3);
            let held = f.map(&pts);
            match &mut f {
                Frame::Quad { now, .. } => now[1] += vec2(0.7, -0.4),
                Frame::Grid { now, .. } => now[5] += vec2(0.3, 0.2),
                Frame::Path { .. } => {}
            }
            f.rebind(&held);
            let base = held.clone();
            for (a, b) in f.map(&base).iter().zip(&held) {
                assert!(close(*a, *b), "{method:?}");
            }
        }
    }

    #[test]
    fn warp_point_drag_moves_nearby_art_only() {
        let pts: Vec<V> = (0..=10).map(|i| vec2(i as f32, 0.0)).collect();
        let mut f = Frame::new(Method::Warp, &pts, 2, 1);
        if let Frame::Grid { now, .. } = &mut f {
            // the top-left grid point
            now[0] += vec2(0.0, -2.0);
        }
        let out = f.map(&pts);
        assert!(out[0].y < -0.5);
        assert!(close(out[10], pts[10]));
    }

    #[test]
    fn path_follows_its_points() {
        let pts: Vec<V> = (0..=4).map(|i| vec2(i as f32, 0.5)).collect();
        let mut f = Frame::new(Method::Path, &pts, 3, 3);
        if let Frame::Path { now, .. } = &mut f {
            *now = vec![vec2(0.0, 0.0), vec2(4.0, 0.0)];
        }
        f.rebind(&pts);
        assert!(f.map(&pts).iter().zip(&pts).all(|(a, b)| close(*a, *b)));
        if let Frame::Path { now, .. } = &mut f {
            now[1] = vec2(0.0, 4.0);
        }
        // the path turned a quarter about its root: so did the art
        let out = f.map(&pts);
        assert!(close(out[4], vec2(-0.5, 4.0)), "{:?}", out[4]);
    }
}
