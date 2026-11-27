const std = @import("std");
const composer_insertion = @import("composer_insertion.zig");
const composer_replacement = @import("composer_replacement.zig");
const edit_history = @import("edit_history.zig");
const editor_state = @import("editor_state.zig");
const input_limit_rejection = @import("input_limit_rejection.zig");
const picker_state = @import("picker_state.zig");
const registered_entities = @import("registered_entities.zig");
const vertical_navigation = @import("vertical_navigation.zig");

const Allocator = std.mem.Allocator;

/// Borrows the Core state changed by composer selection transitions.
/// The containing runtime retains ownership of every pointer and allocation.
pub const State = struct {
    insertion: composer_insertion.State,

    pub fn begin(self: State, raw_offset: usize) bool {
        const changed = self.insertion.edit.beginSelection(raw_offset);
        self.insertion.vertical_navigation.reset();
        return changed;
    }

    pub fn extend(self: State, raw_offset: usize) bool {
        const changed = self.insertion.edit.extendSelection(raw_offset);
        if (!changed) return false;
        self.insertion.vertical_navigation.reset();
        return true;
    }

    pub fn finish(self: State) void {
        self.insertion.edit.finishSelection();
    }

    pub fn selectedText(self: State) ?[]const u8 {
        return self.insertion.edit.selectedText();
    }

    pub fn selectAll(self: State) bool {
        self.insertion.vertical_navigation.reset();
        return self.insertion.edit.selectAll();
    }

    pub fn delete(
        self: State,
        alloc: Allocator,
        images: ?*composer_replacement.ImageBlocks,
    ) bool {
        return (composer_replacement.State{
            .insertion = self.insertion,
            .images = images,
        }).deleteSelection(alloc);
    }
};

const Fixture = struct {
    edit: editor_state.State = .{},
    history: edit_history.State = .{},
    picker: picker_state.State = .{},
    entities: registered_entities.State = .{},
    vertical_navigation: vertical_navigation.State = .{},
    input_limit_rejection: input_limit_rejection.State = .{},

    fn deinit(self: *Fixture, alloc: Allocator) void {
        self.edit.deinit(alloc);
        self.history.deinit(alloc);
        self.picker.deinit(alloc);
        self.entities.deinit(alloc);
    }

    fn state(self: *Fixture) State {
        return .{ .insertion = .{
            .edit = &self.edit,
            .history = &self.history,
            .picker = &self.picker,
            .entities = &self.entities,
            .vertical_navigation = &self.vertical_navigation,
            .input_limit_rejection = &self.input_limit_rejection,
        } };
    }
};

test "composer selection owns pointer transitions and vertical intent reset" {
    const alloc = std.testing.allocator;
    var fixture: Fixture = .{};
    defer fixture.deinit(alloc);
    try fixture.edit.setText(alloc, "alpha beta");

    fixture.vertical_navigation.preferred_column = 4;
    try std.testing.expect(fixture.state().begin(2));
    try std.testing.expectEqual(@as(?usize, null), fixture.vertical_navigation.preferredColumn());

    fixture.vertical_navigation.preferred_column = 5;
    try std.testing.expect(fixture.state().extend(std.math.maxInt(usize)));
    try std.testing.expectEqual(@as(?usize, null), fixture.vertical_navigation.preferredColumn());
    try std.testing.expectEqual(
        editor_state.SelectionRange{ .start = 2, .end = "alpha beta".len },
        fixture.edit.selectionRange().?,
    );
    try std.testing.expectEqualStrings("pha beta", fixture.state().selectedText().?);

    try std.testing.expect(fixture.state().extend(2));
    fixture.state().finish();
    try std.testing.expect(fixture.edit.selectionRange() == null);
}

test "composer select all clamps to the draft and clears vertical intent" {
    const alloc = std.testing.allocator;
    var fixture: Fixture = .{};
    defer fixture.deinit(alloc);
    try fixture.edit.setText(alloc, "alpha beta");

    fixture.vertical_navigation.preferred_column = 3;
    try std.testing.expect(fixture.state().selectAll());
    try std.testing.expectEqual(
        editor_state.SelectionRange{ .start = 0, .end = "alpha beta".len },
        fixture.edit.selectionRange().?,
    );
    try std.testing.expectEqual(@as(?usize, null), fixture.vertical_navigation.preferredColumn());
}
