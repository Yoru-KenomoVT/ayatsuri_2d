//! Blend shapes: keyforms added on top of an object's normal blend instead
//! of multiplied into it (Cubism 5.0, editor manual "Blend Shape").
//!
//! A blend shape parameter has a base key at 0.0 that holds no keyform and
//! keys either side of it that do. The weight is 0% at the base key and
//! rises to 100% at a key; between two keys it splits linearly, and past the
//! outermost key the value holds that key's form. What lands on the object
//! is `weight * (key form - base form)`, the base form being the object's
//! blend at every parameter's default. Several blend shapes simply add up.

/// Up to two keyforms of one blend shape and their weights. An entry with
/// weight 0.0 is unused.
pub type Weights = [(usize, f32); 2];

/// Which of a blend shape's keyforms `value` lands between, and how much of
/// each. `keys` are the keys that hold a keyform (strictly increasing, never
/// 0.0); the base key at 0.0 sits among them and contributes nothing.
pub fn weights(value: f32, keys: &[f32]) -> Weights {
    let mut out = [(0, 0.0); 2];
    // stops are the keys with the base key slotted in at `split`
    let split = keys.partition_point(|&k| k < 0.0);
    let stop = |s: usize| -> f32 {
        match s.cmp(&split) {
            core::cmp::Ordering::Less => keys.get(s).copied().unwrap_or(0.0),
            core::cmp::Ordering::Equal => 0.0,
            core::cmp::Ordering::Greater => keys.get(s - 1).copied().unwrap_or(0.0),
        }
    };
    let key_of = |s: usize| -> Option<usize> {
        match s.cmp(&split) {
            core::cmp::Ordering::Less => Some(s),
            core::cmp::Ordering::Equal => None,
            core::cmp::Ordering::Greater => Some(s - 1),
        }
    };
    let last = keys.len();
    let (a, b, t) = if value.is_nan() || value <= stop(0) {
        (0, 0, 0.0)
    } else if value >= stop(last) {
        (last, last, 0.0)
    } else {
        let mut s = 0;
        while s + 1 < last && value >= stop(s + 1) {
            s += 1;
        }
        let (lo, hi) = (stop(s), stop(s + 1));
        (s, s + 1, if hi > lo { (value - lo) / (hi - lo) } else { 0.0 })
    };
    let mut n = 0;
    for (s, w) in [(a, 1.0 - t), (b, t)] {
        if w > 0.0
            && let Some(k) = key_of(s)
            && let Some(slot) = out.get_mut(n)
        {
            *slot = (k, w);
            n += 1;
        }
    }
    out
}

/// Add `weight * (form - base)` onto `out`, float by float.
pub fn add(out: &mut [f32], form: &[f32], base: &[f32], weight: f32) {
    for ((o, f), b) in out.iter_mut().zip(form).zip(base) {
        *o += weight * (f - b);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn used(w: Weights) -> Vec<(usize, f32)> {
        w.into_iter().filter(|&(_, w)| w > 0.0).collect()
    }

    #[test]
    fn base_key_adds_nothing() {
        assert!(used(weights(0.0, &[1.0])).is_empty());
        assert!(used(weights(-0.5, &[1.0])).is_empty());
        assert!(used(weights(0.3, &[])).is_empty());
    }

    #[test]
    fn rises_to_the_key_and_holds_past_it() {
        assert_eq!(used(weights(0.5, &[1.0])), vec![(0, 0.5)]);
        assert_eq!(used(weights(1.0, &[1.0])), vec![(0, 1.0)]);
        assert_eq!(used(weights(2.0, &[1.0])), vec![(0, 1.0)]);
    }

    #[test]
    fn splits_between_two_keys() {
        assert_eq!(used(weights(0.75, &[0.5, 1.0])), vec![(0, 0.5), (1, 0.5)]);
        assert_eq!(used(weights(0.25, &[0.5, 1.0])), vec![(0, 0.5)]);
    }

    #[test]
    fn keys_below_the_base_work_the_same() {
        assert_eq!(used(weights(-0.5, &[-1.0, 1.0])), vec![(0, 0.5)]);
        assert_eq!(used(weights(0.5, &[-1.0, 1.0])), vec![(1, 0.5)]);
        assert!(used(weights(0.0, &[-1.0, 1.0])).is_empty());
    }

    #[test]
    fn adds_the_difference() {
        // the worked example: normal blend (11, 0), Smile at 0.5 toward an
        // octagon corner (10, 3) drawn over the base (10, 0)
        let mut out = [11.0, 0.0];
        add(&mut out, &[10.0, 3.0], &[10.0, 0.0], 0.5);
        assert_eq!(out, [11.0, 1.5]);
        // a second shape stacks on the first
        add(&mut out, &[12.0, 0.0], &[10.0, 0.0], 1.0);
        assert_eq!(out, [13.0, 1.5]);
    }
}
