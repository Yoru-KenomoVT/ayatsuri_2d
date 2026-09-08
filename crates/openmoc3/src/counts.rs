//! The count info table: how many of each kind of object the file holds.
//!
//! Section 0 of the offset table points at it. Every other section is sized as
//! `count * element_size`, so this table is what makes the rest readable.
//!
//! **Equal counts are not the same thing.** In some models `parts` happens to
//! equal `part_keyforms`, and `art_meshes` equals `draw_group_objects`. That is
//! coincidence. Never infer one from another.

/// Field order is the on-disk order. Indices 0..=24 exist in v4.02; v5 added
/// 25..=34 for blend shapes.
#[derive(Debug, Clone, Copy, Default)]
pub struct Counts {
    pub parts: i32,
    pub deformers: i32,
    pub warp_deformers: i32,
    pub rotation_deformers: i32,
    pub art_meshes: i32,
    pub parameters: i32,
    pub part_keyforms: i32,
    pub warp_deformer_keyforms: i32,
    pub rotation_deformer_keyforms: i32,
    pub art_mesh_keyforms: i32,
    pub keyform_positions: i32,
    pub parameter_binding_indices: i32,
    pub keyform_bindings: i32,
    pub parameter_bindings: i32,
    pub keys: i32,
    pub uvs: i32,
    pub position_indices: i32,
    pub drawable_masks: i32,
    pub draw_order_groups: i32,
    pub draw_order_group_objects: i32,
    pub glue: i32,
    pub glue_info: i32,
    pub glue_keyforms: i32,
    pub keyform_multiply_colors: i32,
    pub keyform_screen_colors: i32,
    // ---- v5.00 and later ----
    pub blend_shape_parameter_bindings: i32,
    pub blend_shape_keyform_bindings: i32,
    pub blend_shapes_warp_deformers: i32,
    pub blend_shapes_art_meshes: i32,
    pub blend_shape_constraint_indices: i32,
    pub blend_shape_constraints: i32,
    pub blend_shape_constraint_values: i32,
    pub blend_shapes_parts: i32,
    pub blend_shapes_rotation_deformers: i32,
    pub blend_shapes_glue: i32,
}

impl Counts {
    /// TODO: read them in declaration order as little-endian i32.
    ///
    /// Reject any negative count, and any count large enough that
    /// `count * 64` would overflow. A hostile file will try both.
    pub fn parse(_bytes: &[u8]) -> Result<Self, crate::Error> {
        todo!()
    }

    /// The one relationship that holds in every model seen, across all three
    /// versions. A file where it fails is malformed or misread.
    ///
    /// TODO: `deformers == warp_deformers + rotation_deformers`
    pub fn is_self_consistent(&self) -> bool {
        todo!()
    }
}
