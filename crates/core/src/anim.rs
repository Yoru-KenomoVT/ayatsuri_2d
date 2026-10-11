//! Playing animation clips: each track is a parameter curve sampled at a
//! time. Linear between points, held flat past either end, which matches
//! what the clip format carries today.

use ayatsuri_format::{Animation, Track};

/// The track's value at `t` seconds, or None for an empty track.
pub fn sample(track: &Track, t: f32) -> Option<f32> {
    let first = track.points.first()?;
    if t <= first.0 {
        return Some(first.1);
    }
    let mut prev = *first;
    for &(time, value) in track.points.iter().skip(1) {
        if t <= time {
            let span = time - prev.0;
            if span <= 1e-9 {
                return Some(value);
            }
            let k = (t - prev.0) / span;
            return Some(prev.1 + (value - prev.1) * k);
        }
        prev = (time, value);
    }
    Some(prev.1)
}

/// Insert or replace the key at `t` (within half a frame), keeping the
/// points sorted.
pub fn set_key(track: &mut Track, t: f32, value: f32, fps: f32) {
    let tol = 0.5 / fps.max(1.0);
    if let Some(p) = track.points.iter_mut().find(|p| (p.0 - t).abs() <= tol) {
        p.1 = value;
        return;
    }
    let at = track.points.iter().position(|p| p.0 > t).unwrap_or(track.points.len());
    track.points.insert(at, (t, value));
}

/// Remove the key at `t` (within half a frame). True when one went.
pub fn remove_key(track: &mut Track, t: f32, fps: f32) -> bool {
    let tol = 0.5 / fps.max(1.0);
    let before = track.points.len();
    track.points.retain(|p| (p.0 - t).abs() > tol);
    track.points.len() != before
}

/// The track for a parameter, created on first use.
pub fn track_mut<'a>(anim: &'a mut Animation, param: &str) -> Option<&'a mut Track> {
    if !anim.tracks.iter().any(|t| t.param == param) {
        anim.tracks.push(Track { param: param.to_string(), points: Vec::new() });
    }
    anim.tracks.iter_mut().find(|t| t.param == param)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn samples_linear_and_holds_ends() {
        let t = Track { param: "A".into(), points: vec![(0.0, 0.0), (1.0, 10.0), (2.0, 0.0)] };
        assert_eq!(sample(&t, -1.0), Some(0.0));
        assert_eq!(sample(&t, 0.5), Some(5.0));
        assert_eq!(sample(&t, 1.5), Some(5.0));
        assert_eq!(sample(&t, 9.0), Some(0.0));
        assert_eq!(sample(&Track::default(), 0.0), None);
    }

    #[test]
    fn keys_insert_sorted_and_replace() {
        let mut a = Animation { fps: 30.0, ..Animation::default() };
        let t = track_mut(&mut a, "A").unwrap();
        set_key(t, 1.0, 1.0, 30.0);
        set_key(t, 0.0, 0.0, 30.0);
        set_key(t, 1.01, 5.0, 30.0);
        assert_eq!(t.points, vec![(0.0, 0.0), (1.0, 5.0)]);
        assert!(remove_key(t, 0.0, 30.0));
        assert_eq!(t.points.len(), 1);
    }
}
