//! How the device stores an `f32` fragment output into an `rgba16float`
//! target. Today's compositor writes every intermediate pass to such a
//! target on the same GPU, so its frames carry that GPU's rounding; the
//! engine's fused composite steps (`half` in `ops.wgsl`) reproduce the
//! rounding the device actually performs instead of assuming one
//! (docs/engine/composite.md "Today's composite drift"; measured toward zero
//! on Apple M-series).

/// Whether the device rounds to nearest (else toward zero), from one
/// probe draw: 0.75 + ¾ of an f16 ulp stores as 0.75 when truncated and as
/// the next half when rounded.
pub fn rounds_to_nearest(device: &wgpu::Device, queue: &wgpu::Queue) -> bool {
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("pm half store probe"),
        source: wgpu::ShaderSource::Wgsl(
            r#"
@group(0) @binding(0) var<uniform> v: vec4<f32>;
@vertex fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
  let p = array(vec2(-1.0, -1.0), vec2(3.0, -1.0), vec2(-1.0, 3.0));
  return vec4(p[i], 0.0, 1.0);
}
@fragment fn fs() -> @location(0) vec4<f32> { return v; }
"#
            .into(),
        ),
    });
    let value = 0.75f32 + 0.75 / 2048.0;
    let uniform = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("pm half store probe"),
        size: 16,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    queue.write_buffer(&uniform, 0, crate::pipelines::bytes_of(&[value; 4]));
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("pm half store probe"),
        size: wgpu::Extent3d {
            width: 1,
            height: 1,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba16Float,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("pm half store probe"),
        layout: None,
        vertex: wgpu::VertexState {
            module: &module,
            entry_point: Some("vs"),
            buffers: &[],
            compilation_options: Default::default(),
        },
        fragment: Some(wgpu::FragmentState {
            module: &module,
            entry_point: Some("fs"),
            targets: &[Some(wgpu::ColorTargetState {
                format: wgpu::TextureFormat::Rgba16Float,
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
            })],
            compilation_options: Default::default(),
        }),
        primitive: Default::default(),
        depth_stencil: None,
        multisample: Default::default(),
        multiview_mask: None,
        cache: None,
    });
    let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("pm half store probe"),
        layout: &pipeline.get_bind_group_layout(0),
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: uniform.as_entire_binding(),
        }],
    });
    let out = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("pm half store probe"),
        size: 256,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let view = texture.create_view(&Default::default());
    let mut encoder = device.create_command_encoder(&Default::default());
    {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("pm half store probe"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    store: wgpu::StoreOp::Store,
                },
            })],
            ..Default::default()
        });
        pass.set_pipeline(&pipeline);
        pass.set_bind_group(0, &bind, &[]);
        pass.draw(0..3, 0..1);
    }
    encoder.copy_texture_to_buffer(
        texture.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &out,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(256),
                rows_per_image: None,
            },
        },
        texture.size(),
    );
    queue.submit([encoder.finish()]);
    let slice = out.slice(..);
    slice.map_async(wgpu::MapMode::Read, |_| {});
    let _ = device.poll(wgpu::PollType::wait_indefinitely());
    let Ok(data) = slice.get_mapped_range() else {
        return false;
    };
    // 0.75 is 0x3A00 in f16; the next half up is 0x3A01.
    u16::from_le_bytes([data[0], data[1]]) == 0x3A01
}
