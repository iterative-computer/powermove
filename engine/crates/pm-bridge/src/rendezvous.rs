//! The Mach bootstrap rendezvous (plan section 18, item 1).
//!
//! Main registers a receive right under a per-launch random bootstrap name
//! and passes the name to the engine in its environment. The engine looks
//! the name up and sends one check-in carrying a send right to its own port.
//! Main verifies the sender's audit token (pid, pidversion, uid, parent) and
//! code signature, then destroys the service port, which removes the name
//! from the bootstrap namespace and discards any other queued check-ins, and
//! replies with a send right to a fresh anonymous port. Every later message
//! on that port must carry the same audit token.

use std::ffi::CString;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use crate::audit::{AuditToken, ExpectedPeer};
use crate::ffi::{self, mach_port_t};
use crate::wire::{self, Incoming, OutMessage, SurfaceInfo};

#[derive(Debug, thiserror::Error)]
pub enum BridgeError {
    #[error("bootstrap {op}({name}): {msg}")]
    Bootstrap {
        op: &'static str,
        name: String,
        msg: String,
    },
    #[error("mach {op}: {msg}")]
    Mach { op: &'static str, msg: String },
    #[error("timed out")]
    TimedOut,
    #[error("rejected: {0}")]
    Rejected(String),
    #[error("peer is gone")]
    PeerGone,
    /// The caller asked the wait to end (the launch ended before the engine
    /// checked in).
    #[error("stopped")]
    Stopped,
}

/// An owned receive right.
#[derive(Debug)]
pub struct ReceiveRight(mach_port_t);

impl ReceiveRight {
    pub fn allocate() -> Result<Self, BridgeError> {
        let mut name = 0;
        // SAFETY: the out-pointer is valid.
        let kr = unsafe {
            ffi::mach_port_allocate(ffi::task_self(), ffi::MACH_PORT_RIGHT_RECEIVE, &mut name)
        };
        check(kr, "mach_port_allocate")?;
        Ok(Self(name))
    }

    pub fn name(&self) -> mach_port_t {
        self.0
    }

    /// Makes a send right on this port and returns it (an extra user
    /// reference the caller owns).
    pub fn make_send(&self) -> Result<SendRight, BridgeError> {
        // SAFETY: inserting a send right on a receive right we own.
        let kr = unsafe {
            ffi::mach_port_insert_right(
                ffi::task_self(),
                self.0,
                self.0,
                ffi::MACH_MSG_TYPE_MAKE_SEND,
            )
        };
        check(kr, "mach_port_insert_right")?;
        Ok(SendRight(self.0))
    }

    fn set_qlimit(&self, limit: u32) -> Result<(), BridgeError> {
        let limits = ffi::mach_port_limits_t { mpl_qlimit: limit };
        // SAFETY: the limits struct is one natural_t, matching the count.
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

    /// Arms a no-more-senders notification delivered to this same port.
    /// `sync` is the make-send count that must have been reached (1 after the
    /// first right was made), so it does not fire before the peer holds its
    /// right.
    fn request_no_senders(&self, sync: u32) -> Result<(), BridgeError> {
        let mut previous = 0;
        // SAFETY: valid port names; `previous` is an out-pointer.
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
        unsafe {
            ffi::mach_port_mod_refs(ffi::task_self(), self.0, ffi::MACH_PORT_RIGHT_RECEIVE, -1)
        };
    }
}

/// An owned send right (one user reference).
#[derive(Debug)]
pub struct SendRight(mach_port_t);

impl SendRight {
    /// Takes ownership of a send right (received in a message, or made by
    /// `IOSurfaceCreateMachPort`).
    pub fn from_raw(name: mach_port_t) -> Self {
        Self(name)
    }

    pub fn name(&self) -> mach_port_t {
        self.0
    }

    /// Gives up ownership (when the right is moved into a message).
    pub fn into_raw(self) -> mach_port_t {
        let name = self.0;
        std::mem::forget(self);
        name
    }
}

impl Drop for SendRight {
    fn drop(&mut self) {
        // SAFETY: we own one user reference of this send right.
        unsafe { ffi::mach_port_deallocate(ffi::task_self(), self.0) };
    }
}

fn check(kr: ffi::kern_return_t, op: &'static str) -> Result<(), BridgeError> {
    if kr == ffi::KERN_SUCCESS {
        Ok(())
    } else {
        Err(BridgeError::Mach {
            op,
            msg: ffi::kr_string(kr),
        })
    }
}

/// The prefix of every bridge bootstrap name.
pub const SERVICE_PREFIX: &str = "com.powermove.engine-bridge";

/// A random, per-launch bootstrap name: the prefix and 128 random bits.
pub fn random_service_name(prefix: &str) -> Result<String, BridgeError> {
    let mut bytes = [0u8; 16];
    // SAFETY: a 16-byte buffer (getentropy allows up to 256).
    let rc = unsafe { libc::getentropy(bytes.as_mut_ptr().cast(), bytes.len()) };
    if rc != 0 {
        return Err(BridgeError::Mach {
            op: "getentropy",
            msg: std::io::Error::last_os_error().to_string(),
        });
    }
    let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    Ok(format!("{prefix}.{hex}"))
}

/// Whether `name` is a bridge name this build would have made: the prefix
/// and 32 lowercase hex digits. The engine refuses anything else from its
/// environment.
pub fn is_service_name(name: &str) -> bool {
    name.strip_prefix(SERVICE_PREFIX)
        .and_then(|rest| rest.strip_prefix('.'))
        .is_some_and(|hex| {
            hex.len() == 32
                && hex
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        })
}

/// An aligned receive buffer for any bridge message plus its trailer.
#[repr(C, align(8))]
struct RecvBuf([u8; wire::MAX_RECEIVE + 8]);

impl RecvBuf {
    fn boxed() -> Box<Self> {
        Box::new(RecvBuf([0; wire::MAX_RECEIVE + 8]))
    }
}

enum Received {
    Message,
    TimedOut,
}

fn receive(
    port: mach_port_t,
    buf: &mut RecvBuf,
    timeout: Duration,
) -> Result<Received, BridgeError> {
    let ms = timeout.as_millis().min(u32::MAX as u128) as u32;
    let options = ffi::MACH_RCV_MSG
        | ffi::MACH_RCV_TIMEOUT
        | ffi::mach_rcv_trailer_type(ffi::MACH_MSG_TRAILER_FORMAT_0)
        | ffi::mach_rcv_trailer_elements(ffi::MACH_RCV_TRAILER_AUDIT);
    loop {
        // SAFETY: the buffer is large and aligned enough for a header; the
        // kernel writes at most rcv_size bytes. Without MACH_RCV_LARGE, an
        // oversized message is destroyed by the kernel and MACH_RCV_TOO_LARGE
        // is returned.
        let kr = unsafe {
            ffi::mach_msg(
                buf.0.as_mut_ptr().cast(),
                options,
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
            ffi::MACH_RCV_INTERRUPTED | ffi::MACH_RCV_TOO_LARGE => continue,
            other => {
                return Err(BridgeError::Mach {
                    op: "mach_msg(receive)",
                    msg: ffi::kr_string(other),
                });
            }
        }
    }
}

/// Releases the rights a received message carries that we are not keeping.
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
            // The kernel returns moved rights to us on a send timeout.
            // SAFETY: the message still holds the pseudo-received rights.
            unsafe { ffi::mach_msg_destroy(msg.as_mut_ptr()) };
            Err(BridgeError::TimedOut)
        }
        ffi::MACH_SEND_INVALID_DEST => {
            // SAFETY: as above; a moved right would otherwise leak.
            unsafe { ffi::mach_msg_destroy(msg.as_mut_ptr()) };
            Err(BridgeError::PeerGone)
        }
        other => Err(BridgeError::Mach {
            op: "mach_msg(send)",
            msg: ffi::kr_string(other),
        }),
    }
}

// --- Server (main) ----------------------------------------------------------

/// The registered bootstrap name, waiting for the engine's one check-in.
pub struct Rendezvous {
    name: String,
    service: ReceiveRight,
}

/// A refused check-in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rejection {
    pub pid: i32,
    pub reason: String,
}

/// The longest single wait inside [`Rendezvous::accept_until`]: how quickly
/// a stop request is seen while no engine has checked in yet.
pub const STOP_POLL: Duration = Duration::from_millis(50);

/// Queued check-ins main keeps on the service port.
const SERVICE_QUEUE: u32 = 16;
/// Queued control messages on the connection (a pool is at most 32 surfaces,
/// and a resize retires one pool and announces another).
const CONNECTION_QUEUE: u32 = 256;

impl Rendezvous {
    /// Registers `name` in the bootstrap namespace with a fresh receive right.
    pub fn register(name: &str) -> Result<Self, BridgeError> {
        let c =
            CString::new(name).map_err(|_| BridgeError::Rejected("name contains NUL".into()))?;
        let mut port = 0;
        // SAFETY: a valid C string and out-pointer.
        let kr = unsafe { ffi::bootstrap_check_in(ffi::bootstrap_port, c.as_ptr(), &mut port) };
        if kr != ffi::KERN_SUCCESS {
            return Err(BridgeError::Bootstrap {
                op: "bootstrap_check_in",
                name: name.into(),
                msg: bootstrap_err(kr),
            });
        }
        let service = ReceiveRight(port);
        service.set_qlimit(SERVICE_QUEUE)?;
        Ok(Self {
            name: name.to_owned(),
            service,
        })
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    /// Waits for the one valid check-in. Invalid attempts are destroyed and
    /// recorded. On success the name is deregistered (the service right is
    /// destroyed with everything still queued on it).
    #[allow(clippy::result_large_err)]
    pub fn accept(
        self,
        expected: &ExpectedPeer,
        timeout: Duration,
    ) -> Result<(Connection, Vec<Rejection>), (BridgeError, Vec<Rejection>)> {
        self.accept_until(expected, timeout, &AtomicBool::new(false))
    }

    /// [`Rendezvous::accept`] that also ends with [`BridgeError::Stopped`]
    /// within [`STOP_POLL`] of `stop` being set. The engine may exit (or the
    /// app quit) before it checks in; whoever owns the launch must not wait
    /// out the whole check-in timeout to stop it.
    #[allow(clippy::result_large_err)]
    pub fn accept_until(
        self,
        expected: &ExpectedPeer,
        timeout: Duration,
        stop: &AtomicBool,
    ) -> Result<(Connection, Vec<Rejection>), (BridgeError, Vec<Rejection>)> {
        let deadline = Instant::now() + timeout;
        let mut rejections = Vec::new();
        let mut buf = RecvBuf::boxed();
        loop {
            if stop.load(Ordering::SeqCst) {
                return Err((BridgeError::Stopped, rejections));
            }
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Err((BridgeError::TimedOut, rejections));
            }
            match receive(self.service.name(), &mut buf, left.min(STOP_POLL)) {
                Ok(Received::TimedOut) => continue,
                Err(e) => return Err((e, rejections)),
                Ok(Received::Message) => {}
            }
            let (msg, token) = match wire::parse(&buf.0) {
                Ok(v) => v,
                Err(e) => {
                    destroy(&mut buf);
                    rejections.push(Rejection {
                        pid: -1,
                        reason: format!("malformed: {e}"),
                    });
                    continue;
                }
            };
            let token = AuditToken(token);
            let Incoming::CheckIn {
                engine_port,
                claimed_pid,
            } = msg
            else {
                destroy(&mut buf);
                rejections.push(Rejection {
                    pid: token.pid(),
                    reason: "not a check-in".into(),
                });
                continue;
            };
            let engine = SendRight::from_raw(engine_port);
            if claimed_pid as i32 != token.pid() {
                rejections.push(Rejection {
                    pid: token.pid(),
                    reason: format!("claimed pid {claimed_pid} differs from the audit token"),
                });
                continue;
            }
            if let Err(reason) = expected.verify(&token) {
                rejections.push(Rejection {
                    pid: token.pid(),
                    reason,
                });
                continue;
            }
            // Accepted: deregister by destroying the service right.
            drop(self);
            let fail = |e: BridgeError, r: &Vec<Rejection>| (e, r.clone());
            let receive_port = ReceiveRight::allocate().map_err(|e| fail(e, &rejections))?;
            receive_port
                .set_qlimit(CONNECTION_QUEUE)
                .map_err(|e| fail(e, &rejections))?;
            send(
                wire::encode_checkin_reply(engine.name(), receive_port.name(), 0),
                Duration::from_millis(1000),
            )
            .map_err(|e| fail(e, &rejections))?;
            receive_port
                .request_no_senders(1)
                .map_err(|e| fail(e, &rejections))?;
            return Ok((
                Connection {
                    receive: receive_port,
                    _engine: engine,
                    peer: token,
                    buf,
                },
                rejections,
            ));
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
            format!(
                "{} ({kr:#x})",
                std::ffi::CStr::from_ptr(p).to_string_lossy()
            )
        }
    }
}

/// What arrives on main's anonymous port after the check-in.
#[derive(Debug)]
pub enum Event {
    Surface {
        surface: SendRight,
        info: SurfaceInfo,
    },
    Retire {
        client_port: u32,
        generation: u32,
    },
    Goodbye,
    /// The engine's last send right is gone (it exited or crashed).
    PeerGone,
}

/// Main's side of an accepted engine.
pub struct Connection {
    receive: ReceiveRight,
    _engine: SendRight,
    peer: AuditToken,
    buf: Box<RecvBuf>,
}

impl Connection {
    pub fn peer(&self) -> AuditToken {
        self.peer
    }

    /// The next event, `Ok(None)` on timeout. A message whose audit token is
    /// not the checked-in engine's, or that does not parse, is destroyed and
    /// reported as an error without ending the connection.
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
            return Err(BridgeError::Rejected(format!(
                "message from pid {} is not the engine",
                AuditToken(token).pid()
            )));
        }
        Ok(Some(match msg {
            Incoming::Surface { surface_port, info } => Event::Surface {
                surface: SendRight::from_raw(surface_port),
                info,
            },
            Incoming::Retire {
                client_port,
                generation,
            } => Event::Retire {
                client_port,
                generation,
            },
            Incoming::Goodbye => Event::Goodbye,
            other => {
                if let Some(port) = other.carried_port() {
                    drop(SendRight::from_raw(port));
                }
                return Err(BridgeError::Rejected(
                    "unexpected message kind after the check-in".into(),
                ));
            }
        }))
    }
}

// --- Client (engine) --------------------------------------------------------

/// The engine's side: checked in, holding a send right to main's port.
pub struct Client {
    bridge: SendRight,
    _reply: ReceiveRight,
    bridge_peer: AuditToken,
}

impl Client {
    /// Looks up the per-launch name and checks in. The reply must come from
    /// our parent (main), checked by audit token.
    pub fn check_in(name: &str, timeout: Duration) -> Result<Self, BridgeError> {
        if !is_service_name(name) {
            return Err(BridgeError::Rejected(format!(
                "{name:?} is not a bridge service name"
            )));
        }
        let c =
            CString::new(name).map_err(|_| BridgeError::Rejected("name contains NUL".into()))?;
        let mut service = 0;
        // SAFETY: a valid C string and out-pointer.
        let kr = unsafe { ffi::bootstrap_look_up(ffi::bootstrap_port, c.as_ptr(), &mut service) };
        if kr != ffi::KERN_SUCCESS {
            return Err(BridgeError::Bootstrap {
                op: "bootstrap_look_up",
                name: name.into(),
                msg: bootstrap_err(kr),
            });
        }
        let service = SendRight::from_raw(service);
        let reply = ReceiveRight::allocate()?;
        send(
            wire::encode_checkin(service.name(), reply.name(), std::process::id()),
            timeout,
        )?;
        drop(service);

        let mut buf = RecvBuf::boxed();
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
        let Incoming::CheckInReply {
            bridge_port,
            status,
        } = msg
        else {
            destroy(&mut buf);
            return Err(BridgeError::Rejected("unexpected reply kind".into()));
        };
        let bridge = SendRight::from_raw(bridge_port);
        // SAFETY: getppid has no preconditions.
        let parent = unsafe { libc::getppid() };
        if token.pid() != parent {
            return Err(BridgeError::Rejected(format!(
                "reply from pid {} is not our parent {parent}",
                token.pid()
            )));
        }
        if status != 0 {
            return Err(BridgeError::Rejected(format!(
                "check-in refused with status {status}"
            )));
        }
        Ok(Self {
            bridge,
            _reply: reply,
            bridge_peer: token,
        })
    }

    pub fn bridge_peer(&self) -> AuditToken {
        self.bridge_peer
    }

    /// Sends one pool surface, moving the send right made by
    /// `IOSurfaceCreateMachPort`.
    pub fn send_surface(&self, surface: SendRight, info: &SurfaceInfo) -> Result<(), BridgeError> {
        wire::validate_surface(info).map_err(|e| BridgeError::Rejected(e.to_string()))?;
        send(
            wire::encode_surface(self.bridge.name(), surface.into_raw(), info),
            Duration::from_millis(1000),
        )
    }

    /// Tells main a pool generation is no longer written.
    pub fn retire(&self, client_port: u32, generation: u32) -> Result<(), BridgeError> {
        send(
            wire::encode_retire(self.bridge.name(), client_port, generation),
            Duration::from_millis(1000),
        )
    }

    pub fn goodbye(&self) -> Result<(), BridgeError> {
        send(
            wire::encode_goodbye(self.bridge.name()),
            Duration::from_millis(200),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn service_names_are_random_and_recognized() {
        let a = random_service_name(SERVICE_PREFIX).unwrap();
        let b = random_service_name(SERVICE_PREFIX).unwrap();
        assert_ne!(a, b);
        assert!(is_service_name(&a));
        assert!(!is_service_name("com.powermove.engine-bridge.XYZ"));
        assert!(!is_service_name(&a.replace("engine-bridge", "other")));
        assert!(!is_service_name(&format!("{a}0")));
        assert!(!is_service_name(&a.to_uppercase()));
    }
}
