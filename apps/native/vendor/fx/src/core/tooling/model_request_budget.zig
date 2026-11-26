const std = @import("std");

pub fn searchWorkerInputFitsLimit(
    input_overhead_bytes: usize,
    query: []const u8,
    allowed_domains: ?[]const []const u8,
    blocked_domains: ?[]const []const u8,
    max_input_bytes: usize,
) bool {
    var input_bytes: usize = 0;
    addBytes(&input_bytes, input_overhead_bytes) catch return false;
    addBytes(&input_bytes, query.len) catch return false;
    addOptionalStrings(&input_bytes, allowed_domains) catch return false;
    addOptionalStrings(&input_bytes, blocked_domains) catch return false;
    return input_bytes <= max_input_bytes;
}

fn addOptionalStrings(total: *usize, maybe_strings: ?[]const []const u8) !void {
    const strings = maybe_strings orelse return;
    for (strings) |value| try addBytes(total, value.len);
}

fn addBytes(total: *usize, count: usize) !void {
    total.* = try std.math.add(usize, total.*, count);
}

test "search worker input cap uses conservative serialized bytes" {
    try std.testing.expect(searchWorkerInputFitsLimit("system".len, "query", &.{"allowed"}, null, 18));
    try std.testing.expect(!searchWorkerInputFitsLimit("system".len, "query", &.{"allowed"}, null, 17));
}
