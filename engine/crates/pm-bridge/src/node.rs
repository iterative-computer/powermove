//! The addon surface Electron main loads (feature `node`), used by
//! `apps/desktop/src/main/engine/bridge.ts`.
//!
//! One [`JsBridgeHost`] per interactive engine launch: it registers the
//! per-launch bootstrap name before the fork, records the engine's identity
//! right after the spawn, and runs the receive loop on a native thread at
//! user-interactive QoS. Events reach JavaScript through a threadsafe
//! function, in order.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use napi::bindgen_prelude::*;
use napi::threadsafe_function::{ThreadsafeFunction, ThreadsafeFunctionCallMode};
use napi_derive::napi;

use crate::audit::{ExpectedPeer, SignaturePolicy};
use crate::host::{self, HostEvent, SurfaceTable};
use crate::rendezvous::{Rendezvous, SERVICE_PREFIX, random_service_name};

fn err(e: impl std::fmt::Display) -> Error {
    Error::from_reason(e.to_string())
}

/// One bridge event for JavaScript.
#[napi(object)]
#[derive(Default)]
pub struct BridgeEvent {
    /// `accepted`, `rejected`, `surface`, `retire`, `goodbye`, `gone`, or `error`.
    pub kind: String,
    pub reason: Option<String>,
    pub pid: Option<i32>,
    pub pidversion: Option<i32>,
    pub client_port: Option<u32>,
    pub generation: Option<u32>,
    pub pool_index: Option<u32>,
    pub pool_size: Option<u32>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    /// Electron's pixel format name: `bgra` or `rgbaf16`.
    pub format: Option<String>,
    pub bytes_per_row: Option<u32>,
    pub alloc_bytes: Option<f64>,
    /// The `IOSurfaceRef` pointer as `SharedTextureHandle.ioSurface` takes it
    /// (pointer-sized, native endian). Valid until `releaseSurface`.
    pub io_surface: Option<Buffer>,
    /// `CLOCK_UPTIME_RAW` µs when the native thread received the message.
    pub received_us: Option<f64>,
}

impl BridgeEvent {
    fn from_host(event: HostEvent) -> Self {
        let received_us = Some(crate::now_us() as f64);
        match event {
            HostEvent::Accepted { pid, pidversion } => Self {
                kind: "accepted".into(),
                pid: Some(pid),
                pidversion: Some(pidversion),
                received_us,
                ..Default::default()
            },
            HostEvent::Rejected { pid, reason } => Self {
                kind: "rejected".into(),
                pid: Some(pid),
                reason: Some(reason),
                received_us,
                ..Default::default()
            },
            HostEvent::Surface {
                info,
                io_surface,
                alloc_bytes,
            } => Self {
                kind: "surface".into(),
                client_port: Some(info.client_port),
                generation: Some(info.generation),
                pool_index: Some(info.pool_index),
                pool_size: Some(info.pool_size),
                width: Some(info.width),
                height: Some(info.height),
                format: Some(info.format.electron_name().into()),
                bytes_per_row: Some(info.bytes_per_row),
                alloc_bytes: Some(alloc_bytes as f64),
                io_surface: Some(Buffer::from(io_surface.to_ne_bytes().to_vec())),
                received_us,
                ..Default::default()
            },
            HostEvent::Retire {
                client_port,
                generation,
            } => Self {
                kind: "retire".into(),
                client_port: Some(client_port),
                generation: Some(generation),
                received_us,
                ..Default::default()
            },
            HostEvent::Goodbye => Self {
                kind: "goodbye".into(),
                received_us,
                ..Default::default()
            },
            HostEvent::Gone => Self {
                kind: "gone".into(),
                received_us,
                ..Default::default()
            },
            HostEvent::Error { reason } => Self {
                kind: "error".into(),
                reason: Some(reason),
                received_us,
                ..Default::default()
            },
        }
    }
}

type Callback = ThreadsafeFunction<BridgeEvent, (), BridgeEvent, Status, false>;

/// A fresh per-launch bootstrap name.
#[napi]
pub fn service_name() -> Result<String> {
    random_service_name(SERVICE_PREFIX).map_err(err)
}

/// The shared host clock in µs (`CLOCK_UPTIME_RAW`).
#[napi]
pub fn now_us() -> f64 {
    crate::now_us() as f64
}

/// Main's own Team ID, or `null` when main is not signed with a certificate.
#[napi]
pub fn own_team_id() -> Result<Option<String>> {
    crate::audit::own_team_id().map_err(err)
}

/// The engine's identity as recorded after the spawn.
#[napi(object)]
pub struct ExpectedEngine {
    pub pid: i32,
    pub pidversion: i32,
    pub parent_pid: i32,
}

/// What main holds for one launch.
#[napi(object)]
pub struct HeldSurfaces {
    pub count: u32,
    pub bytes: f64,
}

/// The bridge for one interactive engine launch.
#[napi(js_name = "BridgeHost")]
pub struct JsBridgeHost {
    name: String,
    rendezvous: Option<Rendezvous>,
    policy: SignaturePolicy,
    expected: Option<ExpectedPeer>,
    table: Arc<Mutex<SurfaceTable>>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

#[napi]
impl JsBridgeHost {
    /// Registers `name` in the bootstrap namespace. The signature policy
    /// comes from main's own signature: a signed main always requires its
    /// Team ID; `allow_unsigned_dev` permits the development bypass only when
    /// main itself has no Team ID.
    #[napi(constructor)]
    pub fn new(name: String, allow_unsigned_dev: bool) -> Result<Self> {
        if !crate::rendezvous::is_service_name(&name) {
            return Err(err(format!("{name:?} is not a bridge service name")));
        }
        let policy = SignaturePolicy::for_this_process(allow_unsigned_dev).map_err(err)?;
        let rendezvous = Rendezvous::register(&name).map_err(err)?;
        Ok(Self {
            name,
            rendezvous: Some(rendezvous),
            policy,
            expected: None,
            table: Arc::new(Mutex::new(SurfaceTable::default())),
            stop: Arc::new(AtomicBool::new(false)),
            thread: None,
        })
    }

    #[napi(getter)]
    pub fn name(&self) -> String {
        self.name.clone()
    }

    /// `team-id:<ID>` or `unsigned-dev-bypass`.
    #[napi]
    pub fn policy(&self) -> String {
        match &self.policy {
            SignaturePolicy::TeamId(team) => format!("team-id:{team}"),
            SignaturePolicy::UnsignedDevBypass => "unsigned-dev-bypass".into(),
        }
    }

    /// Records the engine's identity right after its spawn (pid, pidversion,
    /// parent, uid). Only a check-in from exactly this process instance is
    /// accepted.
    #[napi]
    pub fn expect_child(&mut self, pid: i32) -> Result<ExpectedEngine> {
        if self.thread.is_some() {
            return Err(err("the bridge already started"));
        }
        let expected = ExpectedPeer::for_child(pid, self.policy.clone()).map_err(err)?;
        let out = ExpectedEngine {
            pid: expected.pid,
            pidversion: expected.pidversion.unwrap_or(-1),
            parent_pid: expected.parent_pid,
        };
        self.expected = Some(expected);
        Ok(out)
    }

    /// Starts the receive thread: one check-in, then surfaces, retirements,
    /// and the engine's exit.
    #[napi]
    pub fn start(&mut self, callback: Callback, accept_timeout_ms: u32) -> Result<()> {
        let expected = self
            .expected
            .clone()
            .ok_or_else(|| err("expectChild was not called"))?;
        let rendezvous = self
            .rendezvous
            .take()
            .ok_or_else(|| err("the bridge already started"))?;
        let table = self.table.clone();
        let stop = self.stop.clone();
        let timeout = Duration::from_millis(accept_timeout_ms.into());
        let thread = std::thread::Builder::new()
            .name("pm-bridge".into())
            .spawn(move || {
                crate::set_thread_user_interactive();
                host::run(
                    rendezvous,
                    &expected,
                    timeout,
                    &table,
                    &stop,
                    &mut |event| {
                        callback.call(
                            BridgeEvent::from_host(event),
                            ThreadsafeFunctionCallMode::NonBlocking,
                        );
                    },
                );
            })
            .map_err(err)?;
        self.thread = Some(thread);
        Ok(())
    }

    /// Drops main's reference to a surface after `allReferencesReleased`.
    #[napi]
    pub fn release_surface(&self, generation: u32, pool_index: u32) -> bool {
        self.table
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .release(generation, pool_index)
    }

    /// The retain count of a held surface's `IOSurfaceRef` in main (tests:
    /// whether an import keeps its own reference).
    #[napi]
    pub fn surface_retain_count(&self, generation: u32, pool_index: u32) -> Option<i64> {
        self.table
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .retain_count(generation, pool_index)
            .map(|n| n as i64)
    }

    #[napi]
    pub fn held(&self) -> HeldSurfaces {
        let table = self.table.lock().unwrap_or_else(|e| e.into_inner());
        HeldSurfaces {
            count: table.len() as u32,
            bytes: table.bytes() as f64,
        }
    }

    /// Stops the receive thread. Surfaces stay held until released (Chromium
    /// may still show them); `releaseAll` drops every reference.
    #[napi]
    pub fn stop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        // Never started: dropping the rendezvous deregisters the name.
        self.rendezvous = None;
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }

    /// Drops every surface reference main holds (app quit).
    #[napi]
    pub fn release_all(&self) {
        self.table.lock().unwrap_or_else(|e| e.into_inner()).clear();
    }
}

impl Drop for JsBridgeHost {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(feature = "harness")]
mod harness {
    //! Measurement helpers for the S1 harness only.

    use super::*;

    #[napi(object)]
    pub struct SyntheticInput {
        pub seq: u32,
        /// Generation time on `CLOCK_UPTIME_RAW`, the input event's timestamp.
        pub ts_us: f64,
        pub x: f64,
        pub y: f64,
        /// 0 pointer move (drag, orbit), 1 key (walk).
        pub kind: u32,
    }

    type InputCallback = ThreadsafeFunction<SyntheticInput, (), SyntheticInput, Status, false>;

    /// Timestamped synthetic input at `hz` for `duration_ms`, generated on a
    /// native thread. The events queue behind a busy main thread exactly as
    /// OS input does. Coordinates encode the sequence number
    /// (x = 200 + seq % 1200, y = 300 + (seq / 1200) % 600).
    #[napi]
    pub struct InputGenerator {
        stop: Arc<AtomicBool>,
        thread: Option<JoinHandle<()>>,
    }

    #[napi]
    impl InputGenerator {
        #[napi(constructor)]
        pub fn new(hz: u32, duration_ms: u32, kind: u32, callback: InputCallback) -> Result<Self> {
            if hz == 0 || hz > 1000 {
                return Err(err("hz out of range"));
            }
            let stop = Arc::new(AtomicBool::new(false));
            let flag = stop.clone();
            let thread = std::thread::Builder::new()
                .name("pm-bridge-input".into())
                .spawn(move || {
                    crate::set_thread_user_interactive();
                    let period = 1_000_000 / i64::from(hz);
                    let t0 = crate::now_us();
                    let end = t0 + i64::from(duration_ms) * 1000;
                    let mut seq: u32 = 0;
                    loop {
                        let due = t0 + i64::from(seq) * period;
                        if due >= end || flag.load(Ordering::SeqCst) {
                            break;
                        }
                        let now = crate::now_us();
                        if due > now {
                            std::thread::sleep(Duration::from_micros((due - now) as u64));
                        }
                        callback.call(
                            SyntheticInput {
                                seq,
                                ts_us: crate::now_us() as f64,
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
            Ok(Self {
                stop,
                thread: Some(thread),
            })
        }

        #[napi]
        pub fn stop(&mut self) {
            self.stop.store(true, Ordering::SeqCst);
            if let Some(thread) = self.thread.take() {
                let _ = thread.join();
            }
        }
    }

    #[napi(object)]
    pub struct EdrHeadroom {
        pub screen: String,
        /// The headroom granted now (above 1.0 while EDR content shows).
        pub current: f64,
        pub potential: f64,
        pub reference: f64,
    }

    /// EDR headroom per screen. Call on the main thread.
    #[napi]
    pub fn edr_headroom() -> Result<Vec<EdrHeadroom>> {
        let mtm = objc2_foundation::MainThreadMarker::new()
            .ok_or_else(|| err("not on the main thread"))?;
        let screens = objc2_app_kit::NSScreen::screens(mtm);
        Ok(screens
            .iter()
            .map(|s| EdrHeadroom {
                screen: s.localizedName().to_string(),
                current: s.maximumExtendedDynamicRangeColorComponentValue(),
                potential: s.maximumPotentialExtendedDynamicRangeColorComponentValue(),
                reference: s.maximumReferenceExtendedDynamicRangeColorComponentValue(),
            })
            .collect())
    }
}
