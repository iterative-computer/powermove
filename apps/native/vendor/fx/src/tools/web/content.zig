const std = @import("std");
const content_contract = @import("../../core/tooling/web_fetch_content_contract.zig");
const text_utils = @import("../../core/shared/text_utils.zig");

const Allocator = std.mem.Allocator;

pub const max_converted_content_bytes = content_contract.max_converted_content_bytes;
pub const Kind = content_contract.Kind;
pub const Classification = content_contract.Classification;

pub fn classify(alloc: Allocator, content_type: ?[]const u8, body: []const u8) !Classification {
    if (content_type) |declared| {
        const normalized = try normalizedMime(alloc, declared);
        errdefer alloc.free(normalized);
        return .{
            .kind = declaredKind(normalized),
            .mime_type = normalized,
            .declared = true,
        };
    }

    if (text_utils.isModelSafeText(body)) {
        return .{
            .kind = .text,
            .mime_type = try alloc.dupe(u8, "text/plain"),
            .declared = false,
        };
    }

    return .{
        .kind = .binary,
        .mime_type = try alloc.dupe(u8, "application/octet-stream"),
        .declared = false,
    };
}

fn normalizedMime(alloc: Allocator, content_type: []const u8) ![]u8 {
    const parameter_start = std.mem.findScalar(u8, content_type, ';') orelse content_type.len;
    const trimmed = std.mem.trim(u8, content_type[0..parameter_start], " \t\r\n");
    if (trimmed.len == 0) return alloc.dupe(u8, "application/octet-stream");

    const normalized = try alloc.alloc(u8, trimmed.len);
    for (trimmed, 0..) |char, index| normalized[index] = std.ascii.toLower(char);
    return normalized;
}

fn declaredKind(mime: []const u8) Kind {
    if (std.mem.eql(u8, mime, "text/html")) return .html;
    if (std.mem.eql(u8, mime, "application/xhtml+xml")) return .html;
    if (std.mem.startsWith(u8, mime, "text/")) return .text;
    if (std.mem.eql(u8, mime, "application/json")) return .text;
    if (std.mem.eql(u8, mime, "application/xml")) return .text;
    if (std.mem.eql(u8, mime, "application/javascript")) return .text;
    if (std.mem.eql(u8, mime, "application/x-javascript")) return .text;
    if (std.mem.startsWith(u8, mime, "application/") and std.mem.endsWith(u8, mime, "+json")) return .text;
    if (std.mem.startsWith(u8, mime, "application/") and std.mem.endsWith(u8, mime, "+xml")) return .text;
    return .binary;
}

test "web_fetch classifies declared mime and missing content type deterministically" {
    const alloc = std.testing.allocator;

    const cases = [_]struct {
        content_type: ?[]const u8,
        body: []const u8 = "hello",
        kind: Kind,
        mime_type: []const u8,
        declared: bool = true,
    }{
        .{ .content_type = "Text/HTML; Charset=UTF-8", .kind = .html, .mime_type = "text/html" },
        .{ .content_type = "TEXT/PLAIN", .kind = .text, .mime_type = "text/plain" },
        .{ .content_type = "application/json; charset=utf-8", .kind = .text, .mime_type = "application/json" },
        .{ .content_type = "application/activity+json", .kind = .text, .mime_type = "application/activity+json" },
        .{ .content_type = "application/xml", .kind = .text, .mime_type = "application/xml" },
        .{ .content_type = "application/rss+xml", .kind = .text, .mime_type = "application/rss+xml" },
        .{ .content_type = "application/javascript", .kind = .text, .mime_type = "application/javascript" },
        .{ .content_type = "application/x-javascript", .kind = .text, .mime_type = "application/x-javascript" },
        .{ .content_type = "application/pdf", .kind = .binary, .mime_type = "application/pdf" },
        .{ .content_type = null, .kind = .text, .mime_type = "text/plain", .declared = false },
        .{ .content_type = null, .body = "bad\x00text", .kind = .binary, .mime_type = "application/octet-stream", .declared = false },
    };

    for (cases) |case| {
        var classification = try classify(alloc, case.content_type, case.body);
        defer classification.deinit(alloc);

        try std.testing.expectEqual(case.kind, classification.kind);
        try std.testing.expectEqualStrings(case.mime_type, classification.mime_type);
        try std.testing.expectEqual(case.declared, classification.declared);
    }
}
