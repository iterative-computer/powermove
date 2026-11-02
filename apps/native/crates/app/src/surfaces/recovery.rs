//! S001/S013/S053/S054, design/surfaces/recovery.md (2026-11-01).
//!
//! Worker-owned, inspectable surface models. Platform methods enqueue, never wait
//! for JavaScript, file IO, or a modal run loop (N7/N8). IDs and revisions fence
//! delayed native callbacks; dispatch never guesses a choice after dismissal.
//!
//! Decisions: spec punctuation supersedes tagged Electron copy (N3/quirk policy);
//! Discard requires a confirmed Recently Deleted archive capability (N4);
//! no old consumers removed (prune rule). Visuals remain provisional for Opus.
use std::collections::{BTreeMap, VecDeque};
use std::time::Duration;

#[path = "recovery_notices.rs"]
pub mod notices;
#[path = "recovery_panels.rs"]
pub mod panels;
#[path = "recovery_status.rs"]
pub mod status;

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Owner {
    Window(String),
    Application,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Reload,
    CloseWindow,
    Recover,
    OpenAsCopy,
    Discard,
    OpenSavedVersion,
    Acknowledge,
    Wait,
    StopExtension,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Button {
    pub label: &'static str,
    pub action: Action,
    pub default: bool,
    pub escape: bool,
    pub destructive: bool,
}
impl Button {
    fn new(label: &'static str, action: Action) -> Self {
        Self {
            label,
            action,
            default: false,
            escape: false,
            destructive: false,
        }
    }
    fn default(mut self) -> Self {
        self.default = true;
        self
    }
    fn escape(mut self) -> Self {
        self.escape = true;
        self
    }
    fn destructive(mut self) -> Self {
        self.destructive = true;
        self
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Alert {
    pub message: String,
    pub informative: String,
    /// Listed order is NSAlert order; first button is rightmost.
    pub buttons: Vec<Button>,
}
impl Alert {
    pub fn key(&self, key: Key) -> Option<Action> {
        self.buttons
            .iter()
            .find(|b| match key {
                Key::Return => b.default,
                Key::Escape => b.escape,
            })
            .map(|b| b.action)
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    Return,
    Escape,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditorFailure {
    OutOfMemory,
    UnexpectedStop,
}
pub fn editor_failure(reason: EditorFailure) -> Alert {
    let reason = match reason {
        EditorFailure::OutOfMemory => "The editor ran out of memory.",
        EditorFailure::UnexpectedStop => "The editor stopped unexpectedly.",
    };
    Alert {
        message: "Powermove needs to reload this window.".into(),
        informative: format!(
            "{reason} Reload restores the latest local recovery. The most recent edits may be missing."
        ),
        // No Esc action: neither button leaves the failed window as it is.
        buttons: vec![
            Button::new("Reload", Action::Reload).default(),
            Button::new("Close Window", Action::CloseWindow),
        ],
    }
}
pub fn ask_recovery(name: &str, truncated: bool, can_archive_recently_deleted: bool) -> Alert {
    let mut buttons = vec![
        Button::new("Recover", Action::Recover).default(),
        Button::new("Open as Copy", Action::OpenAsCopy).escape(),
    ];
    if can_archive_recently_deleted {
        buttons.push(Button::new("Discard Edits", Action::Discard).destructive());
    }
    Alert {
        message: format!("Powermove quit unexpectedly while “{name}” had unsaved changes."),
        informative: format!(
            "Recover opens your latest edits. Open as Copy keeps the saved project unchanged and opens the edits beside it.{}",
            if truncated {
                " The last few seconds of edits may be missing."
            } else {
                ""
            }
        ),
        buttons,
    }
}
pub fn unrecoverable(name: &str, saved_file_exists: bool) -> Alert {
    let mut buttons = Vec::new();
    if saved_file_exists {
        buttons.push(Button::new("Open Saved Version", Action::OpenSavedVersion).default());
    }
    let ok = Button::new("OK", Action::Acknowledge);
    buttons.push(if saved_file_exists { ok } else { ok.default() });
    Alert {
        message: format!("“{name}” couldn’t be recovered."),
        informative: "The saved project is unchanged. Details are in Window ▸ Error Log.".into(),
        buttons,
    }
}
pub fn unresponsive_extension(extension: &str) -> Alert {
    Alert {
        message: format!("“{extension}” isn’t responding."),
        informative: "You can wait, or stop it. A stopped extension stays off until you turn it on in Settings ▸ Extensions.".into(),
        buttons: vec![Button::new("Wait", Action::Wait).default(), Button::new("Stop Extension", Action::StopExtension).destructive()],
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RecoveryState {
    None,
    Recovered { truncated: bool },
    OfferCopy { truncated: bool },
    Unrecoverable,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RecoverySurface {
    Alert(Alert),
    Banner(notices::Banner),
}
/// `asking` comes from ShellReply::RecoveryRequired, not RecoveryResult alone.
pub fn recovery_surface(
    name: &str,
    state: RecoveryState,
    asking: bool,
    can_archive: bool,
    saved_exists: bool,
) -> Option<RecoverySurface> {
    match state {
        RecoveryState::None => None,
        RecoveryState::Unrecoverable => {
            Some(RecoverySurface::Alert(unrecoverable(name, saved_exists)))
        }
        RecoveryState::Recovered { truncated } if !asking => Some(RecoverySurface::Banner(
            notices::Banner::recovered(name, truncated),
        )),
        RecoveryState::Recovered { truncated } | RecoveryState::OfferCopy { truncated } => Some(
            RecoverySurface::Alert(ask_recovery(name, truncated, can_archive)),
        ),
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Prompt {
    /// Allocated by the app, never by extension content. Never reuse in a session.
    pub id: String,
    pub revision: u64,
    pub owner: Owner,
    pub alert: Alert,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Presentation {
    Show(Prompt),
    Dismiss { id: String, owner: Owner },
}
/// A macOS implementation uses NSAlert sheets for Window, app-modal only for
/// Application. One active alert per owner; native callbacks return id/revision.
pub trait RecoveryPlatform {
    type Error;
    fn enqueue(&mut self, presentation: Presentation) -> Result<(), Self::Error>;
}
#[derive(Default)]
pub struct HeadlessPlatform {
    pub events: Vec<Presentation>,
    pub fail_next: bool,
}
impl RecoveryPlatform for HeadlessPlatform {
    type Error = &'static str;
    fn enqueue(&mut self, presentation: Presentation) -> Result<(), Self::Error> {
        if std::mem::take(&mut self.fail_next) {
            return Err("injected queue failure");
        }
        self.events.push(presentation);
        Ok(())
    }
}
#[derive(Debug, PartialEq, Eq)]
pub enum QueueError<E> {
    DuplicateId,
    Stale,
    InvalidAction,
    Platform(E),
}
#[derive(Default)]
pub struct AlertQueue {
    owners: BTreeMap<Owner, VecDeque<Prompt>>,
    visible: std::collections::BTreeSet<Owner>,
    /// Retain retired IDs, so a late callback cannot hit a replacement prompt.
    issued: std::collections::BTreeSet<String>,
}
impl AlertQueue {
    pub fn active(&self, owner: &Owner) -> Option<&Prompt> {
        self.owners.get(owner)?.front()
    }
    pub fn push<P: RecoveryPlatform>(
        &mut self,
        prompt: Prompt,
        host: &mut P,
    ) -> Result<(), QueueError<P::Error>> {
        if prompt.id.is_empty() || self.issued.contains(&prompt.id) {
            return Err(QueueError::DuplicateId);
        }
        let queue = self.owners.entry(prompt.owner.clone()).or_default();
        if queue.is_empty() {
            host.enqueue(Presentation::Show(prompt.clone()))
                .map_err(QueueError::Platform)?;
        }
        if queue.is_empty() {
            self.visible.insert(prompt.owner.clone());
        }
        self.issued.insert(prompt.id.clone());
        queue.push_back(prompt);
        Ok(())
    }
    /// Resolve only a visible button. An aborted sheet is not a default choice.
    pub fn respond<P: RecoveryPlatform>(
        &mut self,
        owner: &Owner,
        id: &str,
        revision: u64,
        action: Action,
        _host: &mut P,
    ) -> Result<Action, QueueError<P::Error>> {
        if !self.visible.contains(owner) {
            return Err(QueueError::Stale);
        }
        let prompt = self.active(owner).ok_or(QueueError::Stale)?;
        if prompt.id != id || prompt.revision != revision {
            return Err(QueueError::Stale);
        }
        if !prompt.alert.buttons.iter().any(|b| b.action == action) {
            return Err(QueueError::InvalidAction);
        }
        self.owners
            .get_mut(owner)
            .ok_or(QueueError::Stale)?
            .pop_front();
        self.visible.remove(owner);
        Ok(action)
    }
    /// Call after the previous sheet has ended. Separating response from the
    /// next publication means a failed show cannot swallow the accepted action.
    pub fn pump<P: RecoveryPlatform>(
        &mut self,
        owner: &Owner,
        host: &mut P,
    ) -> Result<(), QueueError<P::Error>> {
        if self.visible.contains(owner) {
            return Ok(());
        }
        if let Some(next) = self.active(owner) {
            host.enqueue(Presentation::Show(next.clone()))
                .map_err(QueueError::Platform)?;
            self.visible.insert(owner.clone());
        }
        Ok(())
    }
    /// Closing an owner invalidates every queued prompt without dispatching work.
    pub fn close<P: RecoveryPlatform>(
        &mut self,
        owner: &Owner,
        host: &mut P,
    ) -> Result<(), QueueError<P::Error>> {
        let dismiss = self
            .visible
            .contains(owner)
            .then(|| self.active(owner).cloned())
            .flatten();
        // Window closure is authoritative even when native dismissal delivery
        // fails. No late callback may retain permission to mutate its project.
        self.owners.remove(owner);
        self.visible.remove(owner);
        if let Some(prompt) = dismiss {
            host.enqueue(Presentation::Dismiss {
                id: prompt.id,
                owner: owner.clone(),
            })
            .map_err(QueueError::Platform)?;
        }
        Ok(())
    }
}

pub const LOADING_DELAY: Duration = Duration::from_millis(500);

#[cfg(test)]
#[path = "recovery_tests.rs"]
mod tests;
