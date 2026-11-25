//! Powermove notice: new file, not in upstream fx (patch P-TOOLS). A host
//! tool executor the embedder installs in `acp_runner.Config`, so a host tool
//! call reaches the host directly, with the model's input as the core holds
//! it, instead of as a JSON-RPC `libfx/tool_call` frame and back.
const std = @import("std");
const tool_dispatch = @import("tool_dispatch.zig");

pub const CallFn = *const fn (
    context: *anyopaque,
    alloc: std.mem.Allocator,
    name: []const u8,
    call_id: []const u8,
    arguments_json: []const u8,
    max_result_bytes: usize,
    cancel_flag: ?*std.atomic.Value(bool),
) tool_dispatch.DispatchError!tool_dispatch.ToolResult;

pub const Bridge = struct {
    context: *anyopaque,
    call_fn: CallFn,
};
