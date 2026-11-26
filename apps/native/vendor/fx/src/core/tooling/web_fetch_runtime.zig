const std = @import("std");
const io_mod = @import("../shared/io.zig");
const content_contract = @import("web_fetch_content_contract.zig");
const web_fetch_artifacts = @import("../session/web_fetch_artifacts.zig");

const Allocator = std.mem.Allocator;

pub const ttl_ms: i64 = 15 * 60 * 1000;
pub const default_max_entries: usize = 1024;
pub const default_max_metadata_bytes: usize = 4 * 1024 * 1024;

pub const Clock = struct {
    ctx: *anyopaque = @ptrCast(&default_clock_context),
    now_fn: *const fn (*anyopaque) i64 = defaultNow,

    pub fn now(self: Clock) i64 {
        return self.now_fn(self.ctx);
    }
};

var default_clock_context: u8 = 0;

fn defaultNow(_: *anyopaque) i64 {
    const ts = std.Io.Clock.awake.now(io_mod.getIo());
    return @intCast(@divFloor(ts.nanoseconds, 1_000_000));
}

pub const ArtifactRef = struct {
    store_id: []const u8,
    handle: []const u8,
    display_path: []const u8,
    byte_count: usize,
};

pub const CacheEntryData = struct {
    submitted_url: []const u8,
    final_url: []const u8,
    status: std.http.Status,
    mime_type: []const u8,
    content_kind: content_contract.Kind,
    converted_content: []const u8 = "",
    artifact_ref: ?ArtifactRef = null,
};

pub const CacheSnapshot = struct {
    submitted_url: []u8,
    final_url: []u8,
    status: std.http.Status,
    mime_type: []u8,
    content_kind: content_contract.Kind,
    converted_content: []u8,
    artifact_ref: ?OwnedArtifactRef = null,
    cache_hit: bool = false,

    pub const OwnedArtifactRef = struct {
        store_id: []u8,
        handle: []u8,
        display_path: []u8,
        byte_count: usize,
    };

    pub fn deinit(self: *CacheSnapshot, alloc: Allocator) void {
        alloc.free(self.submitted_url);
        alloc.free(self.final_url);
        alloc.free(self.mime_type);
        alloc.free(self.converted_content);
        if (self.artifact_ref) |ref| {
            alloc.free(ref.store_id);
            alloc.free(ref.handle);
            alloc.free(ref.display_path);
        }
        self.* = undefined;
    }
};

pub const Config = struct {
    allocator: Allocator = std.heap.c_allocator,
    clock: Clock = .{},
    max_converted_bytes: usize = 50 * 1024 * 1024,
    max_entries: usize = default_max_entries,
    max_metadata_bytes: usize = default_max_metadata_bytes,
};

pub const Runtime = struct {
    config: Config,
    mutex: std.Io.Mutex = .init,
    entries: std.ArrayList(Entry) = .empty,
    converted_bytes: usize = 0,
    metadata_bytes: usize = 0,
    lru_tick: u64 = 0,
    lock_depth: usize = 0,

    pub fn init(config: Config) Runtime {
        return .{ .config = config };
    }

    pub fn deinit(self: *Runtime, _: Allocator) void {
        const alloc = self.config.allocator;
        for (self.entries.items) |*entry| entry.deinit(alloc);
        self.entries.deinit(alloc);
        self.* = Runtime.init(self.config);
    }

    pub fn lookup(self: *Runtime, alloc: Allocator, submitted_url: []const u8, artifact_store: ?*web_fetch_artifacts.Store) !?CacheSnapshot {
        var snapshot: CacheSnapshot = undefined;
        var artifact_handle_for_check: ?[]u8 = null;
        errdefer if (artifact_handle_for_check) |handle| alloc.free(handle);

        {
            self.lock();
            defer self.unlock();

            self.evictExpiredLocked(self.config.clock.now());
            const index = self.findIndexLocked(submitted_url) orelse return null;
            if (self.entries.items[index].artifact_ref) |artifact_ref| {
                const active_store = artifact_store orelse {
                    self.removeAtLocked(index);
                    return null;
                };
                if (!std.mem.eql(u8, artifact_ref.store_id, active_store.id())) {
                    self.removeAtLocked(index);
                    return null;
                }
                artifact_handle_for_check = try alloc.dupe(u8, artifact_ref.handle);
            }

            self.lru_tick += 1;
            self.entries.items[index].last_used = self.lru_tick;
            snapshot = try self.entries.items[index].snapshot(alloc);
            snapshot.cache_hit = true;
        }

        if (artifact_handle_for_check) |handle| {
            artifact_handle_for_check = null;
            defer alloc.free(handle);
            const active_store = artifact_store.?;
            const exists = active_store.contains(handle) catch |err| {
                snapshot.deinit(alloc);
                return err;
            };
            if (!exists) {
                snapshot.deinit(alloc);
                self.removeArtifactRefIfStillMissing(submitted_url, handle);
                return null;
            }
        }
        return snapshot;
    }

    pub fn insert(self: *Runtime, _: Allocator, data: CacheEntryData) !void {
        if (data.converted_content.len > content_contract.max_converted_content_bytes) return error.ConvertedContentTooLarge;

        self.lock();
        defer self.unlock();

        self.evictExpiredLocked(self.config.clock.now());
        if (self.findIndexLocked(data.submitted_url)) |index| self.removeAtLocked(index);

        const metadata_weight = metadataWeight(data);
        if (metadata_weight > self.config.max_metadata_bytes) return;
        if (data.converted_content.len > self.config.max_converted_bytes) return;

        self.lru_tick += 1;
        const cache_alloc = self.config.allocator;
        var entry = try Entry.fromData(cache_alloc, data, .{
            .expires_at_ms = self.config.clock.now() + ttl_ms,
            .last_used = self.lru_tick,
            .metadata_weight = metadata_weight,
        });
        errdefer entry.deinit(cache_alloc);

        try self.entries.append(cache_alloc, entry);
        self.converted_bytes += entry.converted_content.len;
        self.metadata_bytes += entry.metadata_weight;
        self.evictOverBudgetLocked();
    }

    pub fn convertedBytes(self: *Runtime) usize {
        self.lock();
        defer self.unlock();
        return self.converted_bytes;
    }

    pub fn entryCount(self: *Runtime) usize {
        self.lock();
        defer self.unlock();
        return self.entries.items.len;
    }

    pub fn metadataBytes(self: *Runtime) usize {
        self.lock();
        defer self.unlock();
        return self.metadata_bytes;
    }

    pub fn lockHeldForTest(self: *Runtime) bool {
        return self.lock_depth > 0;
    }

    fn lock(self: *Runtime) void {
        self.mutex.lockUncancelable(io_mod.getIo());
        self.lock_depth += 1;
    }

    fn unlock(self: *Runtime) void {
        self.lock_depth -= 1;
        self.mutex.unlock(io_mod.getIo());
    }

    fn findIndexLocked(self: *Runtime, submitted_url: []const u8) ?usize {
        for (self.entries.items, 0..) |entry, index| {
            if (std.mem.eql(u8, entry.submitted_url, submitted_url)) return index;
        }
        return null;
    }

    fn evictExpiredLocked(self: *Runtime, now_ms: i64) void {
        var index: usize = 0;
        while (index < self.entries.items.len) {
            if (self.entries.items[index].expires_at_ms > now_ms) {
                index += 1;
                continue;
            }
            self.removeAtLocked(index);
        }
    }

    fn evictOverBudgetLocked(self: *Runtime) void {
        while (self.entries.items.len > self.config.max_entries or
            self.converted_bytes > self.config.max_converted_bytes or
            self.metadata_bytes > self.config.max_metadata_bytes)
        {
            const index = self.leastRecentlyUsedIndexLocked() orelse return;
            self.removeAtLocked(index);
        }
    }

    fn leastRecentlyUsedIndexLocked(self: *Runtime) ?usize {
        if (self.entries.items.len == 0) return null;
        var selected: usize = 0;
        for (self.entries.items[1..], 1..) |entry, index| {
            if (entry.last_used < self.entries.items[selected].last_used) selected = index;
        }
        return selected;
    }

    fn removeAtLocked(self: *Runtime, index: usize) void {
        var entry = self.entries.swapRemove(index);
        self.converted_bytes -= entry.converted_content.len;
        self.metadata_bytes -= entry.metadata_weight;
        entry.deinit(self.config.allocator);
    }

    fn removeArtifactRefIfStillMissing(self: *Runtime, submitted_url: []const u8, handle: []const u8) void {
        self.lock();
        defer self.unlock();

        const index = self.findIndexLocked(submitted_url) orelse return;
        const artifact_ref = self.entries.items[index].artifact_ref orelse return;
        if (std.mem.eql(u8, artifact_ref.handle, handle)) self.removeAtLocked(index);
    }
};

const EntryInit = struct {
    expires_at_ms: i64,
    last_used: u64,
    metadata_weight: usize,
};

const Entry = struct {
    submitted_url: []u8,
    final_url: []u8,
    status: std.http.Status,
    mime_type: []u8,
    content_kind: content_contract.Kind,
    converted_content: []u8,
    artifact_ref: ?CacheSnapshot.OwnedArtifactRef = null,
    expires_at_ms: i64,
    last_used: u64,
    metadata_weight: usize,

    fn fromData(alloc: Allocator, data: CacheEntryData, init: EntryInit) !Entry {
        const submitted_url = try alloc.dupe(u8, data.submitted_url);
        errdefer alloc.free(submitted_url);
        const final_url = try alloc.dupe(u8, data.final_url);
        errdefer alloc.free(final_url);
        const mime_type = try alloc.dupe(u8, data.mime_type);
        errdefer alloc.free(mime_type);
        const converted_content = try alloc.dupe(u8, data.converted_content);
        errdefer alloc.free(converted_content);
        const artifact_ref = if (data.artifact_ref) |ref| try cloneArtifactRef(alloc, ref) else null;
        errdefer if (artifact_ref) |owned| freeArtifactRef(alloc, owned);

        return .{
            .submitted_url = submitted_url,
            .final_url = final_url,
            .status = data.status,
            .mime_type = mime_type,
            .content_kind = data.content_kind,
            .converted_content = converted_content,
            .artifact_ref = artifact_ref,
            .expires_at_ms = init.expires_at_ms,
            .last_used = init.last_used,
            .metadata_weight = init.metadata_weight,
        };
    }

    fn deinit(self: *Entry, alloc: Allocator) void {
        alloc.free(self.submitted_url);
        alloc.free(self.final_url);
        alloc.free(self.mime_type);
        alloc.free(self.converted_content);
        if (self.artifact_ref) |ref| freeArtifactRef(alloc, ref);
        self.* = undefined;
    }

    fn snapshot(self: Entry, alloc: Allocator) !CacheSnapshot {
        const submitted_url = try alloc.dupe(u8, self.submitted_url);
        errdefer alloc.free(submitted_url);
        const final_url = try alloc.dupe(u8, self.final_url);
        errdefer alloc.free(final_url);
        const mime_type = try alloc.dupe(u8, self.mime_type);
        errdefer alloc.free(mime_type);
        const converted_content = try alloc.dupe(u8, self.converted_content);
        errdefer alloc.free(converted_content);
        const artifact_ref = if (self.artifact_ref) |ref| try cloneOwnedArtifactRef(alloc, ref) else null;
        errdefer if (artifact_ref) |owned| freeArtifactRef(alloc, owned);

        return .{
            .submitted_url = submitted_url,
            .final_url = final_url,
            .status = self.status,
            .mime_type = mime_type,
            .content_kind = self.content_kind,
            .converted_content = converted_content,
            .artifact_ref = artifact_ref,
        };
    }
};

fn metadataWeight(data: CacheEntryData) usize {
    var weight: usize = 32;
    weight += data.submitted_url.len;
    weight += data.final_url.len;
    weight += data.mime_type.len;
    if (data.artifact_ref) |ref| {
        weight += ref.store_id.len;
        weight += ref.handle.len;
        weight += ref.display_path.len;
        weight += @sizeOf(usize);
    }
    return weight;
}

fn cloneArtifactRef(alloc: Allocator, ref: ArtifactRef) !CacheSnapshot.OwnedArtifactRef {
    const store_id = try alloc.dupe(u8, ref.store_id);
    errdefer alloc.free(store_id);
    const handle = try alloc.dupe(u8, ref.handle);
    errdefer alloc.free(handle);
    const display_path = try alloc.dupe(u8, ref.display_path);
    return .{ .store_id = store_id, .handle = handle, .display_path = display_path, .byte_count = ref.byte_count };
}

fn cloneOwnedArtifactRef(alloc: Allocator, ref: CacheSnapshot.OwnedArtifactRef) !CacheSnapshot.OwnedArtifactRef {
    return cloneArtifactRef(alloc, .{
        .store_id = ref.store_id,
        .handle = ref.handle,
        .display_path = ref.display_path,
        .byte_count = ref.byte_count,
    });
}

fn freeArtifactRef(alloc: Allocator, ref: CacheSnapshot.OwnedArtifactRef) void {
    alloc.free(ref.store_id);
    alloc.free(ref.handle);
    alloc.free(ref.display_path);
}

const MutableClock = struct {
    now_ms: i64,

    fn now(raw: *anyopaque) i64 {
        const self: *@This() = @ptrCast(@alignCast(raw));
        return self.now_ms;
    }
};

fn testRuntime(clock: *MutableClock, max_converted_bytes: usize, max_entries: usize, max_metadata_bytes: usize) Runtime {
    return Runtime.init(.{
        .allocator = std.testing.allocator,
        .clock = .{ .ctx = @ptrCast(clock), .now_fn = MutableClock.now },
        .max_converted_bytes = max_converted_bytes,
        .max_entries = max_entries,
        .max_metadata_bytes = max_metadata_bytes,
    });
}

test "web_fetch cache expires at fifteen minutes and evicts by converted bytes" {
    const alloc = std.testing.allocator;
    var clock = MutableClock{ .now_ms = 1000 };
    var runtime = testRuntime(&clock, 10, 8, 4096);
    defer runtime.deinit(alloc);

    try runtime.insert(alloc, .{
        .submitted_url = "https://example.com/a",
        .final_url = "https://example.com/a",
        .status = .ok,
        .mime_type = "text/plain",
        .content_kind = .text,
        .converted_content = "12345",
    });
    try runtime.insert(alloc, .{
        .submitted_url = "https://example.com/b",
        .final_url = "https://example.com/b",
        .status = .ok,
        .mime_type = "text/plain",
        .content_kind = .text,
        .converted_content = "67890",
    });
    try runtime.insert(alloc, .{
        .submitted_url = "https://example.com/c",
        .final_url = "https://example.com/c",
        .status = .ok,
        .mime_type = "text/plain",
        .content_kind = .text,
        .converted_content = "abcde",
    });

    try std.testing.expectEqual(@as(usize, 2), runtime.entryCount());
    try std.testing.expect((try runtime.lookup(alloc, "https://example.com/a", null)) == null);

    var hit = (try runtime.lookup(alloc, "https://example.com/c", null)) orelse return error.TestExpectedEqual;
    defer hit.deinit(alloc);
    try std.testing.expect(hit.cache_hit);

    clock.now_ms += ttl_ms + 1;
    try std.testing.expect((try runtime.lookup(alloc, "https://example.com/c", null)) == null);
}

test "web_fetch cache snapshots remain valid after post lookup eviction" {
    const alloc = std.testing.allocator;
    var clock = MutableClock{ .now_ms = 1 };
    var runtime = testRuntime(&clock, 6, 8, 4096);
    defer runtime.deinit(alloc);

    try runtime.insert(alloc, .{
        .submitted_url = "https://example.com/a",
        .final_url = "https://example.com/a",
        .status = .ok,
        .mime_type = "text/plain",
        .content_kind = .text,
        .converted_content = "abc",
    });
    var snapshot = (try runtime.lookup(alloc, "https://example.com/a", null)) orelse return error.TestExpectedEqual;
    defer snapshot.deinit(alloc);

    try runtime.insert(alloc, .{
        .submitted_url = "https://example.com/b",
        .final_url = "https://example.com/b",
        .status = .ok,
        .mime_type = "text/plain",
        .content_kind = .text,
        .converted_content = "defghi",
    });

    try std.testing.expect((try runtime.lookup(alloc, "https://example.com/a", null)) == null);
    try std.testing.expectEqualStrings("abc", snapshot.converted_content);
}

test "web_fetch cache caps entry count and metadata weight" {
    const alloc = std.testing.allocator;
    var clock = MutableClock{ .now_ms = 1 };
    var runtime = testRuntime(&clock, 1024, 2, 120);
    defer runtime.deinit(alloc);

    try runtime.insert(alloc, .{ .submitted_url = "https://example.com/a", .final_url = "https://example.com/a", .status = .ok, .mime_type = "text/plain", .content_kind = .text });
    try runtime.insert(alloc, .{ .submitted_url = "https://example.com/b", .final_url = "https://example.com/b", .status = .ok, .mime_type = "text/plain", .content_kind = .text });
    try runtime.insert(alloc, .{ .submitted_url = "https://example.com/c", .final_url = "https://example.com/c", .status = .ok, .mime_type = "text/plain", .content_kind = .text });

    try std.testing.expect(runtime.entryCount() <= 2);
    try std.testing.expect(runtime.metadataBytes() <= 120);
}

test "web_fetch durable artifact cache refs are scoped to artifact store identity" {
    const alloc = std.testing.allocator;
    var clock = MutableClock{ .now_ms = 1 };
    var runtime = testRuntime(&clock, 1024, 8, 4096);
    defer runtime.deinit(alloc);
    var tmp_a = std.testing.tmpDir(.{});
    defer tmp_a.cleanup();
    var tmp_b = std.testing.tmpDir(.{});
    defer tmp_b.cleanup();
    const session_a = try io_mod.dirRealpathAlloc(alloc, tmp_a.dir, ".");
    defer alloc.free(session_a);
    const session_b = try io_mod.dirRealpathAlloc(alloc, tmp_b.dir, ".");
    defer alloc.free(session_b);
    var store_a = try web_fetch_artifacts.Store.init(alloc, session_a);
    defer store_a.deinit();
    var store_b = try web_fetch_artifacts.Store.init(alloc, session_b);
    defer store_b.deinit();
    var artifact = try store_a.write(alloc, "application/pdf", "pdf");
    defer artifact.deinit(alloc);

    try runtime.insert(alloc, .{
        .submitted_url = "https://example.com/report.pdf",
        .final_url = "https://example.com/report.pdf",
        .status = .ok,
        .mime_type = "application/pdf",
        .content_kind = .binary,
        .artifact_ref = .{
            .store_id = store_a.id(),
            .handle = artifact.handle,
            .display_path = artifact.display_path,
            .byte_count = artifact.byte_count,
        },
    });

    var hit = (try runtime.lookup(alloc, "https://example.com/report.pdf", &store_a)) orelse return error.TestExpectedEqual;
    defer hit.deinit(alloc);
    try std.testing.expectEqualStrings(store_a.id(), hit.artifact_ref.?.store_id);

    try std.testing.expect((try runtime.lookup(alloc, "https://example.com/report.pdf", &store_b)) == null);
    try std.testing.expect((try runtime.lookup(alloc, "https://example.com/report.pdf", &store_a)) == null);
}

test "web_fetch cache eviction preserves returned durable artifact" {
    const alloc = std.testing.allocator;
    var clock = MutableClock{ .now_ms = 1 };
    var runtime = testRuntime(&clock, 1024, 1, 4096);
    defer runtime.deinit(alloc);
    var tmp = std.testing.tmpDir(.{});
    defer tmp.cleanup();
    const session_dir = try io_mod.dirRealpathAlloc(alloc, tmp.dir, ".");
    defer alloc.free(session_dir);
    var store = try web_fetch_artifacts.Store.init(alloc, session_dir);
    defer store.deinit();
    var artifact = try store.write(alloc, "application/pdf", "pdf bytes");
    defer artifact.deinit(alloc);

    try runtime.insert(alloc, .{
        .submitted_url = "https://example.com/report.pdf",
        .final_url = "https://example.com/report.pdf",
        .status = .ok,
        .mime_type = "application/pdf",
        .content_kind = .binary,
        .artifact_ref = .{
            .store_id = store.id(),
            .handle = artifact.handle,
            .display_path = artifact.display_path,
            .byte_count = artifact.byte_count,
        },
    });
    try runtime.insert(alloc, .{
        .submitted_url = "https://example.com/next",
        .final_url = "https://example.com/next",
        .status = .ok,
        .mime_type = "text/plain",
        .content_kind = .text,
        .converted_content = "next",
    });

    try std.testing.expect((try runtime.lookup(alloc, "https://example.com/report.pdf", &store)) == null);
    try std.testing.expect(try store.contains(artifact.handle));
}

test "web_fetch cache lock is not held across target http" {
    var clock = MutableClock{ .now_ms = 1 };
    var runtime = testRuntime(&clock, 1024, 8, 4096);
    defer runtime.deinit(std.testing.allocator);

    try std.testing.expect(!runtime.lockHeldForTest());
}
