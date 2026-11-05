//! Linked Sparkle 2.10.1 public API. Commands enqueue on main; callbacks only
//! copy bounded events into a worker queue. No main-thread Rust I/O or wait.
use crate::update_distribution::{
    Release,
    sparkle::{Enclosure, Event, Session},
};
use anyhow::{Result, bail, ensure};
use serde::Deserialize;
use std::{
    ffi::c_void,
    ptr::NonNull,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
};

unsafe extern "C" {
    fn pm_sparkle_new(
        context: *mut c_void,
        event: extern "C" fn(*mut c_void, *const u8, usize) -> bool,
        permit: extern "C" fn(*mut c_void) -> bool,
        release: extern "C" fn(*mut c_void),
    ) -> *mut c_void;
    fn pm_sparkle_command(handle: *mut c_void, data: *const u8, length: usize);
    fn pm_sparkle_drop(handle: *mut c_void);
}
struct Context {
    events: mpsc::SyncSender<Vec<u8>>,
    permit: Arc<AtomicBool>,
    overflow: Arc<AtomicBool>,
}
extern "C" fn event(context: *mut c_void, bytes: *const u8, length: usize) -> bool {
    // Native retains Context until its last queued command/session is released.
    let context = unsafe { &*context.cast::<Context>() };
    if length > 16 * 1024 {
        context.overflow.store(true, Ordering::Release);
        return false;
    }
    let bytes = unsafe { std::slice::from_raw_parts(bytes, length) }.to_vec();
    if context.events.try_send(bytes).is_err() {
        context.overflow.store(true, Ordering::Release);
        context.permit.store(false, Ordering::Release);
        return false;
    }
    true
}
extern "C" fn permitted(context: *mut c_void) -> bool {
    unsafe { &*context.cast::<Context>() }
        .permit
        .load(Ordering::Acquire)
}
extern "C" fn release(context: *mut c_void) {
    drop(unsafe { Box::from_raw(context.cast::<Context>()) });
}
#[derive(Deserialize)]
struct Message {
    id: String,
    kind: String,
    enclosure: Option<Enclosure>,
}

/// The application owns `permit`: true only while real source/import leases and
/// its ordinary save/quit termination guard are installed. Revoke before releasing
/// them. This does not install a termination hook or claim installer recovery.
pub struct NativeSession {
    handle: NonNull<c_void>,
    events: mpsc::Receiver<Vec<u8>>,
    permit: Arc<AtomicBool>,
    overflow: Arc<AtomicBool>,
}
impl NativeSession {
    pub fn new(permit: Arc<AtomicBool>) -> Result<Self> {
        ensure!(
            permit.load(Ordering::Acquire),
            "termination/import guard is not active"
        );
        let (events, receive) = mpsc::sync_channel(64);
        let overflow = Arc::new(AtomicBool::new(false));
        let context = Box::into_raw(Box::new(Context {
            events,
            permit: permit.clone(),
            overflow: overflow.clone(),
        }));
        let handle = unsafe { pm_sparkle_new(context.cast(), event, permitted, release) };
        let Some(handle) = NonNull::new(handle) else {
            release(context.cast());
            bail!("Sparkle bridge allocation failed");
        };
        Ok(Self {
            handle,
            events: receive,
            permit,
            overflow,
        })
    }
    fn command(&self, value: serde_json::Value) -> Result<()> {
        let bytes = serde_json::to_vec(&value)?;
        unsafe {
            pm_sparkle_command(self.handle.as_ptr(), bytes.as_ptr(), bytes.len());
        }
        Ok(())
    }
    pub fn poll(&self) -> Result<Option<(String, Event)>> {
        ensure!(
            !self.overflow.load(Ordering::Acquire),
            "Sparkle callback queue overflow; guard revoked"
        );
        let bytes = match self.events.try_recv() {
            Ok(bytes) => bytes,
            Err(mpsc::TryRecvError::Empty) => return Ok(None),
            Err(error) => return Err(error.into()),
        };
        let message: Message = serde_json::from_slice(&bytes)?;
        let event = match message.kind.as_str() {
            "found" => Event::Found(
                message
                    .enclosure
                    .ok_or_else(|| anyhow::anyhow!("missing Sparkle enclosure"))?,
            ),
            "extracting" => Event::Extracting,
            "ready" => Event::Ready,
            "installed" => Event::Installed,
            "recovery_required" => Event::RecoveryRequired,
            _ => Event::Aborted,
        };
        Ok(Some((message.id, event)))
    }
}
impl Session for NativeSession {
    fn check(&mut self, id: &str, release: &Release) -> Result<()> {
        self.command(serde_json::json!({"command":"check","id":id,"release":release}))
    }
    fn download(&mut self, id: &str) -> Result<()> {
        self.command(serde_json::json!({"command":"download","id":id}))
    }
    fn install(&mut self, id: &str) -> Result<()> {
        self.command(serde_json::json!({"command":"install","id":id}))
    }
    fn cancel(&mut self, id: &str) -> Result<()> {
        self.command(serde_json::json!({"command":"cancel","id":id}))
    }
}
impl Drop for NativeSession {
    fn drop(&mut self) {
        self.permit.store(false, Ordering::Release);
        unsafe {
            pm_sparkle_drop(self.handle.as_ptr());
        }
    }
}
