//! The SVG playback resampler: a persistent float source raster, painted
//! once per content, scale bucket and image crop, reprojected at each
//! playback frame's exact layer transform. Translation and rotation reuse
//! the raster; a scale beyond the bucket repaints it. Paused frames and
//! exports never come here: they paint at output resolution.

use pm_protocol::color::Precision;
use pm_vector::kurbo::{Affine, Point, Rect};
use pm_vector::render::scene::PixelRect;
use pm_vector::render::svg::SvgDocument;
use sha2::{Digest, Sha256};

use super::svg::Stamp;

/// A source raster stays a small share of the vector cache budget; larger
/// playback layers paint directly at output resolution.
pub const MAX_RASTER_BYTES: u64 = 32 * 1024 * 1024;
/// The source raster's precision (RGBA16F, 8 bytes per pixel).
pub const SOURCE_PRECISION: Precision = Precision::Float16;
/// Source pixels per output pixel along the transform's largest axis, at
/// least. Upward √2 buckets keep the density in [2, 2√2).
const DENSITY: f64 = 2.0;
const BUCKET: f64 = std::f64::consts::SQRT_2;
/// A fragment integrates its pixel's source footprint with a 4 × 4
/// quadrature of the bilinear raster, which stays below one texel spacing
/// for footprints up to 4.5 texels per axis (the √2 bucket at any
/// rotation). A transform that minifies one axis further paints directly.
const MAX_FOOTPRINT_TEXELS: f64 = 4.5;

pub struct RasterPlan {
    pub key: [u8; 32],
    pub rect: PixelRect,
    pub scale: f64,
    /// The image-box crop in source raster coordinates, painted together
    /// with SVG coverage before the raster enters the cache.
    pub source_clip: [Point; 4],
}

/// Plan the source raster of `doc` under `transform` with its image-box
/// crop `clip` (device space) baked in. The crop is mapped back to SVG user
/// space, so the identity is stable under translation and rotation while a
/// changed crop or scale bucket gets a different key.
pub fn plan_with_clip(
    doc: &SvgDocument,
    path: &std::path::Path,
    stamp: Stamp,
    transform: Affine,
    clip: &[Point; 4],
    limits: &wgpu::Limits,
) -> Option<RasterPlan> {
    let [a, b, c, d, _, _] = transform.as_coeffs();
    let trace = a * a + b * b + c * c + d * d;
    let det = a * d - b * c;
    let discriminant = (trace * trace - 4.0 * det * det).max(0.0);
    let max_scale = ((trace + discriminant.sqrt()) * 0.5).sqrt();
    let inverse = transform.inverse();
    if !max_scale.is_finite()
        || max_scale <= 0.0
        || !inverse.as_coeffs().iter().all(|v| v.is_finite())
    {
        return None;
    }
    let user_clip = clip.map(|point| {
        let p = inverse * point;
        // Inverting a moving/rotating transform adds a few ulps of noise
        // (including negative zero). A 10-nanounit snap keeps one source
        // window under motion without moving authored placement.
        let snap = |v: f64| {
            let q = (v * 1e8).round() / 1e8;
            if q == 0.0 { 0.0 } else { q }
        };
        Point::new(snap(p.x), snap(p.y))
    });
    if !user_clip.iter().all(|p| p.x.is_finite() && p.y.is_finite()) {
        return None;
    }
    let bounds = doc.user_bounds()?;
    let bucket = ((max_scale * DENSITY).ln() / BUCKET.ln() - 1e-8).ceil() as i32;
    let scale = BUCKET.powi(bucket);
    let rect = PixelRect::round_out(
        Affine::scale(scale)
            .transform_rect_bbox(bounds)
            .inflate(2.0, 2.0),
    );
    let bytes = u64::from(rect.width)
        .checked_mul(u64::from(rect.height))?
        .checked_mul(8)?;
    if rect.width == 0
        || rect.height == 0
        || rect.width > limits.max_texture_dimension_2d
        || rect.height > limits.max_texture_dimension_2d
        || bytes > MAX_RASTER_BYTES
    {
        return None;
    }
    let mut hash = Sha256::new();
    hash.update(b"pm-svg-float-raster/3");
    hash.update(path.as_os_str().as_encoded_bytes());
    hash.update(stamp.0.to_le_bytes());
    hash.update(stamp.1.to_le_bytes());
    hash.update(bucket.to_le_bytes());
    for point in user_clip {
        hash.update(point.x.to_bits().to_le_bytes());
        hash.update(point.y.to_bits().to_le_bytes());
    }
    Some(RasterPlan {
        key: hash.finalize().into(),
        rect,
        scale,
        source_clip: user_clip.map(|point| Affine::scale(scale) * point),
    })
}

pub struct SvgSample {
    layout: wgpu::BindGroupLayout,
    pipelines: [wgpu::RenderPipeline; 3],
    uniform: wgpu::Buffer,
    sampler: wgpu::Sampler,
}

impl SvgSample {
    pub fn new(device: &wgpu::Device) -> Self {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("pm svg raster sample"),
            source: wgpu::ShaderSource::Wgsl(include_str!("svg_sample.wgsl").into()),
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("pm svg raster sample"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
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
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("pm svg raster sample"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let make = |format| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("pm svg raster sample"),
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
            })
        };
        Self {
            layout,
            pipelines: [
                make(wgpu::TextureFormat::Rgba8Unorm),
                make(wgpu::TextureFormat::Rgba16Float),
                make(wgpu::TextureFormat::Rgba32Float),
            ],
            uniform: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("pm svg raster sample params"),
                size: 48,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
            // Clamped edges read the raster's transparent 2-pixel margin.
            sampler: device.create_sampler(&wgpu::SamplerDescriptor {
                label: Some("pm svg raster sample"),
                mag_filter: wgpu::FilterMode::Linear,
                min_filter: wgpu::FilterMode::Linear,
                ..Default::default()
            }),
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn render(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        source: &wgpu::Texture,
        raster: &RasterPlan,
        transform: Affine,
        rect: PixelRect,
        clip: &[Point; 4],
        precision: Precision,
    ) -> Option<(wgpu::Texture, u64)> {
        let limits = device.limits();
        if rect.width.max(1) > limits.max_texture_dimension_2d
            || rect.height.max(1) > limits.max_texture_dimension_2d
            || !clip
                .iter()
                .all(|point| point.x.is_finite() && point.y.is_finite())
        {
            return None;
        }
        if !clip
            .iter()
            .zip(&raster.source_clip)
            .all(|(actual, source)| {
                let expected =
                    transform * Point::new(source.x / raster.scale, source.y / raster.scale);
                (actual.x - expected.x).abs() <= 1e-6 && (actual.y - expected.y).abs() <= 1e-6
            })
        {
            return None;
        }
        let map = Affine::translate((-f64::from(raster.rect.x), -f64::from(raster.rect.y)))
            * Affine::scale(raster.scale)
            * transform.inverse()
            * Affine::translate((f64::from(rect.x), f64::from(rect.y)));
        let [a, b, c, d, tx, ty] = map.as_coeffs();
        if ![a, b, c, d, tx, ty]
            .iter()
            .all(|v| v.is_finite() && v.abs() < 16_000_000.0)
            || a.abs() + c.abs() > MAX_FOOTPRINT_TEXELS
            || b.abs() + d.abs() > MAX_FOOTPRINT_TEXELS
        {
            return None;
        }
        let format = pm_gpu::working_format(precision);
        let (index, bpp) = match format {
            wgpu::TextureFormat::Rgba8Unorm => (0, 4),
            wgpu::TextureFormat::Rgba16Float => (1, 8),
            wgpu::TextureFormat::Rgba32Float => (2, 16),
            _ => return None,
        };
        let mut data = [0f32; 12];
        data[..4].copy_from_slice(&[a as f32, c as f32, tx as f32, 0.0]);
        data[4..8].copy_from_slice(&[b as f32, d as f32, ty as f32, 0.0]);
        data[8..12].copy_from_slice(&[
            raster.rect.width as f32,
            raster.rect.height as f32,
            0.0,
            0.0,
        ]);
        // SAFETY: initialized f32 has no padding.
        let bytes = unsafe { std::slice::from_raw_parts(data.as_ptr().cast::<u8>(), 48) };
        queue.write_buffer(&self.uniform, 0, bytes);
        let source_view = source.create_view(&Default::default());
        let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("pm svg raster sample"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&source_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: self.uniform.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
            ],
        });
        let (w, h) = (rect.width.max(1), rect.height.max(1));
        let output = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("pm svg sampled output"),
            size: wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = output.create_view(&Default::default());
        let source_bounds = Rect::new(
            f64::from(raster.rect.x),
            f64::from(raster.rect.y),
            f64::from(raster.rect.right()),
            f64::from(raster.rect.bottom()),
        );
        let projected = (transform * Affine::scale(1.0 / raster.scale))
            .transform_rect_bbox(source_bounds);
        let visible = PixelRect::round_out(projected).intersect(rect);
        let mut encoder = device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("pm svg raster sample"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            if let Some(visible) = visible {
                pass.set_scissor_rect(
                    (visible.x - rect.x) as u32,
                    (visible.y - rect.y) as u32,
                    visible.width,
                    visible.height,
                );
                pass.set_pipeline(&self.pipelines[index]);
                pass.set_bind_group(0, &bind, &[]);
                pass.draw(0..4, 0..1);
            }
        }
        queue.submit([encoder.finish()]);
        Some((output, u64::from(w) * u64::from(h) * bpp))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc(svg: &str) -> SvgDocument {
        SvgDocument::parse(svg.as_bytes()).unwrap()
    }

    fn corners(transform: Affine, [x0, y0, x1, y1]: [f64; 4]) -> [Point; 4] {
        [(x0, y0), (x1, y0), (x1, y1), (x0, y1)].map(|(x, y)| transform * Point::new(x, y))
    }

    const SQUARE: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10"><rect width="10" height="10" fill="white"/></svg>"##;

    #[test]
    fn raster_identity_tracks_content_scale_and_crop_not_motion() {
        let doc = doc(SQUARE);
        let limits = wgpu::Limits::default();
        let path = std::path::Path::new("test.svg");
        let window = [2.0, 1.0, 8.0, 9.0];
        let key = |stamp, t: Affine, window| {
            plan_with_clip(&doc, path, stamp, t, &corners(t, window), &limits)
                .unwrap()
                .key
        };
        let base = key((1, 1), Affine::scale(2.1), window);
        let moved = Affine::translate((31.25, 19.5)) * Affine::rotate(0.4) * Affine::scale(2.1);
        assert_eq!(base, key((1, 1), moved, window));
        // Within one √2 bucket the raster is shared; beyond it, repainted.
        assert_eq!(base, key((1, 1), Affine::scale(2.6), window));
        assert_ne!(base, key((1, 1), Affine::scale(3.0), window));
        assert_ne!(base, key((2, 1), Affine::scale(2.1), window));
        assert_ne!(base, key((1, 1), moved, [2.0, 1.0, 7.5, 9.0]));
    }

    #[test]
    fn oversized_sources_paint_directly() {
        let doc = doc(SQUARE);
        let t = Affine::scale(1000.0);
        assert!(
            plan_with_clip(
                &doc,
                std::path::Path::new("large.svg"),
                (1, 1),
                t,
                &corners(t, [0.0, 0.0, 10.0, 10.0]),
                &wgpu::Limits::default(),
            )
            .is_none()
        );
    }

    /// Paints the source raster and reprojects it at `transform`.
    fn playback(
        gpu: &pm_gpu::Gpu,
        doc: &SvgDocument,
        transform: Affine,
        clip: [Point; 4],
        size: u32,
        precision: Precision,
    ) -> (Vec<f32>, RasterPlan, Vec<f32>) {
        let plan = plan_with_clip(
            doc,
            std::path::Path::new("playback.svg"),
            (1, 1),
            transform,
            &clip,
            &gpu.device.limits(),
        )
        .unwrap();
        let mut painter = super::super::svg_float::SvgPainter::new(&gpu.device).unwrap();
        let mut encoder = gpu.device.create_command_encoder(&Default::default());
        let (source, _) = painter
            .render(
                &gpu.device,
                &gpu.queue,
                &mut encoder,
                doc,
                Affine::scale(plan.scale),
                plan.rect,
                &plan.source_clip,
                SOURCE_PRECISION,
            )
            .unwrap()
            .unwrap();
        gpu.queue.submit([encoder.finish()]);
        let (output, _) = SvgSample::new(&gpu.device)
            .render(
                &gpu.device,
                &gpu.queue,
                &source,
                &plan,
                transform,
                PixelRect::new(0, 0, size, size),
                &clip,
                precision,
            )
            .unwrap();
        let read = |t| pm_gpu::readback::read_f32(&gpu.device, &gpu.queue, t).unwrap();
        (read(&output), plan, read(&source))
    }

    /// Today's playback look of the same raster: RGBA8 storage sampled
    /// bilinearly at each output pixel center (legacy GL texture draw).
    fn bilinear_rgba8(source: &[f32], plan: &RasterPlan, transform: Affine, size: u32) -> Vec<f64> {
        let (w, h) = (plan.rect.width as i64, plan.rect.height as i64);
        let texel = |x: i64, y: i64| -> f64 {
            if x < 0 || y < 0 || x >= w || y >= h {
                return 0.0;
            }
            (f64::from(source[((y * w + x) * 4 + 3) as usize]) * 255.0).round() / 255.0
        };
        let map = Affine::translate((-f64::from(plan.rect.x), -f64::from(plan.rect.y)))
            * Affine::scale(plan.scale)
            * transform.inverse();
        (0..size * size)
            .map(|i| {
                let q = map * Point::new(f64::from(i % size) + 0.5, f64::from(i / size) + 0.5);
                let (fx, fy) = (q.x - 0.5, q.y - 0.5);
                let (x, y) = (fx.floor() as i64, fy.floor() as i64);
                let (tx, ty) = (fx - fx.floor(), fy - fy.floor());
                let top = texel(x, y) * (1.0 - tx) + texel(x + 1, y) * tx;
                let bottom = texel(x, y + 1) * (1.0 - tx) + texel(x + 1, y + 1) * tx;
                top * (1.0 - ty) + bottom * ty
            })
            .collect()
    }

    /// Alpha from an independent 64 × 64 f64 subpixel grid per pixel.
    fn reference(size: u32, inverse: Affine, inside: impl Fn(Point) -> f64) -> Vec<f64> {
        const GRID: u32 = 64;
        (0..size * size)
            .map(|i| {
                let (x, y) = (f64::from(i % size), f64::from(i / size));
                let mut sum = 0.0;
                for sy in 0..GRID {
                    for sx in 0..GRID {
                        let q = Point::new(
                            x + (f64::from(sx) + 0.5) / f64::from(GRID),
                            y + (f64::from(sy) + 0.5) / f64::from(GRID),
                        );
                        sum += inside(inverse * q);
                    }
                }
                sum / f64::from(GRID * GRID)
            })
            .collect()
    }

    struct Closeness {
        max_cv: f64,
        p999_cv: f64,
        mean_cv: f64,
        area: f64,
        centroid: f64,
    }

    fn closeness(actual: &[f64], reference: &[f64], size: u32) -> Closeness {
        let (mut max, mut sum, mut a, mut r, mut ax, mut rx, mut ay, mut ry) =
            (0.0f64, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0);
        let mut errors: Vec<f64> =
            actual.iter().zip(reference).map(|(p, q)| (p - q).abs()).collect();
        errors.sort_by(f64::total_cmp);
        for (i, (&p, &q)) in actual.iter().zip(reference).enumerate() {
            let (x, y) = (f64::from(i as u32 % size) + 0.5, f64::from(i as u32 / size) + 0.5);
            max = max.max((p - q).abs());
            sum += (p - q).abs();
            (a, r) = (a + p, r + q);
            (ax, rx, ay, ry) = (ax + p * x, rx + q * x, ay + p * y, ry + q * y);
        }
        Closeness {
            max_cv: max * 255.0,
            p999_cv: errors[(errors.len() * 999 / 1000).min(errors.len() - 1)] * 255.0,
            mean_cv: sum / actual.len() as f64 * 255.0,
            area: (a - r).abs() / r,
            centroid: (ax / a - rx / r).abs().max((ay / a - ry / r).abs()),
        }
    }

    /// Playback reprojection of a rotated, jointly cropped circle and a
    /// masked stroke, against f64 coverage. It conserves area and centroid,
    /// and is at least as close everywhere as today's bilinear RGBA8 draw of
    /// the same raster, at every working precision.
    #[test]
    fn playback_frames_are_closer_to_f64_than_todays_bilinear_draw() {
        let gpu = pm_gpu::Gpu::new("SVG playback reference").unwrap();
        let transform = Affine::translate((20.25, 18.5)) * Affine::rotate(0.4) * Affine::scale(4.3);
        let inverse = transform.inverse();
        let crop = [2.0, 1.0, 8.0, 9.0];
        let in_crop = |u: Point| (2.0..=8.0).contains(&u.x) && (1.0..=9.0).contains(&u.y);
        type Inside = Box<dyn Fn(Point) -> f64>;
        let cases: [(&str, &str, Inside); 2] = [
            (
                "circle",
                r##"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10"><circle cx="5" cy="5" r="4.5" fill="white"/></svg>"##,
                Box::new(move |u: Point| f64::from(u8::from(in_crop(u) && (u.x - 5.0).hypot(u.y - 5.0) <= 4.5))),
            ),
            (
                "masked stroke",
                r##"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10"><defs><mask id="m" maskUnits="userSpaceOnUse" maskContentUnits="userSpaceOnUse" x="0" y="0" width="10" height="10"><rect width="6" height="10" fill="white"/></mask></defs><g mask="url(#m)" opacity=".8"><rect x="2" y="2" width="6" height="6" fill="none" stroke="white" stroke-width="1"/></g></svg>"##,
                Box::new(move |u: Point| {
                    let outer = (1.5..=8.5).contains(&u.x) && (1.5..=8.5).contains(&u.y);
                    let inner = (2.5..=7.5).contains(&u.x) && (2.5..=7.5).contains(&u.y);
                    0.8 * f64::from(u8::from(in_crop(u) && outer && !inner && u.x <= 6.0))
                }),
            ),
        ];
        const SIZE: u32 = 64;
        for (name, svg, inside) in cases {
            let doc = doc(svg);
            let wanted = reference(SIZE, inverse, inside);
            for precision in [Precision::Float32, Precision::Float16, Precision::Unorm8] {
                let (output, plan, source) =
                    playback(&gpu, &doc, transform, corners(transform, crop), SIZE, precision);
                let alpha: Vec<f64> = output.chunks_exact(4).map(|p| f64::from(p[3])).collect();
                let engine = closeness(&alpha, &wanted, SIZE);
                let today = closeness(&bilinear_rgba8(&source, &plan, transform, SIZE), &wanted, SIZE);
                eprintln!(
                    "{name} {precision:?}: engine max {:.2} p99.9 {:.2} mean {:.3} CV area {:.1e} centroid {:.1e} px; today max {:.2} p99.9 {:.2} mean {:.3} CV area {:.1e} centroid {:.1e} px",
                    engine.max_cv, engine.p999_cv, engine.mean_cv, engine.area, engine.centroid,
                    today.max_cv, today.p999_cv, today.mean_cv, today.area, today.centroid,
                );
                assert!(engine.max_cv <= today.max_cv, "{name} {precision:?}");
                assert!(engine.p999_cv <= today.p999_cv, "{name} {precision:?}");
                assert!(engine.mean_cv <= today.mean_cv, "{name} {precision:?}");
                assert!(engine.area <= 1e-3, "{name} {precision:?}");
                assert!(engine.centroid <= 2e-3, "{name} {precision:?}");
            }
        }
    }
}
