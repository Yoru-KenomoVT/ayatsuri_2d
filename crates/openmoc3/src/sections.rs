//! Which slot in the offset table holds what.
//!
//! Verified by predicting `count * element_size` against each section's
//! measured extent: 218/218 on two v4.02 models, 115/115 on a v5.00 model,
//! 103/103 on a v6 model. Slot numbering is stable across versions; v5 appends
//! 137..=151 and v6 appends 152..=166. Nothing ever moves.

use crate::counts::Counts;

/// Which count sizes a section, and how wide one element is.
#[derive(Debug, Clone, Copy)]
pub struct SectionDesc {
    pub slot: usize,
    pub name: &'static str,
    /// Picks the count out of the table. A closure rather than an index, so a
    /// field rename is a compile error instead of a silent misread.
    pub count_of: fn(&Counts) -> i32,
    pub elem_size: usize,
}

/// Element widths that are not simply 4 bytes.
pub mod elem {
    /// 64-byte fixed record, NUL-terminated ASCII. Max name length 63.
    pub const ID: usize = 64;
    /// Runtime scratch space, not file data. Skip it.
    pub const RUNTIME_SPACE: usize = 8;
    /// A bitfield in ONE byte. The community pattern file says `u32`; real
    /// files disagree (356 art meshes occupy 384 bytes, not 1424).
    pub const DRAWABLE_FLAGS: usize = 1;
    /// Position indices are 16-bit and per-mesh local, so most are below 256.
    pub const INDEX16: usize = 2;
}

/// The v4.02 core layout, present in every later version at the same slots.
pub const SECTIONS: &[SectionDesc] = &[
    SectionDesc { slot:   2, name: "Parts: Runtime Space"                                  , count_of: |c| c.parts                       , elem_size: elem::RUNTIME_SPACE },
    SectionDesc { slot:   3, name: "Parts: IDs"                                            , count_of: |c| c.parts                       , elem_size: elem::ID },
    SectionDesc { slot:   4, name: "Parts: Keyform Binding Sources Indices"                , count_of: |c| c.parts                       , elem_size: 4 },
    SectionDesc { slot:   5, name: "Parts: Keyform Sources Begin Indices"                  , count_of: |c| c.parts                       , elem_size: 4 },
    SectionDesc { slot:   6, name: "Parts: Keyform Sources Counts"                         , count_of: |c| c.parts                       , elem_size: 4 },
    SectionDesc { slot:   7, name: "Parts: Visible"                                        , count_of: |c| c.parts                       , elem_size: 4 },
    SectionDesc { slot:   8, name: "Parts: Enabled"                                        , count_of: |c| c.parts                       , elem_size: 4 },
    SectionDesc { slot:   9, name: "Parts: Parent Part Indices"                            , count_of: |c| c.parts                       , elem_size: 4 },
    SectionDesc { slot:  10, name: "Deformers: Runtime Space"                              , count_of: |c| c.deformers                   , elem_size: elem::RUNTIME_SPACE },
    SectionDesc { slot:  11, name: "Deformers: IDs"                                        , count_of: |c| c.deformers                   , elem_size: elem::ID },
    SectionDesc { slot:  12, name: "Deformers: Keyform Binding Sources Indices"            , count_of: |c| c.deformers                   , elem_size: 4 },
    SectionDesc { slot:  13, name: "Deformers: Visible"                                    , count_of: |c| c.deformers                   , elem_size: 4 },
    SectionDesc { slot:  14, name: "Deformers: Enabled"                                    , count_of: |c| c.deformers                   , elem_size: 4 },
    SectionDesc { slot:  15, name: "Deformers: Parent Part Indices"                        , count_of: |c| c.deformers                   , elem_size: 4 },
    SectionDesc { slot:  16, name: "Deformers: Parent Deformer Indices"                    , count_of: |c| c.deformers                   , elem_size: 4 },
    SectionDesc { slot:  17, name: "Deformers: Types"                                      , count_of: |c| c.deformers                   , elem_size: 4 },
    SectionDesc { slot:  18, name: "Deformers: Specific Sources Indices"                   , count_of: |c| c.deformers                   , elem_size: 4 },
    SectionDesc { slot:  19, name: "Warp Deformers: Keyform Binding Sources Indices"       , count_of: |c| c.warp_deformers              , elem_size: 4 },
    SectionDesc { slot:  20, name: "Warp Deformers: Keyform Sources Begin Indices"         , count_of: |c| c.warp_deformers              , elem_size: 4 },
    SectionDesc { slot:  21, name: "Warp Deformers: Keyform Sources Counts"                , count_of: |c| c.warp_deformers              , elem_size: 4 },
    SectionDesc { slot:  22, name: "Warp Deformers: Vertex Counts"                         , count_of: |c| c.warp_deformers              , elem_size: 4 },
    SectionDesc { slot:  23, name: "Warp Deformers: Rows"                                  , count_of: |c| c.warp_deformers              , elem_size: 4 },
    SectionDesc { slot:  24, name: "Warp Deformers: Columns"                               , count_of: |c| c.warp_deformers              , elem_size: 4 },
    SectionDesc { slot:  25, name: "Rotation Deformers: Keyform Binding Sources Indices"   , count_of: |c| c.rotation_deformers          , elem_size: 4 },
    SectionDesc { slot:  26, name: "Rotation Deformers: Keyform Sources Begin Indices"     , count_of: |c| c.rotation_deformers          , elem_size: 4 },
    SectionDesc { slot:  27, name: "Rotation Deformers: Keyform Sources Counts"            , count_of: |c| c.rotation_deformers          , elem_size: 4 },
    SectionDesc { slot:  28, name: "Rotation Deformers: Base Angles"                       , count_of: |c| c.rotation_deformers          , elem_size: 4 },
    SectionDesc { slot:  29, name: "Art Meshes: Runtime Space 0"                           , count_of: |c| c.art_meshes                  , elem_size: elem::RUNTIME_SPACE },
    SectionDesc { slot:  30, name: "Art Meshes: Runtime Space 1"                           , count_of: |c| c.art_meshes                  , elem_size: elem::RUNTIME_SPACE },
    SectionDesc { slot:  31, name: "Art Meshes: Runtime Space 2"                           , count_of: |c| c.art_meshes                  , elem_size: elem::RUNTIME_SPACE },
    SectionDesc { slot:  32, name: "Art Meshes: Runtime Space 3"                           , count_of: |c| c.art_meshes                  , elem_size: elem::RUNTIME_SPACE },
    SectionDesc { slot:  33, name: "Art Meshes: IDs"                                       , count_of: |c| c.art_meshes                  , elem_size: elem::ID },
    SectionDesc { slot:  34, name: "Art Meshes: Keyform Binding Sources Indices"           , count_of: |c| c.art_meshes                  , elem_size: 4 },
    SectionDesc { slot:  35, name: "Art Meshes: Keyform Sources Begin Indices"             , count_of: |c| c.art_meshes                  , elem_size: 4 },
    SectionDesc { slot:  36, name: "Art Meshes: Keyform Sources Counts"                    , count_of: |c| c.art_meshes                  , elem_size: 4 },
    SectionDesc { slot:  37, name: "Art Meshes: Visible"                                   , count_of: |c| c.art_meshes                  , elem_size: 4 },
    SectionDesc { slot:  38, name: "Art Meshes: Enabled"                                   , count_of: |c| c.art_meshes                  , elem_size: 4 },
    SectionDesc { slot:  39, name: "Art Meshes: Parent Part Indices"                       , count_of: |c| c.art_meshes                  , elem_size: 4 },
    SectionDesc { slot:  40, name: "Art Meshes: Parent Deformer Indices"                   , count_of: |c| c.art_meshes                  , elem_size: 4 },
    SectionDesc { slot:  41, name: "Art Meshes: Texture No"                                , count_of: |c| c.art_meshes                  , elem_size: 4 },
    SectionDesc { slot:  42, name: "Art Meshes: Drawable Flags"                            , count_of: |c| c.art_meshes                  , elem_size: elem::DRAWABLE_FLAGS },
    SectionDesc { slot:  43, name: "Art Meshes: Vertex Counts"                             , count_of: |c| c.art_meshes                  , elem_size: 4 },
    SectionDesc { slot:  44, name: "Art Meshes: UV Sources Begin Indices"                  , count_of: |c| c.art_meshes                  , elem_size: 4 },
    SectionDesc { slot:  45, name: "Art Meshes: Position Index Sources Begin Indices"      , count_of: |c| c.art_meshes                  , elem_size: 4 },
    SectionDesc { slot:  46, name: "Art Meshes: Position Index Sources Counts"             , count_of: |c| c.art_meshes                  , elem_size: 4 },
    SectionDesc { slot:  47, name: "Art Meshes: Drawable Mask Sources Begin Indices"       , count_of: |c| c.art_meshes                  , elem_size: 4 },
    SectionDesc { slot:  48, name: "Art Meshes: Drawable Mask Sources Counts"              , count_of: |c| c.art_meshes                  , elem_size: 4 },
    SectionDesc { slot:  49, name: "Parameters: Runtime Space"                             , count_of: |c| c.parameters                  , elem_size: elem::RUNTIME_SPACE },
    SectionDesc { slot:  50, name: "Parameters: IDs"                                       , count_of: |c| c.parameters                  , elem_size: elem::ID },
    SectionDesc { slot:  51, name: "Parameters: Max Values"                                , count_of: |c| c.parameters                  , elem_size: 4 },
    SectionDesc { slot:  52, name: "Parameters: Min Values"                                , count_of: |c| c.parameters                  , elem_size: 4 },
    SectionDesc { slot:  53, name: "Parameters: Default Values"                            , count_of: |c| c.parameters                  , elem_size: 4 },
    SectionDesc { slot:  54, name: "Parameters: Repeat"                                    , count_of: |c| c.parameters                  , elem_size: 4 },
    SectionDesc { slot:  55, name: "Parameters: Decimal Places"                            , count_of: |c| c.parameters                  , elem_size: 4 },
    SectionDesc { slot:  56, name: "Parameters: Parameter Binding Sources Begin Indices"   , count_of: |c| c.parameters                  , elem_size: 4 },
    SectionDesc { slot:  57, name: "Parameters: Parameter Binding Sources Counts"          , count_of: |c| c.parameters                  , elem_size: 4 },
    SectionDesc { slot:  58, name: "Part Keyforms: Draw Orders"                            , count_of: |c| c.part_keyforms               , elem_size: 4 },
    SectionDesc { slot:  59, name: "Warp Deformer Keyforms: Opacities"                     , count_of: |c| c.warp_deformer_keyforms      , elem_size: 4 },
    SectionDesc { slot:  60, name: "Warp Deformer Keyforms: Keyform Position Sources Beg"  , count_of: |c| c.warp_deformer_keyforms      , elem_size: 4 },
    SectionDesc { slot:  61, name: "Rotation Deformer Keyforms: Opacities"                 , count_of: |c| c.rotation_deformer_keyforms  , elem_size: 4 },
    SectionDesc { slot:  62, name: "Rotation Deformer Keyforms: Angles"                    , count_of: |c| c.rotation_deformer_keyforms  , elem_size: 4 },
    SectionDesc { slot:  63, name: "Rotation Deformer Keyforms: X Origins"                 , count_of: |c| c.rotation_deformer_keyforms  , elem_size: 4 },
    SectionDesc { slot:  64, name: "Rotation Deformer Keyforms: Y Origins"                 , count_of: |c| c.rotation_deformer_keyforms  , elem_size: 4 },
    SectionDesc { slot:  65, name: "Rotation Deformer Keyforms: Scales"                    , count_of: |c| c.rotation_deformer_keyforms  , elem_size: 4 },
    SectionDesc { slot:  66, name: "Rotation Deformer Keyforms: Reflect on X"              , count_of: |c| c.rotation_deformer_keyforms  , elem_size: 4 },
    SectionDesc { slot:  67, name: "Rotation Deformer Keyforms: Reflect on Y"              , count_of: |c| c.rotation_deformer_keyforms  , elem_size: 4 },
    SectionDesc { slot:  68, name: "Art Mesh Keyforms: Opacities"                          , count_of: |c| c.art_mesh_keyforms           , elem_size: 4 },
    SectionDesc { slot:  69, name: "Art Mesh Keyforms: Draw Orders"                        , count_of: |c| c.art_mesh_keyforms           , elem_size: 4 },
    SectionDesc { slot:  70, name: "Art Mesh Keyforms: Keyform Position Sources Begin In"  , count_of: |c| c.art_mesh_keyforms           , elem_size: 4 },
    SectionDesc { slot:  71, name: "Keyform Positions: Coordinates (XY)"                   , count_of: |c| c.keyform_positions           , elem_size: 4 },
    SectionDesc { slot:  72, name: "Parameter Binding Indices: Binding Sources Indices"    , count_of: |c| c.parameter_binding_indices   , elem_size: 4 },
    SectionDesc { slot:  73, name: "Keyform Bindings: Parameter Binding Index Sources Be"  , count_of: |c| c.keyform_bindings            , elem_size: 4 },
    SectionDesc { slot:  74, name: "Keyform Bindings: Parameter Binding Index Sources Co"  , count_of: |c| c.keyform_bindings            , elem_size: 4 },
    SectionDesc { slot:  75, name: "Parameter Bindings: Keys Sources Begin Indices"        , count_of: |c| c.parameter_bindings          , elem_size: 4 },
    SectionDesc { slot:  76, name: "Parameter Bindings: Keys Sources Counts"               , count_of: |c| c.parameter_bindings          , elem_size: 4 },
    SectionDesc { slot:  77, name: "Keys: Key Values"                                      , count_of: |c| c.keys                        , elem_size: 4 },
    SectionDesc { slot:  78, name: "UVs: UVs"                                              , count_of: |c| c.uvs                         , elem_size: 4 },
    SectionDesc { slot:  79, name: "Position Indices: Indices (s16)"                       , count_of: |c| c.position_indices            , elem_size: elem::INDEX16 },
    SectionDesc { slot:  80, name: "Drawable Masks: Art Mesh Sources Indices"              , count_of: |c| c.drawable_masks              , elem_size: 4 },
    SectionDesc { slot:  81, name: "Draw Order Groups: Object Sources Begin Indices"       , count_of: |c| c.draw_order_groups           , elem_size: 4 },
    SectionDesc { slot:  82, name: "Draw Order Groups: Object Sources Counts"              , count_of: |c| c.draw_order_groups           , elem_size: 4 },
    SectionDesc { slot:  83, name: "Draw Order Groups: Object Sources Total Counts"        , count_of: |c| c.draw_order_groups           , elem_size: 4 },
    SectionDesc { slot:  84, name: "Draw Order Groups: Maximum Draw Orders"                , count_of: |c| c.draw_order_groups           , elem_size: 4 },
    SectionDesc { slot:  85, name: "Draw Order Groups: Minimum Draw Orders"                , count_of: |c| c.draw_order_groups           , elem_size: 4 },
    SectionDesc { slot:  86, name: "Draw Order Group Objects: Types"                       , count_of: |c| c.draw_order_group_objects    , elem_size: 4 },
    SectionDesc { slot:  87, name: "Draw Order Group Objects: Indices"                     , count_of: |c| c.draw_order_group_objects    , elem_size: 4 },
    SectionDesc { slot:  88, name: "Draw Order Group Objects: Self Indices"                , count_of: |c| c.draw_order_group_objects    , elem_size: 4 },
    SectionDesc { slot:  89, name: "Glue: Runtime Space"                                   , count_of: |c| c.glue                        , elem_size: elem::RUNTIME_SPACE },
    SectionDesc { slot:  90, name: "Glue: IDs"                                             , count_of: |c| c.glue                        , elem_size: elem::ID },
    SectionDesc { slot:  91, name: "Glue: Keyform Binding Sources Indices"                 , count_of: |c| c.glue                        , elem_size: 4 },
    SectionDesc { slot:  92, name: "Glue: Keyform Sources Begin Indices"                   , count_of: |c| c.glue                        , elem_size: 4 },
    SectionDesc { slot:  93, name: "Glue: Keyform Binding Sources Counts"                  , count_of: |c| c.glue                        , elem_size: 4 },
    SectionDesc { slot:  94, name: "Glue: Art Mesh Indices (A)"                            , count_of: |c| c.glue                        , elem_size: 4 },
    SectionDesc { slot:  95, name: "Glue: Art Mesh Indices (B)"                            , count_of: |c| c.glue                        , elem_size: 4 },
    SectionDesc { slot:  96, name: "Glue: Info Sources Begin Indices"                      , count_of: |c| c.glue                        , elem_size: 4 },
    SectionDesc { slot:  97, name: "Glue: Info Sources Counts"                             , count_of: |c| c.glue                        , elem_size: 4 },
    SectionDesc { slot:  98, name: "Glue Info: Weights"                                    , count_of: |c| c.glue_info                   , elem_size: 4 },
    SectionDesc { slot:  99, name: "Glue Info: Position Indices (s16)"                     , count_of: |c| c.glue_info                   , elem_size: elem::INDEX16 },
    SectionDesc { slot: 100, name: "Glue Keyforms: Intensities"                            , count_of: |c| c.glue_keyforms               , elem_size: 4 },
    SectionDesc { slot: 101, name: "Warp Deformer Keyforms v3.3+: Quad Source (bool32)"    , count_of: |c| c.warp_deformers              , elem_size: 4 },
    SectionDesc { slot: 102, name: "Parameter Extensions v4.2+: Runtime Space"             , count_of: |c| c.parameters                  , elem_size: elem::RUNTIME_SPACE },
    SectionDesc { slot: 103, name: "Parameter Extensions v4.2+: Keys Sources Begin Indic"  , count_of: |c| c.parameters                  , elem_size: 4 },
    SectionDesc { slot: 104, name: "Parameter Extensions v4.2+: Keys Sources Counts"       , count_of: |c| c.parameters                  , elem_size: 4 },
    SectionDesc { slot: 105, name: "Warp Deformer Keyforms v4.2+: Keyform Color Sources "  , count_of: |c| c.warp_deformers              , elem_size: 4 },
    SectionDesc { slot: 106, name: "Rotation Deformer Keyforms v4.2+: Keyform Color Sour"  , count_of: |c| c.rotation_deformers          , elem_size: 4 },
    SectionDesc { slot: 107, name: "Art Mesh Keyforms v4.2+: Keyform Color Sources Begin"  , count_of: |c| c.art_meshes                  , elem_size: 4 },
    SectionDesc { slot: 108, name: "Keyform Multiply Colors v4.2+: Red"                    , count_of: |c| c.keyform_multiply_colors     , elem_size: 4 },
    SectionDesc { slot: 109, name: "Keyform Multiply Colors v4.2+: Green"                  , count_of: |c| c.keyform_multiply_colors     , elem_size: 4 },
    SectionDesc { slot: 110, name: "Keyform Multiply Colors v4.2+: Blue"                   , count_of: |c| c.keyform_multiply_colors     , elem_size: 4 },
    SectionDesc { slot: 111, name: "Keyform Screen Colors v4.2+: Red"                      , count_of: |c| c.keyform_screen_colors       , elem_size: 4 },
    SectionDesc { slot: 112, name: "Keyform Screen Colors v4.2+: Green"                    , count_of: |c| c.keyform_screen_colors       , elem_size: 4 },
    SectionDesc { slot: 113, name: "Keyform Screen Colors v4.2+: Blue"                     , count_of: |c| c.keyform_screen_colors       , elem_size: 4 },
    SectionDesc { slot: 114, name: "Parameters v4.2+: Types"                               , count_of: |c| c.parameters                  , elem_size: 4 },
    SectionDesc { slot: 115, name: "Parameters v4.2+: Blend Shape Binding Begin Indices"   , count_of: |c| c.parameters                  , elem_size: 4 },
    SectionDesc { slot: 116, name: "Parameters v4.2+: Blend Shape Binding Counts"          , count_of: |c| c.parameters                  , elem_size: 4 },
];

/// Slots worth naming: everything else is reached through them.
pub mod slot {
    pub const COUNT_TABLE: usize = 0;
    pub const CANVAS_INFO: usize = 1;
    pub const PART_IDS: usize = 3;
    pub const DEFORMER_IDS: usize = 11;
    pub const ART_MESH_IDS: usize = 33;
    pub const PARAMETER_IDS: usize = 50;
    pub const PARAMETER_MAX: usize = 51;
    pub const PARAMETER_MIN: usize = 52;
    pub const PARAMETER_DEFAULT: usize = 53;
    pub const GLUE_IDS: usize = 90;
}

/// TODO: look a slot up. Linear scan over ~117 entries is fine at load time and
/// this is not on the frame path; a match or a lazy map is premature here.
pub fn describe(_slot: usize) -> Option<&'static SectionDesc> {
    todo!()
}
