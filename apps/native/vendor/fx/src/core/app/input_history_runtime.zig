const std = @import("std");
const prompt_history_store = @import("../session/prompt_history_store.zig");

pub fn HistoryRuntime(comptime App: type) type {
    return struct {
        pub fn installPromptHistory(
            app: *App,
            entries: []const prompt_history_store.LoadedPromptHistoryEntry,
        ) !void {
            var texts: std.ArrayList([]const u8) = .empty;
            defer texts.deinit(app.alloc);
            try texts.ensureTotalCapacity(app.alloc, entries.len);
            for (entries) |entry| texts.appendAssumeCapacity(entry.text);
            try app.input_runtime.composer_history.installTextEntries(
                app.alloc,
                texts.items,
            );
        }

        pub fn loadAndInstallPromptHistory(app: *App) !void {
            const entries = try app.prompt_history.loadRecent(
                app.alloc,
                app.workspace_root,
                100,
            );
            defer prompt_history_store.freeLoadedEntries(app.alloc, entries);
            try installPromptHistory(app, entries);
        }
    };
}
