//! Playing scenes: each curve is sampled at a time. Interpolation lives on
//! the segment after a key, as in Cubism and motion3.json: auto smooth,
//! linear, stepped, inverse stepped or a free bezier. Values hold flat
//! before the first key and after the last.

use ayatsuri_format::{Animation, Curve, CurveTarget, Interp, Key};

/// The out handle and the into handle a segment really uses, each relative
/// to its own key: auto smooth eases (flat, a third of the way in), a free
/// bezier keeps its handles but inside the segment's span, so time along
/// the curve never runs backwards.
pub fn handles(k0: &Key, k1: &Key) -> ((f32, f32), (f32, f32)) {
    let span = (k1.time - k0.time).max(0.0);
    match k0.interp {
        Interp::Bezier { out, into } => {
            ((out.0.clamp(0.0, span), out.1), (into.0.clamp(-span, 0.0), into.1))
        }
        _ => ((span / 3.0, 0.0), (-span / 3.0, 0.0)),
    }
}

fn cubic(p0: f32, p1: f32, p2: f32, p3: f32, u: f32) -> f32 {
    let v = 1.0 - u;
    v * v * v * p0 + 3.0 * v * v * u * p1 + 3.0 * v * u * u * p2 + u * u * u * p3
}

/// The curve parameter whose time is `x`: Newton steps, falling back to
/// bisection, on a time polynomial that only ever rises.
fn solve_u(p0: f32, p1: f32, p2: f32, p3: f32, x: f32) -> f32 {
    let (mut lo, mut hi) = (0.0f32, 1.0f32);
    let mut u = if (p3 - p0).abs() > 1e-9 { ((x - p0) / (p3 - p0)).clamp(0.0, 1.0) } else { 0.5 };
    for _ in 0..24 {
        let at = cubic(p0, p1, p2, p3, u) - x;
        if at.abs() < 1e-6 {
            break;
        }
        if at > 0.0 {
            hi = u;
        } else {
            lo = u;
        }
        let v = 1.0 - u;
        let slope = 3.0 * v * v * (p1 - p0) + 6.0 * v * u * (p2 - p1) + 3.0 * u * u * (p3 - p2);
        let next = if slope.abs() > 1e-9 { u - at / slope } else { f32::NAN };
        u = if next.is_finite() && next > lo && next < hi { next } else { 0.5 * (lo + hi) };
    }
    u
}

/// The value of the segment from `k0` to `k1` at time `t`.
/// `restricted` evaluates beziers the way motion3's AreBeziersRestricted
/// asks: handles sit at 1/3 and 2/3 of the span, so the curve parameter is
/// just the time fraction.
pub fn segment(k0: &Key, k1: &Key, t: f32, restricted: bool) -> f32 {
    let span = k1.time - k0.time;
    if span <= 1e-9 {
        return k1.value;
    }
    let f = ((t - k0.time) / span).clamp(0.0, 1.0);
    match k0.interp {
        Interp::Linear => k0.value + (k1.value - k0.value) * f,
        Interp::Stepped => k0.value,
        Interp::InverseStepped => {
            if t <= k0.time {
                k0.value
            } else {
                k1.value
            }
        }
        Interp::Auto | Interp::Bezier { .. } => {
            let (out, into) = handles(k0, k1);
            let (v1, v2) = (k0.value + out.1, k1.value + into.1);
            let u = if restricted || matches!(k0.interp, Interp::Auto) {
                f
            } else {
                solve_u(k0.time, k0.time + out.0, k1.time + into.0, k1.time, t)
            };
            cubic(k0.value, v1, v2, k1.value, u)
        }
    }
}

/// The curve's value at `t` seconds, or None when it has no keys.
pub fn sample_curve(curve: &Curve, t: f32, restricted: bool) -> Option<f32> {
    let first = curve.keys.first()?;
    if t <= first.time {
        return Some(first.value);
    }
    // keys are sorted, so the segment holding t is found by binary search
    let next = curve.keys.partition_point(|k| k.time <= t);
    match (curve.keys.get(next.wrapping_sub(1)), curve.keys.get(next)) {
        (Some(k0), Some(k1)) => Some(segment(k0, k1, t, restricted)),
        (Some(k0), None) => Some(k0.value),
        _ => Some(first.value),
    }
}

/// A scene curve's value at `t`, with the scene's bezier mode.
pub fn sample(anim: &Animation, curve: &Curve, t: f32) -> Option<f32> {
    sample_curve(curve, t, anim.restricted_beziers)
}

/// Insert or replace the key at `t` (within half a frame), keeping keys
/// sorted. A new key takes `interp`; a replaced one keeps its own.
pub fn set_key(curve: &mut Curve, t: f32, value: f32, fps: f32, interp: Interp) {
    let tol = 0.5 / fps.max(1.0);
    if let Some(k) = curve.keys.iter_mut().find(|k| (k.time - t).abs() <= tol) {
        k.value = value;
        return;
    }
    let at = curve.keys.partition_point(|k| k.time < t);
    curve.keys.insert(at, Key { time: t, value, interp });
}

/// Remove the key at `t` (within half a frame). True when one went.
pub fn remove_key(curve: &mut Curve, t: f32, fps: f32) -> bool {
    let tol = 0.5 / fps.max(1.0);
    let before = curve.keys.len();
    curve.keys.retain(|k| (k.time - t).abs() > tol);
    curve.keys.len() != before
}

/// The curve driving `target`/`id`, created on first use.
pub fn curve_mut<'a>(
    anim: &'a mut Animation,
    target: CurveTarget,
    id: &str,
) -> Option<&'a mut Curve> {
    if !anim.curves.iter().any(|c| c.target == target && c.id == id) {
        anim.curves.push(Curve { target, id: id.to_string(), ..Curve::default() });
    }
    anim.curves.iter_mut().find(|c| c.target == target && c.id == id)
}

/// The parameter curve for `param`, if the scene has one.
pub fn param_curve<'a>(anim: &'a Animation, param: &str) -> Option<&'a Curve> {
    anim.curves.iter().find(|c| c.target == CurveTarget::Parameter && c.id == param)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(time: f32, value: f32, interp: Interp) -> Key {
        Key { time, value, interp }
    }

    fn curve(keys: Vec<Key>) -> Curve {
        Curve { id: "A".into(), keys, ..Curve::default() }
    }

    #[test]
    fn linear_holds_ends() {
        let c = curve(vec![
            key(0.0, 0.0, Interp::Linear),
            key(1.0, 10.0, Interp::Linear),
            key(2.0, 0.0, Interp::Linear),
        ]);
        assert_eq!(sample_curve(&c, -1.0, false), Some(0.0));
        assert_eq!(sample_curve(&c, 0.5, false), Some(5.0));
        assert_eq!(sample_curve(&c, 1.5, false), Some(5.0));
        assert_eq!(sample_curve(&c, 9.0, false), Some(0.0));
        assert_eq!(sample_curve(&curve(vec![]), 0.0, false), None);
    }

    #[test]
    fn stepped_and_inverse_stepped() {
        let s = curve(vec![key(0.0, 1.0, Interp::Stepped), key(1.0, 5.0, Interp::Linear)]);
        assert_eq!(sample_curve(&s, 0.99, false), Some(1.0));
        assert_eq!(sample_curve(&s, 1.0, false), Some(5.0));
        let i = curve(vec![key(0.0, 1.0, Interp::InverseStepped), key(1.0, 5.0, Interp::Linear)]);
        assert_eq!(sample_curve(&i, 0.0, false), Some(1.0));
        assert_eq!(sample_curve(&i, 0.01, false), Some(5.0));
    }

    #[test]
    fn auto_eases_with_flat_ends() {
        let c = curve(vec![key(0.0, 0.0, Interp::Auto), key(1.0, 1.0, Interp::Linear)]);
        let mid = sample_curve(&c, 0.5, false).unwrap();
        assert!((mid - 0.5).abs() < 1e-5, "{mid}");
        // slow out of the first key: well under linear early on
        assert!(sample_curve(&c, 0.1, false).unwrap() < 0.05);
    }

    #[test]
    fn free_bezier_solves_for_time() {
        // handles pulled hard toward the start: the value rises early
        let c = curve(vec![
            key(0.0, 0.0, Interp::Bezier { out: (0.05, 1.0), into: (-0.9, 0.0) }),
            key(1.0, 1.0, Interp::Linear),
        ]);
        let solved = sample_curve(&c, 0.2, false).unwrap();
        let restricted = sample_curve(&c, 0.2, true).unwrap();
        assert!(solved > restricted, "{solved} vs {restricted}");
        // the ends always land on the keys
        assert!((sample_curve(&c, 1.0, false).unwrap() - 1.0).abs() < 1e-6);
    }

    #[test]
    fn keys_insert_sorted_and_replace() {
        let mut a = Animation { fps: 30.0, ..Animation::default() };
        let c = curve_mut(&mut a, CurveTarget::Parameter, "A").unwrap();
        set_key(c, 1.0, 1.0, 30.0, Interp::Auto);
        set_key(c, 0.0, 0.0, 30.0, Interp::Linear);
        set_key(c, 1.01, 5.0, 30.0, Interp::Stepped);
        assert_eq!(c.keys.len(), 2);
        assert_eq!((c.keys[1].value, c.keys[1].interp), (5.0, Interp::Auto));
        assert!(remove_key(c, 0.0, 30.0));
        assert_eq!(c.keys.len(), 1);
        assert_eq!(a.curves.len(), 1);
        curve_mut(&mut a, CurveTarget::PartOpacity, "A");
        assert_eq!(a.curves.len(), 2);
    }
}
