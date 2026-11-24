const std = @import("std");
const workspace_access = @import("workspace_access.zig");

pub const Action = union(enum) {
    add,
    remove: usize,
    clear,
};

pub const State = struct {
    active: bool = false,
    selected_index: usize = 0,

    pub fn open(self: *State) void {
        self.* = .{ .active = true };
    }

    pub fn close(self: *State) void {
        self.* = .{};
    }

    pub fn actionCount(entries: []const workspace_access.Entry) usize {
        const saved_count = savedEntryCount(entries);
        return 1 + saved_count + @intFromBool(saved_count > 0);
    }

    pub fn rowCount(entries: []const workspace_access.Entry) usize {
        return 1 + entries.len + @intFromBool(savedEntryCount(entries) > 0);
    }

    pub fn move(self: *State, delta: i32, entries: []const workspace_access.Entry) bool {
        const count = actionCount(entries);
        if (!self.active or delta == 0 or count == 0) return false;
        const count_signed: i32 = @intCast(count);
        var next: i32 = @intCast(self.selected_index % count);
        next += delta;
        while (next < 0) next += count_signed;
        while (next >= count_signed) next -= count_signed;
        self.selected_index = @intCast(next);
        return true;
    }

    pub fn selectedAction(self: *const State, entries: []const workspace_access.Entry) ?Action {
        if (!self.active) return null;
        const count = actionCount(entries);
        if (count == 0) return null;
        const selected = self.selected_index % count;
        if (selected == 0) return .add;

        var saved_index: usize = 1;
        for (entries, 0..) |entry, entry_index| {
            if (!entry.saved) continue;
            if (selected == saved_index) return .{ .remove = entry_index };
            saved_index += 1;
        }
        return .clear;
    }

    pub fn selectedRow(self: *const State, entries: []const workspace_access.Entry) ?usize {
        if (!self.active) return null;
        const selected = self.selected_index % actionCount(entries);
        if (selected == 0) return 0;

        var saved_index: usize = 1;
        for (entries, 0..) |entry, entry_index| {
            if (!entry.saved) continue;
            if (selected == saved_index) return entry_index + 1;
            saved_index += 1;
        }
        return entries.len + 1;
    }
};

fn savedEntryCount(entries: []const workspace_access.Entry) usize {
    var count: usize = 0;
    for (entries) |entry| count += @intFromBool(entry.saved);
    return count;
}

test "workspace menu maps compact rows to existing command actions" {
    const entries = [_]workspace_access.Entry{
        .{
            .path = @constCast("/tmp/saved-a"),
            .saved = true,
            .command_line = false,
            .available = true,
            .active = true,
        },
        .{
            .path = @constCast("/tmp/saved-b"),
            .saved = true,
            .command_line = false,
            .available = true,
            .active = true,
        },
    };
    var state = State{};
    state.open();

    try std.testing.expectEqual(Action.add, state.selectedAction(&entries).?);
    try std.testing.expect(state.move(1, &entries));
    try std.testing.expectEqual(@as(usize, 0), state.selectedAction(&entries).?.remove);
    try std.testing.expect(state.move(1, &entries));
    try std.testing.expectEqual(@as(usize, 1), state.selectedAction(&entries).?.remove);
    try std.testing.expect(state.move(1, &entries));
    try std.testing.expectEqual(Action.clear, state.selectedAction(&entries).?);
    try std.testing.expect(state.move(1, &entries));
    try std.testing.expectEqual(Action.add, state.selectedAction(&entries).?);

    state.open();
    try std.testing.expectEqual(@as(usize, 1), State.actionCount(&.{}));
    try std.testing.expectEqual(Action.add, state.selectedAction(&.{}).?);
}

test "workspace menu shows launch roots without offering invalid remove actions" {
    const entries = [_]workspace_access.Entry{
        .{
            .path = @constCast("/tmp/launch-only"),
            .saved = false,
            .command_line = true,
            .available = true,
            .active = true,
        },
        .{
            .path = @constCast("/tmp/saved"),
            .saved = true,
            .command_line = false,
            .available = true,
            .active = true,
        },
    };
    var state = State{};
    state.open();

    try std.testing.expectEqual(@as(usize, 3), State.actionCount(&entries));
    try std.testing.expectEqual(@as(usize, 4), State.rowCount(&entries));
    try std.testing.expect(state.move(1, &entries));
    try std.testing.expectEqual(@as(usize, 1), state.selectedAction(&entries).?.remove);
    try std.testing.expectEqual(@as(usize, 2), state.selectedRow(&entries).?);
    try std.testing.expect(state.move(1, &entries));
    try std.testing.expectEqual(Action.clear, state.selectedAction(&entries).?);
}
