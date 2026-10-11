//! Process B: the job engine stand-in (export-like GPU workload).
//!
//! Each "export frame" is: a procedural 4K source pass, then `iters` separable Gaussian
//! blur iterations (horizontal then vertical, radius `radius`) between two 4K
//! `rgba16float` targets. All compute.
//!
//! Modes:
//! - `full`: one command buffer per export frame, `in-flight` frames queued (how a
//!   naive exporter submits). Ignores deadline signals.
//! - `heavy`: one huge dispatch per command buffer (a single long kernel that cannot be
//!   split, standing in for a foreign renderer such as Blender). Ignores signals.
//! - `sliced`: GPU time slicing (contract 13). Work is cut into horizontal bands and
//!   packed into command buffers whose measured GPU time stays under `slice-ms`, at most
//!   `in-flight` command buffers queued, optional lower QoS / Darwin background, and a
//!   back-off of `yield-ms` whenever A signals at-risk or miss (`yield-on`).
//! - `none`: no GPU work (VideoToolbox sessions only).
//!
//! Optional VideoToolbox sessions (`vt-sessions`, `vt-codec`) run concurrently in this
//! process, standing in for the export encoder.
//!
//! Optional Blender EEVEE worker (`--blender PATH`): this process also stands in for
//! main's Blender supervision and applies `--blender-policy` from the same deadline
//! signals (see `s8_contention::blender`).

use s8_contention::{Args, blender, clock, gpu, signal, stats, sys, vt};
use serde::Serialize;
use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

const W: u32 = 3840;
const H: u32 = 2160;
const TS_CAPACITY: usize = 16384;
const UNIFORM_STRIDE: u64 = 256;
const UNIFORM_SLOTS: u64 = 8192;

const CS_BLUR: &str = r#"
struct Band { y0: u32, rows: u32, radius: i32, dir: u32 };
@group(0) @binding(0) var<uniform> band: Band;
@group(0) @binding(1) var src: texture_2d<f32>;
@group(0) @binding(2) var dst: texture_storage_2d<rgba16float, write>;

@compute @workgroup_size(16, 8)
fn blur(@builtin(global_invocation_id) id: vec3<u32>) {
    let dims = vec2<i32>(textureDimensions(src));
    if (id.y >= band.rows) { return; }
    let p = vec2<i32>(i32(id.x), i32(band.y0 + id.y));
    if (p.x >= dims.x || p.y >= dims.y) { return; }
    let d = select(vec2<i32>(1, 0), vec2<i32>(0, 1), band.dir == 1u);
    let sigma = max(f32(band.radius) / 2.5, 0.5);
    var acc = vec4<f32>(0.0);
    var wsum = 0.0;
    for (var k = -band.radius; k <= band.radius; k = k + 1) {
        let q = clamp(p + d * k, vec2<i32>(0), dims - vec2<i32>(1));
        let w = exp(-0.5 * f32(k * k) / (sigma * sigma));
        acc = acc + textureLoad(src, q, 0) * w;
        wsum = wsum + w;
    }
    textureStore(dst, p, acc / wsum);
}
"#;

const CS_GEN: &str = r#"
struct Band { y0: u32, rows: u32, radius: i32, dir: u32 };
@group(0) @binding(0) var<uniform> band: Band;
@group(0) @binding(2) var dst: texture_storage_2d<rgba16float, write>;

@compute @workgroup_size(16, 8)
fn gen(@builtin(global_invocation_id) id: vec3<u32>) {
    let dims = vec2<u32>(textureDimensions(dst));
    if (id.y >= band.rows) { return; }
    let p = vec2<u32>(id.x, band.y0 + id.y);
    if (p.x >= dims.x || p.y >= dims.y) { return; }
    let t = f32(band.dir);   // frame number
    let uv = vec2<f32>(p) / vec2<f32>(dims);
    let r = length(uv - vec2<f32>(0.5 + 0.2 * sin(t * 0.1), 0.5));
    let v = 0.5 + 0.5 * sin(r * 200.0 - t);
    let c = f32((p.x / 16u + p.y / 16u + u32(t)) & 1u);
    textureStore(dst, vec2<i32>(p), vec4<f32>(v, c, uv.x * v, 1.0));
}
"#;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct BandU {
    y0: u32,
    rows: u32,
    radius: i32,
    dir: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Gen,
    BlurH,
    BlurV,
    Heavy,
}

impl Kind {
    fn idx(self) -> usize {
        self as usize
    }
}

#[derive(Clone, Copy, Debug)]
struct Unit {
    kind: Kind,
    y0: u32,
    rows: u32,
    /// Last unit of an export frame.
    frame_end: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Full,
    Heavy,
    Sliced,
    None,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum YieldOn {
    None,
    Miss,
    Risk,
}

struct InFlight {
    index: Option<wgpu::SubmissionIndex>,
    entry: usize,
    submit_ns: u64,
    frames_done: u32,
}

/// The timestamp ring may wrap only once every entry older than one ring length has
/// been resolved; entries are flushed every 64 retired command buffers, so the lag is
/// bounded by `in-flight + 64`, far below the capacity.
fn ts_lag_ok(entry: usize, inflight: &VecDeque<InFlight>) -> bool {
    inflight.front().is_none_or(|f| entry - f.entry < TS_CAPACITY / 2)
}

#[derive(Serialize, Default)]
struct Signals {
    playback_start: u64,
    playback_stop: u64,
    frame_ok: u64,
    late_done: u64,
    at_risk: u64,
    miss: u64,
    /// Receive time minus send time (includes B's polling granularity).
    reaction_ms: stats::Dist,
}

/// Signal-driven state of the job's scheduler.
#[derive(Default)]
struct Ctl {
    sig: Signals,
    reaction: Vec<f64>,
    playback: bool,
    /// Host time playback last started.
    playback_since: u64,
    yield_until: u64,
    yields: u64,
    /// Tick of the most recent frame A reported done, and the frame period inferred from
    /// successive ticks.
    last_done_tick: u64,
    period: u64,
    /// Emulated relay latency (main's hop between the engines): a message takes effect
    /// only `relay_delay_ns` after it was sent.
    relay_delay_ns: u64,
    held: VecDeque<signal::Message>,
}

impl Ctl {
    /// Drain signals that are due. `react`: whether misses open a yield window (sliced
    /// GPU work, or a Blender `yield` policy).
    fn drain(&mut self, rx: &mut Option<signal::Receiver>, yield_on: YieldOn, react: bool, yield_ns: u64) {
        let Some(r) = rx.as_mut() else { return };
        let now = clock::now_ns();
        self.held.extend(r.drain());
        let mut due = Vec::new();
        while let Some(m) = self.held.front() {
            if m.sent_ns + self.relay_delay_ns > now {
                break;
            }
            due.push(self.held.pop_front().unwrap());
        }
        for m in due {
            self.reaction.push((now.saturating_sub(m.sent_ns)) as f64 / 1e6);
            let trigger = match m.kind {
                signal::Kind::PlaybackStart => {
                    self.sig.playback_start += 1;
                    self.playback = true;
                    self.playback_since = m.sent_ns;
                    false
                }
                signal::Kind::PlaybackStop => {
                    self.sig.playback_stop += 1;
                    self.playback = false;
                    self.last_done_tick = 0;
                    false
                }
                signal::Kind::FrameOk | signal::Kind::LateDone => {
                    if m.kind == signal::Kind::FrameOk {
                        self.sig.frame_ok += 1;
                    } else {
                        self.sig.late_done += 1;
                    }
                    if m.tick_ns > self.last_done_tick {
                        if self.last_done_tick > 0 {
                            let d = m.tick_ns - self.last_done_tick;
                            if self.period == 0 || d < self.period {
                                self.period = d;
                            }
                        }
                        self.last_done_tick = m.tick_ns;
                    }
                    false
                }
                signal::Kind::AtRisk => {
                    self.sig.at_risk += 1;
                    yield_on == YieldOn::Risk
                }
                signal::Kind::Miss => {
                    self.sig.miss += 1;
                    matches!(yield_on, YieldOn::Miss | YieldOn::Risk)
                }
            };
            if trigger && react {
                if self.yield_until <= now {
                    self.yields += 1;
                }
                self.yield_until = self.yield_until.max(now + yield_ns);
            }
        }
    }

    /// Frame alignment: true if A's frame for the current period is done and a slice
    /// of `est_ns` started now ends `guard_ns` before A's next tick.
    fn align_ok(&self, now: u64, est_ns: u64, guard_ns: u64) -> bool {
        if !self.playback || self.last_done_tick == 0 || self.period == 0 {
            return true;
        }
        if now < self.last_done_tick {
            return false;
        }
        let k = (now - self.last_done_tick) / self.period;
        if k >= 1 {
            return false;
        }
        now + est_ns + guard_ns <= self.last_done_tick + self.period
    }
}

#[derive(Serialize)]
struct Summary {
    process: &'static str,
    label: String,
    adapter: String,
    mode: &'static str,
    qos: &'static str,
    darwin_bg: bool,
    gpu_priority: Option<usize>,
    slice_ms: f64,
    in_flight: usize,
    yield_on: &'static str,
    yield_ms: f64,
    pause_on_playback: bool,
    relay_delay_ms: f64,
    radius: i32,
    iters: u32,
    heavy_radius: i32,
    seconds: f64,
    export_frames: u64,
    export_fps: f64,
    command_buffers: u64,
    cb_gpu_ms: stats::Dist,
    cb_wall_ms: stats::Dist,
    /// Busy GPU time (sum of command-buffer GPU spans) over wall time.
    gpu_busy_fraction: f64,
    yields: u64,
    yield_total_ms: f64,
    paused_for_playback_ms: f64,
    align: bool,
    align_guard_ms: f64,
    align_wait_total_ms: f64,
    calibration_ms_per_full_pass: Vec<(String, f64)>,
    signals: Signals,
    vt: Vec<vt::SessionStats>,
    blender: Option<blender::Stats>,
}

fn main() {
    sys::install_stop_handler();
    let args = Args::from_env();
    let mode = match args.string_or("mode", "sliced").as_str() {
        "full" => Mode::Full,
        "heavy" => Mode::Heavy,
        "sliced" => Mode::Sliced,
        "none" => Mode::None,
        m => panic!("unknown mode {m}"),
    };
    let label = args.string_or("label", "b");
    let seconds_limit: f64 = args.num("seconds", 3600.0);
    let slice_ms: f64 = args.num("slice-ms", 2.0);
    let max_in_flight: usize = args.num("in-flight", if mode == Mode::Sliced { 1 } else { 2 });
    let qos = args.str("qos").map(|q| sys::Qos::parse(q).expect("qos"));
    let darwin_bg = args.flag("darwin-bg");
    let yield_on = match args
        .string_or("yield-on", if mode == Mode::Sliced { "miss" } else { "none" })
        .as_str()
    {
        "none" => YieldOn::None,
        "miss" => YieldOn::Miss,
        "risk" => YieldOn::Risk,
        y => panic!("unknown yield-on {y}"),
    };
    let yield_ms: f64 = args.num("yield-ms", 100.0);
    let pause_on_playback = args.flag("pause-on-playback");
    let relay_delay_ms: f64 = args.num("relay-delay-ms", 0.0);
    let align = args.flag("align");
    let align_guard_ms: f64 = args.num("align-guard-ms", 1.0);
    let radius: i32 = args.num("radius", 48);
    let iters: u32 = args.num("iters", 6);
    let heavy_radius: i32 = args.num("heavy-radius", 384);
    let fifo = args.str("signal-fifo").map(str::to_string);
    let vt_sessions: usize = args.num("vt-sessions", 0);
    let vt_codec = vt::Codec::parse(&args.string_or("vt-codec", "hevc10")).expect("vt-codec");
    let vt_fps: f64 = args.num("vt-fps", 0.0);
    let blender_cfg = args.str("blender").map(|exe| blender::Config {
        executable: exe.to_string(),
        script: args.string_or(
            "blender-script",
            concat!(env!("CARGO_MANIFEST_DIR"), "/blender/eevee_load.py"),
        ),
        out_dir: args.string_or(
            "blender-out",
            std::env::temp_dir()
                .join(format!("s8-blender-{}", std::process::id()))
                .to_str()
                .unwrap(),
        ),
        width: args.num("blender-width", W),
        height: args.num("blender-height", H),
        samples: args.num("blender-samples", 16),
        policy: blender::Policy::parse(&args.string_or("blender-policy", "run")).expect("blender-policy"),
        run_ms: args.num("blender-run-ms", 10.0),
        stop_ms: args.num("blender-stop-ms", 40.0),
    });
    let react = mode == Mode::Sliced || blender_cfg.as_ref().is_some_and(|c| c.policy == blender::Policy::Yield);

    if darwin_bg {
        sys::set_process_darwin_bg().expect("darwin bg");
    }
    if let Some(q) = qos {
        sys::set_thread_qos(q).expect("qos");
    }
    let mut rx = fifo.as_deref().map(|f| signal::Receiver::open(f).expect("open fifo"));

    let vt_stop = Arc::new(AtomicBool::new(false));
    let vt_handle = (vt_sessions > 0).then(|| {
        vt::spawn(
            vt::Config {
                codec: vt_codec,
                width: W as i32,
                height: H as i32,
                sessions: vt_sessions,
                max_pending: 16,
                fps_cap: (vt_fps > 0.0).then_some(vt_fps),
                qos,
            },
            vt_stop.clone(),
        )
    });

    let g = gpu::create("s8-job");
    let gpu_priority = match args.str("gpu-priority") {
        Some(p) => Some(gpu::set_private_gpu_priority(&g.queue, p.parse().unwrap()).expect("gpu priority")),
        None => gpu::get_private_gpu_priority(&g.queue),
    };
    let device = &g.device;
    let queue = &g.queue;
    let tex = |label| {
        device.create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d {
                width: W,
                height: H,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba16Float,
            usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        })
    };
    let ta = tex("a");
    let tb = tex("b");
    let va = ta.create_view(&Default::default());
    let vb = tb.create_view(&Default::default());
    let ubuf = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("bands"),
        size: UNIFORM_STRIDE * UNIFORM_SLOTS,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let entry_uniform = wgpu::BindGroupLayoutEntry {
        binding: 0,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: true,
            min_binding_size: wgpu::BufferSize::new(16),
        },
        count: None,
    };
    let entry_src = wgpu::BindGroupLayoutEntry {
        binding: 1,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: false },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    };
    let entry_dst = wgpu::BindGroupLayoutEntry {
        binding: 2,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::StorageTexture {
            access: wgpu::StorageTextureAccess::WriteOnly,
            format: wgpu::TextureFormat::Rgba16Float,
            view_dimension: wgpu::TextureViewDimension::D2,
        },
        count: None,
    };
    let gen_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("gen"),
        entries: &[entry_uniform, entry_dst],
    });
    let blur_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("blur"),
        entries: &[entry_uniform, entry_src, entry_dst],
    });
    let pipe = |src: &str, entry: &str, bgl: &wgpu::BindGroupLayout| {
        let m = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some(entry),
            source: wgpu::ShaderSource::Wgsl(src.into()),
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some(entry),
            bind_group_layouts: &[Some(bgl)],
            immediate_size: 0,
        });
        device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some(entry),
            layout: Some(&layout),
            module: &m,
            entry_point: Some(entry),
            compilation_options: Default::default(),
            cache: None,
        })
    };
    let blur_p = pipe(CS_BLUR, "blur", &blur_bgl);
    let gen_p = pipe(CS_GEN, "gen", &gen_bgl);
    let band_binding = wgpu::BindingResource::Buffer(wgpu::BufferBinding {
        buffer: &ubuf,
        offset: 0,
        size: wgpu::BufferSize::new(16),
    });
    let gen_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: None,
        layout: &gen_bgl,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: band_binding.clone(),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: wgpu::BindingResource::TextureView(&va),
            },
        ],
    });
    let blur_bg = |src: &wgpu::TextureView, dst: &wgpu::TextureView| {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &blur_bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: band_binding.clone(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(src),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(dst),
                },
            ],
        })
    };
    let h_bg = blur_bg(&va, &vb);
    let v_bg = blur_bg(&vb, &va);
    let mut ts = gpu::TsLog::new(&g, TS_CAPACITY).expect("timestamp queries required");

    let mut uslot: u64 = 0;
    let mut frame_no: u32 = 0;
    // Encode a list of units into one command buffer (one compute pass) whose begin/end
    // GPU timestamps go to log entry `entry`.
    let encode =
        |units: &[Unit], entry: usize, frame_no: &mut u32, uslot: &mut u64, ts: &gpu::TsLog| -> wgpu::CommandBuffer {
            let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("job") });
            let mut offsets = Vec::with_capacity(units.len());
            for u in units {
                let off = (*uslot % UNIFORM_SLOTS) * UNIFORM_STRIDE;
                *uslot += 1;
                let (r, dir) = match u.kind {
                    Kind::Gen => (0, *frame_no),
                    Kind::BlurH => (radius, 0),
                    Kind::BlurV => (radius, 1),
                    Kind::Heavy => (heavy_radius, 0),
                };
                queue.write_buffer(
                    &ubuf,
                    off,
                    bytemuck::bytes_of(&BandU {
                        y0: u.y0,
                        rows: u.rows,
                        radius: r,
                        dir,
                    }),
                );
                offsets.push(off as u32);
                if u.frame_end {
                    *frame_no += 1;
                }
            }
            {
                let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor {
                    label: Some("job"),
                    timestamp_writes: Some(ts.compute_writes(entry)),
                });
                for (u, off) in units.iter().zip(&offsets) {
                    match u.kind {
                        Kind::Gen => {
                            pass.set_pipeline(&gen_p);
                            pass.set_bind_group(0, &gen_bg, &[*off]);
                        }
                        Kind::BlurH | Kind::Heavy => {
                            pass.set_pipeline(&blur_p);
                            pass.set_bind_group(0, &h_bg, &[*off]);
                        }
                        Kind::BlurV => {
                            pass.set_pipeline(&blur_p);
                            pass.set_bind_group(0, &v_bg, &[*off]);
                        }
                    }
                    pass.dispatch_workgroups(W.div_ceil(16), u.rows.div_ceil(8), 1);
                }
            }
            enc.finish()
        };

    let full = |kind: Kind, frame_end: bool| Unit {
        kind,
        y0: 0,
        rows: H,
        frame_end,
    };
    let frame_units = || -> Vec<Unit> {
        let mut v = vec![full(Kind::Gen, false)];
        for i in 0..iters {
            v.push(full(Kind::BlurH, false));
            v.push(full(Kind::BlurV, i + 1 == iters));
        }
        v
    };

    // Calibrate: full-frame passes per kind, measured alone (ns per row). The GPU is
    // first kept busy for a moment so clocks ramp the way they will during the run.
    let mut entry: usize = 0;
    let mut cost_per_row = [0f64; 4];
    let mut calibration = Vec::new();
    if mode != Mode::None {
        let warm_until = clock::now_ns() + 1_000_000_000;
        while clock::now_ns() < warm_until {
            let cb = encode(&frame_units(), entry, &mut frame_no, &mut uslot, &ts);
            entry += 1;
            let idx = queue.submit([cb]);
            device
                .poll(wgpu::PollType::Wait {
                    submission_index: Some(idx),
                    timeout: None,
                })
                .unwrap();
        }
        ts.finish(device, queue, entry);
        for kind in [Kind::Gen, Kind::BlurH, Kind::BlurV, Kind::Heavy] {
            let first = entry;
            for _ in 0..4 {
                let cb = encode(&[full(kind, false)], entry, &mut frame_no, &mut uslot, &ts);
                entry += 1;
                let idx = queue.submit([cb]);
                device
                    .poll(wgpu::PollType::Wait {
                        submission_index: Some(idx),
                        timeout: None,
                    })
                    .unwrap();
            }
            let samples: Vec<f64> = ts
                .finish(device, queue, entry)
                .into_iter()
                .filter(|(e, _)| *e > first)
                .filter_map(|(_, v)| v.map(|(b, e)| e - b))
                .collect();
            let ns = stats::dist(&samples).p50;
            cost_per_row[kind.idx()] = ns / H as f64;
            calibration.push((format!("{kind:?}"), stats::r(ns / 1e6, 3)));
        }
    }
    let first_run_entry = entry;

    // Blender starts after calibration so it does not perturb the cost model; READY is
    // sent once it has finished two frames (shader compilation done).
    let out_dir = blender_cfg.as_ref().map(|c| c.out_dir.clone());
    let mut blender = blender_cfg.map(|c| blender::Blender::launch(c, 2, 120.0).expect("launch Blender"));

    eprintln!("S8READY");
    let start = clock::now_ns();
    let deadline_end = start + (seconds_limit * 1e9) as u64;
    let budget_ns = slice_ms * 1e6;
    let mut queue_units: VecDeque<Unit> = VecDeque::new();
    let mut inflight: VecDeque<InFlight> = VecDeque::new();
    let mut meta: std::collections::HashMap<usize, (f64, [u32; 4])> = Default::default();
    let mut export_frames: u64 = 0;
    let mut cb_wall = Vec::new();
    let mut yield_total = 0u64;
    let mut paused_total = 0u64;
    let mut align_wait_total = 0u64;
    let mut ctl = Ctl {
        relay_delay_ns: (relay_delay_ms * 1e6) as u64,
        ..Default::default()
    };
    let mut retired_since_flush = 0usize;
    let mut planned: Option<(wgpu::CommandBuffer, InFlight)> = None;
    let yield_ns = (yield_ms * 1e6) as u64;
    let guard_ns = (align_guard_ms * 1e6) as u64;

    if mode != Mode::None {
        loop {
            if sys::stop_requested() || clock::now_ns() >= deadline_end {
                break;
            }
            ctl.drain(&mut rx, yield_on, react, yield_ns);
            supervise(&mut blender, &ctl);
            if pause_on_playback && ctl.playback {
                let t = clock::now_ns();
                clock::sleep_ns(1_000_000);
                paused_total += clock::now_ns() - t;
                continue;
            }
            let now = clock::now_ns();
            if mode == Mode::Sliced && now < ctl.yield_until {
                let step = (ctl.yield_until - now).min(1_000_000);
                clock::sleep_ns(step);
                yield_total += step;
                continue;
            }
            // Plan and encode the next command buffer while earlier ones run.
            if planned.is_none() {
                let mut units = Vec::new();
                let mut est = 0f64;
                match mode {
                    Mode::Full => units = frame_units(),
                    Mode::Heavy => units.push(full(Kind::Heavy, true)),
                    Mode::Sliced => loop {
                        if queue_units.is_empty() {
                            queue_units.extend(frame_units());
                        }
                        let u = *queue_units.front().unwrap();
                        let per_row = cost_per_row[u.kind.idx()].max(1.0);
                        let remaining = budget_ns * 0.9 - est;
                        let fit_rows = ((remaining.max(0.0) / per_row) as u32) / 8 * 8;
                        if fit_rows >= u.rows {
                            units.push(u);
                            est += per_row * u.rows as f64;
                            queue_units.pop_front();
                            if est >= budget_ns * 0.85 {
                                break;
                            }
                        } else if fit_rows >= 8 || units.is_empty() {
                            let rows = fit_rows.max(8);
                            units.push(Unit {
                                kind: u.kind,
                                y0: u.y0,
                                rows,
                                frame_end: false,
                            });
                            est += per_row * rows as f64;
                            let f = queue_units.front_mut().unwrap();
                            f.y0 += rows;
                            f.rows -= rows;
                            break;
                        } else {
                            break;
                        }
                    },
                    Mode::None => unreachable!(),
                }
                let mut kinds = [0u32; 4];
                let mut frames_done = 0;
                for u in &units {
                    kinds[u.kind.idx()] += 1;
                    if u.frame_end {
                        frames_done += 1;
                    }
                }
                assert!(
                    entry - first_run_entry < TS_CAPACITY - 4096 || ts_lag_ok(entry, &inflight),
                    "timestamp ring overrun"
                );
                let cb = encode(&units, entry, &mut frame_no, &mut uslot, &ts);
                meta.insert(entry, (est, kinds));
                planned = Some((
                    cb,
                    InFlight {
                        index: None,
                        entry,
                        submit_ns: 0,
                        frames_done,
                    },
                ));
                entry += 1;
            }
            while inflight.len() >= max_in_flight {
                let f = inflight.pop_front().unwrap();
                device
                    .poll(wgpu::PollType::Wait {
                        submission_index: f.index.clone(),
                        timeout: None,
                    })
                    .unwrap();
                cb_wall.push((clock::now_ns() - f.submit_ns) as f64 / 1e6);
                export_frames += f.frames_done as u64;
                retired_since_flush += 1;
            }
            // A signal may have arrived while waiting: honour it before submitting.
            ctl.drain(&mut rx, yield_on, react, yield_ns);
            supervise(&mut blender, &ctl);
            if (mode == Mode::Sliced && clock::now_ns() < ctl.yield_until) || (pause_on_playback && ctl.playback) {
                continue;
            }
            // Frame-aligned slicing: submit only after A's current frame is done and only
            // if the slice is predicted to finish before A's next vsync tick (minus a guard).
            if align && mode == Mode::Sliced {
                let est = meta.get(&planned.as_ref().unwrap().1.entry).map(|m| m.0).unwrap_or(0.0) as u64;
                let t = clock::now_ns();
                let mut yielded = false;
                while !ctl.align_ok(clock::now_ns(), est, guard_ns) {
                    clock::sleep_ns(100_000);
                    ctl.drain(&mut rx, yield_on, true, yield_ns);
                    supervise(&mut blender, &ctl);
                    if clock::now_ns() < ctl.yield_until || sys::stop_requested() {
                        yielded = true;
                        break;
                    }
                }
                align_wait_total += clock::now_ns() - t;
                if yielded {
                    continue;
                }
            }
            let (cb, mut f) = planned.take().unwrap();
            f.submit_ns = clock::now_ns();
            f.index = Some(queue.submit([cb]));
            inflight.push_back(f);
            // Bookkeeping while the GPU works: batch-resolve timestamps of retired
            // command buffers and feed measured GPU time back into the cost model.
            if retired_since_flush >= 64 {
                let upto = inflight.front().map(|f| f.entry).unwrap_or(entry);
                ts.flush(device, queue, upto);
                retired_since_flush = 0;
            }
            if mode == Mode::Sliced {
                for (e, v) in ts.collect(device) {
                    if let (Some((b, en)), Some((est, kinds))) = (v, meta.remove(&e))
                        && est > 0.0
                    {
                        let ratio = ((en - b) / est).clamp(0.25, 4.0);
                        for (k, n) in kinds.iter().enumerate() {
                            if *n > 0 {
                                cost_per_row[k] *= 0.95 + 0.05 * ratio;
                            }
                        }
                    }
                }
            }
        }
        while let Some(f) = inflight.pop_front() {
            device
                .poll(wgpu::PollType::Wait {
                    submission_index: f.index.clone(),
                    timeout: None,
                })
                .unwrap();
            cb_wall.push((clock::now_ns() - f.submit_ns) as f64 / 1e6);
            export_frames += f.frames_done as u64;
        }
    } else {
        while !sys::stop_requested() && clock::now_ns() < deadline_end {
            ctl.drain(&mut rx, yield_on, react, yield_ns);
            supervise(&mut blender, &ctl);
            clock::sleep_ns(1_000_000);
        }
    }
    let end_ns = clock::now_ns();
    let submitted_upto = if planned.is_some() { entry - 1 } else { entry };
    let mut spans: Vec<(f64, f64)> = ts
        .finish(device, queue, submitted_upto)
        .into_iter()
        .filter(|(e, _)| *e >= first_run_entry && *e < submitted_upto)
        .filter_map(|(_, v)| v)
        .collect();
    spans.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
    let cb_gpu: Vec<f64> = spans.iter().map(|(b, e)| (e - b) / 1e6).collect();
    // Busy time = union of command-buffer GPU spans (they can overlap with 2 in flight).
    let mut busy_ns = 0f64;
    let mut cur: Option<(f64, f64)> = None;
    for &(b, e) in &spans {
        match cur {
            Some((cb, ce)) if b <= ce => cur = Some((cb, ce.max(e))),
            Some((cb, ce)) => {
                busy_ns += ce - cb;
                cur = Some((b, e));
            }
            None => cur = Some((b, e)),
        }
    }
    if let Some((cb, ce)) = cur {
        busy_ns += ce - cb;
    }
    let _ = end_ns;
    let secs = (clock::now_ns() - start) as f64 / 1e9;
    vt_stop.store(true, std::sync::atomic::Ordering::Relaxed);
    let vt_stats = vt_handle.map(|h| h.join().unwrap()).unwrap_or_default();
    let blender_stats = blender.take().map(|b| b.finish());
    if let Some(d) = out_dir {
        let _ = std::fs::remove_dir_all(d);
    }
    ctl.sig.reaction_ms = stats::dist(&ctl.reaction).rounded(3);

    let summary = Summary {
        process: "job",
        label,
        adapter: g.adapter.clone(),
        mode: match mode {
            Mode::Full => "full",
            Mode::Heavy => "heavy",
            Mode::Sliced => "sliced",
            Mode::None => "none",
        },
        qos: qos.map(|q| q.name()).unwrap_or("inherited"),
        darwin_bg,
        gpu_priority,
        slice_ms,
        in_flight: max_in_flight,
        yield_on: match yield_on {
            YieldOn::None => "none",
            YieldOn::Miss => "miss",
            YieldOn::Risk => "risk",
        },
        yield_ms,
        pause_on_playback,
        relay_delay_ms,
        radius,
        iters,
        heavy_radius,
        seconds: stats::r(secs, 3),
        export_frames,
        export_fps: stats::r(export_frames as f64 / secs, 3),
        command_buffers: cb_gpu.len() as u64,
        cb_gpu_ms: stats::dist(&cb_gpu).rounded(3),
        cb_wall_ms: stats::dist(&cb_wall).rounded(3),
        gpu_busy_fraction: stats::r(busy_ns / (secs * 1e9), 4),
        yields: ctl.yields,
        yield_total_ms: stats::r(yield_total as f64 / 1e6, 1),
        paused_for_playback_ms: stats::r(paused_total as f64 / 1e6, 1),
        calibration_ms_per_full_pass: calibration,
        signals: ctl.sig,
        align,
        align_guard_ms,
        align_wait_total_ms: stats::r(align_wait_total as f64 / 1e6, 1),
        vt: vt_stats,
        blender: blender_stats,
    };
    println!("S8RESULT {}", serde_json::to_string(&summary).unwrap());
}

/// Apply the Blender policy for the current signal state.
fn supervise(blender: &mut Option<blender::Blender>, ctl: &Ctl) {
    if let Some(b) = blender.as_mut() {
        let now = clock::now_ns();
        b.apply(
            ctl.playback,
            now < ctl.yield_until,
            now.saturating_sub(ctl.playback_since),
        );
    }
}
