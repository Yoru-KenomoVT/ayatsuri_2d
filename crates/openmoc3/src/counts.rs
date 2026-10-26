//! The count info table: how many of each kind of object the file holds.
//!
//! Section 0 of the offset table points at it. Every other section is sized as
//! `count * element_size`, so this table is what makes the rest readable.
//!
//! **Equal counts are not the same thing.** In some models `parts` happens to
//! equal `part_keyforms`, and `art_meshes` equals `draw_group_objects`. That is
//! coincidence. Never infer one from another.

use crate::{read_i32, Error};

/// Highest count that still cannot overflow `count * 64` in a `u32`. A hostile
/// file will try both a negative count and a huge one.
const MAX_COUNT: i32 = (u32::MAX / 64) as i32;

/// How many counts are defined through v6. The table has room for more
/// (32 entries in v4, 64 in v5 and v6); the rest are reserved zeroes.
const DEFINED: usize = 38;

/// Field order is the on-disk order. Indices 0..=31 exist in v4.02; v5 added
/// 32..=34 for the remaining blend-shape targets; v6 35..=37 for offscreens.
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
    pub blend_shape_parameter_bindings: i32,
    pub blend_shape_keyform_bindings: i32,
    pub blend_shapes_warp_deformers: i32,
    pub blend_shapes_art_meshes: i32,
    pub blend_shape_constraint_indices: i32,
    pub blend_shape_constraints: i32,
    pub blend_shape_constraint_values: i32,
    // ---- v5.00 and later ----
    pub blend_shapes_parts: i32,
    pub blend_shapes_rotation_deformers: i32,
    pub blend_shapes_glue: i32,
    // ---- v6 (5.3) and later ----
    pub offscreens: i32,
    pub offscreen_keyforms: i32,
    pub blend_shapes_offscreens: i32,
}

impl Counts {
    /// Read the counts in declaration order as little-endian i32. A table
    /// shorter than 35 entries (the 128-byte v3/v4 table) leaves the rest
    /// zero. Rejects any negative count, and any count large enough that
    /// `count * 64` would overflow.
    pub fn parse(bytes: &[u8]) -> Result<Self, Error> {
        let mut v = [0i32; DEFINED];
        for (index, out) in v.iter_mut().enumerate() {
            let Some(raw) = read_i32(bytes, index.saturating_mul(4)) else { break };
            if !(0..=MAX_COUNT).contains(&raw) {
                return Err(Error::ImplausibleCount { index, value: raw });
            }
            *out = raw;
        }
        let [parts, deformers, warp_deformers, rotation_deformers, art_meshes, parameters, part_keyforms, warp_deformer_keyforms, rotation_deformer_keyforms, art_mesh_keyforms, keyform_positions, parameter_binding_indices, keyform_bindings, parameter_bindings, keys, uvs, position_indices, drawable_masks, draw_order_groups, draw_order_group_objects, glue, glue_info, glue_keyforms, keyform_multiply_colors, keyform_screen_colors, blend_shape_parameter_bindings, blend_shape_keyform_bindings, blend_shapes_warp_deformers, blend_shapes_art_meshes, blend_shape_constraint_indices, blend_shape_constraints, blend_shape_constraint_values, blend_shapes_parts, blend_shapes_rotation_deformers, blend_shapes_glue, offscreens, offscreen_keyforms, blend_shapes_offscreens] =
            v;
        Ok(Self {
            parts,
            deformers,
            warp_deformers,
            rotation_deformers,
            art_meshes,
            parameters,
            part_keyforms,
            warp_deformer_keyforms,
            rotation_deformer_keyforms,
            art_mesh_keyforms,
            keyform_positions,
            parameter_binding_indices,
            keyform_bindings,
            parameter_bindings,
            keys,
            uvs,
            position_indices,
            drawable_masks,
            draw_order_groups,
            draw_order_group_objects,
            glue,
            glue_info,
            glue_keyforms,
            keyform_multiply_colors,
            keyform_screen_colors,
            blend_shape_parameter_bindings,
            blend_shape_keyform_bindings,
            blend_shapes_warp_deformers,
            blend_shapes_art_meshes,
            blend_shape_constraint_indices,
            blend_shape_constraints,
            blend_shape_constraint_values,
            blend_shapes_parts,
            blend_shapes_rotation_deformers,
            blend_shapes_glue,
            offscreens,
            offscreen_keyforms,
            blend_shapes_offscreens,
        })
    }

    /// The one relationship that holds in every model seen, across all three
    /// versions: `deformers == warp_deformers + rotation_deformers`. A file
    /// where it fails is malformed or misread.
    pub fn is_self_consistent(&self) -> bool {
        self.warp_deformers.checked_add(self.rotation_deformers) == Some(self.deformers)
    }
}
