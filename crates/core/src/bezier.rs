// Bezier control of a warp deformer's grid, the way Cubism edits warps: a
// coarse lattice of anchors (bw x bh patches) with a handle on each side
// of every anchor, shaping the fine division grid as bicubic Bezier
// patches. The lattice is not stored in the model; it is fitted to the
// grid when editing starts and the grid is rewritten from it.
//
// The control net is (3 bw + 1) x (3 bh + 1) points, row-major. Anchors sit
// at indices that are multiples of 3 on both axes; the handles are the net
// points one step away from an anchor along a row or a column; the four
// inner points of each patch are derived from their corner's anchor and
// handles (a parallelogram), so a patch bends with its handles.

/// Which net points a user can grab.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NetPoint {
    Anchor,
    Handle,
    Inner,
}

#[derive(Clone, Debug)]
pub struct Lattice {
    pub bw: usize,
    pub bh: usize,
    /// Net points, (3 bw + 1) per row, (3 bh + 1) rows.
    pub net: Vec<(f32, f32)>,
}

/// Bilinear sample of a warp grid of (cols + 1) x (rows + 1) points at
/// (u, v) in [0, 1].
fn sample_grid(grid: &[f32], cols: usize, rows: usize, u: f32, v: f32) -> (f32, f32) {
    let at = |c: usize, r: usize| -> (f32, f32) {
        let i = (r * (cols + 1) + c) * 2;
        (grid.get(i).copied().unwrap_or(0.0), grid.get(i + 1).copied().unwrap_or(0.0))
    };
    let fx = (u.clamp(0.0, 1.0) * cols as f32).min(cols as f32);
    let fy = (v.clamp(0.0, 1.0) * rows as f32).min(rows as f32);
    let (c0, r0) = (
        (fx.floor() as usize).min(cols.saturating_sub(1)),
        (fy.floor() as usize).min(rows.saturating_sub(1)),
    );
    let (s, t) = (fx - c0 as f32, fy - r0 as f32);
    let (a, b, c, d) = (at(c0, r0), at(c0 + 1, r0), at(c0, r0 + 1), at(c0 + 1, r0 + 1));
    let lerp =
        |p: (f32, f32), q: (f32, f32), k: f32| (p.0 + (q.0 - p.0) * k, p.1 + (q.1 - p.1) * k);
    lerp(lerp(a, b, s), lerp(c, d, s), t)
}

impl Lattice {
    pub fn net_w(&self) -> usize {
        3 * self.bw + 1
    }

    pub fn net_h(&self) -> usize {
        3 * self.bh + 1
    }

    pub fn kind(&self, i: usize) -> NetPoint {
        let w = self.net_w();
        let (x, y) = (i % w, i / w);
        match (x % 3 == 0, y % 3 == 0) {
            (true, true) => NetPoint::Anchor,
            (false, false) => NetPoint::Inner,
            _ => NetPoint::Handle,
        }
    }

    /// The anchor a handle belongs to (the nearer one along its line).
    pub fn anchor_of(&self, i: usize) -> usize {
        let w = self.net_w();
        let (x, y) = (i % w, i / w);
        let snap = |k: usize| if k % 3 == 2 { k + 1 } else { k - k % 3 };
        snap(y) * w + snap(x)
    }

    /// The handles of an anchor (up to four).
    pub fn handles_of(&self, anchor: usize) -> Vec<usize> {
        let (w, h) = (self.net_w(), self.net_h());
        let (x, y) = (anchor % w, anchor / w);
        let mut out = Vec::with_capacity(4);
        if x > 0 {
            out.push(y * w + x - 1);
        }
        if x + 1 < w {
            out.push(y * w + x + 1);
        }
        if y > 0 {
            out.push((y - 1) * w + x);
        }
        if y + 1 < h {
            out.push((y + 1) * w + x);
        }
        out
    }

    /// Fit a lattice to a warp grid: every net point samples the grid at
    /// its own parameter position, then the inner points are re-derived.
    pub fn fit(grid: &[f32], cols: usize, rows: usize, bw: usize, bh: usize) -> Self {
        let (bw, bh) = (bw.max(1), bh.max(1));
        let (w, h) = (3 * bw + 1, 3 * bh + 1);
        let mut net = Vec::with_capacity(w * h);
        for y in 0..h {
            for x in 0..w {
                let (u, v) = (x as f32 / (w - 1) as f32, y as f32 / (h - 1) as f32);
                net.push(sample_grid(grid, cols.max(1), rows.max(1), u, v));
            }
        }
        let mut l = Self { bw, bh, net };
        l.derive_inner();
        l
    }

    /// Recompute each patch's four inner points from its corners: inner =
    /// corner + its row handle offset + its column handle offset.
    pub fn derive_inner(&mut self) {
        let w = self.net_w();
        for py in 0..self.bh {
            for px in 0..self.bw {
                let (x0, y0) = (px * 3, py * 3);
                // each inner point leans on the nearest corner of its patch
                for (dx, dy) in [(1usize, 1usize), (2, 1), (1, 2), (2, 2)] {
                    let cx = if dx == 1 { x0 } else { x0 + 3 };
                    let cy = if dy == 1 { y0 } else { y0 + 3 };
                    let hx = if dx == 1 { x0 + 1 } else { x0 + 2 };
                    let hy = if dy == 1 { y0 + 1 } else { y0 + 2 };
                    let get =
                        |x: usize, y: usize| self.net.get(y * w + x).copied().unwrap_or((0.0, 0.0));
                    let c = get(cx, cy);
                    let along_x = get(hx, cy);
                    let along_y = get(cx, hy);
                    let p = (along_x.0 + along_y.0 - c.0, along_x.1 + along_y.1 - c.1);
                    if let Some(slot) = self.net.get_mut((y0 + dy) * w + x0 + dx) {
                        *slot = p;
                    }
                }
            }
        }
    }

    /// The surface at (u, v) in [0, 1].
    pub fn eval(&self, u: f32, v: f32) -> (f32, f32) {
        let w = self.net_w();
        let fu = (u.clamp(0.0, 1.0) * self.bw as f32).min(self.bw as f32);
        let fv = (v.clamp(0.0, 1.0) * self.bh as f32).min(self.bh as f32);
        let px = (fu.floor() as usize).min(self.bw - 1);
        let py = (fv.floor() as usize).min(self.bh - 1);
        let (s, t) = (fu - px as f32, fv - py as f32);
        let bern = |k: f32| {
            let m = 1.0 - k;
            [m * m * m, 3.0 * m * m * k, 3.0 * m * k * k, k * k * k]
        };
        let (bs, bt) = (bern(s), bern(t));
        let (mut x, mut y) = (0.0, 0.0);
        for (j, wt) in bt.iter().enumerate() {
            for (i, ws) in bs.iter().enumerate() {
                let p = self.net.get((py * 3 + j) * w + px * 3 + i).copied().unwrap_or((0.0, 0.0));
                x += p.0 * ws * wt;
                y += p.1 * ws * wt;
            }
        }
        (x, y)
    }

    /// The warp grid this lattice describes, (cols + 1) x (rows + 1) xy.
    pub fn to_grid(&self, cols: usize, rows: usize) -> Vec<f32> {
        let (cols, rows) = (cols.max(1), rows.max(1));
        let mut out = Vec::with_capacity((cols + 1) * (rows + 1) * 2);
        for r in 0..=rows {
            for c in 0..=cols {
                let (x, y) = self.eval(c as f32 / cols as f32, r as f32 / rows as f32);
                out.push(x);
                out.push(y);
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn identity(cols: usize, rows: usize) -> Vec<f32> {
        let mut g = Vec::new();
        for r in 0..=rows {
            for c in 0..=cols {
                g.push(c as f32 / cols as f32 * 4.0);
                g.push(r as f32 / rows as f32 * 2.0);
            }
        }
        g
    }

    #[test]
    fn fitting_a_flat_grid_changes_nothing() {
        let g = identity(5, 5);
        let l = Lattice::fit(&g, 5, 5, 2, 2);
        let back = l.to_grid(5, 5);
        for (a, b) in g.iter().zip(&back) {
            assert!((a - b).abs() < 1e-4, "{a} vs {b}");
        }
    }

    #[test]
    fn moving_an_anchor_bends_its_neighbourhood() {
        let g = identity(6, 6);
        let mut l = Lattice::fit(&g, 6, 6, 2, 2);
        // the centre anchor, with its handles, moves down by 0.5
        let w = l.net_w();
        let centre = 3 * w + 3;
        assert_eq!(l.kind(centre), NetPoint::Anchor);
        for i in std::iter::once(centre).chain(l.handles_of(centre)) {
            l.net[i].1 += 0.5;
        }
        l.derive_inner();
        let out = l.to_grid(6, 6);
        let y = |c: usize, r: usize| out[(r * 7 + c) * 2 + 1];
        // centre follows fully, corners stay, in between moves part way
        assert!((y(3, 3) - (1.0 + 0.5)).abs() < 1e-4);
        assert!((y(0, 0) - 0.0).abs() < 1e-4);
        let mid = y(2, 3) - 1.0;
        assert!(mid > 0.05 && mid < 0.5, "{mid}");
    }

    #[test]
    fn handles_belong_to_their_anchor() {
        let l = Lattice::fit(&identity(4, 4), 4, 4, 2, 2);
        let w = l.net_w();
        assert_eq!(l.anchor_of(4), 3);
        assert_eq!(l.anchor_of(2), 3);
        assert_eq!(l.anchor_of(w), 0);
        assert_eq!(l.kind(1), NetPoint::Handle);
        assert_eq!(l.kind(w + 1), NetPoint::Inner);
    }
}
