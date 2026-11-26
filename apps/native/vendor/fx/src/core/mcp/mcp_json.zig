const std = @import("std");

/// Rewrites `json` without insignificant whitespace and with raw control
/// bytes inside string literals escaped, so an NDJSON frame stays on one line.
/// All other bytes pass through unchanged, preserving valid input tokens.
pub fn write_compact(writer: *std.Io.Writer, json: []const u8) !void {
    var in_string = false;
    var escape_pending = false;
    for (json) |byte| {
        if (!in_string) {
            switch (byte) {
                ' ', '\t', '\n', '\r' => {},
                else => {
                    if (byte == '"') in_string = true;
                    try writer.writeByte(byte);
                },
            }
            continue;
        }
        if (byte < 0x20) {
            // Raw control bytes are invalid JSON inside a string; escaping
            // them keeps malformed model output inside its NDJSON frame.
            try writer.print("\\u{x:0>4}", .{byte});
            escape_pending = false;
            continue;
        }
        try writer.writeByte(byte);
        if (escape_pending) {
            escape_pending = false;
        } else switch (byte) {
            '\\' => escape_pending = true,
            '"' => in_string = false,
            else => {},
        }
    }
}

test "compact JSON escapes raw control bytes inside strings" {
    const alloc = std.testing.allocator;
    var out: std.Io.Writer.Allocating = .init(alloc);
    defer out.deinit();

    try write_compact(&out.writer, "{\"k\": \"a\nb\"}");
    try std.testing.expectEqualStrings("{\"k\":\"a\\u000ab\"}", out.written());
}

test "compact JSON escapes a raw control byte after a backslash" {
    const alloc = std.testing.allocator;
    var out: std.Io.Writer.Allocating = .init(alloc);
    defer out.deinit();

    try write_compact(&out.writer, "{\"k\":\"a\\\nb\"}");
    try std.testing.expect(std.mem.findScalar(u8, out.written(), '\n') == null);
    try std.testing.expectEqualStrings("{\"k\":\"a\\\\u000ab\"}", out.written());
}

test "compact JSON preserves escaped quotes and backslashes" {
    const alloc = std.testing.allocator;
    var out: std.Io.Writer.Allocating = .init(alloc);
    defer out.deinit();

    try write_compact(&out.writer, "{\"k\": \"say \\\"hi now\\\"\", \"p\": \"x\\\\\", \"q\": 1}");
    try std.testing.expectEqualStrings("{\"k\":\"say \\\"hi now\\\"\",\"p\":\"x\\\\\",\"q\":1}", out.written());
}
