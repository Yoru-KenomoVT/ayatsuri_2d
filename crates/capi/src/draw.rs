//! What to draw after an update: every visible mesh in draw order, as
//! views into the model's own buffers. Nothing is copied; the pointers
//! are good until the next `ay2d_update` or `ay2d_free`.

use crate::error::{Status, guard, set_error};
use crate::handle::{Model, model, output, put};

/// Drawn with additive blending.
pub const AY2D_FLAG_ADDITIVE: u32 = 1;
/// Drawn with multiplicative blending.
pub const AY2D_FLAG_MULTIPLICATIVE: u32 = 2;
/// Both faces drawn; no back-face culling.
pub const AY2D_FLAG_DOUBLE_SIDED: u32 = 4;
/// The masks cut the mesh out where they are drawn, not where they are not.
pub const AY2D_FLAG_INVERTED_MASK: u32 = 8;

/// One mesh to draw: where its vertices are now, where they read the
/// texture, and how. `positions` and `uvs` hold `vertex_count` xy pairs in
/// model units and 0 to 1; `indices` a triangle list into them; `masks`
/// the meshes (by mesh index) whose texture alpha clips this one.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct Drawable {
    pub mesh: u32,
    pub texture: u32,
    /// `AY2D_FLAG_*` bits.
    pub flags: u32,
    pub opacity: f32,
    pub vertex_count: u32,
    pub index_count: u32,
    pub mask_count: u32,
    pub positions: *const f32,
    pub uvs: *const f32,
    pub indices: *const u16,
    pub masks: *const u32,
}

/// The visible meshes after the last update, back to front, into `out`.
/// Returns how many there are; with `n` smaller than that (zero and a
/// null `out` to just ask) only the first `n` are written. Masks are not
/// among the drawables unless drawn in their own right; use
/// `ay2d_mesh_geometry` to render a mask.
///
/// `m` is a live handle; `out` holds `n` writable drawables, or `n` is zero.
#[unsafe(no_mangle)]
pub extern "C" fn ay2d_drawables(m: *const Model, out: *mut Drawable, n: usize) -> u32 {
    guard(0, || {
        let Ok(model) = model(m) else { return 0 };
        let Ok(out) = output(out, n) else { return 0 };
        let mut written = 0;
        for (slot, d) in out.iter_mut().zip(model.puppet.drawables()) {
            *slot = Drawable {
                mesh: d.mesh as u32,
                texture: d.texture,
                flags: u32::from(d.flags),
                opacity: d.opacity,
                vertex_count: (d.positions.len() / 2) as u32,
                index_count: d.indices.len() as u32,
                mask_count: d.masks.len() as u32,
                positions: d.positions.as_ptr(),
                uvs: d.uvs.as_ptr(),
                indices: d.indices.as_ptr(),
                masks: d.masks.as_ptr(),
            };
            written += 1;
        }
        let _ = written;
        model.order.len() as u32
    })
}

/// A mesh's geometry right now, visible or not: for rendering a mask, or
/// for a host that keeps everything on the GPU and only streams positions.
/// `uvs` and `indices` never change for a mesh; upload them once and read
/// `positions` after each update. Fails with range past the mesh count.
///
/// `m` is a live handle; `out` points at one writable drawable.
#[unsafe(no_mangle)]
pub extern "C" fn ay2d_mesh_geometry(m: *const Model, mesh: u32, out: *mut Drawable) -> Status {
    guard(Status::Internal, || {
        let model = match model(m) {
            Ok(m) => m,
            Err(s) => return s,
        };
        let Some(g) = model.puppet.mesh_geometry(mesh as usize) else {
            set_error(format!("mesh {mesh} is past the model's {}", model.mesh_ids.len()));
            return Status::Range;
        };
        let am = model.puppet.model().art_meshes.get(mesh as usize);
        put(
            out,
            Drawable {
                mesh,
                texture: g.texture,
                flags: am.map_or(0, |a| u32::from(a.flags)),
                opacity: 1.0,
                vertex_count: (g.positions.len() / 2) as u32,
                index_count: g.indices.len() as u32,
                mask_count: am.map_or(0, |a| a.masks.len() as u32),
                positions: g.positions.as_ptr(),
                uvs: g.uvs.as_ptr(),
                indices: g.indices.as_ptr(),
                masks: am.map_or(std::ptr::null(), |a| a.masks.as_ptr()),
            },
        )
    })
}
