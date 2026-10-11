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
}
