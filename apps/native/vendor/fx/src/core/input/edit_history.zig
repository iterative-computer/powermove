const std = @import("std");

const Allocator = std.mem.Allocator;

const max_entries: usize = 100;
const max_retained_bytes: usize = 1024 * 1024;

pub const Entry = struct {
    start: usize,
    removed: std.ArrayList(u8) = .empty,
    inserted: std.ArrayList(u8) = .empty,
    cursor_before: usize,
    cursor_after: usize,

    fn init(
        alloc: Allocator,
        start: usize,
        removed: []const u8,
        inserted: []const u8,
        cursor_before: usize,
        cursor_after: usize,
    ) !Entry {
        var entry: Entry = .{
            .start = start,
            .cursor_before = cursor_before,
            .cursor_after = cursor_after,
        };
        errdefer entry.deinit(alloc);
        try entry.removed.appendSlice(alloc, removed);
        try entry.inserted.appendSlice(alloc, inserted);
        return entry;
    }

    fn deinit(self: *Entry, alloc: Allocator) void {
        self.removed.deinit(alloc);
        self.inserted.deinit(alloc);
        self.* = undefined;
    }

    fn retainedBytes(self: Entry) usize {
        return self.removed.items.len +| self.inserted.items.len;
    }
};

pub const Prepared = union(enum) {
    record: Entry,
    boundary,

    pub fn deinit(self: *Prepared, alloc: Allocator) void {
        switch (self.*) {
            .record => |*entry| entry.deinit(alloc),
            .boundary => {},
        }
        self.* = .boundary;
    }
};

pub const State = struct {
    undo_stack: std.ArrayList(Entry) = .empty,
    redo_stack: std.ArrayList(Entry) = .empty,
    retained_bytes: usize = 0,

    pub fn deinit(self: *State, alloc: Allocator) void {
        self.clearStack(alloc, &self.undo_stack);
        self.clearStack(alloc, &self.redo_stack);
        self.undo_stack.deinit(alloc);
        self.redo_stack.deinit(alloc);
        self.* = .{};
    }

    pub fn reset(self: *State, alloc: Allocator) void {
        self.clearStack(alloc, &self.undo_stack);
        self.clearStack(alloc, &self.redo_stack);
        std.debug.assert(self.retained_bytes == 0);
    }

    pub fn prepare(
        self: *State,
        alloc: Allocator,
        start: usize,
        removed: []const u8,
        inserted: []const u8,
        cursor_before: usize,
        cursor_after: usize,
    ) !Prepared {
        const retained = removed.len +| inserted.len;
        if (retained > max_retained_bytes) return .boundary;
        try self.undo_stack.ensureUnusedCapacity(alloc, 1);
        return .{ .record = try Entry.init(
            alloc,
            start,
            removed,
            inserted,
            cursor_before,
            cursor_after,
        ) };
    }

    pub fn commit(self: *State, alloc: Allocator, prepared: *Prepared) void {
        switch (prepared.*) {
            .boundary => self.reset(alloc),
            .record => |*entry| {
                self.clearStack(alloc, &self.redo_stack);
                self.retained_bytes +|= entry.retainedBytes();
                self.undo_stack.appendAssumeCapacity(entry.*);
                prepared.* = .boundary;
                while (self.undo_stack.items.len > max_entries or
                    self.retained_bytes > max_retained_bytes)
                {
                    var dropped = self.undo_stack.orderedRemove(0);
                    self.retained_bytes -|= dropped.retainedBytes();
                    dropped.deinit(alloc);
                }
            },
        }
    }

    pub fn peekUndo(self: *const State) ?*const Entry {
        if (self.undo_stack.items.len == 0) return null;
        return &self.undo_stack.items[self.undo_stack.items.len - 1];
    }

    pub fn peekRedo(self: *const State) ?*const Entry {
        if (self.redo_stack.items.len == 0) return null;
        return &self.redo_stack.items[self.redo_stack.items.len - 1];
    }

    pub fn prepareUndo(self: *State, alloc: Allocator) !bool {
        if (self.undo_stack.items.len == 0) return false;
        try self.redo_stack.ensureUnusedCapacity(alloc, 1);
        return true;
    }

    pub fn prepareRedo(self: *State, alloc: Allocator) !bool {
        if (self.redo_stack.items.len == 0) return false;
        try self.undo_stack.ensureUnusedCapacity(alloc, 1);
        return true;
    }

    pub fn commitUndo(self: *State) void {
        self.redo_stack.appendAssumeCapacity(self.undo_stack.pop().?);
    }

    pub fn commitRedo(self: *State) void {
        self.undo_stack.appendAssumeCapacity(self.redo_stack.pop().?);
    }

    fn clearStack(
        self: *State,
        alloc: Allocator,
        stack: *std.ArrayList(Entry),
    ) void {
        for (stack.items) |*entry| {
            self.retained_bytes -|= entry.retainedBytes();
            entry.deinit(alloc);
        }
        stack.clearRetainingCapacity();
    }
};

test "text edit history moves owned deltas between undo and redo" {
    const alloc = std.testing.allocator;
    var state: State = .{};
    defer state.deinit(alloc);

    var prepared = try state.prepare(alloc, 2, "old", "new", 5, 5);
    defer prepared.deinit(alloc);
    state.commit(alloc, &prepared);
    try std.testing.expectEqualStrings("old", state.peekUndo().?.removed.items);

    try std.testing.expect(try state.prepareUndo(alloc));
    state.commitUndo();
    try std.testing.expect(state.peekUndo() == null);
    try std.testing.expectEqualStrings("new", state.peekRedo().?.inserted.items);

    try std.testing.expect(try state.prepareRedo(alloc));
    state.commitRedo();
    try std.testing.expect(state.peekRedo() == null);
    try std.testing.expectEqual(@as(usize, 1), state.undo_stack.items.len);
}

test "structured boundaries discard both edit directions" {
    const alloc = std.testing.allocator;
    var state: State = .{};
    defer state.deinit(alloc);

    var first = try state.prepare(alloc, 0, "", "a", 0, 1);
    defer first.deinit(alloc);
    state.commit(alloc, &first);
    try std.testing.expect(try state.prepareUndo(alloc));
    state.commitUndo();

    var boundary: Prepared = .boundary;
    state.commit(alloc, &boundary);
    try std.testing.expect(state.peekUndo() == null);
    try std.testing.expect(state.peekRedo() == null);
    try std.testing.expectEqual(@as(usize, 0), state.retained_bytes);
}
