const std = @import("std");
const debug_trace = @import("../shared/debug_trace.zig");
const io_mod = @import("../shared/io.zig");
const prompt_history_provider = @import("../session/prompt_history_provider.zig");
const prompt_history_store = @import("../session/prompt_history_store.zig");

const Allocator = std.mem.Allocator;

pub const InitializeOutcome = enum {
    available,
    unavailable,
};

pub const RecordOutcome = enum {
    appended,
    duplicate,
    record_too_large,
    skipped_disabled,
    skipped_unavailable,
    warning,
    trace_only,
};

pub const PromptHistoryRuntime = struct {
    enabled: bool = true,
    loaded_count: usize = 0,
    store: ?prompt_history_store.Store = null,
    provider: ?prompt_history_provider.Provider = null,
    append_warning_active: bool = false,
    writes_available: bool = false,
    last_append_error: ?anyerror = null,

    pub fn initialize(
        self: *PromptHistoryRuntime,
        alloc: Allocator,
        home_path: ?[]const u8,
        enabled: bool,
        store_allowed: bool,
    ) !InitializeOutcome {
        self.enabled = enabled;
        self.loaded_count = 0;
        self.append_warning_active = false;
        self.last_append_error = null;
        self.writes_available = false;
        if (self.provider != null) {
            self.writes_available = true;
            return .available;
        }
        if (!store_allowed or home_path == null) return .unavailable;

        self.store = prompt_history_store.Store.initFromHome(
            alloc,
            home_path.?,
        ) catch |err| {
            if (err == error.OutOfMemory) return err;
            debug_trace.logf(
                "prompt_history",
                "store initialization unavailable err={s}",
                .{@errorName(err)},
            );
            return .unavailable;
        };
        self.writes_available = true;
        return .available;
    }

    pub fn initializeWithProvider(
        self: *PromptHistoryRuntime,
        enabled: bool,
        provider: prompt_history_provider.Provider,
    ) InitializeOutcome {
        self.enabled = enabled;
        self.loaded_count = 0;
        self.append_warning_active = false;
        self.last_append_error = null;
        self.provider = provider;
        self.writes_available = true;
        return .available;
    }

    pub fn deinit(self: *PromptHistoryRuntime, alloc: Allocator) void {
        if (self.store) |*store| store.deinit(alloc);
        self.* = .{};
    }

    pub fn loadRecent(
        self: *PromptHistoryRuntime,
        alloc: Allocator,
        workspace_root: []const u8,
        limit: usize,
    ) ![]prompt_history_store.LoadedPromptHistoryEntry {
        if (!self.enabled) {
            self.loaded_count = 0;
            return alloc.alloc(prompt_history_store.LoadedPromptHistoryEntry, 0);
        }
        const entries = if (self.provider) |provider|
            try provider.loadRecent(alloc, workspace_root, limit)
        else if (self.store) |*store|
            try store.loadRecentForWorkspace(alloc, workspace_root, limit)
        else blk: {
            self.loaded_count = 0;
            break :blk try alloc.alloc(prompt_history_store.LoadedPromptHistoryEntry, 0);
        };
        self.loaded_count = entries.len;
        return entries;
    }

    pub fn recordAccepted(
        self: *PromptHistoryRuntime,
        alloc: Allocator,
        timestamp_ms: i64,
        workspace_root: []const u8,
        text: []const u8,
    ) RecordOutcome {
        if (!self.enabled) return .skipped_disabled;
        if ((self.provider == null and self.store == null) or !self.writes_available) {
            return .skipped_unavailable;
        }

        const outcome = (if (self.provider) |provider|
            provider.append(alloc, timestamp_ms, workspace_root, text)
        else
            self.store.?.append(alloc, timestamp_ms, workspace_root, text)) catch |err| {
            self.last_append_error = err;
            debug_trace.logf(
                "prompt_history",
                "append failed bytes={d} err={s}",
                .{ text.len, @errorName(err) },
            );
            if (err == error.PromptHistoryLockUnsupported) {
                self.writes_available = false;
            }
            if (!self.append_warning_active) {
                self.append_warning_active = true;
                return .warning;
            }
            return .trace_only;
        };

        self.append_warning_active = false;
        self.last_append_error = null;
        return switch (outcome) {
            .appended => .appended,
            .duplicate => .duplicate,
            .record_too_large => .record_too_large,
            .compaction_stale => blk: {
                debug_trace.logf(
                    "prompt_history",
                    "compaction stale after append",
                    .{},
                );
                break :blk .appended;
            },
            .compaction_indeterminate => blk: {
                debug_trace.logf(
                    "prompt_history",
                    "compaction indeterminate after append",
                    .{},
                );
                break :blk .appended;
            },
        };
    }

    pub fn clearWorkspace(
        self: *PromptHistoryRuntime,
        alloc: Allocator,
        workspace_root: []const u8,
    ) !void {
        if (self.provider == null and self.store == null) return error.PromptHistoryUnavailable;
        if (!self.writes_available) return error.PromptHistoryLockUnsupported;
        if (self.provider) |provider| {
            try provider.clearWorkspace(alloc, workspace_root);
        } else {
            try self.store.?.clearWorkspace(alloc, workspace_root);
        }
        self.loaded_count = 0;
    }

    pub fn disable(self: *PromptHistoryRuntime) void {
        self.enabled = false;
    }

    pub fn enable(self: *PromptHistoryRuntime) void {
        self.enabled = true;
    }

    pub fn displayPath(self: *const PromptHistoryRuntime) ?[]const u8 {
        if (self.provider) |provider| return provider.display_name;
        return if (self.store) |*store| store.displayPath() else null;
    }

    pub fn lastAppendError(self: *const PromptHistoryRuntime) ?anyerror {
        return self.last_append_error;
    }

    pub fn setStoreLockOpsForTest(
        self: *PromptHistoryRuntime,
        ops: io_mod.LockOps,
    ) void {
        if (self.store) |*store| store.setLockOpsForTest(ops);
    }

    pub fn clearStoreTestControls(self: *PromptHistoryRuntime) void {
        if (self.store) |*store| store.clearTestControls();
        self.writes_available = self.store != null;
    }

    pub fn loadRecentIncludingDisabledForTest(
        self: *PromptHistoryRuntime,
        alloc: Allocator,
        workspace_root: []const u8,
        limit: usize,
    ) ![]prompt_history_store.LoadedPromptHistoryEntry {
        if (self.provider) |provider| {
            return provider.loadRecent(alloc, workspace_root, limit);
        }
        if (self.store) |*store| {
            return store.loadRecentForWorkspace(alloc, workspace_root, limit);
        }
        return alloc.alloc(prompt_history_store.LoadedPromptHistoryEntry, 0);
    }
};

test "runtime loads enabled history and tracks loaded count" {
    const alloc = std.testing.allocator;
    var tmp = std.testing.tmpDir(.{});
    defer tmp.cleanup();
    const home = try io_mod.dirRealpathAlloc(alloc, tmp.dir, ".");
    defer alloc.free(home);

    var seed = try prompt_history_store.Store.initFromHome(alloc, home);
    defer seed.deinit(alloc);
    _ = try seed.append(alloc, 1, "/tmp/workspace", "one");
    _ = try seed.append(alloc, 2, "/tmp/workspace", "two");

    var runtime = PromptHistoryRuntime{};
    defer runtime.deinit(alloc);
    try std.testing.expectEqual(
        InitializeOutcome.available,
        try runtime.initialize(alloc, home, true, true),
    );
    const entries = try runtime.loadRecent(alloc, "/tmp/workspace", 100);
    defer prompt_history_store.freeLoadedEntries(alloc, entries);
    try std.testing.expectEqual(@as(usize, 2), runtime.loaded_count);
    try std.testing.expectEqualStrings("one", entries[0].text);
    try std.testing.expectEqualStrings("two", entries[1].text);
}

test "runtime disabled or unavailable history performs no disk load or append" {
    const alloc = std.testing.allocator;
    var tmp = std.testing.tmpDir(.{});
    defer tmp.cleanup();
    const home = try io_mod.dirRealpathAlloc(alloc, tmp.dir, ".");
    defer alloc.free(home);

    var runtime = PromptHistoryRuntime{};
    defer runtime.deinit(alloc);
    try std.testing.expectEqual(
        InitializeOutcome.unavailable,
        try runtime.initialize(alloc, home, true, false),
    );
    try std.testing.expectEqual(
        RecordOutcome.skipped_unavailable,
        runtime.recordAccepted(alloc, 1, "/tmp/workspace", "prompt"),
    );

    runtime.enabled = false;
    try std.testing.expectEqual(
        RecordOutcome.skipped_disabled,
        runtime.recordAccepted(alloc, 2, "/tmp/workspace", "prompt"),
    );
}

const BusyLockState = struct {
    now_ms: i64 = 0,

    fn busy(_: ?*anyopaque, _: std.Io.File) anyerror!bool {
        return false;
    }

    fn now(ctx: ?*anyopaque) i64 {
        const self: *BusyLockState = @ptrCast(@alignCast(ctx.?));
        return self.now_ms;
    }

    fn sleep(ctx: ?*anyopaque, millis: u64) void {
        const self: *BusyLockState = @ptrCast(@alignCast(ctx.?));
        self.now_ms += @intCast(millis);
    }
};

test "runtime emits one append warning per failure interval and resets after success" {
    const alloc = std.testing.allocator;
    var tmp = std.testing.tmpDir(.{});
    defer tmp.cleanup();
    const home = try io_mod.dirRealpathAlloc(alloc, tmp.dir, ".");
    defer alloc.free(home);

    var runtime = PromptHistoryRuntime{};
    defer runtime.deinit(alloc);
    _ = try runtime.initialize(alloc, home, true, true);
    var state = BusyLockState{};
    runtime.setStoreLockOpsForTest(.{
        .ctx = &state,
        .try_lock = BusyLockState.busy,
        .now_ms = BusyLockState.now,
        .sleep_ms = BusyLockState.sleep,
    });

    try std.testing.expectEqual(
        RecordOutcome.warning,
        runtime.recordAccepted(alloc, 1, "/tmp/workspace", "one"),
    );
    try std.testing.expectEqual(
        RecordOutcome.trace_only,
        runtime.recordAccepted(alloc, 2, "/tmp/workspace", "two"),
    );

    runtime.clearStoreTestControls();
    try std.testing.expectEqual(
        RecordOutcome.appended,
        runtime.recordAccepted(alloc, 3, "/tmp/workspace", "three"),
    );
    runtime.setStoreLockOpsForTest(.{
        .ctx = &state,
        .try_lock = BusyLockState.busy,
        .now_ms = BusyLockState.now,
        .sleep_ms = BusyLockState.sleep,
    });
    try std.testing.expectEqual(
        RecordOutcome.warning,
        runtime.recordAccepted(alloc, 4, "/tmp/workspace", "four"),
    );
}

test "runtime clear remains available while recording is disabled" {
    const alloc = std.testing.allocator;
    var tmp = std.testing.tmpDir(.{});
    defer tmp.cleanup();
    const home = try io_mod.dirRealpathAlloc(alloc, tmp.dir, ".");
    defer alloc.free(home);

    var runtime = PromptHistoryRuntime{};
    defer runtime.deinit(alloc);
    _ = try runtime.initialize(alloc, home, true, true);
    try std.testing.expectEqual(
        RecordOutcome.appended,
        runtime.recordAccepted(alloc, 1, "/tmp/workspace", "saved"),
    );
    runtime.enabled = false;
    try runtime.clearWorkspace(alloc, "/tmp/workspace");
    const entries = try runtime.loadRecentIncludingDisabledForTest(
        alloc,
        "/tmp/workspace",
        100,
    );
    defer prompt_history_store.freeLoadedEntries(alloc, entries);
    try std.testing.expectEqual(@as(usize, 0), entries.len);
}
