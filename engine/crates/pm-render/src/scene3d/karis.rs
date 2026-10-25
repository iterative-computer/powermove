//! Luminance-weighted MSAA resolve (Karis).
//!
//! The scene resolves linear light and tones it once afterwards
//! (`3D-TONEMAP-1`), so a box resolve lets one bright sample (a specular
//! highlight at a triangle seam, a lamp against a dark edge) dominate its
//! pixel. Each sample instead counts with weight `w = 1 / (1 + L)`, `L` the
//! Rec. 709 luminance of its unpremultiplied color scaled as the tone
//! curve's input (`exposure / 0.6`), and coverage stays the box average:
//!
//! - color = Σ aᵢwᵢcᵢ / Σ aᵢwᵢ (unpremultiplied `cᵢ`, coverage `aᵢ`)
//! - alpha = Σ aᵢ / n
//!
//! The samples live in tile memory only, so the hardware resolve still does
//! the averaging. After the scene's last draw a full-screen pass reads each
//! sample (Metal programmable blending, run per sample) and stores its color
//! scaled by `w`, its coverage as is. Since `aᵢ − L(wᵢ·aᵢcᵢ) = aᵢwᵢ`, the
//! resolved value holds the weights' sum too: `scene_unweight`
//! (`shaders/scene_output.wgsl`) recovers the weighted color where the scene
//! output tones it.
//!
//! Weighting only above the curve's shoulder would leave ordinary samples
//! exact, but that weight has no linear relation to the stored channels, so
//! the resolve could not be undone; tone mapping each sample before the
//! resolve instead moved interiors and edges further from the plain
//! 64-sample reference (quality ledger).

/// Rec. 709 luminance of linear sRGB (the shaders' literals).
const LUMA: [f32; 3] = [0.2126, 0.7152, 0.0722];

/// One fragment per sample: the sample's color scaled by its weight, stored
/// as the nearest half (Metal's own conversion truncates).
const WEIGHT_MSL: &str = r#"
#include <metal_stdlib>
using namespace metal;

struct PmWeightVertex { float4 position [[position]]; };

vertex PmWeightVertex pm_weight_vs(uint i [[vertex_id]]) {
    PmWeightVertex out;
    out.position = float4(float(i & 1u) * 4.0 - 1.0, float(i >> 1u) * 4.0 - 1.0, 0.0, 1.0);
    return out;
}

static float pm_nearest_half(float value) {
    uint bits = as_type<uint>(value);
    uint sign_bit = bits & 0x80000000u;
    uint magnitude = bits & 0x7fffffffu;
    uint exponent = magnitude >> 23u;
    if (exponent == 255u) return value;
    if (exponent > 142u) return as_type<float>(sign_bit | 0x7f800000u);
    if (exponent < 102u) return as_type<float>(sign_bit);
    if (exponent < 113u) {
        uint significand = (magnitude & 0x007fffffu) | 0x00800000u;
        uint shift = 126u - exponent;
        uint kept = significand >> shift;
        uint lost = significand & ((1u << shift) - 1u);
        uint halfway = 1u << (shift - 1u);
        if (lost > halfway || (lost == halfway && (kept & 1u) != 0u)) kept++;
        float rounded = float(kept) * 5.9604644775390625e-8;
        return as_type<float>(as_type<uint>(rounded) | sign_bit);
    }
    uint kept = magnitude >> 13u;
    uint lost = magnitude & 0x1fffu;
    if (lost > 0x1000u || (lost == 0x1000u && (kept & 1u) != 0u)) kept++;
    uint rounded = kept << 13u;
    if (rounded >= 0x47800000u) rounded = 0x7f800000u;
    return as_type<float>(sign_bit | rounded);
}

struct PmResolveParams { float exposure; float3 unused; };

fragment float4 pm_weight_fs(PmWeightVertex vertex_in [[stage_in]],
                             float4 c [[color(0)]],
                             uint sample_index [[sample_id]],
                             constant PmResolveParams& params [[buffer(0)]]) {
    // A sample without coverage carries no color, as the scene output
    // already treats an uncovered pixel.
    if (c.a <= 0.0) return float4(0.0);
    // The luminance the tone curve sees: exposure / 0.6 scales its input.
    float l = max(dot(c.rgb, float3(0.2126, 0.7152, 0.0722)), 0.0) * (params.exposure / 0.6);
    float3 rgb = c.rgb * (c.a / (c.a + l));
    return float4(pm_nearest_half(rgb.r), pm_nearest_half(rgb.g), pm_nearest_half(rgb.b), c.a);
}
"#;

/// `scene_unweight` (`shaders/scene_output.wgsl`) on the CPU, for readbacks
/// of the raw scene target.
pub fn unweight(pixels: &mut [f32], exposure: f32) {
    let k = exposure / 0.6;
    for p in pixels.chunks_exact_mut(4) {
        if p[3] <= 0.0 {
            p.fill(0.0);
            continue;
        }
        let l = (p[0] * LUMA[0] + p[1] * LUMA[1] + p[2] * LUMA[2]).max(0.0) * k;
        let scale = p[3] / (p[3] - l).max(1.0 / 16384.0);
        for v in &mut p[..3] {
            *v *= scale;
        }
    }
}

/// The weighting pass of a multisampled scene target (Metal only; another
/// back end resolves the box average and reports it unweighted).
pub(crate) struct Weighting {
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    /// Parameters by exposure bits.
    groups: std::collections::HashMap<u32, wgpu::BindGroup>,
}

impl Weighting {
    pub(crate) fn new(device: &wgpu::Device, samples: u32) -> Option<Weighting> {
        if samples < 2 || !pm_gpu::msl::supported(device) {
            return None;
        }
        let vs = pm_gpu::msl::from_source(device, "3d resolve weights", WEIGHT_MSL, "pm_weight_vs")
            .expect("weighting vertex stage");
        let fs = pm_gpu::msl::from_source(device, "3d resolve weights", WEIGHT_MSL, "pm_weight_fs")
            .expect("weighting fragment stage");
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("3d resolve weights"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        // Passthrough MSL stages cannot derive an implicit layout. The
        // fragment stage's only buffer is Metal buffer 0.
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("3d resolve weights"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("3d resolve weights"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &vs.module,
                entry_point: Some(&vs.entry_point),
                compilation_options: Default::default(),
                buffers: &[],
            },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: Some(false),
                depth_compare: Some(wgpu::CompareFunction::Always),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: wgpu::MultisampleState {
                count: samples,
                ..Default::default()
            },
            fragment: Some(wgpu::FragmentState {
                module: &fs.module,
                entry_point: Some(&fs.entry_point),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: wgpu::TextureFormat::Rgba16Float,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        Some(Weighting {
            pipeline,
            layout,
            groups: Default::default(),
        })
    }

    /// The parameters of a scene's `exposure` (few distinct values).
    pub(crate) fn params(&mut self, device: &wgpu::Device, exposure: f32) -> wgpu::BindGroup {
        if self.groups.len() > 16 {
            self.groups.clear();
        }
        let layout = &self.layout;
        self.groups
            .entry(exposure.to_bits())
            .or_insert_with(|| {
                let words = [exposure, 0.0, 0.0, 0.0];
                let bytes: Vec<u8> = words.iter().flat_map(|w| w.to_le_bytes()).collect();
                let buffer = wgpu::util::DeviceExt::create_buffer_init(
                    device,
                    &wgpu::util::BufferInitDescriptor {
                        label: Some("3d resolve weights"),
                        contents: &bytes,
                        usage: wgpu::BufferUsages::UNIFORM,
                    },
                );
                device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("3d resolve weights"),
                    layout,
                    entries: &[wgpu::BindGroupEntry {
                        binding: 0,
                        resource: buffer.as_entire_binding(),
                    }],
                })
            })
            .clone()
    }

    /// Weights every sample of the pass's color attachment: the pass's last
    /// draw.
    pub(crate) fn draw(&self, pass: &mut wgpu::RenderPass<'_>, params: &wgpu::BindGroup) {
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, params, &[]);
        pass.draw(0..3, 0..1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Twice the curve's unit input scale.
    const EXPOSURE: f32 = 1.2;

    /// Resolves 4×1 pixels whose four samples are given (each pixel's
    /// samples from a per-sample color table), weighted, and reads them back.
    fn resolve(gpu: &pm_gpu::Gpu, samples: [[[f32; 4]; 4]; 4]) -> Vec<f32> {
        let mut weighting = Weighting::new(&gpu.device, 4).expect("Metal weighting");
        let params = weighting.params(&gpu.device, EXPOSURE);
        let texture = |count, usage| {
            gpu.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("weights"),
                size: wgpu::Extent3d {
                    width: 4,
                    height: 1,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: count,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba16Float,
                usage,
                view_formats: &[],
            })
        };
        let resolved = texture(
            1,
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        );
        let msaa = texture(4, wgpu::TextureUsages::RENDER_ATTACHMENT);
        let depth = gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("weights depth"),
            size: wgpu::Extent3d {
                width: 4,
                height: 1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 4,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Depth32Float,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        // Each sample's color from a storage table, by pixel and sample.
        let table: Vec<f32> = samples.iter().flatten().flatten().copied().collect();
        let source = format!(
            "var<private> TABLE: array<vec4<f32>, 16> = array<vec4<f32>, 16>({});\n{}",
            table
                .chunks_exact(4)
                .map(|c| format!("vec4<f32>({:?}, {:?}, {:?}, {:?})", c[0], c[1], c[2], c[3]))
                .collect::<Vec<_>>()
                .join(", "),
            r#"
@vertex
fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    return vec4<f32>(f32(i & 1u) * 4.0 - 1.0, f32(i >> 1u) * 4.0 - 1.0, 0.0, 1.0);
}
@fragment
fn fs(@builtin(position) p: vec4<f32>, @builtin(sample_index) s: u32) -> @location(0) vec4<f32> {
    return TABLE[u32(p.x) * 4u + s];
}
"#
        );
        let module = gpu
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("weights samples"),
                source: wgpu::ShaderSource::Wgsl(source.into()),
            });
        let fill = gpu
            .device
            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("weights samples"),
                layout: None,
                vertex: wgpu::VertexState {
                    module: &module,
                    entry_point: Some("vs"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                primitive: Default::default(),
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: wgpu::TextureFormat::Depth32Float,
                    depth_write_enabled: Some(false),
                    depth_compare: Some(wgpu::CompareFunction::Always),
                    stencil: Default::default(),
                    bias: Default::default(),
                }),
                multisample: wgpu::MultisampleState {
                    count: 4,
                    ..Default::default()
                },
                fragment: Some(wgpu::FragmentState {
                    module: &module,
                    entry_point: Some("fs"),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::TextureFormat::Rgba16Float.into())],
                }),
                multiview_mask: None,
                cache: None,
            });
        let msaa_view = msaa.create_view(&Default::default());
        let resolved_view = resolved.create_view(&Default::default());
        let depth_view = depth.create_view(&Default::default());
        let mut encoder = gpu.device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("weights"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &msaa_view,
                    depth_slice: None,
                    resolve_target: Some(&resolved_view),
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Discard,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Discard,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&fill);
            pass.draw(0..3, 0..1);
            weighting.draw(&mut pass, &params);
        }
        gpu.queue.submit([encoder.finish()]);
        let mut out = pm_gpu::readback::read_f32(&gpu.device, &gpu.queue, &resolved)
            .expect("weights readback");
        unweight(&mut out, EXPOSURE);
        out
    }

    /// The expected value in f64: weighted color, box coverage.
    fn expected(samples: &[[f32; 4]; 4]) -> [f64; 4] {
        let (mut rgb, mut weights, mut alpha) = ([0f64; 3], 0f64, 0f64);
        for s in samples {
            let a = f64::from(s[3]);
            alpha += a / 4.0;
            if a <= 0.0 {
                continue;
            }
            let l: f64 = (0..3).map(|i| f64::from(s[i]) * f64::from(LUMA[i])).sum::<f64>()
                * f64::from(EXPOSURE)
                / 0.6;
            let w = a / (a + l);
            weights += a * w;
            for i in 0..3 {
                rgb[i] += w * f64::from(s[i]);
            }
        }
        let scale = if weights > 0.0 { alpha / weights } else { 0.0 };
        [rgb[0] * scale, rgb[1] * scale, rgb[2] * scale, alpha]
    }

    #[test]
    fn samples_resolve_luminance_weighted_with_box_coverage() {
        let gpu = pm_gpu::Gpu::new("scene resolve weights").expect("GPU device");
        if !pm_gpu::msl::supported(&gpu.device) {
            return;
        }
        let dim = [0.25, 0.25, 0.25, 1.0];
        let bright = [64.0, 64.0, 64.0, 1.0];
        let none = [0.0; 4];
        let half = [0.3, 0.1, 0.05, 0.5];
        let samples = [
            // Opaque and uniform: the color itself.
            [dim; 4],
            // One highlight among four: weighted, not a quarter of 64.
            [bright, dim, dim, dim],
            // Half covered: the color is unchanged, the coverage halves.
            [bright, bright, none, none],
            // Translucent premultiplied samples among opaque ones.
            [half, dim, half, none],
        ];
        let out = resolve(&gpu, samples);
        for (k, s) in samples.iter().enumerate() {
            let want = expected(s);
            // Half stores lose about 2^-12 relative; recovering the weights'
            // sum by subtraction scales that by 1 + L (L the unpremultiplied
            // luminance), which only HDR values, toned near 1, notice.
            let l = (0..3).map(|i| want[i] * f64::from(LUMA[i])).sum::<f64>() / want[3] * 2.0;
            for i in 0..4 {
                let got = f64::from(out[k * 4 + i]);
                assert!(
                    (got - want[i]).abs() <= want[i].abs() * (1.0 + l) * 5e-4,
                    "pixel {k} channel {i}: {got} want {}",
                    want[i]
                );
            }
        }
        // The highlight pixel stays near the dim color, not the box mean 16.2.
        assert!(out[4] < 1.0, "{}", out[4]);
        assert_eq!(out[11], 0.5);
    }
}
