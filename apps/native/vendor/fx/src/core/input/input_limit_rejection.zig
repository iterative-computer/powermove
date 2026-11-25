const std = @import("std");
const text_scalar = @import("text_scalar.zig");

pub const State = struct {
    owner: ?text_scalar.Owner = null,
};

pub const BeginTransition = struct {
    next: State,
    should_report: bool,
};

pub fn begin(current: State, owner: text_scalar.Owner) BeginTransition {
    if (current.owner == owner) {
        return .{ .next = current, .should_report = false };
    }
    return .{
        .next = .{ .owner = owner },
        .should_report = true,
    };
}

pub fn clear() State {
    return .{};
}

test "input limit rejection coalesces only within one owner episode" {
    const composer = begin(.{}, .composer);
    try std.testing.expect(composer.should_report);
    try std.testing.expectEqual(text_scalar.Owner.composer, composer.next.owner.?);

    const repeated = begin(composer.next, .composer);
    try std.testing.expect(!repeated.should_report);
    try std.testing.expectEqual(composer.next, repeated.next);

    const question = begin(repeated.next, .question_freeform);
    try std.testing.expect(question.should_report);
    try std.testing.expectEqual(text_scalar.Owner.question_freeform, question.next.owner.?);
}

test "input limit rejection clear starts a new episode" {
    const active = begin(.{}, .approval_amendment).next;
    try std.testing.expectEqual(text_scalar.Owner.approval_amendment, active.owner.?);
    const cleared = clear();
    try std.testing.expect(cleared.owner == null);
    try std.testing.expect(begin(cleared, .approval_amendment).should_report);
}
