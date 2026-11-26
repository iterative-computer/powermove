const std = @import("std");

pub fn freeStringList(alloc: std.mem.Allocator, list: *std.ArrayList([]u8)) void {
    for (list.items) |item| alloc.free(item);
    list.deinit(alloc);
}

test "freeStringList frees and deinits an owned string list" {
    const alloc = std.testing.allocator;
    var list: std.ArrayList([]u8) = .empty;
    errdefer {
        for (list.items) |item| alloc.free(item);
        list.deinit(alloc);
    }

    try list.append(alloc, try alloc.dupe(u8, "alpha"));
    try list.append(alloc, try alloc.dupe(u8, "beta"));

    freeStringList(alloc, &list);
}
