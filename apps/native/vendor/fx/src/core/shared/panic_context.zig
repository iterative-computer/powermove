//! Powermove notice: new file, not in upstream fx (patch P-PANIC). Which
//! runtime a core-owned thread works for, so the C ABI root's panic handler
//! (src/capi_core_main.zig) can fail that runtime alone and park the thread
//! instead of aborting the process. A thread the core spawns for a runtime
//! inherits its parent's context through `spawn` and `bound`.
const std = @import("std");

/// The runtime this thread works for, or null on threads the core does not
/// own (the host's). Opaque here: only the C entry knows its type.
pub threadlocal var current: ?*anyopaque = null;

fn ReturnOf(comptime function: anytype) type {
    return @typeInfo(@TypeOf(function)).@"fn".return_type.?;
}

/// `std.Thread.spawn` whose thread starts in the caller's panic context.
pub fn spawn(config: std.Thread.SpawnConfig, comptime function: anytype, args: anytype) std.Thread.SpawnError!std.Thread {
    const Args = @TypeOf(args);
    const Run = struct {
        fn run(context: ?*anyopaque, inner: Args) ReturnOf(function) {
            current = context;
            return @call(.auto, function, inner);
        }
    };
    return std.Thread.spawn(config, Run.run, .{ current, args });
}

/// `function` taking the panic context as its first argument, for tasks
/// that run on a shared pool thread (`std.Io.concurrent`): the task sets the
/// context while it runs and restores the pool thread's afterwards. Call as
/// `io.concurrent(bound(f), .{ panic_context.current, args... })`.
pub fn bound(comptime function: anytype) BoundFn(function) {
    return Bound(function).run;
}

fn BoundFn(comptime function: anytype) type {
    return @TypeOf(Bound(function).run);
}

fn Bound(comptime function: anytype) type {
    const params = @typeInfo(@TypeOf(function)).@"fn".param_types;
    return switch (params.len) {
        1 => struct {
            fn run(context: ?*anyopaque, a: params[0].?) ReturnOf(function) {
                const saved = current;
                current = context;
                defer current = saved;
                return function(a);
            }
        },
        else => @compileError("panic_context.bound: add an arity"),
    };
}
