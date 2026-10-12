//! The Mach bootstrap rendezvous (plan section 18, step 1).
//!
//! Main registers a receive right under a per-launch random bootstrap name and passes
//! the name to the engine in its environment. The engine looks the name up and sends
//! one check-in carrying a send right to its own port. Main verifies the sender's
//! audit token (pid, pidversion, uid, parent) and code signature, then destroys the
//! service port, which removes the name from the bootstrap namespace and discards any
//! other queued check-ins, and replies with a send right to a fresh anonymous port.
//! Every later message on that port is checked against the same audit token.

use crate::audit::{AuditToken, ExpectedPeer};
use crate::ffi::{self, mach_port_t};
use crate::wire::{self, Incoming, OutMessage};
use std::ffi::CString;
use std::time::{Duration, Instant};

#[derive(Debug, thiserror::Error)]
pub enum BridgeError {
    #[error("bootstrap {op}({name}): {msg}")]
    Bootstrap { op: &'static str, name: String, msg: String },
    #[error("mach {op}: {msg}")]
    Mach { op: &'static str, msg: String },
    #[error("timed out")]
    TimedOut,
    #[error("rejected: {0}")]
    Rejected(String),
    #[error("peer is gone")]
    PeerGone,
}

/// An owned receive right.
#[derive(Debug)]
pub struct ReceiveRight(mach_port_t);
impl ReceiveRight {
    pub fn allocate() -> Result<Self, BridgeError> {
        let mut name = 0;
        // SAFETY: out-pointer valid.
        let kr = unsafe { ffi::mach_port_allocate(ffi::task_self(), ffi::MACH_PORT_RIGHT_RECEIVE, &mut name) };
        check(kr, "mach_port_allocate")?;
        Ok(Self(name))
    }
    pub fn name(&self) -> mach_port_t {
        self.0
    }
    fn set_qlimit(&self, limit: u32) -> Result<(), BridgeError> {
        let limits = ffi::mach_port_limits_t { mpl_qlimit: limit };
        // SAFETY: limits struct is one natural_t, matching the count.
        let kr = unsafe {
            ffi::mach_port_set_attributes(
                ffi::task_self(),
                self.0,
                ffi::MACH_PORT_LIMITS_INFO,
                (&limits as *const ffi::mach_port_limits_t).cast(),
                ffi::MACH_PORT_LIMITS_INFO_COUNT,
            )
        };
        check(kr, "mach_port_set_attributes")
    }
    /// Arms a no-more-senders notification delivered to this same port. `sync` is the
    /// make-send count that must have been reached (1 after the first right was made),
    /// so the notification does not fire before the peer holds its right.
    fn request_no_senders(&self, sync: u32) -> Result<(), BridgeError> {
        let mut previous = 0;
        // SAFETY: valid port names; previous is an out-pointer.
        let kr = unsafe {
            ffi::mach_port_request_notification(
                ffi::task_self(),
                self.0,
                ffi::MACH_NOTIFY_NO_SENDERS,
                sync,
                self.0,
                ffi::MACH_MSG_TYPE_MAKE_SEND_ONCE,
                &mut previous,
            )
        };
        check(kr, "mach_port_request_notification")?;
        if previous != ffi::MACH_PORT_NULL {
            // SAFETY: releasing a previously registered send-once right.
            unsafe { ffi::mach_port_deallocate(ffi::task_self(), previous) };
        }
        Ok(())
    }
}
impl Drop for ReceiveRight {
    fn drop(&mut self) {
        // SAFETY: we own exactly one receive right on this name.
        unsafe { ffi::mach_port_mod_refs(ffi::task_self(), self.0, ffi::MACH_PORT_RIGHT_RECEIVE, -1) };
    }
}

/// An owned send right.
#[derive(Debug)]
pub struct SendRight(mach_port_t);
impl SendRight {
    /// Takes ownership of a send right received in a message.
    pub fn from_raw(name: mach_port_t) -> Self {
        Self(name)
    }
    pub fn name(&self) -> mach_port_t {
        self.0
    }
    /// Gives up ownership (e.g. when the right is moved into a message).
    pub fn into_raw(self) -> mach_port_t {
        let n = self.0;
        std::mem::forget(self);
        n
    }
}
impl Drop for SendRight {
    fn drop(&mut self) {
        // SAFETY: we own one user reference of this send right.
        unsafe { ffi::mach_port_deallocate(ffi::task_self(), self.0) };
    }
}

fn check(kr: ffi::kern_return_t, op: &'static str) -> Result<(), BridgeError> {
    if kr == ffi::KERN_SUCCESS { Ok(()) } else { Err(BridgeError::Mach { op, msg: ffi::kr_string(kr) }) }
}

/// A random, per-launch bootstrap name.
pub fn random_service_name(prefix: &str) -> String {
    let mut bytes = [0u8; 16];
    // SAFETY: buffer of 16 bytes (getentropy allows up to 256).
    let rc = unsafe { libc::getentropy(bytes.as_mut_ptr().cast(), bytes.len()) };
    assert_eq!(rc, 0, "getentropy failed");
    let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    format!("{prefix}.{hex}")
}

/// Aligned receive buffer large enough for any bridge message plus the trailer.
#[repr(C, align(8))]
struct RecvBuf([u8; wire::MAX_RECEIVE + 8]);

enum Received {
    Message,
    TimedOut,
}

fn receive(port: mach_port_t, buf: &mut RecvBuf, timeout: Duration) -> Result<Received, BridgeError> {
    let ms = timeout.as_millis().min(u32::MAX as u128) as u32;
    let opts = ffi::MACH_RCV_MSG
        | ffi::MACH_RCV_TIMEOUT
        | ffi::mach_rcv_trailer_type(ffi::MACH_MSG_TRAILER_FORMAT_0)
        | ffi::mach_rcv_trailer_elements(ffi::MACH_RCV_TRAILER_AUDIT);
    loop {
        // SAFETY: the buffer is large and aligned enough for a header; the kernel
        // writes at most rcv_size bytes. Without MACH_RCV_LARGE, an oversized message
        // is destroyed by the kernel and MACH_RCV_TOO_LARGE is returned.
        let kr = unsafe {
            ffi::mach_msg(
                buf.0.as_mut_ptr().cast(),
                opts,
                0,
                buf.0.len() as u32,
                port,
                ms,
                ffi::MACH_PORT_NULL,
            )
        };
        match kr {
            ffi::KERN_SUCCESS => return Ok(Received::Message),
            ffi::MACH_RCV_TIMED_OUT => return Ok(Received::TimedOut),
            ffi::MACH_RCV_INTERRUPTED => continue,
            // Oversized messages are dropped by the kernel; keep receiving.
            ffi::MACH_RCV_TOO_LARGE => continue,
            other => return Err(BridgeError::Mach { op: "mach_msg(receive)", msg: ffi::kr_string(other) }),
        }
    }
}

/// Releases any rights carried by a received message that we are not keeping.
fn destroy(buf: &mut RecvBuf) {
    // SAFETY: the buffer holds a message the kernel just delivered.
    unsafe { ffi::mach_msg_destroy(buf.0.as_mut_ptr().cast()) };
}

fn msg_id(buf: &RecvBuf) -> i32 {
    i32::from_ne_bytes([buf.0[20], buf.0[21], buf.0[22], buf.0[23]])
}

fn send(mut msg: OutMessage, timeout: Duration) -> Result<(), BridgeError> {
    let ms = timeout.as_millis().min(u32::MAX as u128) as u32;
    // SAFETY: msg is a fully formed message of msg.len() bytes.
    let kr = unsafe {
        ffi::mach_msg(
            msg.as_mut_ptr(),
            ffi::MACH_SEND_MSG | ffi::MACH_SEND_TIMEOUT,
            msg.len() as u32,
            0,
            ffi::MACH_PORT_NULL,
            ms,
            ffi::MACH_PORT_NULL,
        )
    };
    match kr {
        ffi::KERN_SUCCESS => Ok(()),
        ffi::MACH_SEND_TIMED_OUT => {
            // The kernel returns moved rights to us on a send timeout; release them.
            // SAFETY: the message still holds the pseudo-received rights.
            unsafe { ffi::mach_msg_destroy(msg.as_mut_ptr()) };
            Err(BridgeError::TimedOut)
        }
        ffi::MACH_SEND_INVALID_DEST => Err(BridgeError::PeerGone),
        other => Err(BridgeError::Mach { op: "mach_msg(send)", msg: ffi::kr_string(other) }),
    }
}

// --- Server (main) ------------------------------------------------------------------

pub struct Rendezvous {
    name: String,
    service: ReceiveRight,
}

/// Why a check-in attempt was refused. Kept for the spike report and tests.
#[derive(Debug, Clone)]
pub struct Rejection {
    pub pid: i32,
    pub reason: String,
}

impl Rendezvous {
    /// Registers `name` in the bootstrap namespace with a fresh receive right.
    pub fn register(name: &str) -> Result<Self, BridgeError> {
        let c = CString::new(name).map_err(|_| BridgeError::Rejected("name contains NUL".into()))?;
        let mut port = 0;
        // SAFETY: valid C string and out-pointer.
        let kr = unsafe { ffi::bootstrap_check_in(ffi::bootstrap_port, c.as_ptr(), &mut port) };
        if kr != ffi::KERN_SUCCESS {
            return Err(BridgeError::Bootstrap { op: "bootstrap_check_in", name: name.into(), msg: bootstrap_err(kr) });
        }
        let service = ReceiveRight(port);
        service.set_qlimit(16)?;
        Ok(Self { name: name.to_owned(), service })
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    /// Waits for the one valid check-in. Invalid attempts are destroyed and recorded.
    /// On success the name is deregistered (the service right is destroyed).
    pub fn accept(self, expected: &ExpectedPeer, timeout: Duration) -> Result<(Connection, Vec<Rejection>), (BridgeError, Vec<Rejection>)> {
        let deadline = Instant::now() + timeout;
        let mut rejections = Vec::new();
        let mut buf = Box::new(RecvBuf([0; wire::MAX_RECEIVE + 8]));
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Err((BridgeError::TimedOut, rejections));
            }
            match receive(self.service.name(), &mut buf, left) {
                Ok(Received::TimedOut) => return Err((BridgeError::TimedOut, rejections)),
                Err(e) => return Err((e, rejections)),
                Ok(Received::Message) => {}
            }
            let (msg, token) = match wire::parse(&buf.0) {
                Ok(v) => v,
                Err(e) => {
                    destroy(&mut buf);
                    rejections.push(Rejection { pid: -1, reason: format!("malformed: {e}") });
                    continue;
                }
            };
            let token = AuditToken(token);
            let Incoming::CheckIn { engine_port, claimed_pid } = msg else {
                destroy(&mut buf);
                rejections.push(Rejection { pid: token.pid(), reason: "not a check-in".into() });
                continue;
            };
            let engine = SendRight::from_raw(engine_port);
            if claimed_pid as i32 != token.pid() {
                rejections.push(Rejection { pid: token.pid(), reason: format!("claimed pid {claimed_pid} differs from audit token") });
                continue;
            }
            if let Err(reason) = expected.verify(&token) {
                rejections.push(Rejection { pid: token.pid(), reason });
                continue;
            }
            // Accepted: deregister by destroying the service right. Anything else still
            // queued on it is destroyed with it.
            drop(self);
            let receive_port = ReceiveRight::allocate().map_err(|e| (e, rejections.clone()))?;
            receive_port.set_qlimit(256).map_err(|e| (e, rejections.clone()))?;
            send(wire::encode_checkin_reply(engine.name(), receive_port.name(), 0), Duration::from_millis(1000))
                .map_err(|e| (e, rejections.clone()))?;
            receive_port.request_no_senders(1).map_err(|e| (e, rejections.clone()))?;
            return Ok((Connection { receive: receive_port, engine, peer: token, buf }, rejections));
        }
    }
}

fn bootstrap_err(kr: ffi::kern_return_t) -> String {
    // SAFETY: bootstrap_strerror returns a static string.
    unsafe {
        let p = ffi::bootstrap_strerror(kr);
        if p.is_null() {
            format!("{kr:#x}")
        } else {
            format!("{} ({kr:#x})", std::ffi::CStr::from_ptr(p).to_string_lossy())
        }
    }
}

/// Events delivered on the anonymous port after check-in.
#[derive(Debug)]
pub enum Event {
    Surface { surface: SendRight, info: wire::SurfaceInfo },
    Frame(wire::FrameDescriptor),
    Goodbye,
    /// The engine's last send right is gone (it exited or crashed).
    PeerGone,
}

pub struct Connection {
    receive: ReceiveRight,
    engine: SendRight,
    peer: AuditToken,
    buf: Box<RecvBuf>,
}

impl Connection {
    pub fn peer(&self) -> AuditToken {
        self.peer
    }
    pub fn engine_port(&self) -> mach_port_t {
        self.engine.name()
    }

    /// Next event, `Ok(None)` on timeout. Messages whose audit token differs from the
    /// checked-in engine, or that fail to parse, are destroyed and reported as errors
    /// without ending the connection.
    pub fn recv(&mut self, timeout: Duration) -> Result<Option<Event>, BridgeError> {
        match receive(self.receive.name(), &mut self.buf, timeout)? {
            Received::TimedOut => return Ok(None),
            Received::Message => {}
        }
        if msg_id(&self.buf) == ffi::MACH_NOTIFY_NO_SENDERS {
            destroy(&mut self.buf);
            return Ok(Some(Event::PeerGone));
        }
        let (msg, token) = match wire::parse(&self.buf.0) {
            Ok(v) => v,
            Err(e) => {
                destroy(&mut self.buf);
                return Err(BridgeError::Rejected(format!("malformed: {e}")));
            }
        };
        if AuditToken(token) != self.peer {
            destroy(&mut self.buf);
            return Err(BridgeError::Rejected(format!("message from pid {} is not the engine", AuditToken(token).pid())));
        }
        Ok(Some(match msg {
            Incoming::Surface { surface_port, info } => Event::Surface { surface: SendRight::from_raw(surface_port), info },
            Incoming::Frame(d) => Event::Frame(d),
            Incoming::Goodbye => Event::Goodbye,
            other => {
                if let Some(p) = other.carried_port() {
                    drop(SendRight::from_raw(p));
                }
                return Err(BridgeError::Rejected("unexpected message kind after check-in".into()));
            }
        }))
    }
}

// --- Client (engine) ------------------------------------------------------------------

pub struct Client {
    bridge: SendRight,
    _reply: ReceiveRight,
    bridge_peer: AuditToken,
}

impl Client {
    /// Looks up the per-launch name and checks in. The reply must come from our
    /// parent (main), checked by audit token.
    pub fn check_in(name: &str, timeout: Duration) -> Result<Self, BridgeError> {
        let c = CString::new(name).map_err(|_| BridgeError::Rejected("name contains NUL".into()))?;
        let mut service = 0;
        // SAFETY: valid C string and out-pointer.
        let kr = unsafe { ffi::bootstrap_look_up(ffi::bootstrap_port, c.as_ptr(), &mut service) };
        if kr != ffi::KERN_SUCCESS {
            return Err(BridgeError::Bootstrap { op: "bootstrap_look_up", name: name.into(), msg: bootstrap_err(kr) });
        }
        let service = SendRight::from_raw(service);
        let reply = ReceiveRight::allocate()?;
        send(wire::encode_checkin(service.name(), reply.name(), std::process::id()), timeout)?;
        drop(service);

        let mut buf = Box::new(RecvBuf([0; wire::MAX_RECEIVE + 8]));
        match receive(reply.name(), &mut buf, timeout)? {
            Received::TimedOut => return Err(BridgeError::TimedOut),
            Received::Message => {}
        }
        let (msg, token) = match wire::parse(&buf.0) {
            Ok(v) => v,
            Err(e) => {
                destroy(&mut buf);
                return Err(BridgeError::Rejected(format!("malformed reply: {e}")));
            }
        };
        let token = AuditToken(token);
        let Incoming::CheckInReply { bridge_port, status } = msg else {
            destroy(&mut buf);
            return Err(BridgeError::Rejected("unexpected reply kind".into()));
        };
        let bridge = SendRight::from_raw(bridge_port);
        // SAFETY: getppid has no preconditions.
        let parent = unsafe { libc::getppid() };
        if token.pid() != parent {
            return Err(BridgeError::Rejected(format!("reply from pid {} is not our parent {parent}", token.pid())));
        }
        if status != 0 {
            return Err(BridgeError::Rejected(format!("check-in refused with status {status}")));
        }
        Ok(Self { bridge, _reply: reply, bridge_peer: token })
    }

    pub fn bridge_peer(&self) -> AuditToken {
        self.bridge_peer
    }

    /// Sends one pool surface, moving the send right made by `IOSurfaceCreateMachPort`.
    pub fn send_surface(&self, surface_port: SendRight, info: &wire::SurfaceInfo) -> Result<(), BridgeError> {
        wire::validate_surface(info).map_err(|e| BridgeError::Rejected(e.to_string()))?;
        let raw = surface_port.into_raw();
        send(wire::encode_surface(self.bridge.name(), raw, info), Duration::from_millis(1000))
    }

    /// Sends a frame descriptor. Never blocks the render thread for more than `timeout`.
    pub fn send_frame(&self, desc: &wire::FrameDescriptor, timeout: Duration) -> Result<(), BridgeError> {
        send(wire::encode_frame(self.bridge.name(), desc), timeout)
    }

    pub fn goodbye(&self) -> Result<(), BridgeError> {
        send(wire::encode_goodbye(self.bridge.name()), Duration::from_millis(200))
    }
}
