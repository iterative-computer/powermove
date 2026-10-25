//! Legacy Graphite RGBA blur for text save layers only.
//! Extracted from the former path-mask blur executor; masks use the new path.

use crate::pipelines::bytes_of;
use pm_vector::canvas2d::filter::{BlurPlan, Map, Pass, Rect};
use std::mem::size_of;

/// `Pass` in `legacy_blur.wgsl`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
struct PassUniform {
    dst_org: [f32; 2],
    src_org: [f32; 2],
    src_inv: [f32; 2],
    n: u32,
    linear: u32,
    map_s: [f32; 2],
    map_t: [f32; 2],
    dir: [f32; 2],
    mode: [u32; 2],
    k: [[f32; 4]; 7],
    o: [[f32; 4]; 14],
}

const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

pub(crate) struct LegacyTextBlur {
    format: wgpu::TextureFormat,
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    rescale: crate::pipelines::LazyRenderPipeline,
    blur1d: crate::pipelines::LazyRenderPipeline,
    blur2d: crate::pipelines::LazyRenderPipeline,
    over: crate::pipelines::LazyRenderPipeline,
}

impl LegacyTextBlur {
    pub(crate) fn new(device: &wgpu::Device) -> LegacyTextBlur {
        Self::with_format(device, FORMAT)
    }
    fn with_format(device: &wgpu::Device, format: wgpu::TextureFormat) -> LegacyTextBlur {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("pm-render text blur"),
            source: wgpu::ShaderSource::Wgsl(include_str!("legacy_blur.wgsl").into()),
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("pm text blurs"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: wgpu::BufferSize::new(size_of::<PassUniform>() as u64),
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("pm text blurs"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = |entry: &'static str, blend: Option<wgpu::BlendState>| {
            let device = device.clone();
            let module = module.clone();
            let pipeline_layout = pipeline_layout.clone();
            crate::pipelines::LazyRenderPipeline::new(move || {
                device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                    label: Some(entry),
                    layout: Some(&pipeline_layout),
                    vertex: wgpu::VertexState {
                        module: &module,
                        entry_point: Some("vs"),
                        compilation_options: Default::default(),
                        buffers: &[],
                    },
                    primitive: wgpu::PrimitiveState {
                        topology: wgpu::PrimitiveTopology::TriangleStrip,
                        ..Default::default()
                    },
                    depth_stencil: None,
                    multisample: wgpu::MultisampleState::default(),
                    fragment: Some(wgpu::FragmentState {
                        module: &module,
                        entry_point: Some(entry),
                        compilation_options: Default::default(),
                        targets: &[Some(wgpu::ColorTargetState {
                            format,
                            blend,
                            write_mask: wgpu::ColorWrites::ALL,
                        })],
                    }),
                    multiview_mask: None,
                    cache: None,
                })
            })
        };
        let blend = |src: wgpu::BlendFactor| wgpu::BlendState {
            color: wgpu::BlendComponent {
                src_factor: src,
                dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                operation: wgpu::BlendOperation::Add,
            },
            alpha: wgpu::BlendComponent {
                src_factor: src,
                dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                operation: wgpu::BlendOperation::Add,
            },
        };
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("pm text blurs bilinear"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        LegacyTextBlur {
            format,
            rescale: pipeline("fs_rescale", None),
            blur1d: pipeline("fs_blur1d", None),
            blur2d: pipeline("fs_blur2d", None),
            over: pipeline("fs_draw", Some(blend(wgpu::BlendFactor::One))),
            layout,
            sampler,
        }
    }

    #[cfg(test)]
    pub(crate) fn built_count(&self) -> usize {
        [&self.rescale, &self.blur1d, &self.blur2d, &self.over]
            .into_iter()
            .filter(|pipeline| pipeline.is_built())
            .count()
    }

    #[cfg(test)]
    pub(crate) fn probe_all(&self) -> Vec<(String, std::time::Duration)> {
        [
            ("rescale", &self.rescale),
            ("blur1d", &self.blur1d),
            ("blur2d", &self.blur2d),
            ("over", &self.over),
        ]
        .into_iter()
        .map(|(name, pipeline)| {
            let start = std::time::Instant::now();
            pipeline.get();
            (
                format!("text blur {name} {:?}", self.format),
                start.elapsed(),
            )
        })
        .collect()
    }
}

/// An image on the GPU: `rect` in its space, with a transparent 1-pixel
/// border (`ring`), or the bare canvas.
struct Image {
    texture: wgpu::Texture,
    rect: Rect,
    ring: i32,
}

impl Image {
    fn new(device: &wgpu::Device, rect: Rect, ring: i32, format: wgpu::TextureFormat) -> Image {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("pm text blur image"),
            size: wgpu::Extent3d {
                width: (rect.w + 2 * ring as u32).max(1),
                height: (rect.h + 2 * ring as u32).max(1),
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::COPY_DST
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        Image {
            texture,
            rect,
            ring,
        }
    }
    /// The space coordinate of texel (0, 0)'s corner.
    fn origin(&self) -> [f32; 2] {
        [
            (self.rect.x0 - self.ring) as f32,
            (self.rect.y0 - self.ring) as f32,
        ]
    }
    fn inv_size(&self) -> [f32; 2] {
        let s = self.texture.size();
        [1.0 / s.width as f32, 1.0 / s.height as f32]
    }
}

impl LegacyTextBlur {
    #[allow(clippy::too_many_arguments)]
    fn pass(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        pipeline: &wgpu::RenderPipeline,
        u: &PassUniform,
        src: &Image,
        target: &wgpu::TextureView,
        load: wgpu::LoadOp<wgpu::Color>,
        viewport: [f32; 4],
    ) {
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("pm text blur pass"),
            size: size_of::<PassUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let mut u = *u;
        u.mode[0] = 1;
        queue.write_buffer(&buffer, 0, bytes_of(&u));
        let view = src.texture.create_view(&Default::default());
        let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("pm text blur pass"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
            ],
        });
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("pm text blur pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        let [x, y, w, h] = viewport;
        if w > 0.0 && h > 0.0 {
            pass.set_viewport(x, y, w, h, 0.0, 1.0);
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, &bind, &[]);
            pass.draw(0..4, 0..1);
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn blur_rgba(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        src: &pm_gpu::Target,
        origin: [i32; 2],
        canvas: (u32, u32),
        crop: [u32; 4],
        sigma: f32,
    ) -> pm_gpu::Target {
        assert_eq!(self.format, wgpu::TextureFormat::Rgba8Unorm);
        let mut img = Image {
            texture: src.texture.clone(),
            ring: 0,
            rect: Rect {
                x0: origin[0],
                y0: origin[1],
                w: src.width(),
                h: src.height(),
            },
        };
        let plan = BlurPlan::new(canvas.0, canvas.1, sigma);
        let (draw, linear) = if let Some(plan) = plan {
            for pass in &plan.passes {
                img = self.run_pass(device, queue, encoder, pass, &img);
            }
            (plan.draw, plan.linear)
        } else {
            (
                Map {
                    scale: [1.0; 2],
                    translate: [0.0; 2],
                },
                false,
            )
        };
        let out = Image::new(
            device,
            Rect::ltrb(
                crop[0] as i32,
                crop[1] as i32,
                crop[2] as i32,
                crop[3] as i32,
            ),
            0,
            self.format,
        );
        let u = PassUniform {
            src_org: img.origin(),
            src_inv: img.inv_size(),
            linear: u32::from(linear),
            map_s: draw.scale,
            map_t: [
                draw.translate[0] + crop[0] as f32 * draw.scale[0],
                draw.translate[1] + crop[1] as f32 * draw.scale[1],
            ],
            ..Default::default()
        };
        let view = out.texture.create_view(&Default::default());
        self.pass(
            device,
            queue,
            encoder,
            self.over.get(),
            &u,
            &img,
            &view,
            wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
            [
                0.0,
                0.0,
                (crop[2] - crop[0]) as f32,
                (crop[3] - crop[1]) as f32,
            ],
        );
        pm_gpu::Target::wrap(out.texture)
    }

    fn run_pass(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        pass: &Pass,
        src: &Image,
    ) -> Image {
        let dst = pass.dst();
        let out = Image::new(device, dst, 1, self.format);
        let mut u = PassUniform {
            dst_org: out.origin(),
            src_org: src.origin(),
            src_inv: src.inv_size(),
            ..Default::default()
        };
        let pipeline = match pass {
            Pass::Rescale { sample, .. } => {
                u.map_s = sample.scale;
                u.map_t = sample.translate;
                self.rescale.get()
            }
            Pass::Blur1d { dir, taps, .. } => {
                u.dir = *dir;
                u.n = (taps.len() / 2) as u32;
                for (i, pair) in taps.chunks(2).enumerate() {
                    u.o[i] = [pair[0].0, pair[0].1, pair[1].0, pair[1].1];
                }
                self.blur1d.get()
            }
            Pass::Blur2d {
                weights, offsets, ..
            } => {
                u.n = (weights.len() / 4) as u32;
                for (i, k) in weights.chunks(4).enumerate() {
                    u.k[i] = [k[0], k[1], k[2], k[3]];
                }
                for (i, o) in offsets.chunks(2).enumerate() {
                    u.o[i] = [o[0][0], o[0][1], o[1][0], o[1][1]];
                }
                self.blur2d.get()
            }
        };
        let view = out.texture.create_view(&Default::default());
        self.pass(
            device,
            queue,
            encoder,
            pipeline,
            &u,
            src,
            &view,
            wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
            [1.0, 1.0, dst.w as f32, dst.h as f32],
        );
        out
    }
}
