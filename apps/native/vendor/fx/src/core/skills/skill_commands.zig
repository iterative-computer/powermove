const std = @import("std");

const Allocator = std.mem.Allocator;

pub const InstallResult = struct {
    /// Owned names and backing storage returned by the provider.
    installed: std.ArrayList([]u8),

    pub fn deinit(self: *InstallResult, alloc: Allocator) void {
        for (self.installed.items) |name| alloc.free(name);
        self.installed.deinit(alloc);
    }
};

pub const SkillInfo = struct {
    path: []const u8,
    source_label: []const u8,
    managed_install: bool,
};

pub const FindSkillFn = *const fn (ctx: *anyopaque, name: []const u8) ?SkillInfo;

pub const CommandRequest = struct {
    skills_dir: []const u8,
    find_ctx: *anyopaque,
    find_skill: FindSkillFn,
};

pub const InstallCommand = struct {
    source: []const u8,
    filter: ?[]const u8,
};

/// Parsed command slices borrow from the original command input.
pub const Command = union(enum) {
    list,
    show: []const u8,
    install: InstallCommand,
    create: []const u8,
    remove: []const u8,
    path,
    usage,
};

pub const Notice = struct {
    text: []u8,
    reload: bool = false,
};

/// Provider output. Slice payloads are owned and released by `deinit`.
pub const CommandResult = union(enum) {
    open_menu,
    focus_menu: []u8,
    notice: Notice,
    installed: InstallResult,

    /// Releases every allocation returned by a provider.
    pub fn deinit(self: *CommandResult, alloc: Allocator) void {
        switch (self.*) {
            .open_menu => {},
            .focus_menu => |name| alloc.free(name),
            .notice => |notice| alloc.free(notice.text),
            .installed => |*result| result.deinit(alloc),
        }
        self.* = undefined;
    }
};

pub const ParseCommandFn = *const fn (rest: []const u8) Command;
pub const ExecuteCommandFn = *const fn (
    alloc: Allocator,
    command: Command,
    request: CommandRequest,
) anyerror!CommandResult;

/// Product behavior supplied to the Core skills command harness.
pub const Provider = struct {
    parse_command_fn: ParseCommandFn,
    execute_command_fn: ExecuteCommandFn,

    pub fn parseCommand(self: Provider, rest: []const u8) Command {
        return self.parse_command_fn(rest);
    }

    pub fn executeCommand(
        self: Provider,
        alloc: Allocator,
        command: Command,
        request: CommandRequest,
    ) !CommandResult {
        return self.execute_command_fn(alloc, command, request);
    }
};
