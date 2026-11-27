const std = @import("std");

pub const url = "https://fx.sh/feedback";

test "feedback URL stays on the fx.sh domain" {
    try std.testing.expectEqualStrings("https://fx.sh/feedback", url);
}
