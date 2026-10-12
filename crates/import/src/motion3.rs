// motion3.json, the published motion format (CubismSpecs): curves whose
// Segments array is one flat run of numbers. It opens with the first point
// (time, value); then each segment is a type id and its points:
//
//     0 linear           1 point   end
//     1 bezier           3 points  out handle, into handle, end
//     2 stepped          1 point   end
//     3 inverse stepped  1 point   end
//
// The file is untrusted. Every count is capped, every number checked, and a
// malformed curve is an error, never a panic or a runaway allocation.

use ayatsuri_format::{Animation, Curve, CurveTarget, Event, Interp, Key};
use serde::Deserialize;

use crate::ImportError;

/// More numbers than any real motion carries; past this a file is refused.
const MAX_SEGMENT_NUMBERS: usize = 4_000_000;
const MAX_CURVES: usize = 20_000;

#[derive(Deserialize)]
struct File {
    #[serde(rename = "Meta", default)]
    meta: Meta,
    #[serde(rename = "Curves", default)]
    curves: Vec<CurveJson>,
    #[serde(rename = "UserData", default)]
    user_data: Vec<UserData>,
}

#[derive(Deserialize, Default)]
struct Meta {
    #[serde(rename = "Duration", default)]
    duration: f32,
    #[serde(rename = "Fps", default)]
    fps: f32,
    #[serde(rename = "Loop", default)]
    looped: bool,
    #[serde(rename = "AreBeziersRestricted", default)]
    restricted: bool,
    #[serde(rename = "FadeInTime", default)]
    fade_in: Option<f32>,
    #[serde(rename = "FadeOutTime", default)]
    fade_out: Option<f32>,
}

#[derive(Deserialize)]
struct CurveJson {
    #[serde(rename = "Target", default)]
    target: String,
    #[serde(rename = "Id", default)]
    id: String,
    #[serde(rename = "Segments", default)]
    segments: Vec<f32>,
    #[serde(rename = "FadeInTime", default)]
    fade_in: Option<f32>,
    #[serde(rename = "FadeOutTime", default)]
    fade_out: Option<f32>,
}

#[derive(Deserialize)]
struct UserData {
    #[serde(rename = "Time", default)]
    time: f32,
    #[serde(rename = "Value", default)]
    value: String,
}

fn bad(msg: impl Into<String>) -> ImportError {
    ImportError::Motion(msg.into())
}

/// A fade only counts when it is a real, non-negative number of seconds.
fn fade(f: Option<f32>) -> Option<f32> {
    f.filter(|v| v.is_finite() && *v >= 0.0)
}

/// Unpack one curve's Segments into keys, interpolation on the segment's
/// first key.
fn keys_of(id: &str, seg: &[f32]) -> Result<Vec<Key>, ImportError> {
    if seg.iter().any(|v| !v.is_finite()) {
        return Err(bad(format!("{id}: a segment number is not finite")));
    }
    let [t0, v0, rest @ ..] = seg else {
        return Err(bad(format!("{id}: no first point")));
    };
    let mut keys = vec![Key { time: *t0, value: *v0, interp: Interp::Linear }];
    let mut rest = rest;
    while let [kind, tail @ ..] = rest {
        let (interp, end, consumed) = match (*kind as i32, tail) {
            (0, [t, v, ..]) => (Interp::Linear, (*t, *v), 2),
            (2, [t, v, ..]) => (Interp::Stepped, (*t, *v), 2),
            (3, [t, v, ..]) => (Interp::InverseStepped, (*t, *v), 2),
            (1, [t1, v1, t2, v2, t3, v3, ..]) => {
                // handles are stored absolute; keep them relative to their keys
                let (pt, pv) = keys.last().map(|k| (k.time, k.value)).unwrap_or((*t0, *v0));
                let interp = Interp::Bezier { out: (t1 - pt, v1 - pv), into: (t2 - t3, v2 - v3) };
                (interp, (*t3, *v3), 6)
            }
            (k @ 0..=3, _) => return Err(bad(format!("{id}: segment type {k} is cut short"))),
            (k, _) => return Err(bad(format!("{id}: unknown segment type {k}"))),
        };
        if let Some(prev) = keys.last_mut() {
            if end.0 < prev.time {
                return Err(bad(format!("{id}: segment ends before it starts")));
            }
            prev.interp = interp;
        }
        keys.push(Key { time: end.0, value: end.1, interp: Interp::Linear });
        rest = tail.get(consumed..).unwrap_or(&[]);
    }
    Ok(keys)
}

/// Parse a motion3.json into a scene called `name`.
pub fn from_motion3(bytes: &[u8], name: &str) -> Result<Animation, ImportError> {
    let text = core::str::from_utf8(bytes).map_err(|e| bad(e.to_string()))?;
    let file: File = serde_json::from_str(text.trim_start_matches('\u{feff}'))
        .map_err(|e| bad(e.to_string()))?;
    if file.curves.len() > MAX_CURVES {
        return Err(bad(format!("{} curves is more than any motion carries", file.curves.len())));
    }
    let total: usize = file.curves.iter().map(|c| c.segments.len()).sum();
    if total > MAX_SEGMENT_NUMBERS {
        return Err(bad(format!("{total} segment numbers is more than any motion carries")));
    }
    let mut curves = Vec::with_capacity(file.curves.len());
    for c in file.curves {
        let target = match c.target.as_str() {
            "Parameter" => CurveTarget::Parameter,
            "PartOpacity" => CurveTarget::PartOpacity,
            "Model" => CurveTarget::Model,
            other => {
                log::warn!("{name}: curve {} has unknown target {other:?}, skipped", c.id);
                continue;
            }
        };
        let keys = keys_of(&c.id, &c.segments)?;
        curves.push(Curve {
            target,
            id: c.id,
            keys,
            fade_in: fade(c.fade_in),
            fade_out: fade(c.fade_out),
        });
    }
    let m = file.meta;
    let finite = |v: f32, or: f32| if v.is_finite() && v > 0.0 { v } else { or };
    Ok(Animation {
        name: name.to_string(),
        fps: finite(m.fps, 30.0),
        duration: finite(m.duration, 0.0),
        looped: m.looped,
        curves,
        events: file
            .user_data
            .into_iter()
            .filter(|u| u.time.is_finite())
            .map(|u| Event { time: u.time, value: u.value })
            .collect(),
        fade_in: fade(m.fade_in),
        fade_out: fade(m.fade_out),
        restricted_beziers: m.restricted,
        ..Animation::default()
    })
}

/// One curve's Segments, its segment count and its point count.
fn segments_of(curve: &Curve, restricted: bool) -> (Vec<f32>, usize, usize) {
    let mut out = Vec::new();
    let Some(first) = curve.keys.first() else { return (out, 0, 0) };
    out.extend([first.time, first.value]);
    let (mut segs, mut points) = (0usize, 1usize);
    for pair in curve.keys.windows(2) {
        let [k0, k1] = pair else { continue };
        segs += 1;
        match k0.interp {
            Interp::Linear => out.extend([0.0, k1.time, k1.value]),
            Interp::Stepped => out.extend([2.0, k1.time, k1.value]),
            Interp::InverseStepped => out.extend([3.0, k1.time, k1.value]),
            // auto smooth has no motion3 type of its own: it goes out as the
            // bezier it evaluates as
            Interp::Auto | Interp::Bezier { .. } => {
                let (o, i) = ayatsuri_core::anim::handles(k0, k1);
                let span = k1.time - k0.time;
                // restricted files keep handles at 1/3 and 2/3 of the span
                let (t1, t2) = if restricted {
                    (k0.time + span / 3.0, k0.time + span * 2.0 / 3.0)
                } else {
                    (k0.time + o.0, k1.time + i.0)
                };
                out.extend([1.0, t1, k0.value + o.1, t2, k1.value + i.1, k1.time, k1.value]);
                points += 2;
            }
        }
        points += 1;
    }
    (out, segs, points)
}

/// Write a scene as motion3.json, Cubism-style.
pub fn to_motion3(anim: &Animation) -> Vec<u8> {
    use serde_json::{Map, Value, json};
    let (mut seg_total, mut point_total) = (0usize, 0usize);
    let mut curves = Vec::new();
    for c in anim.curves.iter().filter(|c| !c.keys.is_empty()) {
        let (segs, n_segs, n_points) = segments_of(c, anim.restricted_beziers);
        seg_total += n_segs;
        point_total += n_points;
        let mut o = Map::new();
        o.insert(
            "Target".into(),
            match c.target {
                CurveTarget::Parameter => "Parameter",
                CurveTarget::PartOpacity => "PartOpacity",
                CurveTarget::Model => "Model",
            }
            .into(),
        );
        o.insert("Id".into(), c.id.clone().into());
        if let Some(f) = c.fade_in {
            o.insert("FadeInTime".into(), f.into());
        }
        if let Some(f) = c.fade_out {
            o.insert("FadeOutTime".into(), f.into());
        }
        o.insert("Segments".into(), json!(segs));
        curves.push(Value::Object(o));
    }
    let mut meta = Map::new();
    meta.insert("Duration".into(), anim.duration.max(0.0).into());
    meta.insert("Fps".into(), (if anim.fps > 0.0 { anim.fps } else { 30.0 }).into());
    meta.insert("Loop".into(), anim.looped.into());
    meta.insert("AreBeziersRestricted".into(), anim.restricted_beziers.into());
    if let Some(f) = anim.fade_in {
        meta.insert("FadeInTime".into(), f.into());
    }
    if let Some(f) = anim.fade_out {
        meta.insert("FadeOutTime".into(), f.into());
    }
    meta.insert("CurveCount".into(), curves.len().into());
    meta.insert("TotalSegmentCount".into(), seg_total.into());
    meta.insert("TotalPointCount".into(), point_total.into());
    meta.insert("UserDataCount".into(), anim.events.len().into());
    // the spec's own example counts bytes of the strings, no terminators
    meta.insert(
        "TotalUserDataSize".into(),
        anim.events.iter().map(|e| e.value.len()).sum::<usize>().into(),
    );
    let mut doc = Map::new();
    doc.insert("Version".into(), 3.into());
    doc.insert("Meta".into(), Value::Object(meta));
    doc.insert("Curves".into(), curves.into());
    if !anim.events.is_empty() {
        let ud: Vec<Value> =
            anim.events.iter().map(|e| json!({ "Time": e.time, "Value": e.value })).collect();
        doc.insert("UserData".into(), ud.into());
    }
    crate::cubism_json(&Value::Object(doc))
}

#[cfg(test)]
mod tests {
    use super::*;

    // CubismSpecs' own motion3.json example, trimmed to its structure:
    // three curves, one linear segment each, two user data strings
    const SPEC: &str = r#"{
        "Version": 3,
        "Meta": { "Duration": 1.0, "Fps": 30.0, "Loop": true, "AreBeziersRestricted": true,
                  "CurveCount": 3, "TotalSegmentCount": 3, "TotalPointCount": 6,
                  "UserDataCount": 2, "TotalUserDataSize": 8 },
        "Curves": [
            { "Target": "Model", "Id": "Opacity", "Segments": [0, 0, 0, 1, 1] },
            { "Target": "Parameter", "Id": "ParamAngleX", "Segments": [0, 0, 0, 1, 1] },
            { "Target": "PartOpacity", "Id": "PartArmA", "Segments": [0, 0, 0, 1, 1] }
        ],
        "UserData": [ { "Time": 0.0, "Value": "hoge" }, { "Time": 1.0, "Value": "fuga" } ]
    }"#;

    fn meta_of(bytes: &[u8]) -> serde_json::Value {
        let v: serde_json::Value = serde_json::from_slice(bytes).unwrap();
        v["Meta"].clone()
    }

    #[test]
    fn reads_the_spec_example() {
        let a = from_motion3(SPEC.as_bytes(), "spec").unwrap();
        assert_eq!(a.curves.len(), 3);
        assert_eq!(a.curves[0].target, CurveTarget::Model);
        assert_eq!(a.curves[2].target, CurveTarget::PartOpacity);
        assert_eq!(a.curves[1].keys.len(), 2);
        assert!(a.looped && a.restricted_beziers);
        assert_eq!(a.events.len(), 2);
    }

    #[test]
    fn writes_the_spec_counts() {
        let a = from_motion3(SPEC.as_bytes(), "spec").unwrap();
        let m = meta_of(&to_motion3(&a));
        assert_eq!(m["CurveCount"], 3);
        assert_eq!(m["TotalSegmentCount"], 3);
        assert_eq!(m["TotalPointCount"], 6);
        assert_eq!(m["UserDataCount"], 2);
        assert_eq!(m["TotalUserDataSize"], 8);
    }

    #[test]
    fn every_segment_type_round_trips() {
        // linear, bezier, stepped, inverse stepped in one curve
        let src = r#"{"Version":3,"Meta":{"Duration":4,"Fps":30},"Curves":[
            {"Target":"Parameter","Id":"P","Segments":
              [0,0, 0,1,1, 1,1.2,1.5,1.8,0.5,2,0, 2,3,1, 3,4,0]}]}"#;
        let a = from_motion3(src.as_bytes(), "all").unwrap();
        let k = &a.curves[0].keys;
        assert_eq!(k.len(), 5);
        assert_eq!(k[0].interp, Interp::Linear);
        assert!(matches!(k[1].interp, Interp::Bezier { .. }));
        assert_eq!((k[2].interp, k[3].interp), (Interp::Stepped, Interp::InverseStepped));
        let out = to_motion3(&a);
        let m = meta_of(&out);
        assert_eq!(m["TotalSegmentCount"], 4);
        // 1 first + 1 linear + 3 bezier + 1 stepped + 1 inverse stepped
        assert_eq!(m["TotalPointCount"], 7);
        let back = from_motion3(&out, "all").unwrap();
        assert_eq!(back.curves[0].keys, a.curves[0].keys);
    }

    #[test]
    fn hostile_segments_are_errors() {
        let cut = r#"{"Curves":[{"Target":"Parameter","Id":"P","Segments":[0,0,1,0.3,0.3]}]}"#;
        assert!(from_motion3(cut.as_bytes(), "x").is_err());
        let kind = r#"{"Curves":[{"Target":"Parameter","Id":"P","Segments":[0,0,9,1,1]}]}"#;
        assert!(from_motion3(kind.as_bytes(), "x").is_err());
        let back = r#"{"Curves":[{"Target":"Parameter","Id":"P","Segments":[1,0,0,0,1]}]}"#;
        assert!(from_motion3(back.as_bytes(), "x").is_err());
        assert!(from_motion3(b"\xff", "x").is_err());
        // an unknown target is skipped, not fatal
        let odd = r#"{"Curves":[{"Target":"Camera","Id":"C","Segments":[0,0]}]}"#;
        assert!(from_motion3(odd.as_bytes(), "x").unwrap().curves.is_empty());
    }

    #[test]
    fn auto_goes_out_as_its_bezier() {
        let a = Animation {
            fps: 30.0,
            duration: 1.0,
            curves: vec![Curve {
                id: "P".into(),
                keys: vec![
                    Key { time: 0.0, value: 0.0, interp: Interp::Auto },
                    Key { time: 1.0, value: 1.0, interp: Interp::Linear },
                ],
                ..Curve::default()
            }],
            ..Animation::default()
        };
        let back = from_motion3(&to_motion3(&a), "a").unwrap();
        // same shape everywhere along the segment
        for i in 0..=10 {
            let t = i as f32 / 10.0;
            let x = ayatsuri_core::anim::sample_curve(&a.curves[0], t, false).unwrap();
            let y = ayatsuri_core::anim::sample_curve(&back.curves[0], t, false).unwrap();
            assert!((x - y).abs() < 1e-4, "{t}: {x} vs {y}");
        }
    }
}
