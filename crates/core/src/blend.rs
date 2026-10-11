//! Keyform selection: which keyforms a parameter value lands between, and the
//! 2^N corner weights that blend them.
//!
//! Semantics follow PurismCore (MIT), the verified clean-room runtime: values
//! snap to a key within `snap_eps` (10^-decimals of the parameter), a segment
//! shorter than `interp_eps` does not interpolate, and a value outside the key
//! range disables the object rather than clamping.

use ayatsuri_format::model::Binding;

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
        if seg.weight != 0.0 {
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
        let binding =
            Binding { axes: vec![BindingAxis { parameter: 0, keys: vec![0.0, 1.0, 2.0] }] };
        let mut out = Vec::new();
        let ok = corners(&binding, |_| 1.0, |_| EPS, &mut out);
        assert!(ok);
        assert_eq!(out.as_slice(), &[(1, 1.0)]);
    }

    #[test]
    fn out_of_range_disables() {
        let binding = Binding { axes: vec![BindingAxis { parameter: 0, keys: vec![0.0, 1.0] }] };
        let mut out = Vec::new();
        assert!(!corners(&binding, |_| 5.0, |_| EPS, &mut out));
        assert!(out.is_empty());
    }
}
