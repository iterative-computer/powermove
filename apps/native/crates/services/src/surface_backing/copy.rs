//! Exact Opus copy for model states. No views or visual-token substitutes.
//! Authority: docs/native/design/surfaces/{mixer-3d,workspaces,takes-notes-mods,
//! onboarding,updates,diagnostics,migration,colour,tasks-storage,os-authorization}.md.
use super::{authorization::Authorization, maintenance::UpdateState, projects::NoteDraft};
use crate::ServiceErrorCode;
use pm_promedia::tasks::{TaskFailureCode, TaskKind, TaskState};

pub const MIXER_EMPTY_TITLE: &str = "No Audio";
pub const MIXER_EMPTY_LINE: &str = "No audio in this composition";
pub const MIXER_EDIT_FAILED: &str = "Couldn’t change the level";
pub const SCENE_LOAD_FAILED: &str = "This 3D layer couldn’t be loaded.";
pub const SCENE_WRONG_MODEL: &str = "Choose an OBJ, GLB or glTF model.";
pub const PANEL_LIBRARY_TITLE: &str = "Panel Library";
pub const WORKSPACE_CHANGED: &str =
    "This workspace changed while you were editing. Close and reopen to see the latest version.";
pub const WORKSPACE_TRANSFER_FAILED: &str = "Couldn’t start the workspace transfer. Try again.";
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
pub const GENERATED_INVALID: &str = "Its definition isn’t valid.";
pub const GENERATED_NO_CHANGE: &str = "The tool couldn’t produce a valid source change.";
pub fn generated_failure(label: &str) -> String {
    format!("{label} couldn’t finish.")
}
pub const TAKES_EMPTY_TITLE: &str = "No Takes";
pub const TAKES_EMPTY_LINE: &str = "Save a take before exploring a new motion direction.";
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
pub const TAKE_SAVE_FAILED: &str = "Couldn’t save this take.";
pub const NOTES_PLACEHOLDER: &str =
    "Direction notes — saved with this project as its creative brief.";
pub const NOTES_NO_PROJECT: &str = "Open a project to write its notes.";
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
pub const ONBOARDING_WELCOME: &str = "Welcome to Powermove.";
pub const ONBOARDING_TIMELINE: &str = "Pick Your Timeline.";
pub const ONBOARDING_AGENT: &str = "Pick Your Agent.";
pub const ONBOARDING_SPEECH: &str = "Pick Your Speech Model.";
pub const ONBOARDING_WORKSPACE: &str = "Grab Your Suitcase.";
pub const ONBOARDING_FAILED: &str = "Couldn’t open your workspace. Try again, or start fresh.";
pub const SPEECH_UNAVAILABLE: &str = "Speech models can be downloaded later from Settings.";
pub const SPEECH_START_FAILED: &str =
    "Couldn’t start the download. You can download it later in Settings.";
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
pub const UPDATE_NOTES_FALLBACK: &str = "Fixes and improvements.";
pub const PERFORMANCE_NOT_AVAILABLE: &str = "Not available on this Mac.";
pub const PERFORMANCE_NO_PROJECT: &str = "Open a project to see its performance.";
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
pub const GO_BACK_TITLE: &str = "Your original app still has everything from before.";
pub const GO_BACK_LINE: &str = "Work you did in Powermove Native stays here. Save a project as a .pmv file to open it in the original app.";
pub fn task_failure(code: TaskFailureCode) -> Option<&'static str> {
    match code {
        TaskFailureCode::SourceOffline => Some("The source is offline. Relink it and try again."),
        TaskFailureCode::DiskFull => Some("Not enough disk space. Free some space and try again."),
        TaskFailureCode::CodecUnsupported => Some("This format isn’t supported."),
        TaskFailureCode::Cancelled => None,
        TaskFailureCode::Unknown => {
            Some("Something went wrong. Details are in Window ▸ Error Log.")
        }
    }
}
pub fn task_detail(state: TaskState, kind: &TaskKind) -> &'static str {
    match state {
        TaskState::Queued => "Waiting…",
        TaskState::Running => "Working…",
        TaskState::PauseRequested => "Pausing…",
        TaskState::Paused => "Paused",
        TaskState::CancelRequested => "Cancelling…",
        TaskState::Cancelled => "Cancelled",
        TaskState::Failed => "",
        TaskState::Completed => match kind {
            TaskKind::Export => "Exported · Show in Finder",
            TaskKind::Render => "Rendered",
            TaskKind::Import => "Imported",
            TaskKind::Proxy | TaskKind::Optimized => "Created",
            TaskKind::Purge => "Freed",
        },
    }
}
pub const TASKS_EMPTY_TITLE: &str = "No Background Tasks";
pub const TASKS_EMPTY_LINE: &str = "Exports, renders and proxy media appear here.";
pub const STORAGE_ARCHIVE_LINE: &str =
    "Powermove moves media that nothing uses into your archive folder. It’s never deleted.";
pub const STORAGE_SAME_DISK: &str = "The archive is on the same disk, so this won’t free disk space. Choose another location for the archive to free it.";
pub const SAVE_DENIED: &str =
    "Powermove doesn’t have permission to save here. Choose another location.";
pub const LOCAL_NETWORK_DENIED: &str = "Allow Local Network for Powermove in System Settings.";
pub fn notification_detail(status: Authorization) -> &'static str {
    if status == Authorization::Denied {
        "Notifications are off for Powermove in System Settings."
    } else {
        "Powermove tells you when your agent finishes or has a question."
    }
}
