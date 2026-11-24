//! The C ABI of `apps/native/vendor/fx/include/fx_core.h`, declared by hand.
//! Every type here is `repr(C)` and matches the header field for field; the
//! header is the contract, with its threading and ownership rules.
//!
//! Built without the core (`cfg(aero_core = "unavailable")`), the functions
//! are Rust stubs that return `FX_E_UNAVAILABLE`.

#![allow(non_camel_case_types, dead_code)]

use std::ffi::c_void;

pub const FX_CORE_ABI_VERSION: u32 = 1;

pub type fx_status = i32;
pub const FX_OK: fx_status = 0;
pub const FX_EMPTY: fx_status = 1;
pub const FX_STALE: fx_status = 2;
pub const FX_BACKPRESSURE: fx_status = 3;
pub const FX_E_INVALID: fx_status = -1;
pub const FX_E_CLOSED: fx_status = -2;
pub const FX_E_FAILED: fx_status = -3;
pub const FX_E_LIMIT: fx_status = -4;
pub const FX_E_OOM: fx_status = -5;
pub const FX_E_THREAD: fx_status = -6;
pub const FX_E_IO: fx_status = -7;
pub const FX_E_STATE: fx_status = -8;
pub const FX_E_UNAVAILABLE: fx_status = -9;
pub const FX_E_UNSUPPORTED: fx_status = -10;

pub type fx_runtime = u64;

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct fx_str {
    pub ptr: *const u8,
    pub len: usize,
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct fx_bytes {
    pub ptr: *const u8,
    pub len: usize,
}

impl fx_str {
    pub const EMPTY: fx_str = fx_str {
        ptr: std::ptr::null(),
        len: 0,
    };

    pub fn of(text: &str) -> fx_str {
        fx_str {
            ptr: text.as_ptr(),
            len: text.len(),
        }
    }
}

impl fx_bytes {
    pub const EMPTY: fx_bytes = fx_bytes {
        ptr: std::ptr::null(),
        len: 0,
    };

    pub fn of(bytes: &[u8]) -> fx_bytes {
        fx_bytes {
            ptr: bytes.as_ptr(),
            len: bytes.len(),
        }
    }
}

pub const FX_TOOL_IDEMPOTENT: u32 = 1 << 0;
pub const FX_TOOL_WRITES: u32 = 1 << 1;

pub type fx_hook_verdict = i32;
pub const FX_HOOK_ALLOW: fx_hook_verdict = 0;
pub const FX_HOOK_PENDING: fx_hook_verdict = 1;

pub type fx_hook_answer_kind = i32;
pub const FX_ANSWER_ALLOW: fx_hook_answer_kind = 0;
pub const FX_ANSWER_BLOCK: fx_hook_answer_kind = 1;
pub const FX_ANSWER_REWRITE: fx_hook_answer_kind = 2;
pub const FX_ANSWER_CONTINUE: fx_hook_answer_kind = 3;

pub type ToolCallFn = unsafe extern "C" fn(*mut c_void, u64, fx_str, fx_str, fx_str, u32);
pub type ToolCancelFn = unsafe extern "C" fn(*mut c_void, u64);
pub type JournalAppendFn = unsafe extern "C" fn(*mut c_void, u64, fx_str, bool);
pub type PreToolFn = unsafe extern "C" fn(*mut c_void, u64, u64, fx_str, fx_str) -> fx_hook_verdict;
pub type StopFn = unsafe extern "C" fn(*mut c_void, u64, fx_str, u32) -> fx_hook_verdict;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct fx_host {
    pub struct_size: u32,
    pub reserved: u32,
    pub tool_call: Option<ToolCallFn>,
    pub tool_cancel: Option<ToolCancelFn>,
    pub journal_append: Option<JournalAppendFn>,
    pub pre_tool: Option<PreToolFn>,
    pub stop: Option<StopFn>,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct fx_tool_desc {
    pub name: fx_str,
    pub description: fx_str,
    pub input_schema_json: fx_str,
    pub flags: u32,
    pub reserved: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct fx_config {
    pub struct_size: u32,
    pub abi_version: u32,
    pub provider: fx_str,
    pub model: fx_str,
    pub effort: fx_str,
    pub configured_providers_json: fx_str,
    pub endpoint_url: fx_str,
    pub instructions: fx_str,
    pub home: fx_str,
    pub workspace_root: fx_str,
    pub tools: *const fx_tool_desc,
    pub tool_count: usize,
    pub max_agent_steps: u32,
    pub max_concurrent_tools: u32,
    pub max_tool_result_bytes: u32,
    pub max_history_turns: u32,
    pub max_stop_continuations: u32,
    pub reserved: u32,
}

pub type fx_runtime_state = i32;
pub const FX_RUNTIME_RUNNING: fx_runtime_state = 0;
pub const FX_RUNTIME_EXITED: fx_runtime_state = 1;
pub const FX_RUNTIME_FAILED: fx_runtime_state = 2;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct fx_runtime_info {
    pub state: fx_runtime_state,
    pub exit_code: i32,
    pub message: fx_str,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct fx_named_bytes {
    pub name: fx_str,
    pub bytes: fx_bytes,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct fx_session_open_args {
    pub struct_size: u32,
    pub reserved: u32,
    pub session_id: fx_str,
    pub journal_json: fx_str,
    pub checkpoint: fx_bytes,
    pub images: *const fx_named_bytes,
    pub image_count: usize,
    pub inputs_durable: bool,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct fx_image {
    pub mime: fx_str,
    pub data: fx_bytes,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct fx_input {
    pub text: fx_str,
    pub images: *const fx_image,
    pub image_count: usize,
}

pub type fx_event_kind = u32;
pub const FX_EVENT_SESSION_OPENED: fx_event_kind = 1;
pub const FX_EVENT_IMAGES_NEEDED: fx_event_kind = 2;
pub const FX_EVENT_REQUEST_DONE: fx_event_kind = 3;
pub const FX_EVENT_TURN_START: fx_event_kind = 4;
pub const FX_EVENT_TEXT_DELTA: fx_event_kind = 5;
pub const FX_EVENT_THOUGHT_DELTA: fx_event_kind = 6;
pub const FX_EVENT_STEER_JOINED: fx_event_kind = 7;
pub const FX_EVENT_TOOL_START: fx_event_kind = 8;
pub const FX_EVENT_TOOL_END: fx_event_kind = 9;
pub const FX_EVENT_USAGE: fx_event_kind = 10;
pub const FX_EVENT_COMPACTED: fx_event_kind = 11;
pub const FX_EVENT_TURN_END: fx_event_kind = 12;
pub const FX_EVENT_CHECKPOINT: fx_event_kind = 13;
pub const FX_EVENT_ERROR: fx_event_kind = 14;
pub const FX_EVENT_EXITED: fx_event_kind = 15;

pub const FX_OPENED_RESUMABLE: u32 = 1 << 0;

pub type fx_stop_reason = i32;
pub const FX_STOP_END_TURN: fx_stop_reason = 0;
pub const FX_STOP_MAX_TOKENS: fx_stop_reason = 1;
pub const FX_STOP_MAX_STEPS: fx_stop_reason = 2;
pub const FX_STOP_REFUSAL: fx_stop_reason = 3;
pub const FX_STOP_CANCELLED: fx_stop_reason = 4;
pub const FX_STOP_YIELDED: fx_stop_reason = 5;
pub const FX_STOP_ERROR: fx_stop_reason = 6;

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct fx_usage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_write_tokens: u64,
    pub reasoning_tokens: u64,
    pub context_used: u64,
    pub context_size: u64,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct fx_event {
    pub kind: fx_event_kind,
    pub flags: u32,
    pub turn: u64,
    pub request: u64,
    pub id: u64,
    pub status: fx_status,
    pub code: i32,
    pub text: fx_str,
    pub text2: fx_str,
    pub bytes: fx_bytes,
    pub names: *const fx_str,
    pub name_count: usize,
    pub usage: fx_usage,
}

impl fx_event {
    pub const ZERO: fx_event = fx_event {
        kind: 0,
        flags: 0,
        turn: 0,
        request: 0,
        id: 0,
        status: 0,
        code: 0,
        text: fx_str::EMPTY,
        text2: fx_str::EMPTY,
        bytes: fx_bytes::EMPTY,
        names: std::ptr::null(),
        name_count: 0,
        usage: fx_usage {
            input_tokens: 0,
            output_tokens: 0,
            cache_read_tokens: 0,
            cache_write_tokens: 0,
            reasoning_tokens: 0,
            context_used: 0,
            context_size: 0,
        },
    };
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct fx_tool_output {
    pub text: fx_str,
    pub images: *const fx_image,
    pub image_count: usize,
    pub is_error: bool,
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct fx_header {
    pub name: fx_str,
    pub value: fx_str,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct fx_fetch_request {
    pub fetch: u64,
    pub method: fx_str,
    pub url: fx_str,
    pub headers: *const fx_header,
    pub header_count: usize,
    pub body: fx_bytes,
}

pub type fx_fetch_disposition = i32;
pub const FX_FETCH_RETIRED: fx_fetch_disposition = 0;
pub const FX_FETCH_ACTIVE: fx_fetch_disposition = 1;
pub const FX_FETCH_CONSUMED: fx_fetch_disposition = 2;

/// Declares the C functions, or (without the core) stubs of them.
macro_rules! abi {
    ($( fn $name:ident ( $( $arg:ident : $ty:ty ),* ) -> $ret:ty = $unavailable:expr ; )*) => {
        #[cfg(not(aero_core = "unavailable"))]
        unsafe extern "C" {
            $( pub fn $name( $( $arg : $ty ),* ) -> $ret; )*
        }
        $(
            #[cfg(aero_core = "unavailable")]
            #[allow(unused_variables, clippy::missing_safety_doc)]
            pub unsafe fn $name( $( $arg : $ty ),* ) -> $ret { $unavailable }
        )*
    };
}

abi! {
    fn fx_core_abi_version() -> u32 = 0;
    fn fx_runtime_create(cfg: *const fx_config, host: *const fx_host, host_ctx: *mut c_void, out: *mut fx_runtime) -> fx_status = FX_E_UNAVAILABLE;
    fn fx_runtime_destroy(rt: fx_runtime) -> fx_status = FX_E_UNAVAILABLE;
    fn fx_runtime_ready_fd(rt: fx_runtime) -> i32 = -1;
    fn fx_runtime_info_get(rt: fx_runtime, out: *mut fx_runtime_info) -> fx_status = FX_E_UNAVAILABLE;
    fn fx_session_open(rt: fx_runtime, args: *const fx_session_open_args, request: *mut u64) -> fx_status = FX_E_UNAVAILABLE;
    fn fx_turn_prompt(rt: fx_runtime, input: *const fx_input, message_id: fx_str, turn: *mut u64) -> fx_status = FX_E_UNAVAILABLE;
    fn fx_turn_steer(rt: fx_runtime, text: fx_str, message_id: fx_str, request: *mut u64) -> fx_status = FX_E_UNAVAILABLE;
    fn fx_turn_withdraw(rt: fx_runtime, message_id: fx_str, request: *mut u64) -> fx_status = FX_E_UNAVAILABLE;
    fn fx_turn_cancel(rt: fx_runtime) -> fx_status = FX_E_UNAVAILABLE;
    fn fx_checkpoint(rt: fx_runtime, request: *mut u64) -> fx_status = FX_E_UNAVAILABLE;
    fn fx_events_drain(rt: fx_runtime, buf: *mut fx_event, cap: usize, count: *mut usize) -> fx_status = FX_E_UNAVAILABLE;
    fn fx_tool_result(rt: fx_runtime, call: u64, out: *const fx_tool_output) -> fx_status = FX_E_UNAVAILABLE;
    fn fx_fetch_take(rt: fx_runtime, out: *mut fx_fetch_request) -> fx_status = FX_E_UNAVAILABLE;
    fn fx_fetch_status(rt: fx_runtime, fetch: u64, status: u16, retry_after_s: i32) -> fx_status = FX_E_UNAVAILABLE;
    fn fx_fetch_push(rt: fx_runtime, fetch: u64, data: *const u8, len: usize) -> fx_status = FX_E_UNAVAILABLE;
    fn fx_fetch_finish(rt: fx_runtime, fetch: u64, ok: bool) -> fx_status = FX_E_UNAVAILABLE;
    fn fx_fetch_disposition_get(rt: fx_runtime, fetch: u64) -> fx_fetch_disposition = FX_FETCH_RETIRED;
    fn fx_journal_ack(rt: fx_runtime, append: u64) -> fx_status = FX_E_UNAVAILABLE;
    fn fx_journal_fail(rt: fx_runtime, append: u64, reason: fx_str, fenced: bool) -> fx_status = FX_E_UNAVAILABLE;
    fn fx_hook_answer(rt: fx_runtime, question: u64, kind: fx_hook_answer_kind, text: fx_str) -> fx_status = FX_E_UNAVAILABLE;
}
