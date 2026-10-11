//! Device capabilities: rgba32float filtering and blending (reported flags and
//! functional tests), Metal device facts.

use objc2_metal::{MTLDevice, MTLGPUFamily};
use serde::Serialize;

use crate::cf::Result;
use crate::gpu::Gpu;

#[derive(Serialize)]
pub struct FormatCaps {
    pub format: String,
    pub allowed_usages: String,
    pub flags: String,
}

#[derive(Serialize)]
pub struct CapsReport {
    pub adapter: String,
    pub backend: String,
    pub driver: String,
    pub metal_device: String,
    pub metal_registry_id: u64,
    pub metal_supports_32bit_float_filtering: bool,
    pub metal_supports_32bit_msaa: bool,
    pub metal_family_apple9: bool,
    pub metal_family_metal3: bool,
    pub wgpu_features_requested_and_granted: String,
    pub formats: Vec<FormatCaps>,
    pub filter_test: FilterTest,
    pub blend_test: BlendTest,
}

#[derive(Serialize)]
pub struct FilterTest {
    /// Texels 1.0 and 1.0 + 2⁻²⁰ sampled at the midpoint: f32 filtering gives 1 + 2⁻²¹.
    pub midpoint_expected: f64,
    pub midpoint_got: f64,
    pub midpoint_exact: bool,
    /// Texels 0 and 1, sampled at 1024 sub-texel positions: max error of the
    /// interpolation weight, and the implied weight precision in bits.
    pub weight_max_error: f64,
    pub weight_bits: f64,
    pub distinct_weights: usize,
    /// The same sweep on an rgba16float texture (weights are a sampler property).
    pub rgba16float_weight_max_error: f64,
    pub rgba16float_distinct_weights: usize,
}

#[derive(Serialize)]
pub struct BlendTest {
    /// dst = 1.0, src = 2⁻²² with additive blending: f32 blending keeps it.
    pub additive_expected: f64,
    pub additive_got: f64,
    pub additive_exact: bool,
    /// Source-over with α = 0.3 and values needing 24-bit mantissas: max error versus f32 CPU math, in ULP.
    pub over_max_ulp: f64,
}

const TEST_WGSL: &str = r#"
struct U { a: vec4<f32>, b: vec4<f32> };
@group(0) @binding(0) var tex: texture_2d<f32>;
@group(0) @binding(1) var smp: sampler;
@group(0) @binding(2) var<uniform> u: U;

@vertex
fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    var p = array<vec2<f32>, 3>(vec2(-1.0, -3.0), vec2(-1.0, 1.0), vec2(3.0, 1.0));
    return vec4(p[i], 0.0, 1.0);
}

// Output pixel x samples the 2×1 texture at u = (0.5 + x / (width − 1)) / 2,
// sweeping from texel 0's center to texel 1's center.
@fragment
fn fs_sample(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    let w = u.a.x;
    let t = floor(pos.x) / (w - 1.0);
    let uv = vec2((0.5 + t) / 2.0, 0.5);
    return textureSampleLevel(tex, smp, uv, 0.0);
}

@fragment
fn fs_const(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    return u.b;
}
"#;

fn fmt_flags(gpu: &Gpu, f: wgpu::TextureFormat) -> FormatCaps {
    let ff = gpu.adapter.get_texture_format_features(f);
    FormatCaps { format: format!("{f:?}"), allowed_usages: format!("{:?}", ff.allowed_usages), flags: format!("{:?}", ff.flags) }
}

pub fn run(gpu: &Gpu) -> Result<CapsReport> {
    let info = gpu.adapter.get_info();
    let mtl = gpu.mtl_device();
    let formats = [
        wgpu::TextureFormat::Rgba32Float,
        wgpu::TextureFormat::Rgba16Float,
        wgpu::TextureFormat::Rgba16Unorm,
        wgpu::TextureFormat::R16Unorm,
        wgpu::TextureFormat::Rg16Unorm,
        wgpu::TextureFormat::Bgra8Unorm,
    ]
    .into_iter()
    .map(|f| fmt_flags(gpu, f))
    .collect();
    Ok(CapsReport {
        adapter: info.name.clone(),
        backend: format!("{:?}", info.backend),
        driver: format!("{} {}", info.driver, info.driver_info),
        metal_device: mtl.name().to_string(),
        metal_registry_id: mtl.registryID(),
        metal_supports_32bit_float_filtering: mtl.supports32BitFloatFiltering(),
        metal_supports_32bit_msaa: mtl.supports32BitMSAA(),
        metal_family_apple9: mtl.supportsFamily(MTLGPUFamily::Apple9),
        metal_family_metal3: mtl.supportsFamily(MTLGPUFamily::Metal3),
        wgpu_features_requested_and_granted: format!("{:?}", gpu.device.features() & crate::gpu::WANTED_FEATURES),
        formats,
        filter_test: filter_test(gpu),
        blend_test: blend_test(gpu),
    })
}

struct Rig {
    module: wgpu::ShaderModule,
    bgl: wgpu::BindGroupLayout,
    layout: wgpu::PipelineLayout,
}

fn rig(gpu: &Gpu, filterable: bool) -> Rig {
    let dev = &gpu.device;
    let module = dev.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("caps"), source: wgpu::ShaderSource::Wgsl(TEST_WGSL.into()) });
    let bgl = dev.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: None,
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(if filterable { wgpu::SamplerBindingType::Filtering } else { wgpu::SamplerBindingType::NonFiltering }),
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 2,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                count: None,
            },
        ],
    });
    let layout = dev.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: None, bind_group_layouts: &[Some(&bgl)], immediate_size: 0 });
    Rig { module, bgl, layout }
}

fn tex32(gpu: &Gpu, w: u32, h: u32, data: Option<&[f32]>) -> wgpu::Texture {
    let t = gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: None,
        size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba32Float,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    if let Some(d) = data {
        gpu.queue.write_texture(
            t.as_image_copy(),
            &d.iter().flat_map(|f| f.to_le_bytes()).collect::<Vec<u8>>(),
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(w * 16), rows_per_image: None },
            wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        );
    }
    t
}

fn read_tex32(gpu: &Gpu, t: &wgpu::Texture) -> Vec<f32> {
    let (w, h) = (t.width(), t.height());
    let bpr = (w * 16).div_ceil(256) * 256;
    let buf = gpu.device.create_buffer(&wgpu::BufferDescriptor { label: None, size: (bpr * h) as u64, usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
    let mut enc = gpu.device.create_command_encoder(&Default::default());
    enc.copy_texture_to_buffer(
        t.as_image_copy(),
        wgpu::TexelCopyBufferInfo { buffer: &buf, layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(bpr), rows_per_image: None } },
        wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
    );
    gpu.queue.submit([enc.finish()]);
    buf.slice(..).map_async(wgpu::MapMode::Read, |_| {});
    gpu.wait();
    let d = buf.slice(..).get_mapped_range().expect("map");
    let mut out = vec![];
    for y in 0..h {
        let row = &d[(y * bpr) as usize..(y * bpr + w * 16) as usize];
        out.extend(row.chunks_exact(4).map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]])));
    }
    out
}

fn uniform(gpu: &Gpu, v: [f32; 8]) -> wgpu::Buffer {
    let b = gpu.device.create_buffer(&wgpu::BufferDescriptor { label: None, size: 32, usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
    gpu.queue.write_buffer(&b, 0, &v.iter().flat_map(|f| f.to_le_bytes()).collect::<Vec<u8>>());
    b
}

fn draw(gpu: &Gpu, r: &Rig, entry: &str, src: &wgpu::Texture, u: &wgpu::Buffer, target: &wgpu::Texture, blend: Option<wgpu::BlendState>, load: wgpu::LoadOp<wgpu::Color>) {
    let dev = &gpu.device;
    let pipeline = dev.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(entry),
        layout: Some(&r.layout),
        vertex: wgpu::VertexState { module: &r.module, entry_point: Some("vs"), compilation_options: Default::default(), buffers: &[] },
        primitive: Default::default(),
        depth_stencil: None,
        multisample: Default::default(),
        fragment: Some(wgpu::FragmentState {
            module: &r.module,
            entry_point: Some(entry),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState { format: target.format(), blend, write_mask: wgpu::ColorWrites::ALL })],
        }),
        multiview_mask: None,
        cache: None,
    });
    let sampler = dev.create_sampler(&wgpu::SamplerDescriptor {
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        address_mode_u: wgpu::AddressMode::ClampToEdge,
        address_mode_v: wgpu::AddressMode::ClampToEdge,
        ..Default::default()
    });
    let sv = src.create_view(&Default::default());
    let bg = dev.create_bind_group(&wgpu::BindGroupDescriptor {
        label: None,
        layout: &r.bgl,
        entries: &[
            wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&sv) },
            wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&sampler) },
            wgpu::BindGroupEntry { binding: 2, resource: u.as_entire_binding() },
        ],
    });
    let tv = target.create_view(&Default::default());
    let mut enc = dev.create_command_encoder(&Default::default());
    {
        let mut rp = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: None,
            color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: &tv, depth_slice: None, resolve_target: None, ops: wgpu::Operations { load, store: wgpu::StoreOp::Store } })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        rp.set_pipeline(&pipeline);
        rp.set_bind_group(0, &bg, &[]);
        rp.draw(0..3, 0..1);
    }
    gpu.queue.submit([enc.finish()]);
    gpu.wait();
}

fn filter_test(gpu: &Gpu) -> FilterTest {
    let r = rig(gpu, true);
    let eps = 2f32.powi(-20);
    let src = tex32(gpu, 2, 1, Some(&[1.0, 0.0, 0.0, 1.0, 1.0 + eps, 1.0, 0.0, 1.0]));
    // 3 output pixels: t = 0, 0.5, 1.
    let w = 3u32;
    let target = tex32(gpu, w, 1, None);
    let u = uniform(gpu, [w as f32, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0]);
    draw(gpu, &r, "fs_sample", &src, &u, &target, None, wgpu::LoadOp::Clear(wgpu::Color::BLACK));
    let out = read_tex32(gpu, &target);
    let mid = out[4] as f64;
    let expected = 1.0 + (eps as f64) / 2.0;

    let w = 1025u32;
    let target = tex32(gpu, w, 1, None);
    let u = uniform(gpu, [w as f32, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0]);
    draw(gpu, &r, "fs_sample", &src, &u, &target, None, wgpu::LoadOp::Clear(wgpu::Color::BLACK));
    let out = read_tex32(gpu, &target);
    let mut maxerr = 0f64;
    let mut distinct = std::collections::BTreeSet::new();
    for x in 0..w as usize {
        let t = x as f64 / (w - 1) as f64;
        let g = out[x * 4 + 1] as f64; // green: 0 → 1
        maxerr = maxerr.max((g - t).abs());
        distinct.insert(out[x * 4 + 1].to_bits());
    }
    // Same sweep from an rgba16float texture (texels 0 and 1 are exact in half float).
    let half = gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: None,
        size: wgpu::Extent3d { width: 2, height: 1, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba16Float,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let one: u16 = 0x3C00;
    let texels: [u16; 8] = [one, 0, 0, one, one, one, 0, one];
    gpu.queue.write_texture(
        half.as_image_copy(),
        &texels.iter().flat_map(|v| v.to_le_bytes()).collect::<Vec<u8>>(),
        wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(16), rows_per_image: None },
        wgpu::Extent3d { width: 2, height: 1, depth_or_array_layers: 1 },
    );
    draw(gpu, &r, "fs_sample", &half, &u, &target, None, wgpu::LoadOp::Clear(wgpu::Color::BLACK));
    let out16 = read_tex32(gpu, &target);
    let mut maxerr16 = 0f64;
    let mut distinct16 = std::collections::BTreeSet::new();
    for x in 0..w as usize {
        let t = x as f64 / (w - 1) as f64;
        maxerr16 = maxerr16.max((out16[x * 4 + 1] as f64 - t).abs());
        distinct16.insert(out16[x * 4 + 1].to_bits());
    }
    FilterTest {
        rgba16float_weight_max_error: maxerr16,
        rgba16float_distinct_weights: distinct16.len(),
        midpoint_expected: expected,
        midpoint_got: mid,
        midpoint_exact: mid == expected,
        weight_max_error: maxerr,
        weight_bits: -(maxerr * 2.0).log2(),
        distinct_weights: distinct.len(),
    }
}

fn blend_test(gpu: &Gpu) -> BlendTest {
    let r = rig(gpu, true);
    let dummy = tex32(gpu, 1, 1, Some(&[0.0; 4]));
    // Additive: dst 1.0, src 2⁻²².
    let target = tex32(gpu, 4, 4, Some(&[1.0; 64]));
    let tiny = 2f32.powi(-22);
    let u = uniform(gpu, [0.0, 0.0, 0.0, 0.0, tiny, tiny, tiny, 0.0]);
    let add = wgpu::BlendState {
        color: wgpu::BlendComponent { src_factor: wgpu::BlendFactor::One, dst_factor: wgpu::BlendFactor::One, operation: wgpu::BlendOperation::Add },
        alpha: wgpu::BlendComponent { src_factor: wgpu::BlendFactor::Zero, dst_factor: wgpu::BlendFactor::One, operation: wgpu::BlendOperation::Add },
    };
    draw(gpu, &r, "fs_const", &dummy, &u, &target, Some(add), wgpu::LoadOp::Load);
    let out = read_tex32(gpu, &target);
    let add_got = out[0] as f64;
    let add_exp = (1.0f32 + tiny) as f64;

    // Source-over: dst values with full mantissas, src (0.123456789, α = 0.3).
    let dst: Vec<f32> = (0..64).map(|i| 0.1 + i as f32 * 0.0123456).collect();
    let target = tex32(gpu, 4, 4, Some(&dst));
    let (s, a) = (0.123_456_79f32, 0.3f32);
    let u = uniform(gpu, [0.0, 0.0, 0.0, 0.0, s, s, s, a]);
    let over = wgpu::BlendState::ALPHA_BLENDING;
    draw(gpu, &r, "fs_const", &dummy, &u, &target, Some(over), wgpu::LoadOp::Load);
    let out = read_tex32(gpu, &target);
    let mut max_ulp = 0f64;
    for i in 0..16 {
        for c in 0..3 {
            let d = dst[i * 4 + c];
            let want = (s * a + d * (1.0 - a)) as f64;
            let ulp = (f32::from_bits((want as f32).to_bits() + 1) - want as f32) as f64;
            max_ulp = max_ulp.max(((out[i * 4 + c] as f64) - want).abs() / ulp);
        }
    }
    BlendTest { additive_expected: add_exp, additive_got: add_got, additive_exact: add_got == add_exp, over_max_ulp: max_ulp }
}
