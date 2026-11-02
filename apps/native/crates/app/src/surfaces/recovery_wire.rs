//! v4 worker data; Rust owns copy and state. No diagnostic fields are published.
use super::recovery::panels::Content;
use serde_json::{Value, json};
pub fn panel(content: &Content) -> Value {
    match content {
        Content::Unchanged => json!({"kind":"unchanged"}),
        Content::Hidden => json!({"kind":"hidden"}),
        Content::Loading { line } => json!({"kind":"loading", "line":line}),
        Content::EmptyState {
            title,
            line,
            button,
        } => json!({
            "kind":"empty", "title":title, "line":line,
            "button":button.as_ref().map(|b| b.label),
            "axLabel":content.ax_label(), "announcement":"high"
        }),
        Content::DesignTodo(note) => json!({"kind":"design-todo", "note":note}),
    }
}
/// Stable serialized boundary for pm-store-host (not yet a direct app dependency).
/// Callers serialize ViewStatus and the last Failure from lifecycle events. The
/// source-contract tests exercise the actual crate enums, not duplicated fixtures.
pub fn view_status(value: &Value) -> Option<super::recovery::panels::ViewStatus> {
    use super::recovery::panels::ViewStatus::*;
    Some(match value.as_str()? {
        "Starting" => Starting,
        "Ready" => Ready,
        "Sealing" => Sealing,
        "Stopping" => Stopping,
        "StopFailed" => StopFailed,
        "WaitingToRestart" => WaitingToRestart,
        "Disabled" => Disabled,
        "Closed" => Closed,
        _ => return None,
    })
}
pub fn view_failure(value: &Value) -> Option<super::recovery::panels::Failure> {
    use super::recovery::panels::Failure::*;
    Some(match value.as_str()? {
        "Crash" => Crash,
        "Hang" => Hang,
        "UserStop" => UserStop,
        "ReadyTimeout" => ReadyTimeout,
        "PlatformFailure" => PlatformFailure,
        _ => return None,
    })
}
pub fn service_code(value: &Value) -> Option<super::recovery::notices::ServiceCode> {
    use super::recovery::notices::ServiceCode::*;
    Some(match value.as_str()? {
        "offline" => Offline,
        "not_found" => NotFound,
        "too_large" => TooLarge,
        "invalid_project" => InvalidProject,
        "permission_denied" => PermissionDenied,
        "disk_full" => DiskFull,
        "invalid_input" => InvalidInput,
        "invalid_data" => InvalidData,
        "unavailable" => Unavailable,
        "unknown" => Unknown,
        _ => return None,
    })
}
/// CrashRecord names are local display data, never diagnostic exports. The
/// kernel owns the actual truncation fact. An unrecovered record is not a notice.
pub fn crash_banner(record: &Value, truncated: bool) -> Option<super::recovery::notices::Banner> {
    if !record.get("recovered")?.as_bool()? {
        return None;
    }
    let name = record.get("recovered_document")?.as_str()?;
    Some(super::recovery::notices::Banner::recovered(name, truncated))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::recovery::panels::*;
    #[test]
    fn empty_state_wire_retains_exact_ax_copy_and_only_one_action() {
        let content = PanelSnapshot {
            panel: "Tools",
            extension: "Example",
            status: ViewStatus::WaitingToRestart,
            failure: Some(Failure::Hang),
            elapsed: std::time::Duration::ZERO,
            realm_prompt_open: false,
        }
        .content();
        let wire = panel(&content);
        assert_eq!(wire["title"], "Tools Stopped Responding");
        assert_eq!(
            wire["line"],
            "Powermove stopped it and will restart it in a moment."
        );
        assert_eq!(wire["button"], "Restart Now");
        assert_eq!(
            wire["axLabel"],
            "Tools Stopped Responding. Powermove stopped it and will restart it in a moment."
        );
        assert_eq!(wire["announcement"], "high");
    }
    #[test]
    fn non_error_states_have_no_recovery_button() {
        for content in [
            Content::Unchanged,
            Content::Hidden,
            Content::Loading {
                line: "Starting Example…".into(),
            },
            Content::DesignTodo("DESIGN-TODO(opus): missing spec"),
        ] {
            let wire = panel(&content);
            assert!(wire.get("button").is_none());
        }
    }
}
