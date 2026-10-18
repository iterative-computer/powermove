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
    /// `accepted`, `rejected`, `surface`, `retire`, `remoteLayer`, `goodbye`, `gone`, or `error`.
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
    /// `remoteLayer` (the C′ prototype): the Core Animation context id (0:
    /// gone) and its scale (device pixels per point).
    pub context_id: Option<u32>,
    pub scale: Option<f64>,
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
            HostEvent::RemoteLayer {
                client_port,
                generation,
                context_id,
                width,
                height,
                scale_milli,
            } => Self {
                kind: "remoteLayer".into(),
                client_port: Some(client_port),
                generation: Some(generation),
                context_id: Some(context_id),
                width: Some(width),
                height: Some(height),
                scale: Some(f64::from(scale_milli) / 1000.0),
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

/// Takes a snapshot's pixels out of the engine's shared-memory segment
/// (`shm::take`: maps it once, copies, unlinks). The Phase 3c render
/// transport; main hands the buffer to the window.
#[napi]
pub fn take_pixels(name: String, length: f64) -> Result<napi::bindgen_prelude::Buffer> {
    if !length.is_finite() || length < 1.0 || length > crate::shm::MAX_SEGMENT as f64 {
        return Err(napi::Error::from_reason(format!(
            "a {length}-byte pixel segment"
        )));
    }
    crate::shm::take(&name, length as usize)
        .map(Into::into)
        .map_err(|e| napi::Error::from_reason(format!("pixel segment {name}: {e}")))
}

/// Writes pixels into a fresh shared-memory segment for an engine to take
/// (`shm::publish`; the job engine's compatibility frames). Returns its
/// name; main unlinks it with `unlinkPixels` if the engine never takes it.
#[napi]
pub fn publish_pixels(bytes: napi::bindgen_prelude::Buffer) -> Result<String> {
    crate::shm::publish(&bytes).map_err(|e| napi::Error::from_reason(format!("pixel segment: {e}")))
}

/// Unlinks a segment `publishPixels` made (a no-op once it was taken).
#[napi]
pub fn unlink_pixels(name: String) {
    crate::shm::unlink(&name);
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
    /// Set by the receive thread as it exits.
    finished: Arc<AtomicBool>,
    started: bool,
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
            finished: Arc::new(AtomicBool::new(false)),
            started: false,
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
        if self.started {
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
        let finished = self.finished.clone();
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
                finished.store(true, Ordering::SeqCst);
            })
            .map_err(err)?;
        self.started = true;
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

    /// Stops the receive thread without waiting for it: main's JavaScript
    /// thread calls this when the engine exits, possibly before it checked
    /// in, and must not block. The thread sees the flag within
    /// `STOP_POLL` (waiting for the check-in) or `RECV_POLL` (after it),
    /// emits nothing more, takes no more surfaces, and drops the bootstrap
    /// name or the connection as it exits. Surfaces already held stay held
    /// until released (Chromium may still show them); `releaseAll` drops
    /// every reference.
    #[napi]
    pub fn stop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        // Never started: dropping the rendezvous deregisters the name.
        self.rendezvous = None;
        // Detached: it only touches the shared table and its own ports.
        drop(self.thread.take());
    }

    /// Whether the receive thread was started and has not exited yet
    /// (tests: it ends shortly after `stop`, check-in or not).
    #[napi]
    pub fn running(&self) -> bool {
        self.started && !self.finished.load(Ordering::SeqCst)
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

    /// Lets the window owning `view_handle` (`getNativeWindowHandle()`, an
    /// `NSView*`) sit off every screen even though it has a title bar.
    /// AppKit constrains titled windows onto a screen when they are shown
    /// or moved; the real-app measurement must keep the editor window off
    /// screen while Jude uses the machine, so this replaces
    /// `constrainFrameRect:toScreen:` on the window's class with one that
    /// returns the frame unchanged. Measurement processes only (the
    /// `harness` feature is never in release builds). Call on the main
    /// thread before showing the window.
    #[napi]
    pub fn allow_offscreen_window(view_handle: napi::bindgen_prelude::Buffer) -> Result<String> {
        use objc2::runtime::{AnyClass, AnyObject, Imp, Sel};
        use objc2::{msg_send, sel};
        use objc2_foundation::NSRect;

        extern "C-unwind" fn unconstrained(
            _this: &AnyObject,
            _cmd: Sel,
            frame: NSRect,
            _screen: *mut AnyObject,
        ) -> NSRect {
            frame
        }

        objc2_foundation::MainThreadMarker::new().ok_or_else(|| err("not on the main thread"))?;
        let bytes: &[u8] = &view_handle;
        let raw: [u8; 8] = bytes
            .get(..8)
            .and_then(|b| b.try_into().ok())
            .ok_or_else(|| err("the handle is not an NSView pointer"))?;
        let view = usize::from_ne_bytes(raw) as *const AnyObject;
        if view.is_null() {
            return Err(err("null view"));
        }
        // SAFETY: `getNativeWindowHandle()` returns the window's live content
        // NSView; we are on the main thread.
        let window: *const AnyObject = unsafe { msg_send![&*view, window] };
        if window.is_null() {
            return Err(err("the view has no window"));
        }
        let class: &AnyClass = unsafe { &*window }.class();
        let method = class
            .instance_method(sel!(constrainFrameRect:toScreen:))
            .ok_or_else(|| err("no constrainFrameRect:toScreen:"))?;
        let imp = unconstrained
            as extern "C-unwind" fn(&AnyObject, Sel, NSRect, *mut AnyObject) -> NSRect;
        // SAFETY: the replacement has the method's exact signature
        // (`- (NSRect)constrainFrameRect:(NSRect)r toScreen:(NSScreen*)s`).
        unsafe {
            method.set_implementation(std::mem::transmute::<
                extern "C-unwind" fn(&AnyObject, Sel, NSRect, *mut AnyObject) -> NSRect,
                Imp,
            >(imp))
        };
        Ok(class.name().to_string_lossy().into_owned())
    }

    /// Puts the window owning `view_handle` behind the desktop picture: one
    /// window level below `kCGDesktopWindowLevel`, on every Space,
    /// stationary in Mission Control, out of the window cycle, ignoring the
    /// mouse. It is on a display (Core Animation presents its layers and
    /// reports their presentation times, which it does not for a window off
    /// every screen or at opacity 0) but the desktop picture covers it, so
    /// nothing shows while Jude uses the machine. Returns the level. Call
    /// on the main thread before showing the window. Measurement processes
    /// only.
    #[napi]
    pub fn place_behind_desktop(view_handle: napi::bindgen_prelude::Buffer) -> Result<i64> {
        use objc2::msg_send;
        use objc2::runtime::AnyObject;

        unsafe extern "C" {
            /// CoreGraphics (public): the level for a `CGWindowLevelKey`.
            fn CGWindowLevelForKey(key: i32) -> i32;
        }
        /// `kCGDesktopWindowLevelKey`.
        const DESKTOP_LEVEL_KEY: i32 = 2;
        /// NSWindowCollectionBehavior: CanJoinAllSpaces | Stationary | IgnoresCycle.
        const BEHAVIOR: usize = (1 << 0) | (1 << 4) | (1 << 6);

        objc2_foundation::MainThreadMarker::new().ok_or_else(|| err("not on the main thread"))?;
        let bytes: &[u8] = &view_handle;
        let raw: [u8; 8] = bytes
            .get(..8)
            .and_then(|b| b.try_into().ok())
            .ok_or_else(|| err("the handle is not an NSView pointer"))?;
        let view = usize::from_ne_bytes(raw) as *const AnyObject;
        if view.is_null() {
            return Err(err("null view"));
        }
        // SAFETY: `getNativeWindowHandle()` returns the window's live content
        // NSView; main thread; documented NSWindow setters.
        unsafe {
            let window: *const AnyObject = msg_send![&*view, window];
            if window.is_null() {
                return Err(err("the view has no window"));
            }
            let level = isize::try_from(CGWindowLevelForKey(DESKTOP_LEVEL_KEY))
                .map_err(|_| err("window level"))?
                - 1;
            let _: () = msg_send![&*window, setLevel: level];
            let _: () = msg_send![&*window, setCollectionBehavior: BEHAVIOR];
            let _: () = msg_send![&*window, setIgnoresMouseEvents: true];
            let set: isize = msg_send![&*window, level];
            if set != level {
                return Err(err("the window level did not take"));
            }
            Ok(set as i64)
        }
    }

    thread_local! {
        /// The C′ prototype's `CALayerHost`s by id (main thread only).
        static LAYER_HOSTS: std::cell::RefCell<std::collections::HashMap<u32, objc2::rc::Retained<objc2::runtime::AnyObject>>> =
            std::cell::RefCell::new(std::collections::HashMap::new());
        static NEXT_HOST: std::cell::Cell<u32> = const { std::cell::Cell::new(1) };
    }

    /// The C′ prototype (docs/engine/bridge.md): hosts the engine's remote
    /// Core Animation context `context_id` in the window owning
    /// `view_handle` (`getNativeWindowHandle()`), as a `CALayerHost` over
    /// the web contents at `x, y, width, height` points from the content
    /// view's top left. `CALayerHost` is private API, which is why this is
    /// a harness function: the prototype is measured, not shipped. Returns
    /// an id for [`remove_remote_layer`].
    #[napi]
    pub fn host_remote_layer(
        view_handle: napi::bindgen_prelude::Buffer,
        context_id: u32,
        x: f64,
        y: f64,
        width: f64,
        height: f64,
    ) -> Result<u32> {
        use objc2::msg_send;
        use objc2::rc::Retained;
        use objc2::runtime::{AnyClass, AnyObject};
        use objc2_foundation::{NSPoint, NSRect, NSSize};

        objc2_foundation::MainThreadMarker::new().ok_or_else(|| err("not on the main thread"))?;
        let bytes: &[u8] = &view_handle;
        let raw: [u8; 8] = bytes
            .get(..8)
            .and_then(|b| b.try_into().ok())
            .ok_or_else(|| err("the handle is not an NSView pointer"))?;
        let view = usize::from_ne_bytes(raw) as *const AnyObject;
        if view.is_null() {
            return Err(err("null view"));
        }
        let class =
            AnyClass::get(c"CALayerHost").ok_or_else(|| err("CALayerHost is unavailable"))?;
        // SAFETY: main thread; the view is the window's live content view.
        unsafe {
            let parent: *const AnyObject = msg_send![&*view, layer];
            if parent.is_null() {
                return Err(err("the content view has no layer"));
            }
            let flipped: bool = msg_send![&*parent, isGeometryFlipped];
            let bounds: NSRect = msg_send![&*parent, bounds];
            let host: Retained<AnyObject> = msg_send![class, new];
            let _: () = msg_send![objc2_quartz_core_transaction(), begin];
            let _: () = msg_send![objc2_quartz_core_transaction(), setDisableActions: true];
            let _: () = msg_send![&*host, setContextId: context_id];
            let top = if flipped {
                y
            } else {
                bounds.size.height - y - height
            };
            let frame = NSRect::new(NSPoint::new(x, top), NSSize::new(width, height));
            let _: () = msg_send![&*host, setFrame: frame];
            let _: () = msg_send![&*host, setZPosition: 1000.0f64];
            let _: () = msg_send![&*parent, addSublayer: &*host];
            let _: () = msg_send![objc2_quartz_core_transaction(), commit];
            let id = NEXT_HOST.with(|n| {
                let id = n.get();
                n.set(id + 1);
                id
            });
            LAYER_HOSTS.with(|hosts| hosts.borrow_mut().insert(id, host));
            Ok(id)
        }
    }

    /// Removes a `CALayerHost` made by [`host_remote_layer`].
    #[napi]
    pub fn remove_remote_layer(id: u32) -> Result<bool> {
        use objc2::msg_send;
        objc2_foundation::MainThreadMarker::new().ok_or_else(|| err("not on the main thread"))?;
        let host = LAYER_HOSTS.with(|hosts| hosts.borrow_mut().remove(&id));
        Ok(match host {
            Some(host) => {
                // SAFETY: main thread; a layer we added.
                let _: () = unsafe { msg_send![&*host, removeFromSuperlayer] };
                true
            }
            None => false,
        })
    }

    fn objc2_quartz_core_transaction() -> &'static objc2::runtime::AnyClass {
        objc2::runtime::AnyClass::get(c"CATransaction").expect("CATransaction")
    }

    #[napi(object)]
    pub struct DisplayState {
        /// The main display is asleep (`CGDisplayIsAsleep`): nothing is
        /// scanned out, so Core Animation presents no remote layer drawable
        /// and Chromium's presentation times are its own estimates, not
        /// the panel's.
        pub asleep: bool,
        /// The login session's screen is locked (`CGSSessionScreenIsLocked`
        /// in `CGSessionCopyCurrentDictionary`).
        pub locked: bool,
    }

    #[link(name = "CoreGraphics", kind = "framework")]
    unsafe extern "C" {
        fn CGMainDisplayID() -> u32;
        fn CGDisplayIsAsleep(display: u32) -> u32;
        fn CGSessionCopyCurrentDictionary() -> *mut objc2::runtime::AnyObject;
    }

    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        fn CFRelease(cf: *const std::ffi::c_void);
    }

    /// `CAFrameRateRange` (QuartzCore, macOS 12).
    #[repr(C)]
    #[derive(Clone, Copy)]
    struct FrameRateRange {
        minimum: f32,
        maximum: f32,
        preferred: f32,
    }

    // SAFETY: the layout and encoding of `CAFrameRateRange`.
    unsafe impl objc2::Encode for FrameRateRange {
        const ENCODING: objc2::Encoding = objc2::Encoding::Struct(
            "CAFrameRateRange",
            &[
                objc2::Encoding::Float,
                objc2::Encoding::Float,
                objc2::Encoding::Float,
            ],
        );
    }

    /// Display link callbacks seen while a main-process rate request lives
    /// (`targetTimestamp - timestamp` would be the link's own period; the
    /// spacing of `timestamp`s is the panel's).
    static RATE_TICKS: std::sync::Mutex<Vec<f64>> = std::sync::Mutex::new(Vec::new());

    objc2::define_class!(
        /// The main-process display link's target: records refresh times.
        #[unsafe(super(objc2_foundation::NSObject))]
        struct RateTicker;

        impl RateTicker {
            #[unsafe(method(tick:))]
            fn tick(&self, link: &objc2::runtime::AnyObject) {
                // SAFETY: CADisplayLink's timestamp property.
                let now: f64 = unsafe { objc2::msg_send![link, timestamp] };
                if let Ok(mut ticks) = RATE_TICKS.lock()
                    && ticks.len() < 100_000
                {
                    ticks.push(now);
                }
            }
        }
    );

    thread_local! {
        /// The live main-process rate request (main thread only).
        static RATE_LINK: std::cell::RefCell<Option<objc2::rc::Retained<objc2::runtime::AnyObject>>> =
            const { std::cell::RefCell::new(None) };
    }

    /// Section 15's ProMotion request from Electron's main process, through
    /// public API: a display link on the main screen
    /// (`-[NSScreen displayLinkWithTarget:selector:]`, macOS 14) whose
    /// `preferredFrameRateRange` is exactly `hz`, on main's run loop.
    /// Electron has no refresh-rate request for its compositor
    /// (`webContents.setFrameRate` is offscreen rendering only), so this is
    /// the measurement of whether an app-level request reaches the panel
    /// while Chromium draws. `hz` 0 removes the request. Returns the refresh
    /// times recorded since the previous call (seconds, host clock).
    #[napi]
    pub fn request_refresh_rate(hz: f64) -> Result<Vec<f64>> {
        use objc2::msg_send;
        use objc2::rc::Retained;
        use objc2::runtime::{AnyClass, AnyObject};
        objc2_foundation::MainThreadMarker::new().ok_or_else(|| err("not on the main thread"))?;
        let ticks = std::mem::take(&mut *RATE_TICKS.lock().unwrap_or_else(|e| e.into_inner()));
        RATE_LINK.with(|slot| -> Result<()> {
            if let Some(old) = slot.borrow_mut().take() {
                // SAFETY: a link we made on this thread.
                let _: () = unsafe { msg_send![&*old, invalidate] };
            }
            if hz <= 0.0 {
                return Ok(());
            }
            // SAFETY: AppKit and Core Animation on the main thread.
            unsafe {
                let screen: Option<Retained<AnyObject>> =
                    msg_send![AnyClass::get(c"NSScreen").ok_or_else(|| err("no AppKit"))?, mainScreen];
                let screen = screen.ok_or_else(|| err("no main screen"))?;
                let target: Retained<RateTicker> = msg_send![<RateTicker as objc2::ClassType>::class(), new];
                let link: Option<Retained<AnyObject>> = msg_send![&*screen, displayLinkWithTarget: &*target, selector: objc2::sel!(tick:)];
                let link = link.ok_or_else(|| err("no display link"))?;
                let range = FrameRateRange {
                    minimum: hz as f32,
                    maximum: hz as f32,
                    preferred: hz as f32,
                };
                let _: () = msg_send![&*link, setPreferredFrameRateRange: range];
                let run_loop: Retained<AnyObject> =
                    msg_send![AnyClass::get(c"NSRunLoop").ok_or_else(|| err("no NSRunLoop"))?, mainRunLoop];
                let _: () = msg_send![&*link, addToRunLoop: &*run_loop, forMode: objc2_foundation::NSRunLoopCommonModes];
                *slot.borrow_mut() = Some(link);
            }
            Ok(())
        })?;
        Ok(ticks)
    }

    /// Whether a measurement can see the panel: the main display awake and
    /// the session unlocked (S1 harness runs record it and refuse to
    /// measure otherwise).
    #[napi]
    pub fn display_state() -> DisplayState {
        use objc2::msg_send;
        use objc2::runtime::AnyObject;
        // SAFETY: public CoreGraphics calls; the session dictionary is a
        // CFDictionary (toll-free bridged to NSDictionary) we own and release.
        unsafe {
            let asleep = CGDisplayIsAsleep(CGMainDisplayID()) != 0;
            let session = CGSessionCopyCurrentDictionary();
            let locked = if session.is_null() {
                false
            } else {
                let key = objc2_foundation::NSString::from_str("CGSSessionScreenIsLocked");
                let value: *mut AnyObject = msg_send![&*session, objectForKey: &*key];
                let locked = !value.is_null() && {
                    let b: bool = msg_send![&*value, boolValue];
                    b
                };
                CFRelease(session.cast());
                locked
            };
            DisplayState { asleep, locked }
        }
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
