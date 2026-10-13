//! Passthrough MSL with an explicit Metal math mode (`pm_gpu::msl`): safe
//! math keeps IEEE semantics that fast math drops. A fragment computes
//! `isnan(0 / 0)` from uniforms the compiler cannot see; safe math must
//! report NaN.
#![cfg(feature = "glsl")]

use pm_gpu::glsl::naga;
use pm_gpu::msl::{self, MathMode, Slot};

const SHADER: &str = r#"
struct U { a: f32, b: f32, pad0: f32, pad1: f32 };
@group(0) @binding(0) var<uniform> u: U;
@vertex
fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    let p = vec2<f32>(f32(i & 1u) * 4.0 - 1.0, f32(i >> 1u) * 4.0 - 1.0);
    return vec4<f32>(p, 0.0, 1.0);
}
@fragment
fn fs() -> @location(0) vec4<f32> {
    let q = u.a / u.b;
    let nan = select(0.0, 1.0, q != q);
    return vec4<f32>(nan, 0.0, 0.0, 1.0);
}
"#;

fn run(gpu: &pm_gpu::Gpu, mode: MathMode) -> (f32, String) {
    let device = &gpu.device;
    let module = naga::front::wgsl::parse_str(SHADER).unwrap();
    let info = naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::all(),
    )
    .validate(&module)
    .unwrap();
    let slots = [Slot {
        group: 0,
        binding: 0,
        buffer: Some(0),
        texture: None,
        sampler: None,
    }];
    let vs = msl::module(
        device,
        "vs",
        &module,
        &info,
        naga::ShaderStage::Vertex,
        &slots,
        mode,
    )
    .unwrap();
    let fs = msl::module(
        device,
        "fs",
        &module,
        &info,
        naga::ShaderStage::Fragment,
        &slots,
        mode,
    )
    .unwrap();
    let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: None,
        entries: &[wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::FRAGMENT | wgpu::ShaderStages::VERTEX,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        }],
    });
    let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: None,
        bind_group_layouts: &[Some(&layout)],
        immediate_size: 0,
    });
    let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: None,
        layout: Some(&pl),
        vertex: wgpu::VertexState {
            module: &vs.module,
            entry_point: Some(&vs.entry_point),
            compilation_options: Default::default(),
            buffers: &[],
        },
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        fragment: Some(wgpu::FragmentState {
            module: &fs.module,
            entry_point: Some(&fs.entry_point),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::TextureFormat::R32Float.into())],
        }),
        multiview_mask: None,
        cache: None,
    });
    let buffer = wgpu::util::DeviceExt::create_buffer_init(
        device,
        &wgpu::util::BufferInitDescriptor {
            label: None,
            contents: &[0u8; 16],
            usage: wgpu::BufferUsages::UNIFORM,
        },
    );
    let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: None,
        layout: &layout,
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: buffer.as_entire_binding(),
        }],
    });
    let target = pm_gpu::Target::new(
        device,
        "nan",
        1,
        1,
        wgpu::TextureFormat::R32Float,
        wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
    );
    let mut encoder = device.create_command_encoder(&Default::default());
    {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: None,
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &target.view,
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
        pass.set_pipeline(&pipeline);
        pass.set_bind_group(0, &bind, &[]);
        pass.draw(0..3, 0..1);
    }
    gpu.queue.submit([encoder.finish()]);
    let v = pm_gpu::readback::read_f32(device, &gpu.queue, &target.texture).unwrap();
    (v[0], fs.source)
}

#[test]
fn safe_math_keeps_nan() {
    let gpu = pm_gpu::Gpu::new("pm-gpu msl test").expect("a Metal device");
    assert!(
        msl::supported(&gpu.device),
        "the engine device has passthrough shaders"
    );
    let (safe, source) = run(&gpu, MathMode::Safe);
    assert!(source.starts_with("#pragma METAL fp math_mode(safe)\n"));
    assert_eq!(safe, 1.0, "safe math: 0 / 0 is NaN");
    let (fast, _) = run(&gpu, MathMode::Fast);
    eprintln!("isnan(0 / 0): safe {safe}, fast {fast}");
}
