const std = @import("std");
const approval_amendment = @import("approval_amendment.zig");
const edit_contract = @import("../input/editor_state.zig");
const permission_request = @import("permission_request.zig");
const permissions = @import("permissions.zig");
const types = @import("../shared/types.zig");

const Allocator = std.mem.Allocator;
const ToolPermissionDecision = types.ToolPermissionDecision;

const option_count: u8 = 3;

pub const Move = enum {
    previous,
    next,
};

pub const DraftAction = enum {
    cursor_left,
    cursor_right,
    cursor_home,
    cursor_end,
    cursor_word_left,
    cursor_word_right,
    delete_next,
    delete_word_left,
    delete_whitespace_word_left,
    delete_word_right,
    delete_to_line_start,
    delete_to_line_end,
};

pub const Action = union(enum) {
    deny,
    submit,
    tab,
    backspace,
    number: u8,
    insert_ascii: u8,
    move_choice: Move,
    edit_draft: DraftAction,
};

pub const Event = union(enum) {
    none,
    redraw,
    decision: ToolPermissionDecision,
    limit_exceeded,
};

pub const InsertResult = edit_contract.InsertResult;

pub const State = struct {
    choice_index: u8 = 0,
    confirmation_only: bool = false,
    amendment: approval_amendment.State = .{},

    pub fn deinit(self: *State, alloc: Allocator) void {
        self.reset(alloc, "deinit");
    }

    pub fn reset(self: *State, alloc: Allocator, reason: []const u8) void {
        self.choice_index = 0;
        self.confirmation_only = false;
        self.amendment.discard(alloc, reason);
    }

    pub fn resetAfterSubmission(self: *State, alloc: Allocator) void {
        const selected_choice = self.choice_index;
        self.choice_index = 0;
        self.confirmation_only = false;
        self.amendment.discardAfterSubmission(alloc, selected_choice);
    }

    fn selectedDecision(self: State) ToolPermissionDecision {
        if (self.confirmation_only) {
            return if (self.choice_index == 0) .once else .deny;
        }
        return permissions.permissionDecisionFromIndex(self.choice_index);
    }

    pub fn setConfirmationOnly(self: *State, enabled: bool) void {
        self.confirmation_only = enabled;
        if (enabled and self.choice_index >= 2) self.choice_index = 0;
    }

    pub fn isAmending(self: State) bool {
        return self.amendment.isActive();
    }

    pub fn canAmend(self: State, amendment_allowed: bool) bool {
        return amendment_allowed and
            (self.choice_index == 0 or self.choice_index == 2);
    }

    pub fn amendmentChoice(self: State) ?u8 {
        return self.amendment.activeChoice();
    }

    pub fn draftForChoice(self: State, choice: u8) []const u8 {
        return self.amendment.draftForChoice(choice);
    }

    pub fn draftCursorForChoice(self: State, choice: u8) usize {
        return self.amendment.cursorForChoice(choice);
    }

    pub fn selectedDraft(self: State) []const u8 {
        return self.draftForChoice(self.choice_index);
    }

    pub fn materializeResponse(
        self: *const State,
        alloc: Allocator,
        decision: ToolPermissionDecision,
    ) !permission_request.OwnedPermissionResponse {
        const draft = self.selectedDraft();
        const feedback = if (draft.len > 0)
            try alloc.dupe(u8, draft)
        else
            null;
        return permission_request.OwnedPermissionResponse.init(
            alloc,
            decision,
            feedback,
        );
    }

    pub fn apply(
        self: *State,
        alloc: Allocator,
        action: Action,
        amendment_allowed: ?bool,
        max_len: ?usize,
    ) !Event {
        return switch (action) {
            .deny => blk: {
                self.amendment.discard(alloc, "ctrl_c");
                break :blk .{ .decision = .deny };
            },
            .submit => .{ .decision = self.selectedDecision() },
            .tab => blk: {
                if (self.isAmending()) break :blk .redraw;
                const allowed = amendment_allowed orelse break :blk .none;
                if (self.canAmend(allowed)) {
                    self.amendment.begin(self.choice_index);
                } else {
                    self.moveChoice(.next);
                }
                break :blk .redraw;
            },
            .backspace => blk: {
                if (!self.isAmending()) break :blk .none;
                self.amendment.backspace();
                break :blk .redraw;
            },
            .number => |choice| blk: {
                if (choice >= self.optionCount()) break :blk .none;
                if (self.isAmending()) {
                    break :blk try self.insertAscii(alloc, '1' + choice, max_len);
                }
                self.choice_index = choice;
                break :blk .{ .decision = self.selectedDecision() };
            },
            .insert_ascii => |byte| self.insertAscii(alloc, byte, max_len),
            .move_choice => |direction| blk: {
                self.moveChoice(direction);
                break :blk .redraw;
            },
            .edit_draft => |edit| blk: {
                if (!self.isAmending()) break :blk .none;
                self.applyDraftAction(edit);
                break :blk .redraw;
            },
        };
    }

    pub fn insertAmendmentSlice(
        self: *State,
        alloc: Allocator,
        bytes: []const u8,
        max_len: usize,
    ) !InsertResult {
        if (!self.isAmending()) return .inactive;
        if (bytes.len == 0) return .inserted;
        const choice = self.amendmentChoice().?;
        if (!edit_contract.canInsert(
            self.draftForChoice(choice).len,
            bytes.len,
            max_len,
        )) return .limit_exceeded;
        try self.amendment.insertSlice(alloc, bytes);
        return .inserted;
    }

    fn insertAscii(
        self: *State,
        alloc: Allocator,
        byte: u8,
        max_len: ?usize,
    ) !Event {
        if (!self.isAmending() or byte < 0x20 or byte > 0x7E) return .none;
        if (max_len) |limit| {
            return switch (try self.insertAmendmentSlice(alloc, &.{byte}, limit)) {
                .inserted => .redraw,
                .inactive => .none,
                .limit_exceeded => .limit_exceeded,
            };
        }
        try self.amendment.insert(alloc, byte);
        return .redraw;
    }

    fn moveChoice(self: *State, direction: Move) void {
        const count = self.optionCount();
        const delta: i32 = switch (direction) {
            .previous => -1,
            .next => 1,
        };
        var next = @as(i32, self.choice_index) + delta;
        if (next < 0) next = @as(i32, count) - 1;
        if (next >= @as(i32, count)) next = 0;
        self.choice_index = @intCast(next);
        self.amendment.clearActive();
    }

    fn optionCount(self: State) u8 {
        return if (self.confirmation_only) 2 else option_count;
    }

    fn applyDraftAction(self: *State, action: DraftAction) void {
        switch (action) {
            .cursor_left => self.amendment.cursorLeft(),
            .cursor_right => self.amendment.cursorRight(),
            .cursor_home => self.amendment.cursorHome(),
            .cursor_end => self.amendment.cursorEnd(),
            .cursor_word_left => self.amendment.cursorWordLeft(),
            .cursor_word_right => self.amendment.cursorWordRight(),
            .delete_next => self.amendment.deleteNext(),
            .delete_word_left => self.amendment.deleteWordLeft(),
            .delete_whitespace_word_left => self.amendment.deleteWhitespaceDelimitedWordLeft(),
            .delete_word_right => self.amendment.deleteWordRight(),
            .delete_to_line_start => self.amendment.deleteToLineStart(),
            .delete_to_line_end => self.amendment.deleteToLineEnd(),
        }
    }
};

test "approval decision maps selection and submission actions" {
    const alloc = std.testing.allocator;
    var state = State{};
    defer state.deinit(alloc);

    try std.testing.expectEqual(
        ToolPermissionDecision.once,
        state.selectedDecision(),
    );
    try std.testing.expectEqual(
        Event.redraw,
        try state.apply(alloc, .{ .move_choice = .previous }, true, null),
    );
    try std.testing.expectEqual(@as(u8, 2), state.choice_index);
    try std.testing.expectEqual(
        ToolPermissionDecision.deny,
        (try state.apply(alloc, .submit, true, null)).decision,
    );
    try std.testing.expectEqual(
        ToolPermissionDecision.always,
        (try state.apply(alloc, .{ .number = 1 }, true, null)).decision,
    );
}

test "approval decision ignores tab without an active request" {
    const alloc = std.testing.allocator;
    var state = State{};
    defer state.deinit(alloc);

    try std.testing.expectEqual(
        Event.none,
        try state.apply(alloc, .tab, null, null),
    );
    try std.testing.expectEqual(@as(u8, 0), state.choice_index);
}

test "approval decision preserves independent drafts while choices move" {
    const alloc = std.testing.allocator;
    var state = State{};
    defer state.deinit(alloc);

    try std.testing.expectEqual(
        Event.redraw,
        try state.apply(alloc, .tab, true, null),
    );
    _ = try state.apply(alloc, .{ .insert_ascii = 'y' }, true, null);
    _ = try state.apply(alloc, .{ .move_choice = .previous }, true, null);
    try std.testing.expectEqual(@as(u8, 2), state.choice_index);
    _ = try state.apply(alloc, .tab, true, null);
    _ = try state.apply(alloc, .{ .insert_ascii = 'n' }, true, null);

    try std.testing.expectEqualStrings("y", state.draftForChoice(0));
    try std.testing.expectEqualStrings("n", state.draftForChoice(2));
    try std.testing.expectEqual(@as(?u8, 2), state.amendmentChoice());
}

test "approval decision bounded insertion is atomic" {
    const alloc = std.testing.allocator;
    var state = State{};
    defer state.deinit(alloc);

    _ = try state.apply(alloc, .tab, true, null);
    _ = try state.apply(alloc, .{ .insert_ascii = 'a' }, true, 1);
    try std.testing.expectEqual(
        Event.limit_exceeded,
        try state.apply(alloc, .{ .insert_ascii = 'b' }, true, 1),
    );
    try std.testing.expectEqualStrings("a", state.selectedDraft());
}

test "approval decision inserts only printable ascii actions" {
    const alloc = std.testing.allocator;
    var state = State{};
    defer state.deinit(alloc);

    _ = try state.apply(alloc, .tab, true, null);
    try std.testing.expectEqual(
        Event.none,
        try state.apply(alloc, .{ .insert_ascii = 0x1F }, true, null),
    );
    try std.testing.expectEqual(
        Event.none,
        try state.apply(alloc, .{ .insert_ascii = 0x7F }, true, null),
    );
    try std.testing.expectEqual(
        Event.none,
        try state.apply(alloc, .{ .insert_ascii = 0x80 }, true, null),
    );
    try std.testing.expectEqualStrings("", state.selectedDraft());
}

test "approval decision deny discards amendment state" {
    const alloc = std.testing.allocator;
    var state = State{};
    defer state.deinit(alloc);

    _ = try state.apply(alloc, .tab, true, null);
    _ = try state.apply(alloc, .{ .insert_ascii = 'x' }, true, null);
    try std.testing.expectEqual(
        ToolPermissionDecision.deny,
        (try state.apply(alloc, .deny, true, null)).decision,
    );
    try std.testing.expect(!state.isAmending());
    try std.testing.expectEqualStrings("", state.draftForChoice(0));
}

test "approval decision materializes owned feedback" {
    const alloc = std.testing.allocator;
    var state = State{};
    defer state.deinit(alloc);

    _ = try state.apply(alloc, .tab, true, null);
    _ = try state.apply(alloc, .{ .insert_ascii = 'x' }, true, null);
    var response = try state.materializeResponse(alloc, .once);
    defer response.deinit();

    try std.testing.expectEqual(ToolPermissionDecision.once, response.decision);
    try std.testing.expectEqualStrings("x", response.feedback.?);
    try std.testing.expect(response.feedback.?.ptr != state.selectedDraft().ptr);
}

test "approval decision materializes denial from transition" {
    const alloc = std.testing.allocator;
    var state = State{};
    defer state.deinit(alloc);

    const event = try state.apply(alloc, .deny, true, null);
    var response = try state.materializeResponse(alloc, event.decision);
    defer response.deinit();

    try std.testing.expectEqual(ToolPermissionDecision.deny, response.decision);
    try std.testing.expectEqual(@as(u8, 0), state.choice_index);
}
