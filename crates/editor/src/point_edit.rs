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
    /// A brush selection's per-point weights; empty when there is none.
    /// While present, every weighted point moves by its weight and the box
    /// wraps them.
    pub weights: Vec<f32>,
}

pub struct PtOpts {
    /// Show the box around every point when nothing is picked (form
    /// editing transforms the whole object, like Cubism).
    pub whole_box: bool,
    /// Dragging empty space draws a selection rectangle.
    pub marquee: bool,
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
            drag_started: resp.drag_started(),
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
                    if r.contains(*p) {
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
                draw_box(vp, r.expand(pad), st.drag.as_ref().map(|d| d.grab));
            }
        }
        None => {
            if let Some(r) = st.active_box(pts, whole, pad) {
                draw_box(vp, r, hot);
            }
        }
    }
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
        let opts = PtOpts { whole_box: true, marquee: true, reach: 8.0, pad: 0.0 };
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
}
