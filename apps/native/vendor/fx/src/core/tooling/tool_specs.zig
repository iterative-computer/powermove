const std = @import("std");
const tool_dispatch = @import("tool_dispatch.zig");
const model_tool_schema = @import("model_tool_schema.zig");

// Generic aliases and helpers shared by registered tool specifications.

pub const ExecutorKind = tool_dispatch.ExecutorKind;
pub const LabelArgKind = tool_dispatch.LabelArgKind;
pub const PermissionTargetKind = tool_dispatch.PermissionTargetKind;
pub const ToolSpec = tool_dispatch.Tool;

pub fn toolGatewaySchemaJson(alloc: std.mem.Allocator, spec: ToolSpec) ![]u8 {
    return model_tool_schema.builtinFunctionSchemaJsonAlloc(alloc, spec.model_schema);
}

pub fn toolLabelValue(spec: ToolSpec, args: std.json.ObjectMap) ?[]const u8 {
    return tool_dispatch.toolLabelValue(spec, args);
}
