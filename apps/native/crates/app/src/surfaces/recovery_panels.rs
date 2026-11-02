//! UI state from store-host ViewStatus and Failure events. Only content is
//! replaced; panel header, card and dock menu remain owned by pm-shell.
use super::LOADING_DELAY;
use std::time::Duration;

/// Host projection of pm-store-host::view::ViewStatus. The adapter passes the
/// last Failure from its lifecycle event; View does not expose a reason getter.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ViewStatus {
    Starting,
    Ready,
    Sealing,
    Stopping,
    StopFailed,
    WaitingToRestart,
    Disabled,
    Closed,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ViewFailure {
    Crash,
    Hang,
    UserStop,
    ReadyTimeout,
    PlatformFailure,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PanelAction {
    Restart,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PanelButton {
    pub label: &'static str,
    pub action: PanelAction,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Content {
    Unchanged,
    Hidden,
    Loading {
        line: String,
    },
    EmptyState {
        title: String,
        line: Option<String>,
        button: Option<PanelButton>,
    },
    /// Neutral placeholder: impossible/unspecified state has no invented copy.
    DesignTodo(&'static str),
}
impl Content {
    pub fn ax_label(&self) -> Option<String> {
        match self {
            Self::Loading { line } => Some(line.clone()),
            Self::EmptyState { title, line, .. } => Some(
                line.as_ref()
                    .map_or_else(|| title.clone(), |line| format!("{title}. {line}")),
            ),
            _ => None,
        }
    }
    pub fn announcement(&self) -> Option<super::notices::Priority> {
        matches!(self, Self::EmptyState { .. }).then_some(super::notices::Priority::High)
    }
}
pub struct PanelSnapshot<'a> {
    pub panel: &'a str,
    pub extension: &'a str,
    pub status: ViewStatus,
    pub failure: Option<ViewFailure>,
    pub elapsed: Duration,
    pub realm_prompt_open: bool,
}
impl PanelSnapshot<'_> {
    pub fn content(&self) -> Content {
        use ViewFailure::*;
        use ViewStatus::*;
        let empty = |title: String, line: Option<String>, label: Option<&'static str>| {
            Content::EmptyState {
                title,
                line,
                button: label.map(|label| PanelButton {
                    label,
                    action: PanelAction::Restart,
                }),
            }
        };
        let p = self.panel;
        let e = self.extension;
        if self.status == Closed {
            return Content::Hidden;
        }
        if self.realm_prompt_open {
            return empty(
                format!("{p} Stopped Responding"),
                Some("Powermove stopped it and will restart it in a moment.".into()),
                None,
            );
        }
        match (self.status, self.failure) {
            (Closed, _) => Content::Hidden,
            (Ready, _) => Content::Unchanged,
            (StopFailed, _) => empty(
                format!("{p} Couldn’t Be Stopped"),
                Some("Quit and reopen Powermove to stop it.".into()),
                None,
            ),
            (Sealing | Stopping, _) => Content::Loading {
                line: format!("Stopping {e}…"),
            },
            (_, Some(UserStop)) => empty(format!("{p} Is Stopped"), None, Some("Restart")),
            (_, Some(PlatformFailure)) => empty(
                format!("{p} Can’t Run"),
                Some(format!("This Mac couldn’t start {e}.")),
                Some("Try Again"),
            ),
            (Disabled, _) => empty(
                format!("{p} Is Turned Off"),
                Some(format!("{e} stopped working several times.")),
                Some("Turn On"),
            ),
            (_, Some(ReadyTimeout)) => empty(
                format!("{p} Didn’t Start"),
                Some(format!("{e} took too long to start.")),
                Some("Try Again"),
            ),
            (Starting, _) if self.elapsed > LOADING_DELAY => Content::Loading {
                line: format!("Starting {e}…"),
            },
            (Starting, _) => Content::Unchanged,
            // A restart without a recorded failure reads as a plain stop.
            (WaitingToRestart, Some(Crash) | None) => empty(
                format!("{p} Stopped"),
                Some("It will restart in a moment.".into()),
                Some("Restart Now"),
            ),
            (WaitingToRestart, Some(Hang)) => empty(
                format!("{p} Stopped Responding"),
                Some("Powermove stopped it and will restart it in a moment.".into()),
                Some("Restart Now"),
            ),
        }
    }
}
/// A stale generation cannot restart a new view with an old card's button.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RestartRequest {
    pub view: String,
    pub generation: u64,
}
pub fn restart_request(
    view: &str,
    expected: u64,
    current: u64,
    content: &Content,
) -> Option<RestartRequest> {
    if expected != current
        || !matches!(
            content,
            Content::EmptyState {
                button: Some(_),
                ..
            }
        )
    {
        return None;
    }
    Some(RestartRequest {
        view: view.into(),
        generation: current,
    })
}
