//! 3D output with linear blending on (section 9, "3D layers"): the
//! resolved, premultiplied, E-encoded canvas is unpremultiplied (guarded
//! below α = 1/4096, where the pixel is kept), decoded with E⁻¹, and
//! re-premultiplied into a working-format target
//! (`pm_color::linear::premultiplied_e_to_linear`, evaluated in f32 as
//! `pm_color::transfer::display_decode_f32`).

use pm_gpu::{Target, TargetPool};

const LINEAR_WGSL: &str = r#"
@group(0) @binding(0) var src: texture_2d<f32>;
struct Out {
    @builtin(position) pos: vec4<f32>,
};
@vertex
fn vs(@builtin(vertex_index) i: u32) -> Out {
    let a = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    var o: Out;
    o.pos = vec4<f32>(a * 2.0 - 1.0, 0.0, 1.0);
    return o;
}
fn decode(e: f32) -> f32 {
    let m = abs(e);
    var l: f32;
    if (m <= 0.04045) {
        l = m / 12.92;
    } else {
        l = pow((m + 0.055) / 1.055, 2.4);
    }
    return select(l, -l, e < 0.0);
}
@fragment
fn fs(in: Out) -> @location(0) vec4<f32> {
    let p = textureLoad(src, vec2<i32>(in.pos.xy), 0);
    if (p.a < 1.0 / 4096.0) {
        return p;
    }
    let s = p.rgb / p.a;
    return vec4<f32>(vec3<f32>(decode(s.r), decode(s.g), decode(s.b)) * p.a, p.a);
}
"#;

/// The conversion pass, kept across frames.
#[derive(Default)]
pub struct LinearPass {
    pipelines: Vec<(wgpu::TextureFormat, wgpu::RenderPipeline)>,
    layout: Option<wgpu::BindGroupLayout>,
}

impl LinearPass {
    fn pipeline(
        &mut self,
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
    ) -> wgpu::RenderPipeline {
        if let Some((_, p)) = self.pipelines.iter().find(|(f, _)| *f == format) {
            return p.clone();
        }
        let layout = self
            .layout
            .get_or_insert_with(|| {
                device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                    label: Some("3d linear"),
                    entries: &[wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float { filterable: false },
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled: false,
                        },
                        count: None,
                    }],
                })
            })
            .clone();
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("3d linear"),
            source: wgpu::ShaderSource::Wgsl(LINEAR_WGSL.into()),
        });
        let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("3d linear"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let p = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("3d linear"),
            layout: Some(&pl),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vs"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fs"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        self.pipelines.push((format, p.clone()));
        p
    }

    /// Converts `source` (premultiplied E) into a new `format` target of
    /// the same size and row order (premultiplied linear light).
    pub fn convert(
        &mut self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        pool: &mut TargetPool,
        source: &Target,
        format: wgpu::TextureFormat,
    ) -> Target {
        let pipeline = self.pipeline(device, format);
        let out = pool.take(
            device,
            source.width(),
            source.height(),
            format,
            wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
        );
        let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("3d linear"),
            layout: self.layout.as_ref().expect("layout"),
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&source.view),
            }],
        });
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("3d linear"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &out.view,
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
        drop(pass);
        out
    }
}

/// The CPU reference of the conversion of one 8-bit pixel.
pub fn reference(rgba8: [u8; 4]) -> [f64; 4] {
    let p = rgba8.map(|c| f64::from(c) / 255.0);
    pm_color::linear::premultiplied_e_to_linear(p)
}
