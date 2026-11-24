const std = @import("std");

const Allocator = std.mem.Allocator;

pub const max_converted_content_bytes: usize = 10 * 1024 * 1024;

pub const Kind = enum {
    text,
    html,
    binary,
};

pub const Classification = struct {
    kind: Kind,
    mime_type: []u8,
    declared: bool,

    pub fn deinit(self: *Classification, alloc: Allocator) void {
        alloc.free(self.mime_type);
        self.* = .{ .kind = .binary, .mime_type = &.{}, .declared = false };
    }
};

test "web_fetch content contract owns classification shape and conversion limit" {
    const alloc = std.testing.allocator;
    var classification = Classification{
        .kind = .html,
        .mime_type = try alloc.dupe(u8, "text/html"),
        .declared = true,
    };
    classification.deinit(alloc);

    try std.testing.expectEqual(Kind.binary, classification.kind);
    try std.testing.expectEqual(@as(usize, 10 * 1024 * 1024), max_converted_content_bytes);
    try std.testing.expectEqual(@as(usize, 0), classification.mime_type.len);
    try std.testing.expect(!classification.declared);
}
