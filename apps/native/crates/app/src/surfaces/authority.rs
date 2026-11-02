//! S010–S012 authority contracts, queues and worker commands.
//!
//! Decisions: N1/N3 use the Opus copy and system controls; N4 archive releases,
//! layouts and local projects (only explicitly removed credentials are erased);
//! N6 values retain their extension/env identity; N7 every action has an id and
//! generation; N8 publication is nonblocking, with no JS or service IO here.
//! The app's shell must dispatch this module; registration alone is not wiring.
use std::collections::{BTreeMap, VecDeque};
use std::fmt;

#[path = "authority_bridge.rs"]
pub mod bridge;
#[cfg(target_os = "macos")]
#[path = "authority_macos.rs"]
pub mod macos;
#[cfg(test)]
#[path = "authority_tests.rs"]
mod tests;
#[path = "authority_values.rs"]
pub mod values;

/// Deliberately not Serialize, Display, Clone or inspectable. Only the native
/// secure control, explicit Reveal/Copy and worker value service may expose it.
pub struct PrivateValue(String);
impl PrivateValue {
    pub fn new(value: String) -> Self {
        Self(value)
    }
    pub fn expose(&self) -> &str {
        &self.0
    }
}
impl fmt::Debug for PrivateValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("PrivateValue(<redacted>)")
    }
}

/// Generation is supplied by the owning kernel/service, never by display name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Target {
    pub window: String,
    pub object: String,
    pub generation: u64,
}
pub type RequestId = u64;
pub const CONCEALED_PASTEBOARD_TYPE: &str = "org.nspasteboard.ConcealedType";

/// Codes mirror pm-services::error::ServiceErrorCode; no diagnostic string can
/// cross this boundary. Direct dependency/From impl belongs to app integration.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ServiceCode {
    Offline,
    NotFound,
    TooLarge,
    InvalidProject,
    PermissionDenied,
    DiskFull,
    InvalidInput,
    InvalidData,
    Unavailable,
    Unknown,
}
impl ServiceCode {
    pub fn informative(self) -> &'static str {
        match self {
            Self::Offline => "Check your internet connection and try again.",
            Self::NotFound => "It may have been moved or deleted.",
            Self::TooLarge | Self::InvalidProject => "",
            Self::PermissionDenied => "Check its sharing and permissions in Finder.",
            Self::DiskFull => "Free some space and try again.",
            Self::InvalidInput | Self::InvalidData => "The file may be damaged.",
            Self::Unavailable | Self::Unknown => {
                "Try again. If it keeps happening, choose Window ▸ Error Log for details."
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Cancel,
    AllowFullAccess,
    AllowThisRun,
    Done,
    Copy,
    Keep,
    Delete,
    MoveToTrash,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Button {
    pub action: Action,
    pub label: &'static str,
    pub default: bool,
    pub escape: bool,
    pub destructive: bool,
}
fn button(action: Action, label: &'static str, default: bool, destructive: bool) -> Button {
    Button {
        action,
        label,
        default,
        escape: action == Action::Cancel,
        destructive,
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AlertStyle {
    Warning,
    Critical,
    Informational,
}

#[derive(Debug)]
pub enum Prompt {
    FullAccess {
        thread: String,
        effect: u64,
    },
    RunAccess {
        run: String,
        project: String,
        summary: String,
    },
    Reveal {
        key: String,
        label: String,
        value: PrivateValue,
    },
    Uninstall {
        extension: String,
        name: String,
        revision: u64,
    },
    DeleteWorkspace {
        workspace: String,
        name: String,
        revision: u64,
    },
    TrashLocal {
        project: String,
        name: String,
    },
    DeleteLocal {
        project: String,
        name: String,
    },
}
#[derive(Debug)]
pub struct Alert {
    pub id: RequestId,
    pub target: Target,
    pub prompt: Prompt,
}

/// Native-only view. Debug and general-purpose inspection never expose a value.
#[derive(Debug)]
pub struct AlertView<'a> {
    pub id: RequestId,
    pub target: &'a Target,
    pub message: String,
    pub informative: PrivateValue,
    pub style: AlertStyle,
    pub buttons: Vec<Button>,
    /// Native layout must ellipsize at 3 visual lines (not 3 newline segments).
    pub max_lines: Option<usize>,
    pub close_access_popover_first: bool,
}
impl Alert {
    pub fn view(&self) -> AlertView<'_> {
        let cancel = || button(Action::Cancel, "Cancel", true, false);
        let delete = || button(Action::Delete, "Delete", false, true);
        let (message, informative, style, buttons) = match &self.prompt {
            Prompt::FullAccess { .. } => (
                "Give Powermove agents full access to your Mac?".into(),
                "Agents will run commands and change files anywhere on this Mac without asking, until you choose another access level or quit Powermove.".into(),
                AlertStyle::Critical,
                vec![cancel(), button(Action::AllowFullAccess, "Allow Full Access", false, false)],
            ),
            Prompt::RunAccess { project, summary, .. } => (
                format!("Allow computer access for “{project}”?"),
                summary.clone(),
                AlertStyle::Warning,
                vec![cancel(), button(Action::AllowThisRun, "Allow This Run", false, false)],
            ),
            Prompt::Reveal { label, value, .. } => (
                label.clone(), value.expose().into(), AlertStyle::Informational,
                vec![button(Action::Done, "Done", true, false), button(Action::Copy, "Copy", false, false)],
            ),
            Prompt::Uninstall { name, .. } => (
                format!("Also delete the values you entered for {name}?"),
                "If you keep them, they’re used again when you add this extension back.".into(),
                AlertStyle::Warning,
                vec![button(Action::Keep, "Keep", true, false), delete()],
            ),
            Prompt::DeleteWorkspace { name, .. } => (
                format!("Delete the workspace “{name}”?"),
                "Your panels stay as they are. You can undo this.".into(),
                AlertStyle::Warning,
                // Keep S012's listed order; shared grammar forbids a destructive
                // default. The adapter explicitly sets Cancel as default.
                vec![delete(), cancel()],
            ),
            Prompt::TrashLocal { name, .. } => (
                format!("Move “{name}” to the Trash?"),
                "You can put it back from Recently Deleted.".into(),
                AlertStyle::Warning,
                vec![button(Action::MoveToTrash, "Move to Trash", true, false), button(Action::Cancel, "Cancel", false, false)],
            ),
            Prompt::DeleteLocal { name, .. } => (
                format!("Delete “{name}” immediately?"),
                "The project is archived in Powermove’s storage and can’t be opened from the Library again.".into(),
                AlertStyle::Critical,
                vec![delete(), cancel()],
            ),
        };
        AlertView {
            id: self.id,
            target: &self.target,
            message,
            informative: PrivateValue::new(informative),
            style,
            buttons,
            max_lines: matches!(self.prompt, Prompt::RunAccess { .. }).then_some(3),
            close_access_popover_first: matches!(self.prompt, Prompt::FullAccess { .. }),
        }
    }
    pub fn inspection(&self) -> Inspection {
        let view = self.view();
        Inspection {
            id: self.id,
            target: self.target.clone(),
            message: view.message,
            // A summary may contain private agent input too. Inspection has no
            // informative-text field, even while an explicit reveal is open.
            buttons: view.buttons,
            awaiting_answer: matches!(
                self.prompt,
                Prompt::FullAccess { .. } | Prompt::RunAccess { .. }
            ),
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Inspection {
    pub id: RequestId,
    pub target: Target,
    pub message: String,
    pub buttons: Vec<Button>,
    pub awaiting_answer: bool,
}
impl Inspection {
    pub fn agent_state_label(&self) -> Option<&'static str> {
        self.awaiting_answer.then_some("Awaiting answer")
    }
}

/// Worker queue payloads. Implementations must retain ownership on queue-full
/// failure. No implementation may call JS or wait for a kernel future here.
#[derive(Debug)]
pub enum Command {
    AwaitingAnswer {
        target: Target,
        request: RequestId,
        run: Option<String>,
        activity_summary: Option<PrivateValue>,
    },
    ComputerConsent {
        target: Target,
        request: RequestId,
        thread: String,
        effect: u64,
        granted: bool,
    },
    RunDecision {
        target: Target,
        request: RequestId,
        run: String,
        granted: bool,
    },
    ArchiveExtension {
        target: Target,
        request: RequestId,
        extension: String,
        revision: u64,
        delete_values: bool,
    },
    ArchiveWorkspace {
        target: Target,
        request: RequestId,
        workspace: String,
        revision: u64,
    },
    MoveLocalToTrash {
        target: Target,
        request: RequestId,
        project: String,
    },
    ArchiveLocal {
        target: Target,
        request: RequestId,
        project: String,
    },
}
pub trait AuthorityServices {
    /// ComputerConsent -> Panel::acknowledge(effect, granted); RunDecision ->
    /// run-scoped pm-agents::approvals (display copy is NOT its protocol label).
    /// ArchiveExtension -> pm-store-host::install::uninstall, then erase only
    /// explicitly selected values. ArchiveWorkspace -> durable archive + kernel
    /// transaction + Undo Delete Workspace; never LayoutStore::delete_named alone.
    fn enqueue(&mut self, command: Command) -> Result<(), ServiceCode>;
}

/// Commands are main-thread native publication only, never a modal JS bridge.
pub trait AuthorityHost {
    fn present(&mut self, view: AlertView<'_>);
    fn dismiss(&mut self, window: &str, request: RequestId);
    fn copy_concealed(&mut self, value: &PrivateValue);
    fn changed_banner(&mut self, target: &Target, text: &str);
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Stale,
    InvalidAction,
    Busy,
    Service(ServiceCode),
}

#[derive(Default)]
pub struct Authority {
    next: RequestId,
    windows: BTreeMap<String, VecDeque<Alert>>,
}
impl Authority {
    /// Only call for an originating user action or agent request. A fresh id
    /// prevents an old native completion from answering the next queued alert.
    pub fn request(
        &mut self,
        target: Target,
        prompt: Prompt,
        services: &mut impl AuthorityServices,
        host: &mut impl AuthorityHost,
    ) -> Result<RequestId, Error> {
        let id = self.next.checked_add(1).ok_or(Error::Busy)?;
        self.next = id;
        if let Prompt::FullAccess { .. } | Prompt::RunAccess { .. } = &prompt {
            services
                .enqueue(Command::AwaitingAnswer {
                    target: target.clone(),
                    request: id,
                    run: if let Prompt::RunAccess { run, .. } = &prompt {
                        Some(run.clone())
                    } else {
                        None
                    },
                    activity_summary: if let Prompt::RunAccess { summary, .. } = &prompt {
                        Some(PrivateValue::new(summary.clone()))
                    } else {
                        None
                    },
                })
                .map_err(Error::Service)?;
        }
        let queue = self.windows.entry(target.window.clone()).or_default();
        queue.push_back(Alert { id, target, prompt });
        if queue.len() == 1 {
            host.present(queue.front().expect("just inserted").view());
        }
        Ok(id)
    }
    pub fn inspect(&self, window: &str) -> Vec<Inspection> {
        self.windows
            .get(window)
            .into_iter()
            .flatten()
            .map(Alert::inspection)
            .collect()
    }
    pub fn answer(
        &mut self,
        target: &Target,
        id: RequestId,
        action: Action,
        services: &mut impl AuthorityServices,
        host: &mut impl AuthorityHost,
    ) -> Result<(), Error> {
        let queue = self.windows.get_mut(&target.window).ok_or(Error::Stale)?;
        let alert = queue
            .front()
            .filter(|a| a.id == id && a.target == *target)
            .ok_or(Error::Stale)?;
        if !alert.view().buttons.iter().any(|b| b.action == action) {
            return Err(Error::InvalidAction);
        }
        let command = match &alert.prompt {
            Prompt::FullAccess { thread, effect } => Some(Command::ComputerConsent {
                target: target.clone(),
                request: id,
                thread: thread.clone(),
                effect: *effect,
                granted: action == Action::AllowFullAccess,
            }),
            Prompt::RunAccess { run, .. } => Some(Command::RunDecision {
                target: target.clone(),
                request: id,
                run: run.clone(),
                granted: action == Action::AllowThisRun,
            }),
            Prompt::Uninstall {
                extension,
                revision,
                ..
            } => Some(Command::ArchiveExtension {
                target: target.clone(),
                request: id,
                extension: extension.clone(),
                revision: *revision,
                delete_values: action == Action::Delete,
            }),
            Prompt::DeleteWorkspace {
                workspace,
                revision,
                ..
            } if action == Action::Delete => Some(Command::ArchiveWorkspace {
                target: target.clone(),
                request: id,
                workspace: workspace.clone(),
                revision: *revision,
            }),
            Prompt::TrashLocal { project, .. } if action == Action::MoveToTrash => {
                Some(Command::MoveLocalToTrash {
                    target: target.clone(),
                    request: id,
                    project: project.clone(),
                })
            }
            Prompt::DeleteLocal { project, .. } if action == Action::Delete => {
                Some(Command::ArchiveLocal {
                    target: target.clone(),
                    request: id,
                    project: project.clone(),
                })
            }
            Prompt::Reveal { value, .. } if action == Action::Copy => {
                host.copy_concealed(value);
                None
            }
            _ => None,
        };
        if let Some(command) = command {
            services.enqueue(command).map_err(Error::Service)?;
        }
        queue.pop_front();
        host.dismiss(&target.window, id);
        if let Some(next) = queue.front() {
            host.present(next.view());
        }
        Ok(())
    }
    /// Cancel before the owner invalidates its generation. Never accepts a
    /// response for a renamed/replaced/closed target. Call once per affected
    /// target; queued alerts for other objects/windows remain in order.
    pub fn invalidate(
        &mut self,
        target: &Target,
        object_label: &str,
        services: &mut impl AuthorityServices,
        host: &mut impl AuthorityHost,
    ) -> Result<(), Error> {
        let Some(queue) = self.windows.get_mut(&target.window) else {
            return Ok(());
        };
        // Send denials before removing requests, preserving a retry on failure.
        for alert in queue.iter().filter(|a| a.target == *target) {
            let command = match &alert.prompt {
                Prompt::FullAccess { thread, effect } => Some(Command::ComputerConsent {
                    target: target.clone(),
                    request: alert.id,
                    thread: thread.clone(),
                    effect: *effect,
                    granted: false,
                }),
                Prompt::RunAccess { run, .. } => Some(Command::RunDecision {
                    target: target.clone(),
                    request: alert.id,
                    run: run.clone(),
                    granted: false,
                }),
                _ => None,
            };
            if let Some(command) = command {
                services.enqueue(command).map_err(Error::Service)?;
            }
        }
        let dismiss = queue.front().filter(|a| a.target == *target).map(|a| a.id);
        let removed = queue.iter().any(|a| a.target == *target);
        queue.retain(|a| a.target != *target);
        if let Some(id) = dismiss {
            host.dismiss(&target.window, id);
            if let Some(next) = queue.front() {
                host.present(next.view());
            }
        }
        if removed {
            host.changed_banner(
                target,
                &format!("{object_label} was changed, so the dialog closed."),
            );
        }
        Ok(())
    }
    /// No-values uninstall never presents an alert. Worker remains responsible
    /// for archival and the expected-revision admission check.
    pub fn uninstall_without_values(
        &mut self,
        target: Target,
        extension: String,
        revision: u64,
        services: &mut impl AuthorityServices,
    ) -> Result<RequestId, Error> {
        let id = self.next.checked_add(1).ok_or(Error::Busy)?;
        self.next = id;
        services
            .enqueue(Command::ArchiveExtension {
                target,
                request: id,
                extension,
                revision,
                delete_values: false,
            })
            .map_err(Error::Service)?;
        Ok(id)
    }
}

/// Process-only standing grant. Persist only `previous_non_full`, never Full.
/// Backend must acknowledge consent before calling allow_full; quitting drops
/// the instance. A per-run grant must never call allow_full.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AccessLevel {
    Editor,
    Project,
    Full,
}
pub struct StandingAccess {
    current: AccessLevel,
    previous_non_full: AccessLevel,
}
impl StandingAccess {
    pub fn launch(saved: AccessLevel) -> Self {
        let level = if saved == AccessLevel::Full {
            AccessLevel::Project
        } else {
            saved
        };
        Self {
            current: level,
            previous_non_full: level,
        }
    }
    pub fn select_non_full(&mut self, level: AccessLevel) -> Result<(), Error> {
        if level == AccessLevel::Full {
            return Err(Error::InvalidAction);
        }
        self.current = level;
        self.previous_non_full = level;
        Ok(())
    }
    pub fn allow_full(&mut self) {
        self.current = AccessLevel::Full;
    }
    pub fn saved_level(&self) -> AccessLevel {
        self.previous_non_full
    }
    pub fn current(&self) -> AccessLevel {
        self.current
    }
    pub fn full_indicator(&self) -> Option<(&'static str, &'static str, &'static str)> {
        (self.current == AccessLevel::Full).then_some((
            "Full access",
            "exclamationmark.triangle.fill",
            "colors.warning",
        ))
    }
}
