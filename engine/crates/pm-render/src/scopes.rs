//! GPU scopes: waveform, RGB parade, vectorscope and histogram of a finished
//! frame, for the Scopes panel. Counts come back through a buffer mapped off
//! the render thread; `pm_color::scopes` is their f64 reference.
//!
//! The standard range reads display code values. The extended range reads
//! BT.2100 PQ in BT.2020, so its axis carries absolute light.

/// Which signal the scopes read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScopeSignal {
    Display,
    Pq,
}

/// The luma and chroma weights.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScopeMatrix {
    Rec709,
    Rec2020,
}

impl ScopeMatrix {
    fn weights(self) -> [f32; 4] {
        match self {
            ScopeMatrix::Rec709 => [0.2126, 0.7152, 0.0722, 0.0],
            ScopeMatrix::Rec2020 => [0.2627, 0.6780, 0.0593, 0.0],
        }
    }
}

/// The counters' resolution and what they read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ScopeConfig {
    pub columns: u32,
    pub levels: u32,
    pub vector_bins: u32,
    pub signal: ScopeSignal,
    pub matrix: ScopeMatrix,
}

impl ScopeConfig {
    pub const PANEL: ScopeConfig = ScopeConfig {
        columns: 512,
        levels: 256,
        vector_bins: 256,
        signal: ScopeSignal::Display,
        matrix: ScopeMatrix::Rec709,
    };

    fn plane(&self) -> usize {
        self.levels as usize * self.columns as usize
    }

    fn counters(&self) -> usize {
        self.plane() * 4 + (self.vector_bins as usize).pow(2) + self.levels as usize * 4 + TOTALS
    }
}

/// Under, over, outside and not finite.
const TOTALS: usize = 4;

/// One frame's counts. Planes are rows of levels, low to high, then columns.
#[derive(Clone, Debug, PartialEq)]
pub struct ScopeFrame {
    pub config: ScopeConfig,
    pub pixels: u64,
    pub waveform: Vec<u32>,
    pub parade: [Vec<u32>; 3],
    /// Rows of Cr, low to high; columns of Cb.
    pub vectorscope: Vec<u32>,
    pub luma_histogram: Vec<u32>,
    pub histogram: [Vec<u32>; 3],
    /// Pixels with a channel below the range.
    pub under: u32,
    /// Pixels with a channel above the range.
    pub over: u32,
    /// Pixels outside the vectorscope.
    pub outside: u32,
    /// Pixels that aren't finite numbers.
    pub invalid: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct Params {
    a: [u32; 4],
    weights: [f32; 4],
    to_2020: [[f32; 4]; 3],
}

/// The analysis pipeline.
pub struct ScopePass {
    pipeline: wgpu::ComputePipeline,
}

/// Counts on their way back from the GPU.
pub struct ScopeReadback {
    buffer: wgpu::Buffer,
    config: ScopeConfig,
    pixels: u64,
}

impl ScopePass {
    pub fn new(device: &wgpu::Device) -> ScopePass {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("pm-render scopes"),
            source: wgpu::ShaderSource::Wgsl(
                format!(
                    "{}\n{}\n{}",
                    pm_color::wgsl::COLOR,
                    pm_color::tonemap::wgsl(),
                    include_str!("shaders/scopes.wgsl")
                )
                .into(),
            ),
        });
        ScopePass {
            pipeline: device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("pm-render scopes"),
                layout: None,
                module: &module,
                entry_point: Some("main"),
                compilation_options: Default::default(),
                cache: None,
            }),
        }
    }

    /// Analyzes `texture` (any float format, top-down rows) and submits.
    pub fn analyze(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        texture: &wgpu::Texture,
        config: ScopeConfig,
    ) -> ScopeReadback {
        let bytes = (config.counters() * size_of::<u32>()) as u64;
        let counters = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("pm scope counters"),
            size: bytes,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("pm scope readback"),
            size: bytes,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let m = pm_color::primaries::conversion(
            pm_protocol::color::Primaries::Bt709,
            pm_protocol::color::Primaries::Bt2020,
        )
        .expect("known primaries");
        let column = |j: usize| [m[0][j] as f32, m[1][j] as f32, m[2][j] as f32, 0.0];
        let params = Params {
            a: [
                config.columns,
                config.levels,
                config.vector_bins,
                u32::from(config.signal == ScopeSignal::Pq),
            ],
            weights: config.matrix.weights(),
            to_2020: [column(0), column(1), column(2)],
        };
        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("pm scope params"),
            size: size_of::<Params>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        queue.write_buffer(&uniform, 0, crate::pipelines::bytes_of(&params));
        let view = texture.create_view(&Default::default());
        let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("pm scopes"),
            layout: &self.pipeline.get_bind_group_layout(0),
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: uniform.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: counters.as_entire_binding(),
                },
            ],
        });
        let mut encoder = device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("pm scopes"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &bind, &[]);
            pass.dispatch_workgroups(texture.width().div_ceil(8), texture.height().div_ceil(8), 1);
        }
        encoder.copy_buffer_to_buffer(&counters, 0, &buffer, 0, bytes);
        queue.submit([encoder.finish()]);
        ScopeReadback {
            buffer,
            config,
            pixels: u64::from(texture.width()) * u64::from(texture.height()),
        }
    }
}

impl ScopeReadback {
    /// Waits for the GPU. Blocks: call it off the render and main threads.
    pub fn wait(self, device: &wgpu::Device) -> Result<ScopeFrame, String> {
        let (done, ready) = std::sync::mpsc::channel();
        self.buffer
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |result| {
                let _ = done.send(result);
            });
        device
            .poll(wgpu::PollType::wait_indefinitely())
            .map_err(|e| e.to_string())?;
        ready
            .recv()
            .map_err(|e| e.to_string())?
            .map_err(|e| e.to_string())?;
        let counts: Vec<u32> = {
            let data = self
                .buffer
                .slice(..)
                .get_mapped_range()
                .map_err(|e| e.to_string())?;
            data.chunks_exact(4)
                .map(|c| u32::from_ne_bytes([c[0], c[1], c[2], c[3]]))
                .collect()
        };
        self.buffer.unmap();
        let c = self.config;
        let (plane, levels) = (c.plane(), c.levels as usize);
        let vector = (c.vector_bins as usize).pow(2);
        let slice = |at: usize, n: usize| counts[at..at + n].to_vec();
        let histograms = plane * 4 + vector;
        Ok(ScopeFrame {
            config: c,
            pixels: self.pixels,
            waveform: slice(0, plane),
            parade: std::array::from_fn(|i| slice(plane * (i + 1), plane)),
            vectorscope: slice(plane * 4, vector),
            luma_histogram: slice(histograms, levels),
            histogram: std::array::from_fn(|i| slice(histograms + levels * (i + 1), levels)),
            under: counts[histograms + levels * 4],
            over: counts[histograms + levels * 4 + 1],
            outside: counts[histograms + levels * 4 + 2],
            invalid: counts[histograms + levels * 4 + 3],
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pm_color::scopes as reference;

    fn frame(gpu: &pm_gpu::Gpu, pixels: &[[f32; 4]], width: u32) -> pm_gpu::Target {
        let height = pixels.len() as u32 / width;
        let target = pm_gpu::Target::new(
            &gpu.device,
            "scope test frame",
            width,
            height,
            wgpu::TextureFormat::Rgba32Float,
            wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        );
        let bytes: Vec<u8> = pixels
            .iter()
            .flatten()
            .flat_map(|v| v.to_le_bytes())
            .collect();
        gpu.queue.write_texture(
            target.texture.as_image_copy(),
            &bytes,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(width * 16),
                rows_per_image: Some(height),
            },
            target.texture.size(),
        );
        target
    }

    fn u32s(values: &[u64]) -> Vec<u32> {
        values.iter().map(|v| *v as u32).collect()
    }

    #[test]
    fn gpu_counts_equal_the_f64_reference() {
        let gpu = pm_gpu::Gpu::new("scope test").unwrap();
        let pass = ScopePass::new(&gpu.device);
        // A deterministic, wide spread of colours, in and out of range.
        let width = 97u32;
        let pixels: Vec<[f32; 4]> = (0..97 * 61)
            .map(|i: u32| {
                let channel = |k: u32| (i.wrapping_mul(k) % 1409) as f32 / 1280.0 - 0.05;
                [channel(131), channel(311), channel(743), 1.0]
            })
            .collect();
        let target = frame(&gpu, &pixels, width);
        let config = ScopeConfig {
            columns: 64,
            levels: 128,
            vector_bins: 96,
            signal: ScopeSignal::Display,
            matrix: ScopeMatrix::Rec709,
        };
        let counts = pass
            .analyze(&gpu.device, &gpu.queue, &target.texture, config)
            .wait(&gpu.device)
            .unwrap();
        let rgb: Vec<[f64; 3]> = pixels
            .iter()
            .map(|p| [f64::from(p[0]), f64::from(p[1]), f64::from(p[2])])
            .collect();
        let expected = reference::compute(
            &rgb,
            width as usize,
            61,
            reference::ScopeConfig {
                columns: 64,
                levels: 128,
                vector_bins: 96,
                min: 0.0,
                max: 1.0,
                matrix: reference::Matrix::Bt709,
            },
        )
        .unwrap();
        for c in 0..3 {
            assert_eq!(counts.parade[c], u32s(&expected.parade[c]), "parade {c}");
            assert_eq!(
                counts.histogram[c],
                u32s(&expected.histogram[c].bins),
                "histogram {c}"
            );
        }
        // Luma and chroma are f32 against f64 here; a pixel on a bin boundary
        // may land next door, and nowhere further.
        let moved = |a: &[u32], b: &[u64]| -> u64 {
            a.iter()
                .zip(b)
                .map(|(a, b)| u64::from(*a).abs_diff(*b))
                .sum()
        };
        let limit = counts.pixels / 500;
        assert!(moved(&counts.waveform, &expected.waveform) <= limit);
        assert!(moved(&counts.vectorscope, &expected.vectorscope) <= limit);
        assert!(moved(&counts.luma_histogram, &expected.luma_histogram.bins) <= limit);
        assert_eq!(
            counts.waveform.iter().map(|v| u64::from(*v)).sum::<u64>(),
            counts.pixels
        );
        assert_eq!(
            u64::from(counts.outside),
            expected.vector_outside,
            "outside"
        );
        let any = |test: &dyn Fn(f64) -> bool| {
            rgb.iter().filter(|p| p.iter().any(|v| test(*v))).count() as u32
        };
        assert_eq!(counts.under, any(&|v| v < 0.0));
        assert_eq!(counts.over, any(&|v| v > 1.0));
        assert_eq!(counts.invalid, 0);
    }

    #[test]
    fn non_finite_pixels_are_counted_and_left_out_of_the_traces() {
        let gpu = pm_gpu::Gpu::new("scope invalid test").unwrap();
        let pass = ScopePass::new(&gpu.device);
        let mut pixels = [[0.5, 0.5, 0.5, 1.0]; 16];
        pixels[3] = [f32::NAN, 0.5, 0.5, 1.0];
        pixels[7] = [0.5, f32::INFINITY, 0.5, 1.0];
        let target = frame(&gpu, &pixels, 4);
        let counts = pass
            .analyze(&gpu.device, &gpu.queue, &target.texture, ScopeConfig::PANEL)
            .wait(&gpu.device)
            .unwrap();
        assert_eq!(counts.invalid, 2);
        assert_eq!(counts.luma_histogram.iter().sum::<u32>(), 14);
    }

    #[test]
    fn hdr_scopes_read_absolute_pq_light() {
        let gpu = pm_gpu::Gpu::new("hdr scope test").unwrap();
        let pass = ScopePass::new(&gpu.device);
        // Working light 1.0 is SDR white: 203 cd/m², PQ 0.5807 (BT.2408).
        let target = frame(&gpu, &[[1.0, 1.0, 1.0, 1.0]; 16], 4);
        let config = ScopeConfig {
            signal: ScopeSignal::Pq,
            matrix: ScopeMatrix::Rec2020,
            ..ScopeConfig::PANEL
        };
        let counts = pass
            .analyze(&gpu.device, &gpu.queue, &target.texture, config)
            .wait(&gpu.device)
            .unwrap();
        let bin = counts
            .luma_histogram
            .iter()
            .position(|v| *v == 16)
            .expect("one level");
        assert_eq!(bin, (0.5807f32 * 255.0).round() as usize);
    }
}
