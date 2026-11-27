const types = @import("../shared/types.zig");

pub const ToolPolicy = enum {
    full,
    read_only,
};

pub const ModeSpec = struct {
    id: []const u8,
    name: []const u8,
    description: []const u8 = "",
    permission_mode: types.PermissionMode = .ask,
    tool_policy: ToolPolicy = .full,
    tool_policy_denial_message: ?[]const u8 = null,
};
