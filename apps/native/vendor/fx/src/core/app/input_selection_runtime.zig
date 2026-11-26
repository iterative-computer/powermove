const std = @import("std");
const host = @import("../hosts/host.zig");
const composer_selection = @import("../input/composer_selection.zig");
const composer_replacement = @import("../input/composer_replacement.zig");
const composer_undo = @import("../input/composer_undo.zig");
const edit_history = @import("../input/edit_history.zig");
const editor_state = @import("../input/editor_state.zig");
const input_limit_rejection = @import("../input/input_limit_rejection.zig");
const picker_state = @import("../input/picker_state.zig");
const registered_entities = @import("../input/registered_entities.zig");
const vertical_navigation = @import("../input/vertical_navigation.zig");

pub const ClipboardResult = enum {
    inactive,
    copied,
    cut,
    copy_failed,
    delete_failed,
};

pub fn copySelection(
    edit: *const editor_state.State,
    clipboard: host.Clipboard,
) ClipboardResult {
    const selected = edit.selectedText() orelse return .inactive;
    if (!(clipboard.copy(selected) catch false)) return .copy_failed;
    return .copied;
}

pub fn cutSelection(
    alloc: std.mem.Allocator,
    selection: composer_selection.State,
    pending_images: ?*composer_replacement.ImageBlocks,
    clipboard: host.Clipboard,
) ClipboardResult {
    const selected = selection.selectedText() orelse return .inactive;
    if (!(clipboard.copy(selected) catch false)) return .copy_failed;
    if (!selection.delete(alloc, pending_images)) return .delete_failed;
    return .cut;
}

const TestInput = struct {
    edit: editor_state.State = .{},
    history: edit_history.State = .{},
    picker: picker_state.State = .{},
    entities: registered_entities.State = .{},
    vertical_navigation: vertical_navigation.State = .{},
    input_limit_rejection: input_limit_rejection.State = .{},

    fn deinit(self: *TestInput, alloc: std.mem.Allocator) void {
        self.edit.deinit(alloc);
        self.history.deinit(alloc);
        self.picker.deinit(alloc);
        self.entities.deinit(alloc);
    }

    fn selection(self: *TestInput) composer_selection.State {
        return .{ .insertion = .{
            .edit = &self.edit,
            .history = &self.history,
            .picker = &self.picker,
            .entities = &self.entities,
            .vertical_navigation = &self.vertical_navigation,
            .input_limit_rejection = &self.input_limit_rejection,
        } };
    }

    fn undo(self: *TestInput) composer_undo.State {
        return .{
            .edit = &self.edit,
            .history = &self.history,
            .picker = &self.picker,
            .entities = &self.entities,
            .vertical_navigation = &self.vertical_navigation,
            .input_limit_rejection = &self.input_limit_rejection,
        };
    }
};

const TestClipboard = struct {
    copied: std.ArrayList(u8) = .empty,
    succeed: bool = true,

    fn deinit(self: *TestClipboard, alloc: std.mem.Allocator) void {
        self.copied.deinit(alloc);
    }

    fn clipboard(self: *TestClipboard) host.Clipboard {
        return .{ .context = self, .copy_fn = copy };
    }

    fn copy(raw_context: ?*anyopaque, bytes: []const u8) host.ClipboardError!bool {
        const self: *TestClipboard = @ptrCast(@alignCast(raw_context.?));
        if (!self.succeed) return false;
        self.copied.appendSlice(std.testing.allocator, bytes) catch return error.CopyFailed;
        return true;
    }
};

test "selection clipboard operations copy before cut" {
    const alloc = std.testing.allocator;
    var input: TestInput = .{};
    defer input.deinit(alloc);
    try input.edit.setText(alloc, "alpha beta");
    _ = input.selection().begin(0);
    _ = input.selection().extend("alpha".len);

    var test_clipboard: TestClipboard = .{};
    defer test_clipboard.deinit(alloc);
    try std.testing.expectEqual(
        ClipboardResult.copied,
        copySelection(&input.edit, test_clipboard.clipboard()),
    );
    try std.testing.expectEqualStrings("alpha", test_clipboard.copied.items);
    try std.testing.expectEqualStrings("alpha beta", input.edit.input.items);

    test_clipboard.copied.clearRetainingCapacity();
    try std.testing.expectEqual(
        ClipboardResult.cut,
        cutSelection(alloc, input.selection(), null, test_clipboard.clipboard()),
    );
    try std.testing.expectEqualStrings("alpha", test_clipboard.copied.items);
    try std.testing.expectEqualStrings(" beta", input.edit.input.items);
    try std.testing.expect(try input.undo().undo(alloc));
    try std.testing.expectEqualStrings("alpha beta", input.edit.input.items);
}

test "failed clipboard copy preserves selection and text" {
    const alloc = std.testing.allocator;
    var input: TestInput = .{};
    defer input.deinit(alloc);
    try input.edit.setText(alloc, "alpha beta");
    _ = input.selection().begin(0);
    _ = input.selection().extend("alpha".len);

    var test_clipboard: TestClipboard = .{ .succeed = false };
    defer test_clipboard.deinit(alloc);
    try std.testing.expectEqual(
        ClipboardResult.copy_failed,
        cutSelection(alloc, input.selection(), null, test_clipboard.clipboard()),
    );
    try std.testing.expectEqualStrings("alpha beta", input.edit.input.items);
    try std.testing.expectEqualStrings("alpha", input.selection().selectedText().?);
}
