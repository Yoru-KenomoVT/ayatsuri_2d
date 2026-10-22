//! Keyform selection: which keyforms a parameter value lands between, and the
//! 2^N corner weights that blend them.
//!
//! Semantics follow PurismCore (MIT), the verified clean-room runtime: values
//! snap to a key within `snap_eps` (10^-decimals of the parameter), a segment
//! shorter than `interp_eps` does not interpolate, and a value outside the key
//! range disables the object rather than clamping.

use ayatsuri_format::model::{Binding, CurveKind};

/// Where a value sits on one axis's keys.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Segment {
    /// Lower key index.
    pub index: usize,
    /// 0.0 at `index`, approaching 1.0 at `index + 1`.
    pub weight: f32,
    /// The value is outside the keys entirely: the object is off.
    pub outside: bool,
}

/// Find the key segment holding `value`. Keys are strictly increasing.
pub fn key_segment(value: f32, keys: &[f32], snap_eps: f32, interp_eps: f32) -> Segment {
    let mut seg = Segment { index: 0, weight: 0.0, outside: false };
    let Some(&first) = keys.first() else {
        seg.outside = true;
        return seg;
    };
    if keys.len() == 1 {
        seg.outside = (value <= first - snap_eps) || (value >= first + snap_eps);
        return seg;
    }
    if value < first - snap_eps {
        seg.outside = true;
        return seg;
    }
    if value < first + snap_eps {
        return seg;
    }
    let mut lo = first;
    for (k, &hi) in keys.iter().enumerate().skip(1) {
        if value < hi - snap_eps {
            seg.index = k - 1;
            let diff = hi - lo;
            if diff >= interp_eps {
                seg.weight = (value - lo) / diff;
            }
            return seg;
        }
        if value < hi + snap_eps {
            seg.index = k;
            return seg;
        }
        lo = hi;
    }
    seg.outside = true;
    seg
}

/// One blended keyform and its weight.
pub type Corner = (usize, f32);

/// Quadratic (Lagrange) weights of keys `a`, `a+1`, `a+2` at value `x`: the
/// parabola through the three keyforms.
fn quadratic(keys: &[f32], a: usize, x: f32) -> Option<[(usize, f32); 3]> {
    let (k0, k1, k2) = (*keys.get(a)?, *keys.get(a + 1)?, *keys.get(a + 2)?);
    let (d01, d02, d12) = (k0 - k1, k0 - k2, k1 - k2);
    if d01.abs() < 1e-9 || d02.abs() < 1e-9 || d12.abs() < 1e-9 {
        return None;
    }
    Some([
        (a, (x - k1) * (x - k2) / (d01 * d02)),
        (a + 1, (x - k0) * (x - k2) / (-d01 * d12)),
        (a + 2, (x - k0) * (x - k1) / (d02 * d12)),
    ])
}

/// Extended interpolation's weights for a value `t` of the way through the
/// span from key `i` to `i + 1`: a smooth curve through the keyforms instead
/// of the straight line, so a sway between turned keys keeps its size. The
/// two parabolas either side of the span are blended across it, which keeps
/// the motion smooth through every key. Ellipse's `scale` pushes the curve
/// further from (above 1) or back toward (below 1) the straight line. Needs
/// three keys; fewer stay linear.
pub fn curve_weights(keys: &[f32], i: usize, t: f32, kind: CurveKind, out: &mut Vec<Corner>) {
    out.clear();
    out.push((i, 1.0 - t));
    out.push((i + 1, t));
    let (Some(&lo), Some(&hi)) = (keys.get(i), keys.get(i + 1)) else { return };
    if keys.len() < 3 {
        return;
    }
    let x = lo + (hi - lo) * t;
    let left = if i >= 1 { quadratic(keys, i - 1, x) } else { None };
    let right = quadratic(keys, i, x);
    let mut curve: Vec<Corner> = Vec::with_capacity(4);
    let mut add = |w: &[(usize, f32)], f: f32| {
        for &(k, v) in w {
            match curve.iter_mut().find(|c| c.0 == k) {
                Some(c) => c.1 += v * f,
                None => curve.push((k, v * f)),
            }
        }
    };
    match (left, right) {
        (Some(l), Some(r)) => {
            add(&l, 1.0 - t);
            add(&r, t);
        }
        (Some(l), None) => add(&l, 1.0),
        (None, Some(r)) => add(&r, 1.0),
        (None, None) => return,
    }
    let scale = match kind {
        CurveKind::Ellipse { scale, .. } => scale,
        CurveKind::Sns { .. } => 1.0,
    };
    // the straight line plus `scale` times the curve's bend away from it
    for c in out.iter_mut() {
        c.1 *= 1.0 - scale;
    }
    for &(k, v) in &curve {
        match out.iter_mut().find(|c| c.0 == k) {
            Some(c) => c.1 += v * scale,
            None => out.push((k, v * scale)),
        }
    }
}

/// Build the 2^N keyform corners for a binding at the given parameter values.
///
/// The combo construction mirrors the verified runtime: keyform index strides
/// multiply by each axis's key count in axis order (first axis fastest), and
/// an axis sitting exactly on a key contributes one corner instead of two.
/// Returns false when any axis is out of range: the object is disabled.
pub fn corners(
    binding: &Binding,
    value_of: impl Fn(usize) -> f32,
    eps_of: impl Fn(usize) -> (f32, f32),
    out: &mut Vec<Corner>,
) -> bool {
    out.clear();
    out.push((0, 1.0));
    let mut index_stride = 1usize;
    for axis in &binding.axes {
        let p = axis.parameter as usize;
        let (snap_eps, interp_eps) = eps_of(p);
        let seg = key_segment(value_of(p), &axis.keys, snap_eps, interp_eps);
        if seg.outside {
            out.clear();
            return false;
        }
        let base = seg.index.saturating_mul(index_stride);
        if seg.weight != 0.0
            && axis.keys.len() >= 3
            && let Some(kind) = binding.curve(axis.parameter)
        {
            // extended interpolation: three or four keys share the weight
            let mut ws = Vec::with_capacity(4);
            curve_weights(&axis.keys, seg.index, seg.weight, kind, &mut ws);
            let n = out.len();
            let mut next = Vec::with_capacity(n * ws.len());
            for &(i0, w0) in out.iter().take(n) {
                for &(k, w) in &ws {
                    next.push((i0.saturating_add(k.saturating_mul(index_stride)), w0 * w));
                }
            }
            *out = next;
        } else if seg.weight != 0.0 {
            let next = seg.index.saturating_add(1).saturating_mul(index_stride);
            let n = out.len();
            for k in 0..n {
                let Some(&(i0, w0)) = out.get(k) else { break };
                if let Some(c) = out.get_mut(k) {
                    *c = (i0.saturating_add(base), w0 * (1.0 - seg.weight));
                }
                out.push((i0.saturating_add(next), w0 * seg.weight));
            }
        } else {
            for c in out.iter_mut() {
                c.0 = c.0.saturating_add(base);
            }
        }
        index_stride = index_stride.saturating_mul(axis.keys.len().max(1));
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use ayatsuri_format::model::BindingAxis;

    const EPS: (f32, f32) = (0.001, 0.0015);

    #[test]
    fn segment_finds_midpoints_and_snaps() {
        let keys = [-30.0, 0.0, 30.0];
        let s = key_segment(15.0, &keys, EPS.0, EPS.1);
        assert_eq!(s.index, 1);
        assert!((s.weight - 0.5).abs() < 1e-6);
        assert!(!s.outside);

        let s = key_segment(0.0, &keys, EPS.0, EPS.1);
        assert_eq!((s.index, s.weight), (1, 0.0));

        assert!(key_segment(31.0, &keys, EPS.0, EPS.1).outside);
        assert!(key_segment(-31.0, &keys, EPS.0, EPS.1).outside);
    }

    #[test]
    fn corners_blend_two_axes() {
        let binding = Binding {
            curves: Vec::new(),
            axes: vec![
                BindingAxis { parameter: 0, keys: vec![0.0, 1.0] },
                BindingAxis { parameter: 1, keys: vec![0.0, 1.0] },
            ],
        };
        let mut out = Vec::new();
        // axis 0 at 0.25, axis 1 at 0.5: four corners, weights multiply
        let ok = corners(&binding, |p| if p == 0 { 0.25 } else { 0.5 }, |_| EPS, &mut out);
        assert!(ok);
        assert_eq!(out.len(), 4);
        let total: f32 = out.iter().map(|c| c.1).sum();
        assert!((total - 1.0).abs() < 1e-6);
        // keyform layout: axis 0 fastest => indices 0,1 (axis0) then +2 per axis1 key
        let w = |i: usize| out.iter().find(|c| c.0 == i).map(|c| c.1).unwrap();
        assert!((w(0) - 0.75 * 0.5).abs() < 1e-6);
        assert!((w(1) - 0.25 * 0.5).abs() < 1e-6);
        assert!((w(2) - 0.75 * 0.5).abs() < 1e-6);
        assert!((w(3) - 0.25 * 0.5).abs() < 1e-6);
    }

    #[test]
    fn corners_on_key_collapse() {
        let binding = Binding {
            curves: Vec::new(),
            axes: vec![BindingAxis { parameter: 0, keys: vec![0.0, 1.0, 2.0] }],
        };
        let mut out = Vec::new();
        let ok = corners(&binding, |_| 1.0, |_| EPS, &mut out);
        assert!(ok);
        assert_eq!(out.as_slice(), &[(1, 1.0)]);
    }

    #[test]
    fn out_of_range_disables() {
        let binding = Binding {
            curves: Vec::new(),
            axes: vec![BindingAxis { parameter: 0, keys: vec![0.0, 1.0] }],
        };
        let mut out = Vec::new();
        assert!(!corners(&binding, |_| 5.0, |_| EPS, &mut out));
        assert!(out.is_empty());
    }
}
