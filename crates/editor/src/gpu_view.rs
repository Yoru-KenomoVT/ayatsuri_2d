// Bridges the puppet to the wgpu renderer through an egui paint callback.
// Builds the frame packet on the UI thread: vertices in canvas pixels, and
// the mask/draw command list with stencil references assigned.

use std::sync::Mutex;

use ayatsuri_core::Puppet;
use ayatsuri_format::model::{Canvas, flags};
use ayatsuri_render::{BlendMode, Cmd, GpuRenderer, MaskMode, Packet, TexData, Vertex};

pub struct ModelPaint {
    packet: Mutex<Option<Packet>>,
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
    opacity: f32,
) -> std::ops::Range<u32> {
    let base = verts.len() as u32;
    let start = indices.len() as u32;
    let (pos_pairs, _) = positions.as_chunks::<2>();
    let (uv_pairs, _) = uvs.as_chunks::<2>();
    let color = [1.0, 1.0, 1.0, opacity];
    for (xy, uv) in pos_pairs.iter().zip(uv_pairs) {
        verts.push(Vertex {
            pos: [c.origin_x + xy[0] * c.pixels_per_unit, c.origin_y + xy[1] * c.pixels_per_unit],
            uv: [uv[0], uv[1]],
            color,
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
) -> egui::PaintCallback {
    let w = ((rect.width() * pixels_per_point) as u32).max(1);
    let h = ((rect.height() * pixels_per_point) as u32).max(1);
    let sx = scale * pixels_per_point / w as f32 * 2.0;
    let sy = -(scale * pixels_per_point / h as f32 * 2.0);
    let ox = (screen_origin.x - rect.min.x) * pixels_per_point / w as f32 * 2.0 - 1.0;
    let oy = 1.0 - (screen_origin.y - rect.min.y) * pixels_per_point / h as f32 * 2.0;
    let mut packet = packet(puppet, w, h, [sx, sy, ox, oy]);
    packet.textures = textures;
    packet.uploaded = Some(uploaded);
    packet.pages = pages;
    packet.pages_uploaded = Some(pages_uploaded);
    packet.drop_pages = drop_pages;
    egui_wgpu::Callback::new_paint_callback(rect, ModelPaint { packet: Mutex::new(Some(packet)) })
}

/// The frame as the renderer takes it: every drawable's triangles in
/// order, mask sets where they change, and how canvas pixels map to the
/// target. Textures and pages are left for the caller to attach.
pub fn packet(puppet: &Puppet, w: u32, h: u32, to_ndc: [f32; 4]) -> Packet {
    let c = puppet.model().canvas;
    let mut verts = Vec::new();
    let mut indices = Vec::new();
    let mut cmds = Vec::new();
    let mut last_masks: Option<Vec<u32>> = None;

    for d in puppet.drawables() {
        if d.opacity <= 0.0 || d.indices.is_empty() {
            continue;
        }
        let blend = if d.flags & flags::BLEND_ADDITIVE != 0 {
            BlendMode::Additive
        } else if d.flags & flags::BLEND_MULTIPLICATIVE != 0 {
            BlendMode::Multiplicative
        } else {
            BlendMode::Normal
        };
        let mut mask = MaskMode::None;
        if !d.masks.is_empty() {
            let same = last_masks.as_deref() == Some(d.masks);
            if !same {
                last_masks = Some(d.masks.to_vec());
                let mut shapes = Vec::new();
                for &src in d.masks {
                    if let Some(g) = puppet.mesh_geometry(src as usize) {
                        let range = push_mesh(
                            &mut verts,
                            &mut indices,
                            &c,
                            g.positions,
                            g.uvs,
                            g.indices,
                            1.0,
                        );
                        shapes.push((range, g.texture));
                    }
                }
                cmds.push(Cmd::MaskSet { shapes });
            }
            mask = if d.flags & flags::INVERTED_MASK != 0 {
                MaskMode::Inverted
            } else {
                MaskMode::Normal
            };
        }
        let range =
            push_mesh(&mut verts, &mut indices, &c, d.positions, d.uvs, d.indices, d.opacity);
        cmds.push(Cmd::Draw { indices: range, texture: d.texture, blend, mask });
    }

    Packet {
        vertices: verts,
        indices,
        cmds,
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
