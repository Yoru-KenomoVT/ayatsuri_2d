//! The puppet model: what a rig IS, independent of any file encoding.
//!
//! `ayatsuri-import` builds one of these from a `.moc3`; the `.aom` container
//! serializes one; `ayatsuri-core` evaluates one. Pools are flat and
//! keyform-major so the runtime reads linearly.
//!
//! Coordinates are model units, y up, matching moc3. Pixels enter only
//! through [`Canvas::pixels_per_unit`].

use alloc::string::String;
use alloc::vec::Vec;
use serde::{Deserialize, Serialize};

/// Bulk arrays serialize as MessagePack `bin` (packed little endian), never
/// as element lists; docs/aom-format.md rule 1, measured there.
pub mod f32_bin {
    use alloc::vec::Vec;
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(v: &[f32], s: S) -> Result<S::Ok, S::Error> {
        let mut bytes = Vec::with_capacity(v.len() * 4);
        for x in v {
            bytes.extend_from_slice(&x.to_le_bytes());
        }
        s.serialize_bytes(&bytes)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<f32>, D::Error> {
        let bytes = serde_bytes::ByteBuf::deserialize(d)?;
        if bytes.len() % 4 != 0 {
            return Err(serde::de::Error::custom("f32 blob length not a multiple of 4"));
        }
        let (chunks, _) = bytes.as_chunks::<4>();
        Ok(chunks.iter().map(|c| f32::from_le_bytes(*c)).collect())
    }
}

/// Same for 16-bit index arrays.
pub mod u16_bin {
    use alloc::vec::Vec;
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(v: &[u16], s: S) -> Result<S::Ok, S::Error> {
        let mut bytes = Vec::with_capacity(v.len() * 2);
        for x in v {
            bytes.extend_from_slice(&x.to_le_bytes());
        }
        s.serialize_bytes(&bytes)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<u16>, D::Error> {
        let bytes = serde_bytes::ByteBuf::deserialize(d)?;
        if bytes.len() % 2 != 0 {
            return Err(serde::de::Error::custom("u16 blob length not a multiple of 2"));
        }
        let (chunks, _) = bytes.as_chunks::<2>();
        Ok(chunks.iter().map(|c| u16::from_le_bytes(*c)).collect())
    }
}

/// The stage the puppet is placed on.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub struct Canvas {
    pub pixels_per_unit: f32,
    pub origin_x: f32,
    pub origin_y: f32,
    pub width: f32,
    pub height: f32,
}

/// A rigger-facing control: one scalar the host drives.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Parameter {
    pub id: String,
    pub min: f32,
    pub max: f32,
    pub default: f32,
    /// Values wrap around the range instead of clamping (turn-around heads).
    pub repeat: bool,
    /// Displayed decimal places; also sets the key-snap epsilon the runtime
    /// uses (10^-decimals, the behaviour PurismCore verified).
    pub decimals: i32,
    /// Drives blend shapes (see [`BlendShape`]) instead of keyforms: its
    /// base key at 0.0 holds no keyform and the others add on top.
    #[serde(default)]
    pub blend_shape: bool,
}

/// One interpolation axis of a binding: a parameter and the key values at
/// which keyforms exist. Keys are strictly increasing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BindingAxis {
    pub parameter: u32,
    pub keys: Vec<f32>,
}

/// Which parameters move an object, in file order.
///
/// An object's keyforms form the cartesian product of its axes' keys, stored
/// with the FIRST axis varying fastest:
/// `keyform = sum(key_index[i] * stride[i])`, `stride[0] = 1`,
/// `stride[i+1] = stride[i] * keys[i].len()`. No axes means one keyform.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Binding {
    pub axes: Vec<BindingAxis>,
    /// Extended interpolation on some of the axes (Cubism 4.0): keyforms
    /// blend along a curve instead of a straight line. Empty for most.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub curves: Vec<AxisCurve>,
}

/// Extended interpolation on one parameter of a binding.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct AxisCurve {
    pub parameter: u32,
    pub kind: CurveKind,
}

/// How the keyforms between keys follow a curve. `points` is how many keys
/// a moc3 export adds between two keys to carry the curve; `scale` bends
/// an ellipse's orbit further out (1 is the plain curve through the keys).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum CurveKind {
    Ellipse { points: u8, scale: f32 },
    Sns { points: u8 },
}

impl CurveKind {
    pub fn points(self) -> u8 {
        match self {
            CurveKind::Ellipse { points, .. } | CurveKind::Sns { points } => points,
        }
    }
}

impl Binding {
    /// The curve set on `parameter`, if any.
    pub fn curve(&self, parameter: u32) -> Option<CurveKind> {
        self.curves.iter().find(|c| c.parameter == parameter).map(|c| c.kind)
    }
}

impl Binding {
    /// How many keyforms the cartesian product calls for.
    pub fn keyform_count(&self) -> usize {
        self.axes.iter().fold(1usize, |acc, a| acc.saturating_mul(a.keys.len().max(1)))
    }
}

/// Visibility grouping. Parts form a tree; opacity multiplies down it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Part {
    pub id: String,
    pub parent_part: Option<u32>,
    pub visible: bool,
    pub enabled: bool,
    pub binding: Binding,
    /// Per keyform.
    pub draw_orders: Vec<f32>,
    /// Drawn offscreen (Cubism 5.3): its children paint into a layer of
    /// their own first, which lands on the canvas as one image.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub offscreen: Option<Offscreen>,
}

/// How an offscreen part's layer lands on the canvas. Its children keep
/// together in the draw order while it is on, as a draw order group does.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Offscreen {
    /// Art meshes whose alpha clips the layer.
    #[serde(default)]
    pub masks: Vec<u32>,
    /// Show the layer outside the masks instead of inside.
    #[serde(default)]
    pub inverted: bool,
    #[serde(default)]
    pub blend: Blend,
    /// Per part keyform.
    pub opacities: Vec<f32>,
}

/// Free-form deformation lattice. The grid's control points live in the
/// PARENT's space; children of this deformer express their coordinates in
/// this deformer's normalized [0,1] x [0,1] domain.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WarpData {
    pub rows: u32,
    pub cols: u32,
    /// Interpolate cells as quads (bilinear) rather than split triangles.
    pub quad: bool,
    /// Per keyform.
    pub opacities: Vec<f32>,
    /// Keyform-major control points: `keyforms * (rows+1)*(cols+1) * 2`.
    #[serde(with = "f32_bin")]
    pub positions: Vec<f32>,
}

impl WarpData {
    pub fn grid_points(&self) -> usize {
        (self.rows as usize)
            .saturating_add(1)
            .saturating_mul((self.cols as usize).saturating_add(1))
    }
}

/// Rotate / scale / reflect children about an origin given in the parent's
/// space.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RotationData {
    pub base_angle: f32,
    /// All per keyform.
    pub opacities: Vec<f32>,
    pub angles: Vec<f32>,
    pub origins_x: Vec<f32>,
    pub origins_y: Vec<f32>,
    pub scales: Vec<f32>,
    pub reflect_x: Vec<bool>,
    pub reflect_y: Vec<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum DeformerKind {
    Warp(WarpData),
    Rotation(RotationData),
}

/// A node in the deformation hierarchy. Deformers are stored in topological
/// order, parents strictly before children, so the runtime evaluates them in
/// index order; import validates this.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Deformer {
    pub id: String,
    pub parent_part: Option<u32>,
    pub parent_deformer: Option<u32>,
    pub visible: bool,
    pub enabled: bool,
    pub binding: Binding,
    pub kind: DeformerKind,
}

/// Drawable flag bits, matching moc3 slot 42.
pub mod flags {
    pub const BLEND_ADDITIVE: u8 = 1 << 0;
    pub const BLEND_MULTIPLICATIVE: u8 = 1 << 1;
    pub const DOUBLE_SIDED: u8 = 1 << 2;
    pub const INVERTED_MASK: u8 = 1 << 3;
}

/// How a mesh's colour meets what is drawn under it, in moc3 5.3's order
/// (its number is the moc3 code). The two `Compatible` modes are the
/// premultiplied Add and Multiply every moc3 before 5.3 has, which the
/// `flags` bits also say; the rest need the backdrop to draw.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum ColorBlend {
    #[default]
    Normal,
    AddCompatible,
    MultiplyCompatible,
    Add,
    AddGlow,
    Darken,
    Multiply,
    ColorBurn,
    LinearBurn,
    Lighten,
    Screen,
    ColorDodge,
    Overlay,
    SoftLight,
    HardLight,
    LinearLight,
    Hue,
    Color,
}

impl ColorBlend {
    pub const ALL: [ColorBlend; 18] = [
        ColorBlend::Normal,
        ColorBlend::AddCompatible,
        ColorBlend::MultiplyCompatible,
        ColorBlend::Add,
        ColorBlend::AddGlow,
        ColorBlend::Darken,
        ColorBlend::Multiply,
        ColorBlend::ColorBurn,
        ColorBlend::LinearBurn,
        ColorBlend::Lighten,
        ColorBlend::Screen,
        ColorBlend::ColorDodge,
        ColorBlend::Overlay,
        ColorBlend::SoftLight,
        ColorBlend::HardLight,
        ColorBlend::LinearLight,
        ColorBlend::Hue,
        ColorBlend::Color,
    ];

    pub fn code(self) -> u8 {
        self as u8
    }

    pub fn from_code(c: u8) -> Option<Self> {
        Self::ALL.get(c as usize).copied()
    }
}

/// How a mesh's alpha meets the backdrop's, in moc3 5.3's order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum AlphaBlend {
    #[default]
    Over,
    Atop,
    Out,
    Conjoint,
    Disjoint,
}

impl AlphaBlend {
    pub const ALL: [AlphaBlend; 5] = [
        AlphaBlend::Over,
        AlphaBlend::Atop,
        AlphaBlend::Out,
        AlphaBlend::Conjoint,
        AlphaBlend::Disjoint,
    ];

    pub fn code(self) -> u8 {
        self as u8
    }

    pub fn from_code(c: u8) -> Option<Self> {
        Self::ALL.get(c as usize).copied()
    }
}

/// A mesh's blend: colour and alpha. moc3 5.3 stores it as one word,
/// colour in the low byte and alpha in the next.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Blend {
    pub color: ColorBlend,
    pub alpha: AlphaBlend,
}

impl Blend {
    pub const NORMAL: Blend = Blend { color: ColorBlend::Normal, alpha: AlphaBlend::Over };

    /// The blend a mesh draws with: what it stores, or for a mesh saved
    /// before blends had their own field, what its flag bits say.
    pub fn of(stored: Blend, mesh_flags: u8) -> Blend {
        if stored != Blend::NORMAL {
            return stored;
        }
        let color = if mesh_flags & flags::BLEND_ADDITIVE != 0 {
            ColorBlend::AddCompatible
        } else if mesh_flags & flags::BLEND_MULTIPLICATIVE != 0 {
            ColorBlend::MultiplyCompatible
        } else {
            ColorBlend::Normal
        };
        Blend { color, alpha: AlphaBlend::Over }
    }

    /// Draws with plain hardware blending (no backdrop read). The two
    /// compatible modes ignore the alpha blend, as in Cubism.
    pub fn is_simple(self) -> bool {
        matches!(self.color, ColorBlend::AddCompatible | ColorBlend::MultiplyCompatible)
            || self == Blend::NORMAL
    }

    pub fn to_moc3(self) -> i32 {
        i32::from(self.color.code()) | i32::from(self.alpha.code()) << 8
    }

    pub fn from_moc3(word: i32) -> Blend {
        Blend {
            color: ColorBlend::from_code((word & 0xff) as u8).unwrap_or_default(),
            alpha: AlphaBlend::from_code(((word >> 8) & 0xff) as u8).unwrap_or_default(),
        }
    }

    /// The flag bits a moc3 before 5.3 can carry: the closest of normal,
    /// add and multiply. `exact` is false when that changes how it looks.
    pub fn legacy_bits(self) -> (u8, bool) {
        use ColorBlend::*;
        let (bits, same) = match self.color {
            Normal => (0, true),
            AddCompatible => return (flags::BLEND_ADDITIVE, true),
            MultiplyCompatible => return (flags::BLEND_MULTIPLICATIVE, true),
            Add | AddGlow | Lighten | Screen | ColorDodge => (flags::BLEND_ADDITIVE, false),
            Multiply | Darken | ColorBurn | LinearBurn => (flags::BLEND_MULTIPLICATIVE, false),
            Overlay | SoftLight | HardLight | LinearLight | Hue | Color => (0, false),
        };
        (bits, same && self.alpha == AlphaBlend::Over)
    }
}

/// A textured triangle mesh: the thing that is actually drawn.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArtMesh {
    pub id: String,
    pub parent_part: Option<u32>,
    pub parent_deformer: Option<u32>,
    pub visible: bool,
    pub enabled: bool,
    pub texture: u32,
    pub flags: u8,
    /// Colour and alpha blend; Normal and Over by default, and for files
    /// saved before it existed (their flag bits still count, see
    /// [`Blend::of`]).
    #[serde(default)]
    pub blend: Blend,
    pub vertex_count: u32,
    /// `vertex_count * 2`, shared by every keyform.
    #[serde(with = "f32_bin")]
    pub uvs: Vec<f32>,
    /// Triangle list, indices local to this mesh.
    #[serde(with = "u16_bin")]
    pub indices: Vec<u16>,
    /// Art meshes whose alpha clips this one.
    pub masks: Vec<u32>,
    pub binding: Binding,
    /// All per keyform.
    pub opacities: Vec<f32>,
    pub draw_orders: Vec<f32>,
    /// Keyform-major vertex positions: `keyforms * vertex_count * 2`.
    #[serde(with = "f32_bin")]
    pub positions: Vec<f32>,
    /// Shapes added on top of the blended keyform, in no particular order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub blend_shapes: Vec<BlendShape>,
}

/// Keyforms added on top of an object's normal blend instead of multiplied
/// into it (Cubism 5.0). What lands on the object is `weight * (key form -
/// base form)`, the base form being its blend at every parameter's default.
/// The parameter's base key, 0.0, is implied and holds no keyform.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlendShape {
    /// Index into `Model::parameters`; that parameter has `blend_shape` set.
    pub parameter: u32,
    /// The keys that hold a keyform: strictly increasing, never 0.0.
    pub keys: Vec<f32>,
    /// One `vertex_count * 2` block per key, in key order.
    #[serde(with = "f32_bin")]
    pub positions: Vec<f32>,
}

/// Stitches two meshes' edges together: paired vertices pull toward each
/// other by a parameter-bound intensity. Pairs alternate: entry 2i is a
/// vertex of mesh `a`, entry 2i+1 a vertex of mesh `b`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Glue {
    pub id: String,
    pub art_mesh_a: u32,
    pub art_mesh_b: u32,
    pub binding: Binding,
    /// Per keyform.
    pub intensities: Vec<f32>,
    /// Per pair entry (even = mesh a, odd = mesh b).
    #[serde(with = "f32_bin")]
    pub weights: Vec<f32>,
    /// Vertex index in the owning mesh, parallel to `weights`.
    #[serde(with = "u16_bin")]
    pub indices: Vec<u16>,
}

/// One entry in a draw-order group: an art mesh, or a part whose own group
/// nests inside at this position.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DrawItem {
    pub part: bool,
    pub object: u32,
    pub child_group: Option<u32>,
}

/// The painting order tree. Items bucket-sort by their object's blended draw
/// order; a part item reserves a slot range for its child group.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DrawGroup {
    /// Objects in this group and every nested one.
    pub total: u32,
    pub min_order: i32,
    pub max_order: i32,
    pub items: Vec<DrawItem>,
}

/// A whole puppet.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Model {
    pub canvas: Canvas,
    pub parameters: Vec<Parameter>,
    pub parts: Vec<Part>,
    pub deformers: Vec<Deformer>,
    pub art_meshes: Vec<ArtMesh>,
    pub glues: Vec<Glue>,
    pub draw_groups: Vec<DrawGroup>,
    /// Pendulum physics, when the model has any. Older .aom files simply
    /// have none.
    #[serde(default)]
    pub physics: Option<PhysicsRig>,
    #[serde(default)]
    pub art_paths: Vec<ArtPath>,
    #[serde(default)]
    pub art_path_colors: Vec<[u8; 4]>,
    #[serde(default)]
    pub art_path_page: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ArtPath {
    pub mesh: String,
    pub anchors: Vec<[f32; 6]>,
    pub closed: bool,
    pub width: f32,
    pub feather: f32,
    pub stroke: bool,
    pub fill: bool,
    pub stroke_color: [u8; 4],
    pub fill_color: [u8; 4],
    pub step: f32,
    pub mask_only: bool,
}

impl Default for ArtPath {
    fn default() -> Self {
        Self {
            mesh: String::new(),
            anchors: Vec::new(),
            closed: false,
            width: 6.0,
            feather: 1.5,
            stroke: true,
            fill: false,
            stroke_color: [40, 30, 40, 255],
            fill_color: [255, 255, 255, 255],
            step: 6.0,
            mask_only: false,
        }
    }
}

impl Model {
    fn default_canvas() -> Canvas {
        Canvas { pixels_per_unit: 1.0, ..Canvas::default() }
    }

    pub fn new() -> Self {
        Self { canvas: Self::default_canvas(), ..Self::default() }
    }
}

/// What a physics input reads from its source parameter, or what an output
/// writes to its destination: a horizontal pull, a vertical pull, or a tilt.
/// From the published physics3.json spec (CubismSpecs repo).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum PhysicsIo {
    #[default]
    X,
    Y,
    Angle,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PhysicsInput {
    /// Source parameter id; resolved to an index when the puppet loads.
    pub param: String,
    /// Influence in percent, 0 to 100.
    pub weight: f32,
    pub kind: PhysicsIo,
    pub reflect: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PhysicsOutput {
    /// Destination parameter id.
    pub param: String,
    /// Which pendulum node's swing drives this output (0-based).
    pub vertex: u32,
    pub scale: f32,
    /// Influence in percent, 0 to 100.
    pub weight: f32,
    pub kind: PhysicsIo,
    pub reflect: bool,
}

/// One pendulum node. The first node is the anchor the inputs move; each
/// one after hangs off the previous at its rest offset's length.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PhysicsNode {
    pub x: f32,
    pub y: f32,
    /// Ease of swinging, usually 0.7 to 0.99.
    pub mobility: f32,
    /// Reaction time: 1 is neutral, above 1 reacts quicker.
    pub delay: f32,
    /// Scales the force the node feels.
    pub acceleration: f32,
    pub radius: f32,
}

/// Normalization for one input channel: source values map into this range
/// around the default.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct PhysicsRange {
    pub min: f32,
    pub default: f32,
    pub max: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PhysicsGroup {
    pub id: String,
    pub name: String,
    pub inputs: Vec<PhysicsInput>,
    pub outputs: Vec<PhysicsOutput>,
    pub nodes: Vec<PhysicsNode>,
    pub norm_position: PhysicsRange,
    pub norm_angle: PhysicsRange,
}

/// The whole pendulum rig, physics3.json's shape carried natively.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PhysicsRig {
    /// Simulation rate; 0 means step at the render rate.
    pub fps: f32,
    pub gravity_x: f32,
    pub gravity_y: f32,
    pub wind_x: f32,
    pub wind_y: f32,
    pub groups: Vec<PhysicsGroup>,
}
