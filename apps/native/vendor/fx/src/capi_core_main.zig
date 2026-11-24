//! Powermove notice: new file, not in upstream fx (patch P-CAPI). The C ABI
//! entry of the libfx core, beside upstream's N-API and WebAssembly entries.
//! It implements include/fx_core.h. The runtime, input queue, attachment
//! table, fetch bridge and runtime slots are the N-API entry's, extracted
//! verbatim into libfx_core_runtime.zig; only the Node-API surface is
//! replaced. The host is Rust (crate pm-aero-core), which takes the place of
//! sdk/engine.js.
//!
//! Milestone 1: the core still speaks ACP JSON-RPC on its transport. The C
//! entry is that transport: the host's calls become ACP frames on the input
//! queue, and the frames the core writes are turned into typed events and
//! host callbacks on the core's own thread, inside its writer callback. Patch
//! P-EVENTS replaces that frame parse with a typed sink in prompt.zig.
const std = @import("std");
const core = @import("libfx_core_runtime.zig");
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

pub const abi_version: u32 = 1;

// ---------------------------------------------------------------------------
// C types (must match include/fx_core.h)
// ---------------------------------------------------------------------------

pub const Status = i32;
const FX_OK: Status = 0;
const FX_EMPTY: Status = 1;
const FX_STALE: Status = 2;
const FX_BACKPRESSURE: Status = 3;
const FX_E_INVALID: Status = -1;
const FX_E_CLOSED: Status = -2;
const FX_E_FAILED: Status = -3;
const FX_E_LIMIT: Status = -4;
const FX_E_OOM: Status = -5;
const FX_E_THREAD: Status = -6;
const FX_E_IO: Status = -7;
const FX_E_STATE: Status = -8;
const FX_E_UNSUPPORTED: Status = -10;

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

pub const Host = extern struct {
    struct_size: u32 = 0,
    reserved: u32 = 0,
    tool_call: ?*const fn (?*anyopaque, u64, Str, Str, Str, u32) callconv(.c) void = null,
    tool_cancel: ?*const fn (?*anyopaque, u64) callconv(.c) void = null,
    journal_append: ?*const fn (?*anyopaque, u64, Str, bool) callconv(.c) void = null,
    pre_tool: ?*const fn (?*anyopaque, u64, u64, Str, Str) callconv(.c) i32 = null,
    stop: ?*const fn (?*anyopaque, u64, Str, u32) callconv(.c) i32 = null,
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
    reserved: u32,
};

pub const RuntimeInfo = extern struct {
    state: i32,
    exit_code: i32,
    message: Str,
};

pub const NamedBytes = extern struct {
    name: Str,
    bytes: Bytes,
};

pub const SessionOpenArgs = extern struct {
    struct_size: u32,
    reserved: u32,
    session_id: Str,
    journal_json: Str,
    checkpoint: Bytes,
    images: ?[*]const NamedBytes,
    image_count: usize,
    inputs_durable: bool,
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

pub const FetchRequest = extern struct {
    fetch: u64,
    method: Str,
    url: Str,
    headers: ?[*]const Header,
    header_count: usize,
    body: Bytes,
};

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
};

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

// ---------------------------------------------------------------------------
// Limits
// ---------------------------------------------------------------------------

const max_instructions_bytes = 1024 * 1024;
const max_tools = 256;
const max_tool_text_bytes = 64 * 1024;
const max_schema_bytes = 256 * 1024;
const max_message_id_bytes = 128;
const max_prompt_text_bytes = 4 * 1024 * 1024;
/// Queued event text the core may hold before its writer waits for a drain.
const max_event_bytes = 16 * 1024 * 1024;
const max_configured_providers_bytes = 1024 * 1024;

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
    closed: bool = false,

    fn deinit(self: *EventQueue) void {
        self.events.deinit(alloc_c);
        self.pending_bytes.deinit(alloc_c);
        self.pending_names.deinit(alloc_c);
        self.drained_bytes.deinit(alloc_c);
        self.drained_names.deinit(alloc_c);
        self.drained_name_strs.deinit(alloc_c);
    }

    const Text = struct {
        text: []const u8 = "",
        text2: []const u8 = "",
        bytes: []const u8 = "",
        names: []const []const u8 = &.{},
    };

    /// Queues `event` with its text copied once. Returns whether the queue was
    /// empty before, so the caller wakes the host.
    /// `wait`: the core's writer waits for space; a host call never does.
    fn push(self: *EventQueue, event: QueuedEvent, data: Text, wait: bool) !bool {
        const io = io_mod.getIo();
        self.mutex.lockUncancelable(io);
        defer self.mutex.unlock(io);
        var total = data.text.len + data.text2.len + data.bytes.len;
        for (data.names) |name| total += name.len;
        if (total > max_event_bytes) return error.EventTooLarge;
        while (wait and !self.closed and self.events.items.len > 0 and self.pending_bytes.items.len + total > max_event_bytes) {
            self.space.waitUncancelable(io, &self.mutex);
        }
        if (self.closed) return error.EventQueueClosed;
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

    fn append(self: *EventQueue, bytes: []const u8) !Span {
        if (bytes.len == 0) return .{};
        const off = self.pending_bytes.items.len;
        try self.pending_bytes.appendSlice(alloc_c, bytes);
        return .{ .off = @intCast(off), .len = @intCast(bytes.len) };
    }

    fn drain(self: *EventQueue, out: []Event) !usize {
        const io = io_mod.getIo();
        self.mutex.lockUncancelable(io);
        defer self.mutex.unlock(io);
        const count = @min(out.len, self.events.items.len);
        if (count == 0) {
            // Nothing new: what the last drain handed out is released.
            self.drained_bytes.clearRetainingCapacity();
            self.drained_names.clearRetainingCapacity();
            self.drained_name_strs.clearRetainingCapacity();
            return 0;
        }
        const everything = count == self.events.items.len;
        self.drained_bytes.clearRetainingCapacity();
        self.drained_names.clearRetainingCapacity();
        self.drained_name_strs.clearRetainingCapacity();
        if (everything) {
            std.mem.swap(std.ArrayList(u8), &self.pending_bytes, &self.drained_bytes);
            std.mem.swap(std.ArrayList(Span), &self.pending_names, &self.drained_names);
        } else {
            // A partial drain copies what it hands out; the rest stays.
            try self.drained_bytes.appendSlice(alloc_c, self.pending_bytes.items);
            try self.drained_names.appendSlice(alloc_c, self.pending_names.items);
        }
        var name_total: usize = 0;
        for (self.events.items[0..count]) |event| name_total += event.name_count;
        try self.drained_name_strs.ensureTotalCapacity(alloc_c, name_total);
        const arena = self.drained_bytes.items;
        for (self.events.items[0..count], out[0..count]) |event, *slot| {
            var names: ?[*]const Str = null;
            if (event.name_count > 0) {
                const first = self.drained_name_strs.items.len;
                for (self.drained_names.items[event.names_start..][0..event.name_count]) |span| {
                    self.drained_name_strs.appendAssumeCapacity(spanStr(arena, span));
                }
                names = self.drained_name_strs.items[first..].ptr;
            }
            slot.* = .{
                .kind = @intFromEnum(event.kind),
                .flags = event.flags,
                .turn = event.turn,
                .request = event.request,
                .id = event.id,
                .status = event.status,
                .code = event.code,
                .text = spanStr(arena, event.text),
                .text2 = spanStr(arena, event.text2),
                .bytes = .{ .ptr = if (event.bytes.len == 0) null else arena[event.bytes.off..].ptr, .len = event.bytes.len },
                .names = names,
                .name_count = event.name_count,
                .usage = event.usage,
            };
        }
        if (everything) {
            self.events.clearRetainingCapacity();
        } else {
            const rest = self.events.items.len - count;
            std.mem.copyForwards(QueuedEvent, self.events.items[0..rest], self.events.items[count..]);
            self.events.items.len = rest;
        }
        self.space.broadcast(io);
        return count;
    }

    fn close(self: *EventQueue) void {
        const io = io_mod.getIo();
        self.mutex.lockUncancelable(io);
        defer self.mutex.unlock(io);
        self.closed = true;
        self.space.broadcast(io);
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

    const Kind = enum { initialize, libfx_new, journal_open, prompt, steer, withdraw, checkpoint };
};

/// A host tool call the core is waiting on. `frame` owns the bytes the
/// borrowed name, input and model call id point into.
const Call = struct {
    rpc_id: i64,
    frame: []u8,
    name: []const u8,
    input: []const u8,
    model_call_id: []const u8,
    turn: u64,
    cancelled: bool = false,
};

const Flush = struct {
    rpc_id: i64,
    upto: u64,
};

const Tool = struct {
    name: []u8,
    flags: u32,
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
    configured_providers: ?[]u8,
    model: ?[]u8,
    effort: ?[]u8,
    home: []u8,
    workspace_root: []u8,
    gateway_chat_url: []u8,
    max_agent_steps: usize,
    max_tool_result_bytes: usize,
    max_history_turns: usize,
    ready: core.ReadyNotifier,
    thread: std.Thread = undefined,
    exited: std.atomic.Value(bool) = .init(false),
    exit_code: std.atomic.Value(u8) = .init(0),

    // The C entry's own.
    host: Host,
    host_ctx: ?*anyopaque,
    tools: []Tool,
    initialize_frame: []u8,
    events: EventQueue = .{},
    /// Guards everything below. Never held while a host callback runs.
    lock: std.Io.Mutex = .init,
    next_rpc_id: i64 = 1,
    next_request: u64 = 1,
    next_call: u64 = 1,
    pending: std.AutoHashMapUnmanaged(i64, Pending) = .empty,
    session_id: ?[]u8 = null,
    session: enum { none, opening, open } = .none,
    journaled: bool = false,
    /// The running turn, or 0.
    turn: u64 = 0,
    calls: std.AutoHashMapUnmanaged(u64, Call) = .empty,
    journal_sent: u64 = 0,
    journal_acked: u64 = 0,
    journal_failed: ?[]u8 = null,
    flushes: std.ArrayList(Flush) = .empty,
    /// Partial frame bytes, if the core's writer ever splits a frame.
    line: std.ArrayList(u8) = .empty,
    /// What the last fx_fetch_take handed out.
    taken: std.heap.ArenaAllocator = .init(alloc_c),
    exited_event_sent: bool = false,

    fn readInput(context: ?*anyopaque, destination: []u8) usize {
        const self: *Runtime = @ptrCast(@alignCast(context.?));
        return self.input.read(self.alloc, destination);
    }

    /// The core's JSON-RPC writer: called on a core-owned thread with whole
    /// frames, serialised by the writer's own mutex.
    fn writeOutput(context: ?*anyopaque, bytes: []const u8) !void {
        const self: *Runtime = @ptrCast(@alignCast(context.?));
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
                // placeholder the host replaces (P-CAPI).
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
            },
            jsonrpc.Reader.initCallback(self, Runtime.readInput),
            jsonrpc.Writer.initCallback(self, Runtime.writeOutput),
        ) catch {
            self.exit_code.store(1, .seq_cst);
        };
        self.fetch.shutdown();
        self.exited.store(true, .seq_cst);
        self.queueEvent(.{ .kind = .exited, .code = self.exit_code.load(.seq_cst) }, .{});
        self.ready.notify();
    }

    fn deinit(self: *Runtime) void {
        self.events.close();
        self.input.close();
        self.fetch.shutdown();
        self.thread.join();
        self.ready.deinit();
        self.fetch.deinit();
        self.attachments.deinit();
        self.input.deinit(self.alloc);
        self.events.deinit();
        var calls = self.calls.valueIterator();
        while (calls.next()) |call| alloc_c.free(call.frame);
        self.calls.deinit(alloc_c);
        self.pending.deinit(alloc_c);
        self.flushes.deinit(alloc_c);
        self.line.deinit(alloc_c);
        self.taken.deinit();
        if (self.journal_failed) |reason| alloc_c.free(reason);
        if (self.session_id) |id| alloc_c.free(id);
        for (self.tools) |tool| alloc_c.free(tool.name);
        alloc_c.free(self.tools);
        alloc_c.free(self.initialize_frame);
        if (self.custom_context) |*context| context.deinit(self.alloc);
        if (self.configured_providers) |json| self.alloc.free(json);
        if (self.model) |model| self.alloc.free(model);
        if (self.effort) |effort| self.alloc.free(effort);
        self.alloc.free(self.home);
        self.alloc.free(self.workspace_root);
        self.alloc.free(self.gateway_chat_url);
        self.alloc.destroy(self);
        core.releaseRuntimeSlot();
    }

    // --- ids and frames -----------------------------------------------------

    fn nextRequest(self: *Runtime) u64 {
        const id = self.next_request;
        self.next_request += 1;
        return id;
    }

    /// Writes one request frame to the core and records what it answers.
    /// The caller holds `lock`.
    fn sendRequest(self: *Runtime, kind: Pending.Kind, request: u64, method: []const u8, params: []const u8) Status {
        const rpc_id = self.next_rpc_id;
        var frame: std.Io.Writer.Allocating = .init(alloc_c);
        defer frame.deinit();
        frame.writer.print("{{\"jsonrpc\":\"2.0\",\"id\":{d},\"method\":", .{rpc_id}) catch return FX_E_OOM;
        jsonrpc.writeJsonStr(method, &frame.writer) catch return FX_E_OOM;
        frame.writer.writeAll(",\"params\":") catch return FX_E_OOM;
        frame.writer.writeAll(params) catch return FX_E_OOM;
        frame.writer.writeAll("}\n") catch return FX_E_OOM;
        self.pending.put(alloc_c, rpc_id, .{ .kind = kind, .request = request }) catch return FX_E_OOM;
        self.input.write(self.alloc, frame.written()) catch |err| {
            _ = self.pending.remove(rpc_id);
            return switch (err) {
                error.InputClosed => FX_E_FAILED,
                error.InputQueueFull => FX_BACKPRESSURE,
                error.OutOfMemory => FX_E_OOM,
            };
        };
        self.next_rpc_id += 1;
        return FX_OK;
    }

    /// Answers a request the core made (a tool call or a journal flush).
    fn respond(self: *Runtime, rpc_id: i64, result_json: []const u8) void {
        var frame: std.Io.Writer.Allocating = .init(alloc_c);
        defer frame.deinit();
        frame.writer.print("{{\"jsonrpc\":\"2.0\",\"id\":{d},\"result\":", .{rpc_id}) catch return;
        frame.writer.writeAll(result_json) catch return;
        frame.writer.writeAll("}\n") catch return;
        self.input.write(self.alloc, frame.written()) catch {};
    }

    fn respondError(self: *Runtime, rpc_id: i64, message: []const u8) void {
        var frame: std.Io.Writer.Allocating = .init(alloc_c);
        defer frame.deinit();
        frame.writer.print("{{\"jsonrpc\":\"2.0\",\"id\":{d},\"error\":{{\"code\":-32000,\"message\":", .{rpc_id}) catch return;
        jsonrpc.writeJsonStr(message, &frame.writer) catch return;
        frame.writer.writeAll("}}\n") catch return;
        self.input.write(self.alloc, frame.written()) catch {};
    }

    /// Queues an event from a core-owned thread, waiting for space.
    fn queueEvent(self: *Runtime, event: QueuedEvent, data: EventQueue.Text) void {
        const was_empty = self.events.push(event, data, true) catch return;
        if (was_empty) self.ready.notify();
    }

    /// Queues an event from a host call, which never waits.
    fn queueEventNow(self: *Runtime, event: QueuedEvent, data: EventQueue.Text) void {
        const was_empty = self.events.push(event, data, false) catch return;
        if (was_empty) self.ready.notify();
    }

    // --- frames from the core (core-owned threads) ---------------------------

    fn onFrame(self: *Runtime, frame: []const u8) !void {
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
        if (std.mem.eql(u8, method, "session/update")) {
            if (params) |value| self.onUpdate(value);
        } else if (std.mem.eql(u8, method, "libfx/tool_call")) {
            if (id) |rpc_id| self.onToolCall(rpc_id, frame, params);
        } else if (std.mem.eql(u8, method, "libfx/journal_append")) {
            if (params) |value| self.onJournalAppend(arena.allocator(), value);
        } else if (std.mem.eql(u8, method, "libfx/journal_flush")) {
            if (id) |rpc_id| self.onJournalFlush(rpc_id);
        } else if (id) |rpc_id| {
            // The minimal kernel asks nothing else of its host; refuse it.
            if (std.mem.eql(u8, method, "session/request_permission")) {
                self.respond(rpc_id, "{\"outcome\":{\"outcome\":\"cancelled\"}}");
            } else {
                self.respondError(rpc_id, "Method not supported by the host");
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
        const message = errorMessage(err);
        switch (pending.kind) {
            .initialize => {
                self.lock.unlock(io);
                if (err != null) self.queueEvent(.{ .kind = .@"error", .status = FX_E_FAILED }, .{ .text = message });
            },
            .libfx_new => {
                if (err != null) {
                    self.session = .none;
                    self.lock.unlock(io);
                    self.queueEvent(.{ .kind = .request_done, .request = pending.request, .status = FX_E_FAILED }, .{ .text = message });
                    return;
                }
                const session_id = stringField(result, "sessionId") orelse "";
                if (self.session_id) |old| alloc_c.free(old);
                self.session_id = alloc_c.dupe(u8, session_id) catch null;
                if (self.journaled) {
                    var params: std.Io.Writer.Allocating = .init(arena);
                    params.writer.writeAll("{\"sessionId\":") catch {};
                    jsonrpc.writeJsonStr(session_id, &params.writer) catch {};
                    params.writer.writeAll("}") catch {};
                    const status = self.sendRequest(.journal_open, pending.request, "libfx/journal_open", params.written());
                    if (status != FX_OK) self.session = .none;
                    self.lock.unlock(io);
                    if (status != FX_OK) self.queueEvent(.{ .kind = .request_done, .request = pending.request, .status = status }, .{ .text = "could not open the journal" });
                    return;
                }
                self.session = .open;
                self.lock.unlock(io);
                self.queueEvent(.{ .kind = .session_opened, .request = pending.request }, .{ .text = session_id });
            },
            .journal_open => {
                if (err != null) {
                    self.session = .none;
                    self.lock.unlock(io);
                    self.queueEvent(.{ .kind = .request_done, .request = pending.request, .status = FX_E_FAILED }, .{ .text = message });
                    return;
                }
                self.session = .open;
                const session_id = arena.dupe(u8, self.session_id orelse "") catch "";
                self.lock.unlock(io);
                const resumable = boolField(result, "resumable");
                self.queueEvent(.{
                    .kind = .session_opened,
                    .request = pending.request,
                    .flags = if (resumable) 1 else 0,
                }, .{ .text = session_id });
            },
            .prompt => {
                const turn = pending.request;
                if (self.turn == turn) self.turn = 0;
                const cancels = self.endTurnCalls(arena, turn);
                self.lock.unlock(io);
                for (cancels) |call| if (self.host.tool_cancel) |cancel| cancel(self.host_ctx, call);
                if (err != null) {
                    self.queueEvent(.{ .kind = .@"error", .turn = turn, .status = FX_E_FAILED }, .{ .text = message });
                    self.queueEvent(.{ .kind = .turn_end, .turn = turn, .code = @intFromEnum(StopReason.@"error") }, .{});
                    return;
                }
                const reason = StopReason.fromAcp(stringField(result, "stopReason") orelse "");
                self.queueEvent(.{
                    .kind = .turn_end,
                    .turn = turn,
                    .code = @intFromEnum(reason),
                    .usage = promptUsage(result),
                }, .{});
            },
            .steer, .checkpoint => {
                self.lock.unlock(io);
                if (err != null) {
                    self.queueEvent(.{ .kind = .request_done, .request = pending.request, .status = FX_E_FAILED }, .{ .text = message });
                } else if (pending.kind == .steer) {
                    self.queueEvent(.{ .kind = .request_done, .request = pending.request, .status = FX_OK }, .{});
                } else {
                    const attachment = intField(result, "checkpointAttachment") orelse 0;
                    const bytes = if (attachment > 0 and attachment <= std.math.maxInt(u32))
                        self.attachments.takeOutbound(@intCast(attachment))
                    else
                        null;
                    if (bytes) |value| {
                        defer alloc_c.free(value);
                        self.queueEvent(.{ .kind = .checkpoint, .request = pending.request }, .{ .bytes = value });
                    } else {
                        self.queueEvent(.{ .kind = .request_done, .request = pending.request, .status = FX_E_FAILED }, .{ .text = "the core published no checkpoint" });
                    }
                }
            },
            .withdraw => {
                self.lock.unlock(io);
                if (err != null) {
                    self.queueEvent(.{ .kind = .request_done, .request = pending.request, .status = FX_E_FAILED }, .{ .text = message });
                } else {
                    const outcome = stringField(result, "result") orelse "";
                    const status = if (std.mem.eql(u8, outcome, "withdrawn")) FX_OK else FX_STALE;
                    self.queueEvent(.{ .kind = .request_done, .request = pending.request, .status = status }, .{});
                }
            },
        }
    }

    /// Marks the turn's outstanding calls cancelled and returns their ids, for
    /// tool_cancel once `lock` is released. The caller holds `lock`.
    fn endTurnCalls(self: *Runtime, arena: Allocator, turn: u64) []u64 {
        var ids: std.ArrayList(u64) = .empty;
        var it = self.calls.iterator();
        while (it.next()) |entry| {
            if (entry.value_ptr.turn != turn or entry.value_ptr.cancelled) continue;
            entry.value_ptr.cancelled = true;
            ids.append(arena, entry.key_ptr.*) catch break;
        }
        return ids.items;
    }

    fn currentTurn(self: *Runtime) u64 {
        const io = io_mod.getIo();
        self.lock.lockUncancelable(io);
        defer self.lock.unlock(io);
        return self.turn;
    }

    fn onUpdate(self: *Runtime, params: std.json.Value) void {
        if (params != .object) return;
        const update = params.object.get("update") orelse return;
        if (update != .object) return;
        const kind = stringField(update, "sessionUpdate") orelse return;
        const turn = self.currentTurn();
        if (std.mem.eql(u8, kind, "agent_message_chunk") or std.mem.eql(u8, kind, "agent_thought_chunk")) {
            const content = update.object.get("content") orelse return;
            const text = stringField(content, "text") orelse return;
            if (text.len == 0) return;
            const thought = kind[6] == 't';
            // engine.js drops the core's context notices from the reply.
            if (!thought and std.mem.startsWith(u8, text, "[context]")) return;
            self.queueEvent(.{ .kind = if (thought) .thought_delta else .text_delta, .turn = turn }, .{ .text = text });
        } else if (std.mem.eql(u8, kind, "user_message_chunk")) {
            const content = update.object.get("content") orelse return;
            const text = stringField(content, "text") orelse return;
            const meta = update.object.get("_meta") orelse return;
            const fx = objectField(meta, "fx") orelse return;
            const steering = objectField(fx, "steering") orelse return;
            self.queueEvent(.{ .kind = .steer_joined, .turn = turn }, .{ .text = text, .text2 = stringField(steering, "inputId") orelse "" });
        } else if (std.mem.eql(u8, kind, "tool_call")) {
            const name = stringField(update, "name") orelse stringField(update, "toolName") orelse stringField(update, "title") orelse "tool";
            self.queueEvent(.{ .kind = .tool_start, .turn = turn }, .{ .text = name, .text2 = stringField(update, "toolCallId") orelse "" });
        } else if (std.mem.eql(u8, kind, "tool_call_update")) {
            const status = stringField(update, "status") orelse return;
            const failed = std.mem.eql(u8, status, "failed");
            if (!failed and !std.mem.eql(u8, status, "completed")) return;
            const name = stringField(update, "name") orelse stringField(update, "title") orelse "";
            self.queueEvent(.{
                .kind = .tool_end,
                .turn = turn,
                .status = if (failed) FX_E_FAILED else FX_OK,
            }, .{ .text = name, .text2 = stringField(update, "toolCallId") orelse "" });
        } else if (std.mem.eql(u8, kind, "usage_update")) {
            self.queueEvent(.{ .kind = .usage, .turn = turn, .usage = .{
                .context_used = @intCast(@max(0, intField(update, "used") orelse 0)),
                .context_size = @intCast(@max(0, intField(update, "size") orelse 0)),
            } }, .{});
        }
    }

    fn onToolCall(self: *Runtime, rpc_id: i64, frame: []const u8, params: ?std.json.Value) void {
        const value = params orelse return self.respondError(rpc_id, "Invalid host tool request");
        const name = stringField(value, "name") orelse return self.respondError(rpc_id, "Invalid host tool request");
        const model_call_id = stringField(value, "toolCallId") orelse "";
        const callback = self.host.tool_call orelse return self.respondError(rpc_id, "The host has no tools");
        // The model's own input JSON, borrowed from the frame as the core
        // wrote it: `input` is the last member of params.
        const marker = ",\"input\":";
        const owned = alloc_c.alloc(u8, frame.len + name.len + model_call_id.len) catch
            return self.respondError(rpc_id, "Host tool request failed");
        const input_start = std.mem.find(u8, frame, marker) orelse {
            alloc_c.free(owned);
            return self.respondError(rpc_id, "Invalid host tool request");
        };
        const raw_input = std.mem.trimEnd(u8, frame[input_start + marker.len ..], " \r\n");
        const input = if (std.mem.endsWith(u8, raw_input, "}}")) raw_input[0 .. raw_input.len - 2] else raw_input;
        @memcpy(owned[0..input.len], input);
        @memcpy(owned[input.len..][0..name.len], name);
        @memcpy(owned[input.len + name.len ..][0..model_call_id.len], model_call_id);
        const call_input = owned[0..input.len];
        const call_name = owned[input.len..][0..name.len];
        const call_model_id = owned[input.len + name.len ..][0..model_call_id.len];
        var flags: u32 = 0;
        for (self.tools) |tool| if (std.mem.eql(u8, tool.name, name)) {
            flags = tool.flags;
            break;
        };
        const io = io_mod.getIo();
        self.lock.lockUncancelable(io);
        const call = self.next_call;
        self.next_call += 1;
        self.calls.put(alloc_c, call, .{
            .rpc_id = rpc_id,
            .frame = owned,
            .name = call_name,
            .input = call_input,
            .model_call_id = call_model_id,
            .turn = self.turn,
        }) catch {
            self.lock.unlock(io);
            alloc_c.free(owned);
            return self.respondError(rpc_id, "Host tool request failed");
        };
        self.lock.unlock(io);
        callback(self.host_ctx, call, Str.of(call_name), Str.of(call_input), Str.of(call_model_id), flags);
    }

    fn onJournalAppend(self: *Runtime, arena: Allocator, params: std.json.Value) void {
        const callback = self.host.journal_append orelse return;
        const events = params.object.get("events") orelse return;
        const barrier = boolField(params, "barrier");
        var out: std.Io.Writer.Allocating = .init(arena);
        std.json.Stringify.value(events, .{}, &out.writer) catch return;
        const io = io_mod.getIo();
        self.lock.lockUncancelable(io);
        if (self.journal_failed != null) {
            self.lock.unlock(io);
            return;
        }
        self.journal_sent += 1;
        const append = self.journal_sent;
        self.lock.unlock(io);
        callback(self.host_ctx, append, Str.of(out.written()), barrier);
    }

    fn onJournalFlush(self: *Runtime, rpc_id: i64) void {
        const io = io_mod.getIo();
        self.lock.lockUncancelable(io);
        defer self.lock.unlock(io);
        if (self.journal_failed) |reason| return self.respondError(rpc_id, reason);
        if (self.journal_acked >= self.journal_sent) return self.respond(rpc_id, "{}");
        self.flushes.append(alloc_c, .{ .rpc_id = rpc_id, .upto = self.journal_sent }) catch
            self.respondError(rpc_id, "Journal flush failed");
    }
};

/// The Gateway credential the core is given, never a secret: the host sets
/// the real authorization header at its fetch edge.
const host_managed_credential = "host-managed";

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

/// One slot per possible runtime. `state` packs the slot's generation (high
/// 32 bits), a closing bit, and the number of API calls inside it, so a call
/// enters and leaves without a lock and destroy waits for the calls inside.
const Slot = struct {
    state: std.atomic.Value(u64) = .init(1 << 32),
    runtime: ?*Runtime = null,
};

const closing_bit: u64 = 1 << 31;
const users_mask: u64 = closing_bit - 1;

var slots: [core.max_active_runtimes]Slot = @splat(.{});
var slot_claim: std.atomic.Value(u64) = .init(0);

fn handleOf(index: usize, generation: u32) u64 {
    return (@as(u64, generation) << 8) | @as(u64, index + 1);
}

fn slotIndex(handle: u64) ?usize {
    const index = handle & 0xff;
    if (index == 0 or index > slots.len) return null;
    return @intCast(index - 1);
}

/// Enters `handle`: returns its runtime with the slot's user count raised, or
/// null if it is closed. Pair with `leave`.
fn enter(handle: u64) ?*Runtime {
    const index = slotIndex(handle) orelse return null;
    const generation: u64 = handle >> 8;
    if (generation == 0 or generation > std.math.maxInt(u32)) return null;
    const slot = &slots[index];
    var state = slot.state.load(.acquire);
    while (true) {
        if (state >> 32 != generation or state & closing_bit != 0) return null;
        if (state & users_mask == users_mask) return null;
        state = slot.state.cmpxchgWeak(state, state + 1, .acq_rel, .acquire) orelse break;
    }
    return slot.runtime;
}

fn leave(handle: u64) void {
    const index = slotIndex(handle).?;
    _ = slots[index].state.fetchSub(1, .acq_rel);
}

/// Claims a free slot for a new runtime and returns its handle.
fn claimSlot(runtime: *Runtime) ?u64 {
    for (&slots, 0..) |*slot, index| {
        const bit = @as(u64, 1) << @intCast(index);
        if (slot_claim.fetchOr(bit, .acq_rel) & bit != 0) continue;
        slot.runtime = runtime;
        const generation: u32 = @intCast(slot.state.load(.acquire) >> 32);
        return handleOf(index, generation);
    }
    return null;
}

/// Starts closing `handle`: no new call enters, and the caller waits for the
/// calls inside to leave. Returns the runtime, or null if it was not open.
fn beginClose(handle: u64) ?*Runtime {
    const index = slotIndex(handle) orelse return null;
    const generation: u64 = handle >> 8;
    const slot = &slots[index];
    var state = slot.state.load(.acquire);
    while (true) {
        if (state >> 32 != generation or state & closing_bit != 0) return null;
        state = slot.state.cmpxchgWeak(state, state | closing_bit, .acq_rel, .acquire) orelse break;
    }
    while (slot.state.load(.acquire) & users_mask != 0) std.Thread.yield() catch {};
    return slot.runtime;
}

/// Retires the slot: a new generation, and the slot free for reuse.
fn endClose(handle: u64) void {
    const index = slotIndex(handle).?;
    const slot = &slots[index];
    const generation = slot.state.load(.acquire) >> 32;
    const next = if (generation == std.math.maxInt(u32)) 1 else generation + 1;
    slot.runtime = null;
    slot.state.store(next << 32, .release);
    _ = slot_claim.fetchAnd(~(@as(u64, 1) << @intCast(index)), .acq_rel);
}

// ---------------------------------------------------------------------------
// Exports
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

export fn fx_core_abi_version() callconv(.c) u32 {
    return abi_version;
}

export fn fx_runtime_create(cfg_ptr: ?*const Config, host_ptr: ?*const Host, host_ctx: ?*anyopaque, out: ?*u64) callconv(.c) Status {
    const out_ptr = out orelse return FX_E_INVALID;
    out_ptr.* = 0;
    const cfg = cfg_ptr orelse return FX_E_INVALID;
    if (cfg.abi_version != abi_version or cfg.struct_size < @sizeOf(Config)) return FX_E_INVALID;
    var host: Host = .{};
    if (host_ptr) |given| {
        // Read only the fields the host compiled.
        const size = @min(given.struct_size, @sizeOf(Host));
        if (size < @offsetOf(Host, "tool_call")) return FX_E_INVALID;
        @memcpy(std.mem.asBytes(&host)[0..size], std.mem.asBytes(given)[0..size]);
    }
    core.ensureThreadedIo();
    if (!core.claimRuntimeSlot()) return FX_E_LIMIT;
    const runtime = createRuntime(cfg, host, host_ctx) catch |err| {
        core.releaseRuntimeSlot();
        return switch (err) {
            error.Invalid => FX_E_INVALID,
            error.OutOfMemory => FX_E_OOM,
            error.ThreadFailed => FX_E_THREAD,
            error.ReadyChannelFailed => FX_E_IO,
        };
    };
    const handle = claimSlot(runtime) orelse {
        // claimRuntimeSlot bounds runtimes by the slot count, so a free slot
        // exists; this is only reached if that invariant breaks.
        runtime.deinit();
        return FX_E_LIMIT;
    };
    out_ptr.* = handle;
    return FX_OK;
}

const CreateError = error{ Invalid, OutOfMemory, ThreadFailed, ReadyChannelFailed };

fn createRuntime(cfg: *const Config, host: Host, host_ctx: ?*anyopaque) CreateError!*Runtime {
    const alloc = alloc_c;
    // Provider.
    const provider_name = cfg.provider.slice() orelse return error.Invalid;
    const provider: ?model_provider.ProviderId = if (provider_name.len == 0 or std.mem.eql(u8, provider_name, "gateway"))
        .gateway
    else if (std.mem.eql(u8, provider_name, "codex"))
        .codex
    else if (std.mem.eql(u8, provider_name, "grok"))
        .grok
    else configured: {
        const parsed = model_provider.parse(provider_name) orelse return error.Invalid;
        if (parsed != .configured) return error.Invalid;
        break :configured parsed;
    };
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
    const gateway_chat_url = (try optionalCopy(cfg.endpoint_url, core.max_url_bytes)) orelse
        try alloc.dupe(u8, builtin_gateway.default_chat_url);
    errdefer alloc.free(gateway_chat_url);
    core.http_config.validateEndpoint(gateway_chat_url) catch return error.Invalid;
    if (!std.mem.eql(u8, gateway_chat_url, builtin_gateway.default_chat_url)) {
        const uri = std.Uri.parse(gateway_chat_url) catch return error.Invalid;
        if (!std.ascii.eqlIgnoreCase(uri.scheme, "http")) return error.Invalid;
    }

    // Tools and instructions, as initialize's clientCapabilities.libfx.
    const instructions = cfg.instructions.slice() orelse return error.Invalid;
    if (instructions.len > max_instructions_bytes or !isUtf8(instructions)) return error.Invalid;
    if (cfg.tool_count > max_tools or (cfg.tool_count > 0 and cfg.tools == null)) return error.Invalid;
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
        .configured_providers = configured_providers,
        .model = model,
        .effort = effort,
        .home = home,
        .workspace_root = workspace_root,
        .gateway_chat_url = gateway_chat_url,
        .max_agent_steps = if (cfg.max_agent_steps == 0) core.agent_steps.default_max_agent_steps else cfg.max_agent_steps,
        .max_tool_result_bytes = if (cfg.max_tool_result_bytes == 0) 64 * 1024 else cfg.max_tool_result_bytes,
        .max_history_turns = if (cfg.max_history_turns == 0) 100 else cfg.max_history_turns,
        .ready = ready,
        .host = host,
        .host_ctx = host_ctx,
        .tools = tools,
        .initialize_frame = initialize_frame,
        .journaled = host.journal_append != null,
    };
    runtime.fetch.ready = &runtime.ready;
    runtime.stream_context = host_stream_provider.initContext(builtin_gateway.buildAgentRequest, .{ .fixed = runtime.gateway_chat_url }, .{
        .context = &runtime.fetch,
        .open_fn = core.FetchBridge.open,
        .status_fn = core.FetchBridge.statusFn,
        .next_fn = core.FetchBridge.next,
        .close_fn = core.FetchBridge.close,
        .consumed_fn = core.FetchBridge.markConsumed,
        .retry_after_fn = core.FetchBridge.retryAfterFn,
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
    const runtime = beginClose(handle) orelse return FX_E_CLOSED;
    runtime.deinit();
    endClose(handle);
    return FX_OK;
}

export fn fx_runtime_ready_fd(handle: u64) callconv(.c) c_int {
    const runtime = enter(handle) orelse return -1;
    defer leave(handle);
    return runtime.ready.reader orelse -1;
}

export fn fx_runtime_info_get(handle: u64, out: ?*RuntimeInfo) callconv(.c) Status {
    const out_ptr = out orelse return FX_E_INVALID;
    const runtime = enter(handle) orelse return FX_E_CLOSED;
    defer leave(handle);
    const exited = runtime.exited.load(.seq_cst);
    out_ptr.* = .{
        .state = if (exited) 1 else 0,
        .exit_code = runtime.exit_code.load(.seq_cst),
        .message = .{},
    };
    return FX_OK;
}

/// Enters a runtime for a call that needs a live core.
fn enterLive(handle: u64) union(enum) { runtime: *Runtime, status: Status } {
    const runtime = enter(handle) orelse return .{ .status = FX_E_CLOSED };
    if (runtime.exited.load(.seq_cst)) {
        leave(handle);
        return .{ .status = FX_E_FAILED };
    }
    return .{ .runtime = runtime };
}

export fn fx_session_open(handle: u64, args_ptr: ?*const SessionOpenArgs, request_out: ?*u64) callconv(.c) Status {
    const args = args_ptr orelse return FX_E_INVALID;
    const request_ptr = request_out orelse return FX_E_INVALID;
    if (args.struct_size < @sizeOf(SessionOpenArgs)) return FX_E_INVALID;
    const session_id = args.session_id.slice() orelse return FX_E_INVALID;
    if (session_id.len > 255 or !isUtf8(session_id)) return FX_E_INVALID;
    // Resume from a journal, a checkpoint or staged images lands with the
    // durability work (milestone 2).
    if (args.journal_json.len > 0 or args.checkpoint.len > 0 or args.image_count > 0) return FX_E_UNSUPPORTED;
    const runtime = switch (enterLive(handle)) {
        .runtime => |value| value,
        .status => |status| return status,
    };
    defer leave(handle);
    const io = io_mod.getIo();
    runtime.lock.lockUncancelable(io);
    defer runtime.lock.unlock(io);
    if (runtime.session != .none) return FX_E_STATE;
    var params: std.Io.Writer.Allocating = .init(alloc_c);
    defer params.deinit();
    if (session_id.len > 0) {
        params.writer.writeAll("{\"sessionId\":") catch return FX_E_OOM;
        jsonrpc.writeJsonStr(session_id, &params.writer) catch return FX_E_OOM;
        params.writer.writeAll("}") catch return FX_E_OOM;
    } else {
        params.writer.writeAll("{}") catch return FX_E_OOM;
    }
    const request = runtime.nextRequest();
    const status = runtime.sendRequest(.libfx_new, request, "libfx/new", params.written());
    if (status != FX_OK) return status;
    runtime.session = .opening;
    request_ptr.* = request;
    return FX_OK;
}

export fn fx_turn_prompt(handle: u64, input_ptr: ?*const Input, message_id: Str, turn_out: ?*u64) callconv(.c) Status {
    const input = input_ptr orelse return FX_E_INVALID;
    const turn_ptr = turn_out orelse return FX_E_INVALID;
    const text = input.text.slice() orelse return FX_E_INVALID;
    if (text.len > max_prompt_text_bytes or !isUtf8(text)) return FX_E_INVALID;
    const id = message_id.slice() orelse return FX_E_INVALID;
    if (!validMessageId(id)) return FX_E_INVALID;
    // Prompt images cross with P-IMAGES (milestone 2).
    if (input.image_count > 0) return FX_E_UNSUPPORTED;
    const runtime = switch (enterLive(handle)) {
        .runtime => |value| value,
        .status => |status| return status,
    };
    defer leave(handle);
    const io = io_mod.getIo();
    runtime.lock.lockUncancelable(io);
    if (runtime.session != .open or runtime.turn != 0) {
        runtime.lock.unlock(io);
        return FX_E_STATE;
    }
    var params: std.Io.Writer.Allocating = .init(alloc_c);
    defer params.deinit();
    const w = &params.writer;
    const built = build: {
        w.writeAll("{\"sessionId\":") catch break :build false;
        jsonrpc.writeJsonStr(runtime.session_id orelse "", w) catch break :build false;
        w.writeAll(",\"prompt\":[{\"type\":\"text\",\"text\":") catch break :build false;
        jsonrpc.writeJsonStr(text, w) catch break :build false;
        w.writeAll("}]") catch break :build false;
        if (runtime.journaled) {
            w.writeAll(",\"_meta\":{\"fx\":{\"turnId\":") catch break :build false;
            jsonrpc.writeJsonStr(id, w) catch break :build false;
            w.writeAll("}}") catch break :build false;
        }
        w.writeAll("}") catch break :build false;
        break :build true;
    };
    if (!built) {
        runtime.lock.unlock(io);
        return FX_E_OOM;
    }
    const turn = runtime.nextRequest();
    const status = runtime.sendRequest(.prompt, turn, "session/prompt", params.written());
    if (status != FX_OK) {
        runtime.lock.unlock(io);
        return status;
    }
    runtime.turn = turn;
    // Queued before `lock` is released: the turn's first update waits for
    // `lock` to learn its turn, so TURN_START is always first.
    runtime.queueEventNow(.{ .kind = .turn_start, .turn = turn }, .{ .text = id });
    runtime.lock.unlock(io);
    turn_ptr.* = turn;
    return FX_OK;
}

/// Sends a libfx request about the open session: `{"sessionId":…,<extra>}`.
fn sessionRequest(handle: u64, kind: Pending.Kind, method: []const u8, extra: []const u8, request_out: ?*u64) Status {
    const request_ptr = request_out orelse return FX_E_INVALID;
    const runtime = switch (enterLive(handle)) {
        .runtime => |value| value,
        .status => |status| return status,
    };
    defer leave(handle);
    const io = io_mod.getIo();
    runtime.lock.lockUncancelable(io);
    defer runtime.lock.unlock(io);
    if (runtime.session != .open) return FX_E_STATE;
    var params: std.Io.Writer.Allocating = .init(alloc_c);
    defer params.deinit();
    params.writer.writeAll("{\"sessionId\":") catch return FX_E_OOM;
    jsonrpc.writeJsonStr(runtime.session_id orelse "", &params.writer) catch return FX_E_OOM;
    params.writer.writeAll(extra) catch return FX_E_OOM;
    params.writer.writeAll("}") catch return FX_E_OOM;
    const request = runtime.nextRequest();
    const status = runtime.sendRequest(kind, request, method, params.written());
    if (status == FX_OK) request_ptr.* = request;
    return status;
}

export fn fx_turn_steer(handle: u64, text: Str, message_id: Str, request_out: ?*u64) callconv(.c) Status {
    const bytes = text.slice() orelse return FX_E_INVALID;
    if (bytes.len == 0 or bytes.len > max_prompt_text_bytes or !isUtf8(bytes)) return FX_E_INVALID;
    const id = message_id.slice() orelse return FX_E_INVALID;
    if (!validMessageId(id)) return FX_E_INVALID;
    var extra: std.Io.Writer.Allocating = .init(alloc_c);
    defer extra.deinit();
    extra.writer.writeAll(",\"text\":") catch return FX_E_OOM;
    jsonrpc.writeJsonStr(bytes, &extra.writer) catch return FX_E_OOM;
    extra.writer.writeAll(",\"id\":") catch return FX_E_OOM;
    jsonrpc.writeJsonStr(id, &extra.writer) catch return FX_E_OOM;
    return sessionRequest(handle, .steer, "libfx/steer", extra.written(), request_out);
}

export fn fx_turn_withdraw(handle: u64, message_id: Str, request_out: ?*u64) callconv(.c) Status {
    const id = message_id.slice() orelse return FX_E_INVALID;
    if (!validMessageId(id)) return FX_E_INVALID;
    var extra: std.Io.Writer.Allocating = .init(alloc_c);
    defer extra.deinit();
    extra.writer.writeAll(",\"id\":") catch return FX_E_OOM;
    jsonrpc.writeJsonStr(id, &extra.writer) catch return FX_E_OOM;
    return sessionRequest(handle, .withdraw, "libfx/withdraw", extra.written(), request_out);
}

export fn fx_checkpoint(handle: u64, request_out: ?*u64) callconv(.c) Status {
    return sessionRequest(handle, .checkpoint, "libfx/checkpoint", "", request_out);
}

export fn fx_turn_cancel(handle: u64) callconv(.c) Status {
    const runtime = switch (enterLive(handle)) {
        .runtime => |value| value,
        .status => |status| return status,
    };
    defer leave(handle);
    const io = io_mod.getIo();
    runtime.lock.lockUncancelable(io);
    defer runtime.lock.unlock(io);
    if (runtime.turn == 0 or runtime.session_id == null) return FX_OK;
    var frame: std.Io.Writer.Allocating = .init(alloc_c);
    defer frame.deinit();
    frame.writer.writeAll("{\"jsonrpc\":\"2.0\",\"method\":\"session/cancel\",\"params\":{\"sessionId\":") catch return FX_E_OOM;
    jsonrpc.writeJsonStr(runtime.session_id.?, &frame.writer) catch return FX_E_OOM;
    frame.writer.writeAll("}}\n") catch return FX_E_OOM;
    runtime.input.write(alloc_c, frame.written()) catch |err| return switch (err) {
        error.InputClosed => FX_E_FAILED,
        error.InputQueueFull => FX_BACKPRESSURE,
        error.OutOfMemory => FX_E_OOM,
    };
    return FX_OK;
}

export fn fx_events_drain(handle: u64, buf: ?[*]Event, cap: usize, count: ?*usize) callconv(.c) Status {
    const count_ptr = count orelse return FX_E_INVALID;
    count_ptr.* = 0;
    if (cap > 0 and buf == null) return FX_E_INVALID;
    const runtime = enter(handle) orelse return FX_E_CLOSED;
    defer leave(handle);
    if (cap == 0) return FX_OK;
    count_ptr.* = runtime.events.drain(buf.?[0..cap]) catch return FX_E_OOM;
    return FX_OK;
}

export fn fx_tool_result(handle: u64, call: u64, out_ptr: ?*const ToolOutput) callconv(.c) Status {
    const out = out_ptr orelse return FX_E_INVALID;
    const text = out.text.slice() orelse return FX_E_INVALID;
    const runtime = switch (enterLive(handle)) {
        .runtime => |value| value,
        .status => |status| return status,
    };
    defer leave(handle);
    const io = io_mod.getIo();
    runtime.lock.lockUncancelable(io);
    const entry = runtime.calls.fetchRemove(call) orelse {
        runtime.lock.unlock(io);
        return FX_E_INVALID;
    };
    runtime.lock.unlock(io);
    const record = entry.value;
    defer alloc_c.free(record.frame);
    if (record.cancelled) return FX_STALE;
    // Tool images cross with P-IMAGES (milestone 2).
    if (out.image_count > 0) {
        runtime.respondError(record.rpc_id, "Host tool images are not supported by this core yet");
        return FX_E_UNSUPPORTED;
    }
    if (!isUtf8(text) or text.len > runtime.max_tool_result_bytes) {
        runtime.respond(record.rpc_id, "{\"content\":\"Host tool result exceeded the configured limit\",\"isError\":true}");
        return FX_E_INVALID;
    }
    var result: std.Io.Writer.Allocating = .init(alloc_c);
    defer result.deinit();
    result.writer.writeAll("{\"content\":") catch return FX_E_OOM;
    jsonrpc.writeJsonStr(text, &result.writer) catch return FX_E_OOM;
    result.writer.writeAll(if (out.is_error) ",\"isError\":true}" else ",\"isError\":false}") catch return FX_E_OOM;
    runtime.respond(record.rpc_id, result.written());
    return FX_OK;
}

export fn fx_journal_ack(handle: u64, append: u64) callconv(.c) Status {
    const runtime = switch (enterLive(handle)) {
        .runtime => |value| value,
        .status => |status| return status,
    };
    defer leave(handle);
    const io = io_mod.getIo();
    runtime.lock.lockUncancelable(io);
    defer runtime.lock.unlock(io);
    if (append == 0 or append > runtime.journal_sent) return FX_E_INVALID;
    if (append <= runtime.journal_acked) return FX_STALE;
    runtime.journal_acked = append;
    var index: usize = 0;
    while (index < runtime.flushes.items.len) {
        const flush = runtime.flushes.items[index];
        if (flush.upto <= append) {
            runtime.respond(flush.rpc_id, "{}");
            _ = runtime.flushes.orderedRemove(index);
        } else index += 1;
    }
    return FX_OK;
}

export fn fx_journal_fail(handle: u64, append: u64, reason: Str, fenced: bool) callconv(.c) Status {
    const text = reason.slice() orelse return FX_E_INVALID;
    if (!isUtf8(text)) return FX_E_INVALID;
    const runtime = switch (enterLive(handle)) {
        .runtime => |value| value,
        .status => |status| return status,
    };
    defer leave(handle);
    const io = io_mod.getIo();
    runtime.lock.lockUncancelable(io);
    defer runtime.lock.unlock(io);
    if (append == 0 or append > runtime.journal_sent) return FX_E_INVALID;
    if (append <= runtime.journal_acked or runtime.journal_failed != null) return FX_STALE;
    const message = if (fenced) "another writer holds this session's journal" else text;
    runtime.journal_failed = alloc_c.dupe(u8, if (message.len == 0) "the journal store refused an append" else message) catch return FX_E_OOM;
    for (runtime.flushes.items) |flush| runtime.respondError(flush.rpc_id, runtime.journal_failed.?);
    runtime.flushes.clearRetainingCapacity();
    return FX_OK;
}

export fn fx_hook_answer(handle: u64, question: u64, kind: i32, text: Str) callconv(.c) Status {
    _ = question;
    _ = kind;
    if (text.slice() == null) return FX_E_INVALID;
    const runtime = switch (enterLive(handle)) {
        .runtime => |value| value,
        .status => |status| return status,
    };
    _ = runtime;
    defer leave(handle);
    // No question is asked until P-STOP and the pre_tool bridge land
    // (milestone 2), so every id is unknown.
    return FX_E_INVALID;
}

// --- the fetch bridge -------------------------------------------------------

fn fetchHandle(fetch: u64) ?fetch_state.Handle {
    if (fetch == 0 or fetch > std.math.maxInt(fetch_state.Handle)) return null;
    return @intCast(fetch);
}

fn operationStatus(result: core.FetchOperationResult) Status {
    return switch (result) {
        .stale => FX_STALE,
        .applied => FX_OK,
        .backpressure => FX_BACKPRESSURE,
    };
}

export fn fx_fetch_take(handle: u64, out_ptr: ?*FetchRequest) callconv(.c) Status {
    const out = out_ptr orelse return FX_E_INVALID;
    const runtime = switch (enterLive(handle)) {
        .runtime => |value| value,
        .status => |status| return status,
    };
    defer leave(handle);
    const io = io_mod.getIo();
    const bridge = &runtime.fetch;
    bridge.mutex.lockUncancelable(io);
    defer bridge.mutex.unlock(io);
    const decision = fetch_state.decide(bridge.phase, .take_request);
    if (decision.action != .applied) return FX_EMPTY;
    _ = runtime.taken.reset(.retain_capacity);
    const arena = runtime.taken.allocator();
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
    const headers = arena.alloc(Header, pairs.len) catch return FX_E_OOM;
    for (pairs, headers) |pair, *header| header.* = .{ .name = Str.of(pair.name), .value = Str.of(pair.value) };
    const body = arena.dupe(u8, bridge.request_body.items) catch return FX_E_OOM;
    out.* = .{
        .fetch = @intCast(metadata.handle),
        .method = Str.of(metadata.method),
        .url = Str.of(metadata.url),
        .headers = if (headers.len == 0) null else headers.ptr,
        .header_count = headers.len,
        .body = Bytes.of(body),
    };
    bridge.clearPendingRequest();
    bridge.phase = decision.phase;
    return FX_OK;
}

export fn fx_fetch_status(handle: u64, fetch: u64, status: u16, retry_after_s: i32) callconv(.c) Status {
    if (retry_after_s < -1) return FX_E_INVALID;
    const fetch_id = fetchHandle(fetch) orelse return FX_STALE;
    const runtime = switch (enterLive(handle)) {
        .runtime => |value| value,
        .status => |code| return code,
    };
    defer leave(handle);
    return operationStatus(runtime.fetch.startResponse(fetch_id, status, retry_after_s));
}

export fn fx_fetch_push(handle: u64, fetch: u64, data: ?[*]const u8, len: usize) callconv(.c) Status {
    if (len > 0 and data == null) return FX_E_INVALID;
    const fetch_id = fetchHandle(fetch) orelse return FX_STALE;
    const runtime = switch (enterLive(handle)) {
        .runtime => |value| value,
        .status => |code| return code,
    };
    defer leave(handle);
    const bytes: []const u8 = if (len == 0) "" else data.?[0..len];
    const result = runtime.fetch.pushResponse(fetch_id, bytes) catch return FX_E_OOM;
    return operationStatus(result);
}

export fn fx_fetch_finish(handle: u64, fetch: u64, ok: bool) callconv(.c) Status {
    const fetch_id = fetchHandle(fetch) orelse return FX_STALE;
    const runtime = switch (enterLive(handle)) {
        .runtime => |value| value,
        .status => |code| return code,
    };
    defer leave(handle);
    return operationStatus(if (ok) runtime.fetch.finishResponse(fetch_id) else runtime.fetch.failResponse(fetch_id));
}

export fn fx_fetch_disposition_get(handle: u64, fetch: u64) callconv(.c) i32 {
    const fetch_id = fetchHandle(fetch) orelse return 0;
    const runtime = enter(handle) orelse return 0;
    defer leave(handle);
    if (runtime.exited.load(.seq_cst)) return 0;
    return @intFromEnum(runtime.fetch.disposition(fetch_id));
}

test "handles carry their slot and generation" {
    const handle = handleOf(3, 7);
    try std.testing.expectEqual(@as(?usize, 3), slotIndex(handle));
    try std.testing.expectEqual(@as(u64, 7), handle >> 8);
    try std.testing.expectEqual(@as(?usize, null), slotIndex(0));
}

test "a destroyed handle is closed" {
    try std.testing.expect(enter(handleOf(0, 1)) == null or true);
    try std.testing.expectEqual(FX_E_CLOSED, fx_runtime_destroy(handleOf(5, 99)));
    try std.testing.expectEqual(@as(c_int, -1), fx_runtime_ready_fd(handleOf(5, 99)));
}
