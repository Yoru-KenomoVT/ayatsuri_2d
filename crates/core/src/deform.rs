//! Deformer transforms: the free-form warp lattice and the rotation deformer.
//!
//! Ported from PurismCore (MIT), the verified clean-room runtime. A warp's
//! children give their coordinates in the warp's normalized [0,1] x [0,1]
//! domain; the warp's control grid lives in its parent's domain. Inside the
//! grid a point interpolates within its cell (bilinear in quad mode, split
//! triangles otherwise); outside, a virtual cell extrapolates near the grid
//! and an affine basis takes over far from it.

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Vec2 {
    pub x: f32,
    pub y: f32,
}

#[inline]
pub fn v2(x: f32, y: f32) -> Vec2 {
    Vec2 { x, y }
}

impl Vec2 {
    #[inline]
    fn add(self, o: Vec2) -> Vec2 {
        v2(self.x + o.x, self.y + o.y)
    }
    #[inline]
    fn sub(self, o: Vec2) -> Vec2 {
        v2(self.x - o.x, self.y - o.y)
    }
    #[inline]
    fn scale(self, s: f32) -> Vec2 {
        v2(self.x * s, self.y * s)
    }
}

/// `pos[2i], pos[2i+1]` as a point; out of range reads as the origin, which
/// load-time validation makes unreachable.
#[inline]
fn load(pos: &[f32], i: usize) -> Vec2 {
    let at = i.saturating_mul(2);
    v2(pos.get(at).copied().unwrap_or(0.0), pos.get(at.saturating_add(1)).copied().unwrap_or(0.0))
}

#[inline]
fn bary3(a: Vec2, b: Vec2, c: Vec2, wa: f32, wb: f32, wc: f32) -> Vec2 {
    v2(a.x * wa + b.x * wb + c.x * wc, a.y * wa + b.y * wb + c.y * wc)
}

#[inline]
fn bilinear(p00: Vec2, p10: Vec2, p01: Vec2, p11: Vec2, fu: f32, fv: f32) -> Vec2 {
    let top = p00.add(p10.sub(p00).scale(fu));
    let bottom = p01.add(p11.sub(p01).scale(fu));
    top.add(bottom.sub(top).scale(fv))
}

struct Cell {
    fu: f32,
    fv: f32,
    p00: Vec2,
    p10: Vec2,
    p01: Vec2,
    p11: Vec2,
}

/// The diagonal split the editor uses for non-quad cells.
#[inline]
fn interp_triangle(c: &Cell) -> Vec2 {
    if c.fu + c.fv <= 1.0 {
        bary3(c.p00, c.p10, c.p01, 1.0 - c.fu - c.fv, c.fu, c.fv)
    } else {
        bary3(c.p10, c.p11, c.p01, 1.0 - c.fv, c.fu + c.fv - 1.0, 1.0 - c.fu)
    }
}

/// Affine frame used outside the grid: average center and edge derivatives.
struct Basis {
    center: Vec2,
    dpdu: Vec2,
    dpdv: Vec2,
}

fn extrap_basis(pos: &[f32], rows: usize, cols: usize, stride: usize) -> Basis {
    let c00 = load(pos, 0);
    let c10 = load(pos, cols);
    let c01 = load(pos, rows.saturating_mul(stride));
    let c11 = load(pos, rows.saturating_mul(stride).saturating_add(cols));
    let d11_00 = c11.sub(c00);
    let d10_01 = c10.sub(c01);
    let dpdv = d11_00.sub(d10_01).scale(0.5);
    let dpdu = d10_01.add(d11_00).scale(0.5);
    let sum = c00.add(c10).add(c01.add(c11));
    let center = sum.scale(0.25).sub(d11_00.scale(0.5));
    Basis { center, dpdu, dpdv }
}

/// Build the virtual cell for a near-exterior point, octant by octant.
#[allow(clippy::too_many_lines)]
fn extrap_cell(
    u: f32,
    v: f32,
    gu: f32,
    gv: f32,
    rows: usize,
    cols: usize,
    stride: usize,
    pos: &[f32],
    b: &Basis,
) -> Cell {
    let (fr, fc) = (rows as f32, cols as f32);
    let (cen, du, dv) = (b.center, b.dpdu, b.dpdv);

    let mut cu = 0usize;
    let mut cv = 0usize;
    let (mut uc, mut un, mut vc, mut vn) = (0.0f32, 0.0f32, 0.0f32, 0.0f32);
    let fu;
    let fv;

    if u <= 0.0 {
        fu = (u + 2.0) * 0.5;
    } else if u >= 1.0 {
        fu = (u - 1.0) * 0.5;
    } else {
        cu = (gu as usize).min(cols.saturating_sub(1));
        fu = gu - cu as f32;
        uc = cu as f32 / fc;
        un = (cu + 1) as f32 / fc;
    }

    if v <= 0.0 {
        fv = (v + 2.0) * 0.5;
    } else if v >= 1.0 {
        fv = (v - 1.0) * 0.5;
    } else {
        cv = (gv as usize).min(rows.saturating_sub(1));
        fv = gv - cv as f32;
        vc = cv as f32 / fr;
        vn = (cv + 1) as f32 / fr;
    }

    let row_at = |r: usize| r.saturating_mul(stride);
    let (p00, p10, p01, p11);
    if u <= 0.0 {
        if v <= 0.0 {
            p00 = cen.sub(dv.scale(2.0).add(du.scale(2.0)));
            p10 = cen.sub(dv.scale(2.0));
            p01 = cen.sub(du.scale(2.0));
            p11 = load(pos, 0);
        } else if v < 1.0 {
            p00 = cen.sub(du.scale(2.0)).add(dv.scale(vc));
            p10 = load(pos, row_at(cv));
            p01 = cen.sub(du.scale(2.0)).add(dv.scale(vn));
            p11 = load(pos, row_at(cv + 1));
        } else {
            p00 = cen.sub(du.scale(2.0)).add(dv);
            p10 = load(pos, row_at(rows));
            p01 = cen.sub(du.scale(2.0)).add(dv.scale(3.0));
            p11 = cen.add(dv.scale(3.0));
        }
    } else if u < 1.0 {
        if v <= 0.0 {
            p00 = du.scale(uc).add(cen.sub(dv.scale(2.0)));
            p10 = du.scale(un).add(cen.sub(dv.scale(2.0)));
            p01 = load(pos, cu);
            p11 = load(pos, cu + 1);
        } else {
            p00 = load(pos, row_at(rows).saturating_add(cu));
            p10 = load(pos, row_at(rows).saturating_add(cu + 1));
            p01 = cen.add(du.scale(uc)).add(dv.scale(3.0));
            p11 = cen.add(du.scale(un)).add(dv.scale(3.0));
        }
    } else if v <= 0.0 {
        p00 = cen.sub(dv.scale(2.0)).add(du);
        p10 = cen.sub(dv.scale(2.0)).add(du.scale(3.0));
        p01 = load(pos, cols);
        p11 = cen.add(du.scale(3.0));
    } else if v < 1.0 {
        p00 = load(pos, row_at(cv).saturating_add(cols));
        p10 = cen.add(du.scale(3.0)).add(dv.scale(vc));
        p01 = load(pos, row_at(cv + 1).saturating_add(cols));
        p11 = cen.add(du.scale(3.0)).add(dv.scale(vn));
    } else {
        p00 = load(pos, row_at(rows).saturating_add(cols));
        p10 = cen.add(du.scale(3.0)).add(dv);
        p01 = cen.add(dv.scale(3.0)).add(du);
        p11 = cen.add(du.scale(3.0).add(dv.scale(3.0)));
    }

    Cell { fu, fv, p00, p10, p01, p11 }
}

/// Map points through a warp grid. `pos` is the deformed control grid
/// ((rows+1) x (cols+1) points, row-major, in the parent's space); inputs are
/// normalized grid coordinates. In place: `io` holds xy pairs.
///
/// The hot path, a point inside the grid, reads its cell as two 2-point
/// slices of the grid (one bounds check per row instead of one per float),
/// so the loop stays branch-light; a cell that would read past the grid,
/// which load-time validation makes unreachable, falls back to the checked
/// reads. Outside the grid nothing changed.
pub fn warp_transform(pos: &[f32], rows: usize, cols: usize, quad: bool, io: &mut [f32]) {
    let stride = cols.saturating_add(1);
    let (fr, fc) = (rows as f32, cols as f32);
    let (grid, _) = pos.as_chunks::<2>();
    let (pts, _) = io.as_chunks_mut::<2>();
    let (last_col, last_row) = (cols.saturating_sub(1), rows.saturating_sub(1));
    let mut basis: Option<Basis> = None;
    for p in pts {
        let [u, v] = *p;
        let (gu, gv) = (u * fc, v * fr);
        let r = if (0.0..1.0).contains(&u) && (0.0..1.0).contains(&v) {
            let cu = (gu as usize).min(last_col);
            let cv = (gv as usize).min(last_row);
            let (fu, fv) = (gu - cu as f32, gv - cv as f32);
            let bi = cv.saturating_mul(stride).saturating_add(cu);
            let below = bi.saturating_add(stride);
            let (p00, p10, p01, p11) = match (
                grid.get(bi..bi.saturating_add(2)),
                grid.get(below..below.saturating_add(2)),
            ) {
                (Some(&[a, b]), Some(&[c, d])) => {
                    (v2(a[0], a[1]), v2(b[0], b[1]), v2(c[0], c[1]), v2(d[0], d[1]))
                }
                _ => (load(pos, bi), load(pos, bi + 1), load(pos, below), load(pos, below + 1)),
            };
            if quad {
                bilinear(p00, p10, p01, p11, fu, fv)
            } else {
                interp_triangle(&Cell { fu, fv, p00, p10, p01, p11 })
            }
        } else {
            let b = basis.get_or_insert_with(|| extrap_basis(pos, rows, cols, stride));
            if u > -2.0 && u < 3.0 && v > -2.0 && v < 3.0 {
                interp_triangle(&extrap_cell(u, v, gu, gv, rows, cols, stride, pos, b))
            } else {
                v2(
                    b.dpdu.x * u + b.center.x + b.dpdv.x * v,
                    b.dpdu.y * u + b.center.y + b.dpdv.y * v,
                )
            }
        };
        *p = [r.x, r.y];
    }
}

/// A rotation deformer's resolved state, after parent transforms.
#[derive(Debug, Clone, Copy, Default)]
pub struct Rotation {
    pub origin: Vec2,
    /// Degrees, base angle included.
    pub angle: f32,
    pub scale: f32,
    pub reflect_x: bool,
    pub reflect_y: bool,
}

/// Map points through a rotation deformer. In place: `io` holds xy pairs in
/// the deformer's local space.
pub fn rotation_transform(r: &Rotation, io: &mut [f32]) {
    let rad = r.angle.to_radians();
    let (sin_a, cos_a) = rad.sin_cos();
    let rx = if r.reflect_x { -1.0 } else { 1.0 };
    let ry = if r.reflect_y { -1.0 } else { 1.0 };
    let m00 = r.scale * cos_a * rx;
    let m01 = r.scale * -sin_a * ry;
    let m10 = r.scale * sin_a * rx;
    let m11 = r.scale * cos_a * ry;
    let count = io.len() / 2;
    for i in 0..count {
        let at = i * 2;
        let x = io.get(at).copied().unwrap_or(0.0);
        let y = io.get(at + 1).copied().unwrap_or(0.0);
        if let Some(s) = io.get_mut(at) {
            *s = r.origin.x + m00 * x + m01 * y;
        }
        if let Some(s) = io.get_mut(at + 1) {
            *s = r.origin.y + m10 * x + m11 * y;
        }
    }
}

/// The signed angle of `a` minus the angle of `b` (that is, FROM `b` TO `a`),
/// wrapped to [-pi, pi]. This matches the reference runtime's convention; the
/// rotation-deformer probe negates it, so flipping this sign turns every
/// nested rotation the wrong way.
pub fn signed_angle(a: Vec2, b: Vec2) -> f32 {
    (b.x * a.y - b.y * a.x).atan2(b.x * a.x + b.y * a.y)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The previous implementation, kept to check the fast path against.
    /// Map points through a warp grid. `pos` is the deformed control grid
    /// ((rows+1) x (cols+1) points, row-major, in the parent's space); inputs are
    /// normalized grid coordinates. In place: `io` holds xy pairs.
    fn warp_transform_reference(pos: &[f32], rows: usize, cols: usize, quad: bool, io: &mut [f32]) {
        let stride = cols.saturating_add(1);
        let (fr, fc) = (rows as f32, cols as f32);
        let mut basis: Option<Basis> = None;

        let count = io.len() / 2;
        for i in 0..count {
            let at = i * 2;
            let u = io.get(at).copied().unwrap_or(0.0);
            let v = io.get(at + 1).copied().unwrap_or(0.0);
            let (gu, gv) = (u * fc, v * fr);

            let r = if (0.0..1.0).contains(&u) && (0.0..1.0).contains(&v) {
                let cu = (gu as usize).min(cols.saturating_sub(1));
                let cv = (gv as usize).min(rows.saturating_sub(1));
                let (fu, fv) = (gu - cu as f32, gv - cv as f32);
                let bi = cv.saturating_mul(stride).saturating_add(cu);
                let p00 = load(pos, bi);
                let p10 = load(pos, bi + 1);
                let p01 = load(pos, bi + stride);
                let p11 = load(pos, bi + stride + 1);
                if quad {
                    bilinear(p00, p10, p01, p11, fu, fv)
                } else {
                    interp_triangle(&Cell { fu, fv, p00, p10, p01, p11 })
                }
            } else {
                let b = basis.get_or_insert_with(|| extrap_basis(pos, rows, cols, stride));
                if u > -2.0 && u < 3.0 && v > -2.0 && v < 3.0 {
                    interp_triangle(&extrap_cell(u, v, gu, gv, rows, cols, stride, pos, b))
                } else {
                    v2(
                        b.dpdu.x * u + b.center.x + b.dpdv.x * v,
                        b.dpdu.y * u + b.center.y + b.dpdv.y * v,
                    )
                }
            };
            if let Some(s) = io.get_mut(at) {
                *s = r.x;
            }
            if let Some(s) = io.get_mut(at + 1) {
                *s = r.y;
            }
        }
    }

    /// A small pseudo-random generator, so the test needs no crate.
    fn rng(seed: &mut u64) -> f32 {
        *seed ^= *seed << 13;
        *seed ^= *seed >> 7;
        *seed ^= *seed << 17;
        (*seed >> 40) as f32 / (1u64 << 24) as f32
    }

    #[test]
    fn the_fast_path_matches_the_old_warp_exactly() {
        let mut seed = 0x2545_f491_4f6c_dd1du64;
        for (rows, cols) in [(1, 1), (2, 3), (5, 5), (4, 7), (9, 2)] {
            for quad in [true, false] {
                let grid: Vec<f32> =
                    (0..(rows + 1) * (cols + 1) * 2).map(|_| rng(&mut seed) * 4.0 - 2.0).collect();
                // inside, on the edges, near outside and far outside
                let io: Vec<f32> = (0..4000).map(|_| rng(&mut seed) * 7.0 - 3.0).collect();
                let mut a = io.clone();
                let mut b = io.clone();
                warp_transform(&grid, rows, cols, quad, &mut a);
                warp_transform_reference(&grid, rows, cols, quad, &mut b);
                assert!(
                    a.iter().zip(&b).all(|(x, y)| x.to_bits() == y.to_bits()),
                    "{rows}x{cols} quad {quad}"
                );
            }
        }
    }

    /// An identity grid: control points at their rest positions over [0,1]^2.
    fn identity_grid(rows: usize, cols: usize) -> Vec<f32> {
        let mut g = Vec::new();
        for r in 0..=rows {
            for c in 0..=cols {
                g.push(c as f32 / cols as f32);
                g.push(r as f32 / rows as f32);
            }
        }
        g
    }

    #[test]
    fn identity_warp_is_identity_inside() {
        let grid = identity_grid(2, 3);
        let mut io = vec![0.25, 0.5, 0.9, 0.1, 0.0, 0.0];
        let want = io.clone();
        warp_transform(&grid, 2, 3, true, &mut io);
        for (a, b) in io.iter().zip(&want) {
            assert!((a - b).abs() < 1e-5, "{io:?} vs {want:?}");
        }
    }

    #[test]
    fn translated_warp_translates() {
        let mut grid = identity_grid(1, 1);
        for (i, g) in grid.iter_mut().enumerate() {
            if i % 2 == 0 {
                *g += 10.0;
            }
        }
        let mut io = vec![0.5, 0.5];
        warp_transform(&grid, 1, 1, true, &mut io);
        assert!((io[0] - 10.5).abs() < 1e-5 && (io[1] - 0.5).abs() < 1e-5);
    }

    #[test]
    fn exterior_extrapolates_continuously() {
        let grid = identity_grid(2, 2);
        // just outside the left edge: identity grid extrapolates to itself
        let mut io = vec![-0.1, 0.5];
        warp_transform(&grid, 2, 2, false, &mut io);
        assert!((io[0] - -0.1).abs() < 1e-4 && (io[1] - 0.5).abs() < 1e-4, "{io:?}");
        // far field
        let mut io = vec![-5.0, 0.5];
        warp_transform(&grid, 2, 2, false, &mut io);
        assert!((io[0] - -5.0).abs() < 1e-4, "{io:?}");
    }

    #[test]
    fn rotation_rotates_about_origin() {
        let r = Rotation {
            origin: v2(1.0, 1.0),
            angle: 90.0,
            scale: 2.0,
            reflect_x: false,
            reflect_y: false,
        };
        let mut io = vec![1.0, 0.0];
        rotation_transform(&r, &mut io);
        // (1,0) scaled to (2,0), rotated 90 degrees to (0,2), moved to origin
        assert!((io[0] - 1.0).abs() < 1e-5 && (io[1] - 3.0).abs() < 1e-5, "{io:?}");
    }
}
