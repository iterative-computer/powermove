//! Attribution probe: the owned graphics memory macOS charges a process for
//! an active Metal context, against `MTLDevice.currentAllocatedSize`
//! (docs/engine/memory.md, "Active GPU context").
//!
//!   cargo run --release -p pm-gpu --example graphics_footprint -- <case>
//!
//! Cases: `blit` (a buffer copy), `compute` (one dispatch), `render64`,
//! `render-view` (1496 x 904 RGBA16F), `render-4k`, `render-8k` (one
//! cleared target each), `decay` (one small pass, then a timeline), `hold`
//! (a small pass every 400 ms for 10 s). Run each case in a fresh process:
//! the charge depends on what the context has done. Writes nothing.

use std::process::Command;
use std::time::{Duration, Instant};

/// (total footprint, every graphics category) in bytes, from `footprint`.
fn footprint() -> (u64, u64) {
    let out = Command::new("footprint")
        .args(["-p", &std::process::id().to_string(), "-f", "bytes"])
        .output()
        .expect("footprint");
    let text = String::from_utf8_lossy(&out.stdout);
    let mut total = 0;
    let mut graphics = 0;
    let first = |line: &str| -> u64 {
        line.split_whitespace()
            .next()
            .unwrap_or("0")
            .parse()
            .unwrap_or(0)
    };
    for line in text.lines() {
        if let Some(rest) = line.split("Footprint:").nth(1) {
            total = first(rest);
        } else if line.contains("(graphics)") {
            graphics += first(line);
        }
    }
    (total, graphics)
}

fn report(name: &str, device: &wgpu::Device) {
    let (total, graphics) = footprint();
    let mib = |b: u64| b as f64 / 1048576.0;
    println!(
        "{name:<24} footprint={:8.1} MiB graphics={:8.1} MiB metal-allocated={:8.1} MiB",
        mib(total),
        mib(graphics),
        pm_gpu::device::allocated_bytes(device).map_or(f64::NAN, mib)
    );
}

fn target(device: &wgpu::Device, w: u32, h: u32) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some("probe target"),
        size: wgpu::Extent3d {
            width: w,
            height: h,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba16Float,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    })
}

fn clear(device: &wgpu::Device, queue: &wgpu::Queue, texture: &wgpu::Texture) {
    let mut encoder = device.create_command_encoder(&Default::default());
    let view = texture.create_view(&Default::default());
    encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: None,
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view: &view,
            depth_slice: None,
            resolve_target: None,
            ops: wgpu::Operations {
                load: wgpu::LoadOp::Clear(wgpu::Color::RED),
                store: wgpu::StoreOp::Store,
            },
        })],
        ..Default::default()
    });
    queue.submit([encoder.finish()]);
    let _ = device.poll(wgpu::PollType::wait_indefinitely());
}

fn blit(device: &wgpu::Device, queue: &wgpu::Queue) {
    let make = |usage| {
        device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: 1 << 20,
            usage,
            mapped_at_creation: false,
        })
    };
    let from = make(wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::COPY_SRC);
    let to = make(wgpu::BufferUsages::COPY_DST);
    queue.write_buffer(&from, 0, &[1u8; 4096]);
    let mut encoder = device.create_command_encoder(&Default::default());
    encoder.copy_buffer_to_buffer(&from, 0, &to, 0, 4096);
    queue.submit([encoder.finish()]);
    let _ = device.poll(wgpu::PollType::wait_indefinitely());
}

fn compute(device: &wgpu::Device, queue: &wgpu::Queue) {
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: None,
        source: wgpu::ShaderSource::Wgsl(
            "@group(0) @binding(0) var<storage, read_write> b: array<u32>;\n\
             @compute @workgroup_size(64) fn main(@builtin(global_invocation_id) i: vec3<u32>) { b[i.x] = i.x; }"
                .into(),
        ),
    });
    let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: None,
        layout: None,
        module: &module,
        entry_point: Some("main"),
        compilation_options: Default::default(),
        cache: None,
    });
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: 64 * 1024 * 4,
        usage: wgpu::BufferUsages::STORAGE,
        mapped_at_creation: false,
    });
    let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: None,
        layout: &pipeline.get_bind_group_layout(0),
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: buffer.as_entire_binding(),
        }],
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    {
        let mut pass = encoder.begin_compute_pass(&Default::default());
        pass.set_pipeline(&pipeline);
        pass.set_bind_group(0, &bind, &[]);
        pass.dispatch_workgroups(1024, 1, 1);
    }
    queue.submit([encoder.finish()]);
    let _ = device.poll(wgpu::PollType::wait_indefinitely());
}

fn main() {
    let case = std::env::args().nth(1).unwrap_or_else(|| "decay".into());
    let gpu = pm_gpu::Gpu::new("graphics footprint probe").expect("Metal device");
    let (device, queue) = (&gpu.device, &gpu.queue);
    report("device", device);
    let mut kept = Vec::new();
    let mut render = |w, h| {
        let t = target(device, w, h);
        clear(device, queue, &t);
        kept.push(t);
    };
    match case.as_str() {
        "blit" => blit(device, queue),
        "compute" => compute(device, queue),
        "render64" | "decay" => render(64, 64),
        "render-view" => render(1496, 904),
        "render-4k" => render(3840, 2160),
        "render-8k" => render(7680, 4320),
        "hold" => {
            let t = target(device, 64, 64);
            for _ in 0..25 {
                clear(device, queue, &t);
                std::thread::sleep(Duration::from_millis(400));
            }
            report("hold, 400 ms after", device);
        }
        other => panic!("unknown case {other}"),
    }
    let start = Instant::now();
    let marks: &[u64] = if case == "decay" {
        &[0, 250, 500, 750, 1000, 1250, 1500, 2000, 3000]
    } else {
        &[0, 2500]
    };
    for &ms in marks {
        if let Some(rest) = Duration::from_millis(ms).checked_sub(start.elapsed()) {
            std::thread::sleep(rest);
        }
        report(&format!("{case} +{ms} ms"), device);
    }
    drop(kept);
}
