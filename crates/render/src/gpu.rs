// The wgpu backend. Draws the frame into an MSAA offscreen target, with
// Cubism-style soft clipping masks: each mask set renders its texture alpha
// into an R8 buffer, and masked draws multiply by it (or its inverse).
// Everything blends in gamma space, matching Live2D and egui. Normal and
// the two compatible modes blend in hardware; moc3 5.3's other modes need
// what is underneath, so before such a mesh the frame so far is copied to
// a backdrop texture and the mesh's shader mixes against it.

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
    /// moc3 5.3's colour and alpha blend codes, drawn against a copy of
    /// the frame so far.
    Advanced {
        color: u8,
        alpha: u8,
    },
}

const COLOR_MODES: u32 = 18;
const ALPHA_MODES: u32 = 5;
/// One mode pair per uniform slot, at the alignment dynamic offsets need.
const MODE_STRIDE: u64 = 256;

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
    /// Draw into a fresh, clear layer until the matching `EndLayer`
    /// (Cubism 5.3's offscreen parts). Layers nest.
    BeginLayer,
    /// Land the layer on what is beneath it: `indices` is a quad over the
    /// whole target whose uvs cover the layer, its vertex alpha the
    /// layer's opacity; it blends and masks like any draw.
    EndLayer {
        indices: Range<u32>,
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
    // present until the renderer has taken them: egui skips a callback
    // whose area isn't drawn that frame, so a one-shot hand-off can be lost
    // and the model would never show. The flag says they arrived.
    pub textures: Option<std::sync::Arc<Vec<TexData>>>,
    pub uploaded: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
    pub pages: Option<std::sync::Arc<Vec<(usize, TexData)>>>,
    pub pages_uploaded: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
    /// Pages whose full-size texture can go; their proxy stays and draws.
    pub drop_pages: Option<Vec<usize>>,
}

/// One texture page on the GPU: a small proxy that is always there once
/// the model is loaded, and the full page while it is wanted. Drawing
/// takes the full page when it is resident, the proxy otherwise, so a
/// page can be dropped and brought back without a frame going blank.
#[derive(Default)]
struct Slot {
    full: Option<wgpu::BindGroup>,
    proxy: Option<wgpu::BindGroup>,
}

impl Slot {
    fn bind(&self) -> Option<&wgpu::BindGroup> {
        self.full.as_ref().or(self.proxy.as_ref())
    }
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

@group(3) @binding(0) var backdrop: texture_2d<f32>;
@group(3) @binding(1) var<uniform> blend_mode: vec4<u32>;

fn screen1(s: f32, b: f32) -> f32 { return s + b - s * b; }
fn hard1(s: f32, b: f32) -> f32 {
    if s <= 0.5 { return b * 2.0 * s; }
    return screen1(2.0 * s - 1.0, b);
}
fn soft1(s: f32, b: f32) -> f32 {
    if s <= 0.5 { return b - (1.0 - 2.0 * s) * b * (1.0 - b); }
    var d = sqrt(b);
    if b <= 0.25 { d = ((16.0 * b - 12.0) * b + 4.0) * b; }
    return b + (2.0 * s - 1.0) * (d - b);
}
fn burn1(s: f32, b: f32) -> f32 {
    if b >= 1.0 { return 1.0; }
    if s <= 0.0 { return 0.0; }
    return 1.0 - min(1.0, (1.0 - b) / s);
}
fn dodge1(s: f32, b: f32) -> f32 {
    if b <= 0.0 { return 0.0; }
    if s >= 1.0 { return 1.0; }
    return min(1.0, b / (1.0 - s));
}
fn lum(c: vec3<f32>) -> f32 { return dot(c, vec3<f32>(0.3, 0.59, 0.11)); }
fn clip_color(c: vec3<f32>) -> vec3<f32> {
    let l = lum(c);
    let n = min(c.r, min(c.g, c.b));
    let x = max(c.r, max(c.g, c.b));
    var o = c;
    if n < 0.0 && l - n > 1e-6 { o = l + (o - l) * l / (l - n); }
    if x > 1.0 && x - l > 1e-6 { o = l + (o - l) * (1.0 - l) / (x - l); }
    return o;
}
fn set_lum(c: vec3<f32>, l: f32) -> vec3<f32> { return clip_color(c + (l - lum(c))); }
fn sat(c: vec3<f32>) -> f32 { return max(c.r, max(c.g, c.b)) - min(c.r, min(c.g, c.b)); }
fn set_sat(c: vec3<f32>, s: f32) -> vec3<f32> {
    let x = max(c.r, max(c.g, c.b));
    let n = min(c.r, min(c.g, c.b));
    if x - n <= 1e-6 { return vec3<f32>(0.0); }
    return (c - n) * s / (x - n);
}

// the same maths as ayatsuri_core::compose::mix
fn mix_color(mode: u32, s: vec3<f32>, b: vec3<f32>) -> vec3<f32> {
    switch mode {
        case 3u: { return min(s + b, vec3<f32>(1.0)); }
        case 4u: { return s + b; }
        case 5u: { return min(s, b); }
        case 6u: { return s * b; }
        case 7u: { return vec3<f32>(burn1(s.r, b.r), burn1(s.g, b.g), burn1(s.b, b.b)); }
        case 8u: { return max(s + b - 1.0, vec3<f32>(0.0)); }
        case 9u: { return max(s, b); }
        case 10u: { return s + b - s * b; }
        case 11u: { return vec3<f32>(dodge1(s.r, b.r), dodge1(s.g, b.g), dodge1(s.b, b.b)); }
        case 12u: { return vec3<f32>(hard1(b.r, s.r), hard1(b.g, s.g), hard1(b.b, s.b)); }
        case 13u: { return vec3<f32>(soft1(s.r, b.r), soft1(s.g, b.g), soft1(s.b, b.b)); }
        case 14u: { return vec3<f32>(hard1(s.r, b.r), hard1(s.g, b.g), hard1(s.b, b.b)); }
        case 15u: { return clamp(b + 2.0 * s - 1.0, vec3<f32>(0.0), vec3<f32>(1.0)); }
        case 16u: { return set_lum(set_sat(s, sat(b)), lum(b)); }
        case 17u: { return set_lum(s, lum(b)); }
        default: { return s; }
    }
}

fn unpremul(c: vec4<f32>) -> vec3<f32> {
    if c.a <= 1e-6 { return vec3<f32>(0.0); }
    return clamp(c.rgb / c.a, vec3<f32>(0.0), vec3<f32>(1.0));
}

@fragment
fn fs_advanced(in: VsOut) -> @location(0) vec4<f32> {
    let t = textureSample(tex, smp, in.uv);
    let muv = in.pos.xy / uni.view.xy;
    let m = textureSample(mask_tex, smp, muv).r;
    var f = 1.0;
    if mask_mode.x == 1u { f = m; }
    if mask_mode.x == 2u { f = 1.0 - m; }
    let src = vec4<f32>(t.rgb * in.color.rgb, t.a) * in.color.a * f;
    let dst = textureLoad(backdrop, vec2<i32>(in.pos.xy), 0);
    let sa = src.a;
    let da = dst.a;
    let cs = unpremul(src);
    let cb = unpremul(dst);
    let mixed = mix_color(blend_mode.x, cs, cb);
    var p = vec3<f32>(sa * da, sa * (1.0 - da), da * (1.0 - sa));
    if blend_mode.y == 3u { p = vec3<f32>(min(sa, da), max(sa - da, 0.0), max(da - sa, 0.0)); }
    if blend_mode.y == 4u { p = vec3<f32>(max(sa + da - 1.0, 0.0), min(sa, 1.0 - da), min(da, 1.0 - sa)); }
    var keep = vec3<f32>(1.0, 1.0, 1.0);
    if blend_mode.y == 1u { keep = vec3<f32>(1.0, 0.0, 1.0); }
    if blend_mode.y == 2u { keep = vec3<f32>(0.0, 1.0, 0.0); }
    let w = p * keep;
    let rgb = clamp(w.x * mixed + w.y * cs + w.z * cb, vec3<f32>(0.0), vec3<f32>(1.0));
    return vec4<f32>(rgb, clamp(w.x + w.y + w.z, 0.0, 1.0));
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

/// The sample count to really use: the highest at or under `wanted` that
/// this device can multisample the model target with. WebGPU only
/// guarantees 1x and 4x; 2x and 8x also need the adapter to offer them and
/// the device to have TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES on. Asking
/// for one it lacks is a fatal validation error, so this never does.
pub fn supported_samples(adapter: &wgpu::Adapter, device: &wgpu::Device, wanted: u32) -> u32 {
    let extended =
        device.features().contains(wgpu::Features::TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES);
    let flags = adapter.get_texture_format_features(OFFSCREEN_FORMAT).flags;
    [8u32, 4, 2, 1]
        .into_iter()
        .filter(|&n| n <= samples_or_default(wanted))
        .find(|&n| n == 1 || n == 4 || (extended && flags.sample_count_supported(n)))
        .unwrap_or(1)
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
    advanced_pipeline: wgpu::RenderPipeline,
    backdrop_layout: wgpu::BindGroupLayout,
    /// Every colour and alpha mode pair, one per `MODE_STRIDE` slot.
    advanced_modes: wgpu::Buffer,
    blit_pipeline: wgpu::RenderPipeline,
    mode_bufs: [wgpu::Buffer; 3],
    // bound as group 2 during mask passes, where the real mask buffer is the
    // render target and must not also be sampled
    dummy_mode_bind: Option<wgpu::BindGroup>,
    textures: Vec<Slot>,
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
    /// The resolved frame itself, for reading back.
    color_tex: wgpu::Texture,
    blit_bind: wgpu::BindGroup,
    mode_binds: [wgpu::BindGroup; 3],
    mask: wgpu::TextureView,
    /// A copy of the frame so far, for the advanced blend modes to read.
    backdrop_tex: wgpu::Texture,
    backdrop_bind: wgpu::BindGroup,
    /// Offscreen part layers, one per nesting depth, made as needed.
    layers: Vec<LayerTarget>,
    w: u32,
    h: u32,
}

struct LayerTarget {
    msaa: wgpu::TextureView,
    color: wgpu::TextureView,
    color_tex: wgpu::Texture,
    /// The resolved layer as a mesh texture, for landing it.
    bind: wgpu::BindGroup,
}

fn blend_state(mode: BlendMode) -> wgpu::BlendState {
    use wgpu::{BlendComponent, BlendFactor as F, BlendOperation::Add};
    match mode {
        BlendMode::Normal | BlendMode::Advanced { .. } => wgpu::BlendState {
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

        let backdrop_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: None,
            entries: &[
                tex_entry(
                    0,
                    wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                ),
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: true,
                        min_binding_size: wgpu::BufferSize::new(16),
                    },
                    count: None,
                },
            ],
        });
        let advanced_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[
                Some(&uni_layout),
                Some(&tex_layout),
                Some(&mask_layout),
                Some(&backdrop_layout),
            ],
            immediate_size: 0,
        });
        // the shader does all the blending; the target just takes it
        let advanced_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("ayatsuri-draw-advanced"),
            layout: Some(&advanced_layout),
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
                entry_point: Some("fs_advanced"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: OFFSCREEN_FORMAT,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        let advanced_modes = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ayatsuri-blend-modes"),
            size: MODE_STRIDE * u64::from(COLOR_MODES * ALPHA_MODES),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        for c in 0..COLOR_MODES {
            for a in 0..ALPHA_MODES {
                let slot = u64::from(c * ALPHA_MODES + a) * MODE_STRIDE;
                queue.write_buffer(&advanced_modes, slot, bytemuck::cast_slice(&[c, a, 0u32, 0]));
            }
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
            advanced_pipeline,
            backdrop_layout,
            advanced_modes,
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

    /// The proxies for every page of a freshly loaded model; full pages
    /// arrive one at a time through `Packet::pages` as they are wanted.
    pub fn set_textures(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, images: &[TexData]) {
        self.textures = images
            .iter()
            .map(|t| Slot {
                full: None,
                proxy: Some(self.upload_texture_bind(device, queue, t.width, t.height, &t.rgba)),
            })
            .collect();
    }

    /// Let these pages' full-size textures go; their proxies stay and draw.
    /// Callable between frames, so a minimised window that paints nothing
    /// still gives its video memory back.
    pub fn drop_full_pages(&mut self, pages: &[usize]) {
        for &i in pages {
            if let Some(slot) = self.textures.get_mut(i) {
                slot.full = None;
            }
        }
    }

    /// Bytes of texture memory the full pages hold right now, mips included.
    pub fn resident_full(&self) -> usize {
        self.textures.iter().filter(|s| s.full.is_some()).count()
    }

    fn ensure_offscreen(&mut self, device: &wgpu::Device, w: u32, h: u32) {
        if let Some(o) = &self.offscreen
            && o.w == w
            && o.h == h
        {
            return;
        }
        let size = wgpu::Extent3d { width: w.max(1), height: h.max(1), depth_or_array_layers: 1 };
        let make_tex = |label: &str, format, samples, usage| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size,
                mip_level_count: 1,
                sample_count: samples,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage,
                view_formats: &[],
            })
        };
        let make = |label: &str, format, samples, usage| {
            make_tex(label, format, samples, usage).create_view(&Default::default())
        };
        let att = wgpu::TextureUsages::RENDER_ATTACHMENT;
        let att_bind = att | wgpu::TextureUsages::TEXTURE_BINDING;
        let msaa = make("ayatsuri-msaa", OFFSCREEN_FORMAT, self.samples, att);
        let color_tex = make_tex(
            "ayatsuri-offscreen",
            OFFSCREEN_FORMAT,
            1,
            att_bind | wgpu::TextureUsages::COPY_SRC,
        );
        let color = color_tex.create_view(&Default::default());
        let mask = make("ayatsuri-maskbuf", MASK_FORMAT, 1, att_bind);
        let backdrop_tex = make_tex(
            "ayatsuri-backdrop",
            OFFSCREEN_FORMAT,
            1,
            wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        );
        let backdrop_view = backdrop_tex.create_view(&Default::default());
        let backdrop_bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &self.backdrop_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&backdrop_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                        buffer: &self.advanced_modes,
                        offset: 0,
                        size: wgpu::BufferSize::new(16),
                    }),
                },
            ],
        });
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
        self.offscreen = Some(Offscreen {
            msaa,
            color,
            color_tex,
            blit_bind,
            mode_binds,
            mask,
            backdrop_tex,
            backdrop_bind,
            layers: Vec::new(),
            w,
            h,
        });
    }

    /// Enough layer targets for `depth` nested offscreen parts, at the
    /// frame's size (a resize starts the list afresh).
    fn ensure_layers(&mut self, device: &wgpu::Device, depth: usize) {
        let Some(o) = &mut self.offscreen else { return };
        let size =
            wgpu::Extent3d { width: o.w.max(1), height: o.h.max(1), depth_or_array_layers: 1 };
        while o.layers.len() < depth {
            let make_tex = |label: &str, samples, usage| {
                device.create_texture(&wgpu::TextureDescriptor {
                    label: Some(label),
                    size,
                    mip_level_count: 1,
                    sample_count: samples,
                    dimension: wgpu::TextureDimension::D2,
                    format: OFFSCREEN_FORMAT,
                    usage,
                    view_formats: &[],
                })
            };
            let att = wgpu::TextureUsages::RENDER_ATTACHMENT;
            let msaa =
                make_tex("ayatsuri-layer-msaa", self.samples, att).create_view(&Default::default());
            let color_tex = make_tex(
                "ayatsuri-layer",
                1,
                att | wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_SRC,
            );
            let color = color_tex.create_view(&Default::default());
            let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: None,
                layout: &self.tex_layout,
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
            o.layers.push(LayerTarget { msaa, color, color_tex, bind });
        }
    }

    /// The last frame drawn, read back as premultiplied RGBA rows, for an
    /// export that draws on the GPU. Waits for the GPU, so it is for a
    /// worker, not the UI.
    pub fn read_frame(&self, device: &wgpu::Device, queue: &wgpu::Queue) -> Option<Vec<u8>> {
        let o = self.offscreen.as_ref()?;
        let align = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let row = (o.w * 4).div_ceil(align) * align;
        let buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ayatsuri-readback"),
            size: u64::from(row) * u64::from(o.h),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut enc = device.create_command_encoder(&Default::default());
        enc.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &o.color_tex,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &buf,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(row),
                    rows_per_image: Some(o.h),
                },
            },
            wgpu::Extent3d { width: o.w, height: o.h, depth_or_array_layers: 1 },
        );
        queue.submit([enc.finish()]);
        let slice = buf.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |r| {
            let _ = tx.send(r);
        });
        device.poll(wgpu::PollType::wait_indefinitely()).ok()?;
        rx.recv().ok()?.ok()?;
        let data = slice.get_mapped_range().ok()?;
        let mut out = Vec::with_capacity((o.w * 4 * o.h) as usize);
        for y in 0..o.h as usize {
            let from = y * row as usize;
            out.extend_from_slice(data.get(from..from + o.w as usize * 4)?);
        }
        drop(data);
        buf.unmap();
        Some(out)
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
            if let Some(flag) = &packet.uploaded {
                flag.store(true, std::sync::atomic::Ordering::Release);
            }
        }
        if let Some(drop) = &packet.drop_pages {
            self.drop_full_pages(drop);
        }
        if let Some(pages) = &packet.pages {
            for (i, t) in pages.iter() {
                let bind = self.upload_texture_bind(device, queue, t.width, t.height, &t.rgba);
                let len = self.textures.len();
                if let Some(slot) = self.textures.get_mut(*i) {
                    slot.full = Some(bind);
                } else if *i == len {
                    self.textures.push(Slot { full: Some(bind), proxy: None });
                }
            }
            if let Some(flag) = &packet.pages_uploaded {
                flag.store(true, std::sync::atomic::Ordering::Release);
            }
        }
        self.ensure_offscreen(device, packet.width, packet.height);
        let mut deepest = 0usize;
        let mut d = 0usize;
        for cmd in &packet.cmds {
            match cmd {
                Cmd::BeginLayer => {
                    d += 1;
                    deepest = deepest.max(d);
                }
                Cmd::EndLayer { .. } => d = d.saturating_sub(1),
                _ => {}
            }
        }
        self.ensure_layers(device, deepest);
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

        // the target at each depth: 0 is the frame, then each open layer
        let samples = self.samples;
        let target =
            |depth: usize| -> Option<(&wgpu::TextureView, &wgpu::TextureView, &wgpu::Texture)> {
                match depth.checked_sub(1) {
                    None => Some((&o.msaa, &o.color, &o.color_tex)),
                    Some(l) => o.layers.get(l).map(|l| (&l.msaa, &l.color, &l.color_tex)),
                }
            };
        let open = |encoder: &mut wgpu::CommandEncoder, depth: usize, clear: bool| {
            let (msaa, color, _) = target(depth)?;
            let mut pass = encoder
                .begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("ayatsuri-model"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: if samples > 1 { msaa } else { color },
                        depth_slice: None,
                        resolve_target: (samples > 1).then_some(color),
                        ops: wgpu::Operations {
                            load: if clear {
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
                })
                .forget_lifetime();
            pass.set_bind_group(0, &self.uni_bind, &[]);
            pass.set_vertex_buffer(0, vbuf.slice(..));
            pass.set_index_buffer(ibuf.slice(..), wgpu::IndexFormat::Uint32);
            Some(pass)
        };

        // passes break at every mask-set switch (a mask pass fills the mask
        // buffer), at every layer edge and before every advanced blend
        let mut depth = 0usize;
        let mut frame_cleared = false;
        let mut pass: Option<wgpu::RenderPass<'static>> = None;
        for cmd in &packet.cmds {
            let (indices, bind, blend, mask) = match cmd {
                Cmd::MaskSet { shapes } => {
                    drop(pass.take());
                    let mut mp = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
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
                    mp.set_pipeline(&self.mask_pipeline);
                    mp.set_bind_group(0, &self.uni_bind, &[]);
                    let Some(dummy) = &self.dummy_mode_bind else { continue };
                    mp.set_bind_group(2, dummy, &[]);
                    mp.set_vertex_buffer(0, vbuf.slice(..));
                    mp.set_index_buffer(ibuf.slice(..), wgpu::IndexFormat::Uint32);
                    for (range, texture) in shapes {
                        let Some(bind) = self
                            .textures
                            .get(*texture as usize)
                            .and_then(Slot::bind)
                            .or(self.white.as_ref())
                        else {
                            continue;
                        };
                        mp.set_bind_group(1, bind, &[]);
                        mp.draw_indexed(range.clone(), 0, 0..1);
                    }
                    continue;
                }
                Cmd::BeginLayer => {
                    drop(pass.take());
                    if target(depth + 1).is_none() {
                        continue;
                    }
                    depth += 1;
                    pass = open(encoder, depth, true);
                    continue;
                }
                Cmd::EndLayer { indices, blend, mask } => {
                    drop(pass.take());
                    let Some(layer) = depth.checked_sub(1).and_then(|l| o.layers.get(l)) else {
                        continue;
                    };
                    depth -= 1;
                    (indices, &layer.bind, *blend, *mask)
                }
                Cmd::Draw { indices, texture, blend, mask } => {
                    let Some(bind) = self
                        .textures
                        .get(*texture as usize)
                        .and_then(Slot::bind)
                        .or(self.white.as_ref())
                    else {
                        continue;
                    };
                    (indices, bind, *blend, *mask)
                }
            };
            if pass.is_none() {
                pass = open(encoder, depth, depth == 0 && !frame_cleared);
            }
            if depth == 0 {
                frame_cleared = true;
            }
            let mode = match mask {
                MaskMode::None => 0,
                MaskMode::Normal => 1,
                MaskMode::Inverted => 2,
            };
            let Some(mode_bind) = o.mode_binds.get(mode) else { continue };
            if let BlendMode::Advanced { color, alpha } = blend {
                // finish what is drawn so far (resolving it), copy it as the
                // backdrop, and carry on in a fresh pass
                drop(pass.take());
                let Some((_, _, tex)) = target(depth) else { continue };
                encoder.copy_texture_to_texture(
                    tex.as_image_copy(),
                    o.backdrop_tex.as_image_copy(),
                    wgpu::Extent3d {
                        width: o.w.max(1),
                        height: o.h.max(1),
                        depth_or_array_layers: 1,
                    },
                );
                let Some(mut p) = open(encoder, depth, false) else { continue };
                let slot = u32::from(color).min(COLOR_MODES - 1) * ALPHA_MODES
                    + u32::from(alpha).min(ALPHA_MODES - 1);
                p.set_pipeline(&self.advanced_pipeline);
                p.set_bind_group(1, bind, &[]);
                p.set_bind_group(2, mode_bind, &[]);
                p.set_bind_group(3, &o.backdrop_bind, &[slot * MODE_STRIDE as u32]);
                p.draw_indexed(indices.clone(), 0, 0..1);
                pass = Some(p);
                continue;
            }
            let Some(p) = pass.as_mut() else { continue };
            if let Some((_, pipeline)) = self.draw_pipelines.iter().find(|(b, _)| *b == blend) {
                p.set_pipeline(pipeline);
                p.set_bind_group(1, bind, &[]);
                p.set_bind_group(2, mode_bind, &[]);
                p.draw_indexed(indices.clone(), 0, 0..1);
            }
        }
        drop(pass);
        if !frame_cleared {
            drop(open(encoder, 0, true));
        }
    }

    pub fn paint(&self, pass: &mut wgpu::RenderPass<'static>) {
        let Some(o) = &self.offscreen else { return };
        pass.set_pipeline(&self.blit_pipeline);
        pass.set_bind_group(0, &o.blit_bind, &[]);
        pass.draw(0..3, 0..1);
    }
}

#[cfg(test)]
mod tests {
    /// The shaders parse and validate as wgpu will see them, so a typo in
    /// the blend maths fails here and not on someone's GPU.
    #[test]
    fn the_shaders_validate() {
        for (name, src) in [("model", super::SHADER), ("blit", super::BLIT)] {
            let module =
                naga::front::wgsl::parse_str(src).unwrap_or_else(|e| panic!("{name}: {e}"));
            naga::valid::Validator::new(
                naga::valid::ValidationFlags::all(),
                naga::valid::Capabilities::all(),
            )
            .validate(&module)
            .unwrap_or_else(|e| panic!("{name}: {e:?}"));
        }
    }
}
