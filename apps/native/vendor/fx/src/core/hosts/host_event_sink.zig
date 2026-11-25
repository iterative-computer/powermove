//! Powermove notice: new file, not in upstream fx (patch P-EVENTS). A typed
//! sink the embedder installs in `acp_runner.Config`, so the session updates
//! a turn streams (reply and reasoning text, tool cards, context usage, a
//! joined steer) reach the host as values at the point the ACP server would
//! build a `session/update` frame, instead of as JSON it formats and the host
//! parses. Updates the sink does not cover still go out as ACP frames.
const std = @import("std");

pub const TextKind = enum { reply, thought };

pub const Error = error{OutOfMemory};

pub const Sink = struct {
    context: *anyopaque,
    /// A reply or reasoning delta, borrowed for the call.
    text_fn: *const fn (*anyopaque, TextKind, []const u8) Error!void,
    /// A tool card opened: the call's ACP id and the tool's name.
    tool_start_fn: *const fn (*anyopaque, []const u8, []const u8) Error!void,
    /// A tool card's terminal update.
    tool_end_fn: *const fn (*anyopaque, []const u8, bool) Error!void,
    /// The session's context use and size.
    usage_fn: *const fn (*anyopaque, u64, u64) Error!void,
    /// A steer joined the turn: its text and the host's input id.
    steer_joined_fn: *const fn (*anyopaque, []const u8, ?[]const u8) Error!void,

    pub fn text(self: Sink, kind: TextKind, bytes: []const u8) Error!void {
        return self.text_fn(self.context, kind, bytes);
    }

    pub fn toolStart(self: Sink, call_id: []const u8, name: []const u8) Error!void {
        return self.tool_start_fn(self.context, call_id, name);
    }

    pub fn toolEnd(self: Sink, call_id: []const u8, failed: bool) Error!void {
        return self.tool_end_fn(self.context, call_id, failed);
    }

    pub fn usage(self: Sink, used: u64, size: u64) Error!void {
        return self.usage_fn(self.context, used, size);
    }

    pub fn steerJoined(self: Sink, bytes: []const u8, input_id: ?[]const u8) Error!void {
        return self.steer_joined_fn(self.context, bytes, input_id);
    }
};
