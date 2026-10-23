//! GPU-resident WebM frames: float working texture → BGRA8 IOSurface →
//! Chromium's compositor canvas. No display transform, dither or CPU pixels.
use pm_export::render::{GpuGate, Unrestricted};
use pm_render::CommandBracket;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use pm_bridge::rendezvous::{Client, SendRight};
use pm_bridge::wire::{PixelFormat, SurfaceInfo};
use pm_export::Error;
use pm_gpu::bridge_out::pool::SurfacePool;
use pm_protocol::common::Size;
use pm_protocol::interop::SurfaceFormat;

pub(crate) const ENV: &str = "POWERMOVE_EXPORT_BRIDGE";
const SLOTS: u32 = 5; // main acknowledges after at most four unanswered frames
static CLIENT: OnceLock<Result<Client, String>> = OnceLock::new();
static NEXT_GENERATION: AtomicU32 = AtomicU32::new(1);

pub(crate) fn connect() -> Result<Option<&'static Client>, String> {
    let Some(name) = std::env::var(ENV).ok().filter(|s| !s.is_empty()) else {
        return Ok(None);
    };
    CLIENT
        .get_or_init(|| Client::check_in(&name, Duration::from_secs(10)).map_err(|e| e.to_string()))
        .as_ref()
        .map(Some)
        .map_err(Clone::clone)
}

pub(crate) struct GpuEncoder {
    device: wgpu::Device,
    queue: wgpu::Queue,
    pool: Option<SurfacePool>,
    layout: wgpu::BindGroupLayout,
    pipeline: wgpu::RenderPipeline,
    gate: Arc<dyn GpuGate>,
    cancel: Arc<AtomicBool>,
    pass: u64,
    span: Option<pm_export::gpu_time::GpuSpan>,
}

impl GpuEncoder {
    pub(crate) fn new(device: wgpu::Device, queue: wgpu::Queue) -> Self {
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("WebM canvas bytes"),
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
        });
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("WebM canvas bytes"),
            source: wgpu::ShaderSource::Wgsl(include_str!("encode_gpu.wgsl").into()),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("WebM canvas bytes"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("WebM canvas bytes"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vs"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            primitive: Default::default(),
            depth_stencil: None,
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fs"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::TextureFormat::Bgra8Unorm.into())],
            }),
            multiview_mask: None,
            cache: None,
        });
        Self {
            device,
            queue,
            pool: None,
            layout,
            pipeline,
            gate: Arc::new(Unrestricted),
            cancel: Arc::new(AtomicBool::new(false)),
            pass: 0,
            span: None,
        }
    }

    pub(crate) fn with_gate(
        mut self,
        gate: Arc<dyn GpuGate>,
        cancel: Arc<AtomicBool>,
        pass: u64,
    ) -> Self {
        if gate.measures() {
            self.span = pm_export::gpu_time::GpuSpan::new(&self.device, &self.queue);
        }
        self.gate = gate;
        self.cancel = cancel;
        self.pass = pass;
        self
    }

    pub(crate) fn frame(
        &mut self,
        index: u64,
        source: &wgpu::Texture,
    ) -> Result<(u32, u32), Error> {
        let bridge = connect()
            .map_err(Error::Encode)?
            .ok_or_else(|| Error::Encode("the export GPU bridge is missing".into()))?;
        if self.pool.is_none() {
            let generation = NEXT_GENERATION.fetch_add(1, Ordering::Relaxed);
            if generation == 0 {
                return Err(Error::Encode("export surface generations exhausted".into()));
            }
            let pool = SurfacePool::new(
                &self.device,
                generation,
                Size::new(source.width(), source.height()),
                SurfaceFormat::Bgra8Unorm,
                SLOTS,
            )
            .map_err(|e| Error::Encode(e.to_string()))?;
            // Install before sending: every partially announced pool is retired on error.
            self.pool = Some(pool);
            let pool = self.pool.as_ref().unwrap();
            for (slot, surface) in pool.surfaces.iter().enumerate() {
                let port = surface
                    .iosurface
                    .mach_port()
                    .ok_or_else(|| Error::Encode("IOSurfaceCreateMachPort failed".into()))?;
                bridge
                    .send_surface(
                        SendRight::from_raw(port),
                        &SurfaceInfo {
                            client_port: 0,
                            generation,
                            pool_index: slot as u32,
                            pool_size: SLOTS,
                            width: source.width(),
                            height: source.height(),
                            format: PixelFormat::Bgra8,
                            bytes_per_row: surface.iosurface.bytes_per_row(),
                        },
                    )
                    .map_err(|e| Error::Encode(e.to_string()))?;
            }
        }
        self.draw(index, source)
    }

    fn draw(&self, index: u64, source: &wgpu::Texture) -> Result<(u32, u32), Error> {
        let pool = self.pool.as_ref().unwrap();
        if pool.size != Size::new(source.width(), source.height()) {
            return Err(Error::InvalidFrame("export GPU size changed".into()));
        }
        let slot = (index % u64::from(SLOTS)) as u32;
        let source_view = source.create_view(&Default::default());
        let group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("WebM canvas bytes"),
            layout: &self.layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&source_view),
            }],
        });
        const BAND_ROWS: u32 = 16;
        let mut y = 0;
        while y < source.height() {
            let remaining = (source.height() - y).div_ceil(BAND_ROWS);
            let admitted = self
                .gate
                .admit(self.pass, remaining, &self.cancel)
                .ok_or(Error::Cancelled)?
                .clamp(1, remaining);
            self.gate.sync_worker_qos();
            struct Admission<'a> {
                gate: &'a dyn GpuGate,
                pass: u64,
                open: bool,
            }
            impl Drop for Admission<'_> {
                fn drop(&mut self) {
                    if self.open {
                        self.gate.abandoned(self.pass);
                    }
                }
            }
            let mut admission = Admission {
                gate: &*self.gate,
                pass: self.pass,
                open: true,
            };
            let rows = (admitted * BAND_ROWS).min(source.height() - y);
            let mut encoder = self
                .device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("WebM canvas bytes"),
                });
            if let Some(span) = &self.span {
                span.begin(&mut encoder);
            }
            {
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("WebM canvas bytes"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &pool.surfaces[slot as usize].view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: if y == 0 {
                                wgpu::LoadOp::Clear(wgpu::Color::BLACK)
                            } else {
                                wgpu::LoadOp::Load
                            },
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    ..Default::default()
                });
                pass.set_pipeline(&self.pipeline);
                pass.set_bind_group(0, &group, &[]);
                pass.set_scissor_rect(0, y, source.width(), rows);
                pass.draw(0..3, 0..1);
            }
            if let Some(span) = &self.span {
                span.end(&mut encoder, &pool.surfaces[slot as usize].view);
            }
            let started = Instant::now();
            let submission = self.queue.submit([encoder.finish()]);
            self.device
                .poll(wgpu::PollType::Wait {
                    submission_index: Some(submission),
                    timeout: Some(Duration::from_secs(10)),
                })
                .map_err(|e| {
                    Error::Encode(format!("the export GPU frame did not complete: {e}"))
                })?;
            let wall_us = started.elapsed().as_secs_f64() * 1e6;
            let gpu_us = self
                .span
                .as_ref()
                .and_then(|s| s.read_us(&self.device, &self.queue));
            self.gate.finished_timed(
                self.pass,
                admitted,
                gpu_us.unwrap_or(wall_us),
                gpu_us.is_some(),
            );
            admission.open = false;
            y += rows;
        }
        Ok((pool.generation, slot))
    }
}

impl Drop for GpuEncoder {
    fn drop(&mut self) {
        if let Some(pool) = &self.pool
            && let Ok(Some(bridge)) = connect()
        {
            let _ = bridge.retire(0, pool.generation);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gpu_canvas_bytes_match_cpu_quantization_without_dither_or_row_flip() {
        let gpu = pm_gpu::Gpu::new("WebM GPU encoder boundary").unwrap();
        struct OneBand(std::sync::atomic::AtomicU32);
        impl GpuGate for OneBand {
            fn admit(&self, _: u64, _: u32, _: &AtomicBool) -> Option<u32> {
                Some(1)
            }
            fn finished(&self, _: u64, units: u32, _: f64) {
                assert_eq!(units, 1);
                self.0.fetch_add(1, Ordering::Relaxed);
            }
            fn abandoned(&self, _: u64) {
                panic!("successful GPU conversion abandoned its admission");
            }
            fn measures(&self) -> bool {
                false
            }
        }
        let gate = Arc::new(OneBand(std::sync::atomic::AtomicU32::new(0)));
        let (w, h) = (256, 36);
        let mut encoder = GpuEncoder::new(gpu.device.clone(), gpu.queue.clone()).with_gate(
            gate.clone(),
            Arc::new(AtomicBool::new(false)),
            1,
        );
        encoder.pool = Some(
            SurfacePool::new(
                &gpu.device,
                1,
                Size::new(w, h),
                SurfaceFormat::Bgra8Unorm,
                SLOTS,
            )
            .unwrap(),
        );
        let source = gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("WebM boundary test"),
            size: wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba32Float,
            usage: wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let rgba: Vec<f32> = (0..w * h)
            .flat_map(|i| {
                let v = (i as f32 - 256.0) / 510.0;
                [v, 1.0 - v, (i % 256) as f32 / 255.0, 0.25]
            })
            .collect();
        let bytes: Vec<u8> = rgba.iter().flat_map(|v| v.to_ne_bytes()).collect();
        gpu.queue.write_texture(
            source.as_image_copy(),
            &bytes,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(w * 16),
                rows_per_image: Some(h),
            },
            source.size(),
        );
        encoder.draw(0, &source).unwrap();
        assert_eq!(gate.0.load(Ordering::Relaxed), 3);
        let pool = encoder.pool.as_ref().unwrap();
        let surface = &pool.surfaces[0];
        let actual = surface.iosurface.read();
        let stride = surface.iosurface.bytes_per_row() as usize;
        for y in 0..h as usize {
            for x in 0..w as usize {
                let p = (y * w as usize + x) * 4;
                let expected = pm_export::render::to_rgba8(&rgba[p..p + 4]);
                assert_eq!(
                    &actual[y * stride + x * 4..y * stride + x * 4 + 4],
                    &[expected[2], expected[1], expected[0], 255],
                    "pixel {x},{y}"
                );
            }
        }
        // No live bridge is used by this isolated GPU test.
        encoder.pool = None;
    }
}
