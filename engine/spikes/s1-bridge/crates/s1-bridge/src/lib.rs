//! Spike S1 bridge addon, loaded in Electron main (plan section 18, `pm-bridge`).
//!
//! Registers the per-launch bootstrap name, verifies the engine's single check-in,
//! receives pool surfaces (once) and per-frame descriptors on a native thread, and
//! hands them to JavaScript through a threadsafe function. Also hosts two
//! measurement helpers: a timestamped synthetic input source that queues like the
//! OS input queue does when main is busy, and the display's EDR headroom.
#![cfg(target_os = "macos")]

use napi::bindgen_prelude::*;
use napi::threadsafe_function::{ThreadsafeFunction, ThreadsafeFunctionCallMode};
use napi_derive::napi;
use objc2_core_foundation::CFRetained;
use objc2_io_surface::IOSurfaceRef;
use s1_mach::audit::{ExpectedPeer, SignaturePolicy};
use s1_mach::rendezvous::{Event, Rendezvous};
use s1_mach::{now_us, wire};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

fn err(e: impl std::fmt::Display) -> Error {
    Error::from_reason(e.to_string())
}

/// A retained IOSurface held by main for as long as Chromium may import it.
struct HeldSurface(#[allow(dead_code)] CFRetained<IOSurfaceRef>);
// SAFETY: IOSurface references are atomically reference counted; the API is thread-safe.
unsafe impl Send for HeldSurface {}

#[napi(object)]
#[derive(Default)]
pub struct BridgeEvent {
    /// "accepted" | "rejected" | "surface" | "frame" | "goodbye" | "gone" | "error"
    pub kind: String,
    pub reason: Option<String>,
    pub pid: Option<i32>,
    pub pidversion: Option<i32>,
    pub pool_index: Option<u32>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub format: Option<String>,
    pub bytes_per_row: Option<u32>,
    /// The IOSurfaceRef pointer as Electron's `SharedTextureHandle.ioSurface` expects
    /// (pointer-sized, native endian), valid while main holds the surface.
    pub io_surface: Option<Buffer>,
    pub seq: Option<f64>,
    pub pts_us: Option<f64>,
    pub duration_us: Option<f64>,
    pub revision: Option<f64>,
    pub input_ts_us: Option<f64>,
    pub render_start_us: Option<f64>,
    pub gpu_done_us: Option<f64>,
    pub recv_us: Option<f64>,
    pub flags: Option<u32>,
    pub color_space: Option<u32>,
    pub overlay: Option<Vec<f64>>,
}

type Callback = ThreadsafeFunction<BridgeEvent, (), BridgeEvent, Status, false>;

#[napi]
pub fn service_name() -> String {
    s1_mach::rendezvous::random_service_name("com.powermove.s1.bridge")
}

#[napi]
pub fn now_us_main() -> f64 {
    now_us() as f64
}

#[napi]
pub struct Bridge {
    rendezvous: Option<Rendezvous>,
    policy: SignaturePolicy,
    expected: Option<ExpectedPeer>,
    surfaces: Arc<Mutex<HashMap<u32, HeldSurface>>>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

#[napi]
impl Bridge {
    /// Registers `name`. `allow_unsigned_dev` permits the signature bypass only when
    /// main itself carries no Team ID.
    #[napi(constructor)]
    pub fn new(name: String, allow_unsigned_dev: bool) -> Result<Self> {
        let policy = SignaturePolicy::for_this_process(allow_unsigned_dev).map_err(err)?;
        let rendezvous = Rendezvous::register(&name).map_err(err)?;
        Ok(Self {
            rendezvous: Some(rendezvous),
            policy,
            expected: None,
            surfaces: Arc::new(Mutex::new(HashMap::new())),
            stop: Arc::new(AtomicBool::new(false)),
            thread: None,
        })
    }

    #[napi]
    pub fn policy(&self) -> String {
        match &self.policy {
            SignaturePolicy::TeamId(t) => format!("team-id:{t}"),
            SignaturePolicy::UnsignedDevBypass => "unsigned-dev-bypass".into(),
        }
    }

    /// Records the engine's identity right after spawn (pid, pidversion, parent, uid).
    #[napi]
    pub fn expect_child(&mut self, pid: i32) -> Result<String> {
        let e = ExpectedPeer::for_child(pid, self.policy.clone()).map_err(err)?;
        let s = format!("{{\"pid\":{},\"pidversion\":{}}}", e.pid, e.pidversion.unwrap_or(-1));
        self.expected = Some(e);
        Ok(s)
    }

    /// Starts the native receive thread: one check-in, then surfaces and frames.
    #[napi]
    pub fn start(&mut self, callback: Callback, accept_timeout_ms: u32) -> Result<()> {
        let expected = self.expected.clone().ok_or_else(|| err("expect_child was not called"))?;
        let rendezvous = self.rendezvous.take().ok_or_else(|| err("already started"))?;
        let surfaces = self.surfaces.clone();
        let stop = self.stop.clone();
        let thread = std::thread::Builder::new()
            .name("s1-bridge-recv".into())
            .spawn(move || receive_loop(rendezvous, expected, callback, surfaces, stop, accept_timeout_ms))
            .map_err(err)?;
        self.thread = Some(thread);
        Ok(())
    }

    /// Drops main's reference to a surface (after `allReferencesReleased`).
    #[napi]
    pub fn release_surface(&self, pool_index: u32) -> bool {
        self.surfaces.lock().unwrap().remove(&pool_index).is_some()
    }

    #[napi]
    pub fn held_surfaces(&self) -> u32 {
        self.surfaces.lock().unwrap().len() as u32
    }

    #[napi]
    pub fn stop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

fn set_qos_user_interactive() {
    unsafe extern "C" {
        fn pthread_set_qos_class_self_np(qos: u32, rel: i32) -> i32;
    }
    // SAFETY: affects only the calling thread.
    unsafe { pthread_set_qos_class_self_np(0x21, 0) };
}

fn receive_loop(
    rendezvous: Rendezvous,
    expected: ExpectedPeer,
    cb: Callback,
    surfaces: Arc<Mutex<HashMap<u32, HeldSurface>>>,
    stop: Arc<AtomicBool>,
    accept_timeout_ms: u32,
) {
    set_qos_user_interactive();
    let emit = |ev: BridgeEvent| {
        cb.call(ev, ThreadsafeFunctionCallMode::NonBlocking);
    };
    let (mut conn, rejections) = match rendezvous.accept(&expected, Duration::from_millis(accept_timeout_ms.into())) {
        Ok(v) => v,
        Err((e, rejections)) => {
            for r in rejections {
                emit(BridgeEvent { kind: "rejected".into(), pid: Some(r.pid), reason: Some(r.reason), ..Default::default() });
            }
            emit(BridgeEvent { kind: "error".into(), reason: Some(format!("accept: {e}")), ..Default::default() });
            return;
        }
    };
    for r in rejections {
        emit(BridgeEvent { kind: "rejected".into(), pid: Some(r.pid), reason: Some(r.reason), ..Default::default() });
    }
    let peer = conn.peer();
    emit(BridgeEvent {
        kind: "accepted".into(),
        pid: Some(peer.pid()),
        pidversion: Some(peer.pidversion()),
        ..Default::default()
    });
    while !stop.load(Ordering::SeqCst) {
        let ev = match conn.recv(Duration::from_millis(100)) {
            Ok(None) => continue,
            Ok(Some(ev)) => ev,
            Err(e) => {
                emit(BridgeEvent { kind: "rejected".into(), reason: Some(e.to_string()), ..Default::default() });
                continue;
            }
        };
        let recv_us = now_us() as f64;
        match ev {
            Event::Surface { surface, info } => {
                // Takes our own +1 reference; the port right is released when `surface` drops.
                let Some(io) = IOSurfaceRef::lookup_from_mach_port(surface.name()) else {
                    emit(BridgeEvent { kind: "error".into(), reason: Some("IOSurfaceLookupFromMachPort failed".into()), ..Default::default() });
                    continue;
                };
                let ptr = CFRetained::as_ptr(&io).as_ptr() as usize;
                surfaces.lock().unwrap().insert(info.pool_index, HeldSurface(io));
                emit(BridgeEvent {
                    kind: "surface".into(),
                    pool_index: Some(info.pool_index),
                    width: Some(info.width),
                    height: Some(info.height),
                    format: Some(info.format.electron_name().into()),
                    bytes_per_row: Some(info.bytes_per_row),
                    io_surface: Some(Buffer::from(ptr.to_ne_bytes().to_vec())),
                    recv_us: Some(recv_us),
                    ..Default::default()
                });
            }
            Event::Frame(d) => emit(frame_event(&d, recv_us)),
            Event::Goodbye => emit(BridgeEvent { kind: "goodbye".into(), ..Default::default() }),
            Event::PeerGone => {
                emit(BridgeEvent { kind: "gone".into(), ..Default::default() });
                break;
            }
        }
    }
}

fn frame_event(d: &wire::FrameDescriptor, recv_us: f64) -> BridgeEvent {
    BridgeEvent {
        kind: "frame".into(),
        pool_index: Some(d.pool_index),
        seq: Some(d.seq as f64),
        pts_us: Some(d.pts_us as f64),
        duration_us: Some(d.duration_us as f64),
        revision: Some(d.revision as f64),
        input_ts_us: Some(d.input_ts_us as f64),
        render_start_us: Some(d.render_start_us as f64),
        gpu_done_us: Some(d.gpu_done_us as f64),
        recv_us: Some(recv_us),
        flags: Some(d.flags),
        color_space: Some(d.color_space),
        overlay: Some(d.overlay.iter().map(|&v| f64::from(v)).collect()),
        ..Default::default()
    }
}

// --- Measurement helpers ---------------------------------------------------------------

#[napi(object)]
pub struct SyntheticInput {
    pub seq: u32,
    /// Generation time on CLOCK_UPTIME_RAW, the input event's timestamp.
    pub ts_us: f64,
    pub x: f64,
    pub y: f64,
    /// 0 pointer move (drag/orbit), 1 key (walk).
    pub kind: u32,
}

type InputCallback = ThreadsafeFunction<SyntheticInput, (), SyntheticInput, Status, false>;

#[napi]
pub struct InputGenerator {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

#[napi]
impl InputGenerator {
    /// Generates `hz` events per second for `duration_ms` on a native thread, each
    /// stamped at generation. Events queue behind a busy main thread exactly as OS
    /// input does, so main-thread stalls show up as latency. Coordinates encode the
    /// sequence number (x = 200 + seq % 1200, y = 300 + (seq / 1200) % 600), so each
    /// event is identifiable after coalescing.
    #[napi(constructor)]
    pub fn new(hz: u32, duration_ms: u32, kind: u32, callback: InputCallback) -> Result<Self> {
        if hz == 0 || hz > 1000 {
            return Err(err("hz out of range"));
        }
        let stop = Arc::new(AtomicBool::new(false));
        let s = stop.clone();
        let thread = std::thread::Builder::new()
            .name("s1-input".into())
            .spawn(move || {
                set_qos_user_interactive();
                let period = 1_000_000 / i64::from(hz);
                let t0 = now_us();
                let end = t0 + i64::from(duration_ms) * 1000;
                let mut seq: u32 = 0;
                loop {
                    let due = t0 + i64::from(seq) * period;
                    if due >= end || s.load(Ordering::SeqCst) {
                        break;
                    }
                    let now = now_us();
                    if due > now {
                        std::thread::sleep(Duration::from_micros((due - now) as u64));
                    }
                    let ts = now_us();
                    callback.call(
                        SyntheticInput {
                            seq,
                            ts_us: ts as f64,
                            x: f64::from(200 + seq % 1200),
                            y: f64::from(300 + (seq / 1200) % 600),
                            kind,
                        },
                        ThreadsafeFunctionCallMode::NonBlocking,
                    );
                    seq += 1;
                }
            })
            .map_err(err)?;
        Ok(Self { stop, thread: Some(thread) })
    }

    #[napi]
    pub fn stop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

#[napi(object)]
pub struct EdrHeadroom {
    pub screen: String,
    /// Headroom the system grants right now (rises above 1.0 while EDR content shows).
    pub current: f64,
    pub potential: f64,
    pub reference: f64,
}

/// EDR headroom for every screen. Must be called on the main thread (Electron's
/// main-process JavaScript runs there).
#[napi]
pub fn edr_headroom() -> Result<Vec<EdrHeadroom>> {
    let mtm = objc2_foundation::MainThreadMarker::new().ok_or_else(|| err("not on the main thread"))?;
    let screens = objc2_app_kit::NSScreen::screens(mtm);
    let mut out = Vec::new();
    for s in screens.iter() {
        out.push(EdrHeadroom {
            screen: s.localizedName().to_string(),
            current: s.maximumExtendedDynamicRangeColorComponentValue() as f64,
            potential: s.maximumPotentialExtendedDynamicRangeColorComponentValue() as f64,
            reference: s.maximumReferenceExtendedDynamicRangeColorComponentValue() as f64,
        });
    }
    Ok(out)
}
