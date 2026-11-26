//! Powermove notice: new file, not in upstream fx (patches P-STOP and
//! P-PRETOOL). An embedder registers its own lifecycle hook handlers (the
//! stop gate and pre_tool) on the server's hook runtime before it is frozen,
//! so its handlers run where fx's own Stop and PreToolUse hooks do.
const runtime = @import("runtime.zig");

pub const Registrar = struct {
    context: *anyopaque,
    register_fn: *const fn (*anyopaque, *runtime.Runtime) anyerror!void,

    pub fn register(self: Registrar, hooks: *runtime.Runtime) !void {
        return self.register_fn(self.context, hooks);
    }
};
