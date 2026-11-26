const std = @import("std");

pub const ActivityPlacement = union(enum) {
    const Transient = struct {
        row: u16,
        row_count: u16 = 1,
        gap_above_rows: u16,
        footer_clearance_rows: u16 = 0,
        tool_row: ?u16 = null,
    };

    none,
    transient_row: Transient,
    overlay_entry: u16,

    pub fn reservedFooterRows(self: ActivityPlacement) u16 {
        return switch (self) {
            .transient_row => |activity| activity.footer_clearance_rows,
            .overlay_entry, .none => 0,
        };
    }

    pub fn reservesFooterRow(self: ActivityPlacement) bool {
        return self.reservedFooterRows() > 0;
    }

    pub fn reservedTopRow(self: ActivityPlacement) ?u16 {
        return switch (self) {
            .transient_row => |activity| {
                const first_row = activity.tool_row orelse activity.row;
                if (first_row <= activity.gap_above_rows) return 1;
                return first_row - activity.gap_above_rows;
            },
            .overlay_entry, .none => null,
        };
    }

    pub fn transcriptReservationRows(self: ActivityPlacement) u16 {
        return switch (self) {
            .transient_row => |activity| activity.gap_above_rows +| (activity.row -| (activity.tool_row orelse activity.row)) +| activity.row_count +| activity.footer_clearance_rows,
            .overlay_entry, .none => 0,
        };
    }

    pub fn requiredFooterTop(self: ActivityPlacement) ?u16 {
        return switch (self) {
            .transient_row => |activity| activity.row +| activity.row_count +| activity.footer_clearance_rows,
            .overlay_entry, .none => null,
        };
    }

    pub fn row(self: ActivityPlacement) ?u16 {
        return switch (self) {
            .transient_row => |activity| activity.row,
            .overlay_entry => |r| r,
            .none => null,
        };
    }

    pub fn primaryRowCount(self: ActivityPlacement) u16 {
        return switch (self) {
            .transient_row => |activity| activity.row_count,
            .overlay_entry => 1,
            .none => 0,
        };
    }

    pub fn toolRow(self: ActivityPlacement) ?u16 {
        return switch (self) {
            .transient_row => |activity| activity.tool_row,
            .overlay_entry, .none => null,
        };
    }
};

pub const Input = struct {
    active_label_present: bool,
    activity_rows: u16 = 1,
    tool_before_activity: bool = false,
    cursor_row: u16,
    cursor_col: u16,
    transient_gap_rows: u16,
    place_mid_line_active: bool,
    entry_bound_active: bool,
    stream_active: bool,
    footer_top_for_extra: u16,
    footer_clearance_rows: u16 = 0,
};

pub fn resolve(input: Input) ActivityPlacement {
    const activity_rows = @max(input.activity_rows, 1);
    if (input.tool_before_activity) {
        const tool_row = if (input.cursor_col == 1)
            input.cursor_row +| input.transient_gap_rows
        else if (input.place_mid_line_active)
            input.cursor_row +| input.transient_gap_rows +| 1
        else
            return .none;
        return .{ .transient_row = .{
            .row = tool_row +| 2,
            .row_count = activity_rows,
            .gap_above_rows = input.transient_gap_rows,
            .footer_clearance_rows = input.footer_clearance_rows,
            .tool_row = tool_row,
        } };
    }

    if (input.active_label_present) {
        const gap_rows = input.transient_gap_rows;
        if (input.cursor_col == 1) {
            return .{ .transient_row = .{
                .row = input.cursor_row +| gap_rows,
                .row_count = activity_rows,
                .gap_above_rows = gap_rows,
                .footer_clearance_rows = input.footer_clearance_rows,
            } };
        }
        if (input.place_mid_line_active) {
            return .{ .transient_row = .{
                .row = input.cursor_row +| gap_rows +| 1,
                .row_count = activity_rows,
                .gap_above_rows = gap_rows,
                .footer_clearance_rows = input.footer_clearance_rows,
            } };
        }
    }

    if (!input.active_label_present and input.entry_bound_active and input.stream_active) {
        if (input.cursor_col == 1) {
            return .{ .transient_row = .{ .row = input.cursor_row, .gap_above_rows = 0, .footer_clearance_rows = input.footer_clearance_rows } };
        }
        const sticky_row = if (input.footer_top_for_extra > 1) input.footer_top_for_extra - 1 else input.footer_top_for_extra;
        return .{ .transient_row = .{ .row = sticky_row, .gap_above_rows = 0, .footer_clearance_rows = input.footer_clearance_rows } };
    }

    return .none;
}

pub fn suppressEntryOverlayForBanner(activity: *ActivityPlacement, banner_active: bool) void {
    if (!banner_active) return;
    switch (activity.*) {
        .overlay_entry => {
            activity.* = .none;
        },
        .none, .transient_row => {},
    }
}

test "activity placement reserves row only for thinking shimmer" {
    const thinking = resolve(.{
        .active_label_present = true,
        .cursor_row = 5,
        .cursor_col = 1,
        .transient_gap_rows = 0,
        .place_mid_line_active = false,
        .entry_bound_active = false,
        .stream_active = false,
        .footer_top_for_extra = 9,
    });
    try std.testing.expect(!thinking.reservesFooterRow());
    try std.testing.expectEqual(@as(u16, 0), thinking.reservedFooterRows());
    try std.testing.expectEqual(@as(u16, 1), thinking.transcriptReservationRows());
    try std.testing.expectEqual(@as(u16, 5), thinking.row().?);

    const none = resolve(.{
        .active_label_present = true,
        .cursor_row = 5,
        .cursor_col = 3,
        .transient_gap_rows = 0,
        .place_mid_line_active = false,
        .entry_bound_active = false,
        .stream_active = false,
        .footer_top_for_extra = 9,
    });
    try std.testing.expect(!none.reservesFooterRow());
    try std.testing.expectEqual(@as(u16, 0), none.reservedFooterRows());
    try std.testing.expectEqual(@as(u16, 0), none.transcriptReservationRows());
    try std.testing.expectEqual(@as(?u16, null), none.row());
}

test "transient activity tracks leading gap separately from footer clearance" {
    const activity = resolve(.{
        .active_label_present = true,
        .cursor_row = 3,
        .cursor_col = 1,
        .transient_gap_rows = 1,
        .place_mid_line_active = false,
        .entry_bound_active = false,
        .stream_active = false,
        .footer_top_for_extra = 6,
    });

    try std.testing.expect(!activity.reservesFooterRow());
    try std.testing.expectEqual(@as(u16, 4), activity.row().?);
    try std.testing.expectEqual(@as(?u16, 3), activity.reservedTopRow());
    try std.testing.expectEqual(@as(u16, 2), activity.transcriptReservationRows());
    try std.testing.expect(activity.row().? < 6);
}

test "transient activity does not count its leading gap as footer clearance" {
    const activity = resolve(.{
        .active_label_present = true,
        .cursor_row = 10,
        .cursor_col = 1,
        .transient_gap_rows = 1,
        .place_mid_line_active = false,
        .entry_bound_active = false,
        .stream_active = false,
        .footer_top_for_extra = 12,
    });

    try std.testing.expectEqual(@as(u16, 11), activity.row().?);
    try std.testing.expectEqual(@as(u16, 0), activity.reservedFooterRows());
    try std.testing.expectEqual(@as(?u16, 10), activity.reservedTopRow());
}

test "mid-line transient activity reserves gap plus status row only" {
    const activity = resolve(.{
        .active_label_present = true,
        .cursor_row = 5,
        .cursor_col = 12,
        .transient_gap_rows = 1,
        .place_mid_line_active = true,
        .entry_bound_active = false,
        .stream_active = false,
        .footer_top_for_extra = 6,
    });

    switch (activity) {
        .transient_row => |transient| {
            try std.testing.expectEqual(@as(u16, 7), transient.row);
            try std.testing.expectEqual(@as(u16, 1), transient.gap_above_rows);
            try std.testing.expectEqual(@as(?u16, 6), activity.reservedTopRow());
            try std.testing.expectEqual(@as(u16, 2), activity.transcriptReservationRows());
        },
        .none, .overlay_entry => return error.TestUnexpectedResult,
    }
}

test "active entry remains in the transient activity band" {
    const activity = resolve(.{
        .active_label_present = false,
        .cursor_row = 9,
        .cursor_col = 1,
        .transient_gap_rows = 0,
        .place_mid_line_active = false,
        .entry_bound_active = true,
        .stream_active = true,
        .footer_top_for_extra = 10,
    });

    try std.testing.expectEqual(@as(u16, 9), activity.row().?);
    try std.testing.expectEqual(@as(u16, 0), activity.reservedFooterRows());
    switch (activity) {
        .transient_row => |transient| {
            try std.testing.expectEqual(@as(u16, 9), transient.row);
            try std.testing.expectEqual(@as(u16, 0), transient.gap_above_rows);
        },
        .none, .overlay_entry => return error.TestUnexpectedResult,
    }
}

test "banner suppression converts entry overlay to none" {
    var activity = ActivityPlacement{ .overlay_entry = 7 };
    suppressEntryOverlayForBanner(&activity, true);

    switch (activity) {
        .none => {},
        .overlay_entry, .transient_row => return error.TestUnexpectedResult,
    }
}
