//! What to draw after an update: every visible mesh in draw order, as
//! views into the front frame and the model's fixed mesh data. Nothing is
//! copied. Any thread may read while another updates.

use crate::error::{Status, guard, set_error};
use crate::handle::{MeshStatic, Model, model, output, put};

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

fn drawable(mesh: u32, s: &MeshStatic, positions: &[f32], opacity: f32) -> Drawable {
    Drawable {
        mesh,
        texture: s.texture,
        flags: s.flags,
        opacity,
        vertex_count: (positions.len() / 2) as u32,
        index_count: s.indices.len() as u32,
        mask_count: s.masks.len() as u32,
        positions: positions.as_ptr(),
        uvs: s.uvs.as_ptr(),
        indices: s.indices.as_ptr(),
        masks: s.masks.as_ptr(),
    }
}

/// The visible meshes after the last update, back to front, into `out`.
/// Returns how many there are; with `n` smaller than that (zero and a
/// null `out` to just ask) only the first `n` are written. The views stay
/// valid while the next update runs, until the one after it starts. Masks
/// are not among the drawables unless drawn in their own right; use
/// `ay2d_mesh_geometry` to render a mask. Any thread.
#[unsafe(no_mangle)]
pub extern "C" fn ay2d_drawables(m: *const Model, out: *mut Drawable, n: usize) -> u32 {
    guard(0, || {
        let Ok(model) = model(m) else { return 0 };
        let Ok(out) = output(out, n) else { return 0 };
        let frame = model.front();
        for (slot, &(mesh, opacity)) in out.iter_mut().zip(&frame.drawn) {
            if let Some(s) = model.facts.meshes.get(mesh as usize) {
                *slot = drawable(mesh, s, frame.mesh(s.offset, s.len), opacity);
            }
        }
        frame.drawn.len() as u32
    })
}

/// A mesh's geometry after the last update, visible or not: for rendering
/// a mask, or for a host that keeps everything on the GPU and only streams
/// positions. `uvs` and `indices` never change for a mesh; upload them
/// once and read `positions` after each update. Fails with range past the
/// mesh count. Any thread.
#[unsafe(no_mangle)]
pub extern "C" fn ay2d_mesh_geometry(m: *const Model, mesh: u32, out: *mut Drawable) -> Status {
    guard(Status::Internal, || {
        let model = match model(m) {
            Ok(m) => m,
            Err(s) => return s,
        };
        let Some(s) = model.facts.meshes.get(mesh as usize) else {
            set_error(format!("mesh {mesh} is past the model's {}", model.facts.meshes.len()));
            return Status::Range;
        };
        let frame = model.front();
        put(out, drawable(mesh, s, frame.mesh(s.offset, s.len), 1.0))
    })
}
