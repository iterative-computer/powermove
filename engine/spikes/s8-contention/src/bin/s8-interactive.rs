//! Process A: the interactive engine stand-in.
//!
//! Renders a 4K playback frame (3 layers composited, a separable blur, a display
//! transform into an 8-bit output surface from a ring) on a display-rate clock, and
//! measures when each frame's GPU work completes relative to its presentation
//! deadline. Pacing uses a real-time (time-constraint) thread and `mach_wait_until`
//! on absolute vsync ticks, the way a CVDisplayLink thread runs; no window is shown and
//! focus is never taken. Section 18's frame bridge has the engine render into a pool of
//! shared surfaces and send a descriptor after the command buffer completes, so
//! "completed before the vsync it is meant for" is the presentation criterion here.
//!
//! Signals to the job process (contract 13): playback start/stop, per-frame ok,
//! at-risk (not complete at `risk-frac` of the period), and miss (not complete at the
//! deadline), sent the moment they are known rather than after completion.

use s8_contention::display::{self, DisplayLink};
use s8_contention::present::{self, Presenter};
use s8_contention::{Args, clock, gpu, signal, stats, sys};
use serde::Serialize;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, mpsc};

const RING: u32 = 8;

const VS: &str = r#"
@vertex
fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    let x = f32((i << 1u) & 2u);
    let y = f32(i & 2u);
    return vec4<f32>(x * 2.0 - 1.0, 1.0 - y * 2.0, 0.0, 1.0);
}
"#;

const FS_GEN: &str = r#"
struct Gen { layer: u32, _a: u32, _b: u32, _c: u32 };
@group(0) @binding(0) var<uniform> gen: Gen;

@fragment
fn fs_gen(@builtin(position) p: vec4<f32>) -> @location(0) vec4<f32> {
    let l = f32(gen.layer);
    let uv = p.xy / vec2<f32>(3840.0, 2160.0);
    let c = uv - vec2<f32>(0.5, 0.5);
    let r = length(c * vec2<f32>(1.7778, 1.0));
    let rings = 0.5 + 0.5 * sin(r * (60.0 + 20.0 * l));
    let check = f32((u32(p.x / (32.0 + 16.0 * l)) + u32(p.y / (32.0 + 16.0 * l))) & 1u);
    let rgb = vec3<f32>(uv.x, uv.y * rings, mix(rings, check, 0.5)) * (0.6 + 0.2 * l);
    let a = select(1.0, clamp(1.2 - r * 2.0, 0.0, 1.0), gen.layer > 0u);
    return vec4<f32>(rgb * a, a);
}
"#;

const FS_COMP: &str = r#"
struct Comp {
    dims: vec2<f32>,
    layers: u32,
    _pad: u32,
    xf: array<vec4<f32>, 8>,   // scale, angle, tx, ty
    op: array<vec4<f32>, 8>,   // opacity in x
};
@group(0) @binding(0) var<uniform> comp: Comp;
@group(0) @binding(1) var samp: sampler;
@group(0) @binding(2) var layers: texture_2d_array<f32>;

@fragment
fn fs_comp(@builtin(position) p: vec4<f32>) -> @location(0) vec4<f32> {
    let uv = p.xy / comp.dims;
    var acc = vec4<f32>(0.0);
    for (var i = 0u; i < comp.layers; i = i + 1u) {
        let x = comp.xf[i];
        let c = uv - vec2<f32>(0.5, 0.5) - x.zw;
        let cs = cos(-x.y);
        let sn = sin(-x.y);
        let q = vec2<f32>(c.x * cs - c.y * sn, c.x * sn + c.y * cs) / x.x + vec2<f32>(0.5, 0.5);
        let inside = all(q >= vec2<f32>(0.0)) && all(q <= vec2<f32>(1.0));
        let s = textureSampleLevel(layers, samp, q, i32(i), 0.0) * select(0.0, comp.op[i].x, inside);
        acc = s + acc * (1.0 - s.a);
    }
    return acc;
}
"#;

const FS_BLUR: &str = r#"
struct Blur { dir: vec2<i32>, radius: i32, _pad: i32 };
@group(0) @binding(0) var<uniform> blur: Blur;
@group(0) @binding(1) var src: texture_2d<f32>;

@fragment
fn fs_blur(@builtin(position) p: vec4<f32>) -> @location(0) vec4<f32> {
    let dims = vec2<i32>(textureDimensions(src));
    let base = vec2<i32>(p.xy);
    let sigma = max(f32(blur.radius) / 2.5, 0.5);
    var acc = vec4<f32>(0.0);
    var wsum = 0.0;
    for (var k = -blur.radius; k <= blur.radius; k = k + 1) {
        let q = clamp(base + blur.dir * k, vec2<i32>(0), dims - vec2<i32>(1));
        let w = exp(-0.5 * f32(k * k) / (sigma * sigma));
        acc = acc + textureLoad(src, q, 0) * w;
        wsum = wsum + w;
    }
    return acc / wsum;
}
"#;

const FS_OUT: &str = r#"
@group(0) @binding(0) var outsrc: texture_2d<f32>;

fn oetf(c: f32) -> f32 {
    let v = clamp(c, 0.0, 1.0);
    return select(1.055 * pow(v, 1.0 / 2.4) - 0.055, 12.92 * v, v <= 0.0031308);
}

@fragment
fn fs_out(@builtin(position) p: vec4<f32>) -> @location(0) vec4<f32> {
    let c = textureLoad(outsrc, vec2<i32>(p.xy), 0);
    let rgb = select(c.rgb / c.a, vec3<f32>(0.0), c.a <= 0.0);
    let h = fract(sin(dot(p.xy, vec2<f32>(12.9898, 78.233))) * 43758.5453) - 0.5;
    let enc = vec3<f32>(oetf(rgb.r), oetf(rgb.g), oetf(rgb.b)) * c.a + vec3<f32>(h / 255.0);
    return vec4<f32>(enc, c.a);
}
"#;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct CompU {
    dims: [f32; 2],
    layers: u32,
    _pad: u32,
    xf: [[f32; 4]; 8],
    op: [[f32; 4]; 8],
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct BlurU {
    dir: [i32; 2],
    radius: i32,
    _pad: i32,
}

#[derive(Clone)]
struct Submitted {
    n: u64,
    slot: u32,
    tick: u64,
    wake: u64,
    submit: u64,
    index: wgpu::SubmissionIndex,
}

#[derive(Clone, Copy, Serialize)]
struct FrameRec {
    n: u64,
    t_s: f64,
    wake_late_ms: f64,
    encode_ms: f64,
    latency_ms: f64,
    gpu_ms: f64,
    /// GPU start of the frame's first pass minus CPU submit time: queueing delay.
    gpu_start_ms: f64,
    skipped: bool,
    /// Absolute host times (ns) for the raw log only: tick, submit, GPU begin, GPU end,
    /// done (NaN where unknown). Used to attribute late frames to whatever else had the
    /// GPU at the time (B's command buffers are logged on the same clock).
    #[serde(skip)]
    abs: [f64; 5],
}

#[derive(Serialize)]
struct Drops {
    /// Frame not complete by the next vsync after its tick (no render-ahead; the
    /// gesture/scrub path and the strictest playback model).
    deadline_1_frame: usize,
    /// Two frames of render-ahead.
    deadline_2_frames: usize,
    /// Three frames of render-ahead.
    deadline_3_frames: usize,
    /// Frames never submitted because the output ring was still busy.
    skipped_ring_full: usize,
    /// Frames never submitted because the pacing thread woke after the frame's vsync
    /// had already passed (display-link clock only).
    skipped_pacing: usize,
}

/// Presentation through the on-screen layer (`--present`).
#[derive(Serialize)]
struct Presentation {
    placement: String,
    /// AppKit's occlusion state for the window just before the run.
    window_visible: bool,
    /// Presented-handler invocations for measured frames (including skipped frames).
    handler_calls: usize,
    /// Frames whose drawable reported a presented host time.
    presented: usize,
    /// Frames the compositor never showed (presentedTime 0 or no handler call).
    not_presented: usize,
    /// Consecutive presented frames whose spacing differs from the frame period by more
    /// than half a display refresh: a repeat or a skip on the glass.
    irregular_intervals: usize,
    /// Presented time minus the frame's vsync tick.
    present_latency_ms: stats::Dist,
    /// Spacing between consecutive presented frames.
    present_interval_ms: stats::Dist,
    drawable_wait_ms: stats::Dist,
}

#[derive(Serialize)]
struct Summary {
    process: &'static str,
    label: String,
    adapter: String,
    width: u32,
    height: u32,
    fps: f64,
    seconds: f64,
    warmup_seconds: f64,
    layers: u32,
    blur_radius: i32,
    realtime_thread: bool,
    /// `grid` (absolute timer on the vsync grid), `display` (every k-th CVDisplayLink
    /// callback), or `timer` (free-running absolute timer).
    clock: &'static str,
    /// Refresh period measured from display-link callbacks before the run.
    display_refresh_ms: Option<f64>,
    /// CoreVideo's nominal refresh period.
    nominal_refresh_ms: Option<f64>,
    refreshes_per_frame: Option<u64>,
    gpu_priority: Option<usize>,
    frames_measured: usize,
    drops: Drops,
    at_risk: usize,
    latency_ms: stats::Dist,
    gpu_exec_ms: stats::Dist,
    gpu_start_delay_ms: stats::Dist,
    encode_ms: stats::Dist,
    wake_late_ms: stats::Dist,
    completion_jitter_ms: stats::Dist,
    worst: Vec<FrameRec>,
    miss_times_s: Vec<f64>,
    signals_sent: u64,
    signals_dropped: u64,
    presentation: Option<Presentation>,
}

struct Pipelines {
    comp: wgpu::RenderPipeline,
    blur: wgpu::RenderPipeline,
    out: wgpu::RenderPipeline,
    out_bgra: wgpu::RenderPipeline,
}

fn main() {
    let args = Args::from_env();
    let width: u32 = args.num("width", 3840);
    let height: u32 = args.num("height", 2160);
    let g = gpu::create("s8-interactive");
    let Some(placement) = args.str("present") else {
        run(&args, g, None);
        return;
    };
    let placement = present::Placement::parse(placement).expect("--present corner|offscreen");
    let mtm = present::main_thread().expect("main thread");
    let raw_device = {
        // SAFETY: the device outlives the layer that references the raw MTLDevice.
        let hal = unsafe { g.device.as_hal::<wgpu::hal::api::Metal>() }.expect("Metal device");
        hal.raw_device().clone()
    };
    let points: (f64, f64) = (args.num("window-width", 192.0), args.num("window-height", 108.0));
    let window = present::create_window(mtm, &raw_device, width, height, placement, points);
    let visible = window.visible();
    let presenter = Presenter::new(window.layer.clone(), width, height);
    let done = Arc::new(AtomicBool::new(false));
    let worker = {
        let done = done.clone();
        let args = Args::parse(std::env::args().skip(1));
        std::thread::spawn(move || {
            run(
                &args,
                g,
                Some((presenter, format!("{placement:?}").to_lowercase(), visible)),
            );
            done.store(true, Ordering::Release);
        })
    };
    present::pump_until(mtm, &done);
    worker.join().unwrap();
    drop(window);
}

fn run(args: &Args, g: gpu::Gpu, presenter: Option<(Presenter, String, bool)>) {
    let width: u32 = args.num("width", 3840);
    let height: u32 = args.num("height", 2160);
    let mut fps: f64 = args.num("fps", 60.0);
    let seconds: f64 = args.num("seconds", 60.0);
    let warmup: f64 = args.num("warmup", 2.0);
    let layers: u32 = args.num("layers", 3);
    let radius: i32 = args.num("blur-radius", 8);
    let risk_frac: f64 = args.num("risk-frac", 0.75);
    let label = args.string_or("label", "a");
    let realtime = !args.flag("no-rt");
    let raw_path = args.str("raw").map(str::to_string);
    let fifo = args.str("signal-fifo").map(str::to_string);
    let clock_kind = args.string_or("clock", "grid");
    assert!((1..=8).contains(&layers));

    // `grid` (default): an absolute timer on the panel's vsync grid. It is anchored once
    // on a display-link vsync (`output` host time, which is exact on an idle machine) and
    // advances by k nominal refreshes per frame. `display`: frames start on every k-th
    // display-link callback; under GPU load the callbacks on this machine stray by
    // several milliseconds and arrive at ~114 Hz instead of 120 Hz (`vsync_probe`
    // example), so deadlines measured against them are noisy. `timer`: a free-running
    // 60 Hz timer with no display at all.
    let grid = clock_kind == "grid";
    let mut dl = match clock_kind.as_str() {
        "display" | "grid" => Some(DisplayLink::start().expect("display link")),
        "timer" => None,
        c => panic!("--clock grid|display|timer, not {c}"),
    };
    let nominal_ns = dl.as_ref().map(|d| d.refresh_ns).filter(|r| r.is_finite());
    let refresh_ns = dl.as_ref().map(|d| d.measure_refresh(120));
    let k = dl.as_ref().zip(refresh_ns).map(|(d, r)| {
        // Pick k from the nominal period when CoreVideo reports one.
        let basis = if d.refresh_ns.is_finite() { d.refresh_ns } else { r };
        display::refreshes_per_frame(basis, fps)
            .unwrap_or_else(|| panic!("{fps} fps is not a whole number of {:.3} ms refreshes", basis / 1e6))
    });
    if let (Some(r), Some(k)) = (refresh_ns, k) {
        // Deadlines follow the panel's rate, not the nominal content rate: the nominal
        // refresh on the grid clock, the measured one on the display-link clock.
        let basis = if grid { nominal_ns.unwrap_or(r) } else { r };
        fps = 1e9 / (basis * k as f64);
    }
    let gpu_priority = match args.str("gpu-priority") {
        Some(p) => Some(gpu::set_private_gpu_priority(&g.queue, p.parse().unwrap()).expect("gpu priority")),
        None => gpu::get_private_gpu_priority(&g.queue),
    };
    let device = &g.device;
    let queue = &g.queue;

    // Resources.
    let tex = |label: &str, format, layers: u32, usage| {
        device.create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: layers,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage,
            view_formats: &[],
        })
    };
    let rt = wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING;
    let layer_tex = tex("layers", wgpu::TextureFormat::Rgba16Float, layers, rt);
    let comp_tex = tex("comp", wgpu::TextureFormat::Rgba16Float, 1, rt);
    let tmp_tex = tex("tmp", wgpu::TextureFormat::Rgba16Float, 1, rt);
    let out_tex: Vec<_> = (0..RING)
        .map(|_| {
            tex(
                "out",
                wgpu::TextureFormat::Rgba8Unorm,
                1,
                rt | wgpu::TextureUsages::COPY_SRC,
            )
        })
        .collect();
    let layers_view = layer_tex.create_view(&wgpu::TextureViewDescriptor {
        dimension: Some(wgpu::TextureViewDimension::D2Array),
        ..Default::default()
    });
    let comp_view = comp_tex.create_view(&Default::default());
    let tmp_view = tmp_tex.create_view(&Default::default());
    let out_views: Vec<_> = out_tex.iter().map(|t| t.create_view(&Default::default())).collect();

    let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        ..Default::default()
    });
    let pipeline = |entry: &str, fs: &str, format| {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some(entry),
            source: wgpu::ShaderSource::Wgsl(format!("{VS}{fs}").into()),
        });
        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some(entry),
            layout: None,
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
                entry_point: Some(entry),
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
    let p = Pipelines {
        comp: pipeline("fs_comp", FS_COMP, wgpu::TextureFormat::Rgba16Float),
        blur: pipeline("fs_blur", FS_BLUR, wgpu::TextureFormat::Rgba16Float),
        out: pipeline("fs_out", FS_OUT, wgpu::TextureFormat::Rgba8Unorm),
        out_bgra: pipeline("fs_out", FS_OUT, wgpu::TextureFormat::Bgra8Unorm),
    };
    let gen_pipe = pipeline("fs_gen", FS_GEN, wgpu::TextureFormat::Rgba16Float);

    // Generate the "decoded" layer frames once.
    {
        let mut enc = device.create_command_encoder(&Default::default());
        let mut keep = Vec::new();
        for l in 0..layers {
            let ub = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("gen"),
                size: 16,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            queue.write_buffer(&ub, 0, bytemuck::cast_slice(&[l, 0u32, 0, 0]));
            let bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: None,
                layout: &gen_pipe.get_bind_group_layout(0),
                entries: &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: ub.as_entire_binding(),
                }],
            });
            let view = layer_tex.create_view(&wgpu::TextureViewDescriptor {
                dimension: Some(wgpu::TextureViewDimension::D2),
                base_array_layer: l,
                array_layer_count: Some(1),
                ..Default::default()
            });
            {
                let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("gen"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &view,
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
                pass.set_pipeline(&gen_pipe);
                pass.set_bind_group(0, &bg, &[]);
                pass.draw(0..3, 0..1);
            }
            keep.push((ub, bg, view));
        }
        queue.submit([enc.finish()]);
        device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
    }

    let uni = |size: u64| {
        device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        })
    };
    let comp_ub = uni(std::mem::size_of::<CompU>() as u64);
    let blur_h = uni(16);
    let blur_v = uni(16);
    queue.write_buffer(
        &blur_h,
        0,
        bytemuck::bytes_of(&BlurU {
            dir: [1, 0],
            radius,
            _pad: 0,
        }),
    );
    queue.write_buffer(
        &blur_v,
        0,
        bytemuck::bytes_of(&BlurU {
            dir: [0, 1],
            radius,
            _pad: 0,
        }),
    );
    let comp_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: None,
        layout: &p.comp.get_bind_group_layout(0),
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: comp_ub.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::Sampler(&sampler),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: wgpu::BindingResource::TextureView(&layers_view),
            },
        ],
    });
    let blur_bg = |ub: &wgpu::Buffer, src: &wgpu::TextureView| {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &p.blur.get_bind_group_layout(0),
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: ub.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(src),
                },
            ],
        })
    };
    let blur_h_bg = blur_bg(&blur_h, &comp_view);
    let blur_v_bg = blur_bg(&blur_v, &tmp_view);
    let out_bg_for = |pipe: &wgpu::RenderPipeline| {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &pipe.get_bind_group_layout(0),
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&comp_view),
            }],
        })
    };
    let out_bg = out_bg_for(&p.out);
    let out_bgra_bg = out_bg_for(&p.out_bgra);
    let total_frames = ((seconds + warmup) * fps).round() as u64;
    const WARM_PIPE: u64 = 10;
    let mut ts = gpu::TsLog::new(&g, (total_frames + WARM_PIPE) as usize);

    let pass = |enc: &mut wgpu::CommandEncoder,
                view: &wgpu::TextureView,
                pipe: &wgpu::RenderPipeline,
                bg: &wgpu::BindGroup,
                tsw: Option<wgpu::RenderPassTimestampWrites<'_>>| {
        let mut rp = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: None,
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: tsw,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        rp.set_pipeline(pipe);
        rp.set_bind_group(0, bg, &[]);
        rp.draw(0..3, 0..1);
    };

    // `target`: the presented drawable's view; otherwise the output ring slot.
    let encode_frame = |n: u64, slot: u32, entry: usize, target: Option<&wgpu::TextureView>| -> wgpu::CommandBuffer {
        let t = n as f32 / fps as f32;
        let mut u = CompU {
            dims: [width as f32, height as f32],
            layers,
            _pad: 0,
            xf: [[1.0, 0.0, 0.0, 0.0]; 8],
            op: [[1.0, 0.0, 0.0, 0.0]; 8],
        };
        for i in 1..layers as usize {
            let fi = i as f32;
            u.xf[i] = [
                0.6 + 0.1 * (t * 0.7 + fi).sin(),
                0.2 * (t * 0.5 * fi).sin(),
                0.15 * (t + fi).cos(),
                0.1 * (t * 1.3 + fi).sin(),
            ];
            u.op[i] = [0.85, 0.0, 0.0, 0.0];
        }
        queue.write_buffer(&comp_ub, 0, bytemuck::bytes_of(&u));
        let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("frame") });
        let tsr = ts.as_ref();
        pass(
            &mut enc,
            &comp_view,
            &p.comp,
            &comp_bg,
            tsr.map(|t| t.render_writes(entry, true, false)),
        );
        pass(&mut enc, &tmp_view, &p.blur, &blur_h_bg, None);
        pass(&mut enc, &comp_view, &p.blur, &blur_v_bg, None);
        let (view, pipe, bg) = match target {
            Some(v) => (v, &p.out_bgra, &out_bgra_bg),
            None => (&out_views[slot as usize], &p.out, &out_bg),
        };
        pass(
            &mut enc,
            view,
            pipe,
            bg,
            tsr.map(|t| t.render_writes(entry, false, true)),
        );
        enc.finish()
    };

    // Warm the pipelines.
    for i in 0..WARM_PIPE {
        queue.submit([encode_frame(i, (i % RING as u64) as u32, i as usize, None)]);
        device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
    }

    let period_ns = (1e9 / fps) as u64;
    let risk_ns = (risk_frac * period_ns as f64) as u64;
    let warm_frames = (warmup * fps).round() as u64;

    // Slot n % RING is free once the last frame submitted into it has been processed.
    let processed: Arc<Vec<AtomicU64>> = Arc::new((0..RING).map(|_| AtomicU64::new(u64::MAX)).collect());
    let (tx, rx) = mpsc::channel::<Submitted>();
    let mut sender = fifo.as_deref().map(|f| signal::Sender::open(f).expect("open fifo"));
    if let Some(s) = sender.as_mut() {
        s.send(signal::Kind::PlaybackStart);
    }

    // Completion thread: waits on each submission in order, signalling at-risk and miss
    // as soon as the corresponding moment passes without completion.
    let completion = {
        let device = device.clone();
        let processed = processed.clone();
        std::thread::spawn(move || {
            let _ = sys::set_thread_qos(sys::Qos::UserInteractive);
            let mut recs = Vec::new();
            let mut sender = sender;
            let wait_until = |idx: &wgpu::SubmissionIndex, until_ns: Option<u64>| -> bool {
                let timeout = until_ns.map(|u| std::time::Duration::from_nanos(u.saturating_sub(clock::now_ns())));
                match device.poll(wgpu::PollType::Wait {
                    submission_index: Some(idx.clone()),
                    timeout,
                }) {
                    Ok(_) => true,
                    Err(wgpu::PollError::Timeout) => false,
                    Err(e) => panic!("poll: {e}"),
                }
            };
            while let Ok(s) = rx.recv() {
                let deadline = s.tick + period_ns;
                let mut kind = signal::Kind::FrameOk;
                if !wait_until(&s.index, Some(s.tick + risk_ns)) {
                    kind = signal::Kind::AtRisk;
                    if let Some(tx) = sender.as_mut() {
                        tx.send_tick(signal::Kind::AtRisk, s.tick);
                    }
                    if !wait_until(&s.index, Some(deadline)) {
                        kind = signal::Kind::Miss;
                        if let Some(tx) = sender.as_mut() {
                            tx.send_tick(signal::Kind::Miss, s.tick);
                        }
                        wait_until(&s.index, None);
                    }
                }
                let done = clock::now_ns();
                if let Some(tx) = sender.as_mut() {
                    let done_kind = if kind == signal::Kind::FrameOk {
                        signal::Kind::FrameOk
                    } else {
                        signal::Kind::LateDone
                    };
                    tx.send_tick(done_kind, s.tick);
                }
                processed[s.slot as usize].store(s.n, Ordering::Release);
                recs.push((s, done));
            }
            if let Some(tx) = sender.as_mut() {
                tx.send(signal::Kind::PlaybackStop);
            }
            let (sent, dropped) = sender.as_ref().map(|s| (s.sent, s.dropped)).unwrap_or((0, 0));
            (recs, sent, dropped)
        })
    };

    // Pacing thread (this one): real-time, absolute vsync ticks.
    let _ = sys::set_thread_qos(sys::Qos::UserInteractive);
    let rt_ok = realtime && sys::set_thread_realtime(period_ns, 3_000_000, period_ns).is_ok();
    let mut skipped = Vec::new();
    // Last frame submitted into each ring slot. A slot is free once that frame has been
    // processed; comparing against `n - RING` instead would wedge a slot forever after
    // one skipped frame.
    let mut last_in_slot: [Option<u64>; RING as usize] = [None; RING as usize];
    let mut drawable_wait = Vec::new();
    // Display clock: vsync index counted from the first vsync, advanced by the number of
    // refreshes between successive callbacks (robust to a missed callback).
    let mut vs_idx: u64 = 0;
    let mut vs_prev: u64 = 0;
    let frame_ns = 1e9 / fps;
    let t0 = match &dl {
        Some(d) if grid => {
            while d.rx.try_recv().is_ok() {}
            let v = d.rx.recv().expect("vsync");
            // First grid point at least 2 ms ahead.
            let mut t = v.output_ns as f64;
            while t < (clock::now_ns() + 2_000_000) as f64 {
                t += frame_ns;
            }
            t as u64
        }
        Some(d) => {
            while d.rx.try_recv().is_ok() {}
            let v = d.rx.recv().expect("vsync");
            vs_prev = v.now_ns;
            v.now_ns
        }
        None => clock::now_ns() + 200_000_000,
    };
    if grid {
        // The link was only needed for the anchor.
        drop(dl.take());
    }
    for n in 0..total_frames {
        let tick = match (&dl, refresh_ns, k) {
            _ if grid => {
                let tick = t0 + (n as f64 * frame_ns).round() as u64;
                clock::sleep_until_ns(tick);
                tick
            }
            (Some(d), Some(r), Some(k)) => {
                let target = n * k;
                while vs_idx < target {
                    let v = d.rx.recv().expect("vsync");
                    vs_idx += ((v.now_ns - vs_prev) as f64 / r).round().max(1.0) as u64;
                    vs_prev = v.now_ns;
                }
                if vs_idx > target {
                    // This frame's vsync passed before we saw it.
                    skipped.push((n, t0 + (target as f64 * r) as u64, clock::now_ns(), true));
                    continue;
                }
                vs_prev
            }
            _ => {
                let tick = t0 + n * period_ns;
                clock::sleep_until_ns(tick);
                tick
            }
        };
        let wake = clock::now_ns();
        let slot = (n % RING as u64) as u32;
        let prev = processed[slot as usize].load(Ordering::Acquire);
        let free = last_in_slot[slot as usize].is_none_or(|m| prev == m);
        if !free {
            skipped.push((n, tick, wake, false));
            continue;
        }
        let frame = presenter.as_ref().map(|(pr, ..)| {
            let t = clock::now_ns();
            let f = pr.acquire(device).expect("nextDrawable timed out");
            drawable_wait.push((clock::now_ns() - t) as f64 / 1e6);
            f
        });
        let view = frame.as_ref().map(|f| f.texture.create_view(&Default::default()));
        let cb = encode_frame(n, slot, (n + WARM_PIPE) as usize, view.as_ref());
        let submit = clock::now_ns();
        let index = queue.submit([cb]);
        last_in_slot[slot as usize] = Some(n);
        if let (Some((pr, ..)), Some(f)) = (presenter.as_ref(), frame) {
            drop(view);
            pr.present(queue, f, n);
        }
        tx.send(Submitted {
            n,
            slot,
            tick,
            wake,
            submit,
            index,
        })
        .unwrap();
    }
    drop(tx);
    let (recs, signals_sent, signals_dropped) = completion.join().unwrap();
    // GPU timestamps, resolved only now so the measured run carries no extra submissions.
    let mut gpu_times = vec![None; (total_frames + WARM_PIPE) as usize];
    if let Some(t) = ts.as_mut() {
        for (e, v) in t.finish(device, queue, (total_frames + WARM_PIPE) as usize) {
            if e < gpu_times.len() {
                gpu_times[e] = v;
            }
        }
    }

    // Analysis (measured frames only).
    let mut frames: Vec<FrameRec> = Vec::new();
    let mut d1 = 0;
    let mut d2 = 0;
    let mut d3 = 0;
    let mut at_risk = 0;
    let mut miss_times = Vec::new();
    let mut dones: Vec<u64> = Vec::new();
    for (s, done) in &recs {
        let pair = gpu_times[(s.n + WARM_PIPE) as usize];
        let gpu_ms = pair.map(|(b, e)| (e - b) / 1e6).unwrap_or(f64::NAN);
        let gpu_start_ms = pair.map(|(b, _)| (b - s.submit as f64) / 1e6).unwrap_or(f64::NAN);
        if s.n < warm_frames {
            continue;
        }
        let lat = done - s.tick;
        if lat > period_ns {
            d1 += 1;
            miss_times.push(stats::r((s.tick - t0) as f64 / 1e9 - warmup, 3));
        }
        if lat > 2 * period_ns {
            d2 += 1;
        }
        if lat > 3 * period_ns {
            d3 += 1;
        }
        if lat > risk_ns {
            at_risk += 1;
        }
        dones.push(*done);
        frames.push(FrameRec {
            n: s.n,
            t_s: stats::r((s.tick - t0) as f64 / 1e9 - warmup, 4),
            wake_late_ms: (s.wake - s.tick) as f64 / 1e6,
            encode_ms: (s.submit - s.wake) as f64 / 1e6,
            latency_ms: lat as f64 / 1e6,
            gpu_ms,
            gpu_start_ms,
            skipped: false,
            abs: [
                s.tick as f64,
                s.submit as f64,
                pair.map(|p| p.0).unwrap_or(f64::NAN),
                pair.map(|p| p.1).unwrap_or(f64::NAN),
                *done as f64,
            ],
        });
    }
    let skipped_measured: Vec<_> = skipped.iter().filter(|(n, ..)| *n >= warm_frames).collect();
    let skipped_pacing_measured = skipped_measured.iter().filter(|s| s.3).count();
    for (n, tick, wake, _) in &skipped_measured {
        frames.push(FrameRec {
            n: *n,
            t_s: stats::r((tick - t0) as f64 / 1e9 - warmup, 4),
            wake_late_ms: (wake - tick) as f64 / 1e6,
            encode_ms: 0.0,
            latency_ms: f64::INFINITY,
            gpu_ms: f64::NAN,
            gpu_start_ms: f64::NAN,
            skipped: true,
            abs: [*tick as f64, f64::NAN, f64::NAN, f64::NAN, f64::NAN],
        });
        miss_times.push(stats::r((tick - t0) as f64 / 1e9 - warmup, 3));
    }
    let completed: Vec<&FrameRec> = frames.iter().filter(|f| !f.skipped).collect();
    let pick =
        |f: fn(&FrameRec) -> f64| -> Vec<f64> { completed.iter().map(|r| f(r)).filter(|v| v.is_finite()).collect() };
    let jitter: Vec<f64> = dones
        .windows(2)
        .map(|w| ((w[1] - w[0]) as f64 - period_ns as f64).abs() / 1e6)
        .collect();
    let mut worst: Vec<FrameRec> = frames.clone();
    worst.sort_by(|a, b| b.latency_ms.partial_cmp(&a.latency_ms).unwrap());
    worst.truncate(10);
    for w in &mut worst {
        w.latency_ms = stats::r(w.latency_ms, 3);
        w.gpu_ms = stats::r(w.gpu_ms, 3);
        w.gpu_start_ms = stats::r(w.gpu_start_ms, 3);
        w.wake_late_ms = stats::r(w.wake_late_ms, 3);
        w.encode_ms = stats::r(w.encode_ms, 3);
    }
    miss_times.sort_by(|a, b| a.partial_cmp(b).unwrap());
    miss_times.truncate(200);

    if let Some(path) = raw_path {
        let mut out = String::new();
        let mut all = frames.clone();
        all.sort_by_key(|f| f.n);
        for f in &all {
            let mut v = serde_json::to_value(f).unwrap();
            let abs = |x: f64| {
                if x.is_finite() {
                    serde_json::json!(x as u64)
                } else {
                    serde_json::Value::Null
                }
            };
            v["tick_ns"] = abs(f.abs[0]);
            v["submit_ns"] = abs(f.abs[1]);
            v["gpu_begin_ns"] = abs(f.abs[2]);
            v["gpu_end_ns"] = abs(f.abs[3]);
            v["done_ns"] = abs(f.abs[4]);
            out.push_str(&v.to_string());
            out.push('\n');
        }
        std::fs::write(&path, out).expect("write raw");
    }

    let sk = skipped_measured.len();
    let presentation = presenter.map(|(pr, placement, window_visible)| {
        // Presented handlers fire after the glass shows the frame; give the last ones time.
        std::thread::sleep(std::time::Duration::from_millis(300));
        let mut log: Vec<(u64, u64)> = pr.presented.lock().unwrap().clone();
        log.retain(|(n, _)| *n >= warm_frames);
        log.sort();
        let ticks: std::collections::HashMap<u64, u64> = recs.iter().map(|(s, _)| (s.n, s.tick)).collect();
        let shown: Vec<(u64, u64)> = log.iter().copied().filter(|(_, t)| *t > 0).collect();
        let submitted_measured = recs.iter().filter(|(s, _)| s.n >= warm_frames).count();
        let half_refresh = nominal_ns.or(refresh_ns).unwrap_or(period_ns as f64) / 2.0;
        let intervals: Vec<f64> = shown.windows(2).map(|w| w[1].1 as f64 - w[0].1 as f64).collect();
        let expected: Vec<f64> = shown
            .windows(2)
            .map(|w| (w[1].0 - w[0].0) as f64 * period_ns as f64)
            .collect();
        let irregular = intervals
            .iter()
            .zip(&expected)
            .filter(|(i, e)| (*i - *e).abs() > half_refresh)
            .count();
        let latency: Vec<f64> = shown
            .iter()
            .filter_map(|(n, t)| ticks.get(n).map(|tick| (*t as f64 - *tick as f64) / 1e6))
            .collect();
        Presentation {
            placement,
            window_visible,
            handler_calls: log.len(),
            presented: shown.len(),
            not_presented: submitted_measured.saturating_sub(shown.len()),
            irregular_intervals: irregular,
            present_latency_ms: stats::dist(&latency).rounded(3),
            present_interval_ms: stats::dist(&intervals.iter().map(|i| i / 1e6).collect::<Vec<_>>()).rounded(3),
            drawable_wait_ms: stats::dist(&drawable_wait).rounded(3),
        }
    });
    let summary = Summary {
        process: "interactive",
        label,
        adapter: g.adapter.clone(),
        width,
        height,
        fps,
        seconds,
        warmup_seconds: warmup,
        layers,
        blur_radius: radius,
        realtime_thread: rt_ok,
        clock: match clock_kind.as_str() {
            "grid" => "grid",
            "display" => "display",
            _ => "timer",
        },
        display_refresh_ms: refresh_ns.map(|r| stats::r(r / 1e6, 4)),
        nominal_refresh_ms: nominal_ns.map(|r| stats::r(r / 1e6, 4)),
        refreshes_per_frame: k,
        gpu_priority,
        frames_measured: completed.len() + sk,
        drops: Drops {
            deadline_1_frame: d1 + sk,
            deadline_2_frames: d2 + sk,
            deadline_3_frames: d3 + sk,
            skipped_ring_full: sk - skipped_pacing_measured,
            skipped_pacing: skipped_pacing_measured,
        },
        at_risk,
        latency_ms: stats::dist(&pick(|r| r.latency_ms)).rounded(3),
        gpu_exec_ms: stats::dist(&pick(|r| r.gpu_ms)).rounded(3),
        gpu_start_delay_ms: stats::dist(&pick(|r| r.gpu_start_ms)).rounded(3),
        encode_ms: stats::dist(&pick(|r| r.encode_ms)).rounded(3),
        wake_late_ms: stats::dist(&pick(|r| r.wake_late_ms)).rounded(3),
        completion_jitter_ms: stats::dist(&jitter).rounded(3),
        worst,
        miss_times_s: miss_times,
        signals_sent,
        signals_dropped,
        presentation,
    };
    println!("S8RESULT {}", serde_json::to_string(&summary).unwrap());
}
