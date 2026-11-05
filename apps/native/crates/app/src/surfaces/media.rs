//! S006/S007/S024/S038/S039/S040, from Opus' media.md (2026-11-01).
//!
//! N3: semantic descriptions use approved copy and token names only. N7: all
//! actions carry stable target IDs plus document lifetime/revision. N8: hosts
//! enqueue work; neither rendering nor a callback waits for JS or kernel IO.
//! N4: cancellation retains admitted media; this module never deletes bytes.
// Each surface also builds as an independent portable verification target.
#[allow(clippy::duplicate_mod)]
#[path = "sheet_layout.rs"]
pub mod sheet_layout;

use std::collections::{BTreeMap, VecDeque};

#[path = "media_cloud.rs"]
pub mod cloud;
#[path = "media_figma.rs"]
pub mod figma;
#[path = "media_import.rs"]
pub mod import;
#[path = "media_relink.rs"]
pub mod relink;
#[path = "media_sequence.rs"]
pub mod sequence;
#[path = "media_source.rs"]
pub mod source;

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Owner {
    pub window: String,
    pub project: String,
    pub document: u64,
    pub revision: u64,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Target {
    pub owner: Owner,
    pub id: String,
    pub generation: u64,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    UseAnyway,
    Cancel,
    ChooseAnother,
    RelinkAll,
    JustThisOne,
    Download,
    NotNow,
    Retry,
    Locate,
    ShowDetails,
    IndividualImages,
    ImportSequence,
    Replace,
    PasteImage,
    Ok,
}
impl Action {
    pub fn label(self) -> &'static str {
        match self {
            Self::UseAnyway => "Use Anyway",
            Self::Cancel => "Cancel",
            Self::ChooseAnother => "Choose Another…",
            Self::RelinkAll => "Relink All",
            Self::JustThisOne => "Just This One",
            Self::Download => "Download",
            Self::NotNow => "Not Now",
            Self::Retry => "Retry",
            Self::Locate => "Locate File…",
            Self::ShowDetails => "Show Details",
            Self::IndividualImages => "Individual Images",
            Self::ImportSequence => "Import Sequence",
            Self::Replace => "Replace",
            Self::PasteImage => "Paste as Image",
            Self::Ok => "OK",
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Alert {
    pub target: Target,
    pub message: String,
    pub informative: String,
    /// First is the system Return default. Cancel alone gets Escape.
    pub buttons: Vec<Action>,
    pub suppression: Option<&'static str>,
}
impl Alert {
    pub fn key(&self, key: Key) -> Option<Action> {
        match key {
            Key::Return => self.buttons.first().copied(),
            Key::Escape => self
                .buttons
                .contains(&Action::Cancel)
                .then_some(Action::Cancel),
            Key::Tab => None,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    Return,
    Escape,
    Tab,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Severity {
    Info,
    Warning,
    Error,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Banner {
    pub text: String,
    pub severity: Severity,
    pub action: Option<Action>,
}
impl Banner {
    pub fn auto_dismiss_seconds(&self) -> Option<u8> {
        self.action.is_none().then_some(4)
    }
    pub fn close_label(&self) -> Option<&'static str> {
        self.action.map(|_| "Dismiss")
    }
    pub fn announcement_priority(&self) -> &'static str {
        if self.severity == Severity::Error {
            "high"
        } else {
            "medium"
        }
    }
}
/// Commands are requests to the existing single-writer/service owner, never
/// optimistic document writes. Host acceptance and eventual acknowledgements
/// must both check Target. Kernel batches use the given undo label.
#[derive(Clone, Debug, PartialEq)]
pub enum Operation {
    Locate,
    Relink {
        chosen: String,
    },
    RelinkMatching {
        folder: String,
        assets: Vec<String>,
    },
    SetAutomaticCloudDownload,
    Download {
        assets: Vec<String>,
    },
    CancelRemaining {
        jobs: Vec<String>,
        undo_label: Option<&'static str>,
    },
    RetryImports {
        jobs: Vec<String>,
    },
    RetryProxies {
        assets: Vec<String>,
    },
    ShowBackgroundTasks,
    Sequence {
        frames: Vec<String>,
        frame_numbers: Vec<u64>,
        frame_count: u64,
        hold_missing_frames: bool,
        fit_to_first: bool,
        fps: f64,
        replace: bool,
    },
    IndividualImages {
        frames: Vec<String>,
    },
    PasteFigmaImage,
}
#[derive(Clone, Debug, PartialEq)]
pub struct Command {
    pub target: Target,
    pub operation: Operation,
}
/// AppKit adapter implements NSAlert beginSheetModal on target.owner.window,
/// queued FIFO; sheets use native fields/buttons, and pickers use NSOpenPanel.
/// Viewer/task descriptions use the existing v4 UI-kit/pm-design recipes.
/// Implementations must return immediately and route responses asynchronously.
pub trait MediaHost {
    fn enqueue(&mut self, command: Command);
    fn present_alert(&mut self, alert: Alert);
    fn close_alert(&mut self, target: &Target);
}
#[derive(Default)]
pub struct InspectHost {
    pub commands: Vec<Command>,
    pub alerts: Vec<Alert>,
    pub closed: Vec<Target>,
}
impl MediaHost for InspectHost {
    fn enqueue(&mut self, command: Command) {
        self.commands.push(command);
    }
    fn present_alert(&mut self, alert: Alert) {
        self.alerts.push(alert);
    }
    fn close_alert(&mut self, target: &Target) {
        self.closed.push(target.clone());
    }
}
#[derive(Default)]
pub struct AlertQueue {
    windows: BTreeMap<String, VecDeque<Alert>>,
}
impl AlertQueue {
    pub fn push(&mut self, alert: Alert, host: &mut impl MediaHost) {
        let queue = self
            .windows
            .entry(alert.target.owner.window.clone())
            .or_default();
        if queue.iter().any(|a| a.target == alert.target) {
            return;
        }
        if queue.is_empty() {
            host.present_alert(alert.clone());
        }
        queue.push_back(alert);
    }
    pub fn accepts(&self, target: &Target, action: Action) -> bool {
        self.windows
            .get(&target.owner.window)
            .and_then(|q| q.front())
            .is_some_and(|a| &a.target == target && a.buttons.contains(&action))
    }
    /// Reject stale and duplicate callbacks, even if another sheet is queued.
    pub fn finish(&mut self, target: &Target, host: &mut impl MediaHost) -> bool {
        let Some(queue) = self.windows.get_mut(&target.owner.window) else {
            return false;
        };
        if queue.front().map(|a| &a.target) != Some(target) {
            return false;
        }
        queue.pop_front();
        host.close_alert(target);
        if let Some(next) = queue.front() {
            host.present_alert(next.clone());
        }
        true
    }
    pub fn close_document(&mut self, owner: &Owner, host: &mut impl MediaHost) {
        let Some(queue) = self.windows.get_mut(&owner.window) else {
            return;
        };
        let active = queue.front().map(|a| a.target.clone());
        queue.retain(|a| {
            a.target.owner.project != owner.project || a.target.owner.document != owner.document
        });
        if let Some(active) = active
            .filter(|a| a.owner.project == owner.project && a.owner.document == owner.document)
        {
            host.close_alert(&active);
            if let Some(next) = queue.front() {
                host.present_alert(next.clone());
            }
        }
    }
}

#[cfg(test)]
#[path = "media_tests.rs"]
mod tests;

/// One-shot intent bound to the alert actually presented. A queued alert cannot
/// be driven early, and duplicate/aborted/stale responses never repeat an edit.
pub struct AlertInteraction {
    pub alert: Alert,
    operations: Vec<(Action, Operation)>,
    resolved: bool,
}
impl AlertInteraction {
    pub fn new(alert: Alert, operations: Vec<(Action, Operation)>) -> Option<Self> {
        if operations
            .iter()
            .any(|(action, _)| !alert.buttons.contains(action))
        {
            return None;
        }
        Some(Self {
            alert,
            operations,
            resolved: false,
        })
    }
    pub fn respond(
        &mut self,
        target: &Target,
        action: Action,
        queue: &mut AlertQueue,
        host: &mut impl MediaHost,
    ) -> bool {
        if self.resolved || &self.alert.target != target || !queue.accepts(target, action) {
            return false;
        }
        self.resolved = true;
        if let Some((_, operation)) = self.operations.iter().find(|(a, _)| *a == action) {
            host.enqueue(Command {
                target: target.clone(),
                operation: operation.clone(),
            });
        }
        queue.finish(target, host)
    }
    pub fn abort(&mut self, queue: &mut AlertQueue, host: &mut impl MediaHost) -> bool {
        if self.resolved || !queue.finish(&self.alert.target, host) {
            return false;
        }
        self.resolved = true;
        true
    }
}
