//! Stroke mesh-mapping (Cubism editor manual, Mesh Edit: "Stroke
//! mesh-mapping (beta)"): a mesh built along a dragged stroke. The stroke is
//! a smooth curve through its control points; stations are set along it at
//! the repeat spacing, half-stations between them (and half a step past
//! each end), and the mesh width, which width handles can widen or narrow
//! along the way, is split as the manual's pictures show:
//!
//! ```text
//!  1: points on the stroke    2: zigzag             3: diamonds
//!                                t0───t1───t2           t0───t1───t2
//!  ·─·─·─·─·─·─·─·              ╱ ╲  ╱ ╲  ╱ ╲          ╱ ╲  ╱ ╲  ╱ ╲
//!  (no triangles)             b0───b1───b2───b3      m0───m1───m2───m3
//!                                                      ╲ ╱  ╲ ╱  ╲ ╱
//!                                                       b0───b1───b2
//! ```
//!
//! Everything here works in one plane (the editor uses canvas pixels) on
//! plain points; the editor maps the result onto the art.

/// A point in the plane.
pub type P = [f32; 2];

/// How the mesh is laid out.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Settings {
    /// From the stroke to each side of the mesh.
    pub width: f32,
    /// The spacing of stations along the stroke.
    pub density: f32,
    /// How the width is split: 1 (points on the stroke, no triangles),
    /// 2 (a zigzag between the sides) or 3 (both sides and the stroke).
    pub across: u8,
    /// With 2 across: the other side takes the half-stations, which flips
    /// the first and last edges.
    pub invert: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings { width: 10.0, density: 20.0, across: 2, invert: false }
    }
}

/// One stroke: its control points and its width handles, each `(t, scale)`
/// with `t` from 0 (start) to 1 (end) along the stroke and the width there
/// `scale` times the settings' width. Between handles the scale runs
/// straight; with none it is 1.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Stroke {
    pub points: Vec<P>,
    pub widths: Vec<(f32, f32)>,
}

impl Stroke {
    /// A stroke through `points` with a width handle at each end.
    pub fn new(points: Vec<P>) -> Self {
        Stroke { points, widths: vec![(0.0, 1.0), (1.0, 1.0)] }
    }

    /// The width scale at `t`.
    pub fn scale_at(&self, t: f32) -> f32 {
        let mut w: Vec<(f32, f32)> = self.widths.clone();
        w.sort_by(|a, b| a.0.total_cmp(&b.0));
        let (Some(first), Some(last)) = (w.first(), w.last()) else { return 1.0 };
        if t <= first.0 {
            return first.1;
        }
        if t >= last.0 {
            return last.1;
        }
        for pair in w.windows(2) {
            if let [a, b] = pair
                && t >= a.0
                && t <= b.0
            {
                let f = if b.0 > a.0 { (t - a.0) / (b.0 - a.0) } else { 0.0 };
                return a.1 + (b.1 - a.1) * f;
            }
        }
        1.0
    }
}

fn add(a: P, b: P) -> P {
    [a[0] + b[0], a[1] + b[1]]
}
fn sub(a: P, b: P) -> P {
    [a[0] - b[0], a[1] - b[1]]
}
fn mul(a: P, k: f32) -> P {
    [a[0] * k, a[1] * k]
}
fn len(a: P) -> f32 {
    a[0].hypot(a[1])
}
fn dist(a: P, b: P) -> f32 {
    len(sub(a, b))
}

/// Samples per curve segment between two control points.
const STEPS: usize = 12;

/// The smooth curve through the control points (Catmull-Rom), as a dense
/// polyline from the first point to the last.
pub fn curve(points: &[P]) -> Vec<P> {
    let n = points.len();
    let at = |i: isize| -> Option<P> {
        let i = i.clamp(0, n as isize - 1) as usize;
        points.get(i).copied()
    };
    let mut out = Vec::new();
    if n < 2 {
        out.extend(points.first().copied());
        return out;
    }
    for i in 0..n - 1 {
        let i = i as isize;
        let (Some(p0), Some(p1), Some(p2), Some(p3)) = (at(i - 1), at(i), at(i + 1), at(i + 2))
        else {
            continue;
        };
        for s in 0..STEPS {
            let t = s as f32 / STEPS as f32;
            let (t2, t3) = (t * t, t * t * t);
            let c = |k: usize| {
                let (a, b, c, d) = (
                    p0.get(k).copied().unwrap_or(0.0),
                    p1.get(k).copied().unwrap_or(0.0),
                    p2.get(k).copied().unwrap_or(0.0),
                    p3.get(k).copied().unwrap_or(0.0),
                );
                0.5 * (2.0 * b
                    + (c - a) * t
                    + (2.0 * a - 5.0 * b + 4.0 * c - d) * t2
                    + (3.0 * b - a - 3.0 * c + d) * t3)
            };
            out.push([c(0), c(1)]);
        }
    }
    out.extend(points.last().copied());
    out
}

/// A point on the stroke where the mesh puts a row of vertices.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Station {
    pub at: P,
    /// Unit normal, to the left of the stroke's direction.
    pub normal: P,
    /// From the stroke to each side here.
    pub half: f32,
}

/// The polyline's length up to each of its points.
fn lengths(line: &[P]) -> Vec<f32> {
    let mut acc = 0.0;
    let mut out = Vec::with_capacity(line.len());
    let mut prev = line.first().copied();
    for &p in line {
        if let Some(q) = prev {
            acc += dist(p, q);
        }
        out.push(acc);
        prev = Some(p);
    }
    out
}

/// The point at distance `d` along the polyline, and the direction there.
fn point_at(line: &[P], acc: &[f32], d: f32) -> Option<(P, P)> {
    let i = acc.partition_point(|&a| a < d).clamp(1, line.len().checked_sub(1)?);
    let (a, b) = (*line.get(i - 1)?, *line.get(i)?);
    let (la, lb) = (*acc.get(i - 1)?, *acc.get(i)?);
    let f = if lb > la { ((d - la) / (lb - la)).clamp(0.0, 1.0) } else { 0.0 };
    Some((add(a, mul(sub(b, a), f)), sub(b, a)))
}

/// The stroke as a polyline with its running lengths, its length and the
/// spacing that fits it with whole steps (at least one).
fn layout(stroke: &Stroke, s: &Settings) -> Option<(Vec<P>, Vec<f32>, f32, usize)> {
    let line = curve(&stroke.points);
    let acc = lengths(&line);
    let total = acc.last().copied().unwrap_or(0.0);
    if line.len() < 2 || total <= 1e-4 {
        return None;
    }
    let segments = ((total / s.density.max(0.5)).round() as usize).max(1);
    Some((line, acc, total, segments))
}

/// The station `d` along the stroke; past either end it carries on
/// straight along the end's direction.
fn station_at(
    stroke: &Stroke,
    s: &Settings,
    line: &[P],
    acc: &[f32],
    total: f32,
    d: f32,
) -> Option<Station> {
    let on = d.clamp(0.0, total);
    let (at, dir) = point_at(line, acc, on)?;
    let l = len(dir);
    let unit = if l > 1e-6 { [dir[0] / l, dir[1] / l] } else { [1.0, 0.0] };
    Some(Station {
        at: add(at, mul(unit, d - on)),
        normal: [-unit[1], unit[0]],
        half: s.width.max(0.0) * stroke.scale_at(on / total),
    })
}

/// The stations along a stroke: evenly spread at about the settings'
/// spacing, the first and last on the stroke's ends.
pub fn stations(stroke: &Stroke, s: &Settings) -> Vec<Station> {
    let Some((line, acc, total, n)) = layout(stroke, s) else { return Vec::new() };
    let step = total / n as f32;
    (0..=n).filter_map(|k| station_at(stroke, s, &line, &acc, total, k as f32 * step)).collect()
}

/// The half-stations: one before the first station, one between each two,
/// one after the last, each half a step from its neighbours.
pub fn half_stations(stroke: &Stroke, s: &Settings) -> Vec<Station> {
    let Some((line, acc, total, n)) = layout(stroke, s) else { return Vec::new() };
    let step = total / n as f32;
    (0..=n + 1)
        .filter_map(|k| station_at(stroke, s, &line, &acc, total, (k as f32 - 0.5) * step))
        .collect()
}

/// The mesh of one stroke: its points and triangles (indices into the
/// points). With 1 across there are points only.
pub fn build(stroke: &Stroke, s: &Settings) -> (Vec<P>, Vec<[u16; 3]>) {
    let (st, half) = (stations(stroke, s), half_stations(stroke, s));
    let mut pts: Vec<P> = Vec::new();
    let mut tris: Vec<[u16; 3]> = Vec::new();
    let n = st.len();
    if n < 2 || half.len() != n + 1 {
        return (pts, tris);
    }
    let side = |x: &Station, k: f32| add(x.at, mul(x.normal, x.half * k));
    match s.across {
        1 => {
            // every station and the half-station after it, on the stroke
            for (i, x) in st.iter().enumerate() {
                pts.push(x.at);
                if i + 1 < n
                    && let Some(h) = half.get(i + 1)
                {
                    pts.push(h.at);
                }
            }
        }
        3 => {
            // stations on both sides, half-stations on the stroke between
            let (t, m, b) = (0u16, n as u16, (2 * n + 1) as u16);
            pts.extend(st.iter().map(|x| side(x, 1.0)));
            pts.extend(half.iter().map(|x| x.at));
            pts.extend(st.iter().map(|x| side(x, -1.0)));
            for k in 0..n as u16 {
                if k + 1 < n as u16 {
                    tris.push([t + k, t + k + 1, m + k + 1]);
                    tris.push([b + k, m + k + 1, b + k + 1]);
                }
                tris.push([m + k, t + k, m + k + 1]);
                tris.push([m + k, m + k + 1, b + k]);
            }
        }
        _ => {
            // stations on one side, half-stations on the other, taking
            // turns: a strip of triangles on three points in a row
            let (on, off) = if s.invert { (-1.0, 1.0) } else { (1.0, -1.0) };
            for (k, h) in half.iter().enumerate() {
                pts.push(side(h, off));
                if let Some(x) = st.get(k) {
                    pts.push(side(x, on));
                }
            }
            for i in 0..pts.len().saturating_sub(2) as u16 {
                tris.push([i, i + 1, i + 2]);
            }
        }
    }
    (pts, tris)
}

/// Every stroke's mesh together, strokes not sharing points.
pub fn build_all(strokes: &[Stroke], s: &Settings) -> (Vec<P>, Vec<[u16; 3]>) {
    let mut pts: Vec<P> = Vec::new();
    let mut tris: Vec<[u16; 3]> = Vec::new();
    for stroke in strokes {
        let (p, t) = build(stroke, s);
        let base = pts.len() as u16;
        pts.extend(p);
        tris.extend(t.into_iter().map(|t| t.map(|i| i + base)));
    }
    (pts, tris)
}

fn inside_tri(p: P, a: P, b: P, c: P) -> bool {
    let o = |p: P, q: P, r: P| (q[0] - p[0]) * (r[1] - p[1]) - (q[1] - p[1]) * (r[0] - p[0]);
    let (d1, d2, d3) = (o(a, b, p), o(b, c, p), o(c, a, p));
    let neg = d1 < 0.0 || d2 < 0.0 || d3 < 0.0;
    let pos = d1 > 0.0 || d2 > 0.0 || d3 > 0.0;
    !(neg && pos)
}

/// Whether `p` lies on the mesh `pts` / `tris`.
pub fn covers(pts: &[P], tris: &[[u16; 3]], p: P) -> bool {
    tris.iter().any(|t| {
        let g = |i: u16| pts.get(i as usize).copied();
        match (g(t[0]), g(t[1]), g(t[2])) {
            (Some(a), Some(b), Some(c)) => inside_tri(p, a, b, c),
            _ => false,
        }
    })
}

/// What stays of the existing mesh under a new stroke mesh: with
/// `maintain` everything; without it, the points the stroke mesh covers go,
/// with the triangles that use them or sit on it. Returns, per old point,
/// whether it stays, and the old triangles that stay.
pub fn keep_existing(
    old_pts: &[P],
    old_tris: &[[u16; 3]],
    new_pts: &[P],
    new_tris: &[[u16; 3]],
    maintain: bool,
) -> (Vec<bool>, Vec<[u16; 3]>) {
    if maintain {
        return (vec![true; old_pts.len()], old_tris.to_vec());
    }
    let keep: Vec<bool> = old_pts.iter().map(|&p| !covers(new_pts, new_tris, p)).collect();
    let tris = old_tris
        .iter()
        .copied()
        .filter(|t| {
            let kept = t.iter().all(|&i| keep.get(i as usize).copied().unwrap_or(false));
            let g = |i: u16| old_pts.get(i as usize).copied().unwrap_or([0.0, 0.0]);
            let centre = mul(add(add(g(t[0]), g(t[1])), g(t[2])), 1.0 / 3.0);
            kept && !covers(new_pts, new_tris, centre)
        })
        .collect();
    (keep, tris)
}

/// A drag's samples thinned to control points: the corners that matter
/// within `tolerance` (Ramer-Douglas-Peucker), ends always kept.
pub fn simplify(samples: &[P], tolerance: f32) -> Vec<P> {
    fn seg_dist(p: P, a: P, b: P) -> f32 {
        let ab = sub(b, a);
        let l2 = ab[0] * ab[0] + ab[1] * ab[1];
        if l2 <= 1e-12 {
            return dist(p, a);
        }
        let t = (((p[0] - a[0]) * ab[0] + (p[1] - a[1]) * ab[1]) / l2).clamp(0.0, 1.0);
        dist(p, add(a, mul(ab, t)))
    }
    let n = samples.len();
    if n <= 2 {
        return samples.to_vec();
    }
    let mut keep = vec![false; n];
    if let Some(f) = keep.first_mut() {
        *f = true;
    }
    if let Some(l) = keep.last_mut() {
        *l = true;
    }
    let mut todo = vec![(0usize, n - 1)];
    while let Some((a, b)) = todo.pop() {
        let (Some(&pa), Some(&pb)) = (samples.get(a), samples.get(b)) else { continue };
        let mut worst = (0usize, 0.0f32);
        for i in a + 1..b {
            let d = samples.get(i).map_or(0.0, |&p| seg_dist(p, pa, pb));
            if d > worst.1 {
                worst = (i, d);
            }
        }
        if worst.1 > tolerance {
            if let Some(k) = keep.get_mut(worst.0) {
                *k = true;
            }
            todo.push((a, worst.0));
            todo.push((worst.0, b));
        }
    }
    samples.iter().zip(&keep).filter(|(_, k)| **k).map(|(p, _)| *p).collect()
}

/// Where a point lands on a stroke: the `t` (0 to 1) of the nearest spot on
/// its curve and how far away that is.
pub fn nearest_on(stroke: &Stroke, p: P) -> Option<(f32, f32)> {
    let line = curve(&stroke.points);
    let acc = lengths(&line);
    let total = acc.last().copied().filter(|t| *t > 1e-6)?;
    let mut best: Option<(f32, f32)> = None;
    for (i, pair) in line.windows(2).enumerate() {
        let [a, b] = pair else { continue };
        let ab = sub(*b, *a);
        let l2 = (ab[0] * ab[0] + ab[1] * ab[1]).max(1e-12);
        let f = (((p[0] - a[0]) * ab[0] + (p[1] - a[1]) * ab[1]) / l2).clamp(0.0, 1.0);
        let d = dist(p, add(*a, mul(ab, f)));
        let along = acc.get(i).copied().unwrap_or(0.0) + f * l2.sqrt();
        if best.is_none_or(|(_, bd)| d < bd) {
            best = Some((along / total, d));
        }
    }
    best
}

/// The point at `t` (0 to 1) along a stroke's curve, and its normal.
pub fn point_on(stroke: &Stroke, t: f32) -> Option<(P, P)> {
    let line = curve(&stroke.points);
    let acc = lengths(&line);
    let total = acc.last().copied()?;
    let (at, dir) = point_at(&line, &acc, t.clamp(0.0, 1.0) * total)?;
    let l = len(dir);
    Some((at, if l > 1e-6 { [-dir[1] / l, dir[0] / l] } else { [0.0, 1.0] }))
}

/// The control point a click at `t` along the stroke goes in front of, so
/// a point added on the line keeps the order of the curve.
pub fn insert_at(stroke: &Stroke, t: f32) -> usize {
    let line = curve(&stroke.points);
    let acc = lengths(&line);
    let total = acc.last().copied().unwrap_or(0.0).max(1e-6);
    // control point k sits at sample k * STEPS
    (1..stroke.points.len())
        .find(|&k| acc.get(k * STEPS).copied().unwrap_or(total) / total >= t)
        .unwrap_or(stroke.points.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn straight(len: f32) -> Stroke {
        Stroke::new(vec![[0.0, 0.0], [len, 0.0]])
    }

    #[test]
    fn the_curve_passes_through_its_points() {
        let pts = [[0.0, 0.0], [10.0, 5.0], [20.0, 0.0]];
        let c = curve(&pts);
        for p in pts {
            assert!(c.iter().any(|q| dist(*q, p) < 1e-4), "{p:?}");
        }
        assert_eq!(c.first(), Some(&[0.0, 0.0]));
        assert_eq!(c.last(), Some(&[20.0, 0.0]));
    }

    #[test]
    fn stations_follow_the_spacing() {
        let s = Settings { width: 5.0, density: 20.0, across: 2, invert: false };
        let st = stations(&straight(100.0), &s);
        assert_eq!(st.len(), 6);
        assert!((st[1].at[0] - 20.0).abs() < 1e-3);
        assert!(st.iter().all(|x| (x.half - 5.0).abs() < 1e-5));
    }

    #[test]
    fn two_across_zigzags_between_the_sides() {
        let s = Settings { width: 5.0, density: 50.0, across: 2, invert: false };
        let (p, t) = build(&straight(100.0), &s);
        // 3 stations on the left, 4 half-stations on the right
        assert_eq!(p.len(), 7);
        assert_eq!(t.len(), 5);
        // left of a stroke going +x is +y here (normal (-dy, dx)); the
        // half-stations start half a step before the stroke
        assert_eq!(p[0], [-25.0, -5.0]);
        assert_eq!(p[1], [0.0, 5.0]);
        assert_eq!(p[6], [125.0, -5.0]);
        assert_eq!(t[0], [0, 1, 2]);
        // invert swaps the sides
        let (q, _) = build(&straight(100.0), &Settings { invert: true, ..s });
        assert_eq!((q[0], q[1]), ([-25.0, 5.0], [0.0, -5.0]));
    }

    #[test]
    fn one_across_is_points_on_the_stroke() {
        let s = Settings { width: 5.0, density: 25.0, across: 1, invert: false };
        let (p, t) = build(&straight(100.0), &s);
        // 5 stations and the 4 half-stations between them
        assert_eq!((p.len(), t.len()), (9, 0));
        assert_eq!(p[1], [12.5, 0.0]);
        assert!(p.iter().all(|q| q[1] == 0.0));
    }

    #[test]
    fn three_across_makes_diamonds() {
        let s = Settings { width: 5.0, density: 25.0, across: 3, invert: false };
        let (p, t) = build(&straight(100.0), &s);
        // 5 stations each side, 6 half-stations on the stroke
        assert_eq!((p.len(), t.len()), (16, 18));
        assert_eq!(p[0], [0.0, 5.0]);
        assert_eq!(p[5], [-12.5, 0.0]);
        assert_eq!(p[10], [112.5, 0.0]);
        assert_eq!(p[11], [0.0, -5.0]);
        // every triangle has a corner on the stroke and one on a side
        for tri in &t {
            assert!(tri.iter().any(|&i| (5..11).contains(&i)), "{tri:?}");
            assert!(tri.iter().any(|&i| !(5..11).contains(&i)), "{tri:?}");
        }
    }

    #[test]
    fn width_handles_taper_the_mesh() {
        let mut k = straight(100.0);
        k.widths = vec![(0.0, 2.0), (1.0, 0.0)];
        let s = Settings { width: 5.0, density: 50.0, across: 2, invert: false };
        let st = stations(&k, &s);
        assert_eq!(st.iter().map(|x| x.half).collect::<Vec<_>>(), vec![10.0, 5.0, 0.0]);
        assert_eq!(straight(1.0).scale_at(0.5), 1.0);
    }

    #[test]
    fn the_stroke_takes_the_points_it_covers_unless_kept() {
        let s = Settings { width: 5.0, density: 50.0, across: 2, invert: false };
        let (np, nt) = build(&straight(100.0), &s);
        let old = [[50.0, 0.0], [50.0, 50.0], [0.0, 50.0]];
        let (keep, tris) = keep_existing(&old, &[[0, 1, 2]], &np, &nt, false);
        assert_eq!(keep, vec![false, true, true]);
        assert!(tris.is_empty());
        let (keep, tris) = keep_existing(&old, &[[0, 1, 2]], &np, &nt, true);
        assert_eq!(keep, vec![true; 3]);
        assert_eq!(tris.len(), 1);
    }

    #[test]
    fn simplify_keeps_the_corner() {
        let mut samples: Vec<P> = (0..=10).map(|i| [i as f32, 0.0]).collect();
        samples.extend((1..=10).map(|i| [10.0, i as f32]));
        assert_eq!(simplify(&samples, 0.5), vec![[0.0, 0.0], [10.0, 0.0], [10.0, 10.0]]);
    }

    #[test]
    fn points_on_the_line() {
        let k = Stroke::new(vec![[0.0, 0.0], [50.0, 0.0], [100.0, 0.0]]);
        let (t, d) = nearest_on(&k, [25.0, 3.0]).unwrap_or_default();
        assert!((t - 0.25).abs() < 1e-3 && (d - 3.0).abs() < 1e-3);
        assert_eq!(insert_at(&k, 0.25), 1);
        assert_eq!(insert_at(&k, 0.75), 2);
        let (p, _) = point_on(&k, 0.5).unwrap_or_default();
        assert!((p[0] - 50.0).abs() < 1e-3);
    }
}
