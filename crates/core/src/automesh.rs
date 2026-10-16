//! Outline-following automatic meshing, the way Cubism's auto mesh works:
//! points along the artwork's contour (pushed out by a margin), sparser
//! points inside, Delaunay triangulation, then everything outside the shape
//! carved away so concave outlines and holes stay open.
//!
//! The mask is worked on in coarse cells rather than pixels, so a 4096px
//! layer costs about as much as a 400px one.

/// Knobs, all in texture pixels.
#[derive(Clone, Copy, Debug)]
pub struct Params {
    /// Distance between points along the outline.
    pub outline_spacing: f32,
    /// Distance between points inside the shape.
    pub inner_spacing: f32,
    /// How far the outline sits outside the opaque pixels.
    pub margin: f32,
    /// Alpha above this counts as artwork.
    pub threshold: u8,
    /// How the inside is filled.
    pub style: Style,
}

/// How a mesh is laid out inside its outline.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Style {
    /// One ring on the outline and an even lattice inside (Cubism's
    /// Standard, Deform and Light).
    #[default]
    Lattice,
    /// A face as riggers build it: two rings along the edge, one outside
    /// the art at the margin and one as far inside, so the drawn edge stays
    /// in a narrow strip when warps bend it, and two hubs on the centre
    /// line (at 32% and 76% of the height) the inner ring fans into.
    Face,
}

impl Default for Params {
    fn default() -> Self {
        Self {
            outline_spacing: 24.0,
            inner_spacing: 48.0,
            margin: 8.0,
            threshold: 8,
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

    /// Whether a point in pixel space lies on the (dilated) shape.
    fn contains(&self, px: f32, py: f32) -> bool {
        let gx = (px / self.cell + 1.0).floor() as i64;
        let gy = (py / self.cell + 1.0).floor() as i64;
        self.at(gx, gy)
    }

    /// The same with a one-cell allowance: outline triangles straddle the
    /// contour, and the strict test would nibble teeth out of the edge.
    fn near(&self, px: f32, py: f32) -> bool {
        let gx = (px / self.cell + 1.0).floor() as i64;
        let gy = (py / self.cell + 1.0).floor() as i64;
        (-1..=1).any(|dy| (-1..=1).any(|dx| self.at(gx + dx, gy + dy)))
    }
}

fn build_grid(alpha: &[u8], w: usize, h: usize, p: &Params) -> Grid {
    let cell = (p.outline_spacing.min(p.inner_spacing) / 4.0).clamp(1.0, 32.0);
    let gw = (w as f32 / cell).ceil() as usize + 2;
    let gh = (h as f32 / cell).ceil() as usize + 2;
    let mut on = vec![false; gw * gh];
    for y in 0..h {
        let gy = (y as f32 / cell) as usize + 1;
        for x in 0..w {
            if alpha.get(y * w + x).copied().unwrap_or(0) > p.threshold {
                let gx = (x as f32 / cell) as usize + 1;
                if let Some(c) = on.get_mut(gy * gw + gx) {
                    *c = true;
                }
            }
        }
    }
    // grow by the margin with a two-pass chamfer distance, in cell units
    let reach = (p.margin / cell).max(0.0);
    if reach > 0.0 {
        let big = f32::MAX / 4.0;
        let mut d: Vec<f32> = on.iter().map(|&o| if o { 0.0 } else { big }).collect();
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
        // keep the outermost ring empty so contours still close
        for y in 0..gh {
            for x in 0..gw {
                let border = x == 0 || y == 0 || x + 1 == gw || y + 1 == gh;
                if let Some(c) = on.get_mut(y * gw + x) {
                    *c = !border && d.get(y * gw + x).copied().unwrap_or(big) <= reach;
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

/// Evenly spaced points along a closed loop.
fn resample(loop_: &[(f32, f32)], spacing: f32) -> Vec<(f32, f32)> {
    let n = loop_.len();
    let seg = |i: usize| {
        let a = loop_.get(i).copied().unwrap_or((0.0, 0.0));
        let b = loop_.get((i + 1) % n).copied().unwrap_or((0.0, 0.0));
        (a, b, ((b.0 - a.0).powi(2) + (b.1 - a.1).powi(2)).sqrt())
    };
    let total: f32 = (0..n).map(|i| seg(i).2).sum();
    if total < spacing * 3.0 {
        return Vec::new();
    }
    let count = (total / spacing).round().max(3.0) as usize;
    let step = total / count as f32;
    let mut out = Vec::with_capacity(count);
    let mut target = 0.0f32;
    let mut walked = 0.0f32;
    for i in 0..n {
        let (a, b, len) = seg(i);
        while target <= walked + len && out.len() < count {
            let k = if len > 0.0 { (target - walked) / len } else { 0.0 };
            out.push((a.0 + (b.0 - a.0) * k, a.1 + (b.1 - a.1) * k));
            target += step;
        }
        walked += len;
    }
    out
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
    let grid = build_grid(alpha, w, h, &p);

    let mut points: Vec<(f32, f32)> = Vec::new();
    let rings: Vec<Vec<(f32, f32)>> =
        contours(&grid).iter().map(|l| resample(&smooth(l, 3), p.outline_spacing)).collect();
    for r in &rings {
        points.extend(r.iter().copied());
    }
    if points.len() < 3 {
        return Mesh::default();
    }
    if p.style == Style::Face {
        face_inside(&mut points, &rings, &grid, &p);
    } else {
        lattice_inside(&mut points, &grid, w, h, &p);
    }
    if points.len() >= u16::MAX as usize {
        points.truncate(u16::MAX as usize - 1);
    }
    triangulate(points, &grid)
}

/// Cubism's inside: points on a staggered grid, kept clear of the outline.
fn lattice_inside(points: &mut Vec<(f32, f32)>, grid: &Grid, w: usize, h: usize, p: &Params) {
    // inner points on a staggered grid, kept clear of the outline
    let clear = p.outline_spacing * 0.6;
    let rows = (h as f32 / (p.inner_spacing * 0.866)).ceil() as usize + 1;
    let cols = (w as f32 / p.inner_spacing).ceil() as usize + 1;
    let outline: Vec<(f32, f32)> = points.clone();
    for r in 0..rows {
        let y = r as f32 * p.inner_spacing * 0.866;
        let shift = if r % 2 == 1 { p.inner_spacing * 0.5 } else { 0.0 };
        for c in 0..cols {
            let x = c as f32 * p.inner_spacing + shift;
            if !grid.contains(x, y) {
                continue;
            }
            let near_edge =
                outline.iter().any(|&(ox, oy)| (ox - x).powi(2) + (oy - y).powi(2) < clear * clear);
            if !near_edge {
                points.push((x, y));
            }
        }
    }
}

/// A face's inside: a second ring as far inside the art's edge as the
/// first sits outside it, then two hubs on the centre line of the largest
/// outline for the inner ring to fan into.
fn face_inside(points: &mut Vec<(f32, f32)>, rings: &[Vec<(f32, f32)>], grid: &Grid, p: &Params) {
    // the outer ring sits `margin` outside the art: the inner one as far in
    let band = (p.margin * 2.0).max(p.outline_spacing * 0.25);
    for ring in rings {
        let n = ring.len();
        for (i, &(x, y)) in ring.iter().enumerate() {
            let (Some(&(ax, ay)), Some(&(bx, by))) =
                (ring.get((i + n - 1) % n), ring.get((i + 1) % n))
            else {
                continue;
            };
            let (tx, ty) = (bx - ax, by - ay);
            let len = (tx * tx + ty * ty).sqrt();
            if len < 1e-3 {
                continue;
            }
            // the normal that points into the shape
            let (nx, ny) = (-ty / len, tx / len);
            let inside = [(nx, ny), (-nx, -ny)]
                .into_iter()
                .map(|(dx, dy)| (x + dx * band, y + dy * band))
                .find(|&(qx, qy)| grid.contains(qx, qy));
            if let Some(q) = inside {
                points.push(q);
            }
        }
    }
    // the hubs, on the centre line of the largest outline
    let Some(big) = rings.iter().max_by_key(|r| r.len()) else { return };
    let (x0, y0, x1, y1) =
        big.iter().fold((f32::MAX, f32::MAX, f32::MIN, f32::MIN), |b, &(x, y)| {
            (b.0.min(x), b.1.min(y), b.2.max(x), b.3.max(y))
        });
    let cx = (x0 + x1) / 2.0;
    for f in [0.32f32, 0.76] {
        let hub = (cx, y0 + (y1 - y0) * f);
        if grid.contains(hub.0, hub.1) {
            points.push(hub);
        }
    }
}

/// Delaunay over the points, then everything outside the shape carved away
/// and points no triangle uses dropped.
fn triangulate(points: Vec<(f32, f32)>, grid: &Grid) -> Mesh {
    // triangulate, then carve away whatever falls outside the shape
    let mut triangles: Vec<[u16; 3]> = Vec::new();
    for t in delaunay(&points) {
        let (Some(a), Some(b), Some(c)) = (points.get(t[0]), points.get(t[1]), points.get(t[2]))
        else {
            continue;
        };
        let cx = (a.0 + b.0 + c.0) / 3.0;
        let cy = (a.1 + b.1 + c.1) / 3.0;
        // the centroid decides, with a cell of allowance so the outline ring
        // stays whole; triangles spanning a real gap still land far outside
        if grid.near(cx, cy) {
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
/// (u0, v0, u1, v1). Pixels outside the patch belong to neighbouring layers
/// (imported layers sit two pixels apart), so they read as empty and every
/// point is clamped inside: a triangle reaching past the patch would sample
/// the next layer's art and smear it. Returns uvs and triangle indices.
pub fn for_patch(
    alpha_at: &dyn Fn(u32, u32) -> u8,
    page: (u32, u32),
    patch: (f32, f32, f32, f32),
    p: &Params,
) -> Option<(Vec<f32>, Vec<u16>)> {
    let (tw, th) = (page.0 as f32, page.1 as f32);
    let (u0, v0, u1, v1) = patch;
    if !(u0 < u1 && v0 < v1) || tw < 1.0 || th < 1.0 {
        return None;
    }
    // the patch, padded so the margin has room to work out the outline
    let pad = p.margin + 2.0;
    let x0 = (u0 * tw - pad).max(0.0).floor() as u32;
    let y0 = (v0 * th - pad).max(0.0).floor() as u32;
    let x1 = (u1 * tw + pad).min(tw).ceil() as u32;
    let y1 = (v1 * th + pad).min(th).ceil() as u32;
    let (w, h) = (x1.saturating_sub(x0) as usize, y1.saturating_sub(y0) as usize);
    let (tx0, ty0, tx1, ty1) =
        ((u0 * tw).floor(), (v0 * th).floor(), (u1 * tw).ceil(), (v1 * th).ceil());
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
            [(x0 as f32 + x).clamp(tx0, tx1) / tw, (y0 as f32 + y).clamp(ty0, ty1) / th]
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
    fn a_face_is_two_edge_rings_fanned_into_two_hubs() {
        let (w, h) = (300, 380);
        let alpha = ellipse(w, h);
        let face = Params {
            outline_spacing: 18.0,
            inner_spacing: 24.0,
            margin: 4.0,
            style: Style::Face,
            ..Params::default()
        };
        let m = generate(&alpha, w, h, &face);
        assert!(!m.triangles.is_empty());
        // the hubs: the two points most triangles meet at, on the centre line
        let mut used = vec![0usize; m.points.len()];
        for t in &m.triangles {
            for &v in t {
                used[v as usize] += 1;
            }
        }
        let mut by_use: Vec<usize> = (0..m.points.len()).collect();
        by_use.sort_by_key(|&i| std::cmp::Reverse(used[i]));
        let hubs = [by_use[0], by_use[1]];
        for hub in hubs {
            let (x, _) = m.points[hub];
            assert!((x - w as f32 / 2.0).abs() < 4.0, "a hub sits on the centre line");
            assert!(used[hub] >= 8, "a hub fans out to many triangles");
        }
        // everything else lies near the edge: two rings, no lattice inside
        let (cx, cy, rx, ry) = (w as f32 / 2.0, h as f32 / 2.0, w as f32 * 0.4, h as f32 * 0.42);
        let inside = m
            .points
            .iter()
            .enumerate()
            .filter(|(i, _)| !hubs.contains(i))
            .filter(|&(_, &(x, y))| ((x - cx) / rx).powi(2) + ((y - cy) / ry).powi(2) < 0.7)
            .count();
        assert_eq!(inside, 0, "no points deep inside but the hubs");
        // Cubism's lattice fills the inside instead
        let lattice = generate(&alpha, w, h, &Params { style: Style::Lattice, ..face });
        assert!(lattice.points.len() > m.points.len());
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
    fn empty_and_degenerate_inputs_are_harmless() {
        assert!(generate(&[], 0, 0, &Params::default()).triangles.is_empty());
        assert!(generate(&[0; 100], 10, 10, &Params::default()).triangles.is_empty());
        assert!(generate(&[255; 4], 10, 10, &Params::default()).triangles.is_empty());
    }

    #[test]
    fn patch_mesh_stays_on_its_layer() {
        // two opaque layers side by side in a 64x32 page, two pixels apart:
        // A covers x 0..30, B covers x 32..64
        let alpha_at = |x: u32, _y: u32| if !(30..32).contains(&x) { 255u8 } else { 0 };
        let patch = (0.0, 0.0, 30.0 / 64.0, 1.0);
        let p = Params { margin: 6.0, ..Params::default() };
        let (uvs, idx) = for_patch(&alpha_at, (64, 32), patch, &p).unwrap();
        assert!(!idx.is_empty());
        for uv in uvs.chunks(2) {
            assert!(uv[0] <= 30.0 / 64.0 + 1e-6, "point at u {} reaches layer B", uv[0]);
        }
    }
}
