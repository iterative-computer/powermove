//! S003, S014, S016, S055 window and non-modal lifecycle contracts.
use super::*;
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OpenEffect {
    ShowLoading {
        window: WindowId,
        text: String,
    },
    Announce {
        window: WindowId,
        text: String,
    },
    CancelOpen {
        ticket: Ticket,
    },
    CloseWindow(WindowId),
    /// Recovery remains S001's job. Keep this window and the kernel ticket.
    Recovery {
        window: WindowId,
        ticket: String,
        recovery: pm_kernel::session::RecoveryResult,
    },
    Error {
        owner: Owner,
        alert: Alert,
    },
    Focus {
        window: WindowId,
    },
    /// Late open results after cancellation must retire only their generation.
    Retire(SessionHandle),
    Ready {
        window: WindowId,
        session: SessionHandle,
    },
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OpenPhase {
    Opening,
    Loading,
    Recovery,
    Ready,
    Cancelled,
    Failed,
}
pub struct Opening {
    pub ticket: Ticket,
    pub window: WindowId,
    pub name: String,
    pub phase: OpenPhase,
    started: Duration,
}
impl Opening {
    pub fn new(ticket: Ticket, window: WindowId, name: String, now: Duration) -> Self {
        Self {
            ticket,
            window,
            name,
            phase: OpenPhase::Opening,
            started: now,
        }
    }
    pub fn project_actions_enabled(&self) -> bool {
        self.phase == OpenPhase::Ready
    }
    pub fn tick(&mut self, now: Duration) -> Vec<OpenEffect> {
        if self.phase != OpenPhase::Opening
            || now.saturating_sub(self.started) < Duration::from_millis(500)
        {
            return vec![];
        }
        self.phase = OpenPhase::Loading;
        vec![
            OpenEffect::ShowLoading {
                window: self.window,
                text: format!("Opening “{}”…", self.name),
            },
            OpenEffect::Announce {
                window: self.window,
                text: format!("Opening {}", self.name),
            },
        ]
    }
    /// Command-W and tab close share this path, with no confirmation.
    pub fn cancel(&mut self) -> Vec<OpenEffect> {
        if !matches!(
            self.phase,
            OpenPhase::Opening | OpenPhase::Loading | OpenPhase::Recovery
        ) {
            return vec![];
        }
        self.phase = OpenPhase::Cancelled;
        vec![
            OpenEffect::CancelOpen {
                ticket: self.ticket,
            },
            OpenEffect::CloseWindow(self.window),
        ]
    }
    pub fn reply(&mut self, ticket: Ticket, reply: &ShellReply) -> Vec<OpenEffect> {
        if ticket != self.ticket {
            return vec![];
        }
        if self.phase == OpenPhase::Cancelled {
            return match reply {
                ShellReply::Opened { session, .. } => vec![OpenEffect::Retire(session.clone())],
                _ => vec![],
            };
        }
        if !matches!(
            self.phase,
            OpenPhase::Opening | OpenPhase::Loading | OpenPhase::Recovery
        ) {
            return vec![];
        }
        match reply {
            ShellReply::RecoveryRequired { ticket, recovery } => {
                self.phase = OpenPhase::Recovery;
                vec![OpenEffect::Recovery {
                    window: self.window,
                    ticket: ticket.clone(),
                    recovery: recovery.clone(),
                }]
            }
            ShellReply::Opened { session, .. } => {
                self.phase = OpenPhase::Ready;
                vec![
                    OpenEffect::Ready {
                        window: self.window,
                        session: session.clone(),
                    },
                    OpenEffect::Announce {
                        window: self.window,
                        text: format!("{} opened", self.name),
                    },
                ]
            }
            _ => vec![],
        }
    }
    pub fn failed(
        &mut self,
        ticket: Ticket,
        code: &str,
        path_exists: bool,
        remaining_window: Option<WindowId>,
    ) -> Vec<OpenEffect> {
        if ticket != self.ticket
            || !matches!(
                self.phase,
                OpenPhase::Opening | OpenPhase::Loading | OpenPhase::Recovery
            )
        {
            return vec![];
        }
        self.phase = OpenPhase::Failed;
        vec![
            OpenEffect::CloseWindow(self.window),
            OpenEffect::Error {
                owner: remaining_window.map_or(Owner::Application, Owner::Window),
                alert: open_error(&self.name, code, path_exists),
            },
        ]
    }
}
/// `AlreadyOpen` focuses the existing owner, with no toast or second document.
pub fn already_open(window: WindowId) -> OpenEffect {
    OpenEffect::Focus { window }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RestoreFailure {
    pub name: String,
    pub service_code: String,
    pub path: PathBuf,
    pub path_exists: bool,
}
pub fn restore_error(failures: &[RestoreFailure]) -> Option<Alert> {
    match failures {
        [] => None,
        [one] => Some(open_error(&one.name, &one.service_code, one.path_exists)),
        [a, b, rest @ ..] => Some(Alert {
            message: "Some projects couldn’t be reopened.".into(),
            informative: if rest.is_empty() {
                format!(
                    "“{}” and “{}” may have been moved or deleted.",
                    a.name, b.name
                )
            } else {
                format!(
                    "“{}”, “{}” and {} more may have been moved or deleted.",
                    a.name,
                    b.name,
                    rest.len()
                )
            },
            buttons: vec![
                default(Action::Ok, "OK"),
                Button::new(Action::ShowProjectLibrary, "Show Project Library", None),
            ],
        }),
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TransferEffect {
    Request {
        session: SessionHandle,
        destination: WindowId,
    },
    SnapBack,
}
pub struct TabTransfer {
    pub session: SessionHandle,
    pub destination: WindowId,
    pub waiting: bool,
    pub requested: bool,
}
impl TabTransfer {
    pub fn new(
        session: SessionHandle,
        destination: WindowId,
        saving: bool,
    ) -> (Self, Vec<TransferEffect>) {
        let mut transfer = Self {
            session,
            destination,
            waiting: saving,
            requested: false,
        };
        let effects = transfer.observe(saving);
        (transfer, effects)
    }
    pub fn observe(&mut self, saving: bool) -> Vec<TransferEffect> {
        if saving || self.requested {
            return vec![];
        }
        self.waiting = false;
        self.requested = true;
        vec![TransferEffect::Request {
            session: self.session.clone(),
            destination: self.destination,
        }]
    }
    pub fn resolved(&self, accepted: bool) -> Vec<TransferEffect> {
        if accepted {
            vec![]
        } else {
            vec![TransferEffect::SnapBack]
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PopoutSpec {
    pub panel_id: String,
    pub panel_title: String,
    pub project_name: String,
    pub previous_dock: String,
    pub default_dock: String,
}
impl PopoutSpec {
    pub fn title(&self) -> String {
        format!("{} — {}", self.panel_title, self.project_name)
    }
    pub fn return_label(&self) -> String {
        format!("Return {} to Layout", self.panel_title)
    }
    pub fn autosave_name(&self) -> String {
        format!("powermove.panel.{}", self.panel_id)
    }
    pub fn eligible(&self) -> bool {
        !matches!(self.panel_id.as_str(), "viewer" | "toolbar" | "tools")
    }
    pub const RETURN_TOOLTIP: &'static str = "Return to Layout";
    pub const RETURN_SYMBOL: &'static str = "rectangle.inset.filled.and.arrow.left";
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PopoutEffect {
    Create(PopoutSpec),
    Undock(String),
    Dock { panel: String, position: String },
    CloseWindow(String),
    RecordPoppedOut { panel: String, popped_out: bool },
}
pub struct Popout {
    pub spec: PopoutSpec,
    pub popped_out: bool,
    creating: bool,
}
impl Popout {
    pub fn new(spec: PopoutSpec) -> Self {
        Self {
            spec,
            popped_out: false,
            creating: false,
        }
    }
    pub fn open(&mut self) -> Vec<PopoutEffect> {
        if !self.spec.eligible() || self.popped_out || self.creating {
            return vec![];
        }
        self.creating = true;
        vec![PopoutEffect::Create(self.spec.clone())]
    }
    pub fn created(&mut self, success: bool) -> Vec<PopoutEffect> {
        if !std::mem::take(&mut self.creating) || !success {
            return vec![];
        }
        self.popped_out = true;
        vec![
            PopoutEffect::Undock(self.spec.panel_id.clone()),
            PopoutEffect::RecordPoppedOut {
                panel: self.spec.panel_id.clone(),
                popped_out: true,
            },
        ]
    }
    /// Return button, menu and window close all route here, never project close.
    pub fn return_to_layout(&mut self, previous_dock_exists: bool) -> Vec<PopoutEffect> {
        if !std::mem::take(&mut self.popped_out) {
            return vec![];
        }
        vec![
            PopoutEffect::Dock {
                panel: self.spec.panel_id.clone(),
                position: if previous_dock_exists {
                    self.spec.previous_dock.clone()
                } else {
                    self.spec.default_dock.clone()
                },
            },
            PopoutEffect::CloseWindow(self.spec.panel_id.clone()),
            PopoutEffect::RecordPoppedOut {
                panel: self.spec.panel_id.clone(),
                popped_out: false,
            },
        ]
    }
    pub fn project_closed(&mut self) -> Vec<PopoutEffect> {
        self.creating = false;
        if !self.popped_out {
            return vec![];
        }
        // Persisted true survives reopening; programmatic close must bypass the
        // close-button delegate that would normally return the panel to layout.
        self.popped_out = false;
        vec![
            PopoutEffect::RecordPoppedOut {
                panel: self.spec.panel_id.clone(),
                popped_out: true,
            },
            PopoutEffect::CloseWindow(self.spec.panel_id.clone()),
        ]
    }
}
/// The watcher/reload service must decide whether a clean external change can
/// be reloaded. This reducer never merges or writes an unreviewed version.
#[derive(Default)]
pub struct DiskChange {
    changed: bool,
}
impl DiskChange {
    pub fn observed(&mut self, edited: bool) -> bool {
        self.changed = true;
        !edited
    }
    pub fn did_reload(&mut self, name: &str) -> Notice {
        self.changed = false;
        reloaded(name)
    }
    pub fn before_save(&self, name: &str) -> Option<Alert> {
        self.changed.then(|| changed_on_disk(name))
    }
    pub fn approved(&mut self) {
        self.changed = false;
    }
}
