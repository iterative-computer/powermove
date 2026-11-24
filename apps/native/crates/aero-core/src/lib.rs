//! Aero's agent core: the vendored libfx core (`apps/native/vendor/fx`) behind
//! its C ABI (`vendor/fx/include/fx_core.h`), and a minimal safe wrapper.
//!
//! One [`Runtime`] is one agent session on its own core thread. The thread
//! that owns the `Runtime` (the `pm-aero` worker) drains it: typed events,
//! model HTTP requests. Calls the header allows from any thread (tool results,
//! the fetch stream, cancel, journal acks) go through a [`Remote`], which is
//! `Send + Sync` and safe after the runtime is gone (its handle is
//! generation-tagged, so it then returns [`Error::Closed`]).
//!
//! No call may be made on the main thread: every one asserts it in debug
//! builds. Built without the pinned Zig, every call returns
//! [`Error::Unavailable`] (see `build.rs`).

pub mod sys;

use std::ffi::c_void;
use std::fmt;
use std::os::fd::RawFd;
use std::sync::Arc;

/// An error from the core. Mirrors the header's negative statuses.
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
    Unknown(i32),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let text = match self {
            Error::Invalid => "invalid argument",
            Error::Closed => "the runtime is closed",
            Error::Failed => "the runtime failed",
            Error::Limit => "too many runtimes",
            Error::OutOfMemory => "the core is out of memory",
            Error::Thread => "the core could not start a thread",
            Error::Io => "the core could not make its readiness socket",
            Error::State => "not valid in the runtime's current state",
            Error::Unavailable => "this build has no libfx core",
            Error::Unsupported => "not supported by this core yet",
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
}

fn check(status: sys::fx_status) -> Result<Outcome, Error> {
    Ok(match status {
        sys::FX_OK => Outcome::Applied,
        sys::FX_EMPTY => Outcome::Empty,
        sys::FX_STALE => Outcome::Stale,
        sys::FX_BACKPRESSURE => Outcome::Backpressure,
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
        other => return Err(Error::Unknown(other)),
    })
}

fn applied(status: sys::fx_status) -> Result<(), Error> {
    check(status).map(|_| ())
}

/// The main thread never calls the core (design 2.4).
#[inline]
fn assert_off_main() {
    debug_assert!(
        // SAFETY: pthread_main_np has no preconditions.
        unsafe { libc::pthread_main_np() } == 0,
        "the libfx core is never called on the main thread"
    );
}

/// Whether this build links the core (built with the pinned Zig).
pub fn available() -> bool {
    // SAFETY: no arguments, no state.
    unsafe { sys::fx_core_abi_version() == sys::FX_CORE_ABI_VERSION }
}

fn str_of(value: sys::fx_str) -> &'static str {
    if value.len == 0 || value.ptr.is_null() {
        return "";
    }
    // SAFETY: the core hands out valid pointers with their lengths; the
    // caller ties the result to the borrow the header gives it.
    let bytes = unsafe { std::slice::from_raw_parts(value.ptr, value.len) };
    std::str::from_utf8(bytes).unwrap_or("")
}

fn bytes_of(value: sys::fx_bytes) -> &'static [u8] {
    if value.len == 0 || value.ptr.is_null() {
        return &[];
    }
    // SAFETY: as `str_of`.
    unsafe { std::slice::from_raw_parts(value.ptr, value.len) }
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

/// A tool call, borrowed for the callback.
#[derive(Clone, Copy, Debug)]
pub struct ToolCall<'a> {
    pub call: u64,
    pub name: &'a str,
    pub input_json: &'a str,
    pub model_call_id: &'a str,
    pub idempotent: bool,
    pub writes: bool,
}

/// The host side of a runtime. Every method runs on a core-owned thread and
/// must only record or enqueue (see the header's fx_host).
pub trait Host: Send + Sync + 'static {
    /// The model called a host tool; answer later with [`Remote::tool_result`].
    fn tool_call(&self, _call: ToolCall<'_>) {}
    /// A call's turn ended before it was answered; it still needs its result.
    fn tool_cancel(&self, _call: u64) {}
}

struct HostBox {
    host: Arc<dyn Host>,
}

unsafe extern "C" fn host_tool_call(
    ctx: *mut c_void,
    call: u64,
    name: sys::fx_str,
    input_json: sys::fx_str,
    model_call_id: sys::fx_str,
    flags: u32,
) {
    // SAFETY: ctx is the HostBox the runtime was made with; it outlives the
    // runtime (freed after fx_runtime_destroy returns).
    let host = unsafe { &*(ctx as *const HostBox) };
    let call = ToolCall {
        call,
        name: str_of(name),
        input_json: str_of(input_json),
        model_call_id: str_of(model_call_id),
        idempotent: flags & sys::FX_TOOL_IDEMPOTENT != 0,
        writes: flags & sys::FX_TOOL_WRITES != 0,
    };
    // A panic must not unwind into the core.
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| host.host.tool_call(call)));
}

unsafe extern "C" fn host_tool_cancel(ctx: *mut c_void, call: u64) {
    // SAFETY: as host_tool_call.
    let host = unsafe { &*(ctx as *const HostBox) };
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| host.host.tool_cancel(call)));
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

/// A typed event, borrowed from the runtime until its next drain.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Event<'a> {
    SessionOpened {
        request: u64,
        session_id: &'a str,
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
        text: &'a str,
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
        bytes: &'a [u8],
    },
    Error {
        turn: u64,
        error: Error,
        message: &'a str,
    },
    Exited {
        code: i32,
    },
    Unknown {
        kind: u32,
    },
}

impl<'a> Event<'a> {
    fn of(event: &sys::fx_event) -> Event<'a> {
        match event.kind {
            sys::FX_EVENT_SESSION_OPENED => Event::SessionOpened {
                request: event.request,
                session_id: str_of(event.text),
                resumable: event.flags & sys::FX_OPENED_RESUMABLE != 0,
            },
            sys::FX_EVENT_IMAGES_NEEDED => Event::ImagesNeeded {
                request: event.request,
                count: event.name_count,
            },
            sys::FX_EVENT_REQUEST_DONE => Event::RequestDone {
                request: event.request,
                outcome: check(event.status),
                message: str_of(event.text),
            },
            sys::FX_EVENT_TURN_START => Event::TurnStart {
                turn: event.turn,
                message_id: str_of(event.text),
            },
            sys::FX_EVENT_TEXT_DELTA => Event::TextDelta {
                turn: event.turn,
                text: str_of(event.text),
            },
            sys::FX_EVENT_THOUGHT_DELTA => Event::ThoughtDelta {
                turn: event.turn,
                text: str_of(event.text),
            },
            sys::FX_EVENT_STEER_JOINED => Event::SteerJoined {
                turn: event.turn,
                text: str_of(event.text),
                message_id: str_of(event.text2),
            },
            sys::FX_EVENT_TOOL_START => Event::ToolStart {
                turn: event.turn,
                name: str_of(event.text),
                model_call_id: str_of(event.text2),
            },
            sys::FX_EVENT_TOOL_END => Event::ToolEnd {
                turn: event.turn,
                name: str_of(event.text),
                model_call_id: str_of(event.text2),
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
                bytes: bytes_of(event.bytes),
            },
            sys::FX_EVENT_ERROR => Event::Error {
                turn: event.turn,
                error: check(event.status).err().unwrap_or(Error::Failed),
                message: str_of(event.text),
            },
            sys::FX_EVENT_EXITED => Event::Exited { code: event.code },
            kind => Event::Unknown { kind },
        }
    }
}

/// A model HTTP request, borrowed until the next [`Runtime::take_fetch`].
#[derive(Clone, Copy, Debug)]
pub struct FetchRequest<'a> {
    pub fetch: u64,
    pub method: &'a str,
    pub url: &'a str,
    headers: &'a [sys::fx_header],
    pub body: &'a [u8],
}

impl<'a> FetchRequest<'a> {
    /// The request's headers, as the core built them. The host replaces any
    /// authorization header with its own credentials.
    pub fn headers(&self) -> impl Iterator<Item = (&'a str, &'a str)> + 'a {
        self.headers
            .iter()
            .map(|header| (str_of(header.name), str_of(header.value)))
    }
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

/// Runtime state, from [`Runtime::info`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Running,
    Exited(i32),
    Failed,
}

/// One agent session on its own core thread. Owned by the thread that drains
/// it; dropping it destroys the runtime (joining its thread).
pub struct Runtime {
    handle: u64,
    host: *mut HostBox,
    events: Vec<sys::fx_event>,
}

// SAFETY: the handle is a plain value the core checks on every call; the
// host box is only freed by Drop, after the core thread has been joined.
unsafe impl Send for Runtime {}

/// The calls any thread may make on a runtime. Cheap to copy; safe after the
/// runtime is destroyed (every call then returns [`Error::Closed`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Remote {
    handle: u64,
}

const EVENT_BATCH: usize = 256;

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
            reserved: 0,
        };
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
            journal_append: None,
            pre_tool: None,
            stop: None,
        };
        let host = Box::into_raw(Box::new(HostBox { host }));
        let mut handle = 0u64;
        // SAFETY: every pointer is valid for the call; the core copies cfg and
        // callbacks, and keeps `host` as an opaque value.
        let status =
            unsafe { sys::fx_runtime_create(&cfg, &callbacks, host as *mut c_void, &mut handle) };
        if let Err(error) = applied(status) {
            // SAFETY: the core keeps nothing after a failed create.
            drop(unsafe { Box::from_raw(host) });
            return Err(error);
        }
        Ok(Runtime {
            handle,
            host,
            events: Vec::with_capacity(EVENT_BATCH),
        })
    }

    /// The calls other threads may make on this runtime.
    pub fn remote(&self) -> Remote {
        Remote {
            handle: self.handle,
        }
    }

    /// The readiness descriptor: readable when events, a fetch or an exit
    /// wait. Owned by the core: never close it. Read and discard its bytes,
    /// then drain everything.
    pub fn ready_fd(&self) -> RawFd {
        assert_off_main();
        // SAFETY: plain value call.
        unsafe { sys::fx_runtime_ready_fd(self.handle) }
    }

    pub fn info(&self) -> Result<State, Error> {
        assert_off_main();
        let mut info = sys::fx_runtime_info {
            state: 0,
            exit_code: 0,
            message: sys::fx_str::EMPTY,
        };
        // SAFETY: `info` is valid for the call.
        applied(unsafe { sys::fx_runtime_info_get(self.handle, &mut info) })?;
        Ok(match info.state {
            sys::FX_RUNTIME_EXITED => State::Exited(info.exit_code),
            sys::FX_RUNTIME_FAILED => State::Failed,
            _ => State::Running,
        })
    }

    /// Opens the runtime's new session. Answered by
    /// [`Event::SessionOpened`] or [`Event::RequestDone`] with the request id.
    pub fn open_session(&mut self, session_id: &str) -> Result<u64, Error> {
        assert_off_main();
        let args = sys::fx_session_open_args {
            struct_size: std::mem::size_of::<sys::fx_session_open_args>() as u32,
            reserved: 0,
            session_id: sys::fx_str::of(session_id),
            journal_json: sys::fx_str::EMPTY,
            checkpoint: sys::fx_bytes::EMPTY,
            images: std::ptr::null(),
            image_count: 0,
            inputs_durable: false,
        };
        let mut request = 0u64;
        // SAFETY: args and request are valid for the call; the core copies.
        applied(unsafe { sys::fx_session_open(self.handle, &args, &mut request) })?;
        Ok(request)
    }

    /// Starts a turn; returns its id, which its events carry.
    pub fn prompt(&mut self, text: &str, message_id: &str) -> Result<u64, Error> {
        assert_off_main();
        let input = sys::fx_input {
            text: sys::fx_str::of(text),
            images: std::ptr::null(),
            image_count: 0,
        };
        let mut turn = 0u64;
        // SAFETY: valid for the call; the core copies.
        applied(unsafe {
            sys::fx_turn_prompt(self.handle, &input, sys::fx_str::of(message_id), &mut turn)
        })?;
        Ok(turn)
    }

    /// Steers the running turn; answered by [`Event::RequestDone`].
    pub fn steer(&mut self, text: &str, message_id: &str) -> Result<u64, Error> {
        assert_off_main();
        let mut request = 0u64;
        // SAFETY: valid for the call; the core copies.
        applied(unsafe {
            sys::fx_turn_steer(
                self.handle,
                sys::fx_str::of(text),
                sys::fx_str::of(message_id),
                &mut request,
            )
        })?;
        Ok(request)
    }

    /// Withdraws a steer that has not joined; answered by
    /// [`Event::RequestDone`] (`Applied` withdrawn, `Stale` already joined).
    pub fn withdraw(&mut self, message_id: &str) -> Result<u64, Error> {
        assert_off_main();
        let mut request = 0u64;
        // SAFETY: valid for the call; the core copies.
        applied(unsafe {
            sys::fx_turn_withdraw(self.handle, sys::fx_str::of(message_id), &mut request)
        })?;
        Ok(request)
    }

    /// Asks for a checkpoint; it arrives as [`Event::Checkpoint`].
    pub fn checkpoint(&mut self) -> Result<u64, Error> {
        assert_off_main();
        let mut request = 0u64;
        // SAFETY: valid for the call.
        applied(unsafe { sys::fx_checkpoint(self.handle, &mut request) })?;
        Ok(request)
    }

    /// Drains the queued events, oldest first. They borrow `self` until the
    /// next drain; draining allocates nothing.
    pub fn drain(&mut self) -> Result<impl Iterator<Item = Event<'_>> + '_, Error> {
        assert_off_main();
        self.events.clear();
        let mut count = 0usize;
        // SAFETY: the buffer holds EVENT_BATCH events; the core writes at most
        // `cap` of them and sets `count`.
        applied(unsafe {
            sys::fx_events_drain(
                self.handle,
                self.events.as_mut_ptr(),
                self.events.capacity(),
                &mut count,
            )
        })?;
        // SAFETY: the core initialised `count` events.
        unsafe { self.events.set_len(count) };
        Ok(self.events.iter().map(Event::of))
    }

    /// Takes the model request the core published, if any.
    pub fn take_fetch(&mut self) -> Result<Option<FetchRequest<'_>>, Error> {
        assert_off_main();
        let mut out = sys::fx_fetch_request {
            fetch: 0,
            method: sys::fx_str::EMPTY,
            url: sys::fx_str::EMPTY,
            headers: std::ptr::null(),
            header_count: 0,
            body: sys::fx_bytes::EMPTY,
        };
        // SAFETY: `out` is valid for the call.
        if check(unsafe { sys::fx_fetch_take(self.handle, &mut out) })? == Outcome::Empty {
            return Ok(None);
        }
        let headers = if out.header_count == 0 || out.headers.is_null() {
            &[][..]
        } else {
            // SAFETY: the core hands out `header_count` headers, borrowed
            // until the next take, which needs `&mut self`.
            unsafe { std::slice::from_raw_parts(out.headers, out.header_count) }
        };
        Ok(Some(FetchRequest {
            fetch: out.fetch,
            method: str_of(out.method),
            url: str_of(out.url),
            headers,
            body: bytes_of(out.body),
        }))
    }
}

impl Drop for Runtime {
    fn drop(&mut self) {
        // SAFETY: plain value call; joins the core thread.
        let _ = unsafe { sys::fx_runtime_destroy(self.handle) };
        // SAFETY: the core thread is joined: no callback can run any more.
        drop(unsafe { Box::from_raw(self.host) });
    }
}

impl Remote {
    /// Answers tool call `call` with text (copied).
    pub fn tool_result(&self, call: u64, text: &str, is_error: bool) -> Result<Outcome, Error> {
        assert_off_main();
        let out = sys::fx_tool_output {
            text: sys::fx_str::of(text),
            images: std::ptr::null(),
            image_count: 0,
            is_error,
        };
        // SAFETY: valid for the call; the core copies.
        check(unsafe { sys::fx_tool_result(self.handle, call, &out) })
    }

    /// Cancels the running turn.
    pub fn cancel(&self) -> Result<(), Error> {
        assert_off_main();
        // SAFETY: plain value call.
        applied(unsafe { sys::fx_turn_cancel(self.handle) })
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
        check(unsafe { sys::fx_fetch_status(self.handle, fetch, status, retry) })
    }

    pub fn fetch_push(&self, fetch: u64, bytes: &[u8]) -> Result<Outcome, Error> {
        assert_off_main();
        // SAFETY: bytes is valid for the call; the core copies.
        check(unsafe { sys::fx_fetch_push(self.handle, fetch, bytes.as_ptr(), bytes.len()) })
    }

    pub fn fetch_finish(&self, fetch: u64, ok: bool) -> Result<Outcome, Error> {
        assert_off_main();
        // SAFETY: plain value call.
        check(unsafe { sys::fx_fetch_finish(self.handle, fetch, ok) })
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn statuses_map_to_outcomes_and_errors() {
        assert_eq!(check(sys::FX_OK), Ok(Outcome::Applied));
        assert_eq!(check(sys::FX_STALE), Ok(Outcome::Stale));
        assert_eq!(check(sys::FX_E_CLOSED), Err(Error::Closed));
        assert_eq!(check(-77), Err(Error::Unknown(-77)));
    }

    #[test]
    fn struct_layouts_match_the_header() {
        // sizeof() of each header struct, as clang compiles fx_core.h for
        // aarch64-apple-darwin.
        assert_eq!(std::mem::size_of::<sys::fx_str>(), 16);
        assert_eq!(std::mem::size_of::<sys::fx_host>(), 48);
        assert_eq!(std::mem::size_of::<sys::fx_event>(), 160);
        assert_eq!(std::mem::size_of::<sys::fx_config>(), 176);
        assert_eq!(std::mem::size_of::<sys::fx_usage>(), 56);
        assert_eq!(std::mem::size_of::<sys::fx_session_open_args>(), 80);
        assert_eq!(std::mem::size_of::<sys::fx_fetch_request>(), 72);
        assert_eq!(std::mem::size_of::<sys::fx_tool_output>(), 40);
    }
}
