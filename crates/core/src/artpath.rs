//! ArtPaths: Bezier paths drawn on the canvas that become ordinary ArtMeshes.
//!
//! A path is sampled into a polyline, then turned into triangles: a ribbon
//! for the stroke and an ear-clipped polygon for the fill, each with a thin
//! ring outside it that fades to clear. Colour comes from a swatch page:
//!
//! ```text
//!   swatch page (256 x 256), 8 x 8 cells of 32 px, one colour per cell
//!   inside a cell, alpha peaks in the middle and is clear at both edges:
//!        ░░▒▒▓▓████▓▓▒▒░░      x=16 opaque, x=4 and x=28 clear
//!   a vertex's uv picks the cell and how far out along the fade it sits:
//!   0 on the shape itself (x=16), 1 on the outer edge of the soft ring
//!   (x=28). Clear edges matter: zoomed out, the GPU's smaller mip levels
//!   blend neighbouring texels, and a clear edge only ever meets another.
//! ```
//!
//! Since the result is a plain mesh, it clips, deforms, takes keyforms and
//! exports to moc3 like any other.

use ayatsuri_format::model::ArtPath;

/// The swatch page's size in pixels, and a colour cell's.
pub const SWATCH_SIZE: u32 = 256;
pub const SWATCH_CELL: u32 = 32;
/// Cells on the page: 8 x 8.
pub const SWATCH_CELLS: usize =
    ((SWATCH_SIZE / SWATCH_CELL) * (SWATCH_SIZE / SWATCH_CELL)) as usize;
/// Where the fade starts (opaque, the cell's middle) and how far out it
/// runs to clear, in pixels.
const FADE_FROM: f32 = 16.0;
const FADE_LEN: f32 = 12.0;
/// Meshes index with u16; stop well short of it.
const MAX_POINTS: usize = 12_000;

/// What part of the path a vertex belongs to, and how far out in the fade.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Role {
    pub fill: bool,
    /// 0 on the shape, 1 at the outer edge of the soft ring.
    pub fade: f32,
}

/// A path's triangles in canvas pixels, before uvs.
#[derive(Debug, Clone, Default)]
pub struct Built {
    pub points: Vec<[f32; 2]>,
    pub roles: Vec<Role>,
    pub indices: Vec<u16>,
}

impl Built {
    fn push(&mut self, p: [f32; 2], fill: bool, fade: f32) -> u16 {
        self.points.push(p);
        self.roles.push(Role { fill, fade });
        (self.points.len() - 1) as u16
    }

    fn tri(&mut self, a: u16, b: u16, c: u16) {
        self.indices.extend([a, b, c]);
    }

    fn quad(&mut self, a: u16, b: u16, c: u16, d: u16) {
        // a-b on one side, d-c on the other
        self.tri(a, b, c);
        self.tri(a, c, d);
    }
}

fn anchor(a: &[f32; 6]) -> ([f32; 2], [f32; 2], [f32; 2]) {
    let [x, y, ix, iy, ox, oy] = *a;
    ([x, y], [x + ix, y + iy], [x + ox, y + oy])
}

fn cubic(p0: [f32; 2], p1: [f32; 2], p2: [f32; 2], p3: [f32; 2], t: f32) -> [f32; 2] {
    let u = 1.0 - t;
    let (a, b, c, d) = (u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t);
    [a * p0[0] + b * p1[0] + c * p2[0] + d * p3[0], a * p0[1] + b * p1[1] + c * p2[1] + d * p3[1]]
}

fn dist(a: [f32; 2], b: [f32; 2]) -> f32 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2)).sqrt()
}

/// The path as a polyline, a point every `step` pixels or so along each
/// curve. A closed path does not repeat its first point at the end.
pub fn sample(anchors: &[[f32; 6]], closed: bool, step: f32) -> Vec<[f32; 2]> {
    let step = step.max(0.5);
    let n = anchors.len();
    let mut out: Vec<[f32; 2]> = Vec::new();
    let segments = if closed && n > 2 { n } else { n.saturating_sub(1) };
    for i in 0..segments {
        let (Some(a), Some(b)) = (anchors.get(i), anchors.get((i + 1) % n.max(1))) else {
            continue;
        };
        let (p0, _, p1) = anchor(a);
        let (p3, p2, _) = anchor(b);
        // the control polygon's length bounds the curve's
        let reach = dist(p0, p1) + dist(p1, p2) + dist(p2, p3);
        let count = ((reach / step).ceil() as usize).clamp(1, 256);
        for k in 0..count {
            out.push(cubic(p0, p1, p2, p3, k as f32 / count as f32));
        }
    }
    if !(closed && n > 2)
        && let Some(last) = anchors.last()
    {
        out.push([last[0], last[1]]);
    }
    // drop repeats, which have no direction
    out.dedup_by(|a, b| dist(*a, *b) < 1e-3);
    out.truncate(MAX_POINTS);
    out
}

fn normal(a: [f32; 2], b: [f32; 2]) -> [f32; 2] {
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let len = (dx * dx + dy * dy).sqrt().max(1e-6);
    [-dy / len, dx / len]
}

/// Per point: the direction to offset it by and how far (a mitre, limited
/// so sharp turns don't spike).
fn offsets(pts: &[[f32; 2]], closed: bool) -> Vec<([f32; 2], f32)> {
    let n = pts.len();
    (0..n)
        .map(|i| {
            let here = pts.get(i).copied().unwrap_or_default();
            let prev = if i > 0 {
                pts.get(i - 1).copied()
            } else if closed {
                pts.last().copied()
            } else {
                None
            };
            let next = if i + 1 < n {
                pts.get(i + 1).copied()
            } else if closed {
                pts.first().copied()
            } else {
                None
            };
            let a = prev.map(|p| normal(p, here));
            let b = next.map(|q| normal(here, q));
            match (a, b) {
                (Some(a), Some(b)) => {
                    let (sx, sy) = (a[0] + b[0], a[1] + b[1]);
                    let len = (sx * sx + sy * sy).sqrt();
                    if len < 1e-4 {
                        return (a, 1.0);
                    }
                    let m = [sx / len, sy / len];
                    let cos = (m[0] * a[0] + m[1] * a[1]).max(0.35);
                    (m, 1.0 / cos)
                }
                (Some(a), None) | (None, Some(a)) => (a, 1.0),
                (None, None) => ([0.0, -1.0], 1.0),
            }
        })
        .collect()
}

fn along(p: [f32; 2], d: [f32; 2], k: f32) -> [f32; 2] {
    [p[0] + d[0] * k, p[1] + d[1] * k]
}

/// Twice the signed area: positive when the points turn counter-clockwise
/// in a y-down canvas... the sign only has to be consistent.
fn area2(pts: &[[f32; 2]]) -> f32 {
    let n = pts.len();
    (0..n)
        .map(|i| {
            let a = pts.get(i).copied().unwrap_or_default();
            let b = pts.get((i + 1) % n).copied().unwrap_or_default();
            a[0] * b[1] - b[0] * a[1]
        })
        .sum()
}

fn cross(o: [f32; 2], a: [f32; 2], b: [f32; 2]) -> f32 {
    (a[0] - o[0]) * (b[1] - o[1]) - (a[1] - o[1]) * (b[0] - o[0])
}

fn inside_tri(p: [f32; 2], a: [f32; 2], b: [f32; 2], c: [f32; 2]) -> bool {
    let (d1, d2, d3) = (cross(a, b, p), cross(b, c, p), cross(c, a, p));
    let neg = d1 < 0.0 || d2 < 0.0 || d3 < 0.0;
    let pos = d1 > 0.0 || d2 > 0.0 || d3 > 0.0;
    !(neg && pos)
}

/// Ear clipping: triangles over a simple polygon, as index triples into
/// `pts`. A self-crossing outline fills what it can, then fans the rest.
pub fn triangulate(pts: &[[f32; 2]]) -> Vec<[usize; 3]> {
    let mut ring: Vec<usize> = (0..pts.len()).collect();
    if area2(pts) < 0.0 {
        ring.reverse();
    }
    let at = |i: usize| pts.get(i).copied().unwrap_or_default();
    let mut out = Vec::new();
    let mut guard = 0usize;
    while ring.len() > 3 && guard < pts.len() * pts.len() + 8 {
        guard += 1;
        let n = ring.len();
        let mut cut = None;
        for k in 0..n {
            let (Some(&ia), Some(&ib), Some(&ic)) =
                (ring.get((k + n - 1) % n), ring.get(k), ring.get((k + 1) % n))
            else {
                continue;
            };
            let (a, b, c) = (at(ia), at(ib), at(ic));
            if cross(a, b, c) <= 0.0 {
                continue; // reflex corner
            }
            let blocked =
                ring.iter().any(|&j| j != ia && j != ib && j != ic && inside_tri(at(j), a, b, c));
            if !blocked {
                cut = Some((k, [ia, ib, ic]));
                break;
            }
        }
        match cut {
            Some((k, t)) => {
                out.push(t);
                ring.remove(k);
            }
            None => break,
        }
    }
    // whatever is left (a triangle, or a tangle): fan it
    if let Some(&first) = ring.first() {
        for w in ring.windows(2).skip(1) {
            if let [b, c] = *w {
                out.push([first, b, c]);
            }
        }
    }
    out
}

/// A path's geometry in canvas pixels: the fill under the stroke, each with
/// its soft ring. Empty when there is nothing to draw.
pub fn build(path: &ArtPath) -> Built {
    let pts = sample(&path.anchors, path.closed, path.step);
    let mut out = Built::default();
    if pts.len() < 2 {
        return out;
    }
    let closed = path.closed && pts.len() > 2;
    let feather = path.feather.max(0.0);

    if path.fill && closed {
        let tris = triangulate(&pts);
        let base = out.points.len() as u16;
        for p in &pts {
            out.push(*p, true, 0.0);
        }
        for [a, b, c] in tris {
            out.tri(base + a as u16, base + b as u16, base + c as u16);
        }
        // the soft ring, outward (the side away from the inside)
        let covered = path.stroke && path.width >= 1.0;
        if feather > 0.0 && !covered {
            let offs = offsets(&pts, true);
            let out_sign = if area2(&pts) > 0.0 { -1.0 } else { 1.0 };
            let ring: Vec<u16> = pts
                .iter()
                .zip(&offs)
                .map(|(p, (d, k))| out.push(along(*p, *d, out_sign * feather * k), true, 1.0))
                .collect();
            let n = pts.len();
            for i in 0..n {
                let j = (i + 1) % n;
                if let (Some(&ri), Some(&rj)) = (ring.get(i), ring.get(j)) {
                    out.quad(base + i as u16, base + j as u16, rj, ri);
                }
            }
        }
    }

    if path.stroke && path.width > 0.0 {
        let offs = offsets(&pts, closed);
        let half = path.width * 0.5;
        // per point: outer-left, inner-left, inner-right, outer-right
        let mut cols: Vec<[u16; 4]> = Vec::with_capacity(pts.len());
        for (p, (d, k)) in pts.iter().zip(&offs) {
            let ol = out.push(along(*p, *d, (half + feather) * k), false, 1.0);
            let il = out.push(along(*p, *d, half * k), false, 0.0);
            let ir = out.push(along(*p, *d, -half * k), false, 0.0);
            let or = out.push(along(*p, *d, -(half + feather) * k), false, 1.0);
            cols.push([ol, il, ir, or]);
        }
        let n = cols.len();
        let spans = if closed { n } else { n - 1 };
        for i in 0..spans {
            let (Some(a), Some(b)) = (cols.get(i).copied(), cols.get((i + 1) % n).copied()) else {
                continue;
            };
            out.quad(a[0], b[0], b[1], a[1]);
            out.quad(a[1], b[1], b[2], a[2]);
            out.quad(a[2], b[2], b[3], a[3]);
        }
        // soft ends on an open stroke
        if !closed && feather > 0.0 {
            for (end, toward) in [(0usize, 1usize), (n - 1, n - 2)] {
                let (Some(p), Some(q), Some(c)) =
                    (pts.get(end).copied(), pts.get(toward).copied(), cols.get(end).copied())
                else {
                    continue;
                };
                // away from the line, past its end
                let len = dist(p, q).max(1e-6);
                let back = [(p[0] - q[0]) / len, (p[1] - q[1]) / len];
                let cap: Vec<u16> = c
                    .iter()
                    .map(|&v| {
                        let at = out.points.get(v as usize).copied().unwrap_or(p);
                        out.push(along(at, back, feather), false, 1.0)
                    })
                    .collect();
                if let [c0, c1, c2, c3] = cap[..] {
                    out.quad(c[0], c[1], c1, c0);
                    out.quad(c[1], c[2], c2, c1);
                    out.quad(c[2], c[3], c3, c2);
                }
            }
        }
    }
    out
}

/// The swatch cell of a colour: its uv for a vertex `fade` of the way out.
pub fn swatch_uv(cell: usize, fade: f32) -> [f32; 2] {
    let per_row = (SWATCH_SIZE / SWATCH_CELL) as usize;
    let (cx, cy) = ((cell % per_row) as f32, (cell / per_row) as f32);
    let cell_px = SWATCH_CELL as f32;
    let x = cx * cell_px + FADE_FROM + fade.clamp(0.0, 1.0) * FADE_LEN;
    let y = cy * cell_px + cell_px * 0.5;
    [x / SWATCH_SIZE as f32, y / SWATCH_SIZE as f32]
}

/// The swatch page's pixels (RGBA, straight alpha) for these colours.
pub fn swatch_rgba(colors: &[[u8; 4]]) -> Vec<u8> {
    let size = SWATCH_SIZE as usize;
    let cell = SWATCH_CELL as usize;
    let per_row = size / cell;
    let mut px = vec![0u8; size * size * 4];
    for (i, c) in colors.iter().take(SWATCH_CELLS).enumerate() {
        let (ox, oy) = ((i % per_row) * cell, (i / per_row) * cell);
        for y in oy..oy + cell {
            for x in 0..cell {
                // alpha follows the fade out from the middle; the colour
                // fills the whole cell so a blend toward clear keeps it
                // fully opaque for the two texels the middle sits between
                let d = (x as f32 + 0.5 - FADE_FROM).abs();
                let t = ((d - 0.5) / (FADE_LEN - 0.5)).clamp(0.0, 1.0);
                let a = (c[3] as f32 * (1.0 - t)).round() as u8;
                let at = (y * size + ox + x) * 4;
                if let Some(slot) = px.get_mut(at..at + 4) {
                    slot.copy_from_slice(&[c[0], c[1], c[2], a]);
                }
            }
        }
    }
    px
}

#[cfg(test)]
mod tests {
    use super::*;

    fn square(fill: bool, stroke: bool) -> ArtPath {
        ArtPath {
            anchors: vec![
                [0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
                [100.0, 0.0, 0.0, 0.0, 0.0, 0.0],
                [100.0, 100.0, 0.0, 0.0, 0.0, 0.0],
                [0.0, 100.0, 0.0, 0.0, 0.0, 0.0],
            ],
            closed: true,
            fill,
            stroke,
            step: 1000.0,
            ..ArtPath::default()
        }
    }

    #[test]
    fn a_filled_square_is_two_triangles_and_a_ring() {
        let b = build(&square(true, false));
        // 4 shape points, 4 ring points; 2 fill triangles, 8 ring triangles
        assert_eq!(b.points.len(), 8);
        assert_eq!(b.indices.len(), (2 + 8) * 3);
        assert!(b.roles.iter().take(4).all(|r| r.fill && r.fade == 0.0));
        // the ring sits outside the square
        assert!(
            b.points
                .iter()
                .skip(4)
                .all(|p| p[0] < 0.0 || p[0] > 100.0 || p[1] < 0.0 || p[1] > 100.0)
        );
    }

    #[test]
    fn a_concave_outline_triangulates_without_spilling() {
        // an L shape: the fill must not cover the notch
        let pts = [[0.0, 0.0], [2.0, 0.0], [2.0, 1.0], [1.0, 1.0], [1.0, 2.0], [0.0, 2.0]];
        let tris = triangulate(&pts);
        assert_eq!(tris.len(), 4);
        let area: f32 =
            tris.iter().map(|[a, b, c]| cross(pts[*a], pts[*b], pts[*c]).abs() / 2.0).sum();
        assert!((area - 3.0).abs() < 1e-4, "area {area}");
    }

    #[test]
    fn an_open_stroke_has_soft_ends() {
        let p = ArtPath {
            anchors: vec![[0.0, 0.0, 0.0, 0.0, 0.0, 0.0], [50.0, 0.0, 0.0, 0.0, 0.0, 0.0]],
            step: 1000.0,
            ..ArtPath::default()
        };
        let b = build(&p);
        // 2 columns of 4, plus 4 cap points at each end
        assert_eq!(b.points.len(), 16);
        assert!(b.points.iter().any(|q| q[0] < 0.0) && b.points.iter().any(|q| q[0] > 50.0));
        assert!(b.indices.iter().all(|&i| (i as usize) < b.points.len()));
    }

    #[test]
    fn swatch_cells_fade_along_x() {
        let px = swatch_rgba(&[[10, 20, 30, 255], [200, 0, 0, 255]]);
        let alpha = |x: usize| px[(16 * SWATCH_SIZE as usize + x) * 4 + 3];
        assert_eq!(alpha(16), 255);
        // both edges of a cell are clear, so neighbours never bleed
        for x in [0, 1, 2, 3, 29, 30, 31, 32, 33] {
            assert_eq!(alpha(x), 0, "x={x}");
        }
        let [u0, _] = swatch_uv(0, 0.0);
        let [u1, _] = swatch_uv(0, 1.0);
        assert!((u0 * 256.0 - 16.0).abs() < 0.01 && (u1 * 256.0 - 28.0).abs() < 0.01);
    }
}
