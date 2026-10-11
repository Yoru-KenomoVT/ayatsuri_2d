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
