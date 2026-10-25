//! Round linear fragment color before the RGBA16F scene attachment stores it.
//!
//! Metal rounds the physical half-float color write toward zero. On this
//! adapter even GLSL `unpackHalf2x16(packHalf2x16(x))` lowers to a half
//! conversion that rounds toward zero (the known-value GPU regression reads
//! 0.3 as 0.2998047). This integer conversion computes nearest-even binary16
//! *before* the attachment sees the value. MSAA and its hardware resolve
//! remain in linear light; no previously quantized scene color is adjusted.

const NEAREST_HALF_GLSL: &str = r#"
highp float pm_scene_nearest_half(highp float value) {
    highp uint bits = floatBitsToUint(value);
    highp uint sign_bit = bits & 0x80000000u;
    highp uint magnitude = bits & 0x7fffffffu;
    highp uint exponent = magnitude >> 23u;
    if (exponent == 255u) return value;
    if (exponent > 142u) return uintBitsToFloat(sign_bit | 0x7f800000u);
    if (exponent < 102u) return uintBitsToFloat(sign_bit);
    if (exponent < 113u) {
        highp uint significand = (magnitude & 0x007fffffu) | 0x00800000u;
        highp uint shift = 126u - exponent;
        highp uint kept = significand >> shift;
        highp uint lost = significand & ((1u << shift) - 1u);
        highp uint halfway = 1u << (shift - 1u);
        if (lost > halfway || (lost == halfway && (kept & 1u) != 0u)) kept++;
        highp float rounded = float(kept) * 5.9604644775390625e-8;
        return uintBitsToFloat(floatBitsToUint(rounded) | sign_bit);
    }
    highp uint kept = magnitude >> 13u;
    highp uint lost = magnitude & 0x1fffu;
    if (lost > 0x1000u || (lost == 0x1000u && (kept & 1u) != 0u)) kept++;
    highp uint rounded = kept << 13u;
    if (rounded >= 0x47800000u) rounded = 0x7f800000u;
    return uintBitsToFloat(sign_bit | rounded);
}

highp vec4 pm_scene_half_color(highp vec4 color) {
    return vec4(pm_scene_nearest_half(color.r),
                pm_scene_nearest_half(color.g),
                pm_scene_nearest_half(color.b),
                pm_scene_nearest_half(color.a));
}
"#;

/// The same integer conversion for WGSL scene-output fragments. Metal's
/// `quantizeToF16` and native half conversion may lower to round-toward-zero,
/// so the float32 bit pattern is rounded explicitly before an RGBA16F store.
/// Include this only in pipelines whose physical color target is RGBA16F.
pub const NEAREST_HALF_WGSL: &str = r#"
fn pm_scene_nearest_half(value: f32) -> f32 {
    let bits = bitcast<u32>(value);
    let sign_bit = bits & 0x80000000u;
    let magnitude = bits & 0x7fffffffu;
    let exponent = magnitude >> 23u;
    if (exponent == 255u) { return value; }
    if (exponent > 142u) { return bitcast<f32>(sign_bit | 0x7f800000u); }
    if (exponent < 102u) { return bitcast<f32>(sign_bit); }
    if (exponent < 113u) {
        let significand = (magnitude & 0x007fffffu) | 0x00800000u;
        let shift = 126u - exponent;
        var kept = significand >> shift;
        let lost = significand & ((1u << shift) - 1u);
        let halfway = 1u << (shift - 1u);
        if (lost > halfway || (lost == halfway && (kept & 1u) != 0u)) { kept += 1u; }
        let rounded = f32(kept) * 5.9604644775390625e-8;
        return bitcast<f32>(bitcast<u32>(rounded) | sign_bit);
    }
    var kept = magnitude >> 13u;
    let lost = magnitude & 0x1fffu;
    if (lost > 0x1000u || (lost == 0x1000u && (kept & 1u) != 0u)) { kept += 1u; }
    var rounded = kept << 13u;
    if (rounded >= 0x47800000u) { rounded = 0x7f800000u; }
    return bitcast<f32>(sign_bit | rounded);
}

fn pm_scene_half_color(color: vec4<f32>) -> vec4<f32> {
    return vec4<f32>(pm_scene_nearest_half(color.r),
                     pm_scene_nearest_half(color.g),
                     pm_scene_nearest_half(color.b),
                     pm_scene_nearest_half(color.a));
}
"#;

/// The authored three.js source stays unchanged. Only the native 3D color
/// program receives a conversion at the last fragment output assignment.
pub(super) fn at_fragment_output(source: &str) -> Result<String, String> {
    let main = source
        .rfind("void main()")
        .ok_or_else(|| "3D fragment has no main".to_owned())?;
    let close = source
        .rfind('}')
        .ok_or_else(|| "3D fragment has no closing brace".to_owned())?;
    if close < main || !source[close + 1..].trim().is_empty() {
        return Err("3D fragment main is not the final function".into());
    }
    let mut result = String::with_capacity(source.len() + NEAREST_HALF_GLSL.len() + 80);
    result.push_str(&source[..main]);
    result.push_str(NEAREST_HALF_GLSL);
    result.push_str(&source[main..close]);
    result.push_str("\n pc_fragColor = pm_scene_half_color(pc_fragColor);\n");
    result.push_str(&source[close..]);
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    const FULLSCREEN_WGSL: &str = r#"
@vertex
fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    return vec4<f32>(f32(i & 1u) * 2.0 - 1.0, f32(i >> 1u) * 2.0 - 1.0, 0.0, 1.0);
}
"#;

    /// Draws one full-screen strip into a 4×1 RGBA16F target (resolved from
    /// `samples` when multisampled) and reads back each pixel's red channel.
    fn draw(
        gpu: &pm_gpu::Gpu,
        (vs, vs_entry): (&wgpu::ShaderModule, Option<&str>),
        (fs, fs_entry): (&wgpu::ShaderModule, Option<&str>),
        samples: u32,
        blend: Option<wgpu::BlendState>,
    ) -> Vec<f32> {
        let format = wgpu::TextureFormat::Rgba16Float;
        let texture = |samples, usage| {
            gpu.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("half rounding target"),
                size: wgpu::Extent3d {
                    width: 4,
                    height: 1,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: samples,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage,
                view_formats: &[],
            })
        };
        let usage = wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC;
        let resolved = texture(1, usage);
        let msaa = (samples > 1).then(|| texture(samples, wgpu::TextureUsages::RENDER_ATTACHMENT));
        // Passthrough MSL stages cannot derive an implicit layout.
        let layout = gpu
            .device
            .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("half rounding"),
                bind_group_layouts: &[],
                immediate_size: 0,
            });
        let pipeline = gpu
            .device
            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("half rounding"),
                layout: Some(&layout),
                vertex: wgpu::VertexState {
                    module: vs,
                    entry_point: vs_entry,
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::TriangleStrip,
                    ..Default::default()
                },
                depth_stencil: None,
                multisample: wgpu::MultisampleState {
                    count: samples,
                    ..Default::default()
                },
                fragment: Some(wgpu::FragmentState {
                    module: fs,
                    entry_point: fs_entry,
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            });
        let resolved_view = resolved.create_view(&Default::default());
        let msaa_view = msaa.as_ref().map(|t| t.create_view(&Default::default()));
        let mut encoder = gpu.device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("half rounding"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: msaa_view.as_ref().unwrap_or(&resolved_view),
                    depth_slice: None,
                    resolve_target: msaa_view.as_ref().map(|_| &resolved_view),
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
            pass.set_pipeline(&pipeline);
            pass.draw(0..4, 0..1);
        }
        gpu.queue.submit([encoder.finish()]);
        pm_gpu::readback::read_f32(&gpu.device, &gpu.queue, &resolved)
            .expect("half target readback")
            .chunks_exact(4)
            .map(|pixel| pixel[0])
            .collect()
    }

    fn wgsl(gpu: &pm_gpu::Gpu, source: String) -> wgpu::ShaderModule {
        gpu.device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("half rounding"),
                source: wgpu::ShaderSource::Wgsl(source.into()),
            })
    }

    fn nearest(values: [f32; 4]) -> Vec<f32> {
        use pm_gpu::readback::{f32_to_half, half_to_f32};
        values.map(|v| half_to_f32(f32_to_half(v))).to_vec()
    }

    /// The conversion every native 3D color program receives overcomes the
    /// RGBA16F attachment's round-toward-zero store, with and without blend.
    #[test]
    fn glsl_color_output_stores_nearest_half() {
        let gpu = pm_gpu::Gpu::new("scene color half rounding").expect("GPU device");
        let vertex = "#version 300 es\nprecision highp float;\nvoid main() {\n    gl_Position = vec4(float(gl_VertexID & 1) * 2.0 - 1.0, float(gl_VertexID >> 1) * 2.0 - 1.0, 0.0, 1.0);\n}";
        let fragment = at_fragment_output(
            "#version 300 es\nprecision highp float;\nlayout(location = 0) out highp vec4 pc_fragColor;\nvoid main() {\n    float v = 0.1;\n    if (gl_FragCoord.x > 1.0) v = 0.2;\n    if (gl_FragCoord.x > 2.0) v = 0.3;\n    if (gl_FragCoord.x > 3.0) v = 0.6;\n    pc_fragColor = vec4(v, v, v, 1.0);\n}",
        )
        .expect("3D output conversion");
        let program = pm_gpu::glsl::compile(vertex, &fragment).expect("3D GLSL translation");
        let stage = |module, info, stage, label| {
            if pm_gpu::msl::supported(&gpu.device) {
                let m = pm_gpu::msl::module(
                    &gpu.device,
                    label,
                    &module,
                    info,
                    stage,
                    &[],
                    pm_gpu::msl::MathMode::Safe,
                )
                .expect("safe MSL");
                (m.module, Some(m.entry_point))
            } else {
                let m = gpu
                    .device
                    .create_shader_module(wgpu::ShaderModuleDescriptor {
                        label: Some(label),
                        source: wgpu::ShaderSource::Naga(std::borrow::Cow::Owned(module)),
                    });
                (m, None)
            }
        };
        use pm_gpu::glsl::naga::ShaderStage;
        let (vs, vs_entry) = stage(
            program.vertex,
            &program.vertex_info,
            ShaderStage::Vertex,
            "vertex",
        );
        let (fs, fs_entry) = stage(
            program.fragment,
            &program.fragment_info,
            ShaderStage::Fragment,
            "fragment",
        );
        for blend in [None, Some(wgpu::BlendState::ALPHA_BLENDING)] {
            let red = draw(
                &gpu,
                (&vs, vs_entry.as_deref()),
                (&fs, fs_entry.as_deref()),
                1,
                blend,
            );
            assert_eq!(red, nearest([0.1, 0.2, 0.3, 0.6]), "blend {blend:?}");
            assert!(red[2] > 0.3 && red[3] > 0.6, "upward-rounding cases");
        }
    }

    /// The WGSL form used by the encoded scene-output passes rounds up where
    /// nearest is above and ties to even.
    #[test]
    fn wgsl_color_output_stores_nearest_half_ties_to_even() {
        let gpu = pm_gpu::Gpu::new("scene output half rounding").expect("GPU device");
        let module = wgsl(
            &gpu,
            format!(
                "{NEAREST_HALF_WGSL}\n{FULLSCREEN_WGSL}\n@fragment fn fs(@builtin(position) p: vec4<f32>) -> @location(0) vec4<f32> {{\n    var v = array<f32, 4>(0.1, 0.3, 0.6, 1.00146484375)[u32(p.x)];\n    return pm_scene_half_color(vec4<f32>(v, v, v, 1.0));\n}}"
            ),
        );
        let red = draw(&gpu, (&module, Some("vs")), (&module, Some("fs")), 1, None);
        assert_eq!(
            red,
            vec![0.0999755859375, 0.300048828125, 0.60009765625, 1.001953125]
        );
    }

    /// The hardware MSAA resolve of the scene attachment rounds a non-tie
    /// mean to nearest; it adds no second downward conversion.
    #[test]
    fn msaa_resolve_rounds_non_tie_mean_to_nearest() {
        let gpu = pm_gpu::Gpu::new("half MSAA resolve").expect("GPU device");
        // Samples store as one 0.0999755859375 and three 0.10003662109375;
        // their mean is above the halfway point between the two.
        let module = wgsl(
            &gpu,
            format!(
                "{NEAREST_HALF_WGSL}\n{FULLSCREEN_WGSL}\n@fragment fn fs(@builtin(sample_index) sample: u32) -> @location(0) vec4<f32> {{\n    let v = pm_scene_nearest_half(select(0.10003662109375, 0.0999755859375, sample == 0u));\n    return vec4<f32>(v, v, v, 1.0);\n}}"
            ),
        );
        let red = draw(&gpu, (&module, Some("vs")), (&module, Some("fs")), 4, None);
        assert_eq!(red[0], 0.10003662109375);
    }
}
