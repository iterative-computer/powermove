const permission_request = @import("../../core/permissions/permission_request.zig");
const approval_prompt = @import("../../core/permissions/approval_prompt.zig");
const interaction_state = @import("interaction_state.zig");
const resize_runtime = @import("../resize_runtime.zig");

const ApprovalProjection = approval_prompt.Projection;

pub const SettledFileApproval = struct {
    request_id: u64,
    file_request: permission_request.FileApprovalRequest,
    rows: u16,
    cols: u16,
};

pub fn settledFileApproval(
    shell: anytype,
    approval: ApprovalProjection,
) ?SettledFileApproval {
    const Shell = @TypeOf(shell.*);
    if (comptime !(@hasField(Shell, "render_requests") and
        @hasField(Shell, "layout") and
        @hasField(Shell, "terminal_dimensions_invalid") and
        @hasField(Shell, "pending_resize_observation")))
    {
        return null;
    } else {
        const request = approval.request;
        const file_request = request.file orelse return null;
        if (!shell.render_requests.settledForApproval()) return null;
        if (!resize_runtime.lifecycleIdle(shell)) return null;
        if (shell.layout.rows == 0 or shell.layout.cols == 0) return null;
        return .{
            .request_id = request.id,
            .file_request = file_request,
            .rows = shell.layout.rows,
            .cols = shell.layout.cols,
        };
    }
}

pub fn committedFooterApprovalRows(shell: anytype) ?u16 {
    const Shell = @TypeOf(shell.*);
    if (comptime !(@hasField(Shell, "layout") and
        @hasField(Shell, "footer_viewport")))
    {
        return null;
    } else {
        if (!shell.footer_viewport.has_frame or
            shell.footer_viewport.externally_invalidated)
        {
            return null;
        }

        const geometry = shell.footer_viewport.geometry;
        if (geometry.top_divider == 0 or
            geometry.hint < geometry.top_divider or
            geometry.hint > shell.layout.rows)
        {
            return null;
        }
        return geometry.hint - geometry.top_divider + 1;
    }
}

pub fn screenCommitSupportsAffirmative(
    settled: SettledFileApproval,
    screen_state: *const interaction_state.ApprovalScreenState,
) bool {
    const commit = screen_state.screen_commit orelse return false;
    if (commit.request_id != settled.request_id) return false;
    if (commit.rows != settled.rows or commit.cols != settled.cols) return false;
    return commit.file_identity_visible and
        commit.all_decision_controls_visible and
        (commit.changed_or_notice_visible or screen_state.changed_or_notice_observed);
}
