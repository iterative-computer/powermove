const std = @import("std");
const session_codec = @import("session_codec.zig");

const Allocator = std.mem.Allocator;

pub const max_session_bytes: usize = 16 * 1024 * 1024;
pub const max_revision_bytes: usize = 1024;
pub const max_list_bytes: usize = 4 * 1024 * 1024;
const initial_session_bytes: usize = 4 * 1024;
const initial_revision_bytes: usize = 64;
const initial_list_bytes: usize = 4 * 1024;

pub const Loaded = struct {
    state: session_codec.DurableSessionState,
    revision: []u8,

    pub fn deinit(self: *Loaded, alloc: Allocator) void {
        self.state.deinit(alloc);
        alloc.free(self.revision);
        self.* = undefined;
    }
};

pub const Metadata = struct {
    id: []u8,
    updated_at_ms: i64,

    pub fn deinit(self: *Metadata, alloc: Allocator) void {
        alloc.free(self.id);
        self.* = undefined;
    }
};

extern "fx" fn fx_session_load(
    id_ptr: [*]const u8,
    id_len: usize,
    bytes_ptr: [*]u8,
    bytes_cap: usize,
    revision_ptr: [*]u8,
    revision_cap: usize,
    revision_len_out: *usize,
) i32;

extern "fx" fn fx_session_commit(
    id_ptr: [*]const u8,
    id_len: usize,
    bytes_ptr: [*]const u8,
    bytes_len: usize,
    expected_revision_ptr: [*]const u8,
    expected_revision_len: usize,
    revision_ptr: [*]u8,
    revision_cap: usize,
    revision_len_out: *usize,
) i32;

extern "fx" fn fx_session_list(
    response_ptr: [*]u8,
    response_cap: usize,
) i32;

extern "fx" fn fx_session_remove(
    id_ptr: [*]const u8,
    id_len: usize,
) i32;

pub fn load(alloc: Allocator, id: []const u8) !?Loaded {
    var encoded_capacity = initial_session_bytes;
    var revision_capacity = initial_revision_bytes;
    while (true) {
        const encoded = try alloc.alloc(u8, encoded_capacity);
        defer alloc.free(encoded);
        const revision_buffer = try alloc.alloc(u8, revision_capacity);
        defer alloc.free(revision_buffer);
        var revision_len: usize = 0;
        const encoded_len = fx_session_load(
            id.ptr,
            id.len,
            encoded.ptr,
            encoded.len,
            revision_buffer.ptr,
            revision_buffer.len,
            &revision_len,
        );
        if (encoded_len == -2) return null;
        if (encoded_len == -3) {
            const next_encoded = nextCapacity(encoded_capacity, max_session_bytes);
            const next_revision = nextCapacity(revision_capacity, max_revision_bytes);
            if (next_encoded == null and next_revision == null) return error.SessionTooLarge;
            encoded_capacity = next_encoded orelse encoded_capacity;
            revision_capacity = next_revision orelse revision_capacity;
            continue;
        }
        if (encoded_len < 0 or @as(usize, @intCast(encoded_len)) > encoded.len or revision_len > revision_buffer.len) {
            return error.SessionStoreUnavailable;
        }
        var source = std.Io.Reader.fixed(encoded[0..@intCast(encoded_len)]);
        var state = try session_codec.decodeState(alloc, &source, .{
            .max_history_turns = 100_000,
            .max_value_bytes = max_session_bytes,
        });
        errdefer state.deinit(alloc);
        if (!std.mem.eql(u8, state.id, id)) return error.InvalidSessionId;
        return .{
            .state = state,
            .revision = try alloc.dupe(u8, revision_buffer[0..revision_len]),
        };
    }
}

pub fn commit(
    alloc: Allocator,
    state: session_codec.DurableSessionState,
    expected_revision: ?[]const u8,
) ![]u8 {
    var encoded: std.Io.Writer.Allocating = .init(alloc);
    defer encoded.deinit();
    _ = try session_codec.encodeState(state, &encoded.writer);
    if (encoded.written().len > max_session_bytes) return error.SessionTooLarge;

    const revision_buffer = try alloc.alloc(u8, max_revision_bytes);
    defer alloc.free(revision_buffer);
    var revision_len: usize = 0;
    const expected = expected_revision orelse "";
    const status = fx_session_commit(
        state.id.ptr,
        state.id.len,
        encoded.written().ptr,
        encoded.written().len,
        expected.ptr,
        expected.len,
        revision_buffer.ptr,
        revision_buffer.len,
        &revision_len,
    );
    if (status == -2) return error.SessionRevisionConflict;
    if (status < 0 or revision_len > revision_buffer.len) return error.SessionStoreUnavailable;
    return alloc.dupe(u8, revision_buffer[0..revision_len]);
}

pub fn list(alloc: Allocator) ![]Metadata {
    var capacity = initial_list_bytes;
    while (true) {
        const response = try alloc.alloc(u8, capacity);
        defer alloc.free(response);
        const response_len = fx_session_list(response.ptr, response.len);
        if (response_len == -2) {
            capacity = nextCapacity(capacity, max_list_bytes) orelse return error.SessionListTooLarge;
            continue;
        }
        if (response_len < 0 or @as(usize, @intCast(response_len)) > response.len) return error.SessionStoreUnavailable;

        const parsed = try std.json.parseFromSlice(std.json.Value, alloc, response[0..@intCast(response_len)], .{});
        defer parsed.deinit();
        if (parsed.value != .array) return error.InvalidSessionList;
        const entries = try alloc.alloc(Metadata, parsed.value.array.items.len);
        var initialized: usize = 0;
        errdefer {
            for (entries[0..initialized]) |*entry| entry.deinit(alloc);
            alloc.free(entries);
        }
        for (parsed.value.array.items) |value| {
            if (value != .object) return error.InvalidSessionList;
            const id_value = value.object.get("id") orelse return error.InvalidSessionList;
            const updated_value = value.object.get("updatedAtMs") orelse return error.InvalidSessionList;
            if (id_value != .string or updated_value != .integer) return error.InvalidSessionList;
            entries[initialized] = .{
                .id = try alloc.dupe(u8, id_value.string),
                .updated_at_ms = updated_value.integer,
            };
            initialized += 1;
        }
        return entries;
    }
}

pub fn remove(id: []const u8) !void {
    const status = fx_session_remove(id.ptr, id.len);
    if (status < 0) return error.SessionStoreUnavailable;
}

fn nextCapacity(current: usize, maximum: usize) ?usize {
    if (current >= maximum) return null;
    return @min(current * 2, maximum);
}

test "nextCapacity grows to a bounded maximum" {
    try std.testing.expectEqual(@as(?usize, 8192), nextCapacity(4096, 16 * 1024));
    try std.testing.expectEqual(@as(?usize, 16 * 1024), nextCapacity(12 * 1024, 16 * 1024));
    try std.testing.expectEqual(@as(?usize, null), nextCapacity(16 * 1024, 16 * 1024));
}
