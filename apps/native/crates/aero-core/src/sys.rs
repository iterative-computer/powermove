//! The C ABI of `apps/native/vendor/fx/include/fx_core.h` (ABI 2), declared
//! by hand. Every type here is `repr(C)` and matches the header field for
//! field (the layout test in `lib.rs` pins each size); the header is the
//! contract, with its threading, ownership and lease rules.
//!
//! Built without the core (`cfg(aero_core = "unavailable")`), the functions
//! are Rust stubs that return `FX_E_UNAVAILABLE` or the function's sentinel.
//! The stub is Rust only: there is no C stub library.

#![allow(non_camel_case_types, dead_code)]

use std::ffi::c_void;

pub const FX_CORE_ABI_VERSION: u32 = 2;
pub const FX_BUSY_BOUND_MS: u32 = 250;

pub type fx_status = i32;
pub const FX_OK: fx_status = 0;
pub const FX_EMPTY: fx_status = 1;
pub const FX_STALE: fx_status = 2;
pub const FX_BACKPRESSURE: fx_status = 3;
pub const FX_BUSY: fx_status = 4;
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
pub const FX_E_TOO_LARGE: fx_status = -11;

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

pub const FX_MAX_RUNTIMES: u32 = 64;
pub const FX_MAX_PROMPT_TEXT: usize = 4 << 20;
pub const FX_MAX_STEER_TEXT: usize = 64 << 10;
pub const FX_MAX_INPUT_QUEUE: usize = 8 << 20;
pub const FX_MAX_FETCH_CHUNK: usize = 8 << 20;
pub const FX_MAX_CONCURRENT_TOOLS: u32 = 16;

pub const FX_TOOL_IDEMPOTENT: u32 = 1 << 0;
pub const FX_TOOL_WRITES: u32 = 1 << 1;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct fx_tool_call {
    pub struct_size: u32,
    pub flags: u32,
    pub call: u64,
    pub turn: u64,
    pub name: fx_str,
    pub input_json: fx_str,
    pub model_call_id: fx_str,
}

pub const FX_APPEND_BARRIER: u32 = 1 << 0;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct fx_journal_append {
    pub struct_size: u32,
    pub flags: u32,
    pub append: u64,
    pub turn: u64,
    pub first_seq: u64,
    pub last_seq: u64,
    pub events_json: fx_str,
}

pub type fx_hook_kind = u32;
pub const FX_HOOK_PRE_TOOL: fx_hook_kind = 1;
pub const FX_HOOK_STOP: fx_hook_kind = 2;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct fx_hook_question {
    pub struct_size: u32,
    pub kind: fx_hook_kind,
    pub question: u64,
    pub turn: u64,
    pub call: u64,
    pub name: fx_str,
    pub input_json: fx_str,
    pub final_text: fx_str,
    pub continuations: u32,
    pub reserved: u32,
}

pub type fx_hook_verdict = i32;
pub const FX_HOOK_ALLOW: fx_hook_verdict = 0;
pub const FX_HOOK_PENDING: fx_hook_verdict = 1;

pub type fx_hook_answer_kind = i32;
pub const FX_ANSWER_ALLOW: fx_hook_answer_kind = 0;
pub const FX_ANSWER_BLOCK: fx_hook_answer_kind = 1;
pub const FX_ANSWER_REWRITE: fx_hook_answer_kind = 2;
pub const FX_ANSWER_CONTINUE: fx_hook_answer_kind = 3;

pub type ToolCallFn = unsafe extern "C" fn(*mut c_void, *const fx_tool_call);
pub type ToolCancelFn = unsafe extern "C" fn(*mut c_void, u64, u64);
pub type JournalAppendFn = unsafe extern "C" fn(*mut c_void, *const fx_journal_append);
pub type HookFn = unsafe extern "C" fn(*mut c_void, *const fx_hook_question) -> fx_hook_verdict;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct fx_host {
    pub struct_size: u32,
    pub reserved: u32,
    pub tool_call: Option<ToolCallFn>,
    pub tool_cancel: Option<ToolCancelFn>,
    pub journal_append: Option<JournalAppendFn>,
    pub hook: Option<HookFn>,
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
    pub flags: u32,
}

pub type fx_runtime_state = i32;
pub const FX_RUNTIME_RUNNING: fx_runtime_state = 0;
pub const FX_RUNTIME_EXITED: fx_runtime_state = 1;
pub const FX_RUNTIME_FAILED: fx_runtime_state = 2;
pub const FX_RUNTIME_PANICKED: fx_runtime_state = 3;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct fx_runtime_info {
    pub struct_size: u32,
    pub state: fx_runtime_state,
    pub exit_code: i32,
    pub reserved: u32,
    pub message: fx_str,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct fx_named_bytes {
    pub name: fx_str,
    pub bytes: fx_bytes,
}

pub const FX_OPEN_INPUTS_DURABLE: u32 = 1 << 0;
pub const FX_OPEN_IMAGE_STORE: u32 = 1 << 1;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct fx_session_open_args {
    pub struct_size: u32,
    pub flags: u32,
    pub session_id: fx_str,
    pub journal_json: fx_str,
    pub checkpoint: fx_bytes,
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
pub const FX_EVENT_STEER_RETURNED: fx_event_kind = 16;
pub const FX_EVENT_RECOVERY: fx_event_kind = 17;

pub const FX_OPENED_JOURNALED: u32 = 1 << 0;
pub const FX_OPENED_RESUMABLE: u32 = 1 << 1;
pub const FX_OPENED_YIELDED: u32 = 1 << 2;
pub const FX_TURN_REPEATED: u32 = 1 << 0;
pub const FX_TURN_RESUMED: u32 = 1 << 1;
pub const FX_EXITED_EVENTS_LOST: u32 = 1 << 0;

pub const FX_RECOVERY_OPEN_TURN: i32 = 1;
pub const FX_RECOVERY_LAST_TURN: i32 = 2;
pub const FX_RECOVERY_CALL: i32 = 3;
pub const FX_RECOVERY_FOLLOW_UP: i32 = 4;

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

pub type fx_fetch_purpose = u32;
pub const FX_FETCH_MODEL: fx_fetch_purpose = 1;
pub const FX_FETCH_CATALOG: fx_fetch_purpose = 2;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct fx_fetch_request {
    pub struct_size: u32,
    pub purpose: fx_fetch_purpose,
    pub fetch: u64,
    pub turn: u64,
    pub provider: fx_str,
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
pub const FX_FETCH_BUSY: fx_fetch_disposition = 3;

pub const FX_FAULT_FETCH_LOCKED: u32 = 1;
pub const FX_FAULT_EVENT_LOCKED: u32 = 2;
pub const FX_FAULT_STREAMING: u32 = 3;
pub const FX_FAULT_TOOL_RESULT: u32 = 4;
pub const FX_FAULT_JOURNAL_APPEND: u32 = 5;
pub const FX_FAULT_TOOL_CALL: u32 = 6;
pub const FX_FAULT_INPUT_LOCKED: u32 = 7;
pub const FX_FAULT_HOST_TRANSPORT: u32 = 8;
pub const FX_FAULT_CREATE: u32 = 9;
pub const FX_FAULT_DESTROY: u32 = 10;
pub const FX_FAULT_DRAIN_OOM: u32 = 11;
pub const FX_FAULT_TAKE_OOM: u32 = 12;
pub const FX_FAULT_REPLY_OOM: u32 = 13;

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
    fn fx_session_stage_images(rt: fx_runtime, images: *const fx_named_bytes, count: usize, request: *mut u64) -> fx_status = FX_E_UNAVAILABLE;
    fn fx_turn_prompt(rt: fx_runtime, input: *const fx_input, message_id: fx_str, turn: *mut u64) -> fx_status = FX_E_UNAVAILABLE;
    fn fx_turn_resume(rt: fx_runtime, turn: *mut u64) -> fx_status = FX_E_UNAVAILABLE;
    fn fx_turn_steer(rt: fx_runtime, turn: u64, input: *const fx_input, message_id: fx_str, request: *mut u64) -> fx_status = FX_E_UNAVAILABLE;
    fn fx_turn_withdraw(rt: fx_runtime, turn: u64, message_id: fx_str, request: *mut u64) -> fx_status = FX_E_UNAVAILABLE;
    fn fx_turn_cancel(rt: fx_runtime, turn: u64) -> fx_status = FX_E_UNAVAILABLE;
    fn fx_checkpoint(rt: fx_runtime, request: *mut u64) -> fx_status = FX_E_UNAVAILABLE;
    fn fx_events_drain(rt: fx_runtime, buf: *mut fx_event, cap: usize, event_size: usize, count: *mut usize) -> fx_status = FX_E_UNAVAILABLE;
    fn fx_tool_result(rt: fx_runtime, call: u64, out: *const fx_tool_output) -> fx_status = FX_E_UNAVAILABLE;
    fn fx_fetch_take(rt: fx_runtime, out: *mut fx_fetch_request) -> fx_status = FX_E_UNAVAILABLE;
    fn fx_fetch_status(rt: fx_runtime, fetch: u64, status: u16, retry_after_s: i32) -> fx_status = FX_E_UNAVAILABLE;
    fn fx_fetch_push(rt: fx_runtime, fetch: u64, data: *const u8, len: usize) -> fx_status = FX_E_UNAVAILABLE;
    fn fx_fetch_finish(rt: fx_runtime, fetch: u64, ok: bool) -> fx_status = FX_E_UNAVAILABLE;
    fn fx_fetch_disposition_get(rt: fx_runtime, fetch: u64) -> fx_fetch_disposition = FX_FETCH_RETIRED;
    fn fx_journal_ack(rt: fx_runtime, append: u64) -> fx_status = FX_E_UNAVAILABLE;
    fn fx_journal_fail(rt: fx_runtime, append: u64, reason: fx_str, fenced: bool) -> fx_status = FX_E_UNAVAILABLE;
    fn fx_hook_answer(rt: fx_runtime, question: u64, kind: fx_hook_answer_kind, text: fx_str) -> fx_status = FX_E_UNAVAILABLE;
    fn fx_test_arm_fault(rt: fx_runtime, point: u32, skip: u32) -> fx_status = FX_E_UNAVAILABLE;
    fn fx_test_set_slot_generation(slot: u32, generation: u64) -> fx_status = FX_E_UNAVAILABLE;
}
