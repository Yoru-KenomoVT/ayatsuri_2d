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
    pub fn draw(
        &mut self,
        puppet: &Puppet,
        width: usize,
        art: &[[f32; 4]],
    ) -> Option<(usize, usize, Vec<u8>)> {
        let c = puppet.model().canvas;
        let (cw, ch) = (c.width.max(1.0), c.height.max(1.0));
        let w = (width.max(1) as u32).min(self.max_side);
        let h = ((ch * (w as f32 / cw)) as u32).clamp(1, self.max_side);
        // canvas pixels to clip space: the whole canvas fills the frame
        let to_ndc = [2.0 / cw, -2.0 / ch, -1.0, 1.0];
        let mut packet = crate::gpu_view::packet(puppet, w, h, to_ndc, None, art);
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

#[cfg(test)]
mod tests {
    use super::*;
    use ayatsuri_format::model::{ArtMesh, Binding, Canvas, Model, Offscreen, Part};

    /// A white quad from x0 to x1, full height, on an 8 x 8 canvas.
    fn quad(id: &str, x0: f32, x1: f32, part: Option<u32>, opacity: f32) -> ArtMesh {
        ArtMesh {
            id: id.into(),
            parent_part: part,
            parent_deformer: None,
            visible: true,
            enabled: true,
            texture: 0,
            flags: 0,
            blend: Default::default(),
            vertex_count: 4,
            uvs: vec![0.0, 0.0, 1.0, 0.0, 1.0, 1.0, 0.0, 1.0],
            indices: vec![0, 1, 2, 0, 2, 3],
            masks: vec![],
            binding: Binding::default(),
            opacities: vec![opacity],
            draw_orders: vec![500.0],
            positions: vec![x0, 0.0, x1, 0.0, x1, 8.0, x0, 8.0],
            blend_shapes: Vec::new(),
            clip_group: None,
            clip_inherit: false,
            multiply: Vec::new(),
            screen: Vec::new(),
        }
    }

    fn layered(masks: Vec<u32>) -> Model {
        let mut m = Model {
            canvas: Canvas { width: 8.0, height: 8.0, pixels_per_unit: 1.0, ..Canvas::default() },
            ..Model::default()
        };
        m.parts.push(Part {
            id: "P".into(),
            parent_part: None,
            visible: true,
            enabled: true,
            binding: Binding::default(),
            draw_orders: vec![500.0],
            offscreen: Some(Offscreen {
                masks,
                inverted: false,
                blend: Default::default(),
                opacities: vec![0.5],
            }),
        });
        m.art_meshes.push(quad("A", 0.0, 6.0, Some(0), 1.0));
        m.art_meshes.push(quad("B", 2.0, 8.0, Some(0), 1.0));
        m.art_meshes.push(quad("M", 0.0, 4.0, None, 0.0));
        m
    }

    fn alpha_at(px: &[u8], x: usize) -> u8 {
        px.get((4 * 8 + x) * 4 + 3).copied().unwrap_or(0)
    }

    fn frames() -> Option<Frames> {
        let white = TexData { width: 1, height: 1, rgba: vec![255; 4] };
        let f = Frames::new(vec![white]);
        if f.is_none() {
            eprintln!("no GPU adapter here: skipping");
        }
        f
    }

    #[test]
    fn an_offscreen_layer_fades_as_one_image() {
        let Some(mut f) = frames() else { return };
        let mut p = Puppet::new(layered(vec![]));
        p.update();
        let Some((_, _, px)) = f.draw(&p, 8, &[]) else { return };
        // one layer at 50%: the overlap is no darker than the rest
        for x in [1, 4, 7] {
            let a = alpha_at(&px, x);
            assert!((120..=136).contains(&a), "alpha {a} at x {x}");
        }
    }

    #[test]
    fn a_mesh_shows_only_its_own_art() {
        let Some(mut f) = frames() else { return };
        let mut m = Model {
            canvas: Canvas { width: 8.0, height: 8.0, pixels_per_unit: 1.0, ..Canvas::default() },
            ..Model::default()
        };
        m.art_meshes.push(quad("grown", 0.0, 8.0, None, 1.0));
        let mut p = Puppet::new(m);
        p.update();
        // its art is the left half of the page: the right half is a neighbour's
        let Some((_, _, px)) = f.draw(&p, 8, &[[0.0, 0.0, 0.5, 1.0]]) else { return };
        assert!(alpha_at(&px, 2) > 200, "its own art shows");
        assert!(alpha_at(&px, 6) < 8, "the neighbour's does not");
        let Some((_, _, px)) = f.draw(&p, 8, &[]) else { return };
        assert!(alpha_at(&px, 6) > 200, "with no boxes everything shows");
    }

    #[test]
    fn an_offscreen_layer_is_clipped_by_its_masks() {
        let Some(mut f) = frames() else { return };
        let mut p = Puppet::new(layered(vec![2]));
        p.update();
        let Some((_, _, px)) = f.draw(&p, 8, &[]) else { return };
        let (inside, outside) = (alpha_at(&px, 2), alpha_at(&px, 6));
        assert!((120..=136).contains(&inside), "inside the mask: {inside}");
        assert!(outside < 8, "outside the mask: {outside}");
    }

    #[test]
    fn an_inherited_clip_keeps_a_highlight_inside_the_eye_white() {
        let Some(mut f) = frames() else { return };
        let mut m = Model {
            canvas: Canvas { width: 8.0, height: 8.0, pixels_per_unit: 1.0, ..Canvas::default() },
            ..Model::default()
        };
        // eye white and iris drawn at 0% (masks only), the highlight shows
        m.art_meshes.push(quad("white", 0.0, 4.0, None, 0.0));
        let mut iris = quad("iris", 2.0, 8.0, None, 0.0);
        iris.masks = vec![0];
        m.art_meshes.push(iris);
        let mut highlight = quad("highlight", 0.0, 8.0, None, 1.0);
        highlight.masks = vec![1];
        m.art_meshes.push(highlight);
        let shown = |f: &mut Frames, m: &Model| -> Vec<bool> {
            let mut p = Puppet::new(m.clone());
            p.update();
            let px = f.draw(&p, 8, &[]).map(|(_, _, px)| px).unwrap_or_default();
            (0..8).map(|x| alpha_at(&px, x) > 128).collect()
        };
        // as every runtime clips today: inside the iris, eye white or not
        let plain = shown(&mut f, &m);
        assert_eq!(plain, vec![false, false, true, true, true, true, true, true]);
        m.art_meshes[2].clip_inherit = true;
        let inherited = shown(&mut f, &m);
        assert_eq!(inherited, vec![false, false, true, true, false, false, false, false]);
    }
}
