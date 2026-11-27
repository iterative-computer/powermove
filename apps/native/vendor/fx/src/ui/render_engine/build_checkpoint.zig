const std = @import("std");

const work_quota: usize = 4096;

pub const BuildCheckpoint = struct {
    context: *anyopaque,
    input_pending: *const fn (context: *anyopaque) bool,
    remaining: usize = work_quota,

    pub fn init(
        context: *anyopaque,
        input_pending: *const fn (context: *anyopaque) bool,
    ) BuildCheckpoint {
        return .{ .context = context, .input_pending = input_pending };
    }

    fn tick(self: *BuildCheckpoint) error{InputPending}!void {
        try self.consume(1);
    }

    fn consume(self: *BuildCheckpoint, units: usize) error{InputPending}!void {
        if (units < self.remaining) {
            self.remaining -= units;
            return;
        }
        const overflow = units - self.remaining;
        const used_in_next_quota = overflow % work_quota;
        self.remaining = if (used_in_next_quota == 0)
            work_quota
        else
            work_quota - used_in_next_quota;
        try self.poll();
    }

    fn poll(self: *BuildCheckpoint) error{InputPending}!void {
        if (self.input_pending(self.context)) return error.InputPending;
    }
};

pub fn tick(checkpoint: ?*BuildCheckpoint) !void {
    if (checkpoint) |active| try active.tick();
}

pub fn consume(checkpoint: ?*BuildCheckpoint, units: usize) !void {
    if (checkpoint) |active| try active.consume(units);
}

pub fn poll(checkpoint: ?*BuildCheckpoint) !void {
    if (checkpoint) |active| try active.poll();
}

test "checkpoint polls only when its quota is exhausted" {
    const Probe = struct {
        polls: usize = 0,

        fn pending(context: *anyopaque) bool {
            const self: *@This() = @ptrCast(@alignCast(context));
            self.polls += 1;
            return false;
        }
    };

    var probe = Probe{};
    var checkpoint = BuildCheckpoint.init(&probe, Probe.pending);
    try consume(&checkpoint, work_quota - 1);
    try std.testing.expectEqual(@as(usize, 0), probe.polls);
    try tick(&checkpoint);
    try std.testing.expectEqual(@as(usize, 1), probe.polls);
    try consume(&checkpoint, work_quota * 2 + 7);
    try std.testing.expectEqual(@as(usize, 2), probe.polls);
    try consume(&checkpoint, work_quota - 8);
    try std.testing.expectEqual(@as(usize, 2), probe.polls);
    try tick(&checkpoint);
    try std.testing.expectEqual(@as(usize, 3), probe.polls);
    try consume(null, work_quota);
}
