//! N001–N005. Native-only worker adapters, explicitly separate from tag parity.
pub mod tasks;

use super::{Ticket, invalid, stale, ticket, unavailable};
use crate::{Result, ServiceError, ServiceErrorCode};
use pm_first_launch::{Config, Event, Outcome, Step, secrets::SecretAccess};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MigrationSummary {
    pub projects: usize,
    pub history_refusals: usize,
    pub missing_projects: usize,
    pub blocked_owners: usize,
    pub proxy_only: usize,
    pub already_offline: usize,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MigrationState {
    Idle,
    Running,
    WaitingForElectron,
    WaitingForLocalElectron,
    NotEnoughSpace { needed: u64, available: u64 },
    ProfileRefused,
    WaitingForSecrets,
    SecretDenied,
    NeedsSecretConsent { interrupted: bool },
    Paused,
    Failed(ServiceErrorCode),
    Complete(MigrationSummary),
}
/// First-launch currently returns anyhow for these failures. The native host
/// must supply structured facts; do not parse diagnostics or invent byte counts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MigrationFault {
    LowDisk { needed: u64, available: u64 },
    ProfileRefused,
    SecretDenied,
    Other(ServiceErrorCode),
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MigrationRequest {
    pub ticket: Ticket,
    consent: bool,
}
pub struct Migration {
    pub state: MigrationState,
    pub progress: Option<(Step, usize, usize)>,
    pub published: Vec<Step>,
    active: Option<MigrationRequest>,
    pause_requested: bool,
    pause_boundary_reached: bool,
    library_was_ready: bool,
}
impl Default for Migration {
    fn default() -> Self {
        Self {
            state: MigrationState::Idle,
            progress: None,
            published: Vec::new(),
            active: None,
            pause_requested: false,
            pause_boundary_reached: false,
            library_was_ready: false,
        }
    }
}
impl Migration {
    pub fn for_completed_profile() -> Self {
        Self {
            library_was_ready: true,
            ..Default::default()
        }
    }
    /// DESIGN-TODO(opus): N002 requires Not Now/Continue Without Them to stay
    /// visible. pm-first-launch lacks skip_secret_consent; do not fake Complete.
    pub fn decline_secrets(&mut self) -> Result<()> {
        if !matches!(
            self.state,
            MigrationState::NeedsSecretConsent { .. } | MigrationState::SecretDenied
        ) {
            return Err(invalid("no secret consent prompt"));
        }
        Err(unavailable(
            "skip_secret_consent is not implemented by first-launch",
        ))
    }
    pub fn begin(&mut self) -> Result<MigrationRequest> {
        self.start(false)
    }
    /// Call ONLY for a fresh explicit response to secrets::PROMPT; not on retry,
    /// relaunch, automatic resume, or because an old consent receipt exists.
    pub fn consent(&mut self) -> Result<MigrationRequest> {
        if !matches!(
            self.state,
            MigrationState::NeedsSecretConsent { .. } | MigrationState::SecretDenied
        ) {
            return Err(invalid("no current secret consent prompt"));
        }
        self.start(true)
    }
    fn start(&mut self, consent: bool) -> Result<MigrationRequest> {
        if self.active.is_some() {
            return Err(unavailable("migration worker still running"));
        }
        let request = MigrationRequest {
            ticket: ticket(),
            consent,
        };
        self.active = Some(request.clone());
        self.state = if consent {
            MigrationState::WaitingForSecrets
        } else {
            MigrationState::Running
        };
        self.progress = None;
        self.pause_requested = false;
        self.pause_boundary_reached = false;
        Ok(request)
    }
    pub fn request_pause(&mut self) -> Result<()> {
        if self.active.is_none() || self.state != MigrationState::Running {
            return Err(invalid("migration not running"));
        }
        self.pause_requested = true;
        Ok(())
    }
    /// Returns false only at a durable Published boundary. The worker must stop
    /// its hook there, then acknowledge Paused; requests never claim early pause.
    pub fn event(&mut self, request: &MigrationRequest, event: Event) -> Result<bool> {
        if self.active.as_ref() != Some(request) {
            return Err(stale());
        }
        match event {
            Event::Started(step) => self.progress = Some((step, 0, 0)),
            Event::Progress { step, done, total } => {
                if done > total
                    || self
                        .progress
                        .is_some_and(|(s, old, _)| s == step && done < old)
                {
                    return Err(invalid("invalid migration progress"));
                }
                self.progress = Some((step, done, total));
            }
            Event::Published(step) => {
                if !self.published.contains(&step) {
                    self.published.push(step);
                }
                self.pause_boundary_reached = self.pause_requested;
                return Ok(!self.pause_requested);
            }
            // Complete is an event, not permission to open the library. Wait for Outcome.
            Event::WaitingForElectron | Event::SecretPromptRequired | Event::Complete => {}
        }
        Ok(true)
    }
    pub fn paused(&mut self, request: &MigrationRequest) -> Result<()> {
        if self.active.as_ref() != Some(request) || !self.pause_boundary_reached {
            return Err(stale());
        }
        self.active = None;
        self.state = MigrationState::Paused;
        Ok(())
    }
    pub fn settle(&mut self, request: &MigrationRequest, result: Result<Outcome>) -> Result<()> {
        if self.active.as_ref() != Some(request) {
            return Err(stale());
        }
        self.active = None;
        self.state = match result {
            Ok(Outcome::WaitingForElectron) => MigrationState::WaitingForElectron,
            Ok(Outcome::NeedsSecretConsent { interrupted }) => {
                MigrationState::NeedsSecretConsent { interrupted }
            }
            Ok(Outcome::Complete {
                projects,
                history_refusals,
                missing_projects,
                blocked_owners,
                proxy_only,
                already_offline,
            }) => MigrationState::Complete(MigrationSummary {
                projects,
                history_refusals,
                missing_projects,
                blocked_owners,
                proxy_only,
                already_offline,
            }),
            Err(e) => {
                self.state = MigrationState::Failed(e.code);
                return Err(e);
            }
        };
        Ok(())
    }
    pub fn library_may_attach(&self) -> bool {
        self.library_was_ready || matches!(self.state, MigrationState::Complete(_))
    }
    pub fn settle_fault(
        &mut self,
        request: &MigrationRequest,
        fault: MigrationFault,
    ) -> Result<()> {
        if self.active.as_ref() != Some(request) {
            return Err(stale());
        }
        self.active = None;
        self.state = match fault {
            MigrationFault::LowDisk { needed, available } => {
                MigrationState::NotEnoughSpace { needed, available }
            }
            MigrationFault::ProfileRefused => MigrationState::ProfileRefused,
            MigrationFault::SecretDenied => MigrationState::SecretDenied,
            MigrationFault::Other(code) => MigrationState::Failed(code),
        };
        Ok(())
    }
}
/// Real first-launch dispatch. SecretAccess is injected only by a host that
/// received the request's fresh consent. Tests never construct MacKeychain.
pub fn run_migration(
    config: &Config,
    request: &MigrationRequest,
    access: Option<&mut dyn SecretAccess>,
    hook: impl FnMut(Event) -> anyhow::Result<()>,
) -> Result<Outcome> {
    if request.consent {
        let access = access.ok_or_else(|| unavailable("secret access adapter not installed"))?;
        pm_first_launch::resume_with_secret_consent(config, access, hook)
            .map_err(ServiceError::from)
    } else {
        // Ignore any supplied capability; ordinary retries cannot invoke Keychain.
        pm_first_launch::run(config, hook).map_err(ServiceError::from)
    }
}
#[derive(Clone, Debug, PartialEq)]
pub struct MigrationReview {
    pub id: String,
    pub kind: ReviewKind,
    /// Private in-process detail. Do not export through diagnostics or log it.
    pub retained: serde_json::Value,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReviewKind {
    RefusedHistory,
    MissingSlot,
    ProxyOnly,
    Offline,
    BlockedOwner,
    Quarantine,
    Conflict,
    Archive,
}
pub trait MigrationReviewHost {
    fn inspect_retained(&mut self, id: &str) -> Result<MigrationReview>;
    fn reveal_archive(&mut self, id: &str) -> Result<()>;
    /// Three-way/reverse migration is not implemented by the current coordinator.
    fn rollback(&mut self, _id: &str) -> Result<()> {
        Err(unavailable(
            "reverse migration requires the kernel reconciliation service",
        ))
    }
}
