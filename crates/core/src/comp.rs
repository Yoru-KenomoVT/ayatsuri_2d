//! Playing compositions: scenes and other compositions laid out as layers
//! over one clock, the way After Effects nests comps. Each layer plays a
//! stretch of its source from a start time, faded in and out, and mixes
//! over the layers beneath it by its weight. Sampling a composition gives
//! each curve's blend as an affine of whatever the value was before, so a
//! player applies it over the pose and a bake writes it out as keys.

use std::collections::BTreeMap;

use ayatsuri_format::{
    Animation, Composition, Curve, CurveTarget, Interp, Key, Layer, LayerSource, Playlist,
};

/// How deep compositions may nest before sampling gives up, so a layer
/// that leads back to its own composition cannot run forever.
pub const MAX_DEPTH: usize = 8;

/// What a composition can reach: the project's scenes and compositions.
#[derive(Clone, Copy)]
pub struct Scope<'a> {
    pub scenes: &'a [Animation],
    pub comps: &'a [Composition],
}

impl<'a> Scope<'a> {
    pub fn scene(&self, name: &str) -> Option<&'a Animation> {
        self.scenes.iter().find(|a| a.name == name)
    }

    pub fn comp(&self, name: &str) -> Option<&'a Composition> {
        self.comps.iter().find(|c| c.name == name)
    }

    /// A source's length in its own seconds, if it exists.
    pub fn source_len(&self, src: &LayerSource) -> Option<f32> {
        match src {
            LayerSource::Scene(n) => self.scene(n).map(|a| a.duration),
            LayerSource::Composition(n) => self.comp(n).map(|c| c.duration),
            // sound has no length the core can know; its layer carries it
            LayerSource::Audio(_) => None,
        }
    }

    pub fn source_looped(&self, src: &LayerSource) -> bool {
        match src {
            LayerSource::Scene(n) => self.scene(n).is_some_and(|a| a.looped),
            LayerSource::Composition(n) => self.comp(n).is_some_and(|c| c.looped),
            LayerSource::Audio(_) => false,
        }
    }

    /// True when the composition `name` is `comp` or sits inside it at any
    /// depth, so adding it as a layer would loop back.
    pub fn reaches(&self, comp: &Composition, name: &str) -> bool {
        self.reaches_at(comp, name, 0)
    }

    fn reaches_at(&self, comp: &Composition, name: &str, depth: usize) -> bool {
        if comp.name == name {
            return true;
        }
        if depth >= MAX_DEPTH {
            return false;
        }
        comp.layers.iter().any(|l| match &l.source {
            LayerSource::Composition(n) => {
                self.comp(n).is_some_and(|c| self.reaches_at(c, name, depth + 1))
            }
            LayerSource::Scene(_) | LayerSource::Audio(_) => false,
        })
    }
}

/// What the layers leave a curve as: `keep` of whatever the value was
/// before them, plus `add`. Mixing layer over layer is affine in that
/// base value, so one pair carries any stack.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Blend {
    pub keep: f32,
    pub add: f32,
}

impl Default for Blend {
    fn default() -> Self {
        Self { keep: 1.0, add: 0.0 }
    }
}

impl Blend {
    /// The value once the layers have had their say over `base`.
    pub fn over(self, base: f32) -> f32 {
        base * self.keep + self.add
    }

    /// A value mixed over this by weight `w`.
    fn mix(&mut self, v: f32, w: f32) {
        self.mix_blend(Blend { keep: 0.0, add: v }, w);
    }

    /// Another stack's result mixed over this by weight `w`.
    fn mix_blend(&mut self, b: Blend, w: f32) {
        self.keep = self.keep * (1.0 - w) + b.keep * w;
        self.add = self.add * (1.0 - w) + b.add * w;
    }

    /// A value added on top, scaled by `w`; what was beneath stays.
    fn add_over(&mut self, v: f32, w: f32) {
        self.add += v * w;
    }
}

/// Where a layer ends on the composition's clock.
pub fn layer_end(l: &Layer) -> f32 {
    l.start + (l.out_point - l.in_point).max(0.0) / l.speed.max(1e-3)
}

/// A layer's weight at composition time `t`: nothing outside its span,
/// ramped over its fades at either end.
pub fn layer_weight(l: &Layer, t: f32) -> f32 {
    let end = layer_end(l);
    if !l.enabled || t < l.start || t > end {
        return 0.0;
    }
    let mut w = l.weight.clamp(0.0, 1.0);
    if l.fade_in > 0.0 {
        w *= ((t - l.start) / l.fade_in).clamp(0.0, 1.0);
    }
    if l.fade_out > 0.0 {
        w *= ((end - t) / l.fade_out).clamp(0.0, 1.0);
    }
    w
}

/// The source's own time at composition time `t`: from the in point at
/// the layer's speed, wrapping when a looping source runs past its end.
pub fn source_time(l: &Layer, t: f32, len: f32, looped: bool) -> f32 {
    let st = l.in_point + (t - l.start).max(0.0) * l.speed.max(1e-3);
    if looped && len > 0.0 && st > len { st % len } else { st.min(len.max(0.0)) }
}

/// Every curve the composition touches at `t`, as a blend each.
pub fn sample(scope: &Scope, comp: &Composition, t: f32) -> BTreeMap<(CurveTarget, String), Blend> {
    let mut out = BTreeMap::new();
    sample_into(scope, comp, t, 0, &mut out);
    out
}

fn sample_into(
    scope: &Scope,
    comp: &Composition,
    t: f32,
    depth: usize,
    out: &mut BTreeMap<(CurveTarget, String), Blend>,
) {
    if depth >= MAX_DEPTH {
        return;
    }
    // layers list top first; the bottom one goes on first
    for l in comp.layers.iter().rev() {
        let w = layer_weight(l, t);
        if w <= 0.0 {
            continue;
        }
        let Some(len) = scope.source_len(&l.source) else { continue };
        let st = source_time(l, t, len, scope.source_looped(&l.source));
        match &l.source {
            LayerSource::Scene(n) => {
                let Some(a) = scope.scene(n) else { continue };
                for c in &a.curves {
                    let Some(v) = crate::anim::sample(a, c, st) else { continue };
                    let slot = out.entry((c.target, c.id.clone())).or_default();
                    if l.additive {
                        slot.add_over(v, w);
                    } else {
                        slot.mix(v, w);
                    }
                }
            }
            LayerSource::Composition(n) => {
                let Some(inner) = scope.comp(n) else { continue };
                let mut got = BTreeMap::new();
                sample_into(scope, inner, st, depth + 1, &mut got);
                for (k, b) in got {
                    let slot = out.entry(k).or_default();
                    if l.additive {
                        slot.add_over(b.add, w);
                    } else {
                        slot.mix_blend(b, w);
                    }
                }
            }
            LayerSource::Audio(_) => {}
        }
    }
}

/// A physics playlist as a composition: each group's scenes start
/// together, the next group follows when the longest is done, and groups
/// cross over the playlist's fade. A scene missing from the project is
/// skipped.
pub fn playlist_comp(scope: &Scope, pl: &Playlist, fps: f32) -> Composition {
    let mut layers = Vec::new();
    let mut t = 0.0f32;
    let n = pl.groups.len();
    for (gi, g) in pl.groups.iter().enumerate() {
        let lens: Vec<(f32, &ayatsuri_format::PlaylistScene)> = g
            .scenes
            .iter()
            .filter_map(|ps| scope.scene(&ps.scene).map(|a| (a.duration.max(0.0), ps)))
            .collect();
        let Some(len) = lens.iter().map(|(d, _)| *d).reduce(f32::max) else { continue };
        let fade_in = if gi == 0 { 0.0 } else { pl.fade.max(0.0).min(len) };
        let fade_out = if gi + 1 == n { 0.0 } else { pl.fade.max(0.0).min(len) };
        for (d, ps) in lens {
            layers.push(Layer {
                source: LayerSource::Scene(ps.scene.clone()),
                start: t,
                in_point: 0.0,
                // a shorter scene in the group plays round again if it loops
                out_point: if scope.source_looped(&LayerSource::Scene(ps.scene.clone())) {
                    len
                } else {
                    d
                },
                weight: ps.weight.clamp(0.0, 1.0),
                fade_in,
                fade_out,
                additive: ps.additive,
                ..Layer::default()
            });
        }
        t += len - fade_out;
    }
    // the last group plays out whole
    let duration = layers.iter().map(layer_end).fold(0.0f32, f32::max);
    layers.reverse();
    Composition { name: pl.name.clone(), fps, duration, looped: true, work_area: None, layers }
}

/// The composition flattened into one scene: a key on every frame of each
/// curve any layer touches, linear between, the way a precomp bakes.
/// `base` says what a curve is where no layer reaches it.
pub fn bake(
    scope: &Scope,
    comp: &Composition,
    base: impl Fn(CurveTarget, &str) -> f32,
) -> Animation {
    let fps = comp.fps.max(1.0);
    let frames = (comp.duration.max(0.0) * fps).round() as usize;
    let mut keys: BTreeMap<(CurveTarget, String), Vec<Key>> = BTreeMap::new();
    let mut samples = Vec::with_capacity(frames + 1);
    for f in 0..=frames {
        let t = f as f32 / fps;
        samples.push((t, sample(scope, comp, t)));
    }
    for (_, s) in &samples {
        for k in s.keys() {
            keys.entry(k.clone()).or_default();
        }
    }
    for ((target, id), ks) in &mut keys {
        let rest = base(*target, id);
        for (t, s) in &samples {
            let v = s.get(&(*target, id.clone())).map_or(rest, |b| b.over(rest));
            ks.push(Key { time: *t, value: v, interp: Interp::Linear });
        }
    }
    Animation {
        name: comp.name.clone(),
        fps,
        duration: comp.duration,
        looped: comp.looped,
        work_area: comp.work_area,
        curves: keys
            .into_iter()
            .map(|((target, id), keys)| Curve { target, id, keys, ..Curve::default() })
            .collect(),
        ..Animation::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scene(name: &str, duration: f32, v0: f32, v1: f32, looped: bool) -> Animation {
        Animation {
            name: name.into(),
            fps: 30.0,
            duration,
            looped,
            curves: vec![Curve {
                target: CurveTarget::Parameter,
                id: "A".into(),
                keys: vec![
                    Key { time: 0.0, value: v0, interp: Interp::Linear },
                    Key { time: duration, value: v1, interp: Interp::Linear },
                ],
                ..Curve::default()
            }],
            ..Animation::default()
        }
    }

    fn layer(name: &str, start: f32, out: f32) -> Layer {
        Layer {
            source: LayerSource::Scene(name.into()),
            start,
            in_point: 0.0,
            out_point: out,
            ..Layer::default()
        }
    }

    fn value(scope: &Scope, comp: &Composition, t: f32) -> f32 {
        sample(scope, comp, t)
            .get(&(CurveTarget::Parameter, "A".into()))
            .map_or(100.0, |b| b.over(100.0))
    }

    #[test]
    fn a_full_layer_replaces_what_is_beneath() {
        let scenes = vec![scene("lo", 2.0, 0.0, 0.0, false), scene("hi", 2.0, 5.0, 5.0, false)];
        let comp = Composition {
            name: "c".into(),
            fps: 30.0,
            duration: 2.0,
            layers: vec![layer("hi", 0.5, 2.0), layer("lo", 0.0, 2.0)],
            ..Composition::default()
        };
        let scope = Scope { scenes: &scenes, comps: &[] };
        assert_eq!(value(&scope, &comp, 0.2), 0.0);
        assert_eq!(value(&scope, &comp, 1.0), 5.0);
        // past every layer the base shows through
        assert_eq!(value(&scope, &comp, 3.0), 100.0);
    }

    #[test]
    fn fades_and_weight_mix_over_the_base() {
        let scenes = vec![scene("s", 2.0, 10.0, 10.0, false)];
        let mut l = layer("s", 0.0, 2.0);
        l.fade_in = 1.0;
        l.weight = 0.5;
        let comp = Composition {
            name: "c".into(),
            fps: 30.0,
            duration: 2.0,
            layers: vec![l],
            ..Composition::default()
        };
        let scope = Scope { scenes: &scenes, comps: &[] };
        // half way through the fade at half weight: a quarter of the way
        assert!((value(&scope, &comp, 0.5) - 77.5).abs() < 1e-4);
        assert!((value(&scope, &comp, 1.5) - 55.0).abs() < 1e-4);
    }

    #[test]
    fn a_looping_source_wraps_and_a_fast_layer_runs_ahead() {
        let scenes = vec![scene("s", 1.0, 0.0, 1.0, true)];
        let mut l = layer("s", 0.0, 3.0);
        l.speed = 2.0;
        let comp = Composition {
            name: "c".into(),
            fps: 30.0,
            duration: 2.0,
            layers: vec![l],
            ..Composition::default()
        };
        let scope = Scope { scenes: &scenes, comps: &[] };
        assert!((layer_end(&comp.layers[0]) - 1.5).abs() < 1e-6);
        // 0.25 s in at double speed is 0.5 s of the source
        assert!((value(&scope, &comp, 0.25) - 0.5).abs() < 1e-4);
        // 0.75 s in is 1.5 s, wrapped to 0.5 s
        assert!((value(&scope, &comp, 0.75) - 0.5).abs() < 1e-4);
    }

    #[test]
    fn a_nested_composition_mixes_as_one_layer() {
        let scenes = vec![scene("s", 1.0, 4.0, 4.0, false)];
        let inner = Composition {
            name: "inner".into(),
            fps: 30.0,
            duration: 1.0,
            layers: vec![layer("s", 0.0, 1.0)],
            ..Composition::default()
        };
        let mut l = Layer {
            source: LayerSource::Composition("inner".into()),
            out_point: 1.0,
            ..Layer::default()
        };
        l.weight = 0.5;
        let outer = Composition {
            name: "outer".into(),
            fps: 30.0,
            duration: 1.0,
            layers: vec![l],
            ..Composition::default()
        };
        let comps = vec![inner, outer.clone()];
        let scope = Scope { scenes: &scenes, comps: &comps };
        assert!((value(&scope, &outer, 0.5) - 52.0).abs() < 1e-4);
        assert!(scope.reaches(&outer, "inner"));
        assert!(!scope.reaches(&comps[0], "outer"));
    }

    #[test]
    fn a_composition_inside_itself_stops() {
        let comp = Composition {
            name: "c".into(),
            fps: 30.0,
            duration: 1.0,
            layers: vec![Layer {
                source: LayerSource::Composition("c".into()),
                out_point: 1.0,
                ..Layer::default()
            }],
            ..Composition::default()
        };
        let comps = vec![comp.clone()];
        let scope = Scope { scenes: &[], comps: &comps };
        assert!(sample(&scope, &comp, 0.5).is_empty());
        assert!(scope.reaches(&comp, "c"));
    }

    #[test]
    fn an_additive_layer_adds_and_a_playlist_lines_its_groups_up() {
        let scenes = vec![scene("lo", 1.0, 2.0, 2.0, false), scene("hi", 2.0, 3.0, 3.0, false)];
        let scope = Scope { scenes: &scenes, comps: &[] };
        let pl = Playlist {
            name: "p".into(),
            fade: 0.5,
            groups: vec![
                ayatsuri_format::PlaylistGroup {
                    scenes: vec![ayatsuri_format::PlaylistScene {
                        scene: "lo".into(),
                        ..Default::default()
                    }],
                },
                ayatsuri_format::PlaylistGroup {
                    scenes: vec![ayatsuri_format::PlaylistScene {
                        scene: "hi".into(),
                        weight: 0.5,
                        additive: true,
                    }],
                },
            ],
        };
        let comp = playlist_comp(&scope, &pl, 30.0);
        assert_eq!(comp.layers.len(), 2);
        // the second group starts half a second before the first ends
        let hi = comp.layers.iter().find(|l| l.source.name() == "hi").unwrap();
        assert!((hi.start - 0.5).abs() < 1e-6);
        assert!((comp.duration - 2.5).abs() < 1e-6);
        // past the fade only "hi" plays, added at half weight over the base
        assert!((value(&scope, &comp, 1.5) - 101.5).abs() < 1e-4);
        // the empty A of a layerless time is the base
        assert_eq!(value(&scope, &comp, 3.0), 100.0);
    }

    #[test]
    fn a_bake_keys_every_frame_and_holds_the_base_elsewhere() {
        let scenes = vec![scene("s", 1.0, 0.0, 1.0, false)];
        let comp = Composition {
            name: "c".into(),
            fps: 10.0,
            duration: 2.0,
            layers: vec![layer("s", 0.5, 1.0)],
            ..Composition::default()
        };
        let scope = Scope { scenes: &scenes, comps: &[] };
        let baked = bake(&scope, &comp, |_, _| 9.0);
        assert_eq!(baked.curves.len(), 1);
        let keys = &baked.curves[0].keys;
        assert_eq!(keys.len(), 21);
        assert_eq!(keys[0].value, 9.0);
        assert!((keys[10].value - 0.5).abs() < 1e-4);
        assert_eq!(keys[20].value, 9.0);
        assert!(keys.iter().all(|k| k.interp == Interp::Linear));
    }
}
