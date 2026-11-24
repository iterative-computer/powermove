const std = @import("std");
const types = @import("../shared/types.zig");

const Allocator = std.mem.Allocator;

/// Content digest recorded for a file read by the current session.
pub const ContentHash = types.ContentHash;

/// Snapshot of a file at the moment an agent-visible read succeeded.
/// Tracks model-visible coverage separately from full-file freshness coverage so
/// tools can enforce the stricter invariant only where whole-file model context matters.
pub const Record = struct {
    mtime_ns: i128,
    content_hash: ContentHash,
    /// True iff the model-visible read returned the entire file content
    /// (no display cap hit, no start_line/line_count narrowing).
    model_view_covers_full_file: bool,
    /// True iff content_hash was computed over the entire file on disk
    /// at read time (independent of what the model saw).
    snapshot_covers_full_file: bool,
};

/// Session-scoped in-memory index of files read by the agent.
pub const ReadTracker = struct {
    alloc: Allocator,
    entries: std.StringHashMap(Record),

    /// Initializes an empty tracker. Call deinit to free owned path keys.
    pub fn init(alloc: Allocator) ReadTracker {
        return .{
            .alloc = alloc,
            .entries = std.StringHashMap(Record).init(alloc),
        };
    }

    /// Records a snapshot for path, replacing any previous record.
    pub fn record(self: *ReadTracker, path: []const u8, value: Record) Allocator.Error!void {
        const owned_path = try self.alloc.dupe(u8, path);
        errdefer self.alloc.free(owned_path);

        const entry = try self.entries.getOrPut(owned_path);
        if (entry.found_existing) {
            self.alloc.free(owned_path);
        }
        entry.value_ptr.* = value;
    }

    /// Returns the snapshot for path, if the session has one.
    pub fn lookup(self: *const ReadTracker, path: []const u8) ?Record {
        return self.entries.get(path);
    }

    /// Frees all owned path keys and map storage.
    pub fn deinit(self: *ReadTracker) void {
        var keys = self.entries.keyIterator();
        while (keys.next()) |key| self.alloc.free(key.*);
        self.entries.deinit();
        self.* = undefined;
    }
};

/// Computes the content hash used by read-before-overwrite checks.
pub fn contentHash(bytes: []const u8) ContentHash {
    var hasher = std.crypto.hash.sha2.Sha256.init(.{});
    hasher.update(bytes);
    return hasher.finalResult();
}

test "ReadTracker records, looks up, overwrites, and frees owned keys" {
    const alloc = std.testing.allocator;
    var tracker = ReadTracker.init(alloc);
    defer tracker.deinit();

    const first = Record{
        .mtime_ns = 10,
        .content_hash = contentHash("first"),
        .model_view_covers_full_file = true,
        .snapshot_covers_full_file = true,
    };
    try tracker.record("/tmp/file.txt", first);

    const stored_first = tracker.lookup("/tmp/file.txt").?;
    try std.testing.expectEqual(@as(i128, 10), stored_first.mtime_ns);
    try std.testing.expectEqualSlices(u8, &first.content_hash, &stored_first.content_hash);
    try std.testing.expect(stored_first.model_view_covers_full_file);
    try std.testing.expect(stored_first.snapshot_covers_full_file);

    const second = Record{
        .mtime_ns = 20,
        .content_hash = contentHash("second"),
        .model_view_covers_full_file = false,
        .snapshot_covers_full_file = false,
    };
    try tracker.record("/tmp/file.txt", second);

    const stored_second = tracker.lookup("/tmp/file.txt").?;
    try std.testing.expectEqual(@as(i128, 20), stored_second.mtime_ns);
    try std.testing.expectEqualSlices(u8, &second.content_hash, &stored_second.content_hash);
    try std.testing.expect(!stored_second.model_view_covers_full_file);
    try std.testing.expect(!stored_second.snapshot_covers_full_file);
}

test "ReadTracker lookup returns null for unread paths" {
    var tracker = ReadTracker.init(std.testing.allocator);
    defer tracker.deinit();

    try std.testing.expect(tracker.lookup("/tmp/missing.txt") == null);
}

test "contentHash distinguishes empty and non-empty content" {
    const empty = contentHash("");
    const filled = contentHash("x");

    try std.testing.expect(!std.mem.eql(u8, &empty, &filled));
    try std.testing.expectEqualSlices(u8, &empty, &contentHash(""));
}
