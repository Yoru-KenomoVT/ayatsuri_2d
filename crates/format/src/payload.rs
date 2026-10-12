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
}

/// One animation clip: parameter curves over time. Linear segments for now;
/// richer easing lands with the timeline editor.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Animation {
    pub name: String,
    pub fps: f32,
    /// Seconds.
    pub duration: f32,
    pub looped: bool,
    pub tracks: Vec<Track>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Track {
    /// Parameter id the curve drives.
    pub param: String,
    /// (time seconds, value) pairs, sorted by time.
    pub points: Vec<(f32, f32)>,
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
}
