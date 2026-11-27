const std = @import("std");

pub const Span = struct {
    raw_start: usize,
    raw_end: usize,

    pub fn isValid(self: Span, input_len: usize) bool {
        return self.raw_start < self.raw_end and self.raw_end <= input_len;
    }

    pub fn contains(self: Span, raw_offset: usize) bool {
        return self.raw_start < raw_offset and raw_offset < self.raw_end;
    }
};

pub const ImageTokenSpan = struct {
    span: Span,
    id: usize,
};

/// Projects a registered span through an insertion. Insertion at the start
/// moves the entity; insertion at the end leaves it in place. Null means the
/// edit entered the entity or arithmetic overflowed.
pub fn afterInsert(span: Span, raw_offset: usize, byte_len: usize) ?Span {
    if (byte_len == 0 or raw_offset >= span.raw_end) return span;
    if (raw_offset > span.raw_start) return null;
    return .{
        .raw_start = std.math.add(usize, span.raw_start, byte_len) catch return null,
        .raw_end = std.math.add(usize, span.raw_end, byte_len) catch return null,
    };
}

/// Projects a registered span through a deletion. Null means the deleted range
/// overlapped the entity.
pub fn afterDelete(span: Span, delete_start: usize, delete_end: usize) ?Span {
    if (delete_start >= delete_end or delete_start >= span.raw_end) return span;
    if (delete_end <= span.raw_start) {
        const removed = delete_end - delete_start;
        return .{
            .raw_start = span.raw_start - removed,
            .raw_end = span.raw_end - removed,
        };
    }
    return null;
}

pub fn overlaps(a: Span, b: Span) bool {
    return a.raw_start < b.raw_end and b.raw_start < a.raw_end;
}

pub fn containsSpan(outer: Span, inner: Span) bool {
    return outer.raw_start <= inner.raw_start and inner.raw_end <= outer.raw_end;
}

test "entity spans shift only for edits outside their contents" {
    const span = Span{ .raw_start = 4, .raw_end = 10 };

    try std.testing.expectEqual(Span{ .raw_start = 6, .raw_end = 12 }, afterInsert(span, 4, 2).?);
    try std.testing.expectEqual(span, afterInsert(span, 10, 2).?);
    try std.testing.expect(afterInsert(span, 7, 2) == null);

    try std.testing.expectEqual(Span{ .raw_start = 2, .raw_end = 8 }, afterDelete(span, 1, 3).?);
    try std.testing.expectEqual(span, afterDelete(span, 10, 12).?);
    try std.testing.expect(afterDelete(span, 3, 5) == null);
    try std.testing.expect(afterDelete(span, 5, 7) == null);
}

test "entity span relationship helpers use half-open ranges" {
    const entity = Span{ .raw_start = 4, .raw_end = 10 };
    try std.testing.expect(entity.isValid(10));
    try std.testing.expect(!entity.isValid(9));
    try std.testing.expect(!entity.contains(4));
    try std.testing.expect(entity.contains(5));
    try std.testing.expect(!entity.contains(10));
    try std.testing.expect(overlaps(entity, .{ .raw_start = 9, .raw_end = 12 }));
    try std.testing.expect(!overlaps(entity, .{ .raw_start = 10, .raw_end = 12 }));
    try std.testing.expect(containsSpan(entity, .{ .raw_start = 5, .raw_end = 9 }));
}
