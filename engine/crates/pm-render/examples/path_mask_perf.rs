//! An animated 1920 × 1080 bezier path mask, its vertices moving every
//! frame, at σ = 0, 10 and 60 px: GPU time of the coverage canvas per frame
//! (pass timestamps around the frame's mask commands, end-of-encoder
//! samples as `pm-export`'s `GpuSpan`) and the CPU time to encode it.
//! Run the release build: cargo run --release -p pm-render --example path_mask_perf

#[cfg(target_os = "macos")]
fn main() {
    use pm_render::composite::path_masks::{PathCoverage, PathMaskPipelines};
    use pm_vector::{LayerMasks, MaskMode, MaskShape, MaskValues, PathValues, Vertex};

    const FRAMES: usize = 240;
    let gpu = pm_gpu::Gpu::new("path mask perf").expect("a Metal device");
    let (device, queue) = (&gpu.device, &gpu.queue);
    assert!(
        device.features().contains(wgpu::Features::TIMESTAMP_QUERY),
        "the device writes no pass timestamps"
    );
    let period = f64::from(queue.get_timestamp_period());
    let set = device.create_query_set(&wgpu::QuerySetDescriptor {
        label: None,
        ty: wgpu::QueryType::Timestamp,
        count: 2,
    });
    let resolve = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: 16,
        usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });
    let read = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: 16,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let sink = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: 8,
        usage: wgpu::BufferUsages::STORAGE,
        mapped_at_creation: false,
    });
    // `end` reads a texel of the canvas, so it runs after the mask work.
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: None,
        source: wgpu::ShaderSource::Wgsl(
            "@group(0) @binding(0) var<storage, read_write> sink: array<f32>;
             @group(0) @binding(1) var after: texture_2d<f32>;
             @compute @workgroup_size(1) fn begin() { sink[0] = sink[0] + 1.0; }
             @compute @workgroup_size(1) fn end() { sink[1] = textureLoad(after, vec2<u32>(0u, 0u), 0).r; }"
                .into(),
        ),
    });
    let pipeline = |entry: &str| {
        device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: None,
            layout: None,
            module: &module,
            entry_point: Some(entry),
            compilation_options: Default::default(),
            cache: None,
        })
    };
    let (begin, end) = (pipeline("begin"), pipeline("end"));
    let stamp = |encoder: &mut wgpu::CommandEncoder,
                 p: &wgpu::ComputePipeline,
                 index: u32,
                 entries: &[wgpu::BindGroupEntry<'_>]| {
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &p.get_bind_group_layout(0),
            entries,
        });
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: None,
            timestamp_writes: Some(wgpu::ComputePassTimestampWrites {
                query_set: &set,
                beginning_of_pass_write_index: None,
                end_of_pass_write_index: Some(index),
            }),
        });
        pass.set_pipeline(p);
        pass.set_bind_group(0, &group, &[]);
        pass.dispatch_workgroups(1, 1, 1);
    };

    let pipelines = PathMaskPipelines::new(device);
    let mask = |frame: usize, feather: f64| {
        let t = frame as f64 * 0.05;
        let v = |x: f64, y: f64, ix: f64, iy: f64, ox: f64, oy: f64, k: f64| Vertex {
            x: x + 40.0 * (t + k).sin(),
            y: y + 30.0 * (1.3 * t + k).cos(),
            in_x: ix,
            in_y: iy,
            out_x: ox,
            out_y: oy,
        };
        let path = PathValues {
            x: 0.0,
            y: 0.0,
            rotation: 0.0,
            scale_x: 100.0,
            scale_y: 100.0,
            closed: true,
            vertices: vec![
                v(-600.0, -200.0, 0.0, 0.0, 300.0, -400.0, 0.0),
                v(300.0, -380.0, -200.0, 0.0, 400.0, 150.0, 1.0),
                v(650.0, 150.0, 0.0, -300.0, -100.0, 450.0, 2.0),
                v(-150.0, 400.0, 400.0, 0.0, -500.0, 0.0, 3.0),
            ],
            ..PathValues::default()
        };
        let LayerMasks::Path(masks) = LayerMasks::new(&[MaskValues {
            feather,
            shape: MaskShape::Rect,
            mode: MaskMode::Add,
            path: Some(path),
            ..MaskValues::default()
        }]) else {
            unreachable!()
        };
        PathCoverage {
            width: 1920,
            height: 1080,
            world: [1.0, 0.0, 0.0, 1.0, 960.0, 540.0],
            masks,
        }
    };
    let percentile = |v: &mut Vec<f64>, p: f64| {
        v.sort_by(f64::total_cmp);
        v[((v.len() - 1) as f64 * p).round() as usize]
    };
    for sigma in [0.0, 10.0, 60.0] {
        let (mut gpu_ms, mut cpu_ms) = (Vec::new(), Vec::new());
        for frame in 0..FRAMES + 10 {
            let cov = mask(frame, 2.0 * sigma);
            let mut encoder = device.create_command_encoder(&Default::default());
            stamp(
                &mut encoder,
                &begin,
                0,
                &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: sink.as_entire_binding(),
                }],
            );
            let started = std::time::Instant::now();
            let texture = pipelines.coverage(device, &mut encoder, &cov);
            let encoded = started.elapsed();
            let view = texture.create_view(&Default::default());
            stamp(
                &mut encoder,
                &end,
                1,
                &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: sink.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(&view),
                    },
                ],
            );
            queue.submit([encoder.finish()]);
            // Resolved once the frame completed: Metal publishes an
            // encoder's samples only then.
            device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
            let mut encoder = device.create_command_encoder(&Default::default());
            encoder.resolve_query_set(&set, 0..2, &resolve, 0);
            encoder.copy_buffer_to_buffer(&resolve, 0, &read, 0, 16);
            queue.submit([encoder.finish()]);
            read.slice(..).map_async(wgpu::MapMode::Read, |_| {});
            device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
            let ticks: Vec<u64> = read
                .slice(..)
                .get_mapped_range()
                .unwrap()
                .chunks_exact(8)
                .map(|c| u64::from_le_bytes(c.try_into().unwrap()))
                .collect();
            read.unmap();
            // The first frames compile the pipelines.
            if frame >= 10 {
                gpu_ms.push(ticks[1].saturating_sub(ticks[0]) as f64 * period / 1e6);
                cpu_ms.push(encoded.as_secs_f64() * 1e3);
            }
        }
        // Sustained: 60 frames in one submission (the GPU at its playback
        // clocks, not idling between isolated frames).
        let mut encoder = device.create_command_encoder(&Default::default());
        stamp(&mut encoder, &begin, 0, &[wgpu::BindGroupEntry { binding: 0, resource: sink.as_entire_binding() }]);
        let mut last = None;
        for frame in 0..60 {
            last = Some(pipelines.coverage(device, &mut encoder, &mask(frame, 2.0 * sigma)));
        }
        let view = last.unwrap().create_view(&Default::default());
        stamp(
            &mut encoder,
            &end,
            1,
            &[
                wgpu::BindGroupEntry { binding: 0, resource: sink.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&view) },
            ],
        );
        queue.submit([encoder.finish()]);
        device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        let mut encoder = device.create_command_encoder(&Default::default());
        encoder.resolve_query_set(&set, 0..2, &resolve, 0);
        encoder.copy_buffer_to_buffer(&resolve, 0, &read, 0, 16);
        queue.submit([encoder.finish()]);
        read.slice(..).map_async(wgpu::MapMode::Read, |_| {});
        device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        let ticks: Vec<u64> = read
            .slice(..)
            .get_mapped_range()
            .unwrap()
            .chunks_exact(8)
            .map(|c| u64::from_le_bytes(c.try_into().unwrap()))
            .collect();
        read.unmap();
        let sustained = ticks[1].saturating_sub(ticks[0]) as f64 * period / 1e6 / 60.0;
        println!(
            "σ {sigma:>4} px: sustained GPU {sustained:.3} ms/frame | GPU p50 {:.3} ms p95 {:.3} ms max {:.3} ms | CPU encode p50 {:.3} ms p95 {:.3} ms ({FRAMES} frames, 1920x1080)",
            percentile(&mut gpu_ms, 0.5),
            percentile(&mut gpu_ms, 0.95),
            percentile(&mut gpu_ms, 1.0),
            percentile(&mut cpu_ms, 0.5),
            percentile(&mut cpu_ms, 0.95),
        );
    }
}

#[cfg(not(target_os = "macos"))]
fn main() {}
