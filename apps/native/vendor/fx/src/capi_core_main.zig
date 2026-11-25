//! Powermove notice: new file, not in upstream fx (patches P-CAPI, P-PANIC,
//! P-TOOLS, P-EVENTS, P-IMAGES, P-STOP, P-PRETOOL). The C ABI entry of the libfx core, beside upstream's N-API and
//! WebAssembly entries. It implements include/fx_core.h (ABI 2). The
//! runtime, input queue, attachment table, fetch bridge and runtime slots are
//! the N-API entry's, extracted verbatim into libfx_core_runtime.zig; the
//! Node-API surface is replaced. The host is Rust (crate pm-aero-core), which
//! takes the place of sdk/engine.js.
//!
//! The core still speaks ACP JSON-RPC for sessions and turns: the host's
//! calls become frames on the input queue, and the core's answers come back
//! through its writer callback. What a turn streams (text, reasoning, tool
//! cards, usage, a joined steer) never becomes JSON: the core hands it to the
//! typed sink (P-EVENTS, `sink*` below), and the writer drops session/update
//! frames unparsed. Host tool calls bypass ACP (P-TOOLS): the core calls
//! `bridgeToolCall` with the model's input in place, and the host's answer is
//! copied once into the call's record. Images cross as raw bytes through the
//! attachment table (prompts, staging) or straight into the tool result
//! (P-IMAGES); the stop gate and pre_tool are lifecycle hook handlers the
//! core calls where fx's own Stop and PreToolUse hooks run (P-STOP,
//! P-PRETOOL).
//!
//! Liveness (header rules 2 and 12). Host calls take the locks core threads
//! share only through `hostLock`, which gives up when the runtime panics or
//! after the busy bound. Every host call runs inside `guardedCall`, so a panic
//! on the host's thread unwinds to the export and returns FX_E_FAILED. A
//! panic on a core thread marks the runtime PANICKED and parks the thread;
//! the runtime's other threads park at their next boundary with the host.
const std = @import("std");
const builtin = @import("builtin");
const core = @import("libfx_core_runtime.zig");
const panic_context = @import("core/shared/panic_context.zig");
const host_tool_bridge = @import("core/tooling/host_tool_bridge.zig");
const tool_dispatch = @import("core/tooling/tool_dispatch.zig");
const host_event_sink = @import("core/hosts/host_event_sink.zig");
const host_hooks = @import("core/hooks/host_hooks.zig");
const hook_runtime = @import("core/hooks/runtime.zig");
const hook_defs = @import("core/hooks/definitions.zig");
const image_data = @import("core/images/image_data.zig");
const session_codec = @import("core/session/session_codec.zig");
const core_types = @import("core/shared/types.zig");
const acp_server = core.acp_server;
const jsonrpc = core.jsonrpc;
const io_mod = core.io_mod;
const fetch_state = core.fetch_state;
const builtin_gateway = core.builtin_gateway;
const host_stream_provider = core.host_stream_provider;
const host_chat_completions = core.host_chat_completions;
const provider_set = core.provider_set;
const model_provider = core.model_provider;

const Allocator = std.mem.Allocator;
const alloc_c = std.heap.c_allocator;

pub const abi_version: u32 = 2;

/// The root's panic handler (P-PANIC): every safety panic of the core comes
/// here instead of aborting the process.
pub const panic = std.debug.FullPanic(onPanic);

// ---------------------------------------------------------------------------
// C types (must match include/fx_core.h)
// ---------------------------------------------------------------------------

pub const Status = i32;
const FX_OK: Status = 0;
const FX_EMPTY: Status = 1;
const FX_STALE: Status = 2;
const FX_BACKPRESSURE: Status = 3;
const FX_BUSY: Status = 4;
const FX_E_INVALID: Status = -1;
const FX_E_CLOSED: Status = -2;
const FX_E_FAILED: Status = -3;
const FX_E_LIMIT: Status = -4;
const FX_E_OOM: Status = -5;
const FX_E_THREAD: Status = -6;
const FX_E_IO: Status = -7;
const FX_E_STATE: Status = -8;
const FX_E_UNSUPPORTED: Status = -10;
const FX_E_TOO_LARGE: Status = -11;

pub const Str = extern struct {
    ptr: ?[*]const u8 = null,
    len: usize = 0,

    fn of(bytes: []const u8) Str {
        return .{ .ptr = if (bytes.len == 0) null else bytes.ptr, .len = bytes.len };
    }

    /// The bytes, or null for a NULL pointer with a non-zero length.
    fn slice(self: Str) ?[]const u8 {
        if (self.len == 0) return "";
        const ptr = self.ptr orelse return null;
        return ptr[0..self.len];
    }
};

pub const Bytes = extern struct {
    ptr: ?[*]const u8 = null,
    len: usize = 0,

    fn of(bytes: []const u8) Bytes {
        return .{ .ptr = if (bytes.len == 0) null else bytes.ptr, .len = bytes.len };
    }

    fn slice(self: Bytes) ?[]const u8 {
        if (self.len == 0) return "";
        const ptr = self.ptr orelse return null;
        return ptr[0..self.len];
    }
};

const FX_TOOL_IDEMPOTENT: u32 = 1 << 0;
const FX_TOOL_WRITES: u32 = 1 << 1;

pub const ToolCallInfo = extern struct {
    struct_size: u32 = @sizeOf(ToolCallInfo),
    flags: u32,
    call: u64,
    turn: u64,
    name: Str,
    input_json: Str,
    model_call_id: Str,
};

const FX_APPEND_BARRIER: u32 = 1 << 0;

pub const JournalAppendInfo = extern struct {
    struct_size: u32 = @sizeOf(JournalAppendInfo),
    flags: u32,
    append: u64,
    turn: u64,
    first_seq: u64,
    last_seq: u64,
    events_json: Str,
};

pub const HookQuestion = extern struct {
    struct_size: u32 = @sizeOf(HookQuestion),
    kind: u32,
    question: u64,
    turn: u64,
    call: u64,
    name: Str,
    input_json: Str,
    final_text: Str,
    continuations: u32,
    reserved: u32,
};

pub const Host = extern struct {
    struct_size: u32 = 0,
    reserved: u32 = 0,
    tool_call: ?*const fn (?*anyopaque, *const ToolCallInfo) callconv(.c) void = null,
    tool_cancel: ?*const fn (?*anyopaque, u64, u64) callconv(.c) void = null,
    journal_append: ?*const fn (?*anyopaque, *const JournalAppendInfo) callconv(.c) void = null,
    hook: ?*const fn (?*anyopaque, *const HookQuestion) callconv(.c) i32 = null,
};

pub const ToolDesc = extern struct {
    name: Str,
    description: Str,
    input_schema_json: Str,
    flags: u32,
    reserved: u32,
};

pub const Config = extern struct {
    struct_size: u32,
    abi_version: u32,
    provider: Str,
    model: Str,
    effort: Str,
    configured_providers_json: Str,
    endpoint_url: Str,
    instructions: Str,
    home: Str,
    workspace_root: Str,
    tools: ?[*]const ToolDesc,
    tool_count: usize,
    max_agent_steps: u32,
    max_concurrent_tools: u32,
    max_tool_result_bytes: u32,
    max_history_turns: u32,
    max_stop_continuations: u32,
    flags: u32,
};

const FX_RUNTIME_RUNNING: i32 = 0;
const FX_RUNTIME_EXITED: i32 = 1;
const FX_RUNTIME_FAILED: i32 = 2;
const FX_RUNTIME_PANICKED: i32 = 3;

pub const RuntimeInfo = extern struct {
    struct_size: u32,
    state: i32,
    exit_code: i32,
    reserved: u32,
    message: Str,
};

pub const NamedBytes = extern struct {
    name: Str,
    bytes: Bytes,
};

const FX_OPEN_INPUTS_DURABLE: u32 = 1 << 0;
const FX_OPEN_IMAGE_STORE: u32 = 1 << 1;

pub const SessionOpenArgs = extern struct {
    struct_size: u32,
    flags: u32,
    session_id: Str,
    journal_json: Str,
    checkpoint: Bytes,
};

pub const Image = extern struct {
    mime: Str,
    data: Bytes,
};

pub const Input = extern struct {
    text: Str,
    images: ?[*]const Image,
    image_count: usize,
};

pub const Usage = extern struct {
    input_tokens: u64 = 0,
    output_tokens: u64 = 0,
    cache_read_tokens: u64 = 0,
    cache_write_tokens: u64 = 0,
    reasoning_tokens: u64 = 0,
    context_used: u64 = 0,
    context_size: u64 = 0,
};

pub const Event = extern struct {
    kind: u32 = 0,
    flags: u32 = 0,
    turn: u64 = 0,
    request: u64 = 0,
    id: u64 = 0,
    status: Status = 0,
    code: i32 = 0,
    text: Str = .{},
    text2: Str = .{},
    bytes: Bytes = .{},
    names: ?[*]const Str = null,
    name_count: usize = 0,
    usage: Usage = .{},
};

pub const ToolOutput = extern struct {
    text: Str,
    images: ?[*]const Image,
    image_count: usize,
    is_error: bool,
};

pub const Header = extern struct {
    name: Str,
    value: Str,
};

const FX_FETCH_MODEL: u32 = 1;
const FX_FETCH_CATALOG: u32 = 2;
const FX_FETCH_BUSY: i32 = 3;

pub const FetchRequest = extern struct {
    struct_size: u32,
    purpose: u32,
    fetch: u64,
    turn: u64,
    provider: Str,
    method: Str,
    url: Str,
    headers: ?[*]const Header,
    header_count: usize,
    body: Bytes,
};

comptime {
    // The header's FX_*_SIZE_V2 values.
    std.debug.assert(@sizeOf(ToolCallInfo) == 72);
    std.debug.assert(@sizeOf(JournalAppendInfo) == 56);
    std.debug.assert(@sizeOf(HookQuestion) == 88);
    std.debug.assert(@sizeOf(Host) == 40);
    std.debug.assert(@sizeOf(Config) == 176);
    std.debug.assert(@sizeOf(RuntimeInfo) == 32);
    std.debug.assert(@sizeOf(SessionOpenArgs) == 56);
    std.debug.assert(@sizeOf(Event) == 160);
    std.debug.assert(@sizeOf(FetchRequest) == 104);
    std.debug.assert(@sizeOf(ToolDesc) == 56);
    std.debug.assert(@sizeOf(Image) == 32);
    std.debug.assert(@sizeOf(Input) == 32);
    std.debug.assert(@sizeOf(ToolOutput) == 40);
}

const EventKind = enum(u32) {
    session_opened = 1,
    images_needed = 2,
    request_done = 3,
    turn_start = 4,
    text_delta = 5,
    thought_delta = 6,
    steer_joined = 7,
    tool_start = 8,
    tool_end = 9,
    usage = 10,
    compacted = 11,
    turn_end = 12,
    checkpoint = 13,
    @"error" = 14,
    exited = 15,
    steer_returned = 16,
    recovery = 17,
};

const FX_OPENED_JOURNALED: u32 = 1 << 0;
const FX_OPENED_RESUMABLE: u32 = 1 << 1;
const FX_OPENED_YIELDED: u32 = 1 << 2;
const FX_TURN_REPEATED: u32 = 1 << 0;
const FX_TURN_RESUMED: u32 = 1 << 1;
const FX_EXITED_EVENTS_LOST: u32 = 1 << 0;

const FX_RECOVERY_OPEN_TURN: i32 = 1;
const FX_RECOVERY_LAST_TURN: i32 = 2;
const FX_RECOVERY_CALL: i32 = 3;
const FX_RECOVERY_FOLLOW_UP: i32 = 4;

const FX_HOOK_PRE_TOOL: u32 = 1;
const FX_HOOK_STOP: u32 = 2;
const FX_HOOK_ALLOW: i32 = 0;
const FX_HOOK_PENDING: i32 = 1;
const FX_ANSWER_ALLOW: i32 = 0;
const FX_ANSWER_BLOCK: i32 = 1;
const FX_ANSWER_REWRITE: i32 = 2;
const FX_ANSWER_CONTINUE: i32 = 3;

const StopReason = enum(i32) {
    end_turn = 0,
    max_tokens = 1,
    max_steps = 2,
    refusal = 3,
    cancelled = 4,
    yielded = 5,
    @"error" = 6,

    fn fromAcp(text: []const u8) StopReason {
        const map = .{
            .{ "end_turn", StopReason.end_turn },
            .{ "max_output_tokens", StopReason.max_tokens },
            .{ "max_model_turns", StopReason.max_steps },
            .{ "refused", StopReason.refusal },
            .{ "cancelled", StopReason.cancelled },
            .{ "yielded", StopReason.yielded },
        };
        inline for (map) |entry| if (std.mem.eql(u8, text, entry[0])) return entry[1];
        return .@"error";
    }
};

const Fault = enum(u32) {
    fetch_locked = 1,
    event_locked = 2,
    streaming = 3,
    tool_result = 4,
    journal_append = 5,
    tool_call = 6,
    input_locked = 7,
    host_transport = 8,
    create = 9,
    destroy = 10,
    drain_oom = 11,
    take_oom = 12,
    reply_oom = 13,
};
const fault_count = 14;

/// FX_FAULT_CREATE is process-wide: no runtime exists yet when it fires.
var create_fault: std.atomic.Value(i64) = .init(-1);

/// Counts `counter` down; true on the armed hit.
fn faultHit(counter: *std.atomic.Value(i64)) bool {
    if (counter.load(.monotonic) < 0) return false;
    return counter.fetchSub(1, .acq_rel) == 0;
}

// ---------------------------------------------------------------------------
// Limits (the header's FX_MAX_*)
// ---------------------------------------------------------------------------

const max_instructions_bytes = 1024 * 1024;
const max_tools = 256;
const max_tool_text_bytes = 64 * 1024;
const max_schema_bytes = 256 * 1024;
const max_session_id_bytes = 255;
const max_message_id_bytes = 128;
const max_prompt_text_bytes = 4 * 1024 * 1024;
const max_steer_text_bytes = 64 * 1024;
const max_event_bytes = 16 * 1024 * 1024;
const max_queued_events = 65536;
const max_configured_providers_bytes = 1024 * 1024;
const max_tool_result_limit = 16 * 1024 * 1024;
const max_prompt_images = 8;
const max_stage_batch = 8;
const max_stage_batch_bytes = 8 * 1024 * 1024;
const max_image_bytes = 8 * 1024 * 1024;
/// The core's own bounds on what reaches a model (image_attachments.zig,
/// image_data.zig): an image's base64 is at most 5 MiB, a prompt's raw
/// images 6 MiB, a tool result's base64 images 8 MiB. Beyond them a call
/// returns FX_E_TOO_LARGE, as beyond the header's FX_MAX_* limits.
const max_model_image_bytes = image_data.max_encoded_image_bytes / 4 * 3;
const max_prompt_images_bytes = 6 * 1024 * 1024;
/// A hook answer's text: the core's own bounds for a block reason, a
/// continuation and rewritten input (core/hooks/definitions.zig).
const max_hook_reason = hook_defs.Limits.reason_bytes;
const max_hook_context = hook_defs.Limits.context_bytes;
const max_hook_arguments = hook_defs.Limits.arguments_json_bytes;
const max_concurrent_tools_limit = 16;
const busy_bound_ms = 250;
/// How often a core thread waiting on the host re-checks cancellation and
/// the runtime's state.
const pulse: std.Io.Timeout = .{ .duration = .{ .raw = .fromMilliseconds(50), .clock = .awake } };

// ---------------------------------------------------------------------------
// Guarded host calls (P-PANIC): a panic on the host's thread unwinds here
// ---------------------------------------------------------------------------

/// Callee-saved state of a guarded call on aarch64: x19 to x30, sp, d8 to d15.
const JumpBuffer = extern struct { regs: [21]u64 = @splat(0) };

/// Calls `function(context)` and returns its result, or the code a later
/// `unwind` on this thread passes back from anywhere inside that call. The
/// saved state is the caller's at this call, so unwinding resumes the caller
/// exactly as a normal return would.
fn guardedCallImpl() callconv(.naked) noreturn {
    // x0 = function, x1 = context, x2 = buffer.
    asm volatile (
        \\ stp x19, x20, [x2, #0]
        \\ stp x21, x22, [x2, #16]
        \\ stp x23, x24, [x2, #32]
        \\ stp x25, x26, [x2, #48]
        \\ stp x27, x28, [x2, #64]
        \\ stp x29, x30, [x2, #80]
        \\ mov x9, sp
        \\ str x9, [x2, #96]
        \\ stp d8, d9, [x2, #104]
        \\ stp d10, d11, [x2, #120]
        \\ stp d12, d13, [x2, #136]
        \\ stp d14, d15, [x2, #152]
        \\ mov x19, x2
        \\ mov x9, x0
        \\ mov x0, x1
        \\ blr x9
        \\ ldr x30, [x19, #88]
        \\ ldr x19, [x19, #0]
        \\ ret
    );
}

/// Resumes the guarded call that saved `buffer`, returning `code` from it.
fn unwindImpl() callconv(.naked) noreturn {
    // x0 = buffer, x1 = code.
    asm volatile (
        \\ mov x9, x0
        \\ ldp x19, x20, [x9, #0]
        \\ ldp x21, x22, [x9, #16]
        \\ ldp x23, x24, [x9, #32]
        \\ ldp x25, x26, [x9, #48]
        \\ ldp x27, x28, [x9, #64]
        \\ ldp x29, x30, [x9, #80]
        \\ ldr x10, [x9, #96]
        \\ mov sp, x10
        \\ ldp d8, d9, [x9, #104]
        \\ ldp d10, d11, [x9, #120]
        \\ ldp d12, d13, [x9, #136]
        \\ ldp d14, d15, [x9, #152]
        \\ mov w0, w1
        \\ ret
    );
}

const GuardedFn = *const fn (*anyopaque) callconv(.c) i32;
const GuardedCallFn = *const fn (GuardedFn, *anyopaque, *JumpBuffer) callconv(.c) i32;

/// Calls the naked `guardedCallImpl` as an opaque C function. The address
/// goes through an empty asm so the optimiser cannot see the callee: it
/// would otherwise call it directly with its declared `noreturn` type and
/// drop the code after the call.
fn guardedCall(function: GuardedFn, context: *anyopaque, buffer: *JumpBuffer) i32 {
    const address = asm volatile (""
        : [out] "=r" (-> usize),
        : [in] "0" (@intFromPtr(&guardedCallImpl)),
    );
    const call: GuardedCallFn = @ptrFromInt(address);
    return call(function, context, buffer);
}

fn unwind(buffer: *JumpBuffer, code: i32) noreturn {
    const address = asm volatile (""
        : [out] "=r" (-> usize),
        : [in] "0" (@intFromPtr(&unwindImpl)),
    );
    const jump: *const fn (*JumpBuffer, i32) callconv(.c) noreturn = @ptrFromInt(address);
    jump(buffer, code);
}

comptime {
    if (builtin.cpu.arch != .aarch64) @compileError("the C ABI entry's panic guard is written for aarch64");
}

const Guard = struct {
    buffer: JumpBuffer = .{},
    runtime: ?*Runtime = null,
};

threadlocal var tl_guard: ?*Guard = null;
threadlocal var tl_panicking: bool = false;

/// Runs `body(runtime_or_null, args)` as a host call: inside a guard, with
/// the handle entered for the call (when `handle` is not null). A panic in it
/// fails the runtime and returns FX_E_FAILED here. `body` returns a status.
fn hostCall(handle: ?u64, args: anytype, comptime body: anytype) Status {
    const Args = @TypeOf(args);
    const Context = struct {
        guard: Guard = .{},
        handle: ?u64,
        args: Args,
        entered: bool = false,

        fn run(raw: *anyopaque) callconv(.c) i32 {
            const self: *@This() = @ptrCast(@alignCast(raw));
            if (self.handle) |value| {
                const runtime = enter(value) orelse return FX_E_CLOSED;
                self.entered = true;
                self.guard.runtime = runtime;
                return body(runtime, self.args);
            }
            return body(null, self.args);
        }
    };
    var context: Context = .{ .handle = handle, .args = args };
    const saved = tl_guard;
    tl_guard = &context.guard;
    const status = guardedCall(Context.run, &context, &context.guard.buffer);
    tl_guard = saved;
    if (context.entered) leave(handle.?);
    return status;
}

fn onPanic(message: []const u8, first_trace_addr: ?usize) noreturn {
    _ = first_trace_addr;
    if (tl_guard) |guard| {
        // A host call: fail its runtime and return to the export.
        if (!tl_panicking) {
            tl_panicking = true;
            if (guard.runtime) |runtime| runtime.failPanic(message);
            tl_panicking = false;
        }
        unwind(&guard.buffer, FX_E_FAILED);
    }
    if (!tl_panicking) {
        tl_panicking = true;
        if (panic_context.current) |raw| {
            const runtime: *Runtime = @ptrCast(@alignCast(raw));
            runtime.failPanic(message);
        } else {
            failEveryRuntime(message);
        }
    }
    park();
}

/// A panic on a core thread with no runtime (none is known: every thread the
/// core spawns for a runtime inherits its context). Every running runtime is
/// failed, so none stalls without an answer; the process keeps running.
fn failEveryRuntime(message: []const u8) void {
    for (&slots, 0..) |*slot, index| {
        const state = slot.state.load(.acquire);
        if (state & occupied_bit == 0) continue;
        const handle = handleOf(index, state >> gen_shift);
        const runtime = enter(handle) orelse continue;
        runtime.failPanic(message);
        leave(handle);
    }
}

/// Parks the calling core thread for good.
fn park() noreturn {
    while (true) {
        const ts: std.c.timespec = .{ .sec = 3600, .nsec = 0 };
        _ = std.c.nanosleep(&ts, null);
    }
}

fn nowMs() u64 {
    var ts: std.c.timespec = undefined;
    _ = std.c.clock_gettime(.MONOTONIC, &ts);
    return @as(u64, @intCast(ts.sec)) * 1000 + @as(u64, @intCast(ts.nsec)) / 1_000_000;
}

const Acquire = enum { locked, failed, busy };

/// Takes `mutex` for a host call: waits only while its holder runs, gives up
/// when the runtime panics, and after the busy bound (header rule 2).
fn hostLock(runtime: *Runtime, mutex: *std.Io.Mutex) Acquire {
    if (mutex.tryLock()) return .locked;
    var spins: usize = 0;
    var started: u64 = 0;
    while (true) {
        if (runtime.panicked()) return .failed;
        if (mutex.tryLock()) return .locked;
        spins += 1;
        if (spins < 64) {
            std.atomic.spinLoopHint();
        } else {
            std.Thread.yield() catch {};
            if (spins == 64) started = nowMs();
            if (spins % 64 == 0 and nowMs() -| started > busy_bound_ms) return .busy;
        }
    }
}

fn acquireStatus(result: Acquire) Status {
    return switch (result) {
        .locked => FX_OK,
        .failed => FX_E_FAILED,
        .busy => FX_BUSY,
    };
}

// ---------------------------------------------------------------------------
// The typed event queue
// ---------------------------------------------------------------------------

const Span = struct {
    off: u32 = 0,
    len: u32 = 0,
};

const QueuedEvent = struct {
    kind: EventKind,
    flags: u32 = 0,
    turn: u64 = 0,
    request: u64 = 0,
    id: u64 = 0,
    status: Status = 0,
    code: i32 = 0,
    text: Span = .{},
    text2: Span = .{},
    bytes: Span = .{},
    names_start: u32 = 0,
    name_count: u32 = 0,
    usage: Usage = .{},
};

const EventText = struct {
    text: []const u8 = "",
    text2: []const u8 = "",
    bytes: []const u8 = "",
    names: []const []const u8 = &.{},

    fn total(self: EventText) usize {
        var sum = self.text.len + self.text2.len + self.bytes.len;
        for (self.names) |name| sum += name.len;
        return sum;
    }
};

/// How an event waits for room. Deltas wait for the host to drain (bytes and
/// count are bounded); control events (starts, ends, answers) never wait and
/// never meet the bound, because each answers a bounded piece of work.
const EventMode = enum { delta, control };

/// Events the core produced and the host has not drained. Text lives in one
/// byte arena per side: the writer appends into `pending_bytes`; a drain that
/// takes everything swaps the arenas, so what it hands out stays put until the
/// next drain and no byte is copied twice.
const EventQueue = struct {
    mutex: std.Io.Mutex = .init,
    space: std.Io.Condition = .init,
    events: std.ArrayList(QueuedEvent) = .empty,
    pending_bytes: std.ArrayList(u8) = .empty,
    pending_names: std.ArrayList(Span) = .empty,
    drained_bytes: std.ArrayList(u8) = .empty,
    drained_names: std.ArrayList(Span) = .empty,
    drained_name_strs: std.ArrayList(Str) = .empty,
    /// Where a drain builds what it hands out before it can no longer fail;
    /// never handed out itself, so a failed drain leaves the previous
    /// drain's borrows and the queue as they were (Sol's 2a item 5).
    spare_bytes: std.ArrayList(u8) = .empty,
    spare_names: std.ArrayList(Span) = .empty,
    spare_name_strs: std.ArrayList(Str) = .empty,
    closed: bool = false,

    fn deinit(self: *EventQueue) void {
        self.events.deinit(alloc_c);
        self.pending_bytes.deinit(alloc_c);
        self.pending_names.deinit(alloc_c);
        self.drained_bytes.deinit(alloc_c);
        self.drained_names.deinit(alloc_c);
        self.drained_name_strs.deinit(alloc_c);
        self.spare_bytes.deinit(alloc_c);
        self.spare_names.deinit(alloc_c);
        self.spare_name_strs.deinit(alloc_c);
    }

    fn full(self: *const EventQueue, total: usize) bool {
        if (self.events.items.len == 0) return false;
        return self.events.items.len >= max_queued_events or self.pending_bytes.items.len + total > max_event_bytes;
    }

    /// Appends with the lock held. Returns whether the queue was empty.
    fn appendLocked(self: *EventQueue, event: QueuedEvent, data: EventText) error{OutOfMemory}!bool {
        const mark_bytes = self.pending_bytes.items.len;
        const mark_names = self.pending_names.items.len;
        errdefer {
            self.pending_bytes.items.len = mark_bytes;
            self.pending_names.items.len = mark_names;
        }
        var queued = event;
        queued.text = try self.append(data.text);
        queued.text2 = try self.append(data.text2);
        queued.bytes = try self.append(data.bytes);
        if (data.names.len > 0) {
            queued.names_start = @intCast(self.pending_names.items.len);
            queued.name_count = @intCast(data.names.len);
            for (data.names) |name| try self.pending_names.append(alloc_c, try self.append(name));
        }
        const was_empty = self.events.items.len == 0;
        try self.events.append(alloc_c, queued);
        return was_empty;
    }

    fn append(self: *EventQueue, bytes: []const u8) error{OutOfMemory}!Span {
        if (bytes.len == 0) return .{};
        const off = self.pending_bytes.items.len;
        try self.pending_bytes.appendSlice(alloc_c, bytes);
        return .{ .off = @intCast(off), .len = @intCast(bytes.len) };
    }

    fn spanStr(arena: []const u8, span: Span) Str {
        if (span.len == 0) return .{};
        return .{ .ptr = arena[span.off..].ptr, .len = span.len };
    }
};

// ---------------------------------------------------------------------------
// The runtime
// ---------------------------------------------------------------------------

const Pending = struct {
    kind: Kind,
    request: u64,
    /// Inbound attachments the frame names; dropped when it is answered, so
    /// a frame the core refused leaves nothing in the table.
    attachments: Attachments = .{},

    const Kind = enum { initialize, libfx_new, journal_open, prompt, steer, withdraw, handoff, journal_images, snapshot };
};

const Attachments = struct {
    ids: [max_prompt_images]u32 = @splat(0),
    count: u8 = 0,

    fn slice(self: *const Attachments) []const u32 {
        return self.ids[0..self.count];
    }
};

/// A hook question the core waits on (P-STOP, P-PRETOOL); it lives on the
/// stack of the core thread that asked, in `questions` until answered.
const HookRecord = struct {
    id: u64,
    turn: u64,
    kind: u32,
    answered: bool = false,
    answer: i32 = FX_ANSWER_ALLOW,
    text: ?[]u8 = null,
};

/// A host tool call the core waits on; it lives on the stack of the core
/// thread that made it, in `calls` until that thread is done with it.
const ToolRecord = struct {
    id: u64,
    turn: u64,
    writes: bool,
    /// The host's answer: text copied once by fx_tool_result, and its images
    /// (raw bytes, base64-encoded once into the core's own form, P-IMAGES).
    result: ?[]u8 = null,
    images: ?[]core_types.ToolImage = null,
    is_error: bool = false,
    answered: bool = false,
    cancel_requested: bool = false,
};

const Steer = struct {
    request: u64,
    message_id: []u8,
    /// A steer with images: it never joins; the turn hands off at its next
    /// safe point and the steer comes back (P-IMAGES).
    handoff: bool = false,
};

const Tool = struct {
    name: []u8,
    flags: u32,
};

const message_capacity = 1024;

/// A message written once, by whichever writer claims it first, read without
/// a lock after `len` is published.
const Message = struct {
    claimed: std.atomic.Value(bool) = .init(false),
    len: std.atomic.Value(usize) = .init(0),
    bytes: [message_capacity]u8 = undefined,

    fn set(self: *Message, parts: []const []const u8) void {
        if (self.claimed.swap(true, .acq_rel)) return;
        var len: usize = 0;
        for (parts) |part| {
            const room = message_capacity - len;
            const take = @min(room, part.len);
            @memcpy(self.bytes[len..][0..take], part[0..take]);
            len += take;
        }
        self.len.store(len, .release);
    }

    fn get(self: *const Message) []const u8 {
        return self.bytes[0..self.len.load(.acquire)];
    }
};

const Runtime = struct {
    alloc: Allocator,
    // As the N-API entry's runtime.
    fetch: core.FetchBridge = .{},
    attachments: core.AttachmentTable = .{},
    stream_context: host_stream_provider.ProviderContext = undefined,
    codex_context: core.host_openai_codex.Context = undefined,
    grok_context: core.host_xai_grok.Context = undefined,
    custom_context: ?host_chat_completions.Context = null,
    input: core.InputQueue = .{},
    provider: ?model_provider.ProviderId,
    provider_name: []u8,
    configured_providers: ?[]u8,
    model: ?[]u8,
    effort: ?[]u8,
    home: []u8,
    workspace_root: []u8,
    gateway_chat_url: []u8,
    max_agent_steps: usize,
    max_concurrent_tools: usize,
    max_tool_result_bytes: usize,
    max_history_turns: usize,
    max_stop_continuations: u32,
    ready: core.ReadyNotifier,
    thread: std.Thread = undefined,

    // Lock-free state (header rules 2 and 12).
    state: std.atomic.Value(i32) = .init(FX_RUNTIME_RUNNING),
    exit_code: std.atomic.Value(i32) = .init(0),
    fail_message: Message = .{},
    panic_message: Message = .{},
    /// fx_runtime_destroy has started: no callback starts any more.
    closing: std.atomic.Value(bool) = .init(false),
    /// Callbacks running now.
    callbacks: std.atomic.Value(u32) = .init(0),
    /// The core thread is done with the runtime and can be joined.
    thread_done: std.atomic.Value(bool) = .init(false),
    /// A host call met FX_BACKPRESSURE on the input queue or the fetch
    /// response queue; the core wakes the host when it frees room.
    input_blocked: std.atomic.Value(bool) = .init(false),
    push_blocked: std.atomic.Value(bool) = .init(false),
    exited_delivered: std.atomic.Value(bool) = .init(false),
    faults: [fault_count]std.atomic.Value(i64) = @splat(.init(-1)),

    // The C entry's own.
    host: Host,
    host_ctx: ?*anyopaque,
    tools: []Tool,
    initialize_frame: []u8,
    events: EventQueue = .{},
    /// Guards everything below. Never held while a host callback runs.
    lock: std.Io.Mutex = .init,
    /// Signalled when a tool call is answered, cancelled or done.
    calls_changed: std.Io.Condition = .init,
    next_rpc_id: i64 = 1,
    next_request: u64 = 1,
    next_call: u64 = 1,
    pending: std.AutoHashMapUnmanaged(i64, Pending) = .empty,
    session_id: ?[]u8 = null,
    /// `created`: the core holds the session but its journal is not open (an
    /// open that met missing images, or a refused journal): open again.
    session: enum { none, opening, created, open } = .none,
    journaled: bool = false,
    /// The journal_open fx_session_open asked for, sent once libfx/new answers.
    open_params: ?[]u8 = null,
    open_attachments: Attachments = .{},
    open_request: u64 = 0,
    /// The session's journal names stored images (FX_OPEN_IMAGE_STORE).
    image_store: bool = false,
    /// Recovery (header, "Open versus resume"): the turn a crash left open,
    /// and the message id of the turn that ended last.
    resumable: bool = false,
    open_turn_id: ?[]u8 = null,
    last_turn_id: ?[]u8 = null,
    /// The running turn's message id.
    turn_message_id: ?[]u8 = null,
    next_attachment: u32 = 1,
    /// P-EVENTS: tool card names by ACP call id, for TOOL_END.
    tool_names: std.StringHashMapUnmanaged([]u8) = .empty,
    /// P-STOP, P-PRETOOL: questions waiting for fx_hook_answer.
    questions: std.AutoHashMapUnmanaged(u64, *HookRecord) = .empty,
    next_question: u64 = 1,
    /// The stop gate's asks of the running turn.
    stop_turn: u64 = 0,
    stop_asks: u32 = 0,
    /// Answers handed to the core's hook dispatch, which copies them; freed
    /// once the turn has ended.
    hook_texts: std.ArrayList([]u8) = .empty,
    /// Tool call ids pre_tool asked about, by model call id: the call it
    /// allows is made with the id its question named.
    reserved_calls: std.StringHashMapUnmanaged(u64) = .empty,
    /// The running turn, or 0. It stays set until the turn's TURN_END is
    /// queued, so no later turn's events can come first.
    turn: u64 = 0,
    calls: std.AutoHashMapUnmanaged(u64, *ToolRecord) = .empty,
    calls_outstanding: usize = 0,
    writer_outstanding: bool = false,
    steers: std.ArrayList(Steer) = .empty,
    journal_sent: u64 = 0,
    journal_acked: u64 = 0,
    journal_failed: ?[]u8 = null,
    flushes: std.ArrayList(Flush) = .empty,
    /// The running turn fx_turn_cancel cancelled, or 0. A fetch the core
    /// publishes for it afterwards is published retired.
    cancelled_turn: u64 = 0,
    /// Guarded by `fetch.mutex`, not `lock`: the turn the published fetch
    /// serves, set in the same critical section that publishes it.
    fetch_owner: u64 = 0,
    /// Partial frame bytes, if the core's writer ever splits a frame.
    line: std.ArrayList(u8) = .empty,
    /// What the last fx_fetch_take handed out.
    taken: std.heap.ArenaAllocator = .init(alloc_c),
    /// Where a take builds its answer; swapped with `taken` only once the
    /// take can no longer fail, so a failed take keeps the previous borrows.
    taking: std.heap.ArenaAllocator = .init(alloc_c),

    const Flush = struct {
        rpc_id: i64,
        upto: u64,
    };

    fn panicked(self: *const Runtime) bool {
        return self.state.load(.acquire) == FX_RUNTIME_PANICKED;
    }

    fn running(self: *const Runtime) bool {
        return self.state.load(.acquire) == FX_RUNTIME_RUNNING;
    }

    /// The panic handler's part, on any thread: no lock, no allocation.
    fn failPanic(self: *Runtime, message: []const u8) void {
        self.panic_message.set(&.{ "core panic: ", message });
        self.state.store(FX_RUNTIME_PANICKED, .release);
        // Wake the host and every core thread waiting on a pulse.
        self.ready.notify();
    }

    /// The C entry stops the core: an event or frame could not be delivered,
    /// or initialize failed. The failure is published and the host woken
    /// first, without a lock; closing the input comes after and never waits
    /// on a parked thread (Sol's 2a item 2). Core threads wind down.
    fn failTransport(self: *Runtime, message: []const u8) void {
        self.fail_message.set(&.{message});
        if (self.state.cmpxchgStrong(FX_RUNTIME_RUNNING, FX_RUNTIME_FAILED, .acq_rel, .acquire) != null) return;
        self.ready.notify();
        self.closeInput();
    }

    /// Parks a core thread of a panicked runtime at a boundary with the host.
    fn parkIfPanicked(self: *const Runtime) void {
        if (self.panicked()) park();
    }

    fn fault(self: *Runtime, point: Fault) void {
        if (faultHit(&self.faults[@intFromEnum(point)])) {
            std.debug.panic("fault injected at {s}", .{@tagName(point)});
        }
    }

    /// FX_FAULT_INPUT_LOCKED: the thread holding the input lock waits (up to
    /// two seconds) for the runtime to stop running, then panics there, so a
    /// host call failing the transport meets a lock a parked thread holds.
    fn faultInputLocked(self: *Runtime) void {
        if (!faultHit(&self.faults[@intFromEnum(Fault.input_locked)])) return;
        const started = nowMs();
        while (self.running() and nowMs() -| started < 2000) {
            const ts: std.c.timespec = .{ .sec = 0, .nsec = 1_000_000 };
            _ = std.c.nanosleep(&ts, null);
        }
        std.debug.panic("fault injected at {s}", .{@tagName(Fault.input_locked)});
    }

    /// FX_FAULT_DRAIN_OOM, _TAKE_OOM, _REPLY_OOM: not a panic; the host call
    /// fails as out of memory at its last fallible step.
    fn faultOom(self: *Runtime, point: Fault) bool {
        return faultHit(&self.faults[@intFromEnum(point)]);
    }

    /// FX_FAULT_HOST_TRANSPORT: the host call fails the runtime's transport,
    /// as one whose answer cannot be delivered would.
    fn faultHostTransport(self: *Runtime) bool {
        if (!faultHit(&self.faults[@intFromEnum(Fault.host_transport)])) return false;
        self.failTransport("fault injected at host_transport");
        return true;
    }

    // --- callbacks ------------------------------------------------------------

    /// Enters a callback. False if destroy has started (the caller skips the
    /// callback). Parks the thread if the runtime panicked: no callback
    /// starts after the failure is published.
    fn enterCallback(self: *Runtime) bool {
        _ = self.callbacks.fetchAdd(1, .seq_cst);
        if (self.panicked()) {
            _ = self.callbacks.fetchSub(1, .seq_cst);
            park();
        }
        if (self.closing.load(.seq_cst) or self.state.load(.seq_cst) != FX_RUNTIME_RUNNING) {
            _ = self.callbacks.fetchSub(1, .seq_cst);
            return false;
        }
        return true;
    }

    fn leaveCallback(self: *Runtime) void {
        _ = self.callbacks.fetchSub(1, .seq_cst);
    }

    fn waitCallbacks(self: *Runtime) void {
        while (self.callbacks.load(.seq_cst) != 0) std.Thread.yield() catch {};
    }

    // --- the input queue --------------------------------------------------------

    fn readInput(context: ?*anyopaque, destination: []u8) usize {
        const self: *Runtime = @ptrCast(@alignCast(context.?));
        self.parkIfPanicked();
        const read = self.input.read(self.alloc, destination);
        self.parkIfPanicked();
        // Room freed after a host met FX_BACKPRESSURE: wake it.
        if (read > 0 and self.input_blocked.swap(false, .acq_rel)) self.ready.notify();
        return read;
    }

    const InputMode = enum {
        /// A host request: bounded; FX_BACKPRESSURE or FX_E_TOO_LARGE.
        request,
        /// A small answer to something the core asked (a journal flush, a
        /// cancel): never refused for room, so an accepted answer is never
        /// lost.
        control,
    };

    /// Writes `bytes` to the core's input from a host call.
    fn hostInput(self: *Runtime, bytes: []const u8, mode: InputMode) Status {
        if (mode == .request and bytes.len > core.max_input_bytes) return FX_E_TOO_LARGE;
        const got = hostLock(self, &self.input.mutex);
        if (got != .locked) return acquireStatus(got);
        defer self.input.mutex.unlock(io_mod.getIo());
        return self.inputLocked(bytes, mode);
    }

    /// Writes `bytes` to the core's input from a core thread.
    fn coreInput(self: *Runtime, bytes: []const u8) Status {
        const io = io_mod.getIo();
        self.input.mutex.lockUncancelable(io);
        defer self.input.mutex.unlock(io);
        self.faultInputLocked();
        return self.inputLocked(bytes, .control);
    }

    fn inputLocked(self: *Runtime, bytes: []const u8, mode: InputMode) Status {
        const queue = &self.input;
        if (queue.closed) return FX_E_FAILED;
        const queued = queue.bytes.items.len - queue.offset;
        if (mode == .request and queued > core.max_input_bytes - bytes.len) {
            self.input_blocked.store(true, .release);
            return FX_BACKPRESSURE;
        }
        if (queue.offset > 0) {
            @memmove(queue.bytes.items[0..queued], queue.bytes.items[queue.offset..]);
            queue.bytes.items.len = queued;
            queue.offset = 0;
        }
        queue.bytes.appendSlice(self.alloc, bytes) catch return FX_E_OOM;
        queue.wake.signal(io_mod.getIo());
        return FX_OK;
    }

    /// Closes the core's input so its reader sees the end. Never waits on a
    /// thread that panicked holding the input lock: a host call gives up (as
    /// it does after the busy bound; destroy closes the input later), and a
    /// core thread parks, as at any boundary of a panicked runtime.
    fn closeInput(self: *Runtime) void {
        const io = io_mod.getIo();
        const on_host = tl_guard != null;
        var spins: usize = 0;
        const started = nowMs();
        while (true) {
            if (self.input.mutex.tryLock()) break;
            if (self.panicked()) {
                if (on_host) return;
                park();
            }
            if (on_host and nowMs() -| started > busy_bound_ms) return;
            spins += 1;
            if (spins < 64) std.atomic.spinLoopHint() else std.Thread.yield() catch {};
        }
        self.input.closed = true;
        self.input.wake.broadcast(io);
        self.input.mutex.unlock(io);
    }

    // --- events ---------------------------------------------------------------

    /// Queues an event from a core thread. Deltas wait for room; a control
    /// event that cannot be queued (out of memory) fails the runtime, so the
    /// host learns through FX_EVENT_EXITED instead of waiting for it.
    fn queueEvent(self: *Runtime, event: QueuedEvent, data: EventText, mode: EventMode) void {
        self.parkIfPanicked();
        const io = io_mod.getIo();
        const queue = &self.events;
        const total = data.total();
        if (total > max_event_bytes) {
            if (mode == .control) self.failTransport("an event exceeded the event queue");
            return;
        }
        queue.mutex.lockUncancelable(io);
        self.fault(.event_locked);
        if (mode == .delta) {
            while (!queue.closed and queue.full(total)) {
                queue.space.waitTimeout(io, &queue.mutex, pulse) catch {};
                if (self.panicked()) {
                    queue.mutex.unlock(io);
                    park();
                }
            }
        }
        if (queue.closed) {
            queue.mutex.unlock(io);
            return;
        }
        const was_empty = queue.appendLocked(event, data) catch {
            queue.mutex.unlock(io);
            self.failTransport("the core could not queue an event (out of memory)");
            return;
        };
        queue.mutex.unlock(io);
        if (was_empty) self.ready.notify();
    }

    // --- the core's thread --------------------------------------------------------

    /// The core's JSON-RPC writer: called on a core-owned thread with whole
    /// frames, serialised by the writer's own mutex.
    fn writeOutput(context: ?*anyopaque, bytes: []const u8) !void {
        const self: *Runtime = @ptrCast(@alignCast(context.?));
        self.parkIfPanicked();
        var rest = bytes;
        while (std.mem.findScalar(u8, rest, '\n')) |end| {
            if (self.line.items.len > 0) {
                try self.line.appendSlice(alloc_c, rest[0..end]);
                defer self.line.clearRetainingCapacity();
                try self.onFrame(self.line.items);
            } else {
                try self.onFrame(rest[0..end]);
            }
            rest = rest[end + 1 ..];
        }
        if (rest.len > 0) try self.line.appendSlice(alloc_c, rest);
    }

    fn run(self: *Runtime) void {
        panic_context.current = self;
        const provider = core.gateway_provider.Provider{
            .oauth_transport = core.oauth_transport.unavailable_provider,
            .chat_url = builtin_gateway.provider.chat_url,
        };
        var providers = provider_set.gateway_only(.{
            .capabilities = .{ .gateway_prompt_caching = true },
            .presentation = builtin_gateway.provider_bundle.presentation,
            .auth_strategy = .vercel,
            .fallback_model_capabilities_fn = builtin_gateway.provider_bundle.fallback_model_capabilities_fn,
            .agent_stream = core.bridgeBound(host_stream_provider.provider(&self.stream_context)),
            .model_catalog = @import("gateway/host_model_catalog.zig").provider(&self.stream_context.transport),
        });
        providers.codex = .{
            .capabilities = core.openai_codex.bundle_capabilities,
            .presentation = core.provider_catalog.find(.codex),
            .auth_strategy = .chatgpt,
            .agent_stream = core.bridgeBound(core.host_openai_codex.provider(&self.codex_context)),
            .model_catalog = core.host_openai_codex.modelCatalogProvider(&self.codex_context),
        };
        providers.grok = .{
            .presentation = core.provider_catalog.find(.grok),
            .auth_strategy = .grok,
            .agent_stream = core.bridgeBound(core.host_xai_grok.provider(&self.grok_context)),
            .model_catalog = core.host_xai_grok.modelCatalogProvider(&self.grok_context),
        };
        if (self.custom_context) |*context| {
            providers.configured_fn = core.configuredBridgeBundle;
            providers.configured_context = context;
        }
        acp_server.runWithTransport(
            self.alloc,
            .{
                .default_model = builtin_gateway.default_model,
                .default_agent_step_limit = self.max_agent_steps,
                .gateway_retry_count = 0,
                .gateway_chat_url = self.gateway_chat_url,
                .gateway_models_path = builtin_gateway.models_path,
                .gateway_provider = provider,
                .provider_set = providers,
                .secret_store = core.host.unavailable_secret_store,
                .prompt_policy = .{ .system_prompt = "" },
                .ignored_list_entries = &.{},
                .max_list_entries = 0,
                .max_read_file_bytes = 0,
                .max_read_file_lines = 0,
                .max_read_file_line_len = 0,
                .max_command_output_bytes = 0,
                .max_tool_result_bytes = self.max_tool_result_bytes,
                .max_history_turns = self.max_history_turns,
                .context_registry = .{ .default_provider = core.context_contract.empty_provider },
                .mode_registry = core.builtin_modes.registry,
                // Credentials never enter the core: the host adds them at its
                // fetch edge. The Gateway path insists on a key, so it gets a
                // placeholder, which fx_fetch_take strips (rule 13).
                .credential_override = host_managed_credential,
                .provider_override = self.provider,
                .provider_auth_mode = .host_managed,
                .configured_providers_json = self.configured_providers,
                .model_override = self.model,
                .effort_override = self.effort,
                .fast_override = null,
                .ultrafast_override = null,
                .home_override = self.home,
                .workspace_root_override = self.workspace_root,
                .allow_acp_mcp = false,
                .allow_native_tools = false,
                .minimal_kernel = true,
                .host_attachments = self.attachments.store(),
                .host_tool_bridge = if (self.host.tool_call != null) .{ .context = self, .call_fn = bridgeToolCall } else null,
                .host_event_sink = .{
                    .context = self,
                    .text_fn = sinkText,
                    .tool_start_fn = sinkToolStart,
                    .tool_end_fn = sinkToolEnd,
                    .usage_fn = sinkUsage,
                    .steer_joined_fn = sinkSteerJoined,
                },
                .host_hooks = if (self.host.hook != null) .{ .context = self, .register_fn = registerHooks } else null,
            },
            jsonrpc.Reader.initCallback(self, Runtime.readInput),
            jsonrpc.Writer.initCallback(self, Runtime.writeOutput),
        ) catch {
            self.exit_code.store(1, .release);
        };
        self.parkIfPanicked();
        self.coreShutdownFetch();
        _ = self.state.cmpxchgStrong(FX_RUNTIME_RUNNING, FX_RUNTIME_EXITED, .acq_rel, .acquire);
        self.ready.notify();
        self.thread_done.store(true, .release);
    }

    fn coreShutdownFetch(self: *Runtime) void {
        self.fetch.shutdown();
    }

    fn deinit(self: *Runtime) void {
        self.thread.join();
        self.ready.deinit();
        self.fetch.deinit();
        self.attachments.deinit();
        self.input.deinit(self.alloc);
        self.events.deinit();
        self.calls.deinit(alloc_c);
        self.pending.deinit(alloc_c);
        self.flushes.deinit(alloc_c);
        for (self.steers.items) |steer| alloc_c.free(steer.message_id);
        self.steers.deinit(alloc_c);
        self.line.deinit(alloc_c);
        self.taken.deinit();
        self.taking.deinit();
        if (self.journal_failed) |reason| alloc_c.free(reason);
        if (self.session_id) |id| alloc_c.free(id);
        if (self.open_params) |params| alloc_c.free(params);
        for ([_]?[]u8{ self.open_turn_id, self.last_turn_id, self.turn_message_id }) |id| if (id) |bytes| alloc_c.free(bytes);
        var names = self.tool_names.iterator();
        while (names.next()) |entry| {
            alloc_c.free(entry.key_ptr.*);
            alloc_c.free(entry.value_ptr.*);
        }
        self.tool_names.deinit(alloc_c);
        self.questions.deinit(alloc_c);
        self.freeHookTexts();
        self.hook_texts.deinit(alloc_c);
        self.clearReservedCalls();
        self.reserved_calls.deinit(alloc_c);
        for (self.tools) |tool| alloc_c.free(tool.name);
        alloc_c.free(self.tools);
        alloc_c.free(self.initialize_frame);
        if (self.custom_context) |*context| context.deinit(self.alloc);
        if (self.configured_providers) |json| self.alloc.free(json);
        if (self.model) |model| self.alloc.free(model);
        if (self.effort) |effort| self.alloc.free(effort);
        self.alloc.free(self.provider_name);
        self.alloc.free(self.home);
        self.alloc.free(self.workspace_root);
        self.alloc.free(self.gateway_chat_url);
        self.alloc.destroy(self);
    }

    fn freeHookTexts(self: *Runtime) void {
        for (self.hook_texts.items) |text| alloc_c.free(text);
        self.hook_texts.clearRetainingCapacity();
    }

    fn clearReservedCalls(self: *Runtime) void {
        var keys = self.reserved_calls.keyIterator();
        while (keys.next()) |key| alloc_c.free(key.*);
        self.reserved_calls.clearRetainingCapacity();
    }

    /// Copies `bytes` into the inbound attachment table under a new id
    /// (`id_out`), for the frame the caller sends next. A host call; the
    /// caller holds `lock`.
    fn attach(self: *Runtime, bytes: []const u8, id_out: *u32) Status {
        if (bytes.len > core.max_attachment_bytes) return FX_E_TOO_LARGE;
        const table = &self.attachments;
        const got = hostLock(self, &table.mutex);
        if (got != .locked) return acquireStatus(got);
        defer table.mutex.unlock(io_mod.getIo());
        if (table.inbound.count() >= core.max_inbound_attachments or
            bytes.len > core.max_inbound_attachment_bytes - table.inbound_bytes) return FX_E_TOO_LARGE;
        var id = self.next_attachment;
        while (table.inbound.contains(id)) id = if (id == std.math.maxInt(u32)) 1 else id + 1;
        const owned = alloc_c.dupe(u8, bytes) catch return FX_E_OOM;
        table.inbound.put(alloc_c, id, owned) catch {
            alloc_c.free(owned);
            return FX_E_OOM;
        };
        table.inbound_bytes += owned.len;
        self.next_attachment = if (id == std.math.maxInt(u32)) 1 else id + 1;
        id_out.* = id;
        return FX_OK;
    }

    /// Drops inbound attachments a frame named, if the core did not take
    /// them (it refused the frame, or never read it).
    fn dropAttachments(self: *Runtime, ids: []const u32) void {
        if (ids.len == 0) return;
        const table = &self.attachments;
        const io = io_mod.getIo();
        if (tl_guard != null) {
            if (hostLock(self, &table.mutex) != .locked) return;
        } else table.mutex.lockUncancelable(io);
        defer table.mutex.unlock(io);
        for (ids) |id| if (table.inbound.fetchRemove(id)) |entry| {
            table.inbound_bytes -= entry.value.len;
            alloc_c.free(entry.value);
        };
    }

    // --- ids and frames -----------------------------------------------------

    fn nextRequest(self: *Runtime) u64 {
        const id = self.next_request;
        self.next_request += 1;
        return id;
    }

    const Side = enum { host, core };

    /// Writes one request frame to the core and records what it answers.
    /// The caller holds `lock`. From a host call (`.host`) the input queue's
    /// bound applies; a request the core's thread sends for the host
    /// (`.core`) bypasses it.
    fn sendRequestWith(self: *Runtime, kind: Pending.Kind, request: u64, method: []const u8, params: []const u8, attachments: Attachments, side: Side) Status {
        const rpc_id = self.next_rpc_id;
        var frame: std.Io.Writer.Allocating = .init(alloc_c);
        defer frame.deinit();
        frame.writer.print("{{\"jsonrpc\":\"2.0\",\"id\":{d},\"method\":", .{rpc_id}) catch return FX_E_OOM;
        jsonrpc.writeJsonStr(method, &frame.writer) catch return FX_E_OOM;
        frame.writer.writeAll(",\"params\":") catch return FX_E_OOM;
        frame.writer.writeAll(params) catch return FX_E_OOM;
        frame.writer.writeAll("}\n") catch return FX_E_OOM;
        if (side == .host and frame.written().len > core.max_input_bytes) return FX_E_TOO_LARGE;
        self.pending.put(alloc_c, rpc_id, .{ .kind = kind, .request = request, .attachments = attachments }) catch return FX_E_OOM;
        const status = switch (side) {
            .host => self.hostInput(frame.written(), .request),
            .core => self.coreInput(frame.written()),
        };
        if (status != FX_OK) {
            _ = self.pending.remove(rpc_id);
            return status;
        }
        self.next_rpc_id += 1;
        return FX_OK;
    }

    fn sendRequest(self: *Runtime, kind: Pending.Kind, request: u64, method: []const u8, params: []const u8) Status {
        return self.sendRequestWith(kind, request, method, params, .{}, .host);
    }

    /// Sends libfx/journal_open for the session with `tail` (journalOpenParams).
    /// The caller holds `lock`.
    fn sendJournalOpen(self: *Runtime, request: u64, tail: []const u8, attachments: Attachments, on_host: bool) Status {
        var params: std.Io.Writer.Allocating = .init(alloc_c);
        defer params.deinit();
        params.writer.writeAll("{\"sessionId\":") catch return FX_E_OOM;
        jsonrpc.writeJsonStr(self.session_id orelse "", &params.writer) catch return FX_E_OOM;
        if (tail.len > 0) {
            params.writer.writeByte(',') catch return FX_E_OOM;
            params.writer.writeAll(tail) catch return FX_E_OOM;
        }
        params.writer.writeAll("}") catch return FX_E_OOM;
        return self.sendRequestWith(.journal_open, request, "libfx/journal_open", params.written(), attachments, if (on_host) .host else .core);
    }

    /// Answers a request the core made, from a core thread or a host call.
    fn respond(self: *Runtime, rpc_id: i64, result_json: []const u8, on_host: bool) Status {
        var frame: std.Io.Writer.Allocating = .init(alloc_c);
        defer frame.deinit();
        frame.writer.print("{{\"jsonrpc\":\"2.0\",\"id\":{d},\"result\":", .{rpc_id}) catch return FX_E_OOM;
        frame.writer.writeAll(result_json) catch return FX_E_OOM;
        frame.writer.writeAll("}\n") catch return FX_E_OOM;
        return if (on_host) self.hostInput(frame.written(), .control) else self.coreInput(frame.written());
    }

    fn respondError(self: *Runtime, rpc_id: i64, message: []const u8, on_host: bool) Status {
        var frame: std.Io.Writer.Allocating = .init(alloc_c);
        defer frame.deinit();
        frame.writer.print("{{\"jsonrpc\":\"2.0\",\"id\":{d},\"error\":{{\"code\":-32000,\"message\":", .{rpc_id}) catch return FX_E_OOM;
        jsonrpc.writeJsonStr(message, &frame.writer) catch return FX_E_OOM;
        frame.writer.writeAll("}}\n") catch return FX_E_OOM;
        return if (on_host) self.hostInput(frame.written(), .control) else self.coreInput(frame.written());
    }

    /// A control answer that must reach the core; failing to queue it fails
    /// the runtime rather than leaving the core waiting for ever.
    fn mustRespond(self: *Runtime, status: Status) void {
        if (status == FX_OK or status == FX_E_FAILED) return;
        self.failTransport("the core could not queue an answer (out of memory)");
    }

    // --- frames from the core (core-owned threads) ---------------------------

    fn onFrame(self: *Runtime, frame: []const u8) !void {
        // What a turn streams reaches the host through the typed sink; the
        // session/update frames left (notices, recovery status) are not the
        // host's and are dropped without a parse (P-EVENTS).
        if (std.mem.startsWith(u8, frame, "{\"jsonrpc\":\"2.0\",\"method\":\"session/update\"")) return;
        var arena: std.heap.ArenaAllocator = .init(alloc_c);
        defer arena.deinit();
        const parsed = std.json.parseFromSliceLeaky(std.json.Value, arena.allocator(), frame, .{}) catch return;
        if (parsed != .object) return;
        const root = parsed.object;
        const method = if (root.get("method")) |value| (if (value == .string) value.string else "") else "";
        const id: ?i64 = if (root.get("id")) |value| (if (value == .integer) value.integer else null) else null;
        const params = root.get("params");
        if (method.len == 0) {
            if (id) |rpc_id| self.onResponse(arena.allocator(), rpc_id, root.get("result"), root.get("error"));
            return;
        }
        if (std.mem.eql(u8, method, "libfx/journal_append")) {
            if (params) |value| self.onJournalAppend(arena.allocator(), value);
        } else if (std.mem.eql(u8, method, "libfx/journal_flush")) {
            if (id) |rpc_id| self.onJournalFlush(rpc_id);
        } else if (id) |rpc_id| {
            // The minimal kernel asks nothing else of its host (host tools
            // come through the bridge); refuse it.
            if (std.mem.eql(u8, method, "session/request_permission")) {
                self.mustRespond(self.respond(rpc_id, "{\"outcome\":{\"outcome\":\"cancelled\"}}", false));
            } else {
                self.mustRespond(self.respondError(rpc_id, "Method not supported by the host", false));
            }
        }
    }

    fn onResponse(self: *Runtime, arena: Allocator, rpc_id: i64, result: ?std.json.Value, err: ?std.json.Value) void {
        const io = io_mod.getIo();
        self.lock.lockUncancelable(io);
        const entry = self.pending.fetchRemove(rpc_id) orelse {
            self.lock.unlock(io);
            return;
        };
        const pending = entry.value;
        // What the core did not take of the frame's attachments goes now.
        self.dropAttachments(pending.attachments.slice());
        const message = errorMessage(err);
        switch (pending.kind) {
            .initialize => {
                self.lock.unlock(io);
                if (err != null) {
                    // Every request made meanwhile ends with EXITED.
                    self.failTransport(if (message.len > 0) message else "the core refused initialize");
                }
            },
            .libfx_new => {
                const tail = self.open_params;
                self.open_params = null;
                defer if (tail) |owned| alloc_c.free(owned);
                const attachments = self.open_attachments;
                self.open_attachments = .{};
                if (err != null) {
                    self.session = .none;
                    self.dropAttachments(attachments.slice());
                    self.lock.unlock(io);
                    self.queueEvent(.{ .kind = .request_done, .request = pending.request, .status = FX_E_FAILED }, .{ .text = message }, .control);
                    return;
                }
                const session_id = stringField(result, "sessionId") orelse "";
                if (self.session_id) |old| alloc_c.free(old);
                self.session_id = alloc_c.dupe(u8, session_id) catch null;
                if (self.journaled) {
                    // The core asked for nothing: this request is ours, sent
                    // from the core's thread, so it bypasses the bound.
                    const status = self.sendJournalOpen(pending.request, tail orelse "", attachments, false);
                    if (status != FX_OK) {
                        self.session = .created;
                        self.dropAttachments(attachments.slice());
                    }
                    self.lock.unlock(io);
                    if (status != FX_OK) self.queueEvent(.{ .kind = .request_done, .request = pending.request, .status = status }, .{ .text = "could not open the journal" }, .control);
                    return;
                }
                self.session = .open;
                self.lock.unlock(io);
                self.queueEvent(.{ .kind = .session_opened, .request = pending.request }, .{ .text = session_id }, .control);
            },
            .journal_open => self.onJournalOpened(arena, pending.request, result, err, message),
            .prompt => {
                const turn = pending.request;
                // Steers still queued when the turn ends come back first.
                var returned: std.ArrayList(Steer) = .empty;
                while (self.steers.items.len > 0) {
                    returned.append(arena, self.steers.items[0]) catch break;
                    _ = self.steers.orderedRemove(0);
                }
                // The core's hook dispatch copied every answer it was handed.
                self.freeHookTexts();
                self.clearReservedCalls();
                self.lock.unlock(io);
                for (returned.items) |steer| {
                    self.queueEvent(.{ .kind = .steer_returned, .turn = turn, .request = steer.request }, .{ .text2 = steer.message_id }, .control);
                    alloc_c.free(steer.message_id);
                }
                var reason: StopReason = .@"error";
                var usage: Usage = .{};
                if (err != null) {
                    self.queueEvent(.{ .kind = .@"error", .turn = turn, .status = FX_E_FAILED }, .{ .text = message }, .control);
                } else {
                    // The step limit arrives typed, as max_model_turns
                    // (patch P-STEPS), never read from the reply's text.
                    reason = StopReason.fromAcp(stringField(result, "stopReason") orelse "");
                    usage = promptUsage(result);
                }
                self.endTurn(turn, reason, usage);
            },
            .steer => {
                var status: Status = FX_OK;
                if (err != null) {
                    // libfx answers a steer that arrived after its turn with
                    // "No prompt is running": the turn ended first.
                    status = if (std.mem.find(u8, message, "No prompt is running") != null) FX_STALE else FX_E_FAILED;
                    self.dropSteer(pending.request);
                }
                self.lock.unlock(io);
                self.queueEvent(.{ .kind = .request_done, .request = pending.request, .status = status }, .{ .text = if (status == FX_E_FAILED) message else "" }, .control);
            },
            .handoff => {
                // A steer with images: queued as the handoff; it comes back
                // at the turn's end.
                const status: Status = if (err == null) FX_OK else FX_E_FAILED;
                if (err != null) self.dropSteer(pending.request);
                self.lock.unlock(io);
                self.queueEvent(.{ .kind = .request_done, .request = pending.request, .status = status }, .{ .text = if (status == FX_E_FAILED) message else "" }, .control);
            },
            .withdraw => {
                var status: Status = FX_E_FAILED;
                if (err == null) {
                    const outcome = stringField(result, "result") orelse "";
                    status = if (std.mem.eql(u8, outcome, "withdrawn")) FX_OK else FX_STALE;
                }
                self.lock.unlock(io);
                self.queueEvent(.{ .kind = .request_done, .request = pending.request, .status = status }, .{ .text = if (status == FX_E_FAILED) message else "" }, .control);
            },
            .journal_images => {
                self.lock.unlock(io);
                const status = if (err == null) FX_OK else journalErrorStatus(message);
                self.queueEvent(.{ .kind = .request_done, .request = pending.request, .status = status }, .{ .text = if (status == FX_OK) "" else message }, .control);
            },
            .snapshot => {
                self.lock.unlock(io);
                const id = intField(result, "snapshotAttachment");
                if (err != null or id == null or id.? <= 0 or id.? > std.math.maxInt(u32)) {
                    const why = if (err != null) message else if (stringField(result, "skipped")) |skipped| skipped else "the session is not quiet (a turn is open or a follow-up waits)";
                    self.queueEvent(.{ .kind = .request_done, .request = pending.request, .status = if (err != null) FX_E_FAILED else FX_E_STATE }, .{ .text = why }, .control);
                    return;
                }
                const bytes = self.attachments.takeOutbound(@intCast(id.?)) orelse {
                    self.queueEvent(.{ .kind = .request_done, .request = pending.request, .status = FX_E_FAILED }, .{ .text = "the core's checkpoint was not in the attachment table" }, .control);
                    return;
                };
                defer alloc_c.free(bytes);
                const at_seq: u64 = @intCast(@max(0, intField(result, "atSeq") orelse 0));
                self.queueEvent(.{ .kind = .checkpoint, .request = pending.request, .id = at_seq }, .{ .bytes = bytes }, .control);
            },
        }
    }

    /// Queues `turn`'s TURN_END and ends the turn in one critical section
    /// with `lock` and the event lock, so a host that has drained TURN_END
    /// can start the next turn at once. A control event: it never waits for
    /// room; one that cannot be queued fails the runtime.
    fn endTurn(self: *Runtime, turn: u64, reason: StopReason, usage: Usage) void {
        self.parkIfPanicked();
        const io = io_mod.getIo();
        const queue = &self.events;
        self.lock.lockUncancelable(io);
        queue.mutex.lockUncancelable(io);
        var appended = true;
        var was_empty = false;
        if (!queue.closed) {
            if (queue.appendLocked(.{ .kind = .turn_end, .turn = turn, .code = @intFromEnum(reason), .usage = usage }, .{})) |empty| {
                was_empty = empty;
            } else |_| appended = false;
        }
        if (self.turn == turn) {
            self.turn = 0;
            // A turn that ended, however, is the one a retry of its message
            // id names ("Deduplication"); a failed or yielded one is not.
            const ended = self.turn_message_id;
            self.turn_message_id = null;
            if (reason != .@"error" and reason != .yielded) {
                if (self.last_turn_id) |old| alloc_c.free(old);
                self.last_turn_id = ended;
            } else if (ended) |id| alloc_c.free(id);
            // A crash's open turn is resumed or interrupted by now.
            self.resumable = false;
            if (self.open_turn_id) |id| alloc_c.free(id);
            self.open_turn_id = null;
        }
        queue.mutex.unlock(io);
        self.lock.unlock(io);
        if (!appended) return self.failTransport("the core could not queue an event (out of memory)");
        if (was_empty) self.ready.notify();
    }

    /// libfx/journal_open's answer. The caller holds `lock`; this releases it.
    fn onJournalOpened(self: *Runtime, arena: Allocator, request: u64, result: ?std.json.Value, err: ?std.json.Value, message: []const u8) void {
        const io = io_mod.getIo();
        if (err != null) {
            // The core holds the session; another open may succeed.
            self.session = .created;
            self.lock.unlock(io);
            self.queueEvent(.{ .kind = .request_done, .request = request, .status = journalErrorStatus(message) }, .{ .text = message }, .control);
            return;
        }
        if (arrayField(result, "imagesNeeded")) |needed| {
            self.session = .created;
            self.lock.unlock(io);
            var names: std.ArrayList([]const u8) = .empty;
            for (needed) |value| if (value == .string) names.append(arena, value.string) catch {};
            self.queueEvent(.{ .kind = .images_needed, .request = request }, .{ .names = names.items }, .control);
            return;
        }
        self.session = .open;
        const resumable = boolField(result, "resumable");
        const open_turn = if (resumable) stringField(result, "openTurnId") else null;
        const last_turn = stringField(result, "lastTurnId");
        self.resumable = resumable and open_turn != null;
        for ([_]struct { *?[]u8, ?[]const u8 }{ .{ &self.open_turn_id, open_turn }, .{ &self.last_turn_id, last_turn } }) |slot| {
            if (slot[0].*) |old| alloc_c.free(old);
            slot[0].* = if (slot[1]) |id| alloc_c.dupe(u8, id) catch null else null;
        }
        const session_id = arena.dupe(u8, self.session_id orelse "") catch "";
        self.lock.unlock(io);
        // What the crash left, then the open (header, "Open versus resume").
        const recovery = EventKind.recovery;
        if (open_turn) |id| self.queueEvent(.{ .kind = recovery, .request = request, .code = FX_RECOVERY_OPEN_TURN }, .{ .text = id }, .control);
        if (last_turn) |id| self.queueEvent(.{ .kind = recovery, .request = request, .code = FX_RECOVERY_LAST_TURN }, .{ .text = id }, .control);
        if (resumable) if (arrayField(result, "runningCalls")) |calls| for (calls) |call| {
            self.queueEvent(.{ .kind = recovery, .request = request, .code = FX_RECOVERY_CALL }, .{
                .text = stringField(call, "name") orelse "",
                .text2 = stringField(call, "id") orelse "",
                .bytes = stringField(call, "arguments") orelse "",
            }, .control);
        };
        if (arrayField(result, "followUps")) |follow_ups| for (follow_ups) |follow_up| {
            self.queueEvent(.{ .kind = recovery, .request = request, .code = FX_RECOVERY_FOLLOW_UP }, .{
                .text = stringField(follow_up, "text") orelse "",
                .text2 = stringField(follow_up, "id") orelse "",
            }, .control);
        };
        var flags: u32 = FX_OPENED_JOURNALED;
        if (self.resumable) flags |= FX_OPENED_RESUMABLE;
        if (boolField(result, "yielded")) flags |= FX_OPENED_YIELDED;
        self.queueEvent(.{
            .kind = .session_opened,
            .request = request,
            .flags = flags,
            .id = @intCast(@max(0, intField(result, "turns") orelse 0)),
        }, .{ .text = session_id }, .control);
    }

    /// Forgets the steer `request`. The caller holds `lock`.
    fn dropSteer(self: *Runtime, request: u64) void {
        for (self.steers.items, 0..) |steer, index| if (steer.request == request) {
            alloc_c.free(steer.message_id);
            _ = self.steers.orderedRemove(index);
            return;
        };
    }

    fn currentTurn(self: *Runtime) u64 {
        const io = io_mod.getIo();
        self.lock.lockUncancelable(io);
        defer self.lock.unlock(io);
        return self.turn;
    }

    fn onJournalAppend(self: *Runtime, arena: Allocator, params: std.json.Value) void {
        const callback = self.host.journal_append orelse return;
        const events = params.object.get("events") orelse return;
        const barrier = boolField(params, "barrier");
        var first_seq: u64 = 0;
        var last_seq: u64 = 0;
        if (events == .array) for (events.array.items, 0..) |event, index| {
            const seq: u64 = @intCast(@max(0, intField(event, "seq") orelse 0));
            if (index == 0) first_seq = seq;
            last_seq = seq;
        };
        var out: std.Io.Writer.Allocating = .init(arena);
        std.json.Stringify.value(events, .{}, &out.writer) catch return;
        const io = io_mod.getIo();
        self.lock.lockUncancelable(io);
        self.fault(.journal_append);
        if (self.journal_failed != null) {
            self.lock.unlock(io);
            return;
        }
        self.journal_sent += 1;
        const info: JournalAppendInfo = .{
            .flags = if (barrier) FX_APPEND_BARRIER else 0,
            .append = self.journal_sent,
            .turn = self.turn,
            .first_seq = first_seq,
            .last_seq = last_seq,
            .events_json = Str.of(out.written()),
        };
        self.lock.unlock(io);
        if (!self.enterCallback()) return;
        callback(self.host_ctx, &info);
        self.leaveCallback();
    }

    fn onJournalFlush(self: *Runtime, rpc_id: i64) void {
        const io = io_mod.getIo();
        self.lock.lockUncancelable(io);
        defer self.lock.unlock(io);
        if (self.journal_failed) |reason| return self.mustRespond(self.respondError(rpc_id, reason, false));
        if (self.journal_acked >= self.journal_sent) return self.mustRespond(self.respond(rpc_id, "{}", false));
        self.flushes.append(alloc_c, .{ .rpc_id = rpc_id, .upto = self.journal_sent }) catch
            self.mustRespond(self.respondError(rpc_id, "Journal flush failed", false));
    }
};

/// The Gateway credential the core is given, never a secret: the host sets
/// the real authorization header at its fetch edge.
const host_managed_credential = "host-managed";

// ---------------------------------------------------------------------------
// The typed event sink (P-EVENTS), on the core thread that streams
// ---------------------------------------------------------------------------

fn sinkRuntime(raw: *anyopaque) *Runtime {
    return @ptrCast(@alignCast(raw));
}

fn sinkText(raw: *anyopaque, kind: host_event_sink.TextKind, text: []const u8) host_event_sink.Error!void {
    const self = sinkRuntime(raw);
    if (text.len == 0) return;
    const thought = kind == .thought;
    // engine.js drops the core's context notices from the reply.
    if (!thought and std.mem.startsWith(u8, text, "[context]")) return;
    const turn = self.currentTurn();
    if (!thought) self.fault(.streaming);
    self.queueEvent(.{ .kind = if (thought) .thought_delta else .text_delta, .turn = turn }, .{ .text = text }, .delta);
}

fn sinkToolStart(raw: *anyopaque, call_id: []const u8, name: []const u8) host_event_sink.Error!void {
    const self = sinkRuntime(raw);
    self.parkIfPanicked();
    const io = io_mod.getIo();
    self.lock.lockUncancelable(io);
    const turn = self.turn;
    // The card's end names the tool too: remember it by the call's id.
    remember: {
        if (self.tool_names.contains(call_id)) break :remember;
        const key = alloc_c.dupe(u8, call_id) catch break :remember;
        const value = alloc_c.dupe(u8, name) catch {
            alloc_c.free(key);
            break :remember;
        };
        self.tool_names.put(alloc_c, key, value) catch {
            alloc_c.free(key);
            alloc_c.free(value);
        };
    }
    self.lock.unlock(io);
    self.queueEvent(.{ .kind = .tool_start, .turn = turn }, .{ .text = name, .text2 = call_id }, .control);
}

fn sinkToolEnd(raw: *anyopaque, call_id: []const u8, failed: bool) host_event_sink.Error!void {
    const self = sinkRuntime(raw);
    self.parkIfPanicked();
    const io = io_mod.getIo();
    self.lock.lockUncancelable(io);
    const turn = self.turn;
    const entry = self.tool_names.fetchRemove(call_id);
    self.lock.unlock(io);
    defer if (entry) |known| {
        alloc_c.free(known.key);
        alloc_c.free(known.value);
    };
    self.queueEvent(.{
        .kind = .tool_end,
        .turn = turn,
        .status = if (failed) FX_E_FAILED else FX_OK,
    }, .{ .text = if (entry) |known| known.value else "", .text2 = call_id }, .control);
}

fn sinkUsage(raw: *anyopaque, used: u64, size: u64) host_event_sink.Error!void {
    const self = sinkRuntime(raw);
    const turn = self.currentTurn();
    self.queueEvent(.{ .kind = .usage, .turn = turn, .usage = .{ .context_used = used, .context_size = size } }, .{}, .delta);
}

fn sinkSteerJoined(raw: *anyopaque, text: []const u8, input_id: ?[]const u8) host_event_sink.Error!void {
    const self = sinkRuntime(raw);
    self.parkIfPanicked();
    const id = input_id orelse "";
    // The steer joined: it no longer comes back at the turn's end.
    var request: u64 = 0;
    const io = io_mod.getIo();
    self.lock.lockUncancelable(io);
    const turn = self.turn;
    for (self.steers.items, 0..) |steer, index| if (std.mem.eql(u8, steer.message_id, id)) {
        request = steer.request;
        alloc_c.free(steer.message_id);
        _ = self.steers.orderedRemove(index);
        break;
    };
    self.lock.unlock(io);
    self.queueEvent(.{ .kind = .steer_joined, .turn = turn, .request = request }, .{ .text = text, .text2 = id }, .control);
}

// ---------------------------------------------------------------------------
// Hooks (P-STOP, P-PRETOOL), on the core thread that reaches them
// ---------------------------------------------------------------------------

fn registerHooks(raw: *anyopaque, hooks: *hook_runtime.Runtime) anyerror!void {
    try hooks.registerPreToolUse(.{ .name = "powermove.pre_tool", .ctx = raw, .run = preToolHook });
    try hooks.registerStop(.{ .name = "powermove.stop", .ctx = raw, .run = stopHook });
}

const Asked = struct { answer: i32, text: ?[]u8 };

/// Asks the host `info` and waits for the answer: from the callback (ALLOW),
/// or from fx_hook_answer. The question's strings are the core's, valid
/// until this returns (the lease, header rule 9). Null when the runtime is
/// closing; a cancelled turn's question still waits for its answer.
fn askHost(self: *Runtime, info: *HookQuestion) ?Asked {
    const callback = self.host.hook orelse return .{ .answer = FX_ANSWER_ALLOW, .text = null };
    const io = io_mod.getIo();
    var record: HookRecord = .{ .id = 0, .turn = 0, .kind = info.kind };
    self.lock.lockUncancelable(io);
    if (self.panicked()) {
        self.lock.unlock(io);
        park();
    }
    if (self.closing.load(.acquire)) {
        self.lock.unlock(io);
        return null;
    }
    record.id = self.next_question;
    record.turn = self.turn;
    self.questions.put(alloc_c, record.id, &record) catch {
        self.lock.unlock(io);
        return null;
    };
    self.next_question += 1;
    self.lock.unlock(io);
    defer {
        self.lock.lockUncancelable(io);
        _ = self.questions.remove(record.id);
        self.lock.unlock(io);
    }
    info.question = record.id;
    info.turn = record.turn;
    if (!self.enterCallback()) return null;
    const verdict = callback(self.host_ctx, info);
    self.leaveCallback();

    self.lock.lockUncancelable(io);
    defer self.lock.unlock(io);
    // Answering ALLOW from the callback consumes the question at once; an
    // answer it already gave with fx_hook_answer wins.
    if (!record.answered and verdict != FX_HOOK_PENDING) return .{ .answer = FX_ANSWER_ALLOW, .text = null };
    while (!record.answered) {
        if (self.panicked()) {
            self.lock.unlock(io);
            park();
        }
        // Destroy ends the question without an answer (rule 10).
        if (self.closing.load(.acquire)) return null;
        self.calls_changed.waitTimeout(io, &self.lock, pulse) catch {};
    }
    return .{ .answer = record.answer, .text = record.text };
}

/// Keeps `text` until the turn ends: the core's dispatch copies it after the
/// handler returns. The caller holds no lock.
fn keepHookText(self: *Runtime, text: []u8) []const u8 {
    const io = io_mod.getIo();
    self.lock.lockUncancelable(io);
    defer self.lock.unlock(io);
    self.hook_texts.append(alloc_c, text) catch {
        // Kept for the process instead: a leak of one answer beats a use
        // after free.
    };
    return text;
}

fn hostToolFlags(self: *const Runtime, name: []const u8) ?u32 {
    for (self.tools) |tool| if (std.mem.eql(u8, tool.name, name)) return tool.flags;
    return null;
}

fn preToolHook(raw: *anyopaque, input: hook_defs.PreToolUseInput) hook_defs.HandlerError!hook_defs.PreToolUseAction {
    const self: *Runtime = @ptrCast(@alignCast(raw));
    self.parkIfPanicked();
    // Only the host's own tools are the host's to gate.
    if (hostToolFlags(self, input.tool_name) == null) return .continue_;
    const io = io_mod.getIo();
    // The call it would make gets its id now, so the question can name it.
    self.lock.lockUncancelable(io);
    const call = self.next_call;
    self.next_call += 1;
    const key = alloc_c.dupe(u8, input.call_id) catch null;
    if (key) |owned| {
        if (self.reserved_calls.fetchRemove(owned)) |old| alloc_c.free(old.key);
        self.reserved_calls.put(alloc_c, owned, call) catch alloc_c.free(owned);
    }
    self.lock.unlock(io);
    var info: HookQuestion = .{
        .kind = FX_HOOK_PRE_TOOL,
        .question = 0,
        .turn = 0,
        .call = call,
        .name = Str.of(input.tool_name),
        .input_json = Str.of(input.arguments_json),
        .final_text = .{},
        .continuations = 0,
        .reserved = 0,
    };
    const asked = askHost(self, &info) orelse return error.Cancelled;
    switch (asked.answer) {
        FX_ANSWER_BLOCK => {
            self.lock.lockUncancelable(io);
            if (self.reserved_calls.fetchRemove(input.call_id)) |old| alloc_c.free(old.key);
            self.lock.unlock(io);
            const reason = asked.text orelse return .{ .block = "The host's pre_tool check blocked this call." };
            return .{ .block = keepHookText(self, reason) };
        },
        FX_ANSWER_REWRITE => return .{ .rewrite_arguments = keepHookText(self, asked.text.?) },
        else => {
            if (asked.text) |text| alloc_c.free(text);
            return .continue_;
        },
    }
}

fn stopHook(raw: *anyopaque, input: hook_defs.StopInput) hook_defs.HandlerError!hook_defs.StopAction {
    const self: *Runtime = @ptrCast(@alignCast(raw));
    self.parkIfPanicked();
    if (!input.can_continue) return .allow;
    const io = io_mod.getIo();
    self.lock.lockUncancelable(io);
    if (self.stop_turn != self.turn) {
        self.stop_turn = self.turn;
        self.stop_asks = 0;
    }
    const asks = self.stop_asks;
    // Asked at most max_stop_continuations + 1 times a turn.
    const last = asks >= self.max_stop_continuations;
    if (asks > self.max_stop_continuations) {
        self.lock.unlock(io);
        return .allow;
    }
    self.stop_asks += 1;
    self.lock.unlock(io);
    var info: HookQuestion = .{
        .kind = FX_HOOK_STOP,
        .question = 0,
        .turn = 0,
        .call = 0,
        .name = .{},
        .input_json = .{},
        .final_text = Str.of(input.assistant_text),
        .continuations = asks,
        .reserved = 0,
    };
    const asked = askHost(self, &info) orelse return error.Cancelled;
    if (asked.answer == FX_ANSWER_CONTINUE and !last) {
        const text = asked.text orelse return .allow;
        return .{ .continue_once = keepHookText(self, text) };
    }
    if (asked.text) |text| alloc_c.free(text);
    return .allow;
}

// ---------------------------------------------------------------------------
// Host tool calls (P-TOOLS), on the core thread that makes them
// ---------------------------------------------------------------------------

fn bridgeToolCall(
    raw: *anyopaque,
    alloc: Allocator,
    name: []const u8,
    call_id: []const u8,
    arguments_json: []const u8,
    max_result_bytes: usize,
    cancel_flag: ?*std.atomic.Value(bool),
) tool_dispatch.DispatchError!tool_dispatch.ToolResult {
    const self: *Runtime = @ptrCast(@alignCast(raw));
    const callback = self.host.tool_call orelse return .{ .failure = try alloc.dupe(u8, "The host has no tools") };
    const io = io_mod.getIo();
    var flags: u32 = 0;
    for (self.tools) |tool| if (std.mem.eql(u8, tool.name, name)) {
        flags = tool.flags;
        break;
    };
    const writes = flags & FX_TOOL_WRITES != 0;
    var record: ToolRecord = .{ .id = 0, .turn = 0, .writes = writes };
    self.lock.lockUncancelable(io);
    // Writes run alone; idempotent calls run up to the configured limit.
    while (true) {
        if (self.panicked()) {
            self.lock.unlock(io);
            park();
        }
        if (self.closing.load(.acquire) or cancelled(cancel_flag)) {
            self.lock.unlock(io);
            return error.Cancelled;
        }
        const free = if (writes)
            self.calls_outstanding == 0
        else
            !self.writer_outstanding and self.calls_outstanding < self.max_concurrent_tools;
        if (free) break;
        self.calls_changed.waitTimeout(io, &self.lock, pulse) catch {};
    }
    // A call pre_tool asked about keeps the id its question named.
    record.id = if (self.reserved_calls.fetchRemove(call_id)) |reserved| id: {
        alloc_c.free(reserved.key);
        break :id reserved.value;
    } else id: {
        const id = self.next_call;
        self.next_call += 1;
        break :id id;
    };
    record.turn = self.turn;
    self.calls.put(alloc_c, record.id, &record) catch {
        self.lock.unlock(io);
        return error.OutOfMemory;
    };
    self.calls_outstanding += 1;
    if (writes) self.writer_outstanding = true;
    self.lock.unlock(io);
    defer {
        self.lock.lockUncancelable(io);
        _ = self.calls.remove(record.id);
        // An answer that landed after this thread stopped waiting.
        if (record.result) |late| alloc_c.free(late);
        if (record.images) |late| core_types.freeToolImages(alloc_c, late);
        self.calls_outstanding -= 1;
        if (writes) self.writer_outstanding = false;
        self.calls_changed.broadcast(io);
        self.lock.unlock(io);
    }

    self.fault(.tool_call);
    // The input stays the core's until the host answers: this thread frees
    // nothing it handed out before then (the lease, rule 9).
    const info: ToolCallInfo = .{
        .flags = flags,
        .call = record.id,
        .turn = record.turn,
        .name = Str.of(name),
        .input_json = Str.of(arguments_json),
        .model_call_id = Str.of(call_id),
    };
    if (!self.enterCallback()) return error.Cancelled;
    callback(self.host_ctx, &info);
    self.leaveCallback();

    var cancel_sent = false;
    self.lock.lockUncancelable(io);
    while (!record.answered) {
        if (self.panicked()) {
            self.lock.unlock(io);
            park();
        }
        // Destroy ends leases without answers (rule 10).
        if (self.closing.load(.acquire)) break;
        if (!cancel_sent and (record.cancel_requested or cancelled(cancel_flag))) {
            cancel_sent = true;
            record.cancel_requested = true;
            if (self.host.tool_cancel) |cancel| {
                self.lock.unlock(io);
                if (self.enterCallback()) {
                    cancel(self.host_ctx, record.id, record.turn);
                    self.leaveCallback();
                }
                self.lock.lockUncancelable(io);
                continue;
            }
        }
        self.calls_changed.waitTimeout(io, &self.lock, pulse) catch {};
    }
    const answered = record.answered;
    const result = record.result;
    record.result = null;
    const images = record.images;
    record.images = null;
    const is_error = record.is_error;
    const cancel_requested = record.cancel_requested;
    self.lock.unlock(io);

    const text = result orelse return error.Cancelled;
    if (!answered or cancel_requested or cancelled(cancel_flag)) {
        alloc_c.free(text);
        if (images) |list| core_types.freeToolImages(alloc_c, list);
        return error.Cancelled;
    }
    if (images) |list| {
        // Images are the core's from here, without another copy when its
        // allocator is the C allocator.
        if (alloc.vtable == alloc_c.vtable) return .{ .rich = .{ .text = text, .images = list, .is_error = is_error } };
        defer {
            alloc_c.free(text);
            core_types.freeToolImages(alloc_c, list);
        }
        const owned_text = try alloc.dupe(u8, text);
        errdefer alloc.free(owned_text);
        return .{ .rich = .{ .text = owned_text, .images = try core_types.dupeToolImages(alloc, list), .is_error = is_error } };
    }
    // The host's text was copied once into `text`; it becomes the result
    // without another copy when the core's allocator is the C allocator.
    const owned = if (alloc.vtable == alloc_c.vtable) text else owned: {
        defer alloc_c.free(text);
        break :owned try alloc.dupe(u8, text);
    };
    _ = max_result_bytes;
    return if (is_error) .{ .failure = owned } else .{ .success = owned };
}

fn cancelled(flag: ?*std.atomic.Value(bool)) bool {
    return if (flag) |value| value.load(.seq_cst) else false;
}

// ---------------------------------------------------------------------------
// The fetch bridge's core side: the provider's transport
// ---------------------------------------------------------------------------

/// Publishes the core's request (Sol's 2a item 3). It holds `lock` from
/// reading the running turn to publication, and fx_turn_cancel holds `lock`
/// throughout, so the two are linearised: a cancel either finds the
/// published fetch and retires it, or comes first and the fetch is published
/// retired. The fetch's turn is set under the bridge lock with the request,
/// so fx_fetch_take reads both in one critical section.
fn fetchOpen(raw: ?*anyopaque, method: []const u8, url: []const u8, headers: []const u8, body: []const u8) !i32 {
    const self: *Runtime = @ptrCast(@alignCast(raw.?));
    self.parkIfPanicked();
    const io = io_mod.getIo();
    self.lock.lockUncancelable(io);
    defer self.lock.unlock(io);
    const turn = self.turn;
    const retired = turn != 0 and turn == self.cancelled_turn;

    // FetchBridge.open (libfx_core_runtime.zig), with the owner and the
    // retired publication added.
    const bridge = &self.fetch;
    bridge.mutex.lockUncancelable(io);
    defer bridge.mutex.unlock(io);
    if (bridge.next_handle == 0) return error.HostStreamUnavailable;
    const handle = bridge.next_handle;
    const decision = fetch_state.decide(bridge.phase, .{ .open = handle });
    switch (decision.action) {
        .unavailable, .shutting_down => return error.HostStreamUnavailable,
        .applied => {},
        else => unreachable,
    }
    const max = core.max_fetch_request_bytes;
    if (body.len > max or method.len > max or url.len > max or headers.len > max) return error.HostStreamBackpressure;
    const request: struct {
        handle: fetch_state.Handle,
        method: []const u8,
        url: []const u8,
        headers: []const u8,
    } = .{ .handle = handle, .method = method, .url = url, .headers = headers };
    var writer: std.Io.Writer.Allocating = .init(alloc_c);
    defer writer.deinit();
    try std.json.Stringify.value(request, .{}, &writer.writer);
    const metadata = writer.writer.buffered();
    if (metadata.len > max - body.len) return error.HostStreamBackpressure;
    bridge.clearPendingRequest();
    try bridge.request.ensureTotalCapacity(alloc_c, metadata.len);
    try bridge.request_body.ensureTotalCapacity(alloc_c, body.len);
    bridge.request.appendSliceAssumeCapacity(metadata);
    bridge.request_body.appendSliceAssumeCapacity(body);
    bridge.response_offset = 0;
    bridge.response.clearRetainingCapacity();
    bridge.phase = decision.phase;
    bridge.next_handle = if (bridge.next_handle == std.math.maxInt(fetch_state.Handle)) 0 else bridge.next_handle + 1;
    self.fetch_owner = turn;
    if (retired) _ = retireFetchLocked(bridge);
    bridge.wake.broadcast(io);
    bridge.ready.?.notify();
    return handle;
}

/// Retires the published or running fetch, as a cancel does. The caller
/// holds the bridge lock. True if there was one.
fn retireFetchLocked(bridge: *core.FetchBridge) bool {
    const decision = fetch_state.decide(bridge.phase, .cancel);
    if (decision.action != .applied) return false;
    bridge.last_consumed_handle = fetch_state.consumed_after(bridge.phase, bridge.last_consumed_handle, .cancel);
    bridge.clearPendingRequest();
    bridge.phase = decision.phase;
    bridge.wake.broadcast(io_mod.getIo());
    return true;
}

/// The response status, waiting in pulses: 0 means "not yet", so the
/// provider re-checks its cancel flag and deadline between waits instead of
/// waiting for headers for ever (Sol's item 3).
fn fetchStatus(raw: ?*anyopaque, handle: i32, status_out: *u16) i32 {
    const self: *Runtime = @ptrCast(@alignCast(raw.?));
    self.parkIfPanicked();
    const bridge = &self.fetch;
    const io = io_mod.getIo();
    bridge.mutex.lockUncancelable(io);
    self.fault(.fetch_locked);
    defer bridge.mutex.unlock(io);
    switch (bridge.phase) {
        .request_pending, .awaiting_response => |active| {
            if (active != handle) return -2;
            bridge.wake.waitTimeout(io, &bridge.mutex, pulse) catch {};
            if (self.panicked()) {
                bridge.mutex.unlock(io);
                park();
            }
        },
        else => {},
    }
    return switch (bridge.phase) {
        .request_pending, .awaiting_response => |active| if (active == handle) 0 else -2,
        .streaming => |active| if (active == handle) streaming: {
            status_out.* = bridge.status;
            break :streaming 1;
        } else -2,
        .terminal => |terminal| if (terminal.handle != handle) -2 else switch (terminal.kind) {
            .success => success: {
                status_out.* = bridge.status;
                break :success 1;
            },
            .failure => -1,
            .aborted => -2,
        },
        else => -2,
    };
}

fn fetchNext(raw: ?*anyopaque, handle: i32, out: []u8) i32 {
    const self: *Runtime = @ptrCast(@alignCast(raw.?));
    self.parkIfPanicked();
    const read = core.FetchBridge.next(&self.fetch, handle, out);
    self.parkIfPanicked();
    // Room freed after a host met FX_BACKPRESSURE: wake it.
    if (read > 0 and self.push_blocked.swap(false, .acq_rel)) self.ready.notify();
    return read;
}

fn fetchClose(raw: ?*anyopaque, handle: i32) void {
    const self: *Runtime = @ptrCast(@alignCast(raw.?));
    self.parkIfPanicked();
    core.FetchBridge.close(&self.fetch, handle);
}

fn fetchConsumed(raw: ?*anyopaque, handle: i32) void {
    const self: *Runtime = @ptrCast(@alignCast(raw.?));
    self.parkIfPanicked();
    core.FetchBridge.markConsumed(&self.fetch, handle);
}

fn fetchRetryAfter(raw: ?*anyopaque, handle: i32) i32 {
    const self: *Runtime = @ptrCast(@alignCast(raw.?));
    self.parkIfPanicked();
    return core.FetchBridge.retryAfterFn(&self.fetch, handle);
}

// ---------------------------------------------------------------------------
// JSON helpers
// ---------------------------------------------------------------------------

fn errorMessage(err: ?std.json.Value) []const u8 {
    const value = err orelse return "";
    return stringField(value, "message") orelse "the core refused the request";
}

fn objectField(value: std.json.Value, name: []const u8) ?std.json.Value {
    if (value != .object) return null;
    const field = value.object.get(name) orelse return null;
    return if (field == .object) field else null;
}

fn stringField(value: ?std.json.Value, name: []const u8) ?[]const u8 {
    const object = value orelse return null;
    if (object != .object) return null;
    const field = object.object.get(name) orelse return null;
    return if (field == .string) field.string else null;
}

fn arrayField(value: ?std.json.Value, name: []const u8) ?[]const std.json.Value {
    const object = value orelse return null;
    if (object != .object) return null;
    const field = object.object.get(name) orelse return null;
    return if (field == .array) field.array.items else null;
}

/// The status a refused journal open or staging carries, by the core's
/// exact message (as engine.js reads them).
fn journalErrorStatus(message: []const u8) Status {
    if (std.mem.find(u8, message, "newer fx") != null) return FX_E_UNSUPPORTED;
    if (std.mem.find(u8, message, "too large") != null or std.mem.find(u8, message, "holds more than") != null) return FX_E_TOO_LARGE;
    return FX_E_INVALID;
}

fn intField(value: ?std.json.Value, name: []const u8) ?i64 {
    const object = value orelse return null;
    if (object != .object) return null;
    const field = object.object.get(name) orelse return null;
    return if (field == .integer) field.integer else null;
}

fn boolField(value: ?std.json.Value, name: []const u8) bool {
    const object = value orelse return false;
    if (object != .object) return false;
    const field = object.object.get(name) orelse return false;
    return field == .bool and field.bool;
}

fn promptUsage(result: ?std.json.Value) Usage {
    const object = result orelse return .{};
    const usage = objectField(object, "usage") orelse return .{};
    const get = struct {
        fn get(value: std.json.Value, name: []const u8) u64 {
            return @intCast(@max(0, intField(value, name) orelse 0));
        }
    }.get;
    return .{
        .input_tokens = get(usage, "inputTokens"),
        .output_tokens = get(usage, "outputTokens"),
        .cache_read_tokens = get(usage, "cacheReadTokens"),
        .cache_write_tokens = get(usage, "cacheWriteTokens"),
        .reasoning_tokens = get(usage, "reasoningTokens"),
    };
}

// ---------------------------------------------------------------------------
// Generation-tagged handles
// ---------------------------------------------------------------------------

/// One slot per possible runtime. `state` packs the slot's generation (40
/// bits), an occupied bit, a closing bit and the number of API calls inside
/// it. Occupancy, generation and the runtime pointer are published with
/// release and read with acquire, and only an occupied slot of the handle's
/// generation can be entered, so a guessed or stale handle has no effect.
const Slot = struct {
    state: std.atomic.Value(u64) = .init(@as(u64, 1) << gen_shift),
    runtime: std.atomic.Value(?*Runtime) = .init(null),
};

const gen_shift = 24;
const occupied_bit: u64 = 1 << 23;
const closing_bit: u64 = 1 << 22;
const users_mask: u64 = closing_bit - 1;
const max_generation: u64 = (1 << 40) - 1;

var slots: [core.max_active_runtimes]Slot = @splat(.{});
/// A set bit reserves its slot: between claim and publication, while in use,
/// and for good once its generations run out.
var slot_claim: std.atomic.Value(u64) = .init(0);
var active_runtimes: std.atomic.Value(usize) = .init(0);

fn handleOf(index: usize, generation: u64) u64 {
    return (generation << 8) | @as(u64, index + 1);
}

fn slotIndex(handle: u64) ?usize {
    const index = handle & 0xff;
    if (index == 0 or index > slots.len) return null;
    return @intCast(index - 1);
}

fn handleGeneration(handle: u64) ?u64 {
    const generation = handle >> 8;
    if (generation == 0 or generation > max_generation) return null;
    return generation;
}

/// Enters `handle`: returns its runtime with the slot's user count raised, or
/// null, changing nothing, if it is not an open runtime. Pair with `leave`.
fn enter(handle: u64) ?*Runtime {
    const index = slotIndex(handle) orelse return null;
    const generation = handleGeneration(handle) orelse return null;
    const slot = &slots[index];
    var state = slot.state.load(.acquire);
    while (true) {
        if (state >> gen_shift != generation or state & occupied_bit == 0 or state & closing_bit != 0) return null;
        if (state & users_mask == users_mask) return null;
        state = slot.state.cmpxchgWeak(state, state + 1, .acq_rel, .acquire) orelse break;
    }
    return slot.runtime.load(.acquire);
}

fn leave(handle: u64) void {
    const index = slotIndex(handle).?;
    _ = slots[index].state.fetchSub(1, .acq_rel);
}

/// Reserves a usable slot, before anything is started for the runtime
/// (Sol's 2a item 2): with every usable slot taken, create fails first.
fn reserveSlot() ?usize {
    for (0..slots.len) |index| {
        const bit = @as(u64, 1) << @intCast(index);
        if (slot_claim.fetchOr(bit, .acq_rel) & bit == 0) return index;
    }
    return null;
}

/// Gives back a reservation no handle was issued for.
fn releaseSlot(index: usize) void {
    _ = slot_claim.fetchAnd(~(@as(u64, 1) << @intCast(index)), .acq_rel);
}

/// Publishes `runtime` in its reserved slot; returns its handle.
fn publishSlot(index: usize, runtime: *Runtime) u64 {
    const slot = &slots[index];
    const generation = slot.state.load(.acquire) >> gen_shift;
    slot.runtime.store(runtime, .release);
    slot.state.store((generation << gen_shift) | occupied_bit, .release);
    return handleOf(index, generation);
}

/// Starts closing `handle`: no new call enters, and this waits for the calls
/// inside to leave (each is bounded). Returns the runtime, or null, changing
/// nothing, if the handle is not open.
fn beginClose(handle: u64) ?*Runtime {
    const index = slotIndex(handle) orelse return null;
    const generation = handleGeneration(handle) orelse return null;
    const slot = &slots[index];
    var state = slot.state.load(.acquire);
    while (true) {
        if (state >> gen_shift != generation or state & occupied_bit == 0 or state & closing_bit != 0) return null;
        state = slot.state.cmpxchgWeak(state, state | closing_bit, .acq_rel, .acquire) orelse break;
    }
    while (slot.state.load(.acquire) & users_mask != 0) std.Thread.yield() catch {};
    return slot.runtime.load(.acquire);
}

/// Retires `handle`'s slot: the next generation, or never again once the
/// generations run out. Then the runtime no longer counts toward the limit.
fn endClose(handle: u64) void {
    const index = slotIndex(handle).?;
    const slot = &slots[index];
    const generation = slot.state.load(.acquire) >> gen_shift;
    slot.runtime.store(null, .release);
    if (generation >= max_generation) {
        slot.state.store(0, .release);
    } else {
        slot.state.store((generation + 1) << gen_shift, .release);
        _ = slot_claim.fetchAnd(~(@as(u64, 1) << @intCast(index)), .acq_rel);
    }
    _ = active_runtimes.fetchSub(1, .acq_rel);
}

fn claimRuntimeCount() bool {
    var current = active_runtimes.load(.acquire);
    while (current < core.max_active_runtimes) {
        current = active_runtimes.cmpxchgWeak(current, current + 1, .acq_rel, .acquire) orelse return true;
    }
    return false;
}

// ---------------------------------------------------------------------------
// Process-wide io
// ---------------------------------------------------------------------------

var io_state: std.atomic.Value(u8) = .init(0);

/// Starts the core's process-wide io once. A waiter gives up after the busy
/// bound, and a failed start (a panic in it) answers FX_E_FAILED for good.
fn ensureIo() Status {
    if (io_state.load(.acquire) == 2) return FX_OK;
    if (io_state.cmpxchgStrong(0, 1, .acq_rel, .acquire) == null) {
        core.threaded_io = std.Io.Threaded.init(std.heap.c_allocator, .{});
        io_mod.setIo(core.threaded_io.?.io());
        io_mod.setLibcEnviron();
        io_state.store(2, .release);
        return FX_OK;
    }
    const started = nowMs();
    while (true) switch (io_state.load(.acquire)) {
        2 => return FX_OK,
        3 => return FX_E_FAILED,
        else => {
            if (nowMs() -| started > busy_bound_ms) return FX_BUSY;
            std.Thread.yield() catch {};
        },
    };
}

// ---------------------------------------------------------------------------
// Argument helpers
// ---------------------------------------------------------------------------

fn isUtf8(bytes: []const u8) bool {
    return std.unicode.utf8ValidateSlice(bytes);
}

fn validToolName(name: []const u8) bool {
    if (name.len == 0 or name.len > 64) return false;
    for (name) |c| if (!(std.ascii.isAlphanumeric(c) or c == '_' or c == '-')) return false;
    return true;
}

fn validMessageId(id: []const u8) bool {
    if (id.len == 0 or id.len > max_message_id_bytes) return false;
    for (id) |c| if (!(std.ascii.isAlphanumeric(c) or c == '.' or c == '_' or c == ':' or c == '-')) return false;
    return true;
}

/// A bounded, valid UTF-8 copy of `value`, or null if empty.
fn optionalCopy(value: Str, max: usize) error{ Invalid, OutOfMemory }!?[]u8 {
    const bytes = value.slice() orelse return error.Invalid;
    if (bytes.len == 0) return null;
    if (bytes.len > max or !isUtf8(bytes)) return error.Invalid;
    return try alloc_c.dupe(u8, bytes);
}

fn aligned(comptime T: type, address: usize) bool {
    return address != 0 and address % @alignOf(T) == 0;
}

/// Reads a struct the host passed in with its struct_size (header rule 6).
fn readSized(comptime T: type, ptr: ?*align(1) const T, min_size: usize) error{ Invalid, Unsupported }!T {
    const given = ptr orelse return error.Invalid;
    if (!aligned(T, @intFromPtr(given))) return error.Invalid;
    const raw: [*]const u8 = @ptrCast(given);
    const size: usize = std.mem.readInt(u32, raw[0..4], .little);
    if (size < min_size) return error.Invalid;
    if (size > @sizeOf(T)) {
        for (raw[@sizeOf(T)..size]) |byte| if (byte != 0) return error.Unsupported;
    }
    var value: T = std.mem.zeroes(T);
    @memcpy(std.mem.asBytes(&value)[0..@min(size, @sizeOf(T))], raw[0..@min(size, @sizeOf(T))]);
    return value;
}

fn sizedStatus(err: error{ Invalid, Unsupported }) Status {
    return switch (err) {
        error.Invalid => FX_E_INVALID,
        error.Unsupported => FX_E_UNSUPPORTED,
    };
}

/// Takes the runtime lock for a host call that needs a live core.
fn lockLive(runtime: *Runtime) Status {
    if (!runtime.running()) return FX_E_FAILED;
    const got = hostLock(runtime, &runtime.lock);
    if (got != .locked) return acquireStatus(got);
    if (!runtime.running()) {
        runtime.lock.unlock(io_mod.getIo());
        return FX_E_FAILED;
    }
    return FX_OK;
}

fn unlock(runtime: *Runtime) void {
    runtime.lock.unlock(io_mod.getIo());
}

// ---------------------------------------------------------------------------
// Exports: version, runtimes
// ---------------------------------------------------------------------------

export fn fx_core_abi_version() callconv(.c) u32 {
    return abi_version;
}

export fn fx_runtime_create(cfg_ptr: ?*align(1) const Config, host_ptr: ?*align(1) const Host, host_ctx: ?*anyopaque, out: ?*align(1) u64) callconv(.c) Status {
    const out_address = @intFromPtr(out);
    if (!aligned(u64, out_address)) return FX_E_INVALID;
    const out_ptr: *u64 = @ptrFromInt(out_address);
    out_ptr.* = 0;
    var progress: CreateProgress = .{};
    const status = hostCall(null, .{ cfg_ptr, host_ptr, host_ctx, out_ptr, &progress }, createBody);
    if (status == FX_E_FAILED) {
        // A panic unwound create. While starting the process-wide io, it
        // leaves the io failed for good. After the slot was reserved, the
        // reservation and the count go back; a runtime already made is
        // abandoned (it is PANICKED, its threads park), and no handle names it.
        if (progress.initializing_io) io_state.store(3, .release);
        if (progress.slot) |index| if (!progress.published) {
            releaseSlot(index);
            _ = active_runtimes.fetchSub(1, .acq_rel);
            out_ptr.* = 0;
        };
    }
    return status;
}

const CreateProgress = struct {
    initializing_io: bool = false,
    slot: ?usize = null,
    published: bool = false,
};

fn createBody(_: ?*Runtime, args: anytype) Status {
    const cfg = readSized(Config, args[0], @sizeOf(Config)) catch |err| return sizedStatus(err);
    if (cfg.abi_version != abi_version) return FX_E_UNSUPPORTED;
    if (cfg.flags != 0) return FX_E_UNSUPPORTED;
    var host: Host = .{};
    if (args[1] != null) host = readSized(Host, args[1], @sizeOf(Host)) catch |err| return sizedStatus(err);
    const progress: *CreateProgress = args[4];
    if (io_state.load(.acquire) != 2) progress.initializing_io = true;
    const io_status = ensureIo();
    progress.initializing_io = false;
    if (io_status != FX_OK) return io_status;
    if (!claimRuntimeCount()) return FX_E_LIMIT;
    // Slots whose generations ran out lower the limit.
    const index = reserveSlot() orelse {
        _ = active_runtimes.fetchSub(1, .acq_rel);
        return FX_E_LIMIT;
    };
    progress.slot = index;
    const runtime = createRuntime(&cfg, host, args[2]) catch |err| {
        progress.slot = null;
        releaseSlot(index);
        _ = active_runtimes.fetchSub(1, .acq_rel);
        return switch (err) {
            error.Invalid => FX_E_INVALID,
            error.OutOfMemory => FX_E_OOM,
            error.ThreadFailed => FX_E_THREAD,
            error.ReadyChannelFailed => FX_E_IO,
        };
    };
    // From here a panic fails this runtime (it is abandoned, see the export).
    tl_guard.?.runtime = runtime;
    if (faultHit(&create_fault)) std.debug.panic("fault injected at {s}", .{@tagName(Fault.create)});
    args[3].* = publishSlot(index, runtime);
    progress.published = true;
    return FX_OK;
}

const CreateError = error{ Invalid, OutOfMemory, ThreadFailed, ReadyChannelFailed };

fn limit(value: u32, default: u32, max: u32) error{Invalid}!u32 {
    if (value == 0) return default;
    if (value > max) return error.Invalid;
    return value;
}

fn createRuntime(cfg: *const Config, host: Host, host_ctx: ?*anyopaque) CreateError!*Runtime {
    const alloc = alloc_c;
    // Provider.
    const provider_text = cfg.provider.slice() orelse return error.Invalid;
    const provider: ?model_provider.ProviderId = if (provider_text.len == 0 or std.mem.eql(u8, provider_text, "gateway"))
        .gateway
    else if (std.mem.eql(u8, provider_text, "codex"))
        .codex
    else if (std.mem.eql(u8, provider_text, "grok"))
        .grok
    else configured: {
        const parsed = model_provider.parse(provider_text) orelse return error.Invalid;
        if (parsed != .configured) return error.Invalid;
        break :configured parsed;
    };
    const provider_name = try alloc.dupe(u8, if (provider_text.len == 0) "gateway" else provider_text);
    errdefer alloc.free(provider_name);
    const configured_providers = try optionalCopy(cfg.configured_providers_json, max_configured_providers_bytes);
    errdefer if (configured_providers) |json| alloc.free(json);
    if ((provider.? == .configured) != (configured_providers != null)) return error.Invalid;
    const model = try optionalCopy(cfg.model, core.max_model_bytes);
    errdefer if (model) |value| alloc.free(value);
    const effort = try optionalCopy(cfg.effort, core.max_effort_bytes);
    errdefer if (effort) |value| alloc.free(value);
    const home = (try optionalCopy(cfg.home, core.max_path_bytes)) orelse return error.Invalid;
    errdefer alloc.free(home);
    const workspace_root = (try optionalCopy(cfg.workspace_root, core.max_path_bytes)) orelse return error.Invalid;
    errdefer alloc.free(workspace_root);
    const endpoint = try optionalCopy(cfg.endpoint_url, core.max_url_bytes);
    // endpoint_url is the Gateway's route only.
    if (endpoint != null and provider.? != .gateway) {
        alloc.free(endpoint.?);
        return error.Invalid;
    }
    const gateway_chat_url = endpoint orelse try alloc.dupe(u8, builtin_gateway.default_chat_url);
    errdefer alloc.free(gateway_chat_url);
    core.http_config.validateEndpoint(gateway_chat_url) catch return error.Invalid;
    if (!std.mem.eql(u8, gateway_chat_url, builtin_gateway.default_chat_url)) {
        const uri = std.Uri.parse(gateway_chat_url) catch return error.Invalid;
        if (!std.ascii.eqlIgnoreCase(uri.scheme, "http")) return error.Invalid;
    }
    const max_agent_steps = try limit(cfg.max_agent_steps, 64, 10000);
    const max_concurrent_tools = try limit(cfg.max_concurrent_tools, max_concurrent_tools_limit, max_concurrent_tools_limit);
    const max_tool_result_bytes = try limit(cfg.max_tool_result_bytes, 64 * 1024, max_tool_result_limit);
    const max_history_turns = try limit(cfg.max_history_turns, 100, 10000);
    const max_stop_continuations = try limit(cfg.max_stop_continuations, 3, 64);

    // Tools and instructions, as initialize's clientCapabilities.libfx.
    const instructions = cfg.instructions.slice() orelse return error.Invalid;
    if (instructions.len > max_instructions_bytes or !isUtf8(instructions)) return error.Invalid;
    if (cfg.tool_count > max_tools or (cfg.tool_count > 0 and cfg.tools == null)) return error.Invalid;
    if (cfg.tool_count > 0 and !aligned(ToolDesc, @intFromPtr(cfg.tools.?))) return error.Invalid;
    const descs: []const ToolDesc = if (cfg.tool_count == 0) &.{} else cfg.tools.?[0..cfg.tool_count];
    const tools = try alloc.alloc(Tool, descs.len);
    var tools_made: usize = 0;
    errdefer {
        for (tools[0..tools_made]) |tool| alloc.free(tool.name);
        alloc.free(tools);
    }
    var init_frame: std.Io.Writer.Allocating = .init(alloc);
    defer init_frame.deinit();
    const w = &init_frame.writer;
    w.writeAll("{\"jsonrpc\":\"2.0\",\"id\":0,\"method\":\"initialize\",\"params\":{\"protocolVersion\":1,\"clientCapabilities\":{") catch return error.OutOfMemory;
    if (descs.len > 0 or instructions.len > 0) {
        w.writeAll("\"libfx\":{\"tools\":[") catch return error.OutOfMemory;
        for (descs, 0..) |desc, index| {
            const name = desc.name.slice() orelse return error.Invalid;
            const description = desc.description.slice() orelse return error.Invalid;
            const schema = desc.input_schema_json.slice() orelse return error.Invalid;
            if (!validToolName(name)) return error.Invalid;
            if (desc.reserved != 0 or desc.flags & ~(FX_TOOL_IDEMPOTENT | FX_TOOL_WRITES) != 0) return error.Invalid;
            for (tools[0..tools_made]) |tool| if (std.mem.eql(u8, tool.name, name)) return error.Invalid;
            if (description.len > max_tool_text_bytes or !isUtf8(description)) return error.Invalid;
            if (schema.len == 0 or schema.len > max_schema_bytes) return error.Invalid;
            {
                // The schema must be a JSON object; it is written as given.
                const parsed = std.json.parseFromSlice(std.json.Value, alloc, schema, .{}) catch return error.Invalid;
                defer parsed.deinit();
                if (parsed.value != .object) return error.Invalid;
            }
            if (index > 0) w.writeByte(',') catch return error.OutOfMemory;
            w.writeAll("{\"name\":") catch return error.OutOfMemory;
            jsonrpc.writeJsonStr(name, w) catch return error.OutOfMemory;
            w.writeAll(",\"description\":") catch return error.OutOfMemory;
            jsonrpc.writeJsonStr(description, w) catch return error.OutOfMemory;
            w.writeAll(",\"inputSchema\":") catch return error.OutOfMemory;
            w.writeAll(schema) catch return error.OutOfMemory;
            if (desc.flags & FX_TOOL_WRITES != 0) w.writeAll(",\"writes\":true") catch return error.OutOfMemory;
            if (desc.flags & FX_TOOL_IDEMPOTENT != 0) w.writeAll(",\"idempotent\":true") catch return error.OutOfMemory;
            w.writeByte('}') catch return error.OutOfMemory;
            tools[index] = .{ .name = try alloc.dupe(u8, name), .flags = desc.flags };
            tools_made += 1;
        }
        w.writeAll("]") catch return error.OutOfMemory;
        if (instructions.len > 0) {
            w.writeAll(",\"instructions\":") catch return error.OutOfMemory;
            jsonrpc.writeJsonStr(instructions, w) catch return error.OutOfMemory;
        }
        w.writeAll("}") catch return error.OutOfMemory;
    }
    w.writeAll("}}}\n") catch return error.OutOfMemory;
    const initialize_frame = try alloc.dupe(u8, init_frame.written());
    errdefer alloc.free(initialize_frame);

    const runtime = try alloc.create(Runtime);
    errdefer alloc.destroy(runtime);
    var ready = core.ReadyNotifier.init() catch return error.ReadyChannelFailed;
    errdefer ready.deinit();
    runtime.* = .{
        .alloc = alloc,
        .provider = provider,
        .provider_name = provider_name,
        .configured_providers = configured_providers,
        .model = model,
        .effort = effort,
        .home = home,
        .workspace_root = workspace_root,
        .gateway_chat_url = gateway_chat_url,
        .max_agent_steps = max_agent_steps,
        .max_concurrent_tools = max_concurrent_tools,
        .max_tool_result_bytes = max_tool_result_bytes,
        .max_history_turns = max_history_turns,
        .max_stop_continuations = max_stop_continuations,
        .ready = ready,
        .host = host,
        .host_ctx = host_ctx,
        .tools = tools,
        .initialize_frame = initialize_frame,
        .journaled = host.journal_append != null,
    };
    runtime.fetch.ready = &runtime.ready;
    runtime.stream_context = host_stream_provider.initContext(builtin_gateway.buildAgentRequest, .{ .fixed = runtime.gateway_chat_url }, .{
        .context = runtime,
        .open_fn = fetchOpen,
        .status_fn = fetchStatus,
        .next_fn = fetchNext,
        .close_fn = fetchClose,
        .consumed_fn = fetchConsumed,
        .retry_after_fn = fetchRetryAfter,
    });
    runtime.codex_context = .{ .transport = runtime.stream_context.transport };
    runtime.grok_context = .{ .transport = runtime.stream_context.transport };
    if (configured_providers) |json| {
        var context = host_chat_completions.Context.init(alloc, runtime.stream_context.transport, json) catch |err| switch (err) {
            error.OutOfMemory => return error.OutOfMemory,
            else => return error.Invalid,
        };
        errdefer context.deinit(alloc);
        runtime.provider = provider.?.bind(context.registry) catch return error.Invalid;
        runtime.custom_context = context;
    }
    errdefer if (runtime.custom_context) |*context| context.deinit(alloc);
    // initialize is the first frame the core reads; its answer is id 0.
    runtime.pending.put(alloc_c, 0, .{ .kind = .initialize, .request = 0 }) catch return error.OutOfMemory;
    errdefer runtime.pending.deinit(alloc_c);
    runtime.input.write(alloc, initialize_frame) catch return error.OutOfMemory;
    errdefer runtime.input.deinit(alloc);
    runtime.thread = std.Thread.spawn(.{}, Runtime.run, .{runtime}) catch return error.ThreadFailed;
    return runtime;
}

export fn fx_runtime_destroy(handle: u64) callconv(.c) Status {
    var progress: DestroyProgress = .{};
    const status = hostCall(null, .{ handle, &progress }, destroyBody);
    if (progress.runtime) |runtime| if (!progress.retired) {
        // A panic unwound destroy after it began (Sol's 2a item 2): the
        // runtime is PANICKED and abandoned, and its handle is retired all
        // the same, once the callbacks already running have returned.
        runtime.closing.store(true, .seq_cst);
        runtime.waitCallbacks();
        endClose(handle);
        return FX_OK;
    };
    return status;
}

const DestroyProgress = struct {
    runtime: ?*Runtime = null,
    retired: bool = false,
};

fn destroyBody(_: ?*Runtime, args: anytype) Status {
    const handle: u64 = args[0];
    const progress: *DestroyProgress = args[1];
    const runtime = beginClose(handle) orelse return FX_E_CLOSED;
    progress.runtime = runtime;
    // From here a panic on this thread fails this runtime, and the export
    // still retires the handle.
    tl_guard.?.runtime = runtime;
    runtime.closing.store(true, .seq_cst);
    runtime.fault(.destroy);
    // No callback starts after this; one already running finishes first.
    runtime.waitCallbacks();
    if (!runtime.panicked()) shutdown(runtime);
    if (runtime.panicked()) {
        // Abandoned: never joined, freed or closed (header, fx_runtime_destroy).
        runtime.waitCallbacks();
    } else {
        runtime.deinit();
    }
    endClose(handle);
    progress.retired = true;
    return FX_OK;
}

/// Stops a healthy runtime's core and waits for its thread, giving up (and
/// leaving the runtime abandoned) if it panics meanwhile.
fn shutdown(runtime: *Runtime) void {
    const io = io_mod.getIo();
    var events_closed = false;
    var input_closed = false;
    var fetch_closed = false;
    var rounds: usize = 0;
    while (!runtime.thread_done.load(.acquire)) {
        if (runtime.panicked()) return;
        // Wake every core thread that waits on the host; retried until each
        // lock is had (a contended lock is held only briefly, rule 2).
        if (!events_closed and hostLock(runtime, &runtime.events.mutex) == .locked) {
            runtime.events.closed = true;
            runtime.events.space.broadcast(io);
            runtime.events.mutex.unlock(io);
            events_closed = true;
        }
        if (!input_closed and hostLock(runtime, &runtime.input.mutex) == .locked) {
            runtime.input.closed = true;
            runtime.input.wake.broadcast(io);
            runtime.input.mutex.unlock(io);
            input_closed = true;
        }
        if (!fetch_closed and hostLock(runtime, &runtime.fetch.mutex) == .locked) {
            const decision = fetch_state.decide(runtime.fetch.phase, .shutdown);
            runtime.fetch.clearPendingRequest();
            runtime.fetch.phase = decision.phase;
            runtime.fetch.wake.broadcast(io);
            runtime.fetch.mutex.unlock(io);
            fetch_closed = true;
        }
        if (hostLock(runtime, &runtime.lock) == .locked) {
            runtime.calls_changed.broadcast(io);
            runtime.lock.unlock(io);
        }
        if (runtime.thread_done.load(.acquire)) return;
        // The core thread ends within microseconds of its input closing:
        // yield first, then sleep in short steps (close's budget is 0.1 ms).
        rounds += 1;
        if (rounds < 64) {
            std.Thread.yield() catch {};
        } else {
            const ts: std.c.timespec = .{ .sec = 0, .nsec = 50_000 };
            _ = std.c.nanosleep(&ts, null);
        }
    }
}

export fn fx_runtime_ready_fd(handle: u64) callconv(.c) c_int {
    const runtime = enter(handle) orelse return -1;
    defer leave(handle);
    return runtime.ready.reader orelse -1;
}

export fn fx_runtime_info_get(handle: u64, out: ?*align(1) RuntimeInfo) callconv(.c) Status {
    const address = @intFromPtr(out);
    if (!aligned(RuntimeInfo, address)) return FX_E_INVALID;
    const raw: [*]u8 = @ptrFromInt(address);
    const size: usize = std.mem.readInt(u32, raw[0..4], .little);
    if (size < @sizeOf(RuntimeInfo)) return FX_E_INVALID;
    @memset(raw[4..size], 0);
    const runtime = enter(handle) orelse return FX_E_CLOSED;
    defer leave(handle);
    const state = runtime.state.load(.acquire);
    const message = switch (state) {
        FX_RUNTIME_PANICKED => runtime.panic_message.get(),
        FX_RUNTIME_FAILED => runtime.fail_message.get(),
        else => "",
    };
    const info: *RuntimeInfo = @ptrFromInt(address);
    info.state = state;
    info.exit_code = runtime.exit_code.load(.acquire);
    info.message = Str.of(message);
    return FX_OK;
}

// ---------------------------------------------------------------------------
// Exports: sessions and turns
// ---------------------------------------------------------------------------

fn outU64(address: usize) ?*u64 {
    if (!aligned(u64, address)) return null;
    const ptr: *u64 = @ptrFromInt(address);
    ptr.* = 0;
    return ptr;
}

export fn fx_session_open(handle: u64, args_ptr: ?*align(1) const SessionOpenArgs, request_out: ?*align(1) u64) callconv(.c) Status {
    const request = outU64(@intFromPtr(request_out)) orelse return FX_E_INVALID;
    const args = readSized(SessionOpenArgs, args_ptr, @sizeOf(SessionOpenArgs)) catch |err| return sizedStatus(err);
    return hostCall(handle, .{ args, request }, sessionOpenBody);
}

fn sessionOpenBody(runtime: ?*Runtime, a: anytype) Status {
    const self = runtime.?;
    const args: SessionOpenArgs = a[0];
    var status = lockLive(self);
    if (status != FX_OK) return status;
    defer unlock(self);
    const session_id = args.session_id.slice() orelse return FX_E_INVALID;
    if (session_id.len > max_session_id_bytes or !isUtf8(session_id)) return FX_E_INVALID;
    if (args.flags & ~(FX_OPEN_INPUTS_DURABLE | FX_OPEN_IMAGE_STORE) != 0) return FX_E_UNSUPPORTED;
    const journal = args.journal_json.slice() orelse return FX_E_INVALID;
    const checkpoint = args.checkpoint.slice() orelse return FX_E_INVALID;
    // A journal, a checkpoint and the store flags need a journaled session.
    if (!self.journaled and (journal.len > 0 or checkpoint.len > 0 or args.flags != 0)) return FX_E_INVALID;
    if (journal.len > core.agent_checkpoint.max_checkpoint_bytes or checkpoint.len > core.agent_checkpoint.max_checkpoint_bytes) return FX_E_TOO_LARGE;
    if (self.session == .opening or self.session == .open) return FX_E_STATE;
    // The journal and the checkpoint cross as raw bytes beside the frame.
    var attachments: Attachments = .{};
    var params: ?[]u8 = null;
    if (self.journaled) {
        params = journalOpenParams(self, journal, checkpoint, args.flags, &attachments) catch |err| {
            self.dropAttachments(attachments.slice());
            return switch (err) {
                error.OutOfMemory => FX_E_OOM,
                error.TooLarge => FX_E_TOO_LARGE,
                error.Busy => FX_BUSY,
                error.Failed => FX_E_FAILED,
            };
        };
    }
    const id = self.nextRequest();
    if (self.session == .created) {
        // The core holds the session already: open its journal again.
        status = self.sendJournalOpen(id, params.?, attachments, true);
        alloc_c.free(params.?);
        if (status != FX_OK) {
            self.dropAttachments(attachments.slice());
            return status;
        }
    } else {
        var new_params: std.Io.Writer.Allocating = .init(alloc_c);
        defer new_params.deinit();
        const built = build: {
            if (session_id.len == 0) break :build new_params.writer.writeAll("{}");
            new_params.writer.writeAll("{\"sessionId\":") catch |err| break :build err;
            jsonrpc.writeJsonStr(session_id, &new_params.writer) catch |err| break :build err;
            break :build new_params.writer.writeAll("}");
        };
        status = if (built) |_| self.sendRequest(.libfx_new, id, "libfx/new", new_params.written()) else |_| FX_E_OOM;
        if (status != FX_OK) {
            if (params) |owned| alloc_c.free(owned);
            self.dropAttachments(attachments.slice());
            return status;
        }
        // Sent from the core's thread once libfx/new names the session.
        if (self.open_params) |old| alloc_c.free(old);
        self.open_params = params;
        self.open_attachments = attachments;
    }
    self.session = .opening;
    self.open_request = id;
    self.image_store = args.flags & FX_OPEN_IMAGE_STORE != 0;
    a[1].* = id;
    return FX_OK;
}

/// libfx/journal_open's params after the session id: `"inputsDurable":..,
/// "imageStore":..,"journalAttachment":N,"snapshotAttachment":N`, without
/// braces. Attaches the journal and the checkpoint.
fn journalOpenParams(self: *Runtime, journal: []const u8, checkpoint: []const u8, flags: u32, attachments: *Attachments) error{ OutOfMemory, TooLarge, Busy, Failed }![]u8 {
    var out: std.Io.Writer.Allocating = .init(alloc_c);
    defer out.deinit();
    const w = &out.writer;
    w.print("\"inputsDurable\":{s},\"imageStore\":{s}", .{
        if (flags & FX_OPEN_INPUTS_DURABLE != 0) "true" else "false",
        if (flags & FX_OPEN_IMAGE_STORE != 0) "true" else "false",
    }) catch return error.OutOfMemory;
    const parts = [_]struct { []const u8, []const u8 }{ .{ "journalAttachment", journal }, .{ "snapshotAttachment", checkpoint } };
    for (parts) |part| {
        if (part[1].len == 0) continue;
        var id: u32 = 0;
        switch (self.attach(part[1], &id)) {
            FX_OK => {},
            FX_E_TOO_LARGE => return error.TooLarge,
            FX_E_OOM => return error.OutOfMemory,
            FX_BUSY => return error.Busy,
            else => return error.Failed,
        }
        attachments.ids[attachments.count] = id;
        attachments.count += 1;
        w.print(",\"{s}\":{d}", .{ part[0], id }) catch return error.OutOfMemory;
    }
    return alloc_c.dupe(u8, out.written()) catch error.OutOfMemory;
}

export fn fx_session_stage_images(handle: u64, images: ?[*]align(1) const NamedBytes, count: usize, request_out: ?*align(1) u64) callconv(.c) Status {
    const request = outU64(@intFromPtr(request_out)) orelse return FX_E_INVALID;
    if (count > 0 and (images == null or !aligned(NamedBytes, @intFromPtr(images)))) return FX_E_INVALID;
    const list: []const NamedBytes = if (count == 0) &.{} else @as([*]const NamedBytes, @ptrCast(@alignCast(images.?)))[0..count];
    return hostCall(handle, .{ list, request }, stageBody);
}

fn stageBody(runtime: ?*Runtime, a: anytype) Status {
    const self = runtime.?;
    const list: []const NamedBytes = a[0];
    var status = lockLive(self);
    if (status != FX_OK) return status;
    defer unlock(self);
    if (list.len == 0) return FX_E_INVALID;
    if (list.len > max_stage_batch) return FX_E_TOO_LARGE;
    var total: usize = 0;
    for (list) |item| {
        const name = item.name.slice() orelse return FX_E_INVALID;
        const bytes = item.bytes.slice() orelse return FX_E_INVALID;
        if (session_codec.imageName(name) == null or bytes.len == 0) return FX_E_INVALID;
        if (bytes.len > max_image_bytes) return FX_E_TOO_LARGE;
        total += bytes.len;
    }
    if (total > max_stage_batch_bytes) return FX_E_TOO_LARGE;
    // Staging serves the next open of a session whose open named images.
    if (self.session != .created) return FX_E_STATE;
    var attachments: Attachments = .{};
    var params: std.Io.Writer.Allocating = .init(alloc_c);
    defer params.deinit();
    const w = &params.writer;
    const built = build: {
        w.writeAll("{\"sessionId\":") catch break :build false;
        jsonrpc.writeJsonStr(self.session_id orelse "", w) catch break :build false;
        w.writeAll(",\"images\":[") catch break :build false;
        break :build true;
    };
    if (!built) return FX_E_OOM;
    for (list, 0..) |item, index| {
        var id: u32 = 0;
        status = self.attach(item.bytes.slice().?, &id);
        if (status != FX_OK) {
            self.dropAttachments(attachments.slice());
            return status;
        }
        attachments.ids[attachments.count] = id;
        attachments.count += 1;
        w.print("{s}{{\"name\":\"{s}\",\"attachment\":{d}}}", .{ if (index > 0) "," else "", item.name.slice().?, id }) catch {
            self.dropAttachments(attachments.slice());
            return FX_E_OOM;
        };
    }
    w.writeAll("]}") catch {
        self.dropAttachments(attachments.slice());
        return FX_E_OOM;
    };
    const id = self.nextRequest();
    status = self.sendRequestWith(.journal_images, id, "libfx/journal_images", params.written(), attachments, .host);
    if (status != FX_OK) {
        self.dropAttachments(attachments.slice());
        return status;
    }
    a[1].* = id;
    return FX_OK;
}

const PromptArgs = struct { input: *const Input, message_id: Str, turn: *u64 };

export fn fx_turn_prompt(handle: u64, input_ptr: ?*align(1) const Input, message_id: Str, turn_out: ?*align(1) u64) callconv(.c) Status {
    const turn = outU64(@intFromPtr(turn_out)) orelse return FX_E_INVALID;
    if (input_ptr == null or !aligned(Input, @intFromPtr(input_ptr))) return FX_E_INVALID;
    const args: PromptArgs = .{ .input = @ptrCast(@alignCast(input_ptr.?)), .message_id = message_id, .turn = turn };
    return hostCall(handle, args, promptBody);
}

fn promptBody(runtime: ?*Runtime, args: PromptArgs) Status {
    const self = runtime.?;
    var status = lockLive(self);
    if (status != FX_OK) return status;
    defer unlock(self);
    const text = args.input.text.slice() orelse return FX_E_INVALID;
    if (!isUtf8(text)) return FX_E_INVALID;
    if (text.len > max_prompt_text_bytes) return FX_E_TOO_LARGE;
    const id = args.message_id.slice() orelse return FX_E_INVALID;
    if (!validMessageId(id)) return FX_E_INVALID;
    const images = inputImages(args.input) orelse return FX_E_INVALID;
    checkImages(images, max_prompt_images, max_prompt_images_bytes) catch |err| return switch (err) {
        error.Invalid => FX_E_INVALID,
        error.TooLarge => FX_E_TOO_LARGE,
        error.OutOfMemory => FX_E_OOM,
    };
    if (text.len == 0 and images.len == 0) return FX_E_INVALID;
    if (self.session != .open or self.turn != 0 or self.journal_failed != null) return FX_E_STATE;
    // A retry of the turn that ended last runs nothing ("Deduplication").
    if (self.last_turn_id) |last| if (std.mem.eql(u8, last, id)) return repeatTurn(self, id, args.turn);
    // The open turn's own id continues it, as fx_turn_resume does.
    if (self.resumable) if (self.open_turn_id) |open| if (std.mem.eql(u8, open, id)) return resumeTurn(self, args.turn);
    var attachments: Attachments = .{};
    var params: std.Io.Writer.Allocating = .init(alloc_c);
    defer params.deinit();
    const w = &params.writer;
    const built = build: {
        w.writeAll("{\"sessionId\":") catch break :build false;
        jsonrpc.writeJsonStr(self.session_id orelse "", w) catch break :build false;
        w.writeAll(",\"prompt\":[") catch break :build false;
        if (text.len > 0) {
            w.writeAll("{\"type\":\"text\",\"text\":") catch break :build false;
            jsonrpc.writeJsonStr(text, w) catch break :build false;
            w.writeAll("}") catch break :build false;
        }
        break :build true;
    };
    if (!built) return FX_E_OOM;
    // Images cross raw beside the frame (P-IMAGES); an image store names them.
    for (images, 0..) |image, index| {
        var attachment: u32 = 0;
        status = self.attach(image.data.slice().?, &attachment);
        if (status != FX_OK) {
            self.dropAttachments(attachments.slice());
            return status;
        }
        attachments.ids[attachments.count] = attachment;
        attachments.count += 1;
        const written = written: {
            w.print("{s}{{\"type\":\"image\",\"mimeType\":\"{s}\",\"_meta\":{{\"fx\":{{\"attachment\":{d}", .{
                if (index > 0 or text.len > 0) "," else "",
                image.mime.slice().?,
                attachment,
            }) catch break :written false;
            if (self.image_store) {
                const name = session_codec.imageNameOf(image.data.slice().?);
                w.print(",\"stored\":\"{s}\"", .{&name}) catch break :written false;
            }
            w.writeAll("}}}") catch break :written false;
            break :written true;
        };
        if (!written) {
            self.dropAttachments(attachments.slice());
            return FX_E_OOM;
        }
    }
    const closed = close: {
        w.writeAll("]") catch break :close false;
        if (self.journaled) {
            w.writeAll(",\"_meta\":{\"fx\":{\"turnId\":") catch break :close false;
            jsonrpc.writeJsonStr(id, w) catch break :close false;
            w.writeAll("}}") catch break :close false;
        }
        w.writeAll("}") catch break :close false;
        break :close true;
    };
    if (!closed) {
        self.dropAttachments(attachments.slice());
        return FX_E_OOM;
    }
    status = startTurn(self, params.written(), id, 0, attachments, args.turn);
    if (status != FX_OK) self.dropAttachments(attachments.slice());
    return status;
}

/// The input's images, or null for a malformed array.
fn inputImages(input: *const Input) ?[]const Image {
    if (input.image_count == 0) return &.{};
    const images = input.images orelse return null;
    if (!aligned(Image, @intFromPtr(images))) return null;
    return images[0..input.image_count];
}

/// Sends the turn's session/prompt and queues its TURN_START. The caller
/// holds `lock`. TURN_START's room is had before the turn starts, so a
/// started turn is always reported and FX_BUSY or FX_E_OOM start nothing.
/// Lock order: `lock`, the event lock, the input lock.
fn startTurn(self: *Runtime, params: []const u8, message_id: []const u8, flags: u32, attachments: Attachments, turn_out: *u64) Status {
    const io = io_mod.getIo();
    const queue = &self.events;
    const owned_id = alloc_c.dupe(u8, message_id) catch return FX_E_OOM;
    const got = hostLock(self, &queue.mutex);
    if (got != .locked) {
        alloc_c.free(owned_id);
        return acquireStatus(got);
    }
    defer queue.mutex.unlock(io);
    const ready = ready: {
        if (queue.closed) break :ready FX_E_FAILED;
        queue.events.ensureUnusedCapacity(alloc_c, 1) catch break :ready FX_E_OOM;
        queue.pending_bytes.ensureUnusedCapacity(alloc_c, message_id.len) catch break :ready FX_E_OOM;
        break :ready FX_OK;
    };
    if (ready != FX_OK) {
        alloc_c.free(owned_id);
        return ready;
    }
    const turn = self.nextRequest();
    const status = self.sendRequestWith(.prompt, turn, "session/prompt", params, attachments, .host);
    if (status != FX_OK) {
        alloc_c.free(owned_id);
        return status;
    }
    self.turn = turn;
    if (self.turn_message_id) |old| alloc_c.free(old);
    self.turn_message_id = owned_id;
    // Queued before `lock` is released: the turn's first update waits for
    // `lock` to learn its turn, so TURN_START is always first.
    const was_empty = queue.appendLocked(.{ .kind = .turn_start, .turn = turn, .flags = flags }, .{ .text = message_id }) catch unreachable;
    if (was_empty) self.ready.notify();
    turn_out.* = turn;
    return FX_OK;
}

/// A prompt repeating the last ended turn's id: TURN_START flagged
/// FX_TURN_REPEATED, then its TURN_END; the core runs nothing. The caller
/// holds `lock`.
fn repeatTurn(self: *Runtime, message_id: []const u8, turn_out: *u64) Status {
    const io = io_mod.getIo();
    const queue = &self.events;
    const got = hostLock(self, &queue.mutex);
    if (got != .locked) return acquireStatus(got);
    defer queue.mutex.unlock(io);
    if (queue.closed) return FX_E_FAILED;
    queue.events.ensureUnusedCapacity(alloc_c, 2) catch return FX_E_OOM;
    queue.pending_bytes.ensureUnusedCapacity(alloc_c, message_id.len) catch return FX_E_OOM;
    const turn = self.nextRequest();
    const was_empty = queue.appendLocked(.{ .kind = .turn_start, .turn = turn, .flags = FX_TURN_REPEATED }, .{ .text = message_id }) catch unreachable;
    _ = queue.appendLocked(.{ .kind = .turn_end, .turn = turn, .code = @intFromEnum(StopReason.end_turn) }, .{}) catch unreachable;
    if (was_empty) self.ready.notify();
    turn_out.* = turn;
    return FX_OK;
}

/// Continues the turn a crash left open. The caller holds `lock`. A call it
/// left running is never run again: the model reads that it may have run.
fn resumeTurn(self: *Runtime, turn_out: *u64) Status {
    const open = self.open_turn_id orelse return FX_E_STATE;
    var params: std.Io.Writer.Allocating = .init(alloc_c);
    defer params.deinit();
    const w = &params.writer;
    w.writeAll("{\"sessionId\":") catch return FX_E_OOM;
    jsonrpc.writeJsonStr(self.session_id orelse "", w) catch return FX_E_OOM;
    w.writeAll(",\"prompt\":[],\"_meta\":{\"fx\":{\"continueRecovery\":true,\"freshOutput\":true}}}") catch return FX_E_OOM;
    const status = startTurn(self, params.written(), open, FX_TURN_RESUMED, .{}, turn_out);
    if (status == FX_OK) self.resumable = false;
    return status;
}

export fn fx_turn_resume(handle: u64, turn_out: ?*align(1) u64) callconv(.c) Status {
    const turn = outU64(@intFromPtr(turn_out)) orelse return FX_E_INVALID;
    return hostCall(handle, .{turn}, struct {
        fn body(runtime: ?*Runtime, a: anytype) Status {
            const self = runtime.?;
            const status = lockLive(self);
            if (status != FX_OK) return status;
            defer unlock(self);
            if (self.session != .open or self.turn != 0 or self.journal_failed != null) return FX_E_STATE;
            if (!self.resumable) return FX_E_STATE;
            return resumeTurn(self, a[0]);
        }
    }.body);
}

const SteerArgs = struct { turn: u64, input: *const Input, message_id: Str, request: *u64 };

export fn fx_turn_steer(handle: u64, turn: u64, input_ptr: ?*align(1) const Input, message_id: Str, request_out: ?*align(1) u64) callconv(.c) Status {
    const request = outU64(@intFromPtr(request_out)) orelse return FX_E_INVALID;
    if (input_ptr == null or !aligned(Input, @intFromPtr(input_ptr))) return FX_E_INVALID;
    const args: SteerArgs = .{ .turn = turn, .input = @ptrCast(@alignCast(input_ptr.?)), .message_id = message_id, .request = request };
    return hostCall(handle, args, steerBody);
}

fn steerBody(runtime: ?*Runtime, args: SteerArgs) Status {
    const self = runtime.?;
    var status = lockLive(self);
    if (status != FX_OK) return status;
    defer unlock(self);
    const text = args.input.text.slice() orelse return FX_E_INVALID;
    if (!isUtf8(text)) return FX_E_INVALID;
    if (text.len > max_steer_text_bytes) return FX_E_TOO_LARGE;
    const id = args.message_id.slice() orelse return FX_E_INVALID;
    if (!validMessageId(id)) return FX_E_INVALID;
    const images = inputImages(args.input) orelse return FX_E_INVALID;
    checkImages(images, max_prompt_images, max_prompt_images_bytes) catch |err| return switch (err) {
        error.Invalid => FX_E_INVALID,
        error.TooLarge => FX_E_TOO_LARGE,
        error.OutOfMemory => FX_E_OOM,
    };
    if (text.len == 0 and images.len == 0) return FX_E_INVALID;
    if (self.session != .open) return FX_E_STATE;
    // A steer for a turn that is not running reaches nothing (Sol's item 3).
    if (args.turn == 0 or args.turn != self.turn) return FX_STALE;
    // A steer with images cannot join: the turn hands off at its next safe
    // point and the steer comes back to be sent as a prompt (P-IMAGES). The
    // host keeps its images; the core never sees them.
    const handoff = images.len > 0;
    const owned_id = alloc_c.dupe(u8, id) catch return FX_E_OOM;
    self.steers.append(alloc_c, .{ .request = 0, .message_id = owned_id, .handoff = handoff }) catch {
        alloc_c.free(owned_id);
        return FX_E_OOM;
    };
    var extra: std.Io.Writer.Allocating = .init(alloc_c);
    defer extra.deinit();
    const built = build: {
        extra.writer.writeAll("{\"sessionId\":") catch break :build false;
        jsonrpc.writeJsonStr(self.session_id orelse "", &extra.writer) catch break :build false;
        if (!handoff) {
            extra.writer.writeAll(",\"text\":") catch break :build false;
            jsonrpc.writeJsonStr(text, &extra.writer) catch break :build false;
            extra.writer.writeAll(",\"id\":") catch break :build false;
            jsonrpc.writeJsonStr(id, &extra.writer) catch break :build false;
        }
        extra.writer.writeAll("}") catch break :build false;
        break :build true;
    };
    const request = self.nextRequest();
    status = if (!built)
        FX_E_OOM
    else if (handoff)
        self.sendRequest(.handoff, request, "libfx/handoff", extra.written())
    else
        self.sendRequest(.steer, request, "libfx/steer", extra.written());
    if (status != FX_OK) {
        _ = self.steers.pop();
        alloc_c.free(owned_id);
        return status;
    }
    self.steers.items[self.steers.items.len - 1].request = request;
    args.request.* = request;
    return FX_OK;
}

const WithdrawArgs = struct { turn: u64, message_id: Str, request: *u64 };

export fn fx_turn_withdraw(handle: u64, turn: u64, message_id: Str, request_out: ?*align(1) u64) callconv(.c) Status {
    const request = outU64(@intFromPtr(request_out)) orelse return FX_E_INVALID;
    return hostCall(handle, WithdrawArgs{ .turn = turn, .message_id = message_id, .request = request }, withdrawBody);
}

fn withdrawBody(runtime: ?*Runtime, args: WithdrawArgs) Status {
    const self = runtime.?;
    var status = lockLive(self);
    if (status != FX_OK) return status;
    defer unlock(self);
    const id = args.message_id.slice() orelse return FX_E_INVALID;
    if (!validMessageId(id)) return FX_E_INVALID;
    if (self.session != .open) return FX_E_STATE;
    if (args.turn == 0 or args.turn != self.turn) return FX_STALE;
    var extra: std.Io.Writer.Allocating = .init(alloc_c);
    defer extra.deinit();
    extra.writer.writeAll("{\"sessionId\":") catch return FX_E_OOM;
    jsonrpc.writeJsonStr(self.session_id orelse "", &extra.writer) catch return FX_E_OOM;
    extra.writer.writeAll(",\"id\":") catch return FX_E_OOM;
    jsonrpc.writeJsonStr(id, &extra.writer) catch return FX_E_OOM;
    extra.writer.writeAll("}") catch return FX_E_OOM;
    const request = self.nextRequest();
    status = self.sendRequest(.withdraw, request, "libfx/withdraw", extra.written());
    if (status != FX_OK) return status;
    // A withdrawn steer no longer comes back at the turn's end.
    for (self.steers.items, 0..) |steer, index| if (std.mem.eql(u8, steer.message_id, id)) {
        alloc_c.free(steer.message_id);
        _ = self.steers.orderedRemove(index);
        break;
    };
    args.request.* = request;
    return FX_OK;
}

export fn fx_turn_cancel(handle: u64, turn: u64) callconv(.c) Status {
    return hostCall(handle, .{turn}, cancelBody);
}

/// Every lock the cancel needs is taken before anything changes, so FX_BUSY
/// and every error change nothing (Sol's 2a item 3). Lock order: `lock`,
/// then the bridge lock, then the input lock.
fn cancelBody(runtime: ?*Runtime, args: anytype) Status {
    const self = runtime.?;
    const turn: u64 = args[0];
    const status = lockLive(self);
    if (status != FX_OK) return status;
    defer unlock(self);
    if (turn == 0 or turn != self.turn or self.session_id == null) return FX_STALE;
    // Cancelled already: nothing more to do.
    if (self.cancelled_turn == turn) return FX_OK;
    const io = io_mod.getIo();
    var frame: std.Io.Writer.Allocating = .init(alloc_c);
    defer frame.deinit();
    frame.writer.writeAll("{\"jsonrpc\":\"2.0\",\"method\":\"session/cancel\",\"params\":{\"sessionId\":") catch return FX_E_OOM;
    jsonrpc.writeJsonStr(self.session_id.?, &frame.writer) catch return FX_E_OOM;
    frame.writer.writeAll("}}\n") catch return FX_E_OOM;
    const bridge = &self.fetch;
    const got_fetch = hostLock(self, &bridge.mutex);
    if (got_fetch != .locked) return acquireStatus(got_fetch);
    defer bridge.mutex.unlock(io);
    const got_input = hostLock(self, &self.input.mutex);
    if (got_input != .locked) return acquireStatus(got_input);
    const sent = self.inputLocked(frame.written(), .control);
    self.input.mutex.unlock(io);
    if (sent != FX_OK) return sent;
    // Applied: nothing below can fail.
    self.cancelled_turn = turn;
    // The turn's tool calls get tool_cancel from their threads.
    var calls = self.calls.valueIterator();
    while (calls.next()) |record| if (record.*.turn == turn) {
        record.*.cancel_requested = true;
    };
    self.calls_changed.broadcast(io);
    // The turn's fetch is retired now, even while the core waits for its
    // response headers; the host sees RETIRED after the wake. One the core
    // publishes later for this turn is published retired (fetchOpen).
    if (self.fetch_owner == turn and retireFetchLocked(bridge)) self.ready.notify();
    return FX_OK;
}

export fn fx_checkpoint(handle: u64, request_out: ?*align(1) u64) callconv(.c) Status {
    const request = outU64(@intFromPtr(request_out)) orelse return FX_E_INVALID;
    return hostCall(handle, .{request}, struct {
        fn body(runtime: ?*Runtime, a: anytype) Status {
            const self = runtime.?;
            var status = lockLive(self);
            if (status != FX_OK) return status;
            defer unlock(self);
            if (self.session != .open or !self.journaled or self.journal_failed != null) return FX_E_STATE;
            // libfx's journal snapshot: what the host stores beside the events.
            var params: std.Io.Writer.Allocating = .init(alloc_c);
            defer params.deinit();
            params.writer.writeAll("{\"sessionId\":") catch return FX_E_OOM;
            jsonrpc.writeJsonStr(self.session_id orelse "", &params.writer) catch return FX_E_OOM;
            params.writer.writeAll("}") catch return FX_E_OOM;
            const id = self.nextRequest();
            status = self.sendRequest(.snapshot, id, "libfx/snapshot", params.written());
            if (status != FX_OK) return status;
            a[0].* = id;
            return FX_OK;
        }
    }.body);
}

// ---------------------------------------------------------------------------
// Exports: events
// ---------------------------------------------------------------------------

const DrainArgs = struct { buf: [*]u8, cap: usize, stride: usize, count: *usize };

export fn fx_events_drain(handle: u64, buf: ?*anyopaque, cap: usize, event_size: usize, count_out: ?*align(1) usize) callconv(.c) Status {
    const count_address = @intFromPtr(count_out);
    if (!aligned(usize, count_address)) return FX_E_INVALID;
    const count: *usize = @ptrFromInt(count_address);
    count.* = 0;
    if (event_size < @sizeOf(Event) or event_size % 8 != 0) return FX_E_INVALID;
    if (cap > 0 and !aligned(Event, @intFromPtr(buf))) return FX_E_INVALID;
    if (cap > 0 and cap > std.math.maxInt(usize) / event_size) return FX_E_INVALID;
    const base: [*]u8 = if (cap > 0) @ptrCast(buf.?) else @ptrFromInt(@alignOf(Event));
    return hostCall(handle, DrainArgs{ .buf = base, .cap = cap, .stride = event_size, .count = count }, drainBody);
}

fn writeEvent(args: DrainArgs, index: usize, event: Event) void {
    const slot = args.buf + index * args.stride;
    @memset(slot[0..args.stride], 0);
    const typed: *Event = @ptrCast(@alignCast(slot));
    typed.* = event;
}

fn exitedEvent(self: *Runtime, flags: u32) Event {
    const state = self.state.load(.acquire);
    const message = switch (state) {
        FX_RUNTIME_PANICKED => self.panic_message.get(),
        FX_RUNTIME_FAILED => self.fail_message.get(),
        else => "",
    };
    return .{
        .kind = @intFromEnum(EventKind.exited),
        .flags = flags,
        .code = state,
        .status = self.exit_code.load(.acquire),
        .text = Str.of(message),
    };
}

fn drainBody(runtime: ?*Runtime, args: DrainArgs) Status {
    const self = runtime.?;
    const io = io_mod.getIo();
    const queue = &self.events;
    const got = hostLock(self, &queue.mutex);
    switch (got) {
        .locked => {},
        .busy => return FX_BUSY,
        .failed => {
            // The core panicked holding the queue: what it held is lost.
            if (args.cap > 0 and !self.exited_delivered.swap(true, .acq_rel)) {
                writeEvent(args, 0, exitedEvent(self, FX_EXITED_EVENTS_LOST));
                args.count.* = 1;
            }
            return FX_OK;
        },
    }
    defer queue.mutex.unlock(io);
    const count = @min(args.cap, queue.events.items.len);
    if (count == 0) {
        // An empty drain ends the previous drain's borrows too.
        queue.drained_bytes.clearRetainingCapacity();
        queue.drained_names.clearRetainingCapacity();
        queue.drained_name_strs.clearRetainingCapacity();
    } else {
        const everything = count == queue.events.items.len;
        // Everything that can fail comes first, in the spare storage.
        queue.spare_bytes.clearRetainingCapacity();
        queue.spare_names.clearRetainingCapacity();
        queue.spare_name_strs.clearRetainingCapacity();
        if (!everything) {
            // A partial drain copies what it hands out; the rest stays.
            queue.spare_bytes.appendSlice(alloc_c, queue.pending_bytes.items) catch return FX_E_OOM;
            queue.spare_names.appendSlice(alloc_c, queue.pending_names.items) catch return FX_E_OOM;
        }
        var name_total: usize = 0;
        for (queue.events.items[0..count]) |event| name_total += event.name_count;
        queue.spare_name_strs.ensureTotalCapacity(alloc_c, name_total) catch return FX_E_OOM;
        if (self.faultOom(.drain_oom)) return FX_E_OOM;
        // A whole drain hands out the pending arena itself (no copy); its
        // buffers keep their addresses when the lists are swapped below.
        const arena = if (everything) queue.pending_bytes.items else queue.spare_bytes.items;
        const spans = if (everything) queue.pending_names.items else queue.spare_names.items;
        for (queue.events.items[0..count], 0..) |event, index| {
            var names: ?[*]const Str = null;
            if (event.name_count > 0) {
                const first = queue.spare_name_strs.items.len;
                for (spans[event.names_start..][0..event.name_count]) |span| {
                    queue.spare_name_strs.appendAssumeCapacity(EventQueue.spanStr(arena, span));
                }
                names = queue.spare_name_strs.items[first..].ptr;
            }
            writeEvent(args, index, .{
                .kind = @intFromEnum(event.kind),
                .flags = event.flags,
                .turn = event.turn,
                .request = event.request,
                .id = event.id,
                .status = event.status,
                .code = event.code,
                .text = EventQueue.spanStr(arena, event.text),
                .text2 = EventQueue.spanStr(arena, event.text2),
                .bytes = .{ .ptr = if (event.bytes.len == 0) null else arena[event.bytes.off..].ptr, .len = event.bytes.len },
                .names = names,
                .name_count = event.name_count,
                .usage = event.usage,
            });
        }
        // Applied: the previous drain's borrows end here.
        if (everything) {
            std.mem.swap(std.ArrayList(u8), &queue.pending_bytes, &queue.drained_bytes);
            std.mem.swap(std.ArrayList(Span), &queue.pending_names, &queue.drained_names);
            queue.pending_bytes.clearRetainingCapacity();
            queue.pending_names.clearRetainingCapacity();
            queue.events.clearRetainingCapacity();
        } else {
            std.mem.swap(std.ArrayList(u8), &queue.spare_bytes, &queue.drained_bytes);
            std.mem.swap(std.ArrayList(Span), &queue.spare_names, &queue.drained_names);
            const rest = queue.events.items.len - count;
            std.mem.copyForwards(QueuedEvent, queue.events.items[0..rest], queue.events.items[count..]);
            queue.events.items.len = rest;
        }
        std.mem.swap(std.ArrayList(Str), &queue.spare_name_strs, &queue.drained_name_strs);
        queue.space.broadcast(io);
    }
    var written = count;
    // Once the runtime stopped and everything before it is out: EXITED, once.
    if (written < args.cap and queue.events.items.len == 0 and !self.running() and
        (self.panicked() or self.thread_done.load(.acquire) or self.state.load(.acquire) == FX_RUNTIME_FAILED))
    {
        if (!self.exited_delivered.swap(true, .acq_rel)) {
            writeEvent(args, written, exitedEvent(self, 0));
            written += 1;
            // EXITED is the last event: nothing queues after it.
            queue.closed = true;
            queue.space.broadcast(io);
        }
    }
    args.count.* = written;
    return FX_OK;
}

// ---------------------------------------------------------------------------
// Exports: tool results
// ---------------------------------------------------------------------------

const ToolResultArgs = struct { call: u64, out: *const ToolOutput };

export fn fx_tool_result(handle: u64, call: u64, out_ptr: ?*align(1) const ToolOutput) callconv(.c) Status {
    if (out_ptr == null or !aligned(ToolOutput, @intFromPtr(out_ptr))) return FX_E_INVALID;
    return hostCall(handle, ToolResultArgs{ .call = call, .out = @ptrCast(@alignCast(out_ptr.?)) }, toolResultBody);
}

fn toolResultBody(runtime: ?*Runtime, args: ToolResultArgs) Status {
    const self = runtime.?;
    const status = lockLive(self);
    if (status != FX_OK) return status;
    defer unlock(self);
    self.fault(.tool_result);
    const text = args.out.text.slice() orelse return FX_E_INVALID;
    if (args.out.image_count > 0 and args.out.images == null) return FX_E_INVALID;
    const record = self.calls.get(args.call) orelse
        return if (args.call == 0 or args.call >= self.next_call) FX_E_INVALID else FX_STALE;
    if (record.answered) return FX_STALE;
    if (!isUtf8(text)) return FX_E_INVALID;
    const raw_images: []const Image = if (args.out.image_count == 0) &.{} else images: {
        if (!aligned(Image, @intFromPtr(args.out.images.?))) return FX_E_INVALID;
        break :images args.out.images.?[0..args.out.image_count];
    };
    var outcome: Status = FX_OK;
    var images: ?[]core_types.ToolImage = null;
    if (raw_images.len > 0 and text.len <= self.max_tool_result_bytes) {
        images = toolImages(raw_images, self.image_store) catch |err| switch (err) {
            error.Invalid => return FX_E_INVALID,
            error.OutOfMemory => return FX_E_OOM,
            error.TooLarge => null,
        };
        if (images == null) outcome = FX_E_TOO_LARGE;
    }
    const source = if (text.len > self.max_tool_result_bytes)
        "Host tool result exceeded the configured limit"
    else if (outcome == FX_E_TOO_LARGE)
        "Host tool images exceeded the limit"
    else
        text;
    if (source.ptr != text.ptr) outcome = FX_E_TOO_LARGE;
    // The one copy of the host's result (header rule 7).
    const copy = alloc_c.dupe(u8, source) catch {
        if (images) |list| core_types.freeToolImages(alloc_c, list);
        return FX_E_OOM;
    };
    record.is_error = if (outcome == FX_E_TOO_LARGE) true else args.out.is_error;
    record.result = copy;
    record.images = images;
    record.answered = true;
    self.calls_changed.broadcast(io_mod.getIo());
    return outcome;
}

/// The image type `bytes` start as, or null.
fn imageMime(bytes: []const u8) ?[]const u8 {
    return image_data.detectMediaTypeFromBytes(bytes);
}

const ImageError = error{ Invalid, TooLarge, OutOfMemory };

/// Checks host images: each a supported type whose bytes are that type,
/// within the header's FX_MAX_IMAGE_BYTES and the core's model bounds.
fn checkImages(images: []const Image, max_count: usize, max_total: usize) ImageError!void {
    if (images.len > max_count) return error.TooLarge;
    var total: usize = 0;
    for (images) |image| {
        const mime = image.mime.slice() orelse return error.Invalid;
        const data = image.data.slice() orelse return error.Invalid;
        if (!image_data.supportedMediaType(mime) or data.len == 0) return error.Invalid;
        const detected = imageMime(data) orelse return error.Invalid;
        if (!std.mem.eql(u8, detected, mime)) return error.Invalid;
        if (data.len > max_image_bytes or data.len > max_model_image_bytes) return error.TooLarge;
        total += data.len;
    }
    if (total > max_total) return error.TooLarge;
}

/// A tool result's images in the core's own form: base64, encoded once from
/// the host's raw bytes (the core never sees them otherwise), named when the
/// host keeps an image store.
fn toolImages(images: []const Image, named: bool) ImageError![]core_types.ToolImage {
    try checkImages(images, image_data.max_tool_images, image_data.max_result_frame_bytes / 4 * 3);
    const list = try alloc_c.alloc(core_types.ToolImage, images.len);
    var made: usize = 0;
    errdefer {
        for (list[0..made]) |image| {
            alloc_c.free(image.data);
            alloc_c.free(image.mime_type);
        }
        alloc_c.free(list);
    }
    const encoder = std.base64.standard.Encoder;
    for (images) |image| {
        const data = image.data.slice().?;
        const encoded = try alloc_c.alloc(u8, encoder.calcSize(data.len));
        errdefer alloc_c.free(encoded);
        _ = encoder.encode(encoded, data);
        const mime = try alloc_c.dupe(u8, image.mime.slice().?);
        list[made] = .{ .data = encoded, .mime_type = mime, .name = if (named) session_codec.imageNameOf(data) else null };
        made += 1;
    }
    return list;
}

// ---------------------------------------------------------------------------
// Exports: the fetch bridge's host side
// ---------------------------------------------------------------------------

fn fetchHandle(fetch: u64) ?fetch_state.Handle {
    if (fetch == 0 or fetch > std.math.maxInt(fetch_state.Handle)) return null;
    return @intCast(fetch);
}

/// Headers that carry credentials never leave the core (header rule 13).
fn credentialHeader(name: []const u8) bool {
    const names = [_][]const u8{ "authorization", "proxy-authorization", "cookie", "x-api-key", "api-key", "x-goog-api-key" };
    for (names) |candidate| if (std.ascii.eqlIgnoreCase(name, candidate)) return true;
    return false;
}

export fn fx_fetch_take(handle: u64, out_ptr: ?*align(1) FetchRequest) callconv(.c) Status {
    const address = @intFromPtr(out_ptr);
    if (!aligned(FetchRequest, address)) return FX_E_INVALID;
    const raw: [*]u8 = @ptrFromInt(address);
    const size: usize = std.mem.readInt(u32, raw[0..4], .little);
    if (size < @sizeOf(FetchRequest)) return FX_E_INVALID;
    @memset(raw[4..size], 0);
    return hostCall(handle, .{@as(*FetchRequest, @ptrFromInt(address))}, fetchTakeBody);
}

fn fetchTakeBody(runtime: ?*Runtime, args: anytype) Status {
    const self = runtime.?;
    const out: *FetchRequest = args[0];
    if (!self.running()) return FX_E_FAILED;
    const io = io_mod.getIo();
    const bridge = &self.fetch;
    const got = hostLock(self, &bridge.mutex);
    if (got != .locked) return acquireStatus(got);
    defer bridge.mutex.unlock(io);
    const decision = fetch_state.decide(bridge.phase, .take_request);
    // Every FX_OK or FX_EMPTY ends the previous take's borrows; nothing else
    // does (Sol's 2a item 5).
    if (decision.action != .applied) {
        _ = self.taken.reset(.retain_capacity);
        return FX_EMPTY;
    }
    // The turn the fetch serves, published with it under this lock.
    const turn = self.fetch_owner;
    _ = self.taking.reset(.retain_capacity);
    const arena = self.taking.allocator();
    const Metadata = struct {
        handle: fetch_state.Handle,
        method: []const u8,
        url: []const u8,
        headers: []const u8,
    };
    // Copied, never aliased: the bridge's buffers are cleared below.
    const metadata = std.json.parseFromSliceLeaky(Metadata, arena, bridge.request.items, .{ .allocate = .alloc_always }) catch return FX_E_INVALID;
    const Pair = struct { name: []const u8, value: []const u8 };
    const pairs = std.json.parseFromSliceLeaky([]const Pair, arena, metadata.headers, .{ .allocate = .alloc_always }) catch return FX_E_INVALID;
    var headers: std.ArrayList(Header) = .empty;
    headers.ensureTotalCapacity(arena, pairs.len) catch return FX_E_OOM;
    for (pairs) |pair| {
        if (credentialHeader(pair.name)) continue;
        headers.appendAssumeCapacity(.{ .name = Str.of(pair.name), .value = Str.of(pair.value) });
    }
    const body = arena.dupe(u8, bridge.request_body.items) catch return FX_E_OOM;
    const provider = arena.dupe(u8, self.provider_name) catch return FX_E_OOM;
    if (self.faultOom(.take_oom)) return FX_E_OOM;
    out.purpose = if (std.ascii.eqlIgnoreCase(metadata.method, "GET")) FX_FETCH_CATALOG else FX_FETCH_MODEL;
    out.fetch = @intCast(metadata.handle);
    out.turn = turn;
    out.provider = Str.of(provider);
    out.method = Str.of(metadata.method);
    out.url = Str.of(metadata.url);
    out.headers = if (headers.items.len == 0) null else headers.items.ptr;
    out.header_count = headers.items.len;
    out.body = Bytes.of(body);
    bridge.clearPendingRequest();
    bridge.phase = decision.phase;
    // The take succeeded: the previous one's borrows end now.
    std.mem.swap(std.heap.ArenaAllocator, &self.taken, &self.taking);
    _ = self.taking.reset(.retain_capacity);
    return FX_OK;
}

const FetchOp = union(enum) {
    status: struct { code: u16, retry_after: i32 },
    push: []const u8,
    finish: bool,
};

fn fetchOp(self: *Runtime, fetch: u64, op: FetchOp) Status {
    if (!self.running()) return FX_E_FAILED;
    const handle = fetchHandle(fetch) orelse return FX_STALE;
    const io = io_mod.getIo();
    const bridge = &self.fetch;
    const got = hostLock(self, &bridge.mutex);
    if (got != .locked) return acquireStatus(got);
    defer bridge.mutex.unlock(io);
    switch (op) {
        .status => |value| {
            const decision = fetch_state.decide(bridge.phase, .{ .start = handle });
            if (decision.action == .stale) {
                return switch (bridge.phase) {
                    .streaming => |active| if (active == handle) FX_E_STATE else FX_STALE,
                    .terminal => |terminal| if (terminal.handle == handle and terminal.kind == .success) FX_E_STATE else FX_STALE,
                    else => FX_STALE,
                };
            }
            bridge.status = value.code;
            bridge.retry_after_seconds = value.retry_after;
            bridge.phase = decision.phase;
        },
        .push => |bytes| {
            const decision = fetch_state.decide(bridge.phase, .{ .push = handle });
            if (decision.action == .stale) {
                return switch (bridge.phase) {
                    .awaiting_response => |active| if (active == handle) FX_E_STATE else FX_STALE,
                    else => FX_STALE,
                };
            }
            const queued = bridge.response.items.len - bridge.response_offset;
            if (queued > core.max_fetch_response_bytes - bytes.len) {
                self.push_blocked.store(true, .release);
                return FX_BACKPRESSURE;
            }
            if (bridge.response_offset > 0) {
                @memmove(bridge.response.items[0..queued], bridge.response.items[bridge.response_offset..]);
                bridge.response.items.len = queued;
                bridge.response_offset = 0;
            }
            bridge.response.appendSlice(alloc_c, bytes) catch return FX_E_OOM;
        },
        .finish => |ok| {
            const decision = fetch_state.decide(bridge.phase, if (ok) .{ .finish = handle } else .{ .fail = handle });
            if (decision.action == .stale) return FX_STALE;
            bridge.phase = decision.phase;
        },
    }
    bridge.wake.broadcast(io);
    return FX_OK;
}

export fn fx_fetch_status(handle: u64, fetch: u64, status: u16, retry_after_s: i32) callconv(.c) Status {
    if (retry_after_s < -1) return FX_E_INVALID;
    return hostCall(handle, .{ fetch, status, retry_after_s }, struct {
        fn body(runtime: ?*Runtime, a: anytype) Status {
            return fetchOp(runtime.?, a[0], .{ .status = .{ .code = a[1], .retry_after = a[2] } });
        }
    }.body);
}

export fn fx_fetch_push(handle: u64, fetch: u64, data: ?[*]const u8, len: usize) callconv(.c) Status {
    if (len > 0 and data == null) return FX_E_INVALID;
    const bytes: []const u8 = if (len == 0) "" else data.?[0..len];
    return hostCall(handle, .{ fetch, bytes }, struct {
        fn body(runtime: ?*Runtime, a: anytype) Status {
            if (!runtime.?.running()) return FX_E_FAILED;
            if (a[1].len > core.max_fetch_response_bytes) return FX_E_TOO_LARGE;
            return fetchOp(runtime.?, a[0], .{ .push = a[1] });
        }
    }.body);
}

export fn fx_fetch_finish(handle: u64, fetch: u64, ok: bool) callconv(.c) Status {
    return hostCall(handle, .{ fetch, ok }, struct {
        fn body(runtime: ?*Runtime, a: anytype) Status {
            return fetchOp(runtime.?, a[0], .{ .finish = a[1] });
        }
    }.body);
}

export fn fx_fetch_disposition_get(handle: u64, fetch: u64) callconv(.c) i32 {
    const status = hostCall(handle, .{fetch}, struct {
        fn body(runtime: ?*Runtime, a: anytype) Status {
            const self = runtime.?;
            if (!self.running()) return 0;
            const id = fetchHandle(a[0]) orelse return 0;
            switch (hostLock(self, &self.fetch.mutex)) {
                .locked => {},
                .failed => return 0,
                // Never RETIRED for contention: the host keeps the request
                // and asks again (Sol's 2a item 3).
                .busy => return FX_FETCH_BUSY,
            }
            defer self.fetch.mutex.unlock(io_mod.getIo());
            return @intFromEnum(fetch_state.disposition(self.fetch.phase, self.fetch.last_consumed_handle, id));
        }
    }.body);
    return if (status < 0) 0 else status;
}

// ---------------------------------------------------------------------------
// Exports: journal acknowledgements and hook answers
// ---------------------------------------------------------------------------

export fn fx_journal_ack(handle: u64, append: u64) callconv(.c) Status {
    return hostCall(handle, .{append}, struct {
        fn body(runtime: ?*Runtime, a: anytype) Status {
            const self = runtime.?;
            const append_id: u64 = a[0];
            if (self.faultHostTransport()) return FX_E_FAILED;
            const status = lockLive(self);
            if (status != FX_OK) return status;
            defer unlock(self);
            if (append_id == 0 or append_id > self.journal_sent) return FX_E_INVALID;
            if (append_id <= self.journal_acked or self.journal_failed != null) return FX_STALE;
            // The flushes it releases are answered in one write that either
            // lands whole or not at all; only then is the ack applied (Sol's
            // 2a item 5).
            var replies: std.Io.Writer.Allocating = .init(alloc_c);
            defer replies.deinit();
            for (self.flushes.items) |flush| if (flush.upto <= append_id) {
                writeResult(&replies.writer, flush.rpc_id, "{}") catch return FX_E_OOM;
            };
            if (replies.written().len > 0) {
                if (self.faultOom(.reply_oom)) return FX_E_OOM;
                const sent = self.hostInput(replies.written(), .control);
                if (sent != FX_OK) return sent;
            }
            self.journal_acked = append_id;
            var index: usize = 0;
            while (index < self.flushes.items.len) {
                if (self.flushes.items[index].upto <= append_id) {
                    _ = self.flushes.orderedRemove(index);
                } else index += 1;
            }
            return FX_OK;
        }
    }.body);
}

export fn fx_journal_fail(handle: u64, append: u64, reason: Str, fenced: bool) callconv(.c) Status {
    return hostCall(handle, .{ append, reason, fenced }, struct {
        fn body(runtime: ?*Runtime, a: anytype) Status {
            const self = runtime.?;
            const append_id: u64 = a[0];
            const text = a[1].slice() orelse return FX_E_INVALID;
            if (!isUtf8(text)) return FX_E_INVALID;
            const status = lockLive(self);
            if (status != FX_OK) return status;
            defer unlock(self);
            if (append_id == 0 or append_id > self.journal_sent) return FX_E_INVALID;
            if (append_id <= self.journal_acked or self.journal_failed != null) return FX_STALE;
            const message = if (a[2]) "another writer holds this session's journal" else text;
            const failure = alloc_c.dupe(u8, if (message.len == 0) "the journal store refused an append" else message) catch return FX_E_OOM;
            // As fx_journal_ack: the waiting flushes' answers land whole
            // before the failure is recorded.
            var replies: std.Io.Writer.Allocating = .init(alloc_c);
            defer replies.deinit();
            for (self.flushes.items) |flush| writeError(&replies.writer, flush.rpc_id, failure) catch {
                alloc_c.free(failure);
                return FX_E_OOM;
            };
            if (replies.written().len > 0) {
                const sent = if (self.faultOom(.reply_oom)) FX_E_OOM else self.hostInput(replies.written(), .control);
                if (sent != FX_OK) {
                    alloc_c.free(failure);
                    return sent;
                }
            }
            self.journal_failed = failure;
            self.flushes.clearRetainingCapacity();
            return FX_OK;
        }
    }.body);
}

fn writeResult(w: *std.Io.Writer, rpc_id: i64, result_json: []const u8) !void {
    try w.print("{{\"jsonrpc\":\"2.0\",\"id\":{d},\"result\":", .{rpc_id});
    try w.writeAll(result_json);
    try w.writeAll("}\n");
}

fn writeError(w: *std.Io.Writer, rpc_id: i64, message: []const u8) !void {
    try w.print("{{\"jsonrpc\":\"2.0\",\"id\":{d},\"error\":{{\"code\":-32000,\"message\":", .{rpc_id});
    try jsonrpc.writeJsonStr(message, w);
    try w.writeAll("}}\n");
}

export fn fx_hook_answer(handle: u64, question: u64, kind: i32, text: Str) callconv(.c) Status {
    return hostCall(handle, .{ question, kind, text }, hookAnswerBody);
}

fn hookAnswerBody(runtime: ?*Runtime, a: anytype) Status {
    const self = runtime.?;
    const question: u64 = a[0];
    const kind: i32 = a[1];
    const text = a[2].slice() orelse return FX_E_INVALID;
    const status = lockLive(self);
    if (status != FX_OK) return status;
    defer unlock(self);
    const record = self.questions.get(question) orelse
        return if (question == 0 or question >= self.next_question) FX_E_INVALID else FX_STALE;
    if (record.answered) return FX_STALE;
    // Each kind of question takes its own answers.
    const max_text: usize = switch (record.kind) {
        FX_HOOK_PRE_TOOL => switch (kind) {
            FX_ANSWER_ALLOW => 0,
            FX_ANSWER_BLOCK => max_hook_reason,
            FX_ANSWER_REWRITE => max_hook_arguments,
            else => return FX_E_INVALID,
        },
        FX_HOOK_STOP => switch (kind) {
            FX_ANSWER_ALLOW => 0,
            FX_ANSWER_CONTINUE => max_hook_context,
            else => return FX_E_INVALID,
        },
        else => return FX_E_INVALID,
    };
    if (kind != FX_ANSWER_ALLOW) {
        if (text.len == 0 or text.len > max_text or !isUtf8(text)) return FX_E_INVALID;
        if (kind == FX_ANSWER_REWRITE) {
            // The core runs rewritten input only if it is a JSON object.
            const parsed = std.json.parseFromSlice(std.json.Value, alloc_c, text, .{}) catch |err| return switch (err) {
                error.OutOfMemory => FX_E_OOM,
                else => FX_E_INVALID,
            };
            defer parsed.deinit();
            if (parsed.value != .object) return FX_E_INVALID;
        }
    }
    const copy: ?[]u8 = if (kind == FX_ANSWER_ALLOW) null else alloc_c.dupe(u8, text) catch return FX_E_OOM;
    record.text = copy;
    record.answer = kind;
    record.answered = true;
    self.calls_changed.broadcast(io_mod.getIo());
    return FX_OK;
}

// ---------------------------------------------------------------------------
// Exports: testing only
// ---------------------------------------------------------------------------

export fn fx_test_arm_fault(handle: u64, point: u32, skip: u32) callconv(.c) Status {
    if (point == 0 or point >= fault_count) return FX_E_INVALID;
    // FX_FAULT_CREATE is process-wide (handle 0); every other point is a
    // runtime's.
    if (point == @intFromEnum(Fault.create)) {
        if (handle != 0) return FX_E_INVALID;
        create_fault.store(@intCast(skip), .release);
        return FX_OK;
    }
    return hostCall(handle, .{ point, skip }, struct {
        fn body(runtime: ?*Runtime, a: anytype) Status {
            runtime.?.faults[a[0]].store(@intCast(a[1]), .release);
            return FX_OK;
        }
    }.body);
}

export fn fx_test_set_slot_generation(slot: u32, generation: u64) callconv(.c) Status {
    if (slot >= slots.len or generation == 0 or generation > max_generation) return FX_E_INVALID;
    const bit = @as(u64, 1) << @intCast(slot);
    if (slot_claim.fetchOr(bit, .acq_rel) & bit != 0) return FX_E_INVALID;
    slots[slot].state.store(generation << gen_shift, .release);
    _ = slot_claim.fetchAnd(~bit, .acq_rel);
    return FX_OK;
}
