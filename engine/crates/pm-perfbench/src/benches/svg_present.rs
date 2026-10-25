//! Paired SVG routes through one GPU presentation and readback path. This
//! diagnostic stays outside the playback row's counter and wall scope.

use std::time::Instant;

use pm_gpu::bridge_out::display::WorkingTexture;
use pm_protocol::color::Precision;
use pm_protocol::common::Size;
use pm_render::vector::cache::VectorCache;
use pm_render::vector::{VectorQuad, VectorSource, svg};
use pm_vector::kurbo::{Affine, Point};
use pm_vector::render::scene::PixelRect;
use pm_vector::render::svg::SvgDocument;
use serde_json::{Value, json};

use super::{Ctx, VIEW};
use crate::fixtures::Fixture;

const PRESENT_WGSL: &str = r#"
struct Uniform { rect: vec4<i32> };
@group(0) @binding(0) var image: texture_2d<f32>;
@group(0) @binding(1) var<uniform> u: Uniform;

@vertex fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    let p = array<vec2<f32>, 3>(vec2<f32>(-1.0, -1.0), vec2<f32>(3.0, -1.0), vec2<f32>(-1.0, 3.0));
    return vec4<f32>(p[i], 0.0, 1.0);
}

@fragment fn fs(@builtin(position) p: vec4<f32>) -> @location(0) vec4<f32> {
    let bg = vec4<f32>(20.0 / 255.0, 24.0 / 255.0, 33.0 / 255.0, 1.0);
    let q = vec2<i32>(i32(floor(p.x)), i32(floor(p.y))) - u.rect.xy;
    if (q.x < 0 || q.y < 0 || q.x >= u.rect.z || q.y >= u.rect.w) {
        return bg;
    }
    let src = textureLoad(image, q, 0);
    return src + bg * (1.0 - src.a);
}
"#;

struct Presented {
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    uniform: wgpu::Buffer,
    target: WorkingTexture,
    readback: pm_gpu::readback::Readback,
}

#[derive(Clone, Copy)]
struct PixelProof {
    checksum: u64,
    center: [f32; 4],
}

impl Presented {
    fn new(ctx: &Ctx) -> Self {
        let device = &ctx.gpu.device;
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("svg bench matched presentation"),
            source: wgpu::ShaderSource::Wgsl(PRESENT_WGSL.into()),
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("svg bench matched source"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
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
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("svg bench matched presentation"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("svg bench matched presentation"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs"),
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
        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("svg bench matched rect"),
            size: 16,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Self {
            pipeline,
            layout,
            uniform,
            target: WorkingTexture::new(device, Size::new(VIEW.0, VIEW.1)),
            readback: pm_gpu::readback::Readback::default(),
        }
    }

    fn present(&mut self, ctx: &Ctx, source: &wgpu::TextureView, rect: PixelRect) -> PixelProof {
        let device = &ctx.gpu.device;
        let queue = &ctx.gpu.queue;
        let uniform = [rect.x, rect.y, rect.width as i32, rect.height as i32];
        // SAFETY: an i32 array has no padding and all its bytes are initialized.
        let bytes = unsafe {
            std::slice::from_raw_parts(uniform.as_ptr().cast::<u8>(), size_of_val(&uniform))
        };
        queue.write_buffer(&self.uniform, 0, bytes);
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("svg bench matched source"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(source),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: self.uniform.as_entire_binding(),
                },
            ],
        });
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("svg bench matched presentation"),
        });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("svg bench matched presentation"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.target.view,
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
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &group, &[]);
            pass.draw(0..3, 0..1);
        }
        queue.submit([encoder.finish()]);
        let cx = (rect.x + rect.width as i32 / 2).clamp(0, VIEW.0 as i32 - 1) as usize;
        let cy = (rect.y + rect.height as i32 / 2).clamp(0, VIEW.1 as i32 - 1) as usize;
        self.readback
            .with_mapped_rows(device, queue, &self.target.texture, |data, _, padded, _| {
                let mut center = [0.0; 4];
                let at = cy * padded + cx * 8;
                for (i, v) in center.iter_mut().enumerate() {
                    let h = u16::from_le_bytes([data[at + i * 2], data[at + i * 2 + 1]]);
                    *v = pm_gpu::readback::half_to_f32(h);
                }
                let checksum = data
                    .chunks(64)
                    .fold(0u64, |sum, chunk| sum.wrapping_add(u64::from(chunk[0])));
                PixelProof { checksum, center }
            })
            .expect("matched GPU readback")
    }
}

fn upload_resvg(ctx: &Ctx, rect: PixelRect, rgba: &[u8]) -> wgpu::Texture {
    let texture = ctx.gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("svg bench resvg upload"),
        size: wgpu::Extent3d {
            width: rect.width,
            height: rect.height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::COPY_DST
            | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    ctx.gpu.queue.write_texture(
        texture.as_image_copy(),
        rgba,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(rect.width * 4),
            rows_per_image: Some(rect.height),
        },
        wgpu::Extent3d {
            width: rect.width,
            height: rect.height,
            depth_or_array_layers: 1,
        },
    );
    texture
}

fn p50(samples: &mut [f64]) -> f64 {
    samples.sort_by(f64::total_cmp);
    samples[samples.len() / 2]
}

/// Same source transform, image rectangle, stage texture, presentation
/// shader and readback for both paths. Sixteen paired warm frames alternate
/// order so the second route does not always inherit the quiet GPU interval.
pub(super) fn compare(ctx: &Ctx, fixture: &Fixture, doc: &SvgDocument) -> Value {
    let mut cache = VectorCache::new();
    let mut presented = Presented::new(ctx);
    let mut native_ms = Vec::with_capacity(16);
    let mut resvg_ms = Vec::with_capacity(16);
    let mut cold_first_fill_ms = None;
    let mut native_proof = PixelProof {
        checksum: 0,
        center: [0.0; 4],
    };
    let mut resvg_proof = native_proof;
    let path = &fixture.media["art"];
    let stage_scale = f64::from(VIEW.0) / 640.0;
    let stage_y = ((f64::from(VIEW.1) - 360.0 * stage_scale) / 2.0).round();
    let target = PixelRect::new(0, 0, VIEW.0, VIEW.1);
    let mut before = (0, 0);
    for k in 0..18 {
        cache.begin_frame();
        let time = f64::from(k) / 30.0;
        let x = 260.0 + 60.0 * time;
        let world = [
            stage_scale,
            0.0,
            0.0,
            stage_scale,
            x * stage_scale,
            180.0 * stage_scale + stage_y,
        ];
        let quad = svg::plan_svg(path, 180.0, 101.25, [0.0, 0.0, 1.0, 1.0], &world, target)
            .expect("SVG plan")
            .expect("visible SVG");
        let VectorQuad::Device { source, .. } = *quad;
        let VectorSource::SvgRaster {
            transform,
            rect,
            clip,
            ..
        } = &source
        else {
            unreachable!("SVG vector source")
        };
        let (transform, rect) = (Affine::new(*transform), *rect);
        let clip: [Point; 4] = (*clip).map(|p| Point::new(p[0], p[1]));

        let native = |presented: &mut Presented, cache: &mut VectorCache| {
            let start = Instant::now();
            let view = cache
                .resolve(&ctx.gpu.device, &ctx.gpu.queue, &source, Precision::Float16)
                .expect("native SVG resolve");
            let proof = presented.present(ctx, &view, rect);
            (start.elapsed().as_secs_f64() * 1e3, proof)
        };
        let resvg = |presented: &mut Presented| {
            let start = Instant::now();
            let raster = doc.raster_device(transform, rect, Some(&clip));
            assert_eq!((raster.width, raster.height), (rect.width, rect.height));
            assert_eq!(
                raster.rgba.len(),
                rect.width as usize * rect.height as usize * 4
            );
            assert!(
                raster.rgba.chunks_exact(4).any(|p| p[3] > 0),
                "resvg drew pixels"
            );
            let texture = upload_resvg(ctx, rect, &raster.rgba);
            let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
            let proof = presented.present(ctx, &view, rect);
            let source_at = ((rect.height / 2 * rect.width + rect.width / 2) * 4) as usize;
            let alpha = f32::from(raster.rgba[source_at + 3]) / 255.0;
            let bg = [20.0 / 255.0, 24.0 / 255.0, 33.0 / 255.0];
            for (i, background) in bg.into_iter().enumerate() {
                let expected =
                    f32::from(raster.rgba[source_at + i]) / 255.0 + background * (1.0 - alpha);
                assert!(
                    (proof.center[i] - expected).abs() < 0.002,
                    "resvg GPU presentation pixel {i}"
                );
            }
            (start.elapsed().as_secs_f64() * 1e3, proof)
        };

        if k % 2 == 0 {
            let (n, np) = native(&mut presented, &mut cache);
            let (r, rp) = resvg(&mut presented);
            if k == 0 {
                assert_eq!(cache.svg_raster_cache_counts(), (0, 1));
                cold_first_fill_ms = Some((n, r));
            } else if k >= 2 {
                native_ms.push(n);
                resvg_ms.push(r);
            }
            native_proof = np;
            resvg_proof = rp;
        } else {
            let (r, rp) = resvg(&mut presented);
            let (n, np) = native(&mut presented, &mut cache);
            if k >= 2 {
                native_ms.push(n);
                resvg_ms.push(r);
            }
            native_proof = np;
            resvg_proof = rp;
        }
        if k == 1 {
            before = cache.svg_raster_cache_counts();
        }
    }
    let after = cache.svg_raster_cache_counts();
    let (hits, misses) = (after.0 - before.0, after.1 - before.1);
    assert_eq!(hits, 16, "each warm transform reuses the SVG source raster");
    assert_eq!(misses, 0, "warm SVG source raster was rebuilt");
    assert_ne!(
        native_proof.checksum, 0,
        "native presentation produced pixels"
    );
    assert_ne!(
        resvg_proof.checksum, 0,
        "resvg presentation produced pixels"
    );
    let (cold_native_ms, cold_resvg_ms) = cold_first_fill_ms.expect("cold pair measured");
    json!({
        "timing_only": true,
        "cold_first_fill": {
            "native_ms": cold_native_ms,
            "resvg_ms": cold_resvg_ms,
            "order": "native_then_resvg",
            "source_raster_hits": 0,
            "source_raster_misses": 1,
        },
        "native_p50_ms": p50(&mut native_ms),
        "resvg_p50_ms": p50(&mut resvg_ms),
        "frames": 16,
        "alternating_order": true,
        "same_stage_pixels": [VIEW.0, VIEW.1],
        "source_raster_hits": hits,
        "source_raster_misses": misses,
        "gpu_readback_proof": {
            "native_checksum": native_proof.checksum,
            "resvg_checksum": resvg_proof.checksum,
            "resvg_center": resvg_proof.center,
        },
    })
}
