//! The engine: a render thread that fills pool surfaces and announces each frame to
//! main after its command buffer completes.
//!
//! Playback renders ahead on a PTS grid locked to the viewer's display timing; gesture
//! modes (drag, orbit, walk) render as soon as a new input revision arrives and the
//! viewer shows them without the jitter buffer.

use crate::pool::Surface;
use crate::render::{FrameParams, Gpu, SceneKind};
use s1_mach::now_us;
use s1_mach::rendezvous::Client;
use s1_mach::wire::{FLAG_GESTURE, FLAG_SCENE_3D, FrameDescriptor, PixelFormat};
use serde::Serialize;
use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    Idle,
    Playback,
    Gesture,
    Orbit,
    Walk,
}

impl Mode {
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "idle" => Self::Idle,
            "playback" => Self::Playback,
            "gesture" => Self::Gesture,
            "orbit" => Self::Orbit,
            "walk" => Self::Walk,
            _ => return None,
        })
    }
}

#[derive(Clone, Copy, Debug)]
pub enum Slot {
    Free,
    Busy { seq: u64, since_us: i64 },
}

#[derive(Clone, Copy, Debug, Default)]
struct Input {
    revision: u64,
    ts_us: i64,
    x: f32,
    y: f32,
    walk: f32,
}

#[derive(Clone, Copy, Debug)]
struct Grid {
    origin_us: f64,
    dur_us: f64,
    next_k: i64,
}

#[derive(Default, Serialize, Clone)]
pub struct Stats {
    pub frames: u64,
    pub playback_frames: u64,
    pub gesture_frames: u64,
    pub pool_exhausted_waits: u64,
    pub late_skipped: u64,
    pub send_failures: u64,
    pub releases: u64,
    pub release_mismatches: u64,
    pub regrids: u64,
    pub max_busy_slots: u32,
    pub gpu_us_max: i64,
    pub gpu_us_sum: i64,
}

#[derive(Serialize, Clone, Copy)]
pub struct FrameLog {
    pub seq: u64,
    pub k: i64,
    pub slot: u32,
    pub pts: i64,
    pub dur: i64,
    pub rev: u64,
    pub input_ts: i64,
    pub start: i64,
    pub submitted: i64,
    pub gpu_done: i64,
    pub sent: i64,
    pub gesture: bool,
}

struct State {
    mode: Mode,
    stop: bool,
    input: Input,
    slots: Vec<Slot>,
    grid: Option<Grid>,
    lead_frames: u32,
    stats: Stats,
    log: Vec<FrameLog>,
}

pub struct Shared {
    state: Mutex<State>,
    cv: Condvar,
}

pub struct Engine {
    shared: Arc<Shared>,
    thread: Option<JoinHandle<()>>,
    pub info: EngineInfo,
}

#[derive(Serialize, Clone)]
pub struct EngineInfo {
    pub adapter: String,
    pub backend: String,
    pub width: u32,
    pub height: u32,
    pub format: &'static str,
    pub pool_size: u32,
    pub pool_bytes: u64,
    pub lead_frames: u32,
    pub extra_passes: u32,
    pub bridge_pid: i32,
    pub checkin_us: i64,
}

pub struct Config {
    pub width: u32,
    pub height: u32,
    pub format: PixelFormat,
    pub pool_size: u32,
    pub lead_frames: u32,
    pub extra_passes: u32,
    pub bridge_name: String,
}

const LOG_CAP: usize = 200_000;

impl Engine {
    pub fn start(cfg: Config) -> Result<Self, String> {
        let gpu = Gpu::new(cfg.width, cfg.height, cfg.format, cfg.extra_passes)?;
        let mut surfaces = Vec::with_capacity(cfg.pool_size as usize);
        for i in 0..cfg.pool_size {
            surfaces.push(Surface::create(&gpu.device, i, cfg.width, cfg.height, cfg.format)?);
        }
        let t0 = now_us();
        let client = Client::check_in(&cfg.bridge_name, Duration::from_secs(5)).map_err(|e| format!("check-in: {e}"))?;
        let checkin_us = now_us() - t0;
        for s in &surfaces {
            client.send_surface(s.mach_port()?, &s.info).map_err(|e| format!("send surface: {e}"))?;
        }
        let pool_bytes = surfaces.iter().map(|s| u64::from(s.info.bytes_per_row) * u64::from(s.info.height)).sum();
        let info = EngineInfo {
            adapter: gpu.adapter_info.name.clone(),
            backend: format!("{:?}", gpu.adapter_info.backend),
            width: cfg.width,
            height: cfg.height,
            format: cfg.format.electron_name(),
            pool_size: cfg.pool_size,
            pool_bytes,
            lead_frames: cfg.lead_frames,
            extra_passes: cfg.extra_passes,
            bridge_pid: client.bridge_peer().pid(),
            checkin_us,
        };
        let shared = Arc::new(Shared {
            state: Mutex::new(State {
                mode: Mode::Idle,
                stop: false,
                input: Input::default(),
                slots: vec![Slot::Free; cfg.pool_size as usize],
                grid: None,
                lead_frames: cfg.lead_frames,
                stats: Stats::default(),
                log: Vec::with_capacity(4096),
            }),
            cv: Condvar::new(),
        });
        let thread_shared = shared.clone();
        let (w, h) = (cfg.width, cfg.height);
        let thread = std::thread::Builder::new()
            .name("s1-render".into())
            .spawn(move || render_loop(thread_shared, gpu, surfaces, client, w, h))
            .map_err(|e| e.to_string())?;
        Ok(Self { shared, thread: Some(thread), info })
    }

    fn with<R>(&self, f: impl FnOnce(&mut State) -> R) -> R {
        let mut st = self.shared.state.lock().unwrap();
        let r = f(&mut st);
        self.shared.cv.notify_all();
        r
    }

    pub fn set_mode(&self, mode: Mode) {
        self.with(|st| {
            st.mode = mode;
            if mode == Mode::Playback
                && let Some(g) = st.grid.as_mut()
            {
                g.next_k = i64::MIN; // re-seed from "now" on the next iteration
            }
        });
    }

    pub fn set_lead_frames(&self, lead: u32) {
        self.with(|st| st.lead_frames = lead);
    }

    /// `anchor_us` is a predicted display time (a vsync), `period_us` the refresh
    /// interval. Content frames last a whole number of refresh intervals (60 fps
    /// content on a 120 Hz display lasts two), and PTS boundaries sit half an
    /// interval before display times so selection is insensitive to small errors.
    pub fn set_display_timing(&self, anchor_us: f64, period_us: f64) {
        self.with(|st| {
            let mult = (16_666.667 / period_us).round().max(1.0);
            let dur = period_us * mult;
            let origin = anchor_us - period_us / 2.0;
            match st.grid.as_mut() {
                None => st.grid = Some(Grid { origin_us: origin, dur_us: dur, next_k: i64::MIN }),
                Some(g) => {
                    let err = (origin - g.origin_us).rem_euclid(g.dur_us);
                    let err = if err > g.dur_us / 2.0 { err - g.dur_us } else { err };
                    let rate_changed = (dur - g.dur_us).abs() > g.dur_us * 0.002;
                    if rate_changed || err.abs() > period_us / 4.0 {
                        // Keep the next frame's index continuous across the re-grid.
                        let next_pts = g.origin_us + g.next_k.max(0) as f64 * g.dur_us;
                        let k = ((next_pts - origin) / dur).ceil() as i64;
                        *g = Grid { origin_us: origin, dur_us: dur, next_k: if g.next_k == i64::MIN { i64::MIN } else { k } };
                        st.stats.regrids += 1;
                    }
                }
            }
        });
    }

    pub fn input(&self, revision: u64, x: f32, y: f32, ts_us: i64, kind: u32) {
        self.with(|st| {
            if revision <= st.input.revision {
                return;
            }
            st.input.revision = revision;
            st.input.ts_us = ts_us;
            if kind == 1 {
                st.input.walk += 0.25;
            } else {
                st.input.x = x;
                st.input.y = y;
            }
        });
    }

    pub fn release(&self, pool_index: u32, seq: u64) {
        self.with(|st| match st.slots.get(pool_index as usize).copied() {
            Some(Slot::Busy { seq: s, .. }) if s == seq => {
                st.slots[pool_index as usize] = Slot::Free;
                st.stats.releases += 1;
            }
            _ => st.stats.release_mismatches += 1,
        });
    }

    pub fn stats(&self) -> Stats {
        self.with(|st| st.stats.clone())
    }

    pub fn busy_slots(&self) -> Vec<(u32, u64, i64)> {
        self.with(|st| {
            st.slots
                .iter()
                .enumerate()
                .filter_map(|(i, s)| match *s {
                    Slot::Busy { seq, since_us } => Some((i as u32, seq, since_us)),
                    Slot::Free => None,
                })
                .collect()
        })
    }

    pub fn take_log(&self) -> Vec<FrameLog> {
        self.with(|st| std::mem::take(&mut st.log))
    }

    pub fn stop(&mut self) {
        self.with(|st| st.stop = true);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        self.stop();
    }
}

fn set_realtime_qos() {
    unsafe extern "C" {
        fn pthread_set_qos_class_self_np(qos: u32, rel: i32) -> i32;
    }
    // QOS_CLASS_USER_INTERACTIVE
    // SAFETY: no preconditions; affects only the calling thread.
    unsafe { pthread_set_qos_class_self_np(0x21, 0) };
}

enum Job {
    Frame { slot: usize, params: FrameParams, k: i64, pts: i64, dur: i64, input_ts: i64, gesture: bool, three_d: bool },
    Wait(Duration),
    Stop,
}

fn render_loop(shared: Arc<Shared>, gpu: Gpu, surfaces: Vec<Surface>, client: Client, w: u32, h: u32) {
    set_realtime_qos();
    let start = now_us();
    let mut seq: u64 = 0;
    let mut last_gesture_rev: u64 = 0;
    loop {
        let job = {
            let mut st = shared.state.lock().unwrap();
            next_job(&mut st, &mut seq, &mut last_gesture_rev, start, w, h)
        };
        match job {
            Job::Stop => break,
            Job::Wait(d) => {
                let st = shared.state.lock().unwrap();
                let _ = shared.cv.wait_timeout(st, d).unwrap();
            }
            Job::Frame { slot, params, k, pts, dur, input_ts, gesture, three_d } => {
                let t_start = now_us();
                let idx = gpu.render(&surfaces[slot], &params);
                let t_sub = now_us();
                let ok = gpu.wait(idx);
                let t_gpu = now_us();
                let mut flags = 0;
                if gesture {
                    flags |= FLAG_GESTURE;
                }
                if three_d {
                    flags |= FLAG_SCENE_3D;
                }
                let desc = FrameDescriptor {
                    pool_index: slot as u32,
                    generation: 1,
                    seq: params.seq,
                    pts_us: pts,
                    duration_us: dur,
                    revision: params.revision,
                    input_ts_us: input_ts,
                    render_start_us: t_start,
                    gpu_done_us: t_gpu,
                    color_space: if surfaces[slot].info.format == PixelFormat::Rgba16F { 1 } else { 0 },
                    flags,
                    overlay: params.marker,
                };
                let sent = ok.is_ok() && client.send_frame(&desc, Duration::from_millis(50)).is_ok();
                let t_sent = now_us();
                let mut st = shared.state.lock().unwrap();
                st.stats.frames += 1;
                if gesture {
                    st.stats.gesture_frames += 1;
                } else {
                    st.stats.playback_frames += 1;
                }
                let gpu_us = t_gpu - t_start;
                st.stats.gpu_us_max = st.stats.gpu_us_max.max(gpu_us);
                st.stats.gpu_us_sum += gpu_us;
                if !sent {
                    st.stats.send_failures += 1;
                    st.slots[slot] = Slot::Free;
                }
                if st.log.len() < LOG_CAP {
                    st.log.push(FrameLog {
                        seq: params.seq,
                        k,
                        slot: slot as u32,
                        pts,
                        dur,
                        rev: params.revision,
                        input_ts,
                        start: t_start,
                        submitted: t_sub,
                        gpu_done: t_gpu,
                        sent: t_sent,
                        gesture,
                    });
                }
            }
        }
    }
    let _ = client.goodbye();
}

fn take_slot(st: &mut State, seq: u64) -> Option<usize> {
    let i = st.slots.iter().position(|s| matches!(s, Slot::Free))?;
    st.slots[i] = Slot::Busy { seq, since_us: now_us() };
    let busy = st.slots.iter().filter(|s| matches!(s, Slot::Busy { .. })).count() as u32;
    st.stats.max_busy_slots = st.stats.max_busy_slots.max(busy);
    Some(i)
}

fn marker_rect(x: f32, y: f32) -> [f32; 4] {
    [x - 48.0, y - 48.0, 96.0, 96.0]
}

fn next_job(st: &mut State, seq: &mut u64, last_rev: &mut u64, start: i64, w: u32, h: u32) -> Job {
    if st.stop {
        return Job::Stop;
    }
    let now = now_us();
    let input = st.input;
    match st.mode {
        Mode::Idle => Job::Wait(Duration::from_millis(20)),
        Mode::Playback => {
            let lead = st.lead_frames;
            let Some(g) = st.grid.as_mut() else { return Job::Wait(Duration::from_millis(5)) };
            let lead_us = f64::from(lead) * g.dur_us;
            if g.next_k == i64::MIN {
                g.next_k = ((now as f64 + lead_us - g.origin_us) / g.dur_us).ceil() as i64;
            }
            let pts = g.origin_us + g.next_k as f64 * g.dur_us;
            let render_at = pts - lead_us;
            if (now as f64) < render_at {
                let wait = ((render_at - now as f64) as u64).clamp(100, 2000);
                return Job::Wait(Duration::from_micros(wait));
            }
            if pts + g.dur_us < now as f64 {
                // Already past its display interval: skipping is an engine-side drop.
                g.next_k += 1;
                st.stats.late_skipped += 1;
                return Job::Wait(Duration::ZERO);
            }
            let (k, dur) = (g.next_k, g.dur_us);
            let Some(slot) = take_slot(st, *seq) else {
                st.stats.pool_exhausted_waits += 1;
                return Job::Wait(Duration::from_micros(500));
            };
            st.grid.as_mut().unwrap().next_k += 1;
            let s = *seq;
            *seq += 1;
            Job::Frame {
                slot,
                params: FrameParams {
                    time_s: ((pts as i64 - start) as f64 / 1e6) as f32,
                    seq: s,
                    revision: input.revision,
                    marker: marker_rect(input.x, input.y),
                    scene: SceneKind::Pattern,
                },
                k,
                pts: pts.round() as i64,
                dur: dur.round() as i64,
                input_ts: input.ts_us,
                gesture: false,
                three_d: false,
            }
        }
        Mode::Gesture | Mode::Orbit | Mode::Walk => {
            if input.revision == *last_rev {
                return Job::Wait(Duration::from_millis(4));
            }
            let Some(slot) = take_slot(st, *seq) else {
                st.stats.pool_exhausted_waits += 1;
                return Job::Wait(Duration::from_micros(500));
            };
            *last_rev = input.revision;
            let s = *seq;
            *seq += 1;
            let scene = if st.mode == Mode::Gesture {
                SceneKind::Pattern
            } else {
                SceneKind::Cubes {
                    yaw: input.x / w as f32 * std::f32::consts::TAU,
                    pitch: 0.15 + (input.y / h as f32 - 0.5) * 1.2,
                    distance: 26.0,
                    walk: input.walk,
                }
            };
            Job::Frame {
                slot,
                params: FrameParams {
                    time_s: ((now - start) as f64 / 1e6) as f32,
                    seq: s,
                    revision: input.revision,
                    marker: marker_rect(input.x, input.y),
                    scene,
                },
                k: -1,
                pts: now,
                dur: 0,
                input_ts: input.ts_us,
                gesture: true,
                three_d: st.mode != Mode::Gesture,
            }
        }
    }
}
