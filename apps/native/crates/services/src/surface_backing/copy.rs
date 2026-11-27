//! Exact Opus copy for model states. No views or visual-token substitutes.
//! Authority: docs/native/design/surfaces/{mixer-3d,workspaces,takes-notes-mods,
//! onboarding,updates,diagnostics,migration,colour,tasks-storage,os-authorization}.md.
use super::{authorization::Authorization, maintenance::UpdateState, projects::NoteDraft};
use crate::ServiceErrorCode;

pub fn workspace_json_error(parser: &str) -> String {
    format!("This isn’t valid JSON. {parser}")
}
pub fn workspace_definition_error(validator: &str) -> String {
    format!("This workspace can’t be used. {validator}")
}
pub fn workspace_name_exists(name: &str) -> String {
    format!("A workspace named “{name}” already exists.")
}
pub fn generated_missing_binding(property: bool) -> &'static str {
    if property {
        "This control’s property isn’t available."
    } else {
        "This control’s layer isn’t in the composition."
    }
}
pub fn generated_failure(label: &str) -> String {
    format!("{label} couldn’t finish.")
}
pub fn take_saved(label: Option<&str>, oldest_archived: usize) -> String {
    let mut status = label
        .map(|label| format!("Saved {label}"))
        .unwrap_or_else(|| "Take saved".into());
    if oldest_archived > 0 {
        status.push_str(" · Oldest take archived");
    }
    status
}
pub fn take_restored(label: &str) -> String {
    format!("Restored {label}")
}
pub fn take_deleted(label: &str) -> String {
    format!("Deleted {label}")
}
pub fn take_restore_failed(label: &str) -> String {
    format!("Couldn’t restore {label}.")
}
pub fn notes_status(draft: &NoteDraft, can_release: bool) -> &'static str {
    if draft.read_only {
        "This project is read-only."
    } else if draft.failure.is_some() {
        "Couldn’t save notes. They’ll stay here until the next save works."
    } else if !can_release {
        "Saving notes…"
    } else {
        "Notes saved"
    }
}
pub fn mods_summary(count: usize, attention: usize) -> String {
    let mut s = format!("{count} mods");
    if attention > 0 {
        s.push_str(&format!(" · {attention} needs attention"));
    }
    s
}
pub fn update_status(state: &UpdateState, current: &str, manual: bool) -> Option<String> {
    match state {
        UpdateState::Checking if manual => Some("Checking for updates…".into()),
        UpdateState::Current if manual => Some(format!("You’re running {current}.")),
        UpdateState::Downloading(metadata) => Some(format!("Downloading {}…", metadata.name)),
        UpdateState::Verifying(_) => Some("Verifying…".into()),
        UpdateState::Ready { metadata, .. } => Some(format!(
            "Powermove {} is ready to install",
            metadata.version
        )),
        UpdateState::Failed(ServiceErrorCode::Offline) if manual => {
            Some("Check your internet connection, then try again.".into())
        }
        UpdateState::Failed(_) if manual => Some("Couldn’t check for updates.".into()),
        UpdateState::DownloadFailed(_) => Some("Couldn’t download the update.".into()),
        UpdateState::VerificationFailed(_) => {
            Some("This update couldn’t be verified, so it wasn’t installed.".into())
        }
        _ => None,
    }
}
pub const PERFORMANCE_LABELS: &[(&str, &str)] = &[
    ("frame_rate", "Frame Rate"),
    ("frame_time", "Frame Time"),
    ("draw_calls", "Draw Calls"),
    ("effect_passes", "Effect Passes"),
    ("gpu_programs", "GPU Programs"),
    ("raster_cache", "Raster Cache"),
    ("layers", "Layers"),
    ("keyframes", "Keyframes"),
];
pub const PROXY_EXPORT_WARNING: &str =
    "This uses proxy-only media, so quality may be lower. Relink the originals for full quality.";
pub const LOCAL_NETWORK_DENIED: &str = "Allow Local Network for Powermove in System Settings.";
pub fn notification_detail(status: Authorization) -> &'static str {
    if status == Authorization::Denied {
        "Notifications are off for Powermove in System Settings."
    } else {
        "Powermove tells you when your agent finishes or has a question."
    }
}
