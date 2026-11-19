//! Summarize a stream of application events.

const std = @import("std");

const Kind = enum {
    trace,
    info,
    warning,
    err,
};

const Event = struct {
    kind: Kind,
    message: []const u8,

    fn meets(self: Event, minimum: Kind) bool {
        return @intFromEnum(self.kind) >= @intFromEnum(minimum);
    }
};

const startup = [_]Event{
    .{ .kind = .trace, .message = "starting" },
    .{ .kind = .info, .message = "ready" },
    .{ .kind = .warning, .message = "retry" },
    .{ .kind = .err, .message = "timeout" },
};

const Summary = struct {
    count: usize = 0,
    warnings: usize = 0,
    errors: usize = 0,
};

/// Count events at or above the requested level.
fn summarize(events: []const Event, minimum: Kind) Summary {
    var summary: Summary = .{};
    for (events) |event| {
        if (!event.meets(minimum)) continue;
        summary.count += 1;
        switch (event.kind) {
            .trace, .info => {},
            .warning => summary.warnings += 1,
            .err => summary.errors += 1,
        }
    }
    return summary;
}

fn find(events: []const Event, message: []const u8) ?Event {
    for (events) |event| {
        if (std.mem.eql(u8, event.message, message)) return event;
    }
    return null;
}

test "summarize startup events" {
    const summary = summarize(&startup, .info);
    try std.testing.expectEqual(@as(usize, 3), summary.count);
    try std.testing.expectEqual(@as(usize, 1), summary.warnings);
    try std.testing.expectEqual(@as(usize, 1), summary.errors);

    if (find(&startup, "ready")) |event| {
        try std.testing.expectEqual(Kind.info, event.kind);
    } else {
        return error.MissingEvent;
    }
}
