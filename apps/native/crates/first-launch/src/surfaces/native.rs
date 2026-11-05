//! N004–N007 DTO projections and action policies. Engine/kernel/media owners
//! compute all image/edit/storage facts. No substitute rendering or mutation.

// N004 — colour.md. Names here are existing design token ids, never RGB literals.
pub const SCOPE_CHANNEL_TOKENS: [&str; 3] = [
    "colors.scope_red",
    "colors.scope_green",
    "colors.scope_blue",
];
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    Histogram,
    Waveform,
    RgbParade,
    Vectorscope,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnalysisStage {
    WorkingColor,
    Output,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Matrix {
    Rec709,
    Rec2020,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Range {
    Standard,
    Extended,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameState {
    Ready,
    Waiting,
    Unreadable,
    NonFinite,
    NoProject,
}
#[derive(Debug)]
pub struct Scopes {
    pub scope: Option<Scope>,
    pub stage: AnalysisStage,
    pub matrix: Matrix,
    pub range: Range,
    pub visible: bool,
    pub float_project: bool,
    pub state: FrameState,
    last_sample_ms: Option<u64>,
}
impl Scopes {
    /// DESIGN-TODO(opus): the spec illustrates Waveform but does not explicitly
    /// choose an initial scope. None is a neutral placeholder until a saved or
    /// reviewed selection is supplied; no analysis starts without it.
    pub fn new(scope: Option<Scope>, rec2020: bool, float_project: bool) -> Self {
        Self {
            scope,
            stage: AnalysisStage::Output,
            matrix: if rec2020 {
                Matrix::Rec2020
            } else {
                Matrix::Rec709
            },
            range: Range::Standard,
            visible: false,
            float_project,
            state: FrameState::NoProject,
            last_sample_ms: None,
        }
    }
    pub fn extended(&mut self) -> bool {
        if !self.float_project {
            return false;
        }
        self.range = Range::Extended;
        true
    }
    pub fn request_frame(&mut self, now_ms: u64, playing: bool) -> bool {
        if !self.visible || self.scope.is_none() || self.state == FrameState::NoProject {
            return false;
        }
        if playing
            && self
                .last_sample_ms
                .is_some_and(|last| now_ms.saturating_sub(last) < 100)
        {
            return false;
        }
        self.last_sample_ms = Some(now_ms);
        true
    }
    pub fn error(&self) -> Option<&'static str> {
        match self.state {
            FrameState::Unreadable => Some("Scopes can’t read this frame."),
            FrameState::NonFinite => Some("This frame has invalid values."),
            _ => None,
        }
    }
    pub fn keep_previous(&self) -> bool {
        self.state == FrameState::Waiting
    }
    pub fn footer(&self) -> String {
        format!(
            "{} · {}",
            match self.stage {
                AnalysisStage::WorkingColor => "Working Color",
                AnalysisStage::Output => "Output",
            },
            match self.matrix {
                Matrix::Rec709 => "Rec. 709",
                Matrix::Rec2020 => "Rec. 2020",
            }
        )
    }
}

// N005 — tasks-storage.md. Task estimates are owner facts with sample duration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskState {
    Queued,
    Running,
    Pausing,
    Paused,
    Cancelling,
    Completed,
    Cancelled,
    Failed,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskError {
    SourceOffline,
    DiskFull,
    CodecUnsupported,
    Unknown,
}
impl TaskError {
    pub fn text(self) -> &'static str {
        match self {
            Self::SourceOffline => "The source is offline. Relink it and try again.",
            Self::DiskFull => "Not enough disk space. Free some space and try again.",
            Self::CodecUnsupported => "This format isn’t supported.",
            Self::Unknown => "Something went wrong. Details are in Window ▸ Error Log.",
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskAction {
    Pause,
    Resume,
    Cancel,
    Retry,
}
pub fn task_actions(state: TaskState) -> &'static [TaskAction] {
    match state {
        TaskState::Queued => &[TaskAction::Cancel],
        TaskState::Running => &[TaskAction::Pause, TaskAction::Cancel],
        TaskState::Paused => &[TaskAction::Resume, TaskAction::Cancel],
        TaskState::Failed => &[TaskAction::Retry],
        _ => &[],
    }
}
pub fn task_progress(done: u64, total: u64) -> Option<(u64, u64)> {
    (total > 0 && done <= total).then_some((done, total))
}
pub fn show_eta(state: TaskState, sampled_ms: u64) -> bool {
    state == TaskState::Running && sampled_ms >= 5000
}
pub fn retain_task(state: TaskState, window: bool, finished_elapsed_ms: u64) -> bool {
    window
        || !matches!(state, TaskState::Completed | TaskState::Cancelled)
        || finished_elapsed_ms < 5000
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Ord, PartialOrd)]
pub enum StorageClass {
    Originals,
    Proxies,
    Optimized,
    Renders,
    Archive,
}
pub const STORAGE_CLASSES: [StorageClass; 5] = [
    StorageClass::Originals,
    StorageClass::Proxies,
    StorageClass::Optimized,
    StorageClass::Renders,
    StorageClass::Archive,
];
pub fn may_set_quota(class: StorageClass) -> bool {
    matches!(
        class,
        StorageClass::Proxies | StorageClass::Optimized | StorageClass::Renders
    )
}
/// DESIGN-TODO(opus): only Originals has an explicit initial checkbox value.
/// None keeps the other rows unset until the user or a reviewed default selects.
pub fn archive_checked_by_default(class: StorageClass) -> Option<bool> {
    (class == StorageClass::Originals).then_some(false)
}
pub fn may_archive(references: u64, sole_playable_proxy: bool) -> bool {
    references == 0 && !sole_playable_proxy
}
pub const LOCATION_CHANGE_MOVES_EXISTING: bool = false;
pub const LOWER_QUOTA_DELETES: bool = false;
pub const ARCHIVE_DELETE_ACTION: Option<&str> = None;
pub const SAME_VOLUME_WARNING: &str = "The archive is on the same disk, so this won’t free disk space. Choose another location for the archive to free it.";

// N006 — pro-edit.md. Route commands to the kernel, never clamp or calculate edits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditTool {
    Trim,
    Ripple,
    Roll,
    Slide,
    Slip,
}
impl EditTool {
    pub fn key(self) -> &'static str {
        match self {
            Self::Trim => "alt+v",
            Self::Ripple => "alt+r",
            Self::Roll => "alt+l",
            Self::Slide => "alt+s",
            Self::Slip => "alt+y",
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShuttleKey {
    J,
    K,
    L,
}
pub fn claim_shuttle(_key: ShuttleKey, playing: bool, available: bool) -> bool {
    playing && available
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceAction {
    MarkIn,
    MarkOut,
    ClearIn,
    ClearOut,
    Insert,
    Overwrite,
}
impl SourceAction {
    pub fn key(self) -> &'static str {
        match self {
            Self::MarkIn => "alt+i",
            Self::MarkOut => "alt+o",
            Self::ClearIn => "alt+shift+i",
            Self::ClearOut => "alt+shift+o",
            Self::Insert => "alt+,",
            Self::Overwrite => "alt+.",
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceResolution {
    NeedsMark,
    AtPlayhead,
    Resolved,
    DurationsDisagree,
}
impl SourceResolution {
    pub fn may_commit(self) -> bool {
        matches!(self, Self::AtPlayhead | Self::Resolved)
    }
}
pub const LINKED_SELECTION_DEFAULT: bool = true;
pub const FIT_TO_FILL_SUPPORTED: bool = false;
pub const PRO_EDIT_REQUIRED_TOKENS: [&str; 2] = ["metrics.mark_bracket", "metrics.sync_badge"];

// N007 — os-authorization.md. System dialogs remain owned by macOS. Result
// routing never repeats a failed action after returning from System Settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Capability {
    Camera,
    Microphone,
    Screen,
    MediaBookmark,
    ProjectBookmark,
    Save,
    ProtectedFolder,
    AgentFile,
    LocalModelServer,
    Keychain,
    LocalFonts,
    PointerLock,
    ColourSampler,
    OwnWindowCapture,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthorizationRoute {
    RefuseExtension,
    Relink,
    FailedOpen,
    SaveError,
    OwningSurface,
    AgentFileDenied,
    LocalServerFailed,
    KeychainConsent,
    None,
}
pub fn authorization_route(capability: Capability) -> AuthorizationRoute {
    match capability {
        Capability::Camera | Capability::Microphone | Capability::Screen => {
            AuthorizationRoute::RefuseExtension
        }
        Capability::MediaBookmark => AuthorizationRoute::Relink,
        Capability::ProjectBookmark => AuthorizationRoute::FailedOpen,
        Capability::Save => AuthorizationRoute::SaveError,
        Capability::ProtectedFolder => AuthorizationRoute::OwningSurface,
        Capability::AgentFile => AuthorizationRoute::AgentFileDenied,
        Capability::LocalModelServer => AuthorizationRoute::LocalServerFailed,
        Capability::Keychain => AuthorizationRoute::KeychainConsent,
        _ => AuthorizationRoute::None,
    }
}
pub const RETRY_AFTER_SETTINGS_AUTOMATICALLY: bool = false;
pub const USAGE_STRINGS: [(&str, &str); 6] = [
    (
        "NSDocumentsFolderUsageDescription",
        "Powermove opens projects and media you keep in Documents.",
    ),
    (
        "NSDesktopFolderUsageDescription",
        "Powermove opens projects and media you keep on the Desktop.",
    ),
    (
        "NSDownloadsFolderUsageDescription",
        "Powermove opens projects and media you keep in Downloads.",
    ),
    (
        "NSRemovableVolumesUsageDescription",
        "Powermove opens projects and media on external drives.",
    ),
    (
        "NSNetworkVolumesUsageDescription",
        "Powermove opens projects and media on network drives.",
    ),
    (
        "NSLocalNetworkUsageDescription",
        "Powermove connects to the local model server you set up.",
    ),
];
pub const SAVE_PERMISSION_ERROR: &str =
    "Powermove doesn’t have permission to save here. Choose another location.";
pub const LOCAL_NETWORK_DENIED: &str = "Allow Local Network for Powermove in System Settings.";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LutState {
    NoFile,
    Ready,
    Unreadable,
    Missing,
    TooLarge,
}
impl LutState {
    pub fn passes_through(self) -> bool {
        matches!(self, Self::NoFile | Self::Missing)
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StorageUsage {
    pub class: StorageClass,
    pub used: u64,
    pub quota: Option<u64>,
}
impl StorageUsage {
    pub fn new(class: StorageClass, used: u64, quota: Option<u64>) -> Self {
        Self {
            class,
            used,
            quota: if class == StorageClass::Archive {
                None
            } else {
                quota
            },
        }
    }
    pub fn overage(self) -> u64 {
        self.quota
            .map_or(0, |quota| self.used.saturating_sub(quota))
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocationRefusal {
    Overlap,
    ForeignNonempty,
    LinkedOrTraversal,
    NoAccess,
}
impl LocationRefusal {
    pub fn text(self, class: &str) -> String {
        match self {
            Self::Overlap => format!("This folder overlaps {class}. Choose a separate folder."),
            Self::ForeignNonempty => {
                "This folder has other files in it. Choose an empty folder.".into()
            }
            Self::LinkedOrTraversal => "Choose a regular folder.".into(),
            Self::NoAccess => SAVE_PERMISSION_ERROR.into(),
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditRefusal {
    HandleExhausted,
    OneFrame,
    Locked,
    PastEnd,
    RollNotAdjacent,
    SlideNotAdjacent,
    AmbiguousRipple,
    TimeRemap,
    UnknownLength,
    BrokenLink,
}
impl EditRefusal {
    pub fn text(self) -> &'static str {
        match self {
            Self::HandleExhausted => "There’s no more media in this clip to extend it.",
            Self::OneFrame => "A clip must stay at least one frame long.",
            Self::Locked => "A locked clip is in the way.",
            Self::PastEnd => "This would go past the end of the composition. Extend it first.",
            Self::RollNotAdjacent => "Roll needs two clips that touch.",
            Self::SlideNotAdjacent => "Slide needs a clip with a neighbour on each side.",
            Self::AmbiguousRipple => {
                "Ripple Delete can’t work out what should move. Select clips next to each other."
            }
            Self::TimeRemap => {
                "This edit doesn’t work on a clip with time remapping or animated source time."
            }
            Self::UnknownLength => "Powermove doesn’t know this clip’s media length yet.",
            Self::BrokenLink => "This clip’s link is damaged. Unlink it first.",
        }
    }
}

// Main-reference additions from tasks-storage.md at 78294c4c9. These are
// requests/data projections; actual storage capabilities remain with the owner.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StorageReportState {
    Loading,
    Ready,
    Unavailable,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StorageCategory {
    pub label: String,
    pub formatted_size: String,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StorageReport {
    pub state: StorageReportState,
    pub used: String,
    pub reclaimable: String,
    pub reclaimable_bytes: u64,
    pub categories: Vec<StorageCategory>,
}
impl StorageReport {
    pub fn copy(&self) -> (String, String) {
        match self.state {
            StorageReportState::Loading => (
                "Calculating storage…".into(),
                "Checking backups, media and app data.".into(),
            ),
            StorageReportState::Unavailable => ("Storage unavailable".into(), String::new()),
            StorageReportState::Ready => (
                format!("{} used", self.used),
                format!(
                    "{} can be freed from older backups and temporary caches.",
                    self.reclaimable
                ),
            ),
        }
    }
    pub fn free_space_enabled(&self) -> bool {
        self.state == StorageReportState::Ready && self.reclaimable_bytes > 0
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CleanupClass {
    Media(StorageClass),
    OlderBackups,
    TemporaryCaches,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CleanupRequest {
    Keep,
    MoveToArchive,
    RequestRebuildableCacheClear,
}
/// The owner supplies verified reference/protection facts and rechecks them at
/// the actual side effect. Cache clearing is a request, never a deletion here.
pub fn cleanup_request(
    class: CleanupClass,
    references: u64,
    sole_playable_proxy: bool,
    protected_backup: bool,
) -> CleanupRequest {
    if references > 0 || sole_playable_proxy || protected_backup {
        return CleanupRequest::Keep;
    }
    match class {
        CleanupClass::TemporaryCaches => CleanupRequest::RequestRebuildableCacheClear,
        CleanupClass::Media(StorageClass::Archive) => CleanupRequest::Keep,
        _ => CleanupRequest::MoveToArchive,
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OlderBackupBudget {
    OneGb,
    FiveGb,
    TenGb,
    TwentyGb,
}
impl OlderBackupBudget {
    pub fn label(self) -> &'static str {
        match self {
            Self::OneGb => "1 GB",
            Self::FiveGb => "5 GB",
            Self::TenGb => "10 GB",
            Self::TwentyGb => "20 GB",
        }
    }
}
pub const NEWEST_BACKUPS_OVER_BUDGET: &str =
    "The newest recovery copies are over this budget. They’ll be kept.";
pub const TEMPORARY_CACHE_DETAIL: &str = "Rebuilt when needed, so these are removed, not archived.";
