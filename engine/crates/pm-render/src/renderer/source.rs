//! Independent source snapshots. The same media cache and source shader as
//! composition layers decode, interpret, orient, and cache the source.

use std::io::Read;
use std::path::Path;
use std::time::Instant;

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

/// A single source request completes its map before the next request. Keep
/// the small output surface and staging buffer for adjacent seeks/filmstrip
/// frames; a large one-off snapshot does not pin an oversized allocation.
const MAX_REUSED_SOURCE_BYTES: u64 = 16 << 20;
const MAX_SOURCE_BATCH: usize = 16;

#[derive(Default)]
pub(super) struct SourceReuse {
    target: Option<Target>,
    readback: pm_gpu::readback::ReadbackRing,
    batch_buffer: Option<(wgpu::Buffer, u64)>,
}

impl SourceReuse {
    pub(super) fn bytes(&self) -> u64 {
        self.target.as_ref().map_or(0, Target::bytes)
            + self.readback.capacity_bytes()
            + self.batch_buffer.as_ref().map_or(0, |(_, bytes)| *bytes)
    }

    pub(super) fn trim(&mut self, bytes: u64) {
        if self.bytes() > bytes {
            *self = Self::default();
        }
    }
}

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
        self.source_rgba8_with_decode(path, time, output, fit, interpretation, true)
    }

    /// Single-frame job path: preserve the batch job's direct Export decode.
    pub fn source_rgba8_export(
        &mut self,
        path: &Path,
        time: Ticks,
        output: Size,
        fit: SourceFit,
        interpretation: Option<&Interpretation>,
    ) -> Result<Vec<u8>, String> {
        self.source_rgba8_with_decode(path, time, output, fit, interpretation, false)
    }

    fn source_rgba8_with_decode(
        &mut self,
        path: &Path,
        time: Ticks,
        output: Size,
        fit: SourceFit,
        interpretation: Option<&Interpretation>,
        exact_worker: bool,
    ) -> Result<Vec<u8>, String> {
        let started = Instant::now();
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
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("pm source snapshot"),
            });
        let source =
            self.source_texture(path, time, interpretation, kind, &mut encoder, exact_worker)?;
        let record_ms = started.elapsed().as_secs_f64() * 1_000.0;
        let result = self.draw_source_target(source, output, fit, encoder);
        if std::env::var_os("PM_SOURCE_PROFILE").is_some() {
            eprintln!(
                "pm source frame exact_worker={exact_worker} record_ms={record_ms:.3} draw_read_ms={:.3} total_ms={:.3}",
                started.elapsed().as_secs_f64() * 1_000.0 - record_ms,
                started.elapsed().as_secs_f64() * 1_000.0,
            );
        }
        result
    }

    fn source_texture(
        &mut self,
        path: &Path,
        time: Ticks,
        interpretation: Option<&Interpretation>,
        kind: pm_media::image::ImageFormat,
        encoder: &mut wgpu::CommandEncoder,
        exact_worker: bool,
    ) -> Result<Target, String> {
        if kind != pm_media::image::ImageFormat::Other {
            self.media
                .image(
                    &self.device,
                    &self.queue,
                    &self.pipelines,
                    encoder,
                    path,
                    WorkingEncoding::Standard,
                    Precision::Float16,
                    interpretation,
                    interpretation.is_none(),
                )
                .map_err(media_error)
        } else {
            let video = if exact_worker {
                self.media.source_exact_video(
                    &self.device,
                    &self.queue,
                    &self.pipelines,
                    encoder,
                    path,
                    0,
                    time.get() as f64 / TICKS_PER_SECOND as f64,
                    WorkingEncoding::Standard,
                    Precision::Float16,
                    interpretation,
                    pm_time::Rate::new(30, 1).expect("30 fps"),
                    false,
                    interpretation.is_none(),
                    true,
                )
            } else {
                self.media.video(
                    &self.device,
                    &self.queue,
                    &self.pipelines,
                    encoder,
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
            };
            video
                .map_err(media_error)?
                .ok_or_else(|| "No source frame at this time".to_owned())
        }
    }

    /// Maximum number of full output frames one source submission can hold.
    /// A job asks the priority gate for this many units; while playback is
    /// active the gate may grant fewer, including one.
    pub fn source_batch_limit(output: Size) -> usize {
        let row = u64::from(output.width) * 4;
        let padded = row.div_ceil(256) * 256;
        let frame = padded * u64::from(output.height);
        if frame == 0 {
            return 1;
        }
        (MAX_REUSED_SOURCE_BYTES / frame)
            .max(1)
            .min(MAX_SOURCE_BATCH as u64) as usize
    }

    /// Draws a bounded chunk in request order, then maps one staging buffer.
    /// Each frame still has its own source decode/conversion and draw/copy
    /// commands; the output target is overwritten only after its copy has
    /// been recorded. Callers admit the chunk through the job priority gate.
    #[allow(clippy::too_many_arguments)]
    pub fn source_rgba8_batch(
        &mut self,
        path: &Path,
        times: &[Ticks],
        output: Size,
        fit: SourceFit,
        interpretation: Option<&Interpretation>,
    ) -> Result<Vec<Vec<u8>>, String> {
        let started = Instant::now();
        if times.is_empty() || times.len() > Self::source_batch_limit(output) {
            return Err("The source batch exceeds its bounded readback size".into());
        }
        let mut head = [0u8; 512];
        let n = std::fs::File::open(path)
            .and_then(|mut f| f.read(&mut head))
            .map_err(|e| e.to_string())?;
        let kind = pm_media::image::sniff(&head[..n]);
        if kind == pm_media::image::ImageFormat::Svg {
            return Err("Source SVG needs a Chromium raster".into());
        }
        let row = output.width as usize * 4;
        let padded = row.div_ceil(256) * 256;
        let frame_stride = padded * output.height as usize;
        let size = (frame_stride * times.len()) as u64;
        if size > MAX_REUSED_SOURCE_BYTES {
            return Err("The source batch exceeds its bounded readback size".into());
        }
        let (buffer, capacity) = self
            .source_reuse
            .batch_buffer
            .take()
            .filter(|(_, capacity)| *capacity >= size)
            .unwrap_or_else(|| {
                (
                    self.device.create_buffer(&wgpu::BufferDescriptor {
                        label: Some("pm source batch readback"),
                        size,
                        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                        mapped_at_creation: false,
                    }),
                    size,
                )
            });
        let target = self.take_source_target(output);
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("pm source batch"),
            });
        for (index, &time) in times.iter().enumerate() {
            let source =
                self.source_texture(path, time, interpretation, kind, &mut encoder, false)?;
            self.record_source_draw(&source, output, fit, &target, &mut encoder);
            encoder.copy_texture_to_buffer(
                target.texture.as_image_copy(),
                wgpu::TexelCopyBufferInfo {
                    buffer: &buffer,
                    layout: wgpu::TexelCopyBufferLayout {
                        offset: (index * frame_stride) as u64,
                        bytes_per_row: Some(padded as u32),
                        rows_per_image: Some(output.height),
                    },
                },
                wgpu::Extent3d {
                    width: output.width,
                    height: output.height,
                    depth_or_array_layers: 1,
                },
            );
        }
        let record_ms = started.elapsed().as_secs_f64() * 1_000.0;
        self.queue.submit([encoder.finish()]);
        self.media.flush();
        let (signal, ready) = std::sync::mpsc::channel();
        buffer
            .slice(..size)
            .map_async(wgpu::MapMode::Read, move |result| {
                let _ = signal.send(result);
            });
        let _ = self.device.poll(wgpu::PollType::wait_indefinitely());
        ready
            .recv()
            .map_err(|e| e.to_string())?
            .map_err(|e| e.to_string())?;
        let map_ms = started.elapsed().as_secs_f64() * 1_000.0 - record_ms;
        let mapped = buffer
            .slice(..size)
            .get_mapped_range()
            .map_err(|e| format!("{e:?}"))?;
        let mut frames = Vec::with_capacity(times.len());
        for index in 0..times.len() {
            let start = index * frame_stride;
            let mut pixels = Vec::with_capacity(row * output.height as usize);
            for y in 0..output.height as usize {
                let at = start + y * padded;
                pixels.extend_from_slice(&mapped[at..at + row]);
            }
            frames.push(pixels);
        }
        drop(mapped);
        buffer.unmap();
        self.source_reuse.batch_buffer = Some((buffer, capacity));
        self.keep_source_target(target);
        if std::env::var_os("PM_SOURCE_PROFILE").is_some() {
            eprintln!(
                "pm source batch frames={} record_ms={record_ms:.3} submit_map_ms={map_ms:.3} copy_ms={:.3} total_ms={:.3}",
                times.len(),
                started.elapsed().as_secs_f64() * 1_000.0 - record_ms - map_ms,
                started.elapsed().as_secs_f64() * 1_000.0,
            );
        }
        Ok(frames)
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
        let target = self.take_source_target(output);
        let (ticket, copy) = self
            .source_reuse
            .readback
            .reserve(&self.device, &target.texture, MAX_REUSED_SOURCE_BYTES)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| "The source readback is still in flight".to_owned())?;
        self.record_source_draw(&source, output, fit, &target, &mut encoder);
        copy.record(&mut encoder, &target.texture);
        self.queue.submit([encoder.finish()]);
        self.media.flush();
        let (signal, ready) = std::sync::mpsc::channel();
        self.source_reuse.readback.start_map(ticket, move || {
            let _ = signal.send(());
        });
        let _ = self.device.poll(wgpu::PollType::wait_indefinitely());
        ready.recv().map_err(|e| e.to_string())?;
        let pixels = self
            .source_reuse
            .readback
            .try_with_mapped_rows(
                ticket,
                |data, row, padded, height| {
                    if row == padded {
                        return data.to_vec();
                    }
                    let mut out = Vec::with_capacity(row * height);
                    for y in 0..height {
                        out.extend_from_slice(&data[y * padded..y * padded + row]);
                    }
                    out
                },
                MAX_REUSED_SOURCE_BYTES,
            )
            .expect("the source map callback completed")
            .map_err(|e| e.to_string())?;
        self.keep_source_target(target);
        Ok(pixels)
    }

    fn take_source_target(&mut self, output: Size) -> Target {
        self.source_reuse
            .target
            .take()
            .filter(|target| target.width() == output.width && target.height() == output.height)
            .unwrap_or_else(|| {
                Target::new(
                    &self.device,
                    "pm source snapshot pixels",
                    output.width,
                    output.height,
                    wgpu::TextureFormat::Rgba8Unorm,
                    wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
                )
            })
    }

    fn keep_source_target(&mut self, target: Target) {
        if target.bytes() <= MAX_REUSED_SOURCE_BYTES {
            self.source_reuse.target = Some(target);
        }
    }

    fn record_source_draw(
        &self,
        source: &Target,
        output: Size,
        fit: SourceFit,
        target: &Target,
        encoder: &mut wgpu::CommandEncoder,
    ) {
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
}

fn media_error(e: MediaFailure) -> String {
    match e {
        MediaFailure::Failed(message) => message,
        MediaFailure::Unsupported(feature) => format!("Unsupported source: {feature:?}"),
    }
}
