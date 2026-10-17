// Frames for an export drawn by the GPU: a device of its own, off any
// window, the viewport's renderer drawing to an offscreen target with
// MSAA, and the pixels read back. The CPU only copies and encodes, so a
// render no longer takes every core.

use ayatsuri_core::Puppet;
use ayatsuri_render::{GpuRenderer, TexData, wgpu};

pub struct Frames {
    device: wgpu::Device,
    queue: wgpu::Queue,
    renderer: GpuRenderer,
    /// Handed to the renderer with the first frame, then its.
    textures: Option<std::sync::Arc<Vec<TexData>>>,
    max_side: u32,
}

impl Frames {
    /// A device for drawing off screen, or None when no adapter will.
    pub fn new(textures: Vec<TexData>) -> Option<Self> {
        let instance = wgpu::Instance::default();
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            ..Default::default()
        }))
        .ok()?;
        let limits = adapter.limits();
        let max_side = limits.max_texture_dimension_2d;
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("ayatsuri-export"),
            required_limits: limits,
            ..Default::default()
        }))
        .ok()?;
        let samples = ayatsuri_render::supported_samples(&adapter, &device, 4);
        let renderer =
            GpuRenderer::with_samples(&device, &queue, wgpu::TextureFormat::Rgba8Unorm, samples);
        log::info!("render: frames drawn on {} with {samples}x MSAA", adapter.get_info().name);
        Some(Self {
            device,
            queue,
            renderer,
            textures: Some(std::sync::Arc::new(textures)),
            max_side,
        })
    }

    /// The puppet as it stands, `width` pixels wide, as straight RGBA.
    pub fn draw(&mut self, puppet: &Puppet, width: usize) -> Option<(usize, usize, Vec<u8>)> {
        let c = puppet.model().canvas;
        let (cw, ch) = (c.width.max(1.0), c.height.max(1.0));
        let w = (width.max(1) as u32).min(self.max_side);
        let h = ((ch * (w as f32 / cw)) as u32).clamp(1, self.max_side);
        // canvas pixels to clip space: the whole canvas fills the frame
        let to_ndc = [2.0 / cw, -2.0 / ch, -1.0, 1.0];
        let mut packet = crate::gpu_view::packet(puppet, w, h, to_ndc);
        packet.textures = self.textures.take();
        let mut enc = self.device.create_command_encoder(&Default::default());
        self.renderer.prepare(&self.device, &self.queue, &mut enc, &packet);
        self.queue.submit([enc.finish()]);
        let mut px = self.renderer.read_frame(&self.device, &self.queue)?;
        unpremultiply(&mut px);
        Some((w as usize, h as usize, px))
    }
}

/// The renderer composes with colour already multiplied by alpha; files
/// want it straight.
fn unpremultiply(px: &mut [u8]) {
    for p in px.as_chunks_mut::<4>().0 {
        let a = u32::from(p[3]);
        if a == 0 || a == 255 {
            continue;
        }
        for c in p.iter_mut().take(3) {
            *c = ((u32::from(*c) * 255 + a / 2) / a).min(255) as u8;
        }
    }
}
