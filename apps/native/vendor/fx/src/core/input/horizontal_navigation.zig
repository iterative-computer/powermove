const std = @import("std");
const display_width = @import("../shared/display_width.zig");
const editor_state = @import("editor_state.zig");
const input_action = @import("input_action.zig");
const registered_entities = @import("registered_entities.zig");
const text_boundaries = @import("text_boundaries.zig");
const vertical_navigation = @import("vertical_navigation.zig");

pub fn move(
    motion: input_action.MoveKind,
    edit: *editor_state.State,
    entities: *const registered_entities.State,
    vertical: *vertical_navigation.State,
) bool {
    return moveIntent(.{ .kind = motion }, edit, entities, vertical);
}

/// Applies one layout-independent motion intent to Core-owned input state.
pub fn moveIntent(
    intent: input_action.MoveIntent,
    edit: *editor_state.State,
    entities: *const registered_entities.State,
    vertical: *vertical_navigation.State,
) bool {
    vertical.reset();
    return switch (intent.kind) {
        .character_left => moveCharacterLeft(edit, entities, intent.extend_selection),
        .character_right => moveCharacterRight(edit, entities, intent.extend_selection),
        .draft_start => edit.moveCursorTo(0, intent.extend_selection),
        .draft_end => edit.moveCursorTo(edit.input.items.len, intent.extend_selection),
        .line_start => edit.moveCursorTo(
            text_boundaries.logicalLineStart(edit.input.items, edit.cursor),
            intent.extend_selection,
        ),
        .line_end => edit.moveCursorTo(
            text_boundaries.logicalLineEnd(edit.input.items, edit.cursor),
            intent.extend_selection,
        ),
        .paragraph_up => edit.moveCursorTo(
            text_boundaries.previousParagraphStart(edit.input.items, edit.cursor),
            intent.extend_selection,
        ),
        .paragraph_down => edit.moveCursorTo(
            text_boundaries.nextParagraphStart(edit.input.items, edit.cursor),
            intent.extend_selection,
        ),
        .word_left => moveWordLeft(edit, entities, intent.extend_selection),
        .word_right => moveWordRight(edit, entities, intent.extend_selection),
        .visual_up, .visual_down, .page_up, .page_down => false,
    };
}

fn moveCharacterLeft(
    edit: *editor_state.State,
    entities: *const registered_entities.State,
    extend_selection: bool,
) bool {
    if (!extend_selection and edit.collapseSelection(.start)) return true;
    if (edit.cursor == 0) return false;
    if (entities.entityEndingAt(edit.input.items, edit.cursor)) |entity| {
        return edit.moveCursorTo(entity.span.raw_start, extend_selection);
    }
    return edit.moveCursorTo(
        text_boundaries.previousCharacterStart(edit.input.items, edit.cursor),
        extend_selection,
    );
}

fn moveCharacterRight(
    edit: *editor_state.State,
    entities: *const registered_entities.State,
    extend_selection: bool,
) bool {
    if (!extend_selection and edit.collapseSelection(.end)) return true;
    if (edit.cursor >= edit.input.items.len) return false;
    if (entities.entityStartingAt(edit.input.items, edit.cursor)) |entity| {
        return edit.moveCursorTo(entity.span.raw_end, extend_selection);
    }
    return edit.moveCursorTo(
        text_boundaries.nextCharacterEnd(edit.input.items, edit.cursor),
        extend_selection,
    );
}

fn moveWordLeft(
    edit: *editor_state.State,
    entities: *const registered_entities.State,
    extend_selection: bool,
) bool {
    if (!extend_selection and edit.collapseSelection(.start)) return true;
    if (edit.cursor == 0) return false;

    const input = edit.input.items;
    var raw_offset = edit.cursor;
    while (raw_offset > 0) {
        if (entities.entityEndingAt(input, raw_offset)) |entity| {
            return edit.moveCursorTo(entity.span.raw_start, extend_selection);
        }
        const previous = text_boundaries.previousCharacterStart(input, raw_offset);
        const rune = display_width.decodeNextRune(input, previous);
        if (text_boundaries.isWordCharacter(rune.codepoint)) break;
        raw_offset = previous;
    }
    while (raw_offset > 0) {
        if (entities.entityEndingAt(input, raw_offset)) |entity| {
            return edit.moveCursorTo(entity.span.raw_start, extend_selection);
        }
        const previous = text_boundaries.previousCharacterStart(input, raw_offset);
        const rune = display_width.decodeNextRune(input, previous);
        if (!text_boundaries.isWordCharacter(rune.codepoint)) break;
        raw_offset = previous;
    }
    return edit.moveCursorTo(raw_offset, extend_selection);
}

fn moveWordRight(
    edit: *editor_state.State,
    entities: *const registered_entities.State,
    extend_selection: bool,
) bool {
    if (!extend_selection and edit.collapseSelection(.end)) return true;
    const input = edit.input.items;
    if (edit.cursor >= input.len) return false;

    var raw_offset = edit.cursor;
    while (raw_offset < input.len) {
        if (entities.entityStartingAt(input, raw_offset)) |entity| {
            return edit.moveCursorTo(entity.span.raw_end, extend_selection);
        }
        const rune = display_width.decodeNextRune(input, raw_offset);
        if (text_boundaries.isWordCharacter(rune.codepoint)) break;
        raw_offset = text_boundaries.nextCharacterEnd(input, raw_offset);
    }
    while (raw_offset < input.len) {
        if (entities.entityStartingAt(input, raw_offset) != null) break;
        const rune = display_width.decodeNextRune(input, raw_offset);
        if (!text_boundaries.isWordCharacter(rune.codepoint)) break;
        raw_offset = text_boundaries.nextCharacterEnd(input, raw_offset);
    }
    return edit.moveCursorTo(raw_offset, extend_selection);
}

test "horizontal character motion collapses selections at the matching edge" {
    const alloc = std.testing.allocator;
    var edit: editor_state.State = .{};
    defer edit.deinit(alloc);
    try edit.setText(alloc, "abcdef");

    var entities: registered_entities.State = .{};
    defer entities.deinit(alloc);
    var vertical: vertical_navigation.State = .{ .preferred_column = 4 };

    _ = edit.beginSelection(2);
    _ = edit.extendSelection(5);
    try std.testing.expect(moveIntent(.{ .kind = .character_left }, &edit, &entities, &vertical));
    try std.testing.expectEqual(@as(usize, 2), edit.cursor);
    try std.testing.expect(edit.selectionRange() == null);
    try std.testing.expectEqual(@as(?usize, null), vertical.preferredColumn());

    _ = edit.beginSelection(2);
    _ = edit.extendSelection(5);
    try std.testing.expect(moveIntent(.{ .kind = .character_right }, &edit, &entities, &vertical));
    try std.testing.expectEqual(@as(usize, 5), edit.cursor);
    try std.testing.expect(edit.selectionRange() == null);
}

test "horizontal character motion preserves display units and registered entities" {
    const alloc = std.testing.allocator;
    var edit: editor_state.State = .{};
    defer edit.deinit(alloc);
    try edit.setText(alloc, "Ae\u{301}[Image #2]B");

    var entities: registered_entities.State = .{};
    defer entities.deinit(alloc);
    try entities.image_tokens.append(alloc, .{
        .id = 2,
        .span = .{ .raw_start = "Ae\u{301}".len, .raw_end = "Ae\u{301}[Image #2]".len },
    });
    var vertical: vertical_navigation.State = .{};

    _ = edit.setCursor(edit.input.items.len);
    try std.testing.expect(moveIntent(.{ .kind = .character_left }, &edit, &entities, &vertical));
    try std.testing.expectEqual(@as(usize, "Ae\u{301}[Image #2]".len), edit.cursor);
    try std.testing.expect(moveIntent(.{ .kind = .character_left }, &edit, &entities, &vertical));
    try std.testing.expectEqual(@as(usize, "Ae\u{301}".len), edit.cursor);
    try std.testing.expect(moveIntent(.{ .kind = .character_left }, &edit, &entities, &vertical));
    try std.testing.expectEqual(@as(usize, "A".len), edit.cursor);
}

test "horizontal word motion preserves punctuation unicode and entity boundaries" {
    const alloc = std.testing.allocator;
    var edit: editor_state.State = .{};
    defer edit.deinit(alloc);
    try edit.setText(alloc, "h\u{e9}llo.a_b [Image #1] next");

    var entities: registered_entities.State = .{};
    defer entities.deinit(alloc);
    try entities.image_tokens.append(alloc, .{
        .id = 1,
        .span = .{
            .raw_start = "h\u{e9}llo.a_b ".len,
            .raw_end = "h\u{e9}llo.a_b [Image #1]".len,
        },
    });
    var vertical: vertical_navigation.State = .{};

    _ = edit.setCursor(0);
    try std.testing.expect(moveIntent(.{ .kind = .word_right }, &edit, &entities, &vertical));
    try std.testing.expectEqual(@as(usize, "h\u{e9}llo".len), edit.cursor);
    try std.testing.expect(moveIntent(.{ .kind = .word_right }, &edit, &entities, &vertical));
    try std.testing.expectEqual(@as(usize, "h\u{e9}llo.a_b".len), edit.cursor);
    try std.testing.expect(moveIntent(.{ .kind = .word_right }, &edit, &entities, &vertical));
    try std.testing.expectEqual(@as(usize, "h\u{e9}llo.a_b [Image #1]".len), edit.cursor);
    try std.testing.expect(moveIntent(.{ .kind = .word_left }, &edit, &entities, &vertical));
    try std.testing.expectEqual(@as(usize, "h\u{e9}llo.a_b ".len), edit.cursor);
}

test "horizontal line and input motions keep distinct boundaries" {
    const alloc = std.testing.allocator;
    var edit: editor_state.State = .{};
    defer edit.deinit(alloc);
    try edit.setText(alloc, "FIRST\nSECOND\nTHIRD");

    var entities: registered_entities.State = .{};
    defer entities.deinit(alloc);
    var vertical: vertical_navigation.State = .{ .preferred_column = 3 };

    _ = edit.setCursor("FIRST\nSEC".len);
    try std.testing.expect(moveIntent(.{ .kind = .line_start }, &edit, &entities, &vertical));
    try std.testing.expectEqual(@as(usize, "FIRST\n".len), edit.cursor);
    try std.testing.expect(moveIntent(.{ .kind = .line_end }, &edit, &entities, &vertical));
    try std.testing.expectEqual(@as(usize, "FIRST\nSECOND".len), edit.cursor);
    try std.testing.expect(moveIntent(.{ .kind = .draft_start }, &edit, &entities, &vertical));
    try std.testing.expectEqual(@as(usize, 0), edit.cursor);
    try std.testing.expect(moveIntent(.{ .kind = .draft_end }, &edit, &entities, &vertical));
    try std.testing.expectEqual(edit.input.items.len, edit.cursor);

    vertical.preferred_column = 7;
    try std.testing.expect(!moveIntent(.{ .kind = .character_right }, &edit, &entities, &vertical));
    try std.testing.expectEqual(@as(?usize, null), vertical.preferredColumn());
}
