//! Independent source snapshots. The same media cache and source shader as
//! composition layers decode, interpret, orient, and cache the source.

use std::io::Read;
use std::path::Path;

use pm_gpu::Target;
use pm_media::interpret::Interpretation;
use pm_protocol::color::{Precision, WorkingEncoding};
use pm_protocol::common::{Size, TICKS_PER_SECOND, Ticks};
use pm_protocol::snapshot::SourceFit;
use pm_protocol::svg_raster::{SvgRasterKey, SvgRasterRequest};
use sha2::{Digest, Sha256};

use crate::media::MediaFailure;
use crate::pipelines::{QuadUniform, UNIFORM_STRIDE, bytes_of};

use super::Renderer;

impl Renderer {
    pub fn source_is_svg(path: &Path) -> Result<bool, String> {
        let mut head = [0u8; 512];
        let n = std::fs::File::open(path)
            .and_then(|mut file| file.read(&mut head))
            .map_err(|e| e.to_string())?;
        Ok(pm_media::image::sniff(&head[..n]) == pm_media::image::ImageFormat::Svg)
    }

    /// A source frame in top-down premultiplied RGBA8 display pixels.
    /// The output is independent of a composition's color settings.
    #[allow(clippy::too_many_arguments)]
    pub fn source_rgba8(
        &mut self,
        path: &Path,
        time: Ticks,
        output: Size,
        fit: SourceFit,
        interpretation: Option<&Interpretation>,
    ) -> Result<Vec<u8>, String> {
        let mut head = [0u8; 512];
        let n = std::fs::File::open(path)
            .and_then(|mut f| f.read(&mut head))
            .map_err(|e| e.to_string())?;
        let kind = pm_media::image::sniff(&head[..n]);
        if kind == pm_media::image::ImageFormat::Svg {
            return Err(
                "Source SVG needs a Chromium raster; the native source path has none".into(),
            );
        }
        let still = kind != pm_media::image::ImageFormat::Other;
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("pm source snapshot"),
            });
        let source = if still {
            self.media
                .image(
                    &self.device,
                    &self.queue,
                    &self.pipelines,
                    &mut encoder,
                    path,
                    WorkingEncoding::Standard,
                    Precision::Float16,
                    interpretation,
                    interpretation.is_none(),
                )
                .map_err(media_error)?
        } else {
            self.media
                .video(
                    &self.device,
                    &self.queue,
                    &self.pipelines,
                    &mut encoder,
                    path,
                    0,
                    time.get() as f64 / TICKS_PER_SECOND as f64,
                    true,
                    WorkingEncoding::Standard,
                    Precision::Float16,
                    interpretation,
                    pm_time::Rate::new(30, 1).expect("30 fps"),
                    false,
                    interpretation.is_none(),
                    true,
                )
                .map_err(media_error)?
                .ok_or_else(|| "No source frame at this time".to_owned())?
        };
        self.draw_source_target(source, output, fit, encoder)
    }

    /// The exact-size Chromium raster of a source SVG, keyed by its bytes.
    pub fn source_svg_request(
        path: &Path,
        asset: &str,
        output: Size,
        fit: SourceFit,
        intrinsic: (f64, f64),
    ) -> Result<SvgRasterRequest, String> {
        let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
        let mut file = std::io::BufReader::new(file);
        let mut digest = Sha256::new();
        let mut chunk = [0u8; 8192];
        loop {
            let n = file.read(&mut chunk).map_err(|e| e.to_string())?;
            if n == 0 {
                break;
            }
            digest.update(&chunk[..n]);
        }
        let (sw, sh) = intrinsic;
        if !(sw.is_finite() && sh.is_finite() && sw > 0.0 && sh > 0.0) {
            return Err("The source SVG has no valid intrinsic size".into());
        }
        let key = SvgRasterKey {
            content: format!("{:x}", digest.finalize()),
            texture: None,
            source_fit: Some(fit),
            width: output.width,
            height: output.height,
        };
        key.validate()
            .map_err(|e| format!("Source SVG raster: {e}"))?;
        Ok(SvgRasterRequest {
            key,
            asset: asset.to_owned(),
        })
    }

    /// The source SVG's final Canvas2D pixels, already straight RGBA8. The
    /// Chromium host drew the fitted image directly into the output canvas;
    /// a second GPU fit would move fractional edges and unpremultiply twice.
    pub fn source_svg_rgba8(
        &mut self,
        key: &SvgRasterKey,
        output: Size,
        fit: SourceFit,
        interpretation: Option<&Interpretation>,
    ) -> Result<Option<Vec<u8>>, String> {
        if interpretation.is_some_and(|value| !value.is_empty()) {
            return Err(
                "A source SVG interpretation cannot be applied to the Chromium raster".into(),
            );
        }
        if key.source_fit != Some(fit) || key.width != output.width || key.height != output.height {
            return Err("The source SVG raster has another output size or fit".into());
        }
        let Some(pixels) = self.vector.svg.get(key) else {
            return Ok(None);
        };
        Ok(Some(pixels.as_ref().clone()))
    }

    fn draw_source_target(
        &mut self,
        source: Target,
        output: Size,
        fit: SourceFit,
        mut encoder: wgpu::CommandEncoder,
    ) -> Result<Vec<u8>, String> {
        let (w, h) = (output.width, output.height);
        let (sw, sh) = (source.width() as f64, source.height() as f64);
        let (ow, oh) = (w as f64, h as f64);
        let ratio = match fit {
            SourceFit::Stretch => None,
            SourceFit::Contain => Some((ow / sw).min(oh / sh)),
            SourceFit::Cover => Some((ow / sw).max(oh / sh)),
        };
        let (dw, dh) = ratio.map_or((ow, oh), |r| (sw * r, sh * r));
        let uniform = QuadUniform {
            m: [
                [dw as f32, 0.0, 0.0, 0.0],
                [0.0, -(dh as f32), 0.0, 0.0],
                [((ow - dw) * 0.5) as f32, ((oh + dh) * 0.5) as f32, 1.0, 0.0],
            ],
            res: [w as f32, h as f32, 0.0, 0.0],
            uv: [0.0, 0.0, 1.0, 1.0],
            color: [1.0, 0.0, 0.0, 0.0],
        };
        let params = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("pm source snapshot quad"),
            size: UNIFORM_STRIDE,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        self.queue.write_buffer(&params, 0, bytes_of(&uniform));
        let quad = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("pm source snapshot quad"),
            layout: &self.pipelines.quad_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                    buffer: &params,
                    offset: 0,
                    size: wgpu::BufferSize::new(std::mem::size_of::<QuadUniform>() as u64),
                }),
            }],
        });
        let tex = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("pm source snapshot texture"),
            layout: &self.pipelines.texture_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&source.view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.pipelines.linear),
                },
            ],
        });
        let target = Target::new(
            &self.device,
            "pm source snapshot pixels",
            w,
            h,
            wgpu::TextureFormat::Rgba8Unorm,
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        );
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("pm source snapshot draw"),
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
            pass.set_pipeline(self.pipelines.draw(wgpu::TextureFormat::Rgba8Unorm));
            pass.set_bind_group(0, &quad, &[0]);
            pass.set_bind_group(1, &tex, &[]);
            pass.draw(0..4, 0..1);
        }
        self.queue.submit([encoder.finish()]);
        pm_gpu::readback::read_raw(&self.device, &self.queue, &target.texture)
            .map_err(|e| e.to_string())
    }
}

fn media_error(e: MediaFailure) -> String {
    match e {
        MediaFailure::Failed(message) => message,
        MediaFailure::Unsupported(feature) => format!("Unsupported source: {feature:?}"),
    }
}
