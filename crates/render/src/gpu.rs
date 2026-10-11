// The wgpu backend. Draws the frame into an MSAA offscreen target, with
// Cubism-style soft clipping masks: each mask set renders its texture alpha
// into an R8 buffer, and masked draws multiply by it (or its inverse).
// Everything blends in gamma space, matching Live2D and egui.

use std::ops::Range;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Vertex {
    pub pos: [f32; 2],
    pub uv: [f32; 2],
    pub color: [f32; 4],
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum BlendMode {
    Normal,
    Additive,
    Multiplicative,
}

/// How a draw consults the mask buffer.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum MaskMode {
    None,
    Normal,
    Inverted,
}

pub enum Cmd {
    /// Render these mask meshes into the mask buffer (one pass per set);
    /// each shape pairs an index range with the mask mesh's texture.
    MaskSet {
        shapes: Vec<(Range<u32>, u32)>,
    },
    Draw {
        indices: Range<u32>,
        texture: u32,
        blend: BlendMode,
        mask: MaskMode,
    },
}

#[derive(Clone)]
pub struct TexData {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

pub struct Packet {
    pub vertices: Vec<Vertex>,
    pub indices: Vec<u32>,
    pub cmds: Vec<Cmd>,
    // canvas px -> ndc: (sx, sy, ox, oy)
    pub to_ndc: [f32; 4],
    pub width: u32,
    pub height: u32,
    // present on the first frame after a model loads
    pub textures: Option<Vec<TexData>>,
}

const SHADER: &str = r#"
struct Uni { to_ndc: vec4<f32>, view: vec4<f32> }
@group(0) @binding(0) var<uniform> uni: Uni;

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
}

@vertex
fn vs_main(@location(0) pos: vec2<f32>, @location(1) uv: vec2<f32>, @location(2) color: vec4<f32>) -> VsOut {
    var o: VsOut;
    o.pos = vec4<f32>(pos.x * uni.to_ndc.x + uni.to_ndc.z, pos.y * uni.to_ndc.y + uni.to_ndc.w, 0.0, 1.0);
    o.uv = uv;
    o.color = color;
    return o;
}

@group(1) @binding(0) var tex: texture_2d<f32>;
@group(1) @binding(1) var smp: sampler;
@group(2) @binding(0) var mask_tex: texture_2d<f32>;
@group(2) @binding(1) var<uniform> mask_mode: vec4<u32>;

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let t = textureSample(tex, smp, in.uv);
    let muv = in.pos.xy / uni.view.xy;
    let m = textureSample(mask_tex, smp, muv).r;
    var f = 1.0;
    if mask_mode.x == 1u { f = m; }
    if mask_mode.x == 2u { f = 1.0 - m; }
    return vec4<f32>(t.rgb * in.color.rgb, t.a) * in.color.a * f;
}

@fragment
fn fs_mask(in: VsOut) -> @location(0) vec4<f32> {
    let a = textureSample(tex, smp, in.uv).a * in.color.a;
    return vec4<f32>(a, 0.0, 0.0, 1.0);
}
"#;

const BLIT: &str = r#"
struct VsOut { @builtin(position) pos: vec4<f32>, @location(0) uv: vec2<f32> }

@vertex
fn vs_main(@builtin(vertex_index) vi: u32) -> VsOut {
    var o: VsOut;
    let x = f32(i32(vi & 1u) * 4 - 1);
    let y = f32(i32(vi >> 1u) * 4 - 1);
    o.pos = vec4<f32>(x, y, 0.0, 1.0);
    o.uv = vec2<f32>((x + 1.0) * 0.5, (1.0 - y) * 0.5);
    return o;
}

@group(0) @binding(0) var tex: texture_2d<f32>;
@group(0) @binding(1) var smp: sampler;

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    return textureSample(tex, smp, in.uv);
}
"#;

// gamma-space formats on purpose: Live2D blends in gamma, and so does egui
const OFFSCREEN_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
const MASK_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R8Unorm;
const SAMPLES: u32 = 4;

fn samples_or_default(samples: u32) -> u32 {
    match samples {
        1 | 2 | 4 | 8 => samples,
        _ => SAMPLES,
    }
}

pub struct GpuRenderer {
    samples: u32,
    uni_buf: wgpu::Buffer,
    uni_bind: wgpu::BindGroup,
    tex_layout: wgpu::BindGroupLayout,
    mask_layout: wgpu::BindGroupLayout,
    blit_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    mask_pipeline: wgpu::RenderPipeline,
    draw_pipelines: Vec<(BlendMode, wgpu::RenderPipeline)>,
    blit_pipeline: wgpu::RenderPipeline,
    mode_bufs: [wgpu::Buffer; 3],
    // bound as group 2 during mask passes, where the real mask buffer is the
    // render target and must not also be sampled
    dummy_mode_bind: Option<wgpu::BindGroup>,
    textures: Vec<wgpu::BindGroup>,
    white: Option<wgpu::BindGroup>,
    offscreen: Option<Offscreen>,
    vbuf: Option<wgpu::Buffer>,
    ibuf: Option<wgpu::Buffer>,
    vcap: u64,
    icap: u64,
}

struct Offscreen {
    msaa: wgpu::TextureView,
    color: wgpu::TextureView,
    blit_bind: wgpu::BindGroup,
    mode_binds: [wgpu::BindGroup; 3],
    mask: wgpu::TextureView,
    w: u32,
    h: u32,
}

fn blend_state(mode: BlendMode) -> wgpu::BlendState {
    use wgpu::{BlendComponent, BlendFactor as F, BlendOperation::Add};
    match mode {
        BlendMode::Normal => wgpu::BlendState {
            color: BlendComponent {
                src_factor: F::One,
                dst_factor: F::OneMinusSrcAlpha,
                operation: Add,
            },
            alpha: BlendComponent {
                src_factor: F::One,
                dst_factor: F::OneMinusSrcAlpha,
                operation: Add,
            },
        },
        BlendMode::Additive => wgpu::BlendState {
            color: BlendComponent { src_factor: F::One, dst_factor: F::One, operation: Add },
            alpha: BlendComponent { src_factor: F::Zero, dst_factor: F::One, operation: Add },
        },
        BlendMode::Multiplicative => wgpu::BlendState {
            color: BlendComponent {
                src_factor: F::Dst,
                dst_factor: F::OneMinusSrcAlpha,
                operation: Add,
            },
            alpha: BlendComponent { src_factor: F::Zero, dst_factor: F::One, operation: Add },
        },
    }
}

impl GpuRenderer {
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        host_format: wgpu::TextureFormat,
    ) -> Self {
        Self::with_samples(device, queue, host_format, SAMPLES)
    }

    /// Same renderer with the model MSAA sample count chosen by settings;
    /// 1 skips multisampling entirely for weak GPUs.
    pub fn with_samples(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        host_format: wgpu::TextureFormat,
        samples: u32,
    ) -> Self {
        let samples = samples_or_default(samples);
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("ayatsuri-model"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });
        let blit_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("ayatsuri-blit"),
            source: wgpu::ShaderSource::Wgsl(BLIT.into()),
        });

        let uni_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: None,
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let tex_entry = |binding, ty| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty,
            count: None,
        };
        let float_tex = wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: true },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        };
        let tex_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: None,
            entries: &[
                tex_entry(0, float_tex),
                tex_entry(1, wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering)),
            ],
        });
        let mask_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: None,
            entries: &[
                tex_entry(0, float_tex),
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        let blit_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: None,
            entries: &[
                tex_entry(0, float_tex),
                tex_entry(1, wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering)),
            ],
        });

        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[Some(&uni_layout), Some(&tex_layout), Some(&mask_layout)],
            immediate_size: 0,
        });
        let vertex_layout = wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<Vertex>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x2, 2 => Float32x4],
        };

        let mask_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("ayatsuri-mask"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[Some(vertex_layout.clone())],
            },
            primitive: Default::default(),
            depth_stencil: None,
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_mask"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: MASK_FORMAT,
                    // keep the strongest coverage of the set
                    blend: Some(wgpu::BlendState {
                        color: wgpu::BlendComponent {
                            src_factor: wgpu::BlendFactor::One,
                            dst_factor: wgpu::BlendFactor::One,
                            operation: wgpu::BlendOperation::Max,
                        },
                        alpha: wgpu::BlendComponent {
                            src_factor: wgpu::BlendFactor::One,
                            dst_factor: wgpu::BlendFactor::One,
                            operation: wgpu::BlendOperation::Max,
                        },
                    }),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });

        let mut draw_pipelines = Vec::new();
        for blend in [BlendMode::Normal, BlendMode::Additive, BlendMode::Multiplicative] {
            let p = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("ayatsuri-draw"),
                layout: Some(&layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs_main"),
                    compilation_options: Default::default(),
                    buffers: &[Some(vertex_layout.clone())],
                },
                primitive: Default::default(),
                depth_stencil: None,
                multisample: wgpu::MultisampleState { count: samples, ..Default::default() },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some("fs_main"),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: OFFSCREEN_FORMAT,
                        blend: Some(blend_state(blend)),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            });
            draw_pipelines.push((blend, p));
        }

        let blit_pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[Some(&blit_layout)],
            immediate_size: 0,
        });
        let blit_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("ayatsuri-blit"),
            layout: Some(&blit_pl),
            vertex: wgpu::VertexState {
                module: &blit_shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            primitive: Default::default(),
            depth_stencil: None,
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &blit_shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: host_format,
                    blend: Some(wgpu::BlendState {
                        color: wgpu::BlendComponent {
                            src_factor: wgpu::BlendFactor::One,
                            dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                            operation: wgpu::BlendOperation::Add,
                        },
                        alpha: wgpu::BlendComponent {
                            src_factor: wgpu::BlendFactor::One,
                            dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                            operation: wgpu::BlendOperation::Add,
                        },
                    }),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            anisotropy_clamp: 4,
            ..Default::default()
        });
        let uni_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: 32,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let uni_bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &uni_layout,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: uni_buf.as_entire_binding() }],
        });
        let mode_bufs = [0u32, 1, 2].map(|mode| {
            let buf = device.create_buffer(&wgpu::BufferDescriptor {
                label: None,
                size: 16,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            queue.write_buffer(&buf, 0, bytemuck::cast_slice(&[mode, 0, 0, 0]));
            buf
        });

        let mut me = Self {
            samples,
            uni_buf,
            uni_bind,
            tex_layout,
            mask_layout,
            blit_layout,
            sampler,
            mask_pipeline,
            draw_pipelines,
            blit_pipeline,
            mode_bufs,
            dummy_mode_bind: None,
            textures: Vec::new(),
            white: None,
            offscreen: None,
            vbuf: None,
            ibuf: None,
            vcap: 0,
            icap: 0,
        };
        me.white = Some(me.upload_texture_bind(device, queue, 1, 1, &[255, 255, 255, 255]));
        let dummy = device
            .create_texture(&wgpu::TextureDescriptor {
                label: Some("ayatsuri-dummy-mask"),
                size: wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: MASK_FORMAT,
                usage: wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            })
            .create_view(&Default::default());
        me.dummy_mode_bind = Some(device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &me.mask_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&dummy),
                },
                wgpu::BindGroupEntry { binding: 1, resource: me.mode_bufs[0].as_entire_binding() },
            ],
        }));
        me
    }

    fn upload_texture_bind(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        w: u32,
        h: u32,
        rgba: &[u8],
    ) -> wgpu::BindGroup {
        // premultiplied, box-filter mip chain built once at load; without
        // mips a zoomed-out model crunches, without premultiply the edges
        // bleed white halos
        let mips = (32 - w.max(h).leading_zeros()).max(1);
        let tex = device.create_texture(&wgpu::TextureDescriptor {
            label: None,
            size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
            mip_level_count: mips,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let mut level_pixels = rgba.to_vec();
        let (px4, _) = level_pixels.as_chunks_mut::<4>();
        for px in px4 {
            let a = px[3] as u32;
            for v in px.iter_mut().take(3) {
                *v = ((*v as u32 * a) / 255) as u8;
            }
        }
        let (mut lw, mut lh) = (w, h);
        for level in 0..mips {
            queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &tex,
                    mip_level: level,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                &level_pixels,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(4 * lw),
                    rows_per_image: Some(lh),
                },
                wgpu::Extent3d { width: lw, height: lh, depth_or_array_layers: 1 },
            );
            if level + 1 == mips {
                break;
            }
            let (nw, nh) = ((lw / 2).max(1), (lh / 2).max(1));
            let mut next = vec![0u8; (nw * nh * 4) as usize];
            for y in 0..nh as usize {
                for x in 0..nw as usize {
                    let mut acc = [0u32; 4];
                    for (dy, dx) in [(0, 0), (0, 1), (1, 0), (1, 1)] {
                        let sy = (y * 2 + dy).min(lh as usize - 1);
                        let sx = (x * 2 + dx).min(lw as usize - 1);
                        let at = (sy * lw as usize + sx) * 4;
                        for (ch, a) in acc.iter_mut().enumerate() {
                            *a += level_pixels.get(at + ch).copied().unwrap_or(0) as u32;
                        }
                    }
                    let at = (y * nw as usize + x) * 4;
                    for (ch, a) in acc.iter().enumerate() {
                        if let Some(px) = next.get_mut(at + ch) {
                            *px = (a / 4) as u8;
                        }
                    }
                }
            }
            level_pixels = next;
            lw = nw;
            lh = nh;
        }
        let view = tex.create_view(&Default::default());
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &self.tex_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
            ],
        })
    }

    pub fn set_textures(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, images: &[TexData]) {
        self.textures = images
            .iter()
            .map(|t| self.upload_texture_bind(device, queue, t.width, t.height, &t.rgba))
            .collect();
    }

    fn ensure_offscreen(&mut self, device: &wgpu::Device, w: u32, h: u32) {
        if let Some(o) = &self.offscreen
            && o.w == w
            && o.h == h
        {
            return;
        }
        let size = wgpu::Extent3d { width: w.max(1), height: h.max(1), depth_or_array_layers: 1 };
        let make = |label: &str, format, samples, usage| {
            device
                .create_texture(&wgpu::TextureDescriptor {
                    label: Some(label),
                    size,
                    mip_level_count: 1,
                    sample_count: samples,
                    dimension: wgpu::TextureDimension::D2,
                    format,
                    usage,
                    view_formats: &[],
                })
                .create_view(&Default::default())
        };
        let att = wgpu::TextureUsages::RENDER_ATTACHMENT;
        let att_bind = att | wgpu::TextureUsages::TEXTURE_BINDING;
        let msaa = make("ayatsuri-msaa", OFFSCREEN_FORMAT, self.samples, att);
        let color = make("ayatsuri-offscreen", OFFSCREEN_FORMAT, 1, att_bind);
        let mask = make("ayatsuri-maskbuf", MASK_FORMAT, 1, att_bind);
        let blit_bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &self.blit_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&color),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
            ],
        });
        let mode_binds = [0usize, 1, 2].map(|i| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: None,
                layout: &self.mask_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&mask),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: match self.mode_bufs.get(i) {
                            Some(b) => b.as_entire_binding(),
                            None => self.uni_buf.as_entire_binding(),
                        },
                    },
                ],
            })
        });
        self.offscreen = Some(Offscreen { msaa, color, blit_bind, mode_binds, mask, w, h });
    }

    pub fn prepare(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        packet: &Packet,
    ) {
        if let Some(images) = &packet.textures {
            self.set_textures(device, queue, images);
        }
        self.ensure_offscreen(device, packet.width, packet.height);
        let uni: [f32; 8] = [
            packet.to_ndc[0],
            packet.to_ndc[1],
            packet.to_ndc[2],
            packet.to_ndc[3],
            packet.width as f32,
            packet.height as f32,
            0.0,
            0.0,
        ];
        queue.write_buffer(&self.uni_buf, 0, bytemuck::cast_slice(&uni));

        let vbytes: &[u8] = bytemuck::cast_slice(&packet.vertices);
        let ibytes: &[u8] = bytemuck::cast_slice(&packet.indices);
        if self.vbuf.is_none() || self.vcap < vbytes.len() as u64 {
            self.vcap = (vbytes.len() as u64).next_power_of_two().max(1024);
            self.vbuf = Some(device.create_buffer(&wgpu::BufferDescriptor {
                label: None,
                size: self.vcap,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }));
        }
        if self.ibuf.is_none() || self.icap < ibytes.len() as u64 {
            self.icap = (ibytes.len() as u64).next_power_of_two().max(1024);
            self.ibuf = Some(device.create_buffer(&wgpu::BufferDescriptor {
                label: None,
                size: self.icap,
                usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }));
        }
        let (Some(vbuf), Some(ibuf)) = (&self.vbuf, &self.ibuf) else { return };
        if !vbytes.is_empty() {
            queue.write_buffer(vbuf, 0, vbytes);
        }
        if !ibytes.is_empty() {
            queue.write_buffer(ibuf, 0, ibytes);
        }
        let Some(o) = &self.offscreen else { return };

        // the frame splits into segments at every mask-set switch: a mask
        // pass fills the mask buffer, then a main segment draws with it
        let mut first_main = true;
        let mut i = 0usize;
        while i < packet.cmds.len() {
            if let Some(Cmd::MaskSet { shapes }) = packet.cmds.get(i) {
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("ayatsuri-maskset"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &o.mask,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                });
                pass.set_pipeline(&self.mask_pipeline);
                pass.set_bind_group(0, &self.uni_bind, &[]);
                let Some(dummy) = &self.dummy_mode_bind else { continue };
                pass.set_bind_group(2, dummy, &[]);
                pass.set_vertex_buffer(0, vbuf.slice(..));
                pass.set_index_buffer(ibuf.slice(..), wgpu::IndexFormat::Uint32);
                for (range, texture) in shapes {
                    let Some(bind) = self.textures.get(*texture as usize).or(self.white.as_ref())
                    else {
                        continue;
                    };
                    pass.set_bind_group(1, bind, &[]);
                    pass.draw_indexed(range.clone(), 0, 0..1);
                }
                drop(pass);
                i += 1;
                continue;
            }

            let run_start = i;
            while i < packet.cmds.len() && !matches!(packet.cmds.get(i), Some(Cmd::MaskSet { .. }))
            {
                i += 1;
            }
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("ayatsuri-model"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: if self.samples > 1 { &o.msaa } else { &o.color },
                    depth_slice: None,
                    resolve_target: (self.samples > 1).then_some(&o.color),
                    ops: wgpu::Operations {
                        load: if first_main {
                            wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT)
                        } else {
                            wgpu::LoadOp::Load
                        },
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            first_main = false;
            pass.set_bind_group(0, &self.uni_bind, &[]);
            pass.set_vertex_buffer(0, vbuf.slice(..));
            pass.set_index_buffer(ibuf.slice(..), wgpu::IndexFormat::Uint32);
            for cmd in packet.cmds.get(run_start..i).unwrap_or(&[]) {
                let Cmd::Draw { indices, texture, blend, mask } = cmd else { continue };
                let Some(bind) = self.textures.get(*texture as usize).or(self.white.as_ref())
                else {
                    continue;
                };
                if let Some((_, p)) = self.draw_pipelines.iter().find(|(b, _)| b == blend) {
                    pass.set_pipeline(p);
                    pass.set_bind_group(1, bind, &[]);
                    let mode = match mask {
                        MaskMode::None => 0,
                        MaskMode::Normal => 1,
                        MaskMode::Inverted => 2,
                    };
                    let Some(mode_bind) = o.mode_binds.get(mode) else { continue };
                    pass.set_bind_group(2, mode_bind, &[]);
                    pass.draw_indexed(indices.clone(), 0, 0..1);
                }
            }
        }
        if first_main {
            let _ = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("ayatsuri-clear"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: if self.samples > 1 { &o.msaa } else { &o.color },
                    depth_slice: None,
                    resolve_target: (self.samples > 1).then_some(&o.color),
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
        }
    }

    pub fn paint(&self, pass: &mut wgpu::RenderPass<'static>) {
        let Some(o) = &self.offscreen else { return };
        pass.set_pipeline(&self.blit_pipeline);
        pass.set_bind_group(0, &o.blit_bind, &[]);
        pass.draw(0..3, 0..1);
    }
}
