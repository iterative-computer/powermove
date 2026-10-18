//! How this GPU stores f32 fragment outputs into `rgba16float` targets:
//! rounded toward zero. Today's compositor (ANGLE on Metal) stores every
//! intermediate pass the same way, and the engine's composite reproduces
//! today's frames because of it (docs/engine/composite.md, "Today's
//! composite drift"; `half_rtz` in `composite/ops.wgsl`). If a GPU or
//! driver rounds to nearest instead, this test says so.
use pm_render::Renderer;

#[test]
fn rgba16float_targets_store_fragment_outputs_toward_zero() {
    let r = Renderer::headless().unwrap();
    let device = r.device();
    let queue = r.queue();
    let src = r#"
@group(0) @binding(0) var<storage, read> vals: array<vec4<f32>>;
@vertex fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
  let p = array(vec2(-1.0,-1.0), vec2(3.0,-1.0), vec2(-1.0,3.0));
  return vec4(p[i], 0.0, 1.0);
}
@fragment fn fs(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
  let i = u32(pos.x);
  return vals[i];
}
"#;
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: None,
        source: wgpu::ShaderSource::Wgsl(src.into()),
    });
    let n = 64u32;
    let mut vals: Vec<f32> = Vec::new();
    let tests: Vec<f32> = (0..n)
        .map(|k| {
            let base = 0.75f32 + (k as f32) * 0.0004882812;
            // one f32 ulp below / above an f16 value, and halfway-ish
            match k % 4 {
                0 => f32::from_bits(base.to_bits() - 1),
                1 => f32::from_bits(base.to_bits() + 1),
                2 => base + 0.000244140625 - 1e-7,
                _ => base + 0.000244140625 + 1e-7,
            }
        })
        .collect();
    for t in &tests {
        vals.extend_from_slice(&[*t, *t, *t, 1.0]);
    }
    use wgpu::util::DeviceExt;
    let buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: None,
        contents: bytemuck_cast(&vals),
        usage: wgpu::BufferUsages::STORAGE,
    });
    let tex = device.create_texture(&wgpu::TextureDescriptor {
        label: None,
        size: wgpu::Extent3d {
            width: n,
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
        label: None,
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
        label: None,
        layout: &pipeline.get_bind_group_layout(0),
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: buf.as_entire_binding(),
        }],
    });
    let view = tex.create_view(&Default::default());
    let out = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: (n * 8) as u64,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut enc = device.create_command_encoder(&Default::default());
    {
        let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: None,
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
    enc.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture: &tex,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &out,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(256.max(n * 8)),
                rows_per_image: None,
            },
        },
        wgpu::Extent3d {
            width: n,
            height: 1,
            depth_or_array_layers: 1,
        },
    );
    queue.submit([enc.finish()]);
    let slice = out.slice(..);
    slice.map_async(wgpu::MapMode::Read, |_| {});
    device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
    let data = slice.get_mapped_range().unwrap();
    let mut rtz = 0;
    let mut rne = 0;
    for k in 0..n as usize {
        let h = u16::from_le_bytes([data[k * 8], data[k * 8 + 1]]);
        let got = half_to_f32(h);
        let t = tests[k];
        let lo = f16_floor(t);
        let hi = lo + 0.00048828125;
        let nearest = if (t - lo) < (hi - t) { lo } else { hi };
        if got == lo && nearest != lo {
            rtz += 1;
        }
        if got == nearest {
            rne += 1;
        }
        assert_eq!(got, lo, "{t:.9} stored as {got:.9} (toward zero: {lo:.9})");
    }
    // Half the probes lie above the midpoint between two halves: rounding to
    // nearest would store the upper one.
    assert_eq!(rtz, n as usize / 2, "rne {rne} rtz-only {rtz} of {n}");
}

fn f16_floor(t: f32) -> f32 {
    (t / 0.00048828125).floor() * 0.00048828125
}

fn half_to_f32(h: u16) -> f32 {
    let s = (h >> 15) as u32;
    let e = ((h >> 10) & 0x1f) as u32;
    let m = (h & 0x3ff) as u32;
    let bits = if e == 0 {
        0
    } else {
        (s << 31) | ((e + 112) << 23) | (m << 13)
    };
    f32::from_bits(bits)
}

fn bytemuck_cast(v: &[f32]) -> &[u8] {
    unsafe { std::slice::from_raw_parts(v.as_ptr() as *const u8, v.len() * 4) }
}
