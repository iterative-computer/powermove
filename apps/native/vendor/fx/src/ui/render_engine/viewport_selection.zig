const std = @import("std");
const build_checkpoint = @import("build_checkpoint.zig");
const command_output_content = @import("../../core/tooling/command_output_content.zig");
const transcript_blocks = @import("transcript_blocks.zig");

const Allocator = std.mem.Allocator;
const TranscriptBlockKind = transcript_blocks.TranscriptBlockKind;
const visualRowsForLine = transcript_blocks.visualRowsForLine;

pub const LineRef = struct {
    start: u32,
    len: u32,

    pub fn resolve(self: LineRef, buf: []const u8) []const u8 {
        const start: usize = self.start;
        const len: usize = self.len;
        const end = start + len;
        std.debug.assert(end <= buf.len);
        return buf[start..end];
    }
};

pub const HardLineStarts = struct {
    starts: []usize,
    ends_with_newline: bool,

    pub fn deinit(self: HardLineStarts, alloc: Allocator) void {
        alloc.free(self.starts);
    }

    pub fn len(self: HardLineStarts) usize {
        return self.starts.len;
    }
};

pub const TranscriptBuffer = struct {
    bytes: []const u8,
};

pub const TranscriptRef = struct {
    ref: LineRef,

    pub fn resolve(self: TranscriptRef, buf: TranscriptBuffer) []const u8 {
        return self.ref.resolve(buf.bytes);
    }
};

pub const VisibleTranscriptLine = union(enum) {
    transcript: struct {
        ref: TranscriptRef,
    },
    folded_line: struct {
        text: []const u8,
        stream: command_output_content.Stream,
    },
};

pub const VisibleTranscriptBatch = struct {
    transcript: TranscriptBuffer,
    lines: []const VisibleTranscriptLine,

    pub fn resolve(self: VisibleTranscriptBatch, line: VisibleTranscriptLine) []const u8 {
        return switch (line) {
            .transcript => |t| t.ref.resolve(self.transcript),
            .folded_line => |f| f.text,
        };
    }
};

pub const ViewportSelection = struct {
    top_row: u16,
    bottom_row: u16,
    start_line: usize,
    partial_skip_rows: u16,
    line_count: usize,
    last_visible_row: u16 = 0,
    last_visible_row_blank: bool = false,
    tail_kind: ?TranscriptBlockKind = null,
    replaceable_start_row: u16 = 0,
    split_active: bool = false,
    split_prefix_lines: usize = 0,
    split_suffix_start_line: usize = 0,
};

pub const WelcomeDecision = enum {
    none,
    pinned_welcome,
    dropped_partial_welcome,
};

pub const BuildInput = struct {
    top_row: u16,
    bottom_row: u16,
    line_visual_rows: []const u16,
    visible_rows: u16,
    welcome_cut_line: ?usize = null,
    replaceable_line_index: usize,
    tail_kind: ?TranscriptBlockKind = null,
};

pub const BuildResult = struct {
    selection: ViewportSelection,
    rows_budget_unused: u16,
    replaceable_line_index: usize,
    welcome_decision: WelcomeDecision,
    welcome_cut_line: ?usize,
    natural_start_line: usize,
    original_total_lines: usize,
};

pub const TailOffsetRequest = struct {
    total_rows: u32,
    visible_rows: u16,
    policy: TailViewportPolicy,
};

pub const TailViewportPolicy = struct {
    rows_from_bottom: u32,
    prior_total_rows: ?u32 = null,
    preserve_after_append: bool = false,
};

pub const TailOffsetResolution = struct {
    start_visual_row: u32,
    rows_from_bottom: u32,
    max_rows_from_bottom: u32,
    total_rows: u32,
};

/// Resolve a viewport offset without retaining transcript content. A pinned
/// tail follows appended output; an unpinned viewport may preserve its visual
/// row by increasing its distance from the new tail.
pub fn resolveTailOffset(request: TailOffsetRequest) TailOffsetResolution {
    const max_rows_from_bottom = request.total_rows -| request.visible_rows;
    var rows_from_bottom = @min(
        request.policy.rows_from_bottom,
        max_rows_from_bottom,
    );
    if (request.policy.preserve_after_append and rows_from_bottom > 0) {
        if (request.policy.prior_total_rows) |prior_total_rows| {
            rows_from_bottom = @min(
                rows_from_bottom +| (request.total_rows -| prior_total_rows),
                max_rows_from_bottom,
            );
        }
    }
    return .{
        .start_visual_row = max_rows_from_bottom - rows_from_bottom,
        .rows_from_bottom = rows_from_bottom,
        .max_rows_from_bottom = max_rows_from_bottom,
        .total_rows = request.total_rows,
    };
}

test "tail viewport preserves an unpinned row across appended output" {
    const initial = resolveTailOffset(.{
        .total_rows = 40,
        .visible_rows = 10,
        .policy = .{ .rows_from_bottom = 8 },
    });
    try std.testing.expectEqual(@as(u32, 22), initial.start_visual_row);
    try std.testing.expectEqual(@as(u32, 8), initial.rows_from_bottom);
    try std.testing.expectEqual(@as(u32, 30), initial.max_rows_from_bottom);

    const grown = resolveTailOffset(.{
        .total_rows = 47,
        .visible_rows = 10,
        .policy = .{
            .rows_from_bottom = 8,
            .prior_total_rows = 40,
            .preserve_after_append = true,
        },
    });
    try std.testing.expectEqual(initial.start_visual_row, grown.start_visual_row);
    try std.testing.expectEqual(@as(u32, 15), grown.rows_from_bottom);

    const pinned = resolveTailOffset(.{
        .total_rows = 47,
        .visible_rows = 10,
        .policy = .{
            .rows_from_bottom = 0,
            .prior_total_rows = 40,
            .preserve_after_append = true,
        },
    });
    try std.testing.expectEqual(@as(u32, 37), pinned.start_visual_row);
    try std.testing.expectEqual(@as(u32, 0), pinned.rows_from_bottom);
}

pub const RowForLineInput = struct {
    selection: ViewportSelection,
    bytes: []const u8,
    line_index: usize,
    cols: u16,
    max_row: u16,
};

const TailSelection = struct {
    start_line: usize,
    partial_skip_rows: u16,
    rows_budget_unused: u16,
};

const SplitLineMapping = struct {
    prefix_lines: usize,
    suffix_start_line: usize,

    fn fromSelection(selection: ViewportSelection) SplitLineMapping {
        return .{
            .prefix_lines = selection.split_prefix_lines,
            .suffix_start_line = selection.split_suffix_start_line,
        };
    }

    fn sourceLineForSelected(self: SplitLineMapping, selected_index: usize) ?usize {
        if (selected_index < self.prefix_lines) return selected_index;

        const suffix_offset = selected_index - self.prefix_lines;
        const source_index = self.suffix_start_line + suffix_offset;
        if (source_index < self.suffix_start_line) return null;
        return source_index;
    }

    fn selectedIndexForSource(self: SplitLineMapping, source_line_index: usize) ?usize {
        if (source_line_index < self.prefix_lines) return source_line_index;
        if (source_line_index < self.suffix_start_line) return null;
        return self.prefix_lines + (source_line_index - self.suffix_start_line);
    }
};

pub fn visibleFromTranscript(ref: LineRef) VisibleTranscriptLine {
    return .{ .transcript = .{ .ref = .{ .ref = ref } } };
}

pub fn buildHardLineStarts(alloc: Allocator, bytes: []const u8) !HardLineStarts {
    return buildHardLineStartsInterruptible(alloc, bytes, null) catch |err| switch (err) {
        error.InputPending => unreachable,
        else => |other| return other,
    };
}

pub fn buildHardLineStartsInterruptible(
    alloc: Allocator,
    bytes: []const u8,
    checkpoint: ?*build_checkpoint.BuildCheckpoint,
) !HardLineStarts {
    var starts: std.ArrayList(usize) = .empty;
    errdefer starts.deinit(alloc);

    if (bytes.len == 0) {
        return .{
            .starts = try starts.toOwnedSlice(alloc),
            .ends_with_newline = false,
        };
    }

    try starts.append(alloc, 0);
    for (bytes, 0..) |byte, i| {
        try build_checkpoint.tick(checkpoint);
        if (byte == '\n' and i + 1 < bytes.len) {
            try starts.append(alloc, i + 1);
        }
    }

    return .{
        .starts = try starts.toOwnedSlice(alloc),
        .ends_with_newline = bytes[bytes.len - 1] == '\n',
    };
}

pub fn hardLineRefAt(lines: HardLineStarts, total_len: usize, index: usize) LineRef {
    const start = lines.starts[index];
    const end = if (index + 1 < lines.starts.len)
        lines.starts[index + 1] - 1
    else if (lines.ends_with_newline)
        total_len - 1
    else
        total_len;
    return .{ .start = @intCast(start), .len = @intCast(end - start) };
}

pub fn hardLineIndexAtWithPolicy(bytes: []const u8, offset: usize) usize {
    var line: usize = 0;
    var i: usize = 0;
    const end = @min(offset, bytes.len);
    while (i < end) : (i += 1) {
        if (bytes[i] == '\n' and i + 1 < bytes.len) line += 1;
    }
    return line;
}

pub fn transcriptLineAt(starts: []const usize, total_len: usize, index: usize) LineRef {
    const start = starts[index];
    const end = if (index + 1 < starts.len) starts[index + 1] - 1 else total_len;
    return .{ .start = @intCast(start), .len = @intCast(end - start) };
}

pub fn hardLineIndexAt(bytes: []const u8, offset: usize) usize {
    return hardLineIndexAtWithPolicy(bytes, offset);
}

pub fn hardLineAt(bytes: []const u8, index: usize) []const u8 {
    var line: usize = 0;
    var start: usize = 0;
    var i: usize = 0;
    while (i < bytes.len) : (i += 1) {
        if (bytes[i] != '\n') continue;
        if (line == index) return bytes[start..i];
        if (i + 1 < bytes.len) {
            line += 1;
            start = i + 1;
        }
    }
    if (line == index and start < bytes.len) return bytes[start..];
    if (line == index and bytes.len > 0 and bytes[bytes.len - 1] == '\n') return bytes[start .. bytes.len - 1];
    return "";
}

pub fn sumVisualRows(line_visual_rows: []const u16, start_line: usize, end_line: usize) u16 {
    var total: u32 = 0;
    var line = start_line;
    while (line < end_line) : (line += 1) {
        total += line_visual_rows[line];
    }
    return @intCast(@min(total, @as(u32, std.math.maxInt(u16))));
}

pub fn suffixStartForRowsBudget(line_visual_rows: []const u16, min_start: usize, total_lines: usize, rows_budget: u16) usize {
    var remaining = rows_budget;
    var start = total_lines;
    while (start > min_start) {
        const rows = line_visual_rows[start - 1];
        if (rows > remaining) break;
        remaining -= rows;
        start -= 1;
    }
    return start;
}

pub fn buildViewportSelection(input: BuildInput) BuildResult {
    const total_lines = input.line_visual_rows.len;
    const tail = selectTail(input.line_visual_rows, input.visible_rows);
    var result: BuildResult = .{
        .selection = .{
            .top_row = input.top_row,
            .bottom_row = input.bottom_row,
            .start_line = tail.start_line,
            .partial_skip_rows = tail.partial_skip_rows,
            .line_count = total_lines,
            .tail_kind = input.tail_kind,
            .replaceable_start_row = input.top_row,
        },
        .rows_budget_unused = tail.rows_budget_unused,
        .replaceable_line_index = input.replaceable_line_index,
        .welcome_decision = .none,
        .welcome_cut_line = input.welcome_cut_line,
        .natural_start_line = tail.start_line,
        .original_total_lines = total_lines,
    };
    applyWelcomeCut(input, &result);
    return result;
}

fn applyWelcomeCut(
    input: BuildInput,
    result: *BuildResult,
) void {
    const welcome_cut_line = input.welcome_cut_line orelse return;
    const total_lines = input.line_visual_rows.len;
    if (result.selection.start_line == 0 or welcome_cut_line >= total_lines) return;

    const welcome_rows = sumVisualRows(input.line_visual_rows, 0, welcome_cut_line);
    const suffix_start = if (welcome_rows < input.visible_rows)
        suffixStartForRowsBudget(
            input.line_visual_rows,
            welcome_cut_line,
            total_lines,
            input.visible_rows - welcome_rows,
        )
    else
        total_lines;
    if (suffix_start >= welcome_cut_line and suffix_start < total_lines) {
        const selected_line_count = welcome_cut_line + (total_lines - suffix_start);
        result.selection.start_line = 0;
        result.selection.partial_skip_rows = 0;
        result.selection.line_count = selected_line_count;
        result.selection.split_active = true;
        result.selection.split_prefix_lines = welcome_cut_line;
        result.selection.split_suffix_start_line = suffix_start;
        result.replaceable_line_index = remapReplaceableLineIndex(
            result.replaceable_line_index,
            welcome_cut_line,
            suffix_start,
            selected_line_count,
        );
        result.rows_budget_unused = unusedRowsBudgetForSplit(
            input.line_visual_rows,
            welcome_cut_line,
            suffix_start,
            total_lines,
            input.visible_rows,
        );
        result.welcome_decision = .pinned_welcome;
        return;
    }

    const suffix_tail = selectTail(input.line_visual_rows[welcome_cut_line..], input.visible_rows);
    result.selection.start_line = welcome_cut_line + suffix_tail.start_line;
    result.selection.partial_skip_rows = suffix_tail.partial_skip_rows;
    result.rows_budget_unused = suffix_tail.rows_budget_unused;
    result.welcome_decision = .dropped_partial_welcome;
}

pub fn rowForLineIndex(input: RowForLineInput) ?u16 {
    const selection = input.selection;
    if (selection.split_active) {
        const mapping = SplitLineMapping.fromSelection(selection);
        const target_visible = mapping.selectedIndexForSource(input.line_index) orelse return null;
        var row = selection.top_row;
        var selected_index: usize = 0;
        while (selected_index < target_visible) : (selected_index += 1) {
            const source_line = mapping.sourceLineForSelected(selected_index) orelse return null;
            row +|= visualRowsForLine(hardLineAt(input.bytes, source_line), input.cols);
        }
        if (row > input.max_row) return null;
        return row;
    }

    if (input.line_index < selection.start_line) return null;

    var row = selection.top_row;
    var line_index = selection.start_line;
    var skip_rows = selection.partial_skip_rows;
    while (line_index < input.line_index) : (line_index += 1) {
        const line = hardLineAt(input.bytes, line_index);
        const rows = visualRowsForLine(line, input.cols);
        if (skip_rows >= rows) {
            skip_rows -= rows;
            continue;
        }
        row +|= rows - skip_rows;
        skip_rows = 0;
    }
    if (row > input.max_row) return null;
    return row;
}

pub fn selectedSourceLineIndex(selection: ViewportSelection, selected_index: usize) ?usize {
    if (!selection.split_active) return selected_index;
    return SplitLineMapping.fromSelection(selection).sourceLineForSelected(selected_index);
}

fn selectTail(line_visual_rows: []const u16, visible_rows: u16) TailSelection {
    const total_lines = line_visual_rows.len;
    var rows_budget: u16 = visible_rows;
    var start_line: usize = total_lines;
    var partial_skip_rows: u16 = 0;
    while (start_line > 0) {
        const line_rows = line_visual_rows[start_line - 1];
        if (line_rows > rows_budget) {
            // Include the overflowing predecessor partially so the top
            // of the viewport is painted with current content instead
            // of exposing stale cells from a previous frame.
            if (rows_budget > 0) {
                start_line -= 1;
                partial_skip_rows = line_rows - rows_budget;
                rows_budget = 0;
            }
            break;
        }
        rows_budget -= line_rows;
        start_line -= 1;
    }
    if (start_line == total_lines and total_lines > 0) start_line = total_lines - 1;
    return .{
        .start_line = start_line,
        .partial_skip_rows = partial_skip_rows,
        .rows_budget_unused = rows_budget,
    };
}

fn remapReplaceableLineIndex(replaceable_line_index: usize, welcome_cut_line: usize, suffix_start: usize, selected_line_count: usize) usize {
    const mapping = SplitLineMapping{
        .prefix_lines = welcome_cut_line,
        .suffix_start_line = suffix_start,
    };
    return mapping.selectedIndexForSource(replaceable_line_index) orelse selected_line_count;
}

fn unusedRowsBudgetForSplit(line_visual_rows: []const u16, welcome_cut_line: usize, suffix_start: usize, total_lines: usize, visible_rows: u16) u16 {
    const mapping = SplitLineMapping{
        .prefix_lines = welcome_cut_line,
        .suffix_start_line = suffix_start,
    };
    const selected_line_count = welcome_cut_line + (total_lines - suffix_start);
    var used: u32 = 0;
    var selected_index: usize = 0;
    while (selected_index < selected_line_count) : (selected_index += 1) {
        const source_line = mapping.sourceLineForSelected(selected_index) orelse return 0;
        used += line_visual_rows[source_line];
        if (used >= visible_rows) return 0;
    }
    return @intCast(@as(u32, visible_rows) - used);
}

test "hard-line EOF policy treats final newline position as cursor only" {
    const alloc = std.testing.allocator;

    const empty = try buildHardLineStarts(alloc, "");
    defer empty.deinit(alloc);
    try std.testing.expectEqual(@as(usize, 0), empty.len());
    try std.testing.expect(!empty.ends_with_newline);

    const no_newline = try buildHardLineStarts(alloc, "hello");
    defer no_newline.deinit(alloc);
    try std.testing.expectEqual(@as(usize, 1), no_newline.len());
    try std.testing.expect(!no_newline.ends_with_newline);
    try std.testing.expectEqualStrings("hello", hardLineAt("hello", 0));

    const one_newline = try buildHardLineStarts(alloc, "hello\n");
    defer one_newline.deinit(alloc);
    try std.testing.expectEqual(@as(usize, 1), one_newline.len());
    try std.testing.expect(one_newline.ends_with_newline);

    const two_newlines = try buildHardLineStarts(alloc, "hello\n\n");
    defer two_newlines.deinit(alloc);
    try std.testing.expectEqual(@as(usize, 2), two_newlines.len());
    try std.testing.expect(two_newlines.ends_with_newline);
    try std.testing.expectEqualStrings("hello", hardLineAt("hello\n\n", 0));
    try std.testing.expectEqualStrings("", hardLineAt("hello\n\n", 1));
}

test "hardLineIndexAt ignores only the final EOF newline position" {
    try std.testing.expectEqual(@as(usize, 0), hardLineIndexAt("hello\n", 5));
    try std.testing.expectEqual(@as(usize, 0), hardLineIndexAt("hello\n", 6));
    try std.testing.expectEqual(@as(usize, 1), hardLineIndexAt("hello\n\n", 6));
    try std.testing.expectEqual(@as(usize, 1), hardLineIndexAt("hello\n\n", 7));
    try std.testing.expectEqual(@as(usize, 2), hardLineIndexAt("a\nb\nc", 4));
}

test "TranscriptRef resolves against captured snapshot buffer" {
    const alloc = std.testing.allocator;
    const bytes = "first line\nsecond line\n";
    const snapshot_bytes = try alloc.dupe(u8, bytes);
    defer alloc.free(snapshot_bytes);

    var starts: std.ArrayList(usize) = .empty;
    defer starts.deinit(alloc);
    try starts.append(alloc, 0);
    for (snapshot_bytes, 0..) |byte, i| {
        if (byte == '\n') try starts.append(alloc, i + 1);
    }

    const snapshot = TranscriptBuffer{ .bytes = snapshot_bytes };
    const ref = TranscriptRef{ .ref = transcriptLineAt(starts.items, snapshot.bytes.len, 0) };
    try std.testing.expectEqualStrings("first line", ref.resolve(snapshot));
}

test "buildViewportSelection pins welcome prefix and remaps replaceable line" {
    const rows = [_]u16{ 1, 1, 1, 1, 1, 1, 1, 1 };
    const result = buildViewportSelection(.{
        .top_row = 2,
        .bottom_row = 6,
        .line_visual_rows = rows[0..],
        .visible_rows = 5,
        .welcome_cut_line = 2,
        .replaceable_line_index = 7,
    });

    try std.testing.expectEqual(WelcomeDecision.pinned_welcome, result.welcome_decision);
    try std.testing.expect(result.selection.split_active);
    try std.testing.expectEqual(@as(usize, 2), result.selection.split_prefix_lines);
    try std.testing.expectEqual(@as(usize, 5), result.selection.split_suffix_start_line);
    try std.testing.expectEqual(@as(usize, 5), result.selection.line_count);
    try std.testing.expectEqual(@as(usize, 4), result.replaceable_line_index);
}

test "buildViewportSelection drops partial welcome when prefix cannot fit" {
    const rows = [_]u16{ 2, 2, 1, 1 };
    const result = buildViewportSelection(.{
        .top_row = 2,
        .bottom_row = 4,
        .line_visual_rows = rows[0..],
        .visible_rows = 3,
        .welcome_cut_line = 2,
        .replaceable_line_index = 3,
    });

    try std.testing.expectEqual(WelcomeDecision.dropped_partial_welcome, result.welcome_decision);
    try std.testing.expect(!result.selection.split_active);
    try std.testing.expectEqual(@as(usize, 2), result.selection.start_line);
    try std.testing.expectEqual(@as(u16, 0), result.selection.partial_skip_rows);
    try std.testing.expectEqual(@as(u16, 1), result.rows_budget_unused);
    try std.testing.expectEqual(@as(usize, 3), result.replaceable_line_index);
}

test "buildViewportSelection drops partial welcome when no suffix row fits" {
    const rows = [_]u16{ 1, 3, 5 };
    const result = buildViewportSelection(.{
        .top_row = 2,
        .bottom_row = 4,
        .line_visual_rows = rows[0..],
        .visible_rows = 3,
        .welcome_cut_line = 1,
        .replaceable_line_index = 2,
    });

    try std.testing.expectEqual(WelcomeDecision.dropped_partial_welcome, result.welcome_decision);
    try std.testing.expect(!result.selection.split_active);
    try std.testing.expectEqual(@as(usize, 2), result.selection.start_line);
    try std.testing.expectEqual(@as(u16, 2), result.selection.partial_skip_rows);
    try std.testing.expectEqual(@as(u16, 0), result.rows_budget_unused);
    try std.testing.expectEqual(@as(usize, 2), result.replaceable_line_index);
}

test "split selection maps selected indexes and hidden source lines" {
    const selection: ViewportSelection = .{
        .top_row = 3,
        .bottom_row = 7,
        .start_line = 0,
        .partial_skip_rows = 0,
        .line_count = 4,
        .split_active = true,
        .split_prefix_lines = 2,
        .split_suffix_start_line = 5,
    };

    try std.testing.expectEqual(@as(usize, 0), selectedSourceLineIndex(selection, 0).?);
    try std.testing.expectEqual(@as(usize, 1), selectedSourceLineIndex(selection, 1).?);
    try std.testing.expectEqual(@as(usize, 5), selectedSourceLineIndex(selection, 2).?);
    try std.testing.expectEqual(@as(usize, 6), selectedSourceLineIndex(selection, 3).?);
    try std.testing.expectEqual(@as(?usize, null), SplitLineMapping.fromSelection(selection).selectedIndexForSource(3));
    try std.testing.expectEqual(@as(usize, 3), SplitLineMapping.fromSelection(selection).selectedIndexForSource(6).?);
}

test "rowForLineIndex maps normal partial and split selections" {
    const normal: ViewportSelection = .{
        .top_row = 1,
        .bottom_row = 2,
        .start_line = 0,
        .partial_skip_rows = 1,
        .line_count = 2,
    };
    try std.testing.expectEqual(@as(u16, 1), rowForLineIndex(.{
        .selection = normal,
        .bytes = "abcdef\nstatus",
        .line_index = 0,
        .cols = 3,
        .max_row = 2,
    }).?);
    try std.testing.expectEqual(@as(u16, 2), rowForLineIndex(.{
        .selection = normal,
        .bytes = "abcdef\nstatus",
        .line_index = 1,
        .cols = 3,
        .max_row = 2,
    }).?);

    const split: ViewportSelection = .{
        .top_row = 3,
        .bottom_row = 7,
        .start_line = 0,
        .partial_skip_rows = 0,
        .line_count = 4,
        .split_active = true,
        .split_prefix_lines = 2,
        .split_suffix_start_line = 5,
    };
    const bytes = "w0\nw1\na\nb\nc\nd\nstatus";
    try std.testing.expect(rowForLineIndex(.{
        .selection = split,
        .bytes = bytes,
        .line_index = 3,
        .cols = 80,
        .max_row = 7,
    }) == null);
    try std.testing.expectEqual(@as(u16, 6), rowForLineIndex(.{
        .selection = split,
        .bytes = bytes,
        .line_index = 6,
        .cols = 80,
        .max_row = 7,
    }).?);
}
