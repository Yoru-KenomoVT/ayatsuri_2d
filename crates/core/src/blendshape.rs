//! Blend shapes: keyforms added on top of an object's normal blend instead
//! of multiplied into it (Cubism 5.0, editor manual "Blend Shape").
//!
//! A blend shape parameter has a base key at 0.0 that holds no keyform and
//! keys either side of it that do. The weight is 0% at the base key and
//! rises to 100% at a key; between two keys it splits linearly, and past the
//! outermost key the value holds that key's form. What lands on the object
//! is `weight * (key form - base form)`, the base form being the object's
//! blend at every parameter's default. Several blend shapes simply add up.

use ayatsuri_format::model::{Parameter, WeightLimit};

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

/// What a blend shape's weight is multiplied by: the smallest of its limit
/// graphs at the current values (editor manual "Limit settings for blend
/// shape weights"), 1.0 with none. A parameter with no value reads its
/// default.
pub fn limit(params: &[Parameter], limits: &[WeightLimit], values: &[f32]) -> f32 {
    limits
        .iter()
        .map(|l| {
            let p = l.parameter as usize;
            let v = values.get(p).copied().or_else(|| params.get(p).map(|q| q.default));
            graph(&l.keys, &l.weights, v.unwrap_or(0.0))
        })
        .fold(1.0, f32::min)
}

/// A limit graph's weight at `value`: straight lines between its points,
/// the end points' weights past either end.
pub fn graph(keys: &[f32], weights: &[f32], value: f32) -> f32 {
    let at = |i: usize| (keys.get(i).copied(), weights.get(i).copied());
    let n = keys.len().min(weights.len());
    let (Some(first), Some(last)) = (keys.first(), keys.get(n.saturating_sub(1))) else {
        return 1.0;
    };
    let w = if value.is_nan() || value <= *first {
        weights.first().copied().unwrap_or(1.0)
    } else if value >= *last {
        weights.get(n - 1).copied().unwrap_or(1.0)
    } else {
        let i = keys.partition_point(|&k| k <= value).clamp(1, n - 1);
        match (at(i - 1), at(i)) {
            ((Some(k0), Some(w0)), (Some(k1), Some(w1))) if k1 > k0 => {
                w0 + (w1 - w0) * (value - k0) / (k1 - k0)
            }
            ((_, Some(w0)), _) => w0,
            _ => 1.0,
        }
    };
    w.clamp(0.0, 1.0)
}

/// The weight limit graph presets, in the manual's order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Preset {
    /// Always 100%.
    Straight1,
    /// 0% at the minimum up to 100% at the maximum.
    Straight2,
    /// 100% at the minimum down to 0% at the maximum.
    Straight3,
    /// 0%, 100% at the middle, 0%.
    Fold1,
    /// 0% to the middle, then up to 100%.
    Fold2,
    /// 100% down to 0% at the middle, 0% after.
    Fold3,
    /// 0% up to 100% at the middle, 100% after.
    Fold4,
    /// 100% to the middle, then down to 0%.
    Fold5,
    /// 0% then 100%, a step at the middle.
    Step1,
    /// 100% then 0%, a step at the middle.
    Step2,
}

impl Preset {
    pub const ALL: [Preset; 10] = [
        Preset::Straight1,
        Preset::Straight2,
        Preset::Straight3,
        Preset::Fold1,
        Preset::Fold2,
        Preset::Fold3,
        Preset::Fold4,
        Preset::Fold5,
        Preset::Step1,
        Preset::Step2,
    ];

    /// Cubism's names for them.
    pub fn name(self) -> &'static str {
        match self {
            Preset::Straight1 => "Straight line 1",
            Preset::Straight2 => "Straight line 2",
            Preset::Straight3 => "Straight line 3",
            Preset::Fold1 => "Fold line 1",
            Preset::Fold2 => "Fold line 2",
            Preset::Fold3 => "Fold line 3",
            Preset::Fold4 => "Fold line 4",
            Preset::Fold5 => "Fold line 5",
            Preset::Step1 => "Step 1",
            Preset::Step2 => "Step 2",
        }
    }

    /// The graph over a parameter running `min` to `max`: (keys, weights).
    /// A step jumps at the middle over the smallest gap two points may have.
    pub fn graph(self, min: f32, max: f32) -> (Vec<f32>, Vec<f32>) {
        let mid = (min + max) * 0.5;
        let step = (mid + MIN_GAP).min(max);
        let (keys, weights): (Vec<f32>, Vec<f32>) = match self {
            Preset::Straight1 => (vec![min, max], vec![1.0, 1.0]),
            Preset::Straight2 => (vec![min, max], vec![0.0, 1.0]),
            Preset::Straight3 => (vec![min, max], vec![1.0, 0.0]),
            Preset::Fold1 => (vec![min, mid, max], vec![0.0, 1.0, 0.0]),
            Preset::Fold2 => (vec![min, mid, max], vec![0.0, 0.0, 1.0]),
            Preset::Fold3 => (vec![min, mid, max], vec![1.0, 0.0, 0.0]),
            Preset::Fold4 => (vec![min, mid, max], vec![0.0, 1.0, 1.0]),
            Preset::Fold5 => (vec![min, mid, max], vec![1.0, 1.0, 0.0]),
            Preset::Step1 => (vec![min, mid, step, max], vec![0.0, 0.0, 1.0, 1.0]),
            Preset::Step2 => (vec![min, mid, step, max], vec![1.0, 1.0, 0.0, 0.0]),
        };
        (keys, weights)
    }

    /// The preset a graph over `min` to `max` is, if it is one.
    pub fn of(keys: &[f32], weights: &[f32], min: f32, max: f32) -> Option<Preset> {
        Preset::ALL.into_iter().find(|p| {
            let (k, w) = p.graph(min, max);
            k.len() == keys.len()
                && w.len() == weights.len()
                && k.iter().zip(keys).all(|(a, b)| (a - b).abs() < 1e-4)
                && w.iter().zip(weights).all(|(a, b)| (a - b).abs() < 1e-4)
        })
    }
}

/// The most points a limit graph may have.
pub const MAX_POINTS: usize = 5;
/// Two parameter values closer than this are the same point.
pub const MIN_GAP: f32 = 0.01;

/// Why a limit graph cannot be used.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GraphError {
    /// Fewer than 2 or more than [`MAX_POINTS`] points.
    Points,
    /// Values not increasing, or two within [`MIN_GAP`] of each other.
    Order,
    /// A value outside the parameter's range.
    Range,
    /// A weight outside 0% to 100%.
    Weight,
}

/// Check a limit graph for a parameter running `min` to `max`.
pub fn check_graph(keys: &[f32], weights: &[f32], min: f32, max: f32) -> Result<(), GraphError> {
    if keys.len() != weights.len() || !(2..=MAX_POINTS).contains(&keys.len()) {
        return Err(GraphError::Points);
    }
    let gap_short = |a: f32, b: f32| a.is_nan() || b.is_nan() || b - a < MIN_GAP - 1e-6;
    if keys.windows(2).any(|w| matches!(w, [a, b] if gap_short(*a, *b))) {
        return Err(GraphError::Order);
    }
    if keys.iter().any(|k| k.is_nan() || *k < min - 1e-5 || *k > max + 1e-5) {
        return Err(GraphError::Range);
    }
    if weights.iter().any(|w| !(0.0..=1.0).contains(w)) {
        return Err(GraphError::Weight);
    }
    Ok(())
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
    fn a_limit_graph_is_straight_lines_held_past_its_ends() {
        let (k, w) = (vec![0.0, 1.0], vec![1.0, 0.0]);
        assert_eq!(graph(&k, &w, -1.0), 1.0);
        assert_eq!(graph(&k, &w, 0.25), 0.75);
        assert_eq!(graph(&k, &w, 2.0), 0.0);
        let (k, w) = Preset::Fold1.graph(-1.0, 1.0);
        assert_eq!(graph(&k, &w, 0.0), 1.0);
        assert_eq!(graph(&k, &w, 0.5), 0.5);
        assert_eq!(graph(&[], &[], 0.5), 1.0);
    }

    #[test]
    fn the_smallest_limit_wins() {
        let p = |id: &str| Parameter {
            id: id.into(),
            min: 0.0,
            max: 1.0,
            default: 0.0,
            repeat: false,
            decimals: 3,
            blend_shape: true,
        };
        let params = [p("a"), p("b"), p("c")];
        let (k, w) = Preset::Straight3.graph(0.0, 1.0);
        let limits = [
            WeightLimit { parameter: 1, keys: k.clone(), weights: w.clone() },
            WeightLimit { parameter: 2, keys: k, weights: w },
        ];
        assert_eq!(limit(&params, &limits, &[1.0, 0.25, 0.5]), 0.5);
        assert_eq!(limit(&params, &[], &[1.0, 1.0, 1.0]), 1.0);
    }

    #[test]
    fn presets_pass_the_rules_and_are_recognised() {
        for p in Preset::ALL {
            let (k, w) = p.graph(-1.0, 1.0);
            assert_eq!(check_graph(&k, &w, -1.0, 1.0), Ok(()), "{p:?}");
            assert_eq!(Preset::of(&k, &w, -1.0, 1.0), Some(p));
        }
        assert_eq!(check_graph(&[0.0, 0.005], &[1.0, 0.0], 0.0, 1.0), Err(GraphError::Order));
        assert_eq!(check_graph(&[0.0], &[1.0], 0.0, 1.0), Err(GraphError::Points));
        assert_eq!(check_graph(&[0.0, 2.0], &[1.0, 0.0], 0.0, 1.0), Err(GraphError::Range));
        assert_eq!(check_graph(&[0.0, 1.0], &[1.5, 0.0], 0.0, 1.0), Err(GraphError::Weight));
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
