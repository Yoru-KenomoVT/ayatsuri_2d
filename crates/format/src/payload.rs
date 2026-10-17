// What an .aom file carries: the model, its textures, and a little meta.
// MessagePack map with string keys (self-describing; readers skip unknown
// fields), bulk arrays inside the model as bin blobs.

use alloc::string::String;
use alloc::vec::Vec;

use serde::{Deserialize, Serialize};

use crate::model::Model;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Meta {
    pub name: String,
    pub author: String,
    pub tool: String,
    /// Record a timelapse beside this project: None follows the editor's
    /// Settings default.
    pub timelapse: Option<bool>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct AomPayload {
    pub meta: Meta,
    pub model: Model,
    /// Encoded images (PNG as imported), one per texture slot, so an .aom
    /// is self-contained.
    pub textures: Vec<serde_bytes::ByteBuf>,
    /// Editor-facing display info: names, palette groups, label colors.
    pub display: Option<DisplayInfo>,
    /// Animations ride inside the container, per docs/aom-format.md; the
    /// embedded export writes each one out as `<name>.motion3.json`.
    pub animations: Vec<Animation>,
    /// Compositions: scenes and other compositions laid out as layers over
    /// one clock. Empty in files from before 0.5.6; older readers skip them.
    pub compositions: Vec<Composition>,
    /// Expressions (exp3.json): parameter targets layered over the pose.
    /// Empty in older files.
    pub expressions: Vec<Expression>,
}

/// One expression, the published exp3.json shape: parameters held at a
/// value and blended over whatever the pose is, faded in and out.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Expression {
    pub name: String,
    /// Seconds; None leaves it to the player (the spec's default is 1).
    pub fade_in: Option<f32>,
    pub fade_out: Option<f32>,
    pub params: Vec<ExpressionParam>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ExpressionParam {
    /// Parameter id.
    pub id: String,
    pub value: f32,
    pub blend: ExpressionBlend,
}

/// How an expression value meets the pose's.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ExpressionBlend {
    /// pose + value (the spec's default)
    #[default]
    Add,
    /// pose * value; what eye open/close should use, so blinks still work
    Multiply,
    /// value, whatever the pose
    Overwrite,
}

/// One scene (Cubism's word for a clip): curves over time, the shape of
/// motion3.json, plus what the editor needs around it.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Animation {
    pub name: String,
    pub fps: f32,
    /// Seconds.
    pub duration: f32,
    pub looped: bool,
    /// Straight-line parameter tracks from files before 0.5; `upgrade`
    /// turns them into curves when the file is read, so only old files
    /// ever carry them.
    pub tracks: Vec<Track>,
    pub curves: Vec<Curve>,
    /// motion3 UserData: a string fired at a time.
    pub events: Vec<Event>,
    /// Whole-motion fades in seconds; None leaves them to the player.
    pub fade_in: Option<f32>,
    pub fade_out: Option<f32>,
    /// motion3's AreBeziersRestricted: handles keep to 1/3 and 2/3 of their
    /// segment in time, so a bezier can be evaluated without solving for
    /// time (what Cubism's Unity target asks for).
    pub restricted_beziers: bool,
    /// The play range (Cubism's Work Area) in seconds; None is the whole
    /// scene.
    pub work_area: Option<(f32, f32)>,
    pub kind: SceneKind,
}

/// What a scene is for.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum SceneKind {
    /// A motion, exported as motion3.json.
    #[default]
    Motion,
    /// A pose that becomes an expression (exp3.json), the way Cubism makes
    /// them from short scenes.
    Expression,
}

/// A composition: scenes and other compositions laid out as layers over
/// one clock, the way After Effects nests comps, so one timeline can run
/// a whole animation. Layers list top first; each mixes over everything
/// beneath it by its weight, faded in and out at its ends. A composition
/// exports by baking into a motion.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Composition {
    pub name: String,
    pub fps: f32,
    /// Seconds.
    pub duration: f32,
    pub looped: bool,
    /// The play range in seconds; None is the whole composition.
    pub work_area: Option<(f32, f32)>,
    pub layers: Vec<Layer>,
}

/// One layer of a composition: a stretch of a source played from a time.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Layer {
    pub source: LayerSource,
    /// Where the layer starts on the composition's clock, seconds.
    pub start: f32,
    /// The stretch of the source it plays, in the source's own seconds.
    /// An out point past a looping source's end plays it round again.
    pub in_point: f32,
    pub out_point: f32,
    /// Source seconds per composition second.
    pub speed: f32,
    /// How much of the layer shows over what is beneath, 0 to 1.
    pub weight: f32,
    /// Seconds at each end over which the weight ramps.
    pub fade_in: f32,
    pub fade_out: f32,
    pub enabled: bool,
}

impl Default for Layer {
    fn default() -> Self {
        Self {
            source: LayerSource::default(),
            start: 0.0,
            in_point: 0.0,
            out_point: 0.0,
            speed: 1.0,
            weight: 1.0,
            fade_in: 0.0,
            fade_out: 0.0,
            enabled: true,
        }
    }
}

/// What a layer plays, by name.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum LayerSource {
    Scene(String),
    Composition(String),
}

impl Default for LayerSource {
    fn default() -> Self {
        Self::Scene(String::new())
    }
}

impl LayerSource {
    pub fn name(&self) -> &str {
        match self {
            Self::Scene(n) | Self::Composition(n) => n,
        }
    }
}

/// Pre-0.5 straight-line track.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Track {
    /// Parameter id the curve drives.
    pub param: String,
    /// (time seconds, value) pairs, sorted by time.
    pub points: Vec<(f32, f32)>,
}

/// One animated value: what it drives and its keys, sorted by time.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Curve {
    pub target: CurveTarget,
    /// Parameter id, part id, or for Model one of Opacity, EyeBlink, LipSync.
    pub id: String,
    pub keys: Vec<Key>,
    pub fade_in: Option<f32>,
    pub fade_out: Option<f32>,
}

/// motion3.json's curve targets.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum CurveTarget {
    #[default]
    Parameter,
    PartOpacity,
    Model,
}

/// A key: a value at a time, and how the segment after it gets to the next
/// key (Cubism keeps interpolation on the segment, not the key).
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Key {
    /// Seconds.
    pub time: f32,
    pub value: f32,
    pub interp: Interp,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub enum Interp {
    /// Cubism's default "auto smooth": eases out of this key and into the
    /// next (flat handles a third of the way along). Written as a bezier.
    #[default]
    Auto,
    Linear,
    /// Holds this key's value until the next key.
    Stepped,
    /// Jumps to the next key's value straight away.
    InverseStepped,
    /// Free handles: `out` from this key, `into` from the next one back,
    /// each (seconds, value) relative to its own key.
    Bezier {
        out: (f32, f32),
        into: (f32, f32),
    },
}

/// A string fired at a time while the motion plays (motion3 UserData).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Event {
    pub time: f32,
    pub value: String,
}

impl Animation {
    /// Bring a scene read from an older file up to date: straight-line
    /// tracks become linear curves.
    pub fn upgrade(&mut self) {
        for t in core::mem::take(&mut self.tracks) {
            let keys = t
                .points
                .iter()
                .map(|&(time, value)| Key { time, value, interp: Interp::Linear })
                .collect();
            self.curves.push(Curve {
                target: CurveTarget::Parameter,
                id: t.param,
                keys,
                fade_in: None,
                fade_out: None,
            });
        }
    }
}

/// How the editor presents the model: display names, the parameter palette's
/// folders, and label colors. None of it affects evaluation.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct DisplayInfo {
    pub param_names: Vec<String>,
    /// 0xRRGGBB per parameter, -1 for no label color.
    pub param_colors: Vec<i32>,
    /// Folder name plus the parameter indices inside, palette order.
    pub groups: Vec<(String, Vec<u32>)>,
    /// 0xRRGGBB per folder, -1 for none; empty in older files.
    pub group_colors: Vec<i32>,
    /// Display names of parts and art meshes (PSD layer and group names);
    /// empty in older files, which fall back to IDs.
    pub part_names: Vec<String>,
    pub mesh_names: Vec<String>,
    /// Parameter pairs shown as one 2D pad. None in older files, which
    /// keep the conventional X/Y pairs.
    pub combined: Option<Vec<(u32, u32)>>,
    /// The model3.json Groups (EyeBlink, LipSync...) as target, name and
    /// ids, kept so an export writes back what the model came with. None
    /// in older files and models that never had any.
    pub model_groups: Option<Vec<(String, String, Vec<String>)>>,
    pub guides: Vec<GuideLine>,
    pub bezier_nets: Vec<(String, u32, u32, u32, Vec<f32>)>,
}

/// A guide line on the canvas, as Cubism's Guide Line Settings has them.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct GuideLine {
    pub vertical: bool,
    pub at: f32,
    pub color: [u8; 4],
    pub memo: String,
}

impl Default for GuideLine {
    fn default() -> Self {
        Self { vertical: false, at: 0.0, color: GuideLine::DEFAULT_COLOR, memo: String::new() }
    }
}

impl GuideLine {
    pub const DEFAULT_COLOR: [u8; 4] = [0, 190, 255, 255];

    pub fn new(vertical: bool, at: f32) -> Self {
        Self { vertical, at, ..Self::default() }
    }
}
