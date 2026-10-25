//! Execute the shared CSS gamut WGSL on Metal against the independent f64
//! reference and published Web Platform Tests MINDE examples.

#[cfg(target_os = "macos")]
#[test]
fn css_gamut_wgsl_matches_reference_on_metal() {
    const DRIVER: &str = r#"
@group(0) @binding(0) var<storage, read> colors: array<vec4<f32>>;
@group(0) @binding(1) var mapped: texture_storage_2d<rgba32float, write>;

@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    let index = id.x;
    if (index >= textureDimensions(mapped).x) { return; }
    textureStore(mapped, vec2<i32>(i32(index), 0),
        vec4<f32>(pm_css_gamut_srgb(colors[index].rgb), 1.0));
}
"#;

    let controls = [
        [0.0, 0.0, 0.0],
        [1.0, 1.0, 1.0],
        [0.2, 0.5, 0.8],
        [200.0 / 255.0, 100.0 / 255.0, 50.0 / 255.0],
        [1.0, 0.0, 0.5],
    ];
    let mut samples = controls.to_vec();
    // Straight, encoded extended-sRGB triplets on both sides of each face.
    for r in [-0.5, 0.0, 0.3, 0.7, 1.0, 1.5] {
        for g in [-0.5, 0.0, 0.3, 0.7, 1.0, 1.5] {
            for b in [-0.5, 0.0, 0.3, 0.7, 1.0, 1.5] {
                samples.push([r, g, b]);
            }
        }
    }
    samples.extend([
        [1.001, 0.2, 0.3],
        [0.0, 1.02, 0.0],
        [2.0, 2.0, 2.0],
        [-1.0, -1.0, -1.0],
    ]);
    // Encoded sRGB conversions of WPT's oklch(80% 0.35 30) and
    // oklch(60% 0.35 60). The paired values are the published MINDE
    // results, in 8-bit code values (not WPT's three-algorithm mean):
    // https://github.com/web-platform-tests/wpt/blob/master/css/css-color/canvas-srgb-gamut-mapping.html
    let wpt = [
        (
            [1.40671530241805, -0.21907629207175694, -0.18751316144355487],
            [255.0, 153.55, 135.55],
        ),
        (
            [
                1.0135949638751496,
                -0.22647951735446825,
                -0.3724720233721723,
            ],
            [195.29, 98.41, 0.0],
        ),
    ];
    let wpt_start = samples.len();
    samples.extend(wpt.iter().map(|(rgb, _)| *rgb));

    let gpu = pm_gpu::device::Gpu::new("CSS gamut WGSL gate").unwrap();
    let device = &gpu.device;
    let queue = &gpu.queue;
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("shared CSS gamut WGSL"),
        source: wgpu::ShaderSource::Wgsl(format!("{}\n{DRIVER}", pm_color::wgsl::CSS_GAMUT).into()),
    });
    let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some("CSS gamut execution"),
        layout: None,
        module: &shader,
        entry_point: Some("main"),
        compilation_options: Default::default(),
        cache: None,
    });
    let mut input_bytes = Vec::with_capacity(samples.len() * 16);
    for rgb in &samples {
        for value in [rgb[0] as f32, rgb[1] as f32, rgb[2] as f32, 1.0] {
            input_bytes.extend_from_slice(&value.to_ne_bytes());
        }
    }
    let input = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("extended sRGB samples"),
        size: input_bytes.len() as u64,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    queue.write_buffer(&input, 0, &input_bytes);
    let output = pm_gpu::Target::new(
        device,
        "CSS gamut float output",
        samples.len() as u32,
        1,
        wgpu::TextureFormat::Rgba32Float,
        wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::COPY_SRC,
    );
    let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("CSS gamut inputs and output"),
        layout: &pipeline.get_bind_group_layout(0),
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: input.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::TextureView(&output.view),
            },
        ],
    });
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("CSS gamut sample execution"),
    });
    {
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("CSS gamut execution"),
            timestamp_writes: None,
        });
        pass.set_pipeline(&pipeline);
        pass.set_bind_group(0, &bind, &[]);
        pass.dispatch_workgroups((samples.len() as u32).div_ceil(64), 1, 1);
    }
    queue.submit([encoder.finish()]);
    let result = pm_gpu::readback::read_f32(device, queue, &output.texture).unwrap();
    assert_eq!(result.len(), samples.len() * 4);

    let mut max_cv = 0.0f64;
    let mut worst = (0usize, 0usize);
    for (index, &rgb) in samples.iter().enumerate() {
        let expected = pm_reference::color::css_gamut_map_srgb(rgb);
        let actual = &result[index * 4..index * 4 + 4];
        assert_eq!(actual[3].to_bits(), 1.0f32.to_bits());
        for channel in 0..3 {
            let value = actual[channel];
            assert!(
                value.is_finite() && (-1e-6..=1.0 + 1e-6).contains(&value),
                "sample {index} {rgb:?} channel {channel}: {value}"
            );
            let cv = (f64::from(value) - expected[channel]).abs() * 255.0;
            if cv > max_cv {
                max_cv = cv;
                worst = (index, channel);
            }
        }
        if index < controls.len() {
            for channel in 0..3 {
                assert_eq!(
                    actual[channel].to_bits(),
                    (rgb[channel] as f32).to_bits(),
                    "in-gamut control {index} changed channel {channel}"
                );
            }
        }
    }
    for (offset, (_, published)) in wpt.iter().enumerate() {
        let index = wpt_start + offset;
        let expected = pm_reference::color::css_gamut_map_srgb(samples[index]);
        for channel in 0..3 {
            assert!(
                (expected[channel] * 255.0 - published[channel]).abs() < 2.0,
                "WPT MINDE case {offset} channel {channel}: reference {expected:?}, published {published:?}"
            );
        }
    }
    eprintln!(
        "CSS gamut WGSL: {} samples, max GPU/reference {:.4} CV at {:?}",
        samples.len(),
        max_cv,
        worst
    );
    assert!(
        max_cv <= 0.5,
        "max GPU/reference {max_cv:.4} CV at sample {} channel {} ({:?})",
        worst.0,
        worst.1,
        samples[worst.0]
    );
}
