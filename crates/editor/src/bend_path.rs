//! A smooth path through control points, and points tied to it: how far
//! along it and how far to the side, so they bend with it the way Cubism's
//! deform paths carry the art.

use egui::{Vec2, vec2};

/// Samples per span between two control points; control point `i` is
/// sample `i * SPAN`.
pub const SPAN: usize = 24;

/// The path sampled finely, with a unit tangent at every sample.
pub struct Curve {
    pub pts: Vec<Vec2>,
    tan: Vec<Vec2>,
}

/// A point held by a curve: sample `k`, fraction `t` towards the next, and
/// its offset along and across the curve there.
#[derive(Clone, Copy, Debug, Default)]
pub struct Tie {
    k: usize,
    t: f32,
    along: f32,
    across: f32,
}

/// One span of a centripetal Catmull-Rom curve, p1 to p2 at `u` in 0..1.
/// Centripetal spacing keeps the curve from overshooting or looping where
/// the points sit unevenly, which the uniform kind does.
fn span_at(p0: Vec2, p1: Vec2, p2: Vec2, p3: Vec2, u: f32) -> Vec2 {
    let knot = |a: Vec2, b: Vec2| (b - a).length().sqrt().max(1e-4);
    let t1 = knot(p0, p1);
    let t2 = t1 + knot(p1, p2);
    let t3 = t2 + knot(p2, p3);
    let t = t1 + (t2 - t1) * u;
    let mix = |a: Vec2, b: Vec2, ta: f32, tb: f32| {
        a * ((tb - t) / (tb - ta)) + b * ((t - ta) / (tb - ta))
    };
    let a1 = mix(p0, p1, 0.0, t1);
    let a2 = mix(p1, p2, t1, t2);
    let a3 = mix(p2, p3, t2, t3);
    let b1 = mix(a1, a2, 0.0, t2);
    let b2 = mix(a2, a3, t1, t3);
    mix(b1, b2, t1, t2)
}

impl Curve {
    pub fn new(ctrl: &[Vec2]) -> Self {
        let n = ctrl.len();
        let pts = if n < 2 {
            ctrl.to_vec()
        } else {
            let at = |i: usize| ctrl.get(i).copied().unwrap_or(Vec2::ZERO);
            let mut out = Vec::with_capacity((n - 1) * SPAN + 1);
            for i in 0..n - 1 {
                let (p1, p2) = (at(i), at(i + 1));
                // past the ends the path carries straight on
                let p0 = if i == 0 { p1 * 2.0 - p2 } else { at(i - 1) };
                let p3 = if i + 2 >= n { p2 * 2.0 - p1 } else { at(i + 2) };
                for s in 0..SPAN {
                    out.push(span_at(p0, p1, p2, p3, s as f32 / SPAN as f32));
                }
            }
            out.push(at(n - 1));
            out
        };
        let m = pts.len();
        let mut tan: Vec<Vec2> = Vec::with_capacity(m);
        for i in 0..m {
            let a = pts.get(i.saturating_sub(1)).copied().unwrap_or(Vec2::ZERO);
            let b = pts.get((i + 1).min(m.saturating_sub(1))).copied().unwrap_or(Vec2::ZERO);
            let d = b - a;
            let prev = tan.last().copied().unwrap_or(Vec2::X);
            tan.push(if d.length_sq() > 1e-12 { d.normalized() } else { prev });
        }
        Self { pts, tan }
    }

    /// The point, tangent and normal at sample `k`, fraction `t`.
    fn frame(&self, k: usize, t: f32) -> (Vec2, Vec2, Vec2) {
        let get = |v: &[Vec2], i: usize| v.get(i).or(v.last()).copied().unwrap_or(Vec2::ZERO);
        let o = get(&self.pts, k) + (get(&self.pts, k + 1) - get(&self.pts, k)) * t;
        let mut tan = get(&self.tan, k) * (1.0 - t) + get(&self.tan, k + 1) * t;
        tan = if tan.length_sq() > 1e-12 { tan.normalized() } else { get(&self.tan, k) };
        (o, tan, vec2(-tan.y, tan.x))
    }

    /// The unit tangent at control point `i`.
    pub fn tangent_at(&self, i: usize) -> Vec2 {
        self.tan.get(i * SPAN).or(self.tan.last()).copied().unwrap_or(Vec2::X)
    }

    /// Tie `p` to the nearest place on the curve; past an end it rides the
    /// end's tangent.
    pub fn tie(&self, p: Vec2) -> Tie {
        if self.pts.len() < 2 {
            let d = self.pts.first().map_or(p, |&o| p - o);
            return Tie { k: 0, t: 0.0, along: d.x, across: d.y };
        }
        let mut best = (f32::MAX, 0, 0.0);
        for k in 0..self.pts.len() - 1 {
            let (Some(&a), Some(&b)) = (self.pts.get(k), self.pts.get(k + 1)) else {
                continue;
            };
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
        let (o, tan, nor) = self.frame(best.1, best.2);
        Tie { k: best.1, t: best.2, along: (p - o).dot(tan), across: (p - o).dot(nor) }
    }

    /// Where a tied point is on this curve.
    pub fn place(&self, tie: &Tie) -> Vec2 {
        if self.pts.len() < 2 {
            return self.pts.first().copied().unwrap_or(Vec2::ZERO) + vec2(tie.along, tie.across);
        }
        let (o, tan, nor) = self.frame(tie.k.min(self.pts.len() - 2), tie.t);
        o + tan * tie.along + nor * tie.across
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_curve_passes_through_every_control_point() {
        let ctrl = [vec2(0.0, 0.0), vec2(10.0, 4.0), vec2(30.0, 0.0), vec2(31.0, 9.0)];
        let c = Curve::new(&ctrl);
        for (i, p) in ctrl.iter().enumerate() {
            assert!((c.pts[i * SPAN] - *p).length() < 1e-4);
        }
    }

    #[test]
    fn a_tied_point_comes_back_where_it_was() {
        let c = Curve::new(&[vec2(0.0, 0.0), vec2(10.0, 5.0), vec2(20.0, 0.0)]);
        for p in [vec2(3.0, 6.0), vec2(-4.0, 1.0), vec2(25.0, -3.0), vec2(12.0, -8.0)] {
            assert!((c.place(&c.tie(p)) - p).length() < 1e-3, "{p:?}");
        }
    }

    #[test]
    fn uneven_points_do_not_loop() {
        // a short span next to a long one: the uniform kind overshoots here
        let c = Curve::new(&[vec2(0.0, 0.0), vec2(1.0, 0.0), vec2(40.0, 0.0)]);
        assert!(c.pts.iter().all(|p| p.x >= -1e-3 && p.x <= 40.0 + 1e-3 && p.y.abs() < 1e-3));
        assert!(c.pts.windows(2).all(|w| w[1].x >= w[0].x - 1e-4));
    }
}
