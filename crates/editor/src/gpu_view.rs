// Bridges the puppet to the wgpu renderer through an egui paint callback.
// Builds the frame packet on the UI thread: vertices in canvas pixels, and
// the mask/draw command list with stencil references assigned.

use std::sync::Mutex;

use ayatsuri_core::Puppet;
use ayatsuri_format::model::{Canvas, ColorBlend, flags};
use ayatsuri_render::{BlendMode, Cmd, GpuRenderer, MaskMode, NO_CLIP, Packet, TexData, Vertex};

pub struct ModelPaint {
    packet: Mutex<Option<Packet>>,
}

/// The largest texture side the canvas's GPU takes, once it has drawn; 0
/// before that.
static MAX_TEXTURE: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

/// The largest texture side this GPU takes, if the canvas has drawn yet.
pub fn max_texture_side() -> Option<u32> {
    match MAX_TEXTURE.load(std::sync::atomic::Ordering::Relaxed) {
        0 => None,
        n => Some(n),
    }
}

impl egui_wgpu::CallbackTrait for ModelPaint {
    fn prepare(
        &self,
        device: &egui_wgpu::wgpu::Device,
        queue: &egui_wgpu::wgpu::Queue,
        _screen: &egui_wgpu::ScreenDescriptor,
        encoder: &mut egui_wgpu::wgpu::CommandEncoder,
        resources: &mut egui_wgpu::CallbackResources,
    ) -> Vec<egui_wgpu::wgpu::CommandBuffer> {
        MAX_TEXTURE
            .store(device.limits().max_texture_dimension_2d, std::sync::atomic::Ordering::Relaxed);
        if let Ok(mut guard) = self.packet.lock()
            && let Some(packet) = guard.take()
            && let Some(renderer) = resources.get_mut::<GpuRenderer>()
        {
            // this runs inside eframe's render, outside the frame guard, so
            // it gets its own: a bad frame skips drawing the model, the hook
            // has logged why, and the editor reports it next frame
            let ok = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                renderer.prepare(device, queue, encoder, &packet);
            }));
            if ok.is_err() {
                log::error!("model rendering failed this frame and was skipped");
            }
        }
        Vec::new()
    }

    fn paint(
        &self,
        _info: egui::PaintCallbackInfo,
        render_pass: &mut egui_wgpu::wgpu::RenderPass<'static>,
        resources: &egui_wgpu::CallbackResources,
    ) {
        if let Some(renderer) = resources.get::<GpuRenderer>() {
            let ok = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                renderer.paint(render_pass);
            }));
            if ok.is_err() {
                log::error!("model painting failed this frame and was skipped");
            }
        }
    }
}

fn push_mesh(
    verts: &mut Vec<Vertex>,
    indices: &mut Vec<u32>,
    c: &Canvas,
    positions: &[f32],
    uvs: &[f32],
    tris: &[u16],
    color: [f32; 4],
    clip: [f32; 4],
) -> std::ops::Range<u32> {
    let base = verts.len() as u32;
    let start = indices.len() as u32;
    let (pos_pairs, _) = positions.as_chunks::<2>();
    let (uv_pairs, _) = uvs.as_chunks::<2>();
    for (xy, uv) in pos_pairs.iter().zip(uv_pairs) {
        verts.push(Vertex {
            pos: [c.origin_x + xy[0] * c.pixels_per_unit, c.origin_y + xy[1] * c.pixels_per_unit],
            uv: [uv[0], uv[1]],
            color,
            clip,
        });
    }
    indices.extend(tris.iter().map(|&i| base + i as u32));
    start..indices.len() as u32
}

/// Build the frame and hand back the egui callback to add to a painter.
#[allow(clippy::too_many_arguments)]
pub fn callback(
    rect: egui::Rect,
    puppet: &Puppet,
    textures: Option<std::sync::Arc<Vec<TexData>>>,
    uploaded: std::sync::Arc<std::sync::atomic::AtomicBool>,
    pages: Option<std::sync::Arc<Vec<(usize, TexData)>>>,
    pages_uploaded: std::sync::Arc<std::sync::atomic::AtomicBool>,
    drop_pages: Option<Vec<usize>>,
    // canvas px -> screen points: screen = screen_origin + px * scale
    screen_origin: egui::Pos2,
    scale: f32,
    pixels_per_point: f32,
    solo: Option<&Solo>,
    art: &[[f32; 4]],
) -> egui::PaintCallback {
    let w = ((rect.width() * pixels_per_point) as u32).max(1);
    let h = ((rect.height() * pixels_per_point) as u32).max(1);
    let sx = scale * pixels_per_point / w as f32 * 2.0;
    let sy = -(scale * pixels_per_point / h as f32 * 2.0);
    let ox = (screen_origin.x - rect.min.x) * pixels_per_point / w as f32 * 2.0 - 1.0;
    let oy = 1.0 - (screen_origin.y - rect.min.y) * pixels_per_point / h as f32 * 2.0;
    let mut packet = packet(puppet, w, h, [sx, sy, ox, oy], solo, art);
    packet.textures = textures;
    packet.uploaded = Some(uploaded);
    packet.pages = pages;
    packet.pages_uploaded = Some(pages_uploaded);
    packet.drop_pages = drop_pages;
    egui_wgpu::Callback::new_paint_callback(rect, ModelPaint { packet: Mutex::new(Some(packet)) })
}

/// The renderer's blend for a mesh or layer blend.
fn blend_mode(b: ayatsuri_format::model::Blend) -> BlendMode {
    match b.color {
        ColorBlend::AddCompatible => BlendMode::Additive,
        ColorBlend::MultiplyCompatible => BlendMode::Multiplicative,
        _ if b.is_simple() => BlendMode::Normal,
        _ => BlendMode::Advanced { color: b.color.code(), alpha: b.alpha.code() },
    }
}

/// A frame's packet as it is built: geometry, commands, and the mask set
/// the mask buffer holds by then.
struct Frame<'a> {
    puppet: &'a Puppet,
    /// Each mesh's own art on its page (see `Session::art_boxes`).
    art: &'a [[f32; 4]],
    c: Canvas,
    to_ndc: [f32; 4],
    verts: Vec<Vertex>,
    indices: Vec<u32>,
    cmds: Vec<Cmd>,
    /// The masks the buffer holds by now, with their inherited sets.
    last_masks: Option<(Vec<u32>, Vec<Vec<u32>>)>,
}

impl Frame<'_> {
    /// Fill the mask buffer with `masks`, cut down to inside `and`'s sets
    /// too (inherited clips), unless it already holds exactly that.
    fn mask_set(&mut self, masks: &[u32], and: &[Vec<u32>]) {
        if self.last_masks.as_ref().is_some_and(|(m, a)| m == masks && a == and) {
            return;
        }
        self.last_masks = Some((masks.to_vec(), and.to_vec()));
        let shapes = self.mask_shapes(masks);
        let and = and.iter().map(|set| self.mask_shapes(set)).collect();
        self.cmds.push(Cmd::MaskSet { shapes, and });
    }

    /// One mask set's meshes as geometry and their textures.
    fn mask_shapes(&mut self, masks: &[u32]) -> Vec<(std::ops::Range<u32>, u32)> {
        let mut shapes = Vec::new();
        for &src in masks {
            if let Some(g) = self.puppet.mesh_geometry(src as usize) {
                let range = push_mesh(
                    &mut self.verts,
                    &mut self.indices,
                    &self.c,
                    g.positions,
                    g.uvs,
                    g.indices,
                    [1.0; 4],
                    self.art.get(src as usize).copied().unwrap_or(NO_CLIP),
                );
                shapes.push((range, g.texture));
            }
        }
        shapes
    }

    /// Land an offscreen part's layer: a quad over the whole target, its
    /// uvs covering the layer, its alpha the layer's opacity.
    fn land(&mut self, part: u32) {
        let Some(layer) = self.puppet.layer(part as usize) else { return };
        let mut mask = MaskMode::None;
        if !layer.masks.is_empty() {
            self.mask_set(layer.masks, &[]);
            mask = if layer.inverted { MaskMode::Inverted } else { MaskMode::Normal };
        }
        let [sx, sy, ox, oy] = self.to_ndc;
        let base = self.verts.len() as u32;
        let color = [1.0, 1.0, 1.0, layer.opacity];
        let corners = [
            ([-1.0, 1.0], [0.0, 0.0]),
            ([1.0, 1.0], [1.0, 0.0]),
            ([1.0, -1.0], [1.0, 1.0]),
            ([-1.0, -1.0], [0.0, 1.0]),
        ];
        for ([nx, ny], uv) in corners {
            self.verts.push(Vertex {
                pos: [(nx - ox) / sx, (ny - oy) / sy],
                uv,
                color,
                clip: NO_CLIP,
            });
        }
        let start = self.indices.len() as u32;
        self.indices.extend([0, 1, 2, 0, 2, 3].map(|i| base + i));
        self.cmds.push(Cmd::EndLayer {
            indices: start..self.indices.len() as u32,
            blend: blend_mode(layer.blend),
            mask,
        });
    }
}

/// Solo as the frame draws it: these meshes go last, in front and as they
/// are; every other one is multiplied by `tint` (rgb, and alpha for how
/// much of it shows), Cubism's inactive color.
pub struct Solo {
    pub meshes: std::collections::HashSet<usize>,
    pub tint: [f32; 4],
}

/// The frame as the renderer takes it: every drawable's triangles in
/// order, mask sets where they change, offscreen part layers opened and
/// landed around their meshes, and how canvas pixels map to the target.
/// Textures and pages are left for the caller to attach. Each mesh shows
/// only its own art, `art[mesh]` (none given: everything).
pub fn packet(
    puppet: &Puppet,
    w: u32,
    h: u32,
    to_ndc: [f32; 4],
    solo: Option<&Solo>,
    art: &[[f32; 4]],
) -> Packet {
    let mut f = Frame {
        puppet,
        art,
        c: puppet.model().canvas,
        to_ndc,
        verts: Vec::new(),
        indices: Vec::new(),
        cmds: Vec::new(),
        last_masks: None,
    };
    // the offscreen parts open now, outermost first
    let mut open: Vec<u32> = Vec::new();
    let tint = solo.map_or([1.0; 4], |s| s.tint);
    // soloed meshes wait to be drawn last, in front of everything
    let mut front = Vec::new();

    for d in puppet.drawables() {
        if d.opacity <= 0.0 || d.indices.is_empty() {
            continue;
        }
        if solo.is_some_and(|s| s.meshes.contains(&d.mesh)) {
            front.push(d);
            continue;
        }
        // land the layers this mesh is not in, open the ones it is
        let chain = puppet.layers_of(d.mesh);
        let shared = open.iter().zip(chain).take_while(|(a, b)| a == b).count();
        while open.len() > shared {
            let Some(part) = open.pop() else { break };
            f.land(part);
        }
        for &part in chain.get(shared..).unwrap_or_default() {
            f.cmds.push(Cmd::BeginLayer);
            open.push(part);
        }
        let mut mask = MaskMode::None;
        if !d.masks.is_empty() {
            f.mask_set(d.masks, d.clip_and);
            mask = if d.flags & flags::INVERTED_MASK != 0 {
                MaskMode::Inverted
            } else {
                MaskMode::Normal
            };
        }
        let color = [tint[0], tint[1], tint[2], d.opacity * tint[3]];
        let clip = art.get(d.mesh).copied().unwrap_or(NO_CLIP);
        let range = push_mesh(
            &mut f.verts,
            &mut f.indices,
            &f.c,
            d.positions,
            d.uvs,
            d.indices,
            color,
            clip,
        );
        f.cmds.push(Cmd::Draw {
            indices: range,
            texture: d.texture,
            blend: blend_mode(d.blend),
            mask,
        });
    }
    while let Some(part) = open.pop() {
        f.land(part);
    }
    for d in front {
        let mut mask = MaskMode::None;
        if !d.masks.is_empty() {
            f.mask_set(d.masks, d.clip_and);
            mask = if d.flags & flags::INVERTED_MASK != 0 {
                MaskMode::Inverted
            } else {
                MaskMode::Normal
            };
        }
        let color = [1.0, 1.0, 1.0, d.opacity];
        let clip = art.get(d.mesh).copied().unwrap_or(NO_CLIP);
        let range = push_mesh(
            &mut f.verts,
            &mut f.indices,
            &f.c,
            d.positions,
            d.uvs,
            d.indices,
            color,
            clip,
        );
        f.cmds.push(Cmd::Draw {
            indices: range,
            texture: d.texture,
            blend: blend_mode(d.blend),
            mask,
        });
    }

    Packet {
        vertices: f.verts,
        indices: f.indices,
        cmds: f.cmds,
        to_ndc,
        width: w,
        height: h,
        textures: None,
        uploaded: None,
        pages: None,
        pages_uploaded: None,
        drop_pages: None,
    }
}
