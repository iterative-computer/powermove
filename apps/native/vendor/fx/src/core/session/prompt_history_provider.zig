const std = @import("std");
const prompt_history_store = @import("prompt_history_store.zig");

const Allocator = std.mem.Allocator;

pub const AppendOutcome = prompt_history_store.AppendOutcome;
pub const LoadedEntry = prompt_history_store.LoadedPromptHistoryEntry;

pub const Provider = struct {
    ctx: ?*anyopaque = null,
    load_recent_fn: *const fn (
        ctx: ?*anyopaque,
        alloc: Allocator,
        workspace_root: []const u8,
        limit: usize,
    ) anyerror![]LoadedEntry,
    append_fn: *const fn (
        ctx: ?*anyopaque,
        alloc: Allocator,
        timestamp_ms: i64,
        workspace_root: []const u8,
        text: []const u8,
    ) anyerror!AppendOutcome,
    clear_workspace_fn: *const fn (
        ctx: ?*anyopaque,
        alloc: Allocator,
        workspace_root: []const u8,
    ) anyerror!void,
    display_name: ?[]const u8 = null,

    pub fn loadRecent(
        self: Provider,
        alloc: Allocator,
        workspace_root: []const u8,
        limit: usize,
    ) ![]LoadedEntry {
        return self.load_recent_fn(self.ctx, alloc, workspace_root, limit);
    }

    pub fn append(
        self: Provider,
        alloc: Allocator,
        timestamp_ms: i64,
        workspace_root: []const u8,
        text: []const u8,
    ) !AppendOutcome {
        return self.append_fn(self.ctx, alloc, timestamp_ms, workspace_root, text);
    }

    pub fn clearWorkspace(
        self: Provider,
        alloc: Allocator,
        workspace_root: []const u8,
    ) !void {
        return self.clear_workspace_fn(self.ctx, alloc, workspace_root);
    }
};
