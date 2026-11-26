const std = @import("std");
const editor_state = @import("editor_state.zig");
const registered_entities = @import("registered_entities.zig");

const Decision = union(enum) {
    reset,
    move: struct {
        raw_offset: usize,
        preferred_column: usize,
    },
};

fn decideTarget(
    input: []const u8,
    entities: *const registered_entities.State,
    target_raw_offset: ?usize,
    preferred_column: usize,
) Decision {
    const raw_offset = target_raw_offset orelse return .reset;
    if (entities.entityContaining(input, raw_offset) != null) return .reset;
    return .{ .move = .{
        .raw_offset = raw_offset,
        .preferred_column = preferred_column,
    } };
}

/// Owns the sticky display-column intent for composer vertical navigation.
/// Terminal layout adapters provide target offsets and measured columns.
pub const State = struct {
    preferred_column: ?usize = null,

    pub fn reset(self: *State) void {
        self.* = .{};
    }

    pub fn preferredColumn(self: State) ?usize {
        return self.preferred_column;
    }

    pub fn applyTarget(
        self: *State,
        edit: *editor_state.State,
        entities: *const registered_entities.State,
        target_raw_offset: ?usize,
        preferred_column: usize,
    ) bool {
        return self.applyTargetWithSelection(
            edit,
            entities,
            target_raw_offset,
            preferred_column,
            false,
        );
    }

    pub fn applyTargetWithSelection(
        self: *State,
        edit: *editor_state.State,
        entities: *const registered_entities.State,
        target_raw_offset: ?usize,
        preferred_column: usize,
        extend_selection: bool,
    ) bool {
        switch (decideTarget(
            edit.input.items,
            entities,
            target_raw_offset,
            preferred_column,
        )) {
            .reset => {
                self.reset();
                return false;
            },
            .move => |target| {
                _ = edit.moveCursorTo(target.raw_offset, extend_selection);
                self.preferred_column = target.preferred_column;
                return true;
            },
        }
    }
};

test "vertical navigation applies a legal target and clears selection" {
    const alloc = std.testing.allocator;
    var edit: editor_state.State = .{};
    defer edit.deinit(alloc);
    try edit.setText(alloc, "abcdef");
    _ = edit.beginSelection(1);
    _ = edit.extendSelection(4);

    var entities: registered_entities.State = .{};
    defer entities.deinit(alloc);
    var state: State = .{};

    try std.testing.expect(state.applyTarget(&edit, &entities, 5, 3));
    try std.testing.expectEqual(@as(usize, 5), edit.cursor);
    try std.testing.expect(edit.selectionRange() == null);
    try std.testing.expectEqual(@as(?usize, 3), state.preferredColumn());
}

test "vertical navigation reset decisions preserve the editor" {
    const alloc = std.testing.allocator;
    var edit: editor_state.State = .{};
    defer edit.deinit(alloc);
    try edit.setText(alloc, "abcdef");
    _ = edit.beginSelection(1);
    _ = edit.extendSelection(4);

    var entities: registered_entities.State = .{};
    defer entities.deinit(alloc);
    var state: State = .{ .preferred_column = 4 };

    try std.testing.expect(!state.applyTarget(&edit, &entities, null, 0));
    try std.testing.expectEqual(@as(usize, 4), edit.cursor);
    try std.testing.expectEqual(
        editor_state.SelectionRange{ .start = 1, .end = 4 },
        edit.selectionRange().?,
    );
    try std.testing.expectEqual(@as(?usize, null), state.preferredColumn());
}

test "vertical navigation rejects targets inside registered entities" {
    const alloc = std.testing.allocator;
    var edit: editor_state.State = .{};
    defer edit.deinit(alloc);
    try edit.setText(alloc, "[Image #1]");
    _ = edit.setCursor(0);

    var entities: registered_entities.State = .{};
    defer entities.deinit(alloc);
    try entities.image_tokens.append(alloc, .{
        .id = 1,
        .span = .{ .raw_start = 0, .raw_end = edit.input.items.len },
    });
    var state: State = .{ .preferred_column = 2 };

    try std.testing.expect(!state.applyTarget(&edit, &entities, 3, 3));
    try std.testing.expectEqual(@as(usize, 0), edit.cursor);
    try std.testing.expectEqual(@as(?usize, null), state.preferredColumn());
}

test "vertical navigation extends from the original cursor when requested" {
    const alloc = std.testing.allocator;
    var edit: editor_state.State = .{};
    defer edit.deinit(alloc);
    try edit.setText(alloc, "abcdef");
    _ = edit.setCursor(2);

    var entities: registered_entities.State = .{};
    defer entities.deinit(alloc);
    var state: State = .{};

    try std.testing.expect(state.applyTargetWithSelection(&edit, &entities, 5, 3, true));
    try std.testing.expectEqual(
        editor_state.SelectionRange{ .start = 2, .end = 5 },
        edit.selectionRange().?,
    );
    try std.testing.expectEqual(@as(?usize, 3), state.preferredColumn());
}
