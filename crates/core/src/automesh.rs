//! Outline-following automatic meshing, the way Cubism's Automatic Mesh
//! generator works: points along the artwork's contour (pushed out by a
//! margin), a second ring a margin inside the art's edge, points down the
//! middle of strokes too thin for that ring, an even lattice deeper in,
//! then Delaunay triangulation with the outline forced in and everything
//! outside it carved away, so concave outlines and holes stay open.
//!
//! The mask is worked on in coarse cells rather than pixels, so a 4096px
//! layer costs about as much as a 400px one.

/// Knobs, named after Cubism's dialog, all in texture pixels. Cubism gives
/// its margins for a 1024px texture and scales them with the texture's
/// size; the caller does that.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Params {
    /// Vertices interval (outside): between points along the outline.
    pub outline_spacing: f32,
    /// Vertices interval (inside): between points inside the shape.
    pub inner_spacing: f32,
    /// Boundary margin (outside): how far the outline sits off the art.
    pub margin: f32,
    /// Boundary margin (inside): how far in from the art's edge the inner
    /// ring runs; 0 leaves the ring out.
    pub inner_margin: f32,
    /// Minimum boundary margin: where the outline would cut a corner closer
    /// to the art than this, a point goes in to hold it off.
    pub min_margin: f32,
    /// Minimum number of points on a boundary: the fewest a ring gets.
    pub min_points: usize,
    /// Alpha value determined as transparent: alpha above it is art.
    pub threshold: u8,
    /// How the inside is filled.
    pub style: Style,
}

/// How a mesh is laid out inside its outline.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Style {
    /// Cubism's layout: the outline ring, an inner ring, a middle row in
    /// thin strokes and an even lattice deeper in.
    #[default]
    Lattice,
    /// A face as riggers build it: the outline ring outside the art at the
    /// margin, a second ring hugging the drawn edge just inside it, set
    /// between the outline points so the strip between them zigzags, and an
    /// even lattice inside at the inner spacing. The drawn edge stays in
    /// that narrow strip when warps bend the face.
    Face,
}

impl Default for Params {
    /// Cubism's Standard preset.
    fn default() -> Self {
        Self {
            outline_spacing: 50.0,
            inner_spacing: 50.0,
            margin: 14.0,
            inner_margin: 14.0,
            min_margin: 5.0,
            min_points: 10,
            threshold: 0,
            style: Style::Lattice,
        }
    }
}

/// The result, in the alpha image's pixel coordinates.
#[derive(Debug, Default)]
pub struct Mesh {
    pub points: Vec<(f32, f32)>,
    pub triangles: Vec<[u16; 3]>,
}

/// A coarse occupancy grid with a one-cell empty border, so every contour
/// closes inside it.
struct Grid {
    w: usize,
    h: usize,
    cell: f32,
    on: Vec<bool>,
}

impl Grid {
    fn at(&self, x: i64, y: i64) -> bool {
        if x < 0 || y < 0 || x as usize >= self.w || y as usize >= self.h {
            return false;
        }
        self.on.get(y as usize * self.w + x as usize).copied().unwrap_or(false)
    }

    /// Whether a point in pixel space lies on the shape.
    fn contains(&self, px: f32, py: f32) -> bool {
        let gx = (px / self.cell + 1.0).floor() as i64;
        let gy = (py / self.cell + 1.0).floor() as i64;
        self.at(gx, gy)
    }

    /// The same grid with other cells on.
    fn with(&self, on: Vec<bool>) -> Grid {
        Grid { w: self.w, h: self.h, cell: self.cell, on }
    }

    /// The shape grown by `px`, the outermost ring kept empty so contours
    /// still close.
    fn grown(&self, px: f32) -> Grid {
        let d = chamfer(&self.on, self.w, self.h);
        let reach = (px / self.cell).max(0.0);
        let on = (0..self.on.len())
            .map(|i| {
                let (x, y) = (i % self.w, i / self.w);
                let border = x == 0 || y == 0 || x + 1 == self.w || y + 1 == self.h;
                !border && d.get(i).is_some_and(|&v| v <= reach)
            })
            .collect();
        self.with(on)
    }

    /// Per cell, how deep inside the shape it is, in pixels (0 outside).
    fn depth(&self) -> Vec<f32> {
        let empty: Vec<bool> = self.on.iter().map(|o| !o).collect();
        // a cell next to an empty one has its centre half a cell in
        chamfer(&empty, self.w, self.h).iter().map(|d| ((d - 0.5) * self.cell).max(0.0)).collect()
    }

    /// The shape shrunk by `px`.
    fn shrunk(&self, px: f32) -> Grid {
        let on = self.depth().iter().map(|&d| d > px).collect();
        self.with(on)
    }
}

/// Distance in cells from each cell to the nearest `seed` cell, by two
/// chamfer passes.
fn chamfer(seed: &[bool], gw: usize, gh: usize) -> Vec<f32> {
    let big = f32::MAX / 4.0;
    let mut d: Vec<f32> = seed.iter().map(|&o| if o { 0.0 } else { big }).collect();
    let (a, b) = (1.0f32, std::f32::consts::SQRT_2);
    let get = |d: &Vec<f32>, x: usize, y: usize| d.get(y * gw + x).copied().unwrap_or(big);
    for y in 0..gh {
        for x in 0..gw {
            let mut v = get(&d, x, y);
            if x > 0 {
                v = v.min(get(&d, x - 1, y) + a);
            }
            if y > 0 {
                v = v.min(get(&d, x, y - 1) + a);
                if x > 0 {
                    v = v.min(get(&d, x - 1, y - 1) + b);
                }
                if x + 1 < gw {
                    v = v.min(get(&d, x + 1, y - 1) + b);
                }
            }
            if let Some(s) = d.get_mut(y * gw + x) {
                *s = v;
            }
        }
    }
    for y in (0..gh).rev() {
        for x in (0..gw).rev() {
            let mut v = get(&d, x, y);
            if x + 1 < gw {
                v = v.min(get(&d, x + 1, y) + a);
            }
            if y + 1 < gh {
                v = v.min(get(&d, x, y + 1) + a);
                if x + 1 < gw {
                    v = v.min(get(&d, x + 1, y + 1) + b);
                }
                if x > 0 {
                    v = v.min(get(&d, x - 1, y + 1) + b);
                }
            }
            if let Some(s) = d.get_mut(y * gw + x) {
                *s = v;
            }
        }
    }
    d
}

/// The art's cells: on where any pixel in the cell is art.
fn art_grid(alpha: &[u8], w: usize, h: usize, cell: f32, threshold: u8) -> Grid {
    let gw = (w as f32 / cell).ceil() as usize + 2;
    let gh = (h as f32 / cell).ceil() as usize + 2;
    let mut on = vec![false; gw * gh];
    for y in 0..h {
        let gy = (y as f32 / cell) as usize + 1;
        for x in 0..w {
            if alpha.get(y * w + x).copied().unwrap_or(0) > threshold {
                let gx = (x as f32 / cell) as usize + 1;
                if let Some(c) = on.get_mut(gy * gw + gx) {
                    *c = true;
                }
            }
        }
    }
    Grid { w: gw, h: gh, cell, on }
}

/// A point in doubled grid units, and a directed contour edge between two.
type Node = (i64, i64);
type Edge = (Node, Node);

/// Marching squares over the grid's cell centres: closed loops in pixel
/// space, outer outlines and holes alike.
fn contours(g: &Grid) -> Vec<Vec<(f32, f32)>> {
    // edges keyed by their start point (doubled coordinates keep them
    // integral), each pointing at its end
    let mut next: std::collections::HashMap<(i64, i64), Vec<(i64, i64)>> =
        std::collections::HashMap::new();
    for y in -1..g.h as i64 {
        for x in -1..g.w as i64 {
            let tl = g.at(x, y);
            let tr = g.at(x + 1, y);
            let br = g.at(x + 1, y + 1);
            let bl = g.at(x, y + 1);
            // midpoints of the square's sides, in doubled grid units
            let t = (x * 2 + 2, y * 2 + 1);
            let r = (x * 2 + 3, y * 2 + 2);
            let b = (x * 2 + 2, y * 2 + 3);
            let l = (x * 2 + 1, y * 2 + 2);
            let case = (tl as u8) << 3 | (tr as u8) << 2 | (br as u8) << 1 | bl as u8;
            // oriented so the shape is always on the left
            let segs: &[Edge] = match case {
                1 => &[(b, l)],
                2 => &[(r, b)],
                3 => &[(r, l)],
                4 => &[(t, r)],
                5 => &[(t, l), (b, r)],
                6 => &[(t, b)],
                7 => &[(t, l)],
                8 => &[(l, t)],
                9 => &[(b, t)],
                10 => &[(l, b), (r, t)],
                11 => &[(r, t)],
                12 => &[(l, r)],
                13 => &[(b, r)],
                14 => &[(l, b)],
                _ => &[],
            };
            for &(a, z) in segs {
                next.entry(a).or_default().push(z);
            }
        }
    }
    let mut loops = Vec::new();
    let starts: Vec<(i64, i64)> = next.keys().copied().collect();
    for s in starts {
        while let Some(first) = next.get_mut(&s).and_then(|v| v.pop()) {
            let mut pts = vec![s];
            let mut cur = first;
            let mut guard = 0usize;
            while cur != s && guard < 1_000_000 {
                pts.push(cur);
                let Some(n) = next.get_mut(&cur).and_then(|v| v.pop()) else { break };
                cur = n;
                guard += 1;
            }
            if pts.len() >= 3 {
                // grid cell gx covers pixels (gx - 1) * cell up to gx * cell,
                // the border cell shifting everything by one
                loops.push(
                    pts.iter()
                        .map(|&(x, y)| {
                            ((x as f32 / 2.0 - 1.0) * g.cell, (y as f32 / 2.0 - 1.0) * g.cell)
                        })
                        .collect(),
                );
            }
        }
    }
    loops
}

/// Ease the stair steps marching squares leaves on a coarse grid: each point
/// moves toward the average of its neighbours, a few passes.
fn smooth(loop_: &[(f32, f32)], passes: usize) -> Vec<(f32, f32)> {
    let n = loop_.len();
    let mut cur = loop_.to_vec();
    if n < 5 {
        return cur;
    }
    for _ in 0..passes {
        let prev = cur.clone();
        for (i, slot) in cur.iter_mut().enumerate() {
            let mut sx = 0.0;
            let mut sy = 0.0;
            for k in [n - 2, n - 1, 0, 1, 2] {
                let p = prev.get((i + k) % n).copied().unwrap_or(*slot);
                sx += p.0;
                sy += p.1;
            }
            *slot = (sx / 5.0, sy / 5.0);
        }
    }
    cur
}

/// A closed loop with each point's distance along it, for finding the
/// point at any distance.
struct Walk {
    pts: Vec<(f32, f32)>,
    at: Vec<f32>,
    total: f32,
}

impl Walk {
    fn new(loop_: &[(f32, f32)]) -> Walk {
        let n = loop_.len();
        let mut at = Vec::with_capacity(n);
        let mut total = 0.0;
        for i in 0..n {
            at.push(total);
            let a = loop_.get(i).copied().unwrap_or((0.0, 0.0));
            let b = loop_.get((i + 1) % n).copied().unwrap_or((0.0, 0.0));
            total += ((b.0 - a.0).powi(2) + (b.1 - a.1).powi(2)).sqrt();
        }
        Walk { pts: loop_.to_vec(), at, total }
    }

    fn point(&self, s: f32) -> (f32, f32) {
        let n = self.pts.len();
        let s = s.rem_euclid(self.total.max(1e-6));
        let i = self.at.partition_point(|&a| a <= s).saturating_sub(1);
        let a = self.pts.get(i).copied().unwrap_or((0.0, 0.0));
        let b = self.pts.get((i + 1) % n.max(1)).copied().unwrap_or(a);
        let start = self.at.get(i).copied().unwrap_or(0.0);
        let end = self.at.get(i + 1).copied().unwrap_or(self.total);
        let k = if end > start { (s - start) / (end - start) } else { 0.0 };
        (a.0 + (b.0 - a.0) * k, a.1 + (b.1 - a.1) * k)
    }
}

/// Evenly spaced points along a closed loop, at least `min` of them, as
/// distances along it. Loops shorter than `tiny` (specks, pinholes) get
/// none.
fn resample(walk: &Walk, spacing: f32, min: usize, tiny: f32) -> Vec<f32> {
    if walk.pts.len() < 3 || walk.total < tiny {
        return Vec::new();
    }
    let count = ((walk.total / spacing).round() as usize).max(min).max(3);
    let step = walk.total / count as f32;
    (0..count).map(|k| k as f32 * step).collect()
}

/// Cubism's Minimum boundary margin: where the straight edge between two
/// outline points runs onto `near` (the art grown by that margin), the
/// loop's point halfway between them goes in, a few rounds deep.
fn hold_off(walk: &Walk, mut at: Vec<f32>, near: &Grid, finest: f32) -> Vec<f32> {
    for _ in 0..4 {
        let n = at.len();
        let mut out = Vec::with_capacity(n * 2);
        let mut added = false;
        for i in 0..n {
            let s0 = at.get(i).copied().unwrap_or(0.0);
            out.push(s0);
            let mut s1 = at.get((i + 1) % n).copied().unwrap_or(0.0);
            if s1 <= s0 {
                s1 += walk.total;
            }
            if s1 - s0 < finest * 2.0 {
                continue;
            }
            let (a, b) = (walk.point(s0), walk.point(s1));
            let cuts = (1..8).any(|k| {
                let f = k as f32 / 8.0;
                near.contains(a.0 + (b.0 - a.0) * f, a.1 + (b.1 - a.1) * f)
            });
            if cuts {
                out.push((s0 + s1) / 2.0);
                added = true;
            }
        }
        at = out;
        if !added {
            break;
        }
    }
    at
}

/// Bowyer-Watson Delaunay triangulation. Quadratic, which is fine for the
/// few hundred to few thousand points a layer gets.
pub fn delaunay(pts: &[(f32, f32)]) -> Vec<[usize; 3]> {
    if pts.len() < 3 {
        return Vec::new();
    }
    let p: Vec<(f64, f64)> = pts.iter().map(|&(x, y)| (x as f64, y as f64)).collect();
    let (mut x0, mut y0, mut x1, mut y1) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
    for &(x, y) in &p {
        x0 = x0.min(x);
        y0 = y0.min(y);
        x1 = x1.max(x);
        y1 = y1.max(y);
    }
    let d = (x1 - x0).max(y1 - y0).max(1.0) * 20.0;
    let (mx, my) = ((x0 + x1) * 0.5, (y0 + y1) * 0.5);
    let n = p.len();
    let mut all = p.clone();
    all.push((mx - d, my - d));
    all.push((mx + d, my - d));
    all.push((mx, my + d));
    let at = |i: usize| all.get(i).copied().unwrap_or((0.0, 0.0));
    let circum = |t: &[usize; 3]| -> (f64, f64, f64) {
        let (ax, ay) = at(t[0]);
        let (bx, by) = at(t[1]);
        let (cx, cy) = at(t[2]);
        let dd = 2.0 * (ax * (by - cy) + bx * (cy - ay) + cx * (ay - by));
        if dd.abs() < 1e-12 {
            return (0.0, 0.0, f64::MAX);
        }
        let a2 = ax * ax + ay * ay;
        let b2 = bx * bx + by * by;
        let c2 = cx * cx + cy * cy;
        let ux = (a2 * (by - cy) + b2 * (cy - ay) + c2 * (ay - by)) / dd;
        let uy = (a2 * (cx - bx) + b2 * (ax - cx) + c2 * (bx - ax)) / dd;
        (ux, uy, (ax - ux).powi(2) + (ay - uy).powi(2))
    };
    let mut tris: Vec<([usize; 3], (f64, f64, f64))> = {
        let t = [n, n + 1, n + 2];
        vec![(t, circum(&t))]
    };
    for i in 0..n {
        let (px, py) = at(i);
        let mut bad = Vec::new();
        tris.retain(|(t, (ux, uy, r2))| {
            let inside = (px - ux).powi(2) + (py - uy).powi(2) < *r2 - 1e-9;
            if inside {
                bad.push(*t);
            }
            !inside
        });
        // the hole's boundary: edges used by exactly one bad triangle
        let mut edges: Vec<(usize, usize)> = Vec::new();
        for t in &bad {
            for (a, b) in [(t[0], t[1]), (t[1], t[2]), (t[2], t[0])] {
                if let Some(k) =
                    edges.iter().position(|&(x, y)| (x == b && y == a) || (x == a && y == b))
                {
                    edges.swap_remove(k);
                } else {
                    edges.push((a, b));
                }
            }
        }
        for (a, b) in edges {
            let t = [a, b, i];
            let c = circum(&t);
            tris.push((t, c));
        }
    }
    tris.into_iter().map(|(t, _)| t).filter(|t| t.iter().all(|&v| v < n)).collect()
}

/// Which side of the line a-b the point c is on, with a tolerance scaled to
/// the coordinates so pixel and uv spaces both work: 1 left, -1 right, 0 on it.
fn side(a: (f64, f64), b: (f64, f64), c: (f64, f64), eps: f64) -> i8 {
    let o = (b.0 - a.0) * (c.1 - a.1) - (b.1 - a.1) * (c.0 - a.0);
    if o > eps {
        1
    } else if o < -eps {
        -1
    } else {
        0
    }
}

/// Segments a-b and c-d cross at a point inside both (touching ends or a
/// point lying on the other segment does not count).
fn crossing(a: (f64, f64), b: (f64, f64), c: (f64, f64), d: (f64, f64), eps: f64) -> bool {
    let (s1, s2) = (side(a, b, c, eps), side(a, b, d, eps));
    let (s3, s4) = (side(c, d, a, eps), side(c, d, b, eps));
    s1 * s2 < 0 && s3 * s4 < 0
}

fn key(a: usize, b: usize) -> (usize, usize) {
    (a.min(b), a.max(b))
}

/// Delaunay with `edges` forced in, each a pair of point indices: the edges
/// crossing one are flipped away until it appears (Sloan's method). An edge
/// that cannot go in (a point lying on it, or two forced edges crossing)
/// is left out rather than breaking one already in.
pub fn constrained(pts: &[(f32, f32)], edges: &[(usize, usize)]) -> Vec<[usize; 3]> {
    let mut tris = delaunay(pts);
    let p: Vec<(f64, f64)> = pts.iter().map(|&(x, y)| (x as f64, y as f64)).collect();
    let span = p.iter().fold(1e-9f64, |m, &(x, y)| m.max(x.abs()).max(y.abs()));
    let eps = span * span * 1e-12;
    let at = |i: usize| p.get(i).copied().unwrap_or((0.0, 0.0));
    let has = |tris: &[[usize; 3]], a: usize, b: usize| {
        tris.iter().any(|t| t.contains(&a) && t.contains(&b))
    };
    let mut fixed: std::collections::HashSet<(usize, usize)> = std::collections::HashSet::new();
    for &(a, b) in edges {
        if a == b || a >= p.len() || b >= p.len() {
            continue;
        }
        let limit = tris.len() * 4 + 64;
        for _ in 0..limit {
            if has(&tris, a, b) {
                fixed.insert(key(a, b));
                break;
            }
            // flip the first crossing edge whose two faces make a convex quad
            let mut flipped = false;
            'scan: for ti in 0..tris.len() {
                let Some(t) = tris.get(ti).copied() else { continue };
                for (u, v) in [(t[0], t[1]), (t[1], t[2]), (t[2], t[0])] {
                    if [u, v].iter().any(|x| *x == a || *x == b)
                        || fixed.contains(&key(u, v))
                        || !crossing(at(a), at(b), at(u), at(v), eps)
                    {
                        continue;
                    }
                    let Some(tj) = (0..tris.len()).find(|&j| {
                        j != ti && tris.get(j).is_some_and(|o| o.contains(&u) && o.contains(&v))
                    }) else {
                        continue;
                    };
                    let third = |t: [usize; 3]| t.into_iter().find(|&x| x != u && x != v);
                    let (Some(pp), Some(q)) = (third(t), tris.get(tj).copied().and_then(third))
                    else {
                        continue;
                    };
                    if !crossing(at(pp), at(q), at(u), at(v), eps) {
                        continue;
                    }
                    if let Some(s) = tris.get_mut(ti) {
                        *s = [pp, q, u];
                    }
                    if let Some(s) = tris.get_mut(tj) {
                        *s = [q, pp, v];
                    }
                    flipped = true;
                    break 'scan;
                }
            }
            if !flipped {
                break;
            }
        }
    }
    tris
}

/// Per triangle, whether it lies outside every closed run of `edges`: a
/// flood from the hull that stops at those edges. All true when they close
/// nothing off.
pub fn outside(tris: &[[usize; 3]], edges: &[(usize, usize)]) -> Vec<bool> {
    let walls: std::collections::HashSet<(usize, usize)> =
        edges.iter().map(|&(a, b)| key(a, b)).collect();
    let mut faces_of: std::collections::HashMap<(usize, usize), Vec<usize>> =
        std::collections::HashMap::new();
    for (ti, t) in tris.iter().enumerate() {
        for (a, b) in [(t[0], t[1]), (t[1], t[2]), (t[2], t[0])] {
            faces_of.entry(key(a, b)).or_default().push(ti);
        }
    }
    let mut out = vec![false; tris.len()];
    let mut queue: Vec<usize> = Vec::new();
    for (e, faces) in &faces_of {
        if let [only] = faces.as_slice()
            && !walls.contains(e)
            && let Some(o) = out.get_mut(*only)
            && !*o
        {
            *o = true;
            queue.push(*only);
        }
    }
    while let Some(ti) = queue.pop() {
        let Some(t) = tris.get(ti).copied() else { continue };
        for (a, b) in [(t[0], t[1]), (t[1], t[2]), (t[2], t[0])] {
            let e = key(a, b);
            if walls.contains(&e) {
                continue;
            }
            for &n in faces_of.get(&e).map(Vec::as_slice).unwrap_or(&[]) {
                if let Some(o) = out.get_mut(n)
                    && !*o
                {
                    *o = true;
                    queue.push(n);
                }
            }
        }
    }
    out
}

/// Even-odd test against closed loops: inside the outlines and out of
/// their holes.
fn inside_rings(rings: &[Vec<(f32, f32)>], x: f32, y: f32) -> bool {
    let mut inside = false;
    for r in rings {
        let n = r.len();
        for i in 0..n {
            let (Some(&a), Some(&b)) = (r.get(i), r.get((i + 1) % n)) else { continue };
            if (a.1 > y) != (b.1 > y) && x < a.0 + (y - a.1) / (b.1 - a.1) * (b.0 - a.0) {
                inside = !inside;
            }
        }
    }
    inside
}

/// Mesh one layer's alpha. `alpha` is `w * h`, row-major.
pub fn generate(alpha: &[u8], w: usize, h: usize, p: &Params) -> Mesh {
    if w == 0 || h == 0 || alpha.len() < w * h {
        return Mesh::default();
    }
    // small layers would get a handful of points at fixed spacing, so the
    // spacing shrinks with the layer: at least about five along its short side
    let short = w.min(h) as f32;
    let outline = p.outline_spacing.min((short / 5.0).max(4.0)).max(2.0);
    let p = Params {
        outline_spacing: outline,
        inner_spacing: p.inner_spacing.min((short / 3.0).max(outline)).max(outline),
        ..*p
    };
    // cells a quarter of the spacing (a sixth for a face, whose outline has
    // to sit a set distance off the art), and fine enough for the margins
    let per = if p.style == Style::Face { 6.0 } else { 4.0 };
    let mut cell = p.outline_spacing.min(p.inner_spacing) / per;
    for m in [p.margin, p.inner_margin, p.min_margin] {
        if m > 0.0 {
            cell = cell.min(m / 1.5);
        }
    }
    let art = art_grid(alpha, w, h, cell.clamp(1.0, 32.0), p.threshold);
    let outer = art.grown(p.margin);
    // the minimum margin can't ask for more room than the margin gives, and
    // the grid's cells blur both by a cell or so, which must not count
    let floor = p.min_margin.min(p.margin * 0.9) - art.cell * 1.5;
    let near = (p.min_margin > 0.0).then(|| art.grown(floor.max(0.0)));
    let tiny = art.cell * 8.0;

    let mut points: Vec<(f32, f32)> = Vec::new();
    let mut rings: Vec<Vec<(f32, f32)>> = Vec::new();
    for l in contours(&outer) {
        let walk = Walk::new(&smooth(&l, 3));
        let mut at = resample(&walk, p.outline_spacing, p.min_points, tiny);
        if let Some(near) = &near {
            at = hold_off(&walk, at, near, art.cell);
        }
        let ring: Vec<(f32, f32)> = at.iter().map(|&s| walk.point(s)).collect();
        points.extend(ring.iter().copied());
        rings.push(ring);
    }
    if points.len() < 3 {
        return Mesh::default();
    }
    if p.style == Style::Face {
        face_inside(&mut points, &rings, &outer, alpha, w, &p);
        lattice_inside(&mut points, &outer, w, h, p.inner_spacing, p.outline_spacing * 0.7);
    } else {
        let clear = p.outline_spacing.min(p.inner_spacing) * 0.5;
        let inner = (p.inner_margin > 0.0).then(|| art.shrunk(p.inner_margin));
        if let Some(inner) = &inner {
            for l in contours(inner) {
                let walk = Walk::new(&smooth(&l, 3));
                for s in resample(&walk, p.inner_spacing, 3, tiny) {
                    let q = walk.point(s);
                    if !points
                        .iter()
                        .any(|o| (o.0 - q.0).powi(2) + (o.1 - q.1).powi(2) < clear * clear)
                    {
                        points.push(q);
                    }
                }
            }
        }
        spine_inside(&mut points, &art, &p, clear);
        lattice_inside(
            &mut points,
            inner.as_ref().unwrap_or(&outer),
            w,
            h,
            p.inner_spacing,
            p.inner_spacing * 0.6,
        );
    }
    if points.len() >= u16::MAX as usize {
        points.truncate(u16::MAX as usize - 1);
    }
    triangulate(points, &rings)
}

/// A row of points down the middle of the strokes too thin for the inner
/// ring (a lash, a strand), as Cubism lays them: outline, middle, outline,
/// so the strip can bend along its length. The middle is where the depth
/// inside the art peaks across the stroke.
fn spine_inside(points: &mut Vec<(f32, f32)>, art: &Grid, p: &Params, clear: f32) {
    let (gw, gh) = (art.w, art.h);
    let depth = art.depth();
    let get = |x: i64, y: i64| {
        if x < 0 || y < 0 || x as usize >= gw || y as usize >= gh {
            0.0
        } else {
            depth.get(y as usize * gw + x as usize).copied().unwrap_or(0.0)
        }
    };
    // thin: where the inner ring can't reach (no deeper than its margin)
    let thin = if p.inner_margin > 0.0 { p.inner_margin } else { p.inner_spacing * 0.5 };
    let mut ridge: Vec<(f32, f32, f32)> = Vec::new();
    for y in 0..gh as i64 {
        for x in 0..gw as i64 {
            let v = get(x, y);
            if v < art.cell || v > thin {
                continue;
            }
            // not lower than both neighbours across some direction
            let peak = [(1, 0), (0, 1), (1, 1), (1, -1)]
                .iter()
                .any(|&(dx, dy)| v >= get(x + dx, y + dy) && v >= get(x - dx, y - dy));
            if peak {
                // cell gx covers pixels (gx - 1) * cell up to gx * cell
                ridge.push(((x as f32 - 0.5) * art.cell, (y as f32 - 0.5) * art.cell, v));
            }
        }
    }
    // thickest first, so the row settles on the stroke's true middle
    ridge.sort_by(|a, b| b.2.total_cmp(&a.2));
    let gap = p.inner_spacing;
    let placed = points.clone();
    let mut row: Vec<(f32, f32)> = Vec::new();
    for (x, y, _) in ridge {
        let near = |q: &(f32, f32), r: f32| (q.0 - x).powi(2) + (q.1 - y).powi(2) < r * r;
        if placed.iter().any(|q| near(q, clear)) || row.iter().any(|q| near(q, gap)) {
            continue;
        }
        row.push((x, y));
    }
    points.extend(row);
}

/// Cubism's inside: points on a staggered grid `spacing` apart over the
/// cells of `grid`, kept `clear` pixels from the points already placed.
fn lattice_inside(
    points: &mut Vec<(f32, f32)>,
    grid: &Grid,
    w: usize,
    h: usize,
    spacing: f32,
    clear: f32,
) {
    let rows = (h as f32 / (spacing * 0.866)).ceil() as usize + 1;
    let cols = (w as f32 / spacing).ceil() as usize + 1;
    let placed: Vec<(f32, f32)> = points.clone();
    for r in 0..rows {
        let y = r as f32 * spacing * 0.866;
        let shift = if r % 2 == 1 { spacing * 0.5 } else { 0.0 };
        for c in 0..cols {
            let x = c as f32 * spacing + shift;
            if !grid.contains(x, y) {
                continue;
            }
            let near_edge =
                placed.iter().any(|&(ox, oy)| (ox - x).powi(2) + (oy - y).powi(2) < clear * clear);
            if !near_edge {
                points.push((x, y));
            }
        }
    }
}

/// A face's second ring, measured from a face mesh a rigger built by hand:
/// one point between each two outline points, found by walking in from
/// their midpoint to where the art starts and a little past it (a twentieth
/// of the spacing). The outline sits about a quarter spacing outside the
/// art, so the drawn edge runs through a thin zigzag strip.
fn face_inside(
    points: &mut Vec<(f32, f32)>,
    rings: &[Vec<(f32, f32)>],
    grid: &Grid,
    alpha: &[u8],
    w: usize,
    p: &Params,
) {
    let h = alpha.len() / w.max(1);
    let solid = |x: f32, y: f32| {
        x >= 0.0
            && y >= 0.0
            && (x as usize) < w
            && (y as usize) < h
            && alpha.get(y as usize * w + x as usize).is_some_and(|&a| a > p.threshold)
    };
    let inset = (p.outline_spacing * 0.05).max(1.0);
    // how far in to look for the art: past the margin and the grid's cells
    let reach = p.margin + p.outline_spacing;
    for ring in rings {
        let n = ring.len();
        for i in 0..n {
            let (Some(&(ax, ay)), Some(&(bx, by))) = (ring.get(i), ring.get((i + 1) % n)) else {
                continue;
            };
            let (mx, my) = ((ax + bx) / 2.0, (ay + by) / 2.0);
            let (tx, ty) = (bx - ax, by - ay);
            let len = (tx * tx + ty * ty).sqrt();
            if len < 1e-3 {
                continue;
            }
            // the normal that points into the shape
            let (nx, ny) = (-ty / len, tx / len);
            let probe = p.outline_spacing * 0.25;
            let Some((dx, dy)) = [(nx, ny), (-nx, -ny)]
                .into_iter()
                .find(|&(dx, dy)| grid.contains(mx + dx * probe, my + dy * probe))
            else {
                continue;
            };
            let mut t = 0.0;
            while t <= reach {
                if solid(mx + dx * t, my + dy * t) {
                    let q = (mx + dx * (t + inset), my + dy * (t + inset));
                    if grid.contains(q.0, q.1) {
                        points.push(q);
                    }
                    break;
                }
                t += 0.5;
            }
        }
    }
}

/// Delaunay over the points with the outline rings forced in as edges, so
/// no triangle can bridge a notch or a gap between strokes, then everything
/// outside the rings carved away and points no triangle uses dropped. The
/// rings' points come first in `points`, ring after ring.
fn triangulate(points: Vec<(f32, f32)>, rings: &[Vec<(f32, f32)>]) -> Mesh {
    let mut edges = Vec::new();
    let mut start = 0;
    for r in rings {
        let n = r.len();
        for i in 0..n {
            edges.push((start + i, start + (i + 1) % n));
        }
        start += n;
    }
    edges.retain(|&(a, b)| a < points.len() && b < points.len());
    let mut triangles: Vec<[u16; 3]> = Vec::new();
    for t in constrained(&points, &edges) {
        let (Some(a), Some(b), Some(c)) = (points.get(t[0]), points.get(t[1]), points.get(t[2]))
        else {
            continue;
        };
        // with the rings in as edges a triangle is wholly in or wholly out,
        // so its centroid decides
        let cx = (a.0 + b.0 + c.0) / 3.0;
        let cy = (a.1 + b.1 + c.1) / 3.0;
        if inside_rings(rings, cx, cy) {
            triangles.push([t[0] as u16, t[1] as u16, t[2] as u16]);
        }
    }
    // drop points no triangle uses
    let mut remap = vec![u16::MAX; points.len()];
    let mut kept = Vec::new();
    for t in &mut triangles {
        for v in t.iter_mut() {
            let i = *v as usize;
            if remap.get(i).copied() == Some(u16::MAX) {
                if let Some(slot) = remap.get_mut(i) {
                    *slot = kept.len() as u16;
                }
                kept.push(points.get(i).copied().unwrap_or((0.0, 0.0)));
            }
            *v = remap.get(i).copied().unwrap_or(0);
        }
    }
    Mesh { points: kept, triangles }
}

/// Mesh one layer's patch of a texture atlas. `alpha_at` reads the page's
/// alpha, `page` is its size in pixels and `patch` the layer's uv box
/// (u0, v0, u1, v1): only art inside it counts. `room` is the uv box the
/// mesh may reach into, the patch grown over the page's empty space up to
/// the neighbouring layers, so the outline can keep its margin; a triangle
/// reaching past it would sample the next layer's art and smear it, so
/// every point is clamped inside. Returns uvs and triangle indices.
pub fn for_patch(
    alpha_at: &dyn Fn(u32, u32) -> u8,
    page: (u32, u32),
    patch: (f32, f32, f32, f32),
    room: (f32, f32, f32, f32),
    p: &Params,
) -> Option<(Vec<f32>, Vec<u16>)> {
    let (tw, th) = (page.0 as f32, page.1 as f32);
    let (u0, v0, u1, v1) = patch;
    if !(u0 < u1 && v0 < v1) || tw < 1.0 || th < 1.0 {
        return None;
    }
    // the room never shrinks the patch
    let room = (room.0.min(u0), room.1.min(v0), room.2.max(u1), room.3.max(v1));
    // the patch, padded so the margin has room to work out the outline
    let pad = p.margin + 2.0;
    let x0 = (u0 * tw - pad).max(0.0).floor() as u32;
    let y0 = (v0 * th - pad).max(0.0).floor() as u32;
    let x1 = (u1 * tw + pad).min(tw).ceil() as u32;
    let y1 = (v1 * th + pad).min(th).ceil() as u32;
    let (w, h) = (x1.saturating_sub(x0) as usize, y1.saturating_sub(y0) as usize);
    let (tx0, ty0, tx1, ty1) =
        ((u0 * tw).floor(), (v0 * th).floor(), (u1 * tw).ceil(), (v1 * th).ceil());
    let (rx0, ry0, rx1, ry1) = (
        (room.0 * tw).max(0.0),
        (room.1 * th).max(0.0),
        (room.2 * tw).min(tw),
        (room.3 * th).min(th),
    );
    let mut alpha = vec![0u8; w.checked_mul(h)?];
    for y in 0..h {
        for x in 0..w {
            let (ax, ay) = (x0 + x as u32, y0 + y as u32);
            let (fx, fy) = (ax as f32, ay as f32);
            if fx < tx0 || fy < ty0 || fx >= tx1 || fy >= ty1 {
                continue;
            }
            if let Some(slot) = alpha.get_mut(y * w + x) {
                *slot = alpha_at(ax, ay);
            }
        }
    }
    let mesh = generate(&alpha, w, h, p);
    if mesh.triangles.is_empty() || mesh.points.len() >= u16::MAX as usize {
        return None;
    }
    let uvs = mesh
        .points
        .iter()
        .flat_map(|&(x, y)| {
            [(x0 as f32 + x).clamp(rx0, rx1) / tw, (y0 as f32 + y).clamp(ry0, ry1) / th]
        })
        .collect();
    let indices = mesh.triangles.iter().flat_map(|t| t.iter().copied()).collect();
    Some((uvs, indices))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An opaque ellipse, `w` by `h`, in a transparent field.
    fn ellipse(w: usize, h: usize) -> Vec<u8> {
        let (cx, cy, rx, ry) = (w as f32 / 2.0, h as f32 / 2.0, w as f32 * 0.4, h as f32 * 0.42);
        (0..w * h)
            .map(|i| {
                let (x, y) = ((i % w) as f32, (i / w) as f32);
                if ((x - cx) / rx).powi(2) + ((y - cy) / ry).powi(2) <= 1.0 { 255 } else { 0 }
            })
            .collect()
    }

    #[test]
    fn a_face_has_a_ring_hugging_the_edge_and_an_even_inside() {
        let (w, h) = (300, 380);
        let alpha = ellipse(w, h);
        let face = Params {
            outline_spacing: 18.0,
            inner_spacing: 36.0,
            margin: 3.0,
            style: Style::Face,
            ..Params::default()
        };
        let m = generate(&alpha, w, h, &face);
        assert!(!m.triangles.is_empty());
        // how far each point is from the drawn edge, in px: + inside
        let (cx, cy, rx, ry) = (w as f32 / 2.0, h as f32 / 2.0, w as f32 * 0.4, h as f32 * 0.42);
        let depth = |(x, y): (f32, f32)| {
            let r = (((x - cx) / rx).powi(2) + ((y - cy) / ry).powi(2)).sqrt();
            (1.0 - r) * rx.min(ry)
        };
        let outside = m.points.iter().filter(|&&p| depth(p) < -1.0).count();
        let hugging = m.points.iter().filter(|&&p| (0.0..=4.0).contains(&depth(p))).count();
        let deep = m.points.iter().filter(|&&p| depth(p) > 12.0).count();
        assert!(outside > 30, "an outline ring outside the art: {outside}");
        assert!(
            hugging * 10 >= outside * 8,
            "a second ring just inside the edge: {hugging} for {outside}"
        );
        // the inside is an even lattice at the inner spacing
        let area = std::f32::consts::PI * rx * ry;
        assert!(deep as f32 > 0.6 * area / (36.0 * 36.0 * 0.866), "an even inside: {deep}");
    }

    fn disc(size: usize, r: f32) -> Vec<u8> {
        let c = size as f32 / 2.0;
        (0..size * size)
            .map(|i| {
                let (x, y) = ((i % size) as f32 + 0.5, (i / size) as f32 + 0.5);
                if (x - c).powi(2) + (y - c).powi(2) <= r * r { 255 } else { 0 }
            })
            .collect()
    }

    #[test]
    fn disc_gets_an_outline_and_an_interior() {
        let a = disc(200, 80.0);
        let p =
            Params { outline_spacing: 16.0, inner_spacing: 32.0, margin: 4.0, ..Params::default() };
        let m = generate(&a, 200, 200, &p);
        assert!(m.triangles.len() > 20, "{} triangles", m.triangles.len());
        // perimeter of r=84 at 16px spacing is about 33 points, interior a
        // few dozen more; far below the grid mesher's count
        assert!(m.points.len() > 40 && m.points.len() < 200, "{} points", m.points.len());
        // every point sits on or near the disc
        let c = 100.0;
        for &(x, y) in &m.points {
            let r = ((x - c).powi(2) + (y - c).powi(2)).sqrt();
            assert!(r < 80.0 + 4.0 + 16.0, "point at radius {r}");
        }
        // indices are valid
        for t in &m.triangles {
            assert!(t.iter().all(|&v| (v as usize) < m.points.len()));
        }
    }

    #[test]
    fn ring_keeps_its_hole_open() {
        let size = 200;
        let c = 100.0;
        let a: Vec<u8> = (0..size * size)
            .map(|i| {
                let (x, y) = ((i % size) as f32 + 0.5, (i / size) as f32 + 0.5);
                let r = ((x - c).powi(2) + (y - c).powi(2)).sqrt();
                if (40.0..=85.0).contains(&r) { 255 } else { 0 }
            })
            .collect();
        let p =
            Params { outline_spacing: 12.0, inner_spacing: 24.0, margin: 2.0, ..Params::default() };
        let m = generate(&a, size, size, &p);
        // no triangle may cover the centre of the hole
        for t in &m.triangles {
            let pts: Vec<(f32, f32)> = t.iter().map(|&v| m.points[v as usize]).collect();
            let cx = (pts[0].0 + pts[1].0 + pts[2].0) / 3.0;
            let cy = (pts[0].1 + pts[1].1 + pts[2].1) / 3.0;
            let r = ((cx - c).powi(2) + (cy - c).powi(2)).sqrt();
            assert!(r > 25.0, "triangle centred at radius {r} fills the hole");
        }
        assert!(m.triangles.len() > 30);
    }

    #[test]
    fn a_narrow_notch_is_not_bridged() {
        // a block with a V cut in from the left, 40px open at the edge and
        // closing 150px in: the inner corner of an eyelash
        let (w, h) = (240usize, 140usize);
        let solid = |x: f32, y: f32| {
            let in_block = (20.0..220.0).contains(&x) && (20.0..120.0).contains(&y);
            let half = 20.0 * (1.0 - (x - 20.0) / 150.0);
            in_block && !(half > 0.0 && (y - 70.0).abs() < half)
        };
        let a: Vec<u8> = (0..w * h)
            .map(|i| if solid((i % w) as f32 + 0.5, (i / w) as f32 + 0.5) { 255 } else { 0 })
            .collect();
        let p =
            Params { outline_spacing: 12.0, inner_spacing: 24.0, margin: 3.0, ..Params::default() };
        let m = generate(&a, w, h, &p);
        assert!(m.triangles.len() > 20);
        let opaque: Vec<(f32, f32)> = (0..w * h)
            .filter(|&i| a[i] > 0)
            .map(|i| ((i % w) as f32 + 0.5, (i / w) as f32 + 0.5))
            .collect();
        let off = |q: (f32, f32)| {
            opaque
                .iter()
                .map(|o| (o.0 - q.0).powi(2) + (o.1 - q.1).powi(2))
                .fold(f32::MAX, f32::min)
                .sqrt()
        };
        let mut worst = 0.0f32;
        for t in &m.triangles {
            let c = t.iter().fold((0.0, 0.0), |s, &v| {
                (s.0 + m.points[v as usize].0 / 3.0, s.1 + m.points[v as usize].1 / 3.0)
            });
            worst = worst.max(off(c));
            for (x, y) in [(t[0], t[1]), (t[1], t[2]), (t[2], t[0])] {
                let (p0, p1) = (m.points[x as usize], m.points[y as usize]);
                worst = worst.max(off(((p0.0 + p1.0) / 2.0, (p0.1 + p1.1) / 2.0)));
            }
        }
        assert!(worst < 7.0, "a face reaches {worst}px into the notch");
    }

    #[test]
    fn forced_edges_go_in_and_close_off_the_outside() {
        // a drawn diamond with a point in its middle and one in the corner
        // outside it
        let pts = vec![(0.0, 5.0), (5.0, 0.0), (10.0, 5.0), (5.0, 10.0), (5.0, 5.0), (0.0, 0.0)];
        let edges = [(0, 1), (1, 2), (2, 3), (3, 0)];
        let t = constrained(&pts, &edges);
        for &(a, b) in &edges {
            assert!(t.iter().any(|f| f.contains(&a) && f.contains(&b)), "edge {a}-{b} missing");
        }
        // only the corner triangle outside the diamond is outside
        let out = outside(&t, &edges);
        let kept: Vec<[usize; 3]> =
            t.iter().zip(&out).filter(|(_, o)| !**o).map(|(f, _)| *f).collect();
        assert!(kept.iter().all(|f| !f.contains(&5)));
        assert_eq!(kept.len(), 4);
    }

    #[test]
    fn empty_and_degenerate_inputs_are_harmless() {
        assert!(generate(&[], 0, 0, &Params::default()).triangles.is_empty());
        assert!(generate(&[0; 100], 10, 10, &Params::default()).triangles.is_empty());
        assert!(generate(&[255; 4], 10, 10, &Params::default()).triangles.is_empty());
    }

    #[test]
    fn the_outline_keeps_its_margin_where_the_page_has_room() {
        // a layer cut tight to its art, alone on the page: the outline may
        // stand off it, past the layer's own box
        let alpha_at =
            |x: u32, y: u32| if (40..80).contains(&x) && (40..80).contains(&y) { 255 } else { 0 };
        let patch = (40.0 / 128.0, 40.0 / 128.0, 80.0 / 128.0, 80.0 / 128.0);
        let room = (0.0, 0.0, 1.0, 1.0);
        let p =
            Params { outline_spacing: 10.0, inner_spacing: 10.0, margin: 6.0, ..Params::default() };
        let (uvs, _) = for_patch(&alpha_at, (128, 128), patch, room, &p).unwrap();
        let left = uvs.chunks(2).map(|uv| uv[0] * 128.0).fold(f32::MAX, f32::min);
        assert!(left < 37.0, "the outline stops at the box: {left}");
        // with no room it stays on the box
        let (uvs, _) = for_patch(&alpha_at, (128, 128), patch, patch, &p).unwrap();
        let left = uvs.chunks(2).map(|uv| uv[0] * 128.0).fold(f32::MAX, f32::min);
        assert!(left >= 40.0 - 1e-3);
    }

    #[test]
    fn patch_mesh_stays_on_its_layer() {
        // two opaque layers side by side in a 64x32 page, two pixels apart:
        // A covers x 0..30, B covers x 32..64
        let alpha_at = |x: u32, _y: u32| if !(30..32).contains(&x) { 255u8 } else { 0 };
        let patch = (0.0, 0.0, 30.0 / 64.0, 1.0);
        let p = Params { margin: 6.0, ..Params::default() };
        let (uvs, idx) = for_patch(&alpha_at, (64, 32), patch, patch, &p).unwrap();
        assert!(!idx.is_empty());
        for uv in uvs.chunks(2) {
            assert!(uv[0] <= 30.0 / 64.0 + 1e-6, "point at u {} reaches layer B", uv[0]);
        }
    }
}
