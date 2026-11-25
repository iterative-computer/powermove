//! Aero's agent core: the vendored libfx core (`apps/native/vendor/fx`) behind
//! its C ABI (`vendor/fx/include/fx_core.h`, ABI 2), and a safe wrapper.
//!
//! One [`Runtime`] is one agent session on its own core thread. The thread
//! that owns the `Runtime` (the `pm-aero` worker) drains it through two
//! fields that borrow independently: [`Runtime::events`] (typed events) and
//! [`Runtime::fetches`] (model HTTP requests). Calls the header allows from
//! any thread (cancel, the fetch stream, journal acks) go through a
//! [`Remote`], which is `Copy` and safe after the runtime is gone (its handle
//! is generation-tagged, so it then returns [`Error::Closed`]).
//!
//! Tool calls arrive as [`ToolLease`]s: the core's own input, borrowed
//! without a copy, valid on any thread until the lease is answered. A lease
//! dropped unanswered (a panicking tool included) answers the core with an
//! error, so the core never waits for ever. Teardown waits for every lease,
//! then destroys the runtime; dropping a `Runtime` hands that to its own
//! thread, so the dropping thread (the UI thread included) never blocks.
//!
//! No call may be made on the main thread: every one asserts it in debug
//! builds. Built without the pinned Zig, every call returns
//! [`Error::Unavailable`] (see `build.rs`).

pub mod sys;

use std::ffi::c_void;
use std::fmt;
use std::marker::PhantomData;
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

/// An error from the core. Mirrors the header's negative statuses, plus the
/// outcomes a call that must apply cannot accept.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Invalid,
    Closed,
    Failed,
    Limit,
    OutOfMemory,
    Thread,
    Io,
    State,
    Unavailable,
    Unsupported,
    TooLarge,
    /// A bounded queue is full: retry after the next wake.
    Backpressure,
    /// A lock stayed contended past the bound: retry.
    Busy,
    /// The id or turn is no longer current.
    Stale,
    Unknown(i32),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let text = match self {
            Error::Invalid => "invalid argument",
            Error::Closed => "the runtime is closed",
            Error::Failed => "the runtime is not running",
            Error::Limit => "too many runtimes",
            Error::OutOfMemory => "the core is out of memory",
            Error::Thread => "the core could not start a thread",
            Error::Io => "the core could not make its readiness socket",
            Error::State => "not valid in the runtime's current state",
            Error::Unavailable => "this build has no libfx core",
            Error::Unsupported => "not supported by this core yet",
            Error::TooLarge => "the input can never fit",
            Error::Backpressure => "the core's queue is full",
            Error::Busy => "the core held a lock past the bound",
            Error::Stale => "no longer current",
            Error::Unknown(code) => return write!(f, "unknown core status {code}"),
        };
        f.write_str(text)
    }
}

impl std::error::Error for Error {}

/// A non-error outcome of a call that may find nothing to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    Applied,
    Empty,
    Stale,
    Backpressure,
    Busy,
}

fn check(status: sys::fx_status) -> Result<Outcome, Error> {
    Ok(match status {
        sys::FX_OK => Outcome::Applied,
        sys::FX_EMPTY => Outcome::Empty,
        sys::FX_STALE => Outcome::Stale,
        sys::FX_BACKPRESSURE => Outcome::Backpressure,
        sys::FX_BUSY => Outcome::Busy,
        sys::FX_E_INVALID => return Err(Error::Invalid),
        sys::FX_E_CLOSED => return Err(Error::Closed),
        sys::FX_E_FAILED => return Err(Error::Failed),
        sys::FX_E_LIMIT => return Err(Error::Limit),
        sys::FX_E_OOM => return Err(Error::OutOfMemory),
        sys::FX_E_THREAD => return Err(Error::Thread),
        sys::FX_E_IO => return Err(Error::Io),
        sys::FX_E_STATE => return Err(Error::State),
        sys::FX_E_UNAVAILABLE => return Err(Error::Unavailable),
        sys::FX_E_UNSUPPORTED => return Err(Error::Unsupported),
        sys::FX_E_TOO_LARGE => return Err(Error::TooLarge),
        other => return Err(Error::Unknown(other)),
    })
}

/// For calls whose success means "applied": every other outcome is an error,
/// so a refused call never looks accepted (Sol's item 6).
fn applied(status: sys::fx_status) -> Result<(), Error> {
    match check(status)? {
        Outcome::Applied => Ok(()),
        Outcome::Stale => Err(Error::Stale),
        Outcome::Backpressure => Err(Error::Backpressure),
        Outcome::Busy => Err(Error::Busy),
        Outcome::Empty => Err(Error::Unknown(sys::FX_EMPTY)),
    }
}

/// The main thread never calls the core (design 2.4).
#[inline]
fn assert_off_main() {
    debug_assert!(
        !on_main_thread(),
        "the libfx core is never called on the main thread"
    );
}

#[cfg(target_vendor = "apple")]
fn on_main_thread() -> bool {
    // SAFETY: pthread_main_np has no preconditions.
    unsafe { libc::pthread_main_np() != 0 }
}

#[cfg(target_os = "linux")]
fn on_main_thread() -> bool {
    // SAFETY: neither call has preconditions.
    unsafe { libc::gettid() == libc::getpid() }
}

#[cfg(not(any(target_vendor = "apple", target_os = "linux")))]
fn on_main_thread() -> bool {
    false
}

/// Whether this build links the core (built with the pinned Zig).
pub fn available() -> bool {
    // SAFETY: no arguments, no state; any thread may call it.
    unsafe { sys::fx_core_abi_version() == sys::FX_CORE_ABI_VERSION }
}

/// `value` as a string borrowed for as long as `owner` is: the owner is
/// whatever the header says keeps the core's pointer valid.
fn str_in<T: ?Sized>(owner: &T, value: sys::fx_str) -> &str {
    std::str::from_utf8(bytes_in(
        owner,
        sys::fx_bytes {
            ptr: value.ptr,
            len: value.len,
        },
    ))
    .unwrap_or("")
}

fn bytes_in<T: ?Sized>(_owner: &T, value: sys::fx_bytes) -> &[u8] {
    if value.len == 0 || value.ptr.is_null() {
        return &[];
    }
    // SAFETY: the core hands out valid pointers with their lengths, valid
    // while `_owner` is borrowed (each caller ties `_owner` to the header's
    // window for the pointer).
    unsafe { std::slice::from_raw_parts(value.ptr, value.len) }
}

/// Retries a call the core answered FX_BUSY (a lock held past the bound,
/// which only a bug in the core causes) a few times before reporting it.
fn retry_busy(mut call: impl FnMut() -> sys::fx_status) -> sys::fx_status {
    for _ in 0..8 {
        let status = call();
        if status != sys::FX_BUSY {
            return status;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    call()
}

/// A host tool the model may call.
#[derive(Clone, Copy, Debug)]
pub struct Tool<'a> {
    pub name: &'a str,
    pub description: &'a str,
    pub input_schema_json: &'a str,
    /// May run beside other idempotent calls.
    pub idempotent: bool,
    /// Changes state: runs alone and is never re-run after a crash.
    pub writes: bool,
}

/// What a runtime is made with. Empty strings and zero limits pick the
/// core's defaults (see the header).
#[derive(Clone, Copy, Debug, Default)]
pub struct Config<'a> {
    pub provider: &'a str,
    pub model: &'a str,
    pub effort: &'a str,
    pub configured_providers_json: &'a str,
    pub endpoint_url: &'a str,
    pub instructions: &'a str,
    pub home: &'a str,
    pub workspace_root: &'a str,
    pub tools: &'a [Tool<'a>],
    pub max_agent_steps: u32,
    pub max_concurrent_tools: u32,
    pub max_tool_result_bytes: u32,
    pub max_history_turns: u32,
    pub max_stop_continuations: u32,
}

/// What the host of one runtime does with what the core asks. Every method
/// runs on a core-owned thread and must only record or enqueue (header,
/// fx_host): never block, never call the runtime.
pub trait Host: Send + Sync + 'static {
    /// The model called a host tool. Answer the lease, from any thread, when
    /// the tool is done; dropping it unanswered answers with an error.
    fn tool_call(&self, call: ToolLease) {
        drop(call);
    }
    /// `call`'s turn was cancelled or steered past it. Its lease still needs
    /// an answer (the core drops it).
    fn tool_cancel(&self, _call: u64, _turn: u64) {}
    /// Whether the session is journaled: the core then sends appends.
    fn journaled(&self) -> bool {
        false
    }
    /// A journal append, borrowed for the call: copy what you keep, store it
    /// in order, and acknowledge with [`Remote::journal_ack`] once durable.
    fn journal_append(&self, _append: JournalAppend<'_>) {}
}

/// What the runtime's leases and teardown share.
struct Shared {
    /// Set once, right after create; no callback runs before that.
    handle: std::sync::atomic::AtomicU64,
    leases: Mutex<Leases>,
    changed: Condvar,
}

#[derive(Default)]
struct Leases {
    open: usize,
    /// Teardown has started: new tool calls are answered at once.
    closing: bool,
}

impl Shared {
    fn handle(&self) -> u64 {
        self.handle.load(std::sync::atomic::Ordering::Acquire)
    }

    fn lease_opened(&self) -> bool {
        let mut leases = self.leases.lock().unwrap_or_else(|e| e.into_inner());
        if leases.closing {
            return false;
        }
        leases.open += 1;
        true
    }

    fn lease_closed(&self) {
        let mut leases = self.leases.lock().unwrap_or_else(|e| e.into_inner());
        leases.open -= 1;
        self.changed.notify_all();
    }

    fn wait_leases(&self) {
        let mut leases = self.leases.lock().unwrap_or_else(|e| e.into_inner());
        leases.closing = true;
        while leases.open > 0 {
            leases = self.changed.wait(leases).unwrap_or_else(|e| e.into_inner());
        }
    }
}

struct HostBox {
    host: Arc<dyn Host>,
    shared: Arc<Shared>,
}

/// A tool call the core waits on. Its strings are the core's own (the
/// model's input without a copy), valid until the lease is answered, on any
/// thread (header rule 9).
pub struct ToolLease {
    shared: Arc<Shared>,
    call: u64,
    turn: u64,
    flags: u32,
    name: sys::fx_str,
    input_json: sys::fx_str,
    model_call_id: sys::fx_str,
    answered: bool,
}

// SAFETY: the core keeps the lease's strings valid, for reads from any
// thread, until it is answered; answering takes the lease by value.
unsafe impl Send for ToolLease {}

impl fmt::Debug for ToolLease {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ToolLease")
            .field("call", &self.call)
            .field("turn", &self.turn)
            .field("name", &self.name())
            .finish()
    }
}

/// How the core took an answer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Answered {
    /// Applied (for a cancelled call too: the core drops the result).
    Applied,
    /// The result exceeded max_tool_result_bytes: the model reads that it did.
    TooLarge,
    /// The call was already answered.
    Stale,
}

impl ToolLease {
    pub fn call(&self) -> u64 {
        self.call
    }
    pub fn turn(&self) -> u64 {
        self.turn
    }
    pub fn name(&self) -> &str {
        str_in(self, self.name)
    }
    /// The model's input JSON, as the core holds it (no copy).
    pub fn input_json(&self) -> &str {
        str_in(self, self.input_json)
    }
    pub fn model_call_id(&self) -> &str {
        str_in(self, self.model_call_id)
    }
    pub fn idempotent(&self) -> bool {
        self.flags & sys::FX_TOOL_IDEMPOTENT != 0
    }
    pub fn writes(&self) -> bool {
        self.flags & sys::FX_TOOL_WRITES != 0
    }
    pub fn remote(&self) -> Remote {
        Remote {
            handle: self.shared.handle(),
        }
    }

    /// The input's address, for tests that check it is the core's own.
    #[doc(hidden)]
    pub fn input_ptr(&self) -> *const u8 {
        self.input_json.ptr
    }

    /// Answers the call (text copied once) and ends the lease.
    pub fn answer(mut self, text: &str, is_error: bool) -> Result<Answered, Error> {
        self.answered = true;
        let result = self.send(text, is_error);
        self.shared.lease_closed();
        result
    }

    fn send(&self, text: &str, is_error: bool) -> Result<Answered, Error> {
        assert_off_main();
        let out = sys::fx_tool_output {
            text: sys::fx_str::of(text),
            images: std::ptr::null(),
            image_count: 0,
            is_error,
        };
        // SAFETY: `out` and `text` are valid for the call; the core copies.
        let status =
            retry_busy(|| unsafe { sys::fx_tool_result(self.shared.handle(), self.call, &out) });
        match status {
            sys::FX_OK => Ok(Answered::Applied),
            sys::FX_E_TOO_LARGE => Ok(Answered::TooLarge),
            sys::FX_STALE => Ok(Answered::Stale),
            other => Err(check(other).err().unwrap_or(Error::Unknown(other))),
        }
    }
}

impl Drop for ToolLease {
    fn drop(&mut self) {
        if self.answered {
            return;
        }
        // Never leave the core waiting: a tool that panicked or forgot its
        // lease answers with an error the model can read.
        let text = if std::thread::panicking() {
            "The host tool failed (it panicked)."
        } else {
            "The host dropped this tool call without answering."
        };
        let _ = self.send(text, true);
        self.shared.lease_closed();
    }
}

unsafe extern "C" fn host_tool_call(ctx: *mut c_void, info: *const sys::fx_tool_call) {
    // SAFETY: ctx is the HostBox the runtime was made with; it lives until
    // fx_runtime_destroy has returned, after which no callback runs. info is
    // valid for the callback and its strings for the lease.
    let (host, info) = unsafe { (&*(ctx as *const HostBox), &*info) };
    let mut lease = ToolLease {
        shared: host.shared.clone(),
        call: info.call,
        turn: info.turn,
        flags: info.flags,
        name: info.name,
        input_json: info.input_json,
        model_call_id: info.model_call_id,
        answered: false,
    };
    if !host.shared.lease_opened() {
        // Teardown has started: answer at once, without the host.
        lease.answered = true;
        let _ = lease.send("The session is closing.", true);
        return;
    }
    // A panic must not unwind into the core; the lease, dropped while
    // unwinding, answers the call.
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| host.host.tool_call(lease)));
}

unsafe extern "C" fn host_tool_cancel(ctx: *mut c_void, call: u64, turn: u64) {
    // SAFETY: as host_tool_call.
    let host = unsafe { &*(ctx as *const HostBox) };
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        host.host.tool_cancel(call, turn)
    }));
}

/// A journal append, borrowed for the [`Host::journal_append`] call.
#[derive(Clone, Copy, Debug)]
pub struct JournalAppend<'a> {
    pub append: u64,
    pub turn: u64,
    pub first_seq: u64,
    pub last_seq: u64,
    pub barrier: bool,
    pub events_json: &'a str,
}

unsafe extern "C" fn host_journal_append(ctx: *mut c_void, info: *const sys::fx_journal_append) {
    // SAFETY: as host_tool_call; info is valid for the callback.
    let (host, info) = unsafe { (&*(ctx as *const HostBox), &*info) };
    let append = JournalAppend {
        append: info.append,
        turn: info.turn,
        first_seq: info.first_seq,
        last_seq: info.last_seq,
        barrier: info.flags & sys::FX_APPEND_BARRIER != 0,
        events_json: str_in(info, info.events_json),
    };
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        host.host.journal_append(append)
    }));
}

/// Why a turn ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StopReason {
    EndTurn,
    MaxTokens,
    MaxSteps,
    Refusal,
    Cancelled,
    Yielded,
    Error,
    Unknown(i32),
}

impl StopReason {
    fn of(code: i32) -> StopReason {
        match code {
            sys::FX_STOP_END_TURN => StopReason::EndTurn,
            sys::FX_STOP_MAX_TOKENS => StopReason::MaxTokens,
            sys::FX_STOP_MAX_STEPS => StopReason::MaxSteps,
            sys::FX_STOP_REFUSAL => StopReason::Refusal,
            sys::FX_STOP_CANCELLED => StopReason::Cancelled,
            sys::FX_STOP_YIELDED => StopReason::Yielded,
            sys::FX_STOP_ERROR => StopReason::Error,
            other => StopReason::Unknown(other),
        }
    }
}

pub use sys::fx_usage as Usage;

/// Runtime state, from [`Runtime::info`] and [`Event::Exited`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Running,
    /// The core thread returned on its own, with its exit code.
    Exited(i32),
    /// The C entry stopped the core (initialize failed, or an event could not
    /// be delivered).
    Failed,
    /// A core thread panicked and is parked; the runtime is abandoned.
    Panicked,
}

impl State {
    fn of(state: i32, exit_code: i32) -> State {
        match state {
            sys::FX_RUNTIME_EXITED => State::Exited(exit_code),
            sys::FX_RUNTIME_FAILED => State::Failed,
            sys::FX_RUNTIME_PANICKED => State::Panicked,
            _ => State::Running,
        }
    }
}

/// A typed event, borrowed from the runtime's [`Events`] until its next drain.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Event<'a> {
    SessionOpened {
        request: u64,
        session_id: &'a str,
        journaled: bool,
        resumable: bool,
    },
    ImagesNeeded {
        request: u64,
        count: usize,
    },
    RequestDone {
        request: u64,
        outcome: Result<Outcome, Error>,
        message: &'a str,
    },
    TurnStart {
        turn: u64,
        message_id: &'a str,
        repeated: bool,
        resumed: bool,
    },
    TextDelta {
        turn: u64,
        text: &'a str,
    },
    ThoughtDelta {
        turn: u64,
        text: &'a str,
    },
    SteerJoined {
        turn: u64,
        request: u64,
        text: &'a str,
        message_id: &'a str,
    },
    /// A steer the turn did not take: send it as a prompt.
    SteerReturned {
        turn: u64,
        request: u64,
        message_id: &'a str,
    },
    ToolStart {
        turn: u64,
        name: &'a str,
        model_call_id: &'a str,
    },
    ToolEnd {
        turn: u64,
        name: &'a str,
        model_call_id: &'a str,
        failed: bool,
    },
    Usage {
        turn: u64,
        usage: Usage,
    },
    Compacted {
        turn: u64,
    },
    TurnEnd {
        turn: u64,
        reason: StopReason,
        usage: Usage,
    },
    Checkpoint {
        request: u64,
        at_seq: u64,
        bytes: &'a [u8],
    },
    Recovery {
        request: u64,
        kind: i32,
        text: &'a str,
        text2: &'a str,
        bytes: &'a [u8],
    },
    Error {
        turn: u64,
        error: Error,
        message: &'a str,
    },
    /// The last event: the runtime stopped.
    Exited {
        state: State,
        message: &'a str,
        events_lost: bool,
    },
    Unknown {
        kind: u32,
    },
}

impl<'a> Event<'a> {
    fn of<T: ?Sized>(owner: &'a T, event: &sys::fx_event) -> Event<'a> {
        let text = str_in(owner, event.text);
        let text2 = str_in(owner, event.text2);
        match event.kind {
            sys::FX_EVENT_SESSION_OPENED => Event::SessionOpened {
                request: event.request,
                session_id: text,
                journaled: event.flags & sys::FX_OPENED_JOURNALED != 0,
                resumable: event.flags & sys::FX_OPENED_RESUMABLE != 0,
            },
            sys::FX_EVENT_IMAGES_NEEDED => Event::ImagesNeeded {
                request: event.request,
                count: event.name_count,
            },
            sys::FX_EVENT_REQUEST_DONE => Event::RequestDone {
                request: event.request,
                outcome: check(event.status),
                message: text,
            },
            sys::FX_EVENT_TURN_START => Event::TurnStart {
                turn: event.turn,
                message_id: text,
                repeated: event.flags & sys::FX_TURN_REPEATED != 0,
                resumed: event.flags & sys::FX_TURN_RESUMED != 0,
            },
            sys::FX_EVENT_TEXT_DELTA => Event::TextDelta {
                turn: event.turn,
                text,
            },
            sys::FX_EVENT_THOUGHT_DELTA => Event::ThoughtDelta {
                turn: event.turn,
                text,
            },
            sys::FX_EVENT_STEER_JOINED => Event::SteerJoined {
                turn: event.turn,
                request: event.request,
                text,
                message_id: text2,
            },
            sys::FX_EVENT_STEER_RETURNED => Event::SteerReturned {
                turn: event.turn,
                request: event.request,
                message_id: text2,
            },
            sys::FX_EVENT_TOOL_START => Event::ToolStart {
                turn: event.turn,
                name: text,
                model_call_id: text2,
            },
            sys::FX_EVENT_TOOL_END => Event::ToolEnd {
                turn: event.turn,
                name: text,
                model_call_id: text2,
                failed: event.status != sys::FX_OK,
            },
            sys::FX_EVENT_USAGE => Event::Usage {
                turn: event.turn,
                usage: event.usage,
            },
            sys::FX_EVENT_COMPACTED => Event::Compacted { turn: event.turn },
            sys::FX_EVENT_TURN_END => Event::TurnEnd {
                turn: event.turn,
                reason: StopReason::of(event.code),
                usage: event.usage,
            },
            sys::FX_EVENT_CHECKPOINT => Event::Checkpoint {
                request: event.request,
                at_seq: event.id,
                bytes: bytes_in(owner, event.bytes),
            },
            sys::FX_EVENT_RECOVERY => Event::Recovery {
                request: event.request,
                kind: event.code,
                text,
                text2,
                bytes: bytes_in(owner, event.bytes),
            },
            sys::FX_EVENT_ERROR => Event::Error {
                turn: event.turn,
                error: check(event.status).err().unwrap_or(Error::Failed),
                message: text,
            },
            sys::FX_EVENT_EXITED => Event::Exited {
                state: State::of(event.code, event.status),
                message: text,
                events_lost: event.flags & sys::FX_EXITED_EVENTS_LOST != 0,
            },
            kind => Event::Unknown { kind },
        }
    }
}

/// Who a fetch is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Purpose {
    Model,
    Catalog,
}

/// A credential the host's fetch edge may add: only to requests for
/// `provider` whose destination origin is `origin` (header rule 13).
#[derive(Clone, Copy, Debug)]
pub struct Credential<'a> {
    pub provider: &'a str,
    /// scheme://host[:port], as [`origin_of`] writes it.
    pub origin: &'a str,
    pub header: &'a str,
    pub value: &'a str,
}

/// A model HTTP request, borrowed until the next [`Fetches::take`].
#[derive(Clone, Copy, Debug)]
pub struct FetchRequest<'a> {
    pub fetch: u64,
    pub turn: u64,
    pub purpose: Purpose,
    pub provider: &'a str,
    pub method: &'a str,
    pub url: &'a str,
    headers: &'a [sys::fx_header],
    pub body: &'a [u8],
}

impl<'a> FetchRequest<'a> {
    /// The request's headers as the core built them, credentials stripped.
    pub fn headers(&self) -> impl Iterator<Item = (&'a str, &'a str)> + 'a {
        let headers = self.headers;
        headers
            .iter()
            .map(move |header| (str_in(headers, header.name), str_in(headers, header.value)))
    }

    /// The headers to send: the core's, plus the first credential whose
    /// provider and origin match this request's. Credentials never reach any
    /// other destination (a loopback fake, an endpoint override).
    pub fn outgoing_headers(&self, credentials: &[Credential<'_>]) -> Vec<(String, String)> {
        let mut headers: Vec<(String, String)> = self
            .headers()
            .map(|(name, value)| (name.to_string(), value.to_string()))
            .collect();
        if let Some(origin) = origin_of(self.url)
            && let Some(credential) = credentials
                .iter()
                .find(|c| c.provider == self.provider && c.origin == origin)
        {
            headers.retain(|(name, _)| !name.eq_ignore_ascii_case(credential.header));
            headers.push((credential.header.to_string(), credential.value.to_string()));
        }
        headers
    }
}

/// `url`'s origin, scheme://host[:port], lowercased, without the scheme's
/// default port; None if it is not an http(s) URL.
pub fn origin_of(url: &str) -> Option<String> {
    let (scheme, rest) = url.split_once("://")?;
    let scheme = scheme.to_ascii_lowercase();
    let default_port = match scheme.as_str() {
        "https" => "443",
        "http" => "80",
        _ => return None,
    };
    let authority = rest.split(['/', '?', '#']).next()?;
    let authority = authority.rsplit('@').next()?.to_ascii_lowercase();
    if authority.is_empty() {
        return None;
    }
    // [v6]:port or host:port; the port is dropped when it is the default.
    let (host, port) = if let Some(end) = authority.find(']') {
        let port = authority[end + 1..].strip_prefix(':');
        (&authority[..=end], port)
    } else {
        match authority.rsplit_once(':') {
            Some((host, port)) => (host, Some(port)),
            None => (authority.as_str(), None),
        }
    };
    let host = match port {
        Some(port) if !port.is_empty() && port != default_port => format!("{host}:{port}"),
        _ => host.to_string(),
    };
    Some(format!("{scheme}://{host}"))
}

/// Where the core stands on a fetch.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Disposition {
    /// The core gave it up: abort the HTTP request.
    Retired,
    /// The core still reads it.
    Active,
    /// The core has all it needs: drain the rest quietly.
    Consumed,
}

/// A steer's fate at the call.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Steered {
    /// Accepted for the turn; [`Event::RequestDone`] with this id follows.
    Accepted(u64),
    /// The turn is not running: send the input as a prompt instead.
    Stale,
}

const EVENT_BATCH: usize = 256;

/// The runtime's event stream. Events borrow it until the next drain.
pub struct Events {
    handle: u64,
    buf: Vec<sys::fx_event>,
}

impl Events {
    /// Drains the queued events, oldest first. They borrow `self` until the
    /// next drain; draining allocates nothing.
    pub fn drain(&mut self) -> Result<impl Iterator<Item = Event<'_>> + '_, Error> {
        assert_off_main();
        self.buf.clear();
        let mut count = 0usize;
        let handle = self.handle;
        let buf = self.buf.as_mut_ptr();
        let cap = self.buf.capacity();
        // SAFETY: the buffer holds `cap` events of the size we pass; the core
        // writes at most `cap` of them and sets `count`.
        applied(retry_busy(|| unsafe {
            sys::fx_events_drain(
                handle,
                buf,
                cap,
                std::mem::size_of::<sys::fx_event>(),
                &mut count,
            )
        }))?;
        // SAFETY: the core initialised `count` events.
        unsafe { self.buf.set_len(count) };
        let this: &Self = self;
        Ok(this.buf.iter().map(move |event| Event::of(this, event)))
    }
}

/// The runtime's model requests. A request borrows it until the next take.
pub struct Fetches {
    handle: u64,
}

impl Fetches {
    /// Takes the model request the core published, if any.
    pub fn take(&mut self) -> Result<Option<FetchRequest<'_>>, Error> {
        assert_off_main();
        // SAFETY: an all-zero fx_fetch_request is a valid value.
        let mut out: sys::fx_fetch_request = unsafe { std::mem::zeroed() };
        out.struct_size = std::mem::size_of::<sys::fx_fetch_request>() as u32;
        let handle = self.handle;
        // SAFETY: `out` is valid for the call.
        if check(retry_busy(|| unsafe {
            sys::fx_fetch_take(handle, &mut out)
        }))? != Outcome::Applied
        {
            return Ok(None);
        }
        let this: &Self = self;
        let headers = if out.header_count == 0 || out.headers.is_null() {
            &[][..]
        } else {
            // SAFETY: the core hands out `header_count` headers, borrowed
            // until the next take, which needs `&mut self`.
            unsafe { std::slice::from_raw_parts(out.headers, out.header_count) }
        };
        Ok(Some(FetchRequest {
            fetch: out.fetch,
            turn: out.turn,
            purpose: if out.purpose == sys::FX_FETCH_CATALOG {
                Purpose::Catalog
            } else {
                Purpose::Model
            },
            provider: str_in(this, out.provider),
            method: str_in(this, out.method),
            url: str_in(this, out.url),
            headers,
            body: bytes_in(this, out.body),
        }))
    }
}

/// One agent session on its own core thread, owned by the thread that drains
/// it. [`Runtime::close`] tears it down on that thread; dropping it hands
/// the teardown to a thread of its own, so the dropper never blocks.
pub struct Runtime {
    shared: Arc<Shared>,
    host: *mut HostBox,
    pub events: Events,
    pub fetches: Fetches,
    closed: bool,
    /// Owned by one thread at a time; not shared.
    _not_sync: PhantomData<std::cell::Cell<()>>,
}

// SAFETY: the handle is a plain value the core checks on every call; the
// host box is only freed after fx_runtime_destroy has returned.
unsafe impl Send for Runtime {}

/// The calls any thread may make on a runtime. Cheap to copy; safe after the
/// runtime is destroyed (every call then returns [`Error::Closed`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Remote {
    handle: u64,
}

impl Runtime {
    /// Makes a runtime: copies `config`, starts the core thread and sends it
    /// `initialize`. Open its session with [`Runtime::open_session`].
    pub fn create(config: &Config<'_>, host: Arc<dyn Host>) -> Result<Runtime, Error> {
        assert_off_main();
        let tools: Vec<sys::fx_tool_desc> = config
            .tools
            .iter()
            .map(|tool| sys::fx_tool_desc {
                name: sys::fx_str::of(tool.name),
                description: sys::fx_str::of(tool.description),
                input_schema_json: sys::fx_str::of(tool.input_schema_json),
                flags: if tool.idempotent {
                    sys::FX_TOOL_IDEMPOTENT
                } else {
                    0
                } | if tool.writes { sys::FX_TOOL_WRITES } else { 0 },
                reserved: 0,
            })
            .collect();
        let cfg = sys::fx_config {
            struct_size: std::mem::size_of::<sys::fx_config>() as u32,
            abi_version: sys::FX_CORE_ABI_VERSION,
            provider: sys::fx_str::of(config.provider),
            model: sys::fx_str::of(config.model),
            effort: sys::fx_str::of(config.effort),
            configured_providers_json: sys::fx_str::of(config.configured_providers_json),
            endpoint_url: sys::fx_str::of(config.endpoint_url),
            instructions: sys::fx_str::of(config.instructions),
            home: sys::fx_str::of(config.home),
            workspace_root: sys::fx_str::of(config.workspace_root),
            tools: if tools.is_empty() {
                std::ptr::null()
            } else {
                tools.as_ptr()
            },
            tool_count: tools.len(),
            max_agent_steps: config.max_agent_steps,
            max_concurrent_tools: config.max_concurrent_tools,
            max_tool_result_bytes: config.max_tool_result_bytes,
            max_history_turns: config.max_history_turns,
            max_stop_continuations: config.max_stop_continuations,
            flags: 0,
        };
        let journaled = host.journaled();
        let callbacks = sys::fx_host {
            struct_size: std::mem::size_of::<sys::fx_host>() as u32,
            reserved: 0,
            tool_call: if tools.is_empty() {
                None
            } else {
                Some(host_tool_call)
            },
            tool_cancel: if tools.is_empty() {
                None
            } else {
                Some(host_tool_cancel)
            },
            journal_append: if journaled {
                Some(host_journal_append)
            } else {
                None
            },
            hook: None,
        };
        let shared = Arc::new(Shared {
            handle: std::sync::atomic::AtomicU64::new(0),
            leases: Mutex::new(Leases::default()),
            changed: Condvar::new(),
        });
        let host = Box::into_raw(Box::new(HostBox {
            host,
            shared: shared.clone(),
        }));
        let mut handle = 0u64;
        // SAFETY: every pointer is valid for the call; the core copies cfg
        // and callbacks, and keeps `host` as an opaque value until destroy.
        let status =
            unsafe { sys::fx_runtime_create(&cfg, &callbacks, host as *mut c_void, &mut handle) };
        if let Err(error) = applied(status) {
            // SAFETY: the core keeps nothing after a failed create.
            drop(unsafe { Box::from_raw(host) });
            return Err(error);
        }
        // The handle is known only now. No callback can have run yet: none
        // comes before the session is open, which needs `&mut self`.
        shared
            .handle
            .store(handle, std::sync::atomic::Ordering::Release);
        Ok(Runtime {
            shared,
            host,
            events: Events {
                handle,
                buf: Vec::with_capacity(EVENT_BATCH),
            },
            fetches: Fetches { handle },
            closed: false,
            _not_sync: PhantomData,
        })
    }

    fn handle(&self) -> u64 {
        self.shared.handle()
    }

    /// The calls other threads may make on this runtime.
    pub fn remote(&self) -> Remote {
        Remote {
            handle: self.handle(),
        }
    }

    /// The readiness descriptor, borrowed from the runtime (the core owns and
    /// closes it): readable when events, a fetch or an exit wait. Read and
    /// discard its bytes, then drain everything. Unregister it from any
    /// poller before the runtime goes.
    #[cfg(unix)]
    pub fn ready_fd(&self) -> Option<std::os::fd::BorrowedFd<'_>> {
        assert_off_main();
        // SAFETY: plain value call.
        let fd = unsafe { sys::fx_runtime_ready_fd(self.handle()) };
        // SAFETY: the core keeps the descriptor open until destroy, which
        // needs the runtime by value, after this borrow ends.
        (fd >= 0).then(|| unsafe { std::os::fd::BorrowedFd::borrow_raw(fd) })
    }

    pub fn info(&self) -> Result<(State, String), Error> {
        assert_off_main();
        // SAFETY: an all-zero fx_runtime_info is a valid value.
        let mut info: sys::fx_runtime_info = unsafe { std::mem::zeroed() };
        info.struct_size = std::mem::size_of::<sys::fx_runtime_info>() as u32;
        // SAFETY: `info` is valid for the call.
        applied(unsafe { sys::fx_runtime_info_get(self.handle(), &mut info) })?;
        Ok((
            State::of(info.state, info.exit_code),
            str_in(self, info.message).to_string(),
        ))
    }

    /// Opens the runtime's new session. Answered by
    /// [`Event::SessionOpened`] or [`Event::RequestDone`] with the request id.
    pub fn open_session(&mut self, session_id: &str) -> Result<u64, Error> {
        assert_off_main();
        let args = sys::fx_session_open_args {
            struct_size: std::mem::size_of::<sys::fx_session_open_args>() as u32,
            flags: 0,
            session_id: sys::fx_str::of(session_id),
            journal_json: sys::fx_str::EMPTY,
            checkpoint: sys::fx_bytes::EMPTY,
        };
        let mut request = 0u64;
        // SAFETY: args and request are valid for the call; the core copies.
        applied(retry_busy(|| unsafe {
            sys::fx_session_open(self.handle(), &args, &mut request)
        }))?;
        Ok(request)
    }

    /// Starts a turn; returns its id, which its events carry. A refused
    /// prompt is an error, never a turn id.
    pub fn prompt(&mut self, text: &str, message_id: &str) -> Result<u64, Error> {
        assert_off_main();
        let input = sys::fx_input {
            text: sys::fx_str::of(text),
            images: std::ptr::null(),
            image_count: 0,
        };
        let mut turn = 0u64;
        // SAFETY: valid for the call; the core copies.
        applied(retry_busy(|| unsafe {
            sys::fx_turn_prompt(
                self.handle(),
                &input,
                sys::fx_str::of(message_id),
                &mut turn,
            )
        }))?;
        Ok(turn)
    }

    /// Steers running turn `turn` with text.
    pub fn steer(&mut self, turn: u64, text: &str, message_id: &str) -> Result<Steered, Error> {
        assert_off_main();
        let input = sys::fx_input {
            text: sys::fx_str::of(text),
            images: std::ptr::null(),
            image_count: 0,
        };
        let mut request = 0u64;
        // SAFETY: valid for the call; the core copies.
        match applied(retry_busy(|| unsafe {
            sys::fx_turn_steer(
                self.handle(),
                turn,
                &input,
                sys::fx_str::of(message_id),
                &mut request,
            )
        })) {
            Ok(()) => Ok(Steered::Accepted(request)),
            Err(Error::Stale) => Ok(Steered::Stale),
            Err(error) => Err(error),
        }
    }

    /// Withdraws a steer of turn `turn`; answered by [`Event::RequestDone`]
    /// (`Applied` withdrawn, `Stale` already joined). `Err(Stale)` when the
    /// turn is not running.
    pub fn withdraw(&mut self, turn: u64, message_id: &str) -> Result<u64, Error> {
        assert_off_main();
        let mut request = 0u64;
        // SAFETY: valid for the call; the core copies.
        applied(retry_busy(|| unsafe {
            sys::fx_turn_withdraw(
                self.handle(),
                turn,
                sys::fx_str::of(message_id),
                &mut request,
            )
        }))?;
        Ok(request)
    }

    /// Destroys the runtime on this (worker) thread: new tool calls are
    /// answered at once, every lease is waited for, then the core is
    /// stopped and freed. Never call it on the main thread.
    pub fn close(mut self) {
        assert_off_main();
        self.closed = true;
        teardown(&self.shared, self.host);
    }

    #[doc(hidden)]
    pub fn arm_fault(&self, point: u32, skip: u32) -> Result<(), Error> {
        // SAFETY: plain value call.
        applied(unsafe { sys::fx_test_arm_fault(self.handle(), point, skip) })
    }
}

/// Waits for the leases, destroys the runtime, frees the host box.
fn teardown(shared: &Shared, host: *mut HostBox) {
    shared.wait_leases();
    // SAFETY: plain value call; when it returns no callback runs or will run.
    let _ = unsafe { sys::fx_runtime_destroy(shared.handle()) };
    // SAFETY: the box was made by create and nothing uses it any more.
    drop(unsafe { Box::from_raw(host) });
}

struct SendBox(*mut HostBox);
// SAFETY: the box is handed to the teardown thread and used only there.
unsafe impl Send for SendBox {}

impl Drop for Runtime {
    fn drop(&mut self) {
        if self.closed {
            return;
        }
        // Teardown waits for leases and for the core's threads: it never runs
        // on the dropping thread, which may be the UI thread.
        let shared = self.shared.clone();
        let host = SendBox(self.host);
        let spawned = std::thread::Builder::new()
            .name("aero-teardown".into())
            .spawn(move || {
                let host = host;
                teardown(&shared, host.0);
            });
        if spawned.is_err() && !on_main_thread() {
            teardown(&self.shared, self.host);
        }
        // On the main thread with no thread to spare, the runtime is leaked
        // rather than blocking the UI.
    }
}

impl Remote {
    /// Cancels running turn `turn`. `Ok(Stale)` when it is not running.
    pub fn cancel(&self, turn: u64) -> Result<Outcome, Error> {
        assert_off_main();
        // SAFETY: plain value call.
        check(retry_busy(|| unsafe {
            sys::fx_turn_cancel(self.handle, turn)
        }))
    }

    pub fn fetch_status(
        &self,
        fetch: u64,
        status: u16,
        retry_after_s: Option<u32>,
    ) -> Result<Outcome, Error> {
        assert_off_main();
        let retry = retry_after_s.map_or(-1, |seconds| seconds.min(i32::MAX as u32) as i32);
        // SAFETY: plain value call.
        check(retry_busy(|| unsafe {
            sys::fx_fetch_status(self.handle, fetch, status, retry)
        }))
    }

    pub fn fetch_push(&self, fetch: u64, bytes: &[u8]) -> Result<Outcome, Error> {
        assert_off_main();
        // SAFETY: bytes is valid for the call; the core copies.
        check(retry_busy(|| unsafe {
            sys::fx_fetch_push(self.handle, fetch, bytes.as_ptr(), bytes.len())
        }))
    }

    pub fn fetch_finish(&self, fetch: u64, ok: bool) -> Result<Outcome, Error> {
        assert_off_main();
        // SAFETY: plain value call.
        check(retry_busy(|| unsafe {
            sys::fx_fetch_finish(self.handle, fetch, ok)
        }))
    }

    pub fn fetch_disposition(&self, fetch: u64) -> Disposition {
        assert_off_main();
        // SAFETY: plain value call.
        match unsafe { sys::fx_fetch_disposition_get(self.handle, fetch) } {
            sys::FX_FETCH_ACTIVE => Disposition::Active,
            sys::FX_FETCH_CONSUMED => Disposition::Consumed,
            _ => Disposition::Retired,
        }
    }

    /// The store holds every append up to `append` durably.
    pub fn journal_ack(&self, append: u64) -> Result<Outcome, Error> {
        assert_off_main();
        // SAFETY: plain value call.
        check(retry_busy(|| unsafe {
            sys::fx_journal_ack(self.handle, append)
        }))
    }

    /// The store refused `append`.
    pub fn journal_fail(&self, append: u64, reason: &str, fenced: bool) -> Result<Outcome, Error> {
        assert_off_main();
        // SAFETY: reason is valid for the call; the core copies.
        check(retry_busy(|| unsafe {
            sys::fx_journal_fail(self.handle, append, sys::fx_str::of(reason), fenced)
        }))
    }

    /// Runtime state, lock-free (works on a panicked runtime).
    pub fn state(&self) -> Result<State, Error> {
        assert_off_main();
        // SAFETY: an all-zero fx_runtime_info is a valid value.
        let mut info: sys::fx_runtime_info = unsafe { std::mem::zeroed() };
        info.struct_size = std::mem::size_of::<sys::fx_runtime_info>() as u32;
        // SAFETY: `info` is valid for the call.
        applied(unsafe { sys::fx_runtime_info_get(self.handle, &mut info) })?;
        Ok(State::of(info.state, info.exit_code))
    }

    #[doc(hidden)]
    pub fn handle(&self) -> u64 {
        self.handle
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn statuses_map_to_outcomes_and_errors() {
        assert_eq!(check(sys::FX_OK), Ok(Outcome::Applied));
        assert_eq!(check(sys::FX_STALE), Ok(Outcome::Stale));
        assert_eq!(check(sys::FX_BUSY), Ok(Outcome::Busy));
        assert_eq!(check(sys::FX_E_CLOSED), Err(Error::Closed));
        assert_eq!(check(sys::FX_E_TOO_LARGE), Err(Error::TooLarge));
        assert_eq!(check(-77), Err(Error::Unknown(-77)));
        // A call that must apply never reads a refusal as success.
        assert_eq!(applied(sys::FX_BACKPRESSURE), Err(Error::Backpressure));
        assert_eq!(applied(sys::FX_STALE), Err(Error::Stale));
        assert_eq!(applied(sys::FX_BUSY), Err(Error::Busy));
    }

    #[test]
    fn struct_layouts_match_the_header() {
        // The header's FX_*_SIZE_V2, which the Zig entry asserts too.
        use std::mem::size_of;
        assert_eq!(size_of::<sys::fx_str>(), 16);
        assert_eq!(size_of::<sys::fx_tool_call>(), 72);
        assert_eq!(size_of::<sys::fx_journal_append>(), 56);
        assert_eq!(size_of::<sys::fx_hook_question>(), 88);
        assert_eq!(size_of::<sys::fx_host>(), 40);
        assert_eq!(size_of::<sys::fx_config>(), 176);
        assert_eq!(size_of::<sys::fx_runtime_info>(), 32);
        assert_eq!(size_of::<sys::fx_session_open_args>(), 56);
        assert_eq!(size_of::<sys::fx_event>(), 160);
        assert_eq!(size_of::<sys::fx_usage>(), 56);
        assert_eq!(size_of::<sys::fx_fetch_request>(), 104);
        assert_eq!(size_of::<sys::fx_tool_desc>(), 56);
        assert_eq!(size_of::<sys::fx_tool_output>(), 40);
        assert_eq!(size_of::<sys::fx_input>(), 32);
        assert_eq!(size_of::<sys::fx_image>(), 32);
    }

    #[test]
    fn origins_are_scheme_host_and_non_default_port() {
        assert_eq!(
            origin_of("https://ai-gateway.vercel.sh/v1/chat").as_deref(),
            Some("https://ai-gateway.vercel.sh")
        );
        assert_eq!(
            origin_of("HTTPS://AI-Gateway.vercel.sh:443/x").as_deref(),
            Some("https://ai-gateway.vercel.sh")
        );
        assert_eq!(
            origin_of("http://127.0.0.1:8080/fx").as_deref(),
            Some("http://127.0.0.1:8080")
        );
        assert_eq!(
            origin_of("https://user@evil.example/x").as_deref(),
            Some("https://evil.example")
        );
        assert_eq!(origin_of("file:///etc/passwd"), None);
    }
}
