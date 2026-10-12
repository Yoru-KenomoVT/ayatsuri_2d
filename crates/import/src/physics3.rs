// physics3.json in, PhysicsRig out. The shape comes from the published
// schema in Live2D's CubismSpecs repository; the behaviour of each field is
// documented in prose in the editor manual. Files are hostile input like
// everything else, so every number is sanitized and every count capped.

use ayatsuri_format::model::{
    PhysicsGroup, PhysicsInput, PhysicsIo, PhysicsNode, PhysicsOutput, PhysicsRange, PhysicsRig,
};
use serde::Deserialize;

use crate::ImportError;

const MAX_GROUPS: usize = 1024;
const MAX_PER_GROUP: usize = 512;

#[derive(Deserialize)]
struct File {
    #[serde(rename = "Meta", default)]
    meta: Option<Meta>,
    #[serde(rename = "PhysicsSettings", default)]
    settings: Vec<Setting>,
}

#[derive(Deserialize, Default)]
struct Meta {
    #[serde(rename = "Fps", default)]
    fps: Option<f32>,
    #[serde(rename = "EffectiveForces", default)]
    forces: Option<Forces>,
    #[serde(rename = "PhysicsDictionary", default)]
    dictionary: Vec<DictEntry>,
}

#[derive(Deserialize, Default)]
struct Forces {
    #[serde(rename = "Gravity", default)]
    gravity: Option<Vec2>,
    #[serde(rename = "Wind", default)]
    wind: Option<Vec2>,
}

#[derive(Deserialize, Default, Clone, Copy)]
struct Vec2 {
    #[serde(rename = "X", default)]
    x: f32,
    #[serde(rename = "Y", default)]
    y: f32,
}

#[derive(Deserialize)]
struct DictEntry {
    #[serde(rename = "Id")]
    id: String,
    #[serde(rename = "Name", default)]
    name: String,
}

#[derive(Deserialize)]
struct Setting {
    #[serde(rename = "Id", default)]
    id: String,
    #[serde(rename = "Input", default)]
    input: Vec<Input>,
    #[serde(rename = "Output", default)]
    output: Vec<Output>,
    #[serde(rename = "Vertices", default)]
    vertices: Vec<Vertex>,
    #[serde(rename = "Normalization", default)]
    normalization: Option<Normalization>,
}

#[derive(Deserialize)]
struct Input {
    #[serde(rename = "Source")]
    source: ParamRef,
    #[serde(rename = "Weight", default)]
    weight: f32,
    #[serde(rename = "Type", default)]
    kind: String,
    #[serde(rename = "Reflect", default)]
    reflect: bool,
}

#[derive(Deserialize)]
struct Output {
    #[serde(rename = "Destination")]
    destination: ParamRef,
    #[serde(rename = "VertexIndex", default)]
    vertex: f64,
    #[serde(rename = "Scale", default)]
    scale: f32,
    #[serde(rename = "Weight", default)]
    weight: f32,
    #[serde(rename = "Type", default)]
    kind: String,
    #[serde(rename = "Reflect", default)]
    reflect: bool,
}

#[derive(Deserialize)]
struct ParamRef {
    #[serde(rename = "Id", default)]
    id: String,
}

#[derive(Deserialize)]
struct Vertex {
    #[serde(rename = "Position", default)]
    position: Option<Vec2>,
    #[serde(rename = "Mobility", default)]
    mobility: f32,
    #[serde(rename = "Delay", default)]
    delay: f32,
    #[serde(rename = "Acceleration", default)]
    acceleration: f32,
    #[serde(rename = "Radius", default)]
    radius: f32,
}

#[derive(Deserialize)]
struct Normalization {
    #[serde(rename = "Position")]
    position: Range,
    #[serde(rename = "Angle")]
    angle: Range,
}

#[derive(Deserialize, Clone, Copy)]
struct Range {
    #[serde(rename = "Minimum", default)]
    minimum: f32,
    #[serde(rename = "Default", default)]
    default: f32,
    #[serde(rename = "Maximum", default)]
    maximum: f32,
}

fn finite(v: f32, fallback: f32) -> f32 {
    if v.is_finite() { v } else { fallback }
}

fn io_kind(s: &str) -> PhysicsIo {
    match s {
        "Y" => PhysicsIo::Y,
        "Angle" => PhysicsIo::Angle,
        _ => PhysicsIo::X,
    }
}

fn range(r: Option<Range>) -> PhysicsRange {
    let r = r.unwrap_or(Range { minimum: -10.0, default: 0.0, maximum: 10.0 });
    let min = finite(r.minimum, -10.0);
    let max = finite(r.maximum, 10.0).max(min + 1e-3);
    PhysicsRange { min, default: finite(r.default, 0.0).clamp(min, max), max }
}

/// Serialize a rig back to physics3.json, the inverse of `from_physics3`,
/// for the embedded export.
pub fn to_physics3(rig: &ayatsuri_format::model::PhysicsRig) -> Vec<u8> {
    use serde_json::json;
    let settings: Vec<_> = rig
        .groups
        .iter()
        .map(|g| {
            json!({
                "Id": g.id,
                "Input": g.inputs.iter().map(|i| json!({
                    "Source": {"Target": "Parameter", "Id": i.param},
                    "Weight": i.weight,
                    "Type": match i.kind { PhysicsIo::Angle => "Angle", PhysicsIo::Y => "Y", _ => "X" },
                    "Reflect": i.reflect,
                })).collect::<Vec<_>>(),
                "Output": g.outputs.iter().map(|o| json!({
                    "Destination": {"Target": "Parameter", "Id": o.param},
                    "VertexIndex": o.vertex,
                    "Scale": o.scale,
                    "Weight": o.weight,
                    "Type": match o.kind { PhysicsIo::Angle => "Angle", PhysicsIo::Y => "Y", _ => "X" },
                    "Reflect": o.reflect,
                })).collect::<Vec<_>>(),
                "Vertices": g.nodes.iter().map(|n| json!({
                    "Position": {"X": n.x, "Y": n.y},
                    "Mobility": n.mobility,
                    "Delay": n.delay,
                    "Acceleration": n.acceleration,
                    "Radius": n.radius,
                })).collect::<Vec<_>>(),
                "Normalization": {
                    "Position": {"Minimum": g.norm_position.min, "Default": g.norm_position.default, "Maximum": g.norm_position.max},
                    "Angle": {"Minimum": g.norm_angle.min, "Default": g.norm_angle.default, "Maximum": g.norm_angle.max},
                },
            })
        })
        .collect();
    let doc = json!({
        "Version": 3,
        "Meta": {
            "PhysicsSettingCount": rig.groups.len(),
            "TotalInputCount": rig.groups.iter().map(|g| g.inputs.len()).sum::<usize>(),
            "TotalOutputCount": rig.groups.iter().map(|g| g.outputs.len()).sum::<usize>(),
            "VertexCount": rig.groups.iter().map(|g| g.nodes.len()).sum::<usize>(),
            "Fps": if rig.fps > 0.0 { rig.fps } else { 60.0 },
            "EffectiveForces": {
                "Gravity": {"X": rig.gravity_x, "Y": rig.gravity_y},
                "Wind": {"X": rig.wind_x, "Y": rig.wind_y},
            },
            "PhysicsDictionary": rig.groups.iter().map(|g| json!({
                "Id": g.id,
                "Name": if g.name.is_empty() { g.id.clone() } else { g.name.clone() },
            })).collect::<Vec<_>>(),
        },
        "PhysicsSettings": settings,
    });
    cubism_json(&doc)
}

/// Write JSON the way Cubism's own exporter does: whole numbers as
/// integers (`60`, not `60.0`), tab indents, keys in the order written.
/// Strict typed loaders (Unity's JsonUtility among them) refuse `60.0`
/// where the schema has an integer, and then the whole bundle reads as
/// unrecognized.
pub fn cubism_json(doc: &serde_json::Value) -> Vec<u8> {
    use serde::Serialize;
    fn ints(v: &serde_json::Value) -> serde_json::Value {
        use serde_json::Value;
        match v {
            Value::Number(n) => match n.as_f64() {
                Some(f) if n.is_f64() && f.fract() == 0.0 && f.abs() < 9.0e15 => {
                    Value::from(f as i64)
                }
                _ => v.clone(),
            },
            Value::Array(a) => Value::Array(a.iter().map(ints).collect()),
            Value::Object(o) => {
                Value::Object(o.iter().map(|(k, x)| (k.clone(), ints(x))).collect())
            }
            _ => v.clone(),
        }
    }
    let mut out = Vec::new();
    let fmt = serde_json::ser::PrettyFormatter::with_indent(b"\t");
    let mut ser = serde_json::Serializer::with_formatter(&mut out, fmt);
    if ints(doc).serialize(&mut ser).is_err() {
        return Vec::new();
    }
    out
}

/// Serialize an animation as motion3.json: linear segments only, which is
/// all the clip format carries today.
pub fn to_motion3(anim: &ayatsuri_format::Animation) -> Vec<u8> {
    use serde_json::json;
    let mut total_segments = 0usize;
    let mut total_points = 0usize;
    let curves: Vec<_> = anim
        .tracks
        .iter()
        .map(|t| {
            let mut seg: Vec<serde_json::Value> = Vec::new();
            for (i, (time, value)) in t.points.iter().enumerate() {
                if i == 0 {
                    seg.push(json!(time));
                    seg.push(json!(value));
                    total_points += 1;
                } else {
                    // 0 marks a linear segment in the published schema
                    seg.push(json!(0));
                    seg.push(json!(time));
                    seg.push(json!(value));
                    total_segments += 1;
                    total_points += 1;
                }
            }
            json!({"Target": "Parameter", "Id": t.param, "Segments": seg})
        })
        .collect();
    let doc = json!({
        "Version": 3,
        "Meta": {
            "Duration": anim.duration,
            "Fps": if anim.fps > 0.0 { anim.fps } else { 30.0 },
            "Loop": anim.looped,
            "AreBeziersRestricted": true,
            "CurveCount": curves.len(),
            "TotalSegmentCount": total_segments,
            "TotalPointCount": total_points,
            "UserDataCount": 0,
            "TotalUserDataSize": 0,
        },
        "Curves": curves,
    });
    cubism_json(&doc)
}

/// Parse a physics3.json file into the native rig.
pub fn from_physics3(bytes: &[u8]) -> Result<PhysicsRig, ImportError> {
    let file: File =
        serde_json::from_slice(bytes).map_err(|e| ImportError::Physics(e.to_string()))?;
    let meta = file.meta.unwrap_or_default();
    let forces = meta.forces.unwrap_or_default();
    let gravity = forces.gravity.unwrap_or(Vec2 { x: 0.0, y: -1.0 });
    let wind = forces.wind.unwrap_or_default();

    let mut groups = Vec::new();
    for setting in file.settings.into_iter().take(MAX_GROUPS) {
        let name = meta
            .dictionary
            .iter()
            .find(|d| d.id == setting.id)
            .map(|d| d.name.clone())
            .unwrap_or_default();
        let nodes: Vec<PhysicsNode> = setting
            .vertices
            .into_iter()
            .take(MAX_PER_GROUP)
            .map(|v| {
                let p = v.position.unwrap_or_default();
                PhysicsNode {
                    x: finite(p.x, 0.0),
                    y: finite(p.y, 0.0),
                    mobility: finite(v.mobility, 1.0).clamp(0.0, 1.0),
                    delay: finite(v.delay, 1.0).clamp(0.0, 10.0),
                    acceleration: finite(v.acceleration, 1.0).clamp(0.0, 100.0),
                    radius: finite(v.radius, 0.0).max(0.0),
                }
            })
            .collect();
        let node_count = nodes.len();
        groups.push(PhysicsGroup {
            id: setting.id,
            name,
            inputs: setting
                .input
                .into_iter()
                .take(MAX_PER_GROUP)
                .map(|i| PhysicsInput {
                    param: i.source.id,
                    weight: finite(i.weight, 0.0).clamp(0.0, 100.0),
                    kind: io_kind(&i.kind),
                    reflect: i.reflect,
                })
                .collect(),
            outputs: setting
                .output
                .into_iter()
                .take(MAX_PER_GROUP)
                .filter_map(|o| {
                    let vertex = o.vertex as i64;
                    if vertex < 0 || vertex as usize >= node_count.max(1) {
                        return None;
                    }
                    Some(PhysicsOutput {
                        param: o.destination.id,
                        vertex: vertex as u32,
                        scale: finite(o.scale, 1.0),
                        weight: finite(o.weight, 100.0).clamp(0.0, 100.0),
                        kind: io_kind(&o.kind),
                        reflect: o.reflect,
                    })
                })
                .collect(),
            nodes,
            norm_position: range(setting.normalization.as_ref().map(|n| n.position)),
            norm_angle: range(setting.normalization.as_ref().map(|n| n.angle)),
        });
    }

    Ok(PhysicsRig {
        fps: finite(meta.fps.unwrap_or(0.0), 0.0).clamp(0.0, 240.0),
        gravity_x: finite(gravity.x, 0.0),
        gravity_y: finite(gravity.y, -1.0),
        wind_x: finite(wind.x, 0.0),
        wind_y: finite(wind.y, 0.0),
        groups,
    })
}

#[cfg(test)]
mod cubism_json_tests {
    use super::*;

    #[test]
    fn whole_numbers_are_integers_and_order_is_kept() {
        let doc = serde_json::json!({
            "Version": 3,
            "Meta": { "Fps": 60.0, "Gravity": { "X": 0.0, "Y": -1.0 }, "Duration": 2.5 },
        });
        let text = String::from_utf8(cubism_json(&doc)).unwrap();
        assert!(text.contains("\"Fps\": 60,"), "{text}");
        assert!(text.contains("\"Y\": -1"), "{text}");
        assert!(text.contains("\"Duration\": 2.5"), "{text}");
        // Version first, as written, and tab indents
        assert!(text.starts_with("{\n\t\"Version\": 3"), "{text}");
    }

    #[test]
    fn physics_export_matches_cubism_number_style() {
        let src = br#"{"Version":3,"Meta":{"Fps":60,"EffectiveForces":{"Gravity":{"X":0,"Y":-1},"Wind":{"X":0,"Y":0}}},
            "PhysicsSettings":[{"Id":"P1",
              "Input":[{"Source":{"Target":"Parameter","Id":"ParamAngleX"},"Weight":100,"Type":"X","Reflect":false}],
              "Output":[{"Destination":{"Target":"Parameter","Id":"ParamHair"},"VertexIndex":1,"Scale":8,"Weight":100,"Type":"Angle","Reflect":false}],
              "Vertices":[{"Position":{"X":0,"Y":0},"Mobility":1,"Delay":1,"Acceleration":1,"Radius":0},
                          {"Position":{"X":0,"Y":10},"Mobility":0.9,"Delay":0.6,"Acceleration":1.5,"Radius":10}],
              "Normalization":{"Position":{"Minimum":-10,"Default":0,"Maximum":10},"Angle":{"Minimum":-10,"Default":0,"Maximum":10}}}]}"#;
        let rig = from_physics3(src).unwrap();
        let text = String::from_utf8(to_physics3(&rig)).unwrap();
        assert!(!text.contains("\"Weight\": 100.0"), "{text}");
        let first_key = text.find("\"Version\"").unwrap();
        assert!(first_key < text.find("\"Meta\"").unwrap());
    }
}
