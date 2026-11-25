const types = @import("../../core/shared/types.zig");
const terminal = @import("terminal.zig");

extern "fx" fn fx_term_size(cols: *u16, rows: *u16) void;
extern "fx" fn fx_term_poll_input(timeout_ms: i32) i32;

pub const Size = struct {
    cols: u16,
    rows: u16,
};

pub fn pollInput(timeout_ms: i32) i32 {
    return fx_term_poll_input(timeout_ms);
}

pub fn querySize() !Size {
    var cols: u16 = 0;
    var rows: u16 = 0;
    fx_term_size(&cols, &rows);
    if (cols == 0 or rows == 0) return error.UnableToReadTerminalSize;
    return .{ .cols = cols, .rows = rows };
}

pub fn queryLayout(footer_rows: u16) !types.Layout {
    const size = try querySize();
    return terminal.layoutFromSize(size.rows, size.cols, footer_rows);
}
