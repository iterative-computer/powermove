//! Reproject a persistent high density float SVG raster at the exact layer
//! transform. The raster is reused while translation or rotation changes.

use pm_protocol::color::Precision;
use pm_vector::kurbo::{Affine, Point, Rect};
use pm_vector::render::scene::PixelRect;
use pm_vector::render::svg::SvgDocument;
use sha2::{Digest, Sha256};

use super::svg::Stamp;

/// A standing cache entry stays small enough for normal frame working sets.
pub const MAX_RASTER_BYTES: u64 = 128 * 1024 * 1024;
// A fragment walks the inverse-mapped source cells of one output pixel.
// Twenty-fourfold text at the upper end of a sqrt(2) scale bucket can span
// 48 cells on each axis after a 45-degree rotation. The extra cell on each
// axis accounts for fractional alignment at the pixel boundary.
const MAX_SOURCE_CELLS_PER_PIXEL: f64 = 50.0 * 50.0;

pub struct RasterPlan {
    pub key: [u8; 32],
    pub rect: PixelRect,
    pub scale: f64,
    pub source_precision: Precision,
    /// The image-box crop in canonical raster coordinates. It is painted
    /// together with SVG coverage before the raster enters the cache.
    pub source_clip: [Point; 4],
}

/// Upward scale buckets are invariant under translation and rotation. Text
/// and ordinary paths need denser sources for moving edges. Masks and filters
/// keep an eightfold source because each source pixel can multiply the joint
/// fine-grid cost. A source that does not fit uses the exact direct float path.
pub fn plan(
    doc: &SvgDocument,
    path: &std::path::Path,
    stamp: Stamp,
    transform: Affine,
    limits: &wgpu::Limits,
) -> Option<RasterPlan> {
    let (width, height) = doc.size();
    let clip = [
        transform * Point::new(0.0, 0.0),
        transform * Point::new(width, 0.0),
        transform * Point::new(width, height),
        transform * Point::new(0.0, height),
    ];
    plan_with_clip(doc, path, stamp, transform, &clip, limits)
}

/// Plan a source raster with its actual image-box crop baked into the source.
/// Inverting the layer transform makes this identity stable under translation
/// and rotation, while a changed crop gets a different key.
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
    if !max_scale.is_finite()
        || max_scale <= 0.0
        || !transform
            .inverse()
            .as_coeffs()
            .iter()
            .all(|v| v.is_finite())
    {
        return None;
    }
    let inverse = transform.inverse();
    let user_clip = clip.map(|point| {
        let p = inverse * point;
        // Matrix inversion after a moving/rotating image introduces a few
        // ulps of noise, including negative zero. A 10-nanounit snap keeps
        // one source window under motion without moving authored placement.
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
    let candidate = |density: f64, bytes_per_pixel: u64, bucket_ratio: f64| {
        let bucket = (((max_scale * density).max(1.0).ln() / bucket_ratio.ln()) - 1e-8)
            .ceil() as i32;
        let scale = bucket_ratio.powi(bucket);
        let rect = PixelRect::round_out(
            Affine::scale(scale)
                .transform_rect_bbox(bounds)
                .inflate(2.0, 2.0),
        );
        let bytes = u64::from(rect.width)
            .checked_mul(u64::from(rect.height))?
            .checked_mul(bytes_per_pixel)?;
        (rect.width > 0
            && rect.height > 0
            && rect.width <= limits.max_texture_dimension_2d
            && rect.height <= limits.max_texture_dimension_2d
            && bytes <= MAX_RASTER_BYTES)
            .then_some((bucket, scale, rect))
    };
    // Eightfold text and ordinary paths failed the moving-edge reference
    // probe. Text uses 24x RGBA16F; ordinary paths use 13x RGBA16F with
    // tighter 1.1x buckets to bound source memory. Masks and filters use
    // 8x RGBA32F to bound their joint fine-grid fill cost.
    let (bucket, scale, rect, source_precision) = if doc.has_text() {
        let (bucket, scale, rect) = candidate(24.0, 8, std::f64::consts::SQRT_2)?;
        (bucket, scale, rect, Precision::Float16)
    } else if doc.has_mask_or_filter() {
        let (bucket, scale, rect) = candidate(8.0, 16, std::f64::consts::SQRT_2)?;
        (bucket, scale, rect, Precision::Float32)
    } else {
        let (bucket, scale, rect) = candidate(13.0, 8, 1.1)?;
        (bucket, scale, rect, Precision::Float16)
    };
    let source_clip = user_clip.map(|point| Affine::scale(scale) * point);
    let mut hash = Sha256::new();
    hash.update(b"pm-svg-float-raster/2");
    hash.update(path.as_os_str().as_encoded_bytes());
    hash.update(stamp.0.to_le_bytes());
    hash.update(stamp.1.to_le_bytes());
    hash.update(bucket.to_le_bytes());
    hash.update([source_precision as u8]);
    for point in user_clip {
        hash.update(point.x.to_bits().to_le_bytes());
        hash.update(point.y.to_bits().to_le_bytes());
    }
    Some(RasterPlan {
        key: hash.finalize().into(),
        rect,
        scale,
        source_precision,
        source_clip,
    })
}

pub struct SvgSample {
    layout: wgpu::BindGroupLayout,
    pipelines: [wgpu::RenderPipeline; 3],
    uniform: wgpu::Buffer,
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
        let cells_x = (a.abs() + c.abs()).ceil() + 1.0;
        let cells_y = (b.abs() + d.abs()).ceil() + 1.0;
        if ![a, b, c, d, tx, ty]
            .iter()
            .all(|v| v.is_finite() && v.abs() < 16_000_000.0)
            || cells_x * cells_y > MAX_SOURCE_CELLS_PER_PIXEL
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
    use pm_protocol::color::Precision;
    use pm_reference::coverage::{FillRule, fill};
    use pm_vector::kurbo::BezPath;

    #[test]
    fn raster_identity_tracks_content_and_scale_not_translation_or_rotation() {
        let doc = SvgDocument::parse(br##"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10"><rect width="10" height="10"/></svg>"##).unwrap();
        let limits = wgpu::Limits::default();
        let path = std::path::Path::new("test.svg");
        let base = plan(&doc, path, (1, 1), Affine::scale(2.0), &limits).unwrap();
        let moved = plan(
            &doc,
            path,
            (1, 1),
            Affine::translate((20.0, 30.0)) * Affine::rotate(0.7) * Affine::scale(2.0),
            &limits,
        )
        .unwrap();
        assert_eq!(base.key, moved.key);
        let larger = plan(&doc, path, (1, 1), Affine::scale(2.5), &limits).unwrap();
        assert_ne!(base.key, larger.key);
        assert_ne!(
            base.key,
            plan(&doc, path, (2, 1), Affine::scale(2.0), &limits)
                .unwrap()
                .key
        );
    }

    #[test]
    fn crop_identity_tracks_user_window_across_motion() {
        let doc = SvgDocument::parse(br##"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10"><rect width="10" height="10"/></svg>"##).unwrap();
        let path = std::path::Path::new("cropped.svg");
        let limits = wgpu::Limits::default();
        let window = [
            Point::new(2.0, 1.0),
            Point::new(8.0, 1.0),
            Point::new(8.0, 9.0),
            Point::new(2.0, 9.0),
        ];
        let first = Affine::scale(2.0);
        let moved = Affine::translate((31.25, 19.5)) * Affine::rotate(0.4) * first;
        let a = plan_with_clip(
            &doc,
            path,
            (1, 1),
            first,
            &window.map(|point| first * point),
            &limits,
        )
        .unwrap();
        let b = plan_with_clip(
            &doc,
            path,
            (1, 1),
            moved,
            &window.map(|point| moved * point),
            &limits,
        )
        .unwrap();
        assert_eq!(a.key, b.key);
        assert_eq!(a.source_clip, b.source_clip);
        let mut changed = window;
        changed[1].x -= 0.5;
        let c = plan_with_clip(
            &doc,
            path,
            (1, 1),
            moved,
            &changed.map(|point| moved * point),
            &limits,
        )
        .unwrap();
        assert_ne!(a.key, c.key);
    }

    #[test]
    fn cached_source_has_the_image_crop_in_its_pixels() {
        let gpu = pm_gpu::Gpu::new("SVG cached crop pixels").unwrap();
        let doc = SvgDocument::parse(br##"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10"><rect width="10" height="10" fill="white"/></svg>"##).unwrap();
        let transform = Affine::scale(2.0);
        let clip = [
            Point::new(4.0, 2.0),
            Point::new(16.0, 2.0),
            Point::new(16.0, 18.0),
            Point::new(4.0, 18.0),
        ];
        let plan = plan_with_clip(
            &doc,
            std::path::Path::new("cropped.svg"),
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
                &doc,
                Affine::scale(plan.scale),
                plan.rect,
                &plan.source_clip,
                plan.source_precision,
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
                PixelRect::new(0, 0, 24, 24),
                &clip,
                Precision::Float16,
            )
            .unwrap();
        let pixels = pm_gpu::readback::read_f32(&gpu.device, &gpu.queue, &output).unwrap();
        let alpha = |x: usize, y: usize| pixels[(y * 24 + x) * 4 + 3];
        assert!(alpha(10, 10) > 0.95);
        assert!(alpha(1, 10) < 0.01);
        assert!(alpha(20, 10) < 0.01);
    }

    #[test]
    fn sampled_rotated_crop_circle_tracks_joint_f64_area() {
        let gpu = pm_gpu::Gpu::new("SVG sampled joint crop reference").unwrap();
        let doc = SvgDocument::parse(br##"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10"><circle cx="5" cy="5" r="4.5" fill="white"/></svg>"##).unwrap();
        let transform = Affine::translate((20.25, 18.5)) * Affine::rotate(0.4) * Affine::scale(2.0);
        let user_crop = [
            Point::new(2.0, 1.0),
            Point::new(8.0, 1.0),
            Point::new(8.0, 9.0),
            Point::new(2.0, 9.0),
        ];
        let clip = user_crop.map(|point| transform * point);
        let plan = plan_with_clip(
            &doc,
            std::path::Path::new("circle.svg"),
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
                &doc,
                Affine::scale(plan.scale),
                plan.rect,
                &plan.source_clip,
                plan.source_precision,
            )
            .unwrap()
            .unwrap();
        gpu.queue.submit([encoder.finish()]);
        let sample = SvgSample::new(&gpu.device);
        let inverse = transform.inverse();
        for precision in [Precision::Float32, Precision::Float16, Precision::Unorm8] {
            let (output, _) = sample
                .render(
                    &gpu.device,
                    &gpu.queue,
                    &source,
                    &plan,
                    transform,
                    PixelRect::new(0, 0, 64, 64),
                    &clip,
                    precision,
                )
                .unwrap();
            let actual = pm_gpu::readback::read_f32(&gpu.device, &gpu.queue, &output).unwrap();
            let mut max_cv = 0.0_f64;
            let mut max_storage_cv = 0.0_f64;
            let mut actual_area = 0.0_f64;
            let mut reference_area = 0.0_f64;
            let mut actual_x = 0.0_f64;
            let mut reference_x = 0.0_f64;
            for y in 0..64usize {
                for x in 0..64usize {
                    const GRID: usize = 64;
                    let mut hits = 0usize;
                    for sy in 0..GRID {
                        for sx in 0..GRID {
                            let q = Point::new(
                                x as f64 + (sx as f64 + 0.5) / GRID as f64,
                                y as f64 + (sy as f64 + 0.5) / GRID as f64,
                            );
                            let u = inverse * q;
                            let inside_circle = (u.x - 5.0).hypot(u.y - 5.0) <= 4.5;
                            if inside_circle
                                && (2.0..=8.0).contains(&u.x)
                                && (1.0..=9.0).contains(&u.y)
                            {
                                hits += 1;
                            }
                        }
                    }
                    let reference = hits as f64 / (GRID * GRID) as f64;
                    let alpha = f64::from(actual[(y * 64 + x) * 4 + 3]);
                    max_cv = max_cv.max((alpha - reference).abs() * 255.0);
                    let storage_error_cv = if precision == Precision::Unorm8 {
                        ((alpha * 255.0).round() - (reference * 255.0).round()).abs()
                    } else {
                        (alpha - reference).abs() * 255.0
                    };
                    max_storage_cv = max_storage_cv.max(storage_error_cv);
                    actual_area += alpha;
                    reference_area += reference;
                    actual_x += alpha * (x as f64 + 0.5);
                    reference_x += reference * (x as f64 + 0.5);
                }
            }
            let area_error = (actual_area - reference_area).abs() / reference_area;
            let centroid_error = (actual_x / actual_area - reference_x / reference_area).abs();
            eprintln!(
                "sampled crop circle {precision:?}: raw max {max_cv:.3} CV, storage max {max_storage_cv:.3} CV, area {area_error:.3e}, centroid {centroid_error:.3e} px"
            );
            assert!(max_storage_cv <= 1.0);
            assert!(area_error <= 1e-3);
            assert!(centroid_error <= 1e-3);
        }
    }

    #[test]
    fn sampled_rotated_masked_stroke_tracks_joint_f64_area() {
        let gpu = pm_gpu::Gpu::new("SVG sampled mask and stroke reference").unwrap();
        let doc = SvgDocument::parse(br##"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10"><defs><mask id="m" maskUnits="userSpaceOnUse" maskContentUnits="userSpaceOnUse" x="0" y="0" width="10" height="10"><rect width="6" height="10" fill="white"/></mask></defs><g mask="url(#m)" opacity=".8"><rect x="2" y="2" width="6" height="6" fill="none" stroke="white" stroke-width="1"/></g></svg>"##).unwrap();
        let transform = Affine::translate((20.25, 18.5)) * Affine::rotate(0.4) * Affine::scale(2.0);
        let user_crop = [
            Point::new(2.0, 1.0),
            Point::new(8.0, 1.0),
            Point::new(8.0, 9.0),
            Point::new(2.0, 9.0),
        ];
        let clip = user_crop.map(|point| transform * point);
        let plan = plan_with_clip(
            &doc,
            std::path::Path::new("masked-stroke.svg"),
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
                &doc,
                Affine::scale(plan.scale),
                plan.rect,
                &plan.source_clip,
                plan.source_precision,
            )
            .unwrap()
            .unwrap();
        gpu.queue.submit([encoder.finish()]);
        let sample = SvgSample::new(&gpu.device);
        let inverse = transform.inverse();
        for precision in [Precision::Float32, Precision::Float16, Precision::Unorm8] {
            let (output, _) = sample
                .render(
                    &gpu.device,
                    &gpu.queue,
                    &source,
                    &plan,
                    transform,
                    PixelRect::new(0, 0, 64, 64),
                    &clip,
                    precision,
                )
                .unwrap();
            let actual = pm_gpu::readback::read_f32(&gpu.device, &gpu.queue, &output).unwrap();
            let mut max_cv = 0.0_f64;
            let mut max_storage_cv = 0.0_f64;
            let mut actual_area = 0.0_f64;
            let mut reference_area = 0.0_f64;
            let mut actual_x = 0.0_f64;
            let mut reference_x = 0.0_f64;
            for y in 0..64usize {
                for x in 0..64usize {
                    const GRID: usize = 64;
                    let mut hits = 0usize;
                    for sy in 0..GRID {
                        for sx in 0..GRID {
                            let q = Point::new(
                                x as f64 + (sx as f64 + 0.5) / GRID as f64,
                                y as f64 + (sy as f64 + 0.5) / GRID as f64,
                            );
                            let u = inverse * q;
                            let outer = (1.5..=8.5).contains(&u.x) && (1.5..=8.5).contains(&u.y);
                            let inner = (2.5..=7.5).contains(&u.x) && (2.5..=7.5).contains(&u.y);
                            if outer
                                && !inner
                                && u.x <= 6.0
                                && (2.0..=8.0).contains(&u.x)
                                && (1.0..=9.0).contains(&u.y)
                            {
                                hits += 1;
                            }
                        }
                    }
                    let reference = 0.8 * hits as f64 / (GRID * GRID) as f64;
                    let alpha = f64::from(actual[(y * 64 + x) * 4 + 3]);
                    max_cv = max_cv.max((alpha - reference).abs() * 255.0);
                    let storage_error_cv = if precision == Precision::Unorm8 {
                        ((alpha * 255.0).round() - (reference * 255.0).round()).abs()
                    } else {
                        (alpha - reference).abs() * 255.0
                    };
                    max_storage_cv = max_storage_cv.max(storage_error_cv);
                    actual_area += alpha;
                    reference_area += reference;
                    actual_x += alpha * (x as f64 + 0.5);
                    reference_x += reference * (x as f64 + 0.5);
                }
            }
            let area_error = (actual_area - reference_area).abs() / reference_area;
            let centroid_error = (actual_x / actual_area - reference_x / reference_area).abs();
            eprintln!(
                "sampled masked stroke {precision:?}: raw max {max_cv:.3} CV, storage max {max_storage_cv:.3} CV, area {area_error:.3e}, centroid {centroid_error:.3e} px"
            );
            assert!(max_storage_cv <= 1.0);
            assert!(area_error <= 1e-3);
            assert!(centroid_error <= 1e-3);
        }
    }

    #[test]
    fn text_uses_dense_float_cached_source_with_precise_output() {
        let gpu = pm_gpu::Gpu::new("SVG large text source quality").unwrap();
        let doc = SvgDocument::parse(br##"<svg xmlns="http://www.w3.org/2000/svg" width="80" height="30"><text x="4" y="22" font-family="Helvetica" font-size="18" fill="#4C8DFF">Pm Svg</text></svg>"##).unwrap();
        let transform = Affine::translate((80.0, 60.0)) * Affine::rotate(0.16) * Affine::scale(2.5);
        let corners = [
            Point::new(0.0, 0.0),
            Point::new(80.0, 0.0),
            Point::new(80.0, 30.0),
            Point::new(0.0, 30.0),
        ];
        let clip = corners.map(|point| transform * point);
        let plan = plan_with_clip(
            &doc,
            std::path::Path::new("large-text.svg"),
            (1, 1),
            transform,
            &clip,
            &gpu.device.limits(),
        )
        .unwrap();
        assert!(plan.scale / 2.5 >= 24.0);
        assert_eq!(plan.source_precision, Precision::Float16);
        let moved = Affine::translate((90.0, 60.0)) * Affine::rotate(0.16) * Affine::scale(2.5);
        let moved_clip = corners.map(|point| moved * point);
        assert_eq!(
            plan.key,
            plan_with_clip(
                &doc,
                std::path::Path::new("large-text.svg"),
                (1, 1),
                moved,
                &moved_clip,
                &gpu.device.limits(),
            )
            .unwrap()
            .key
        );
        let rect = PixelRect::new(0, 0, 300, 150);
        let mut painter = super::super::svg_float::SvgPainter::new(&gpu.device).unwrap();
        let mut source_encoder = gpu.device.create_command_encoder(&Default::default());
        let (source, _) = painter
            .render(
                &gpu.device,
                &gpu.queue,
                &mut source_encoder,
                &doc,
                Affine::scale(plan.scale),
                plan.rect,
                &plan.source_clip,
                plan.source_precision,
            )
            .unwrap()
            .unwrap();
        gpu.queue.submit([source_encoder.finish()]);
        let mut direct_encoder = gpu.device.create_command_encoder(&Default::default());
        let (direct, _) = painter
            .render(
                &gpu.device,
                &gpu.queue,
                &mut direct_encoder,
                &doc,
                transform,
                rect,
                &clip,
                Precision::Float32,
            )
            .unwrap()
            .unwrap();
        gpu.queue.submit([direct_encoder.finish()]);
        let float_reference = pm_gpu::readback::read_f32(&gpu.device, &gpu.queue, &direct).unwrap();
        let sample = SvgSample::new(&gpu.device);
        let (float_output, _) = sample
            .render(
                &gpu.device,
                &gpu.queue,
                &source,
                &plan,
                transform,
                rect,
                &clip,
                Precision::Float32,
            )
            .unwrap();
        let float_sample =
            pm_gpu::readback::read_f32(&gpu.device, &gpu.queue, &float_output).unwrap();
        for precision in [Precision::Float32, Precision::Float16, Precision::Unorm8] {
            let reference = if precision == Precision::Float32 {
                float_reference.clone()
            } else {
                let mut encoder = gpu.device.create_command_encoder(&Default::default());
                let (same_format, _) = painter
                    .render(
                        &gpu.device,
                        &gpu.queue,
                        &mut encoder,
                        &doc,
                        transform,
                        rect,
                        &clip,
                        precision,
                    )
                    .unwrap()
                    .unwrap();
                gpu.queue.submit([encoder.finish()]);
                pm_gpu::readback::read_f32(&gpu.device, &gpu.queue, &same_format).unwrap()
            };
            let (output, _) = sample
                .render(
                    &gpu.device,
                    &gpu.queue,
                    &source,
                    &plan,
                    transform,
                    rect,
                    &clip,
                    precision,
                )
                .unwrap();
            let actual = pm_gpu::readback::read_f32(&gpu.device, &gpu.queue, &output).unwrap();
            let mut max_cv = 0.0_f64;
            let mut storage_max_cv = 0.0_f64;
            let mut actual_area = 0.0_f64;
            let mut reference_area = 0.0_f64;
            let mut actual_x = 0.0_f64;
            let mut reference_x = 0.0_f64;
            let mut float_area = 0.0_f64;
            let mut float_x = 0.0_f64;
            for (index, (&a, &r)) in actual.iter().zip(&reference).enumerate() {
                let error_cv = if precision == Precision::Unorm8 {
                    ((f64::from(a) * 255.0).round() - (f64::from(r) * 255.0).round()).abs()
                } else {
                    f64::from((a - r).abs()) * 255.0
                };
                max_cv = max_cv.max(error_cv);
                storage_max_cv = storage_max_cv
                    .max((f64::from(a) - f64::from(float_sample[index])).abs() * 255.0);
                if index % 4 == 3 {
                    actual_area += f64::from(a);
                    reference_area += f64::from(r);
                    let f = f64::from(float_reference[index]);
                    float_area += f;
                    actual_x += f64::from(a) * ((index / 4 % 300) as f64 + 0.5);
                    reference_x += f64::from(r) * ((index / 4 % 300) as f64 + 0.5);
                    float_x += f * ((index / 4 % 300) as f64 + 0.5);
                }
            }
            let area_error = (actual_area - reference_area).abs() / reference_area;
            let centroid_error = (actual_x / actual_area - reference_x / reference_area).abs();
            let storage_centroid_error =
                (reference_x / reference_area - float_x / float_area).abs();
            eprintln!(
                "dense float text {precision:?}: max {max_cv:.3} CV vs same-format direct, storage max {storage_max_cv:.3} CV, area {area_error:.3e}, centroid {centroid_error:.3e} px, storage centroid {storage_centroid_error:.3e} px"
            );
            assert!(max_cv <= 1.0);
            assert!(area_error <= 1e-3);
            if precision == Precision::Float32 {
                assert!(centroid_error <= 1e-3);
            } else {
                // The finite output format is the only extra loss: values
                // are within half an output code/ULP of the float sample.
                let storage_bound = if precision == Precision::Float16 {
                    0.125
                } else {
                    0.5
                };
                assert!(storage_max_cv <= storage_bound + 1e-4);
            }
        }
    }

    #[test]
    fn rotated_raster_tracks_f64_coverage_at_authored_placement() {
        let gpu = pm_gpu::Gpu::new("SVG raster reference").unwrap();
        let doc = SvgDocument::parse(br##"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10"><rect width="10" height="10" fill="white"/></svg>"##).unwrap();
        let transform = Affine::translate((20.25, 18.5)) * Affine::rotate(0.4) * Affine::scale(2.0);
        let plan = plan(
            &doc,
            std::path::Path::new("test.svg"),
            (1, 1),
            transform,
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
                &doc,
                Affine::scale(plan.scale),
                plan.rect,
                &[],
                plan.source_precision,
            )
            .unwrap()
            .unwrap();
        gpu.queue.submit([encoder.finish()]);
        let clip = [(0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.0, 10.0)]
            .map(|(x, y)| transform * Point::new(x, y));
        let (output, _) = SvgSample::new(&gpu.device)
            .render(
                &gpu.device,
                &gpu.queue,
                &source,
                &plan,
                transform,
                PixelRect::new(0, 0, 64, 64),
                &clip,
                Precision::Float32,
            )
            .unwrap();
        let actual = pm_gpu::readback::read_f32(&gpu.device, &gpu.queue, &output).unwrap();
        let mut rect = BezPath::new();
        rect.move_to((0.0, 0.0));
        rect.line_to((10.0, 0.0));
        rect.line_to((10.0, 10.0));
        rect.line_to((0.0, 10.0));
        rect.close_path();
        let reference = fill(&(transform * rect), FillRule::NonZero, 16, 64, 64);
        let mut max_error = 0.0_f64;
        for y in 0..64 {
            for x in 0..64 {
                max_error = max_error
                    .max((f64::from(actual[(y * 64 + x) * 4 + 3]) - reference.at(x, y)).abs());
            }
        }
        eprintln!(
            "conservative sampled rectangle max {:.3} CV",
            max_error * 255.0
        );
        assert!(
            max_error <= 0.01,
            "raster versus f64 max {:.3} CV",
            max_error * 255.0
        );
    }
}
