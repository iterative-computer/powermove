//! N007/S114. Native OS outcomes, not Chromium permission-prompt imitation.
//! Actual picker/Keychain/TCC/notification presentation is system-owned.
use super::{Session, Ticket, invalid, stale, ticket, unavailable};
use crate::{Result, ServiceErrorCode};
use std::collections::BTreeSet;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Capability {
    SelectedFile,
    FileAccess,
    LocalNetwork,
    Notifications,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Authorization {
    NotDetermined,
    Authorized,
    Provisional,
    Denied,
    Restricted,
    Cancelled,
    Unavailable,
    Failed(ServiceErrorCode),
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuthorizationRequest {
    pub ticket: Ticket,
    pub session: Option<Session>,
    pub capability: Capability,
    pub resource_id: String,
}
pub trait AuthorizationHost {
    fn status(&mut self, capability: Capability, resource_id: &str) -> Result<Authorization>;
    /// Call only for an explicit user operation. No blanket camera/microphone/
    /// screen-recording grant is inferred from trusted local UI or old Chromium policy.
    fn request(&mut self, request: &AuthorizationRequest) -> Result<Authorization>;
}
#[derive(Default)]
pub struct AuthorizationFlow {
    active: Option<AuthorizationRequest>,
    pub outcome: Option<Authorization>,
}
impl AuthorizationFlow {
    pub fn begin(
        &mut self,
        session: Option<Session>,
        capability: Capability,
        resource_id: String,
    ) -> Result<AuthorizationRequest> {
        if self.active.is_some() {
            return Err(unavailable("authorization operation already active"));
        }
        if capability == Capability::SelectedFile && resource_id.is_empty() {
            return Err(invalid("file authorization requires a selected resource"));
        }
        let request = AuthorizationRequest {
            ticket: ticket(),
            session,
            capability,
            resource_id,
        };
        self.active = Some(request.clone());
        self.outcome = None;
        Ok(request)
    }
    pub fn settle(
        &mut self,
        request: &AuthorizationRequest,
        current_session: Option<&Session>,
        result: Result<Authorization>,
    ) -> Result<Authorization> {
        if self.active.as_ref() != Some(request) {
            return Err(stale());
        }
        self.active = None;
        if request.session.as_ref() != current_session {
            return Err(stale());
        }
        match result {
            Ok(outcome) => {
                self.outcome = Some(outcome);
                Ok(outcome)
            }
            Err(e) => {
                self.outcome = Some(Authorization::Failed(e.code));
                Err(e)
            }
        }
    }
    /// Cancels ownership of a callback, not an already-visible OS dialog.
    pub fn detach(&mut self) {
        self.active = None;
        self.outcome = None;
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompletionNotice {
    pub event_id: String,
    pub run_id: String,
    pub thread_id: String,
    pub window_id: String,
    /// Existing agent/service supplies reviewed title/body. No new copy here.
    pub title: String,
    pub body: String,
    pub question: bool,
}
impl CompletionNotice {
    pub fn finished(
        event_id: String,
        run_id: String,
        thread_id: String,
        window_id: String,
    ) -> Self {
        Self {
            event_id,
            run_id,
            thread_id,
            window_id,
            title: "Powermove".into(),
            body: "Your agent has finished. Your result is ready.".into(),
            question: false,
        }
    }
    pub fn question(
        event_id: String,
        run_id: String,
        thread_id: String,
        window_id: String,
        question: &str,
    ) -> Self {
        Self {
            event_id,
            run_id,
            thread_id,
            window_id,
            title: "Your agent has a question".into(),
            body: question
                .chars()
                .map(|c| if c.is_control() { ' ' } else { c })
                .collect::<String>()
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" "),
            question: true,
        }
    }
    /// Same thread replaces its old notification. Play through the notification
    /// service, so Focus/system sound settings apply; never spawn afplay.
    pub fn identifier(&self) -> &str {
        &self.thread_id
    }
    pub fn sound(&self) -> &'static str {
        "Little Victory (Deep)"
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NotificationContext {
    pub enabled: bool,
    pub frontmost: bool,
    pub agent_visible: bool,
    pub stopped_by_user: bool,
}
pub trait NotificationHost: AuthorizationHost {
    fn deliver(&mut self, notice: &CompletionNotice) -> Result<()>;
    fn activate_thread(&mut self, window_id: &str, thread_id: &str) -> Result<()>;
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Delivery {
    Sent,
    Disabled,
    Stopped,
    Foreground,
    Duplicate,
    Authorization(Authorization),
}
#[derive(Default)]
pub struct Notifications {
    delivered: BTreeSet<(String, String, String)>,
}
impl Notifications {
    /// S114 asks on the first sent run, without a pre-prompt. A completion never
    /// prompts. Denial leaves the in-app card/dot/thread state untouched.
    pub fn run_sent(&mut self, host: &mut impl NotificationHost) -> Result<Authorization> {
        let status = host.status(Capability::Notifications, "")?;
        if status != Authorization::NotDetermined {
            return Ok(status);
        }
        host.request(&AuthorizationRequest {
            ticket: ticket(),
            session: None,
            capability: Capability::Notifications,
            resource_id: String::new(),
        })
    }
    pub fn complete(
        &mut self,
        host: &mut impl NotificationHost,
        notice: &CompletionNotice,
        context: NotificationContext,
    ) -> Result<Delivery> {
        if !context.enabled {
            return Ok(Delivery::Disabled);
        }
        if context.stopped_by_user {
            return Ok(Delivery::Stopped);
        }
        if context.frontmost && context.agent_visible {
            return Ok(Delivery::Foreground);
        }
        let key = (
            notice.thread_id.clone(),
            notice.run_id.clone(),
            notice.event_id.clone(),
        );
        if self.delivered.contains(&key) {
            return Ok(Delivery::Duplicate);
        }
        if notice.event_id.is_empty()
            || notice.run_id.is_empty()
            || notice.thread_id.is_empty()
            || notice.window_id.is_empty()
        {
            return Err(invalid("notification identity missing"));
        }
        let outcome = host.status(Capability::Notifications, "")?;
        if !matches!(
            outcome,
            Authorization::Authorized | Authorization::Provisional
        ) {
            return Ok(Delivery::Authorization(outcome));
        }
        host.deliver(notice)?;
        self.delivered.insert(key);
        Ok(Delivery::Sent)
    }
    pub fn activate(
        &self,
        host: &mut impl NotificationHost,
        notice: &CompletionNotice,
    ) -> Result<()> {
        if !self.delivered.contains(&(
            notice.thread_id.clone(),
            notice.run_id.clone(),
            notice.event_id.clone(),
        )) {
            return Err(invalid("notification was not delivered"));
        }
        host.activate_thread(&notice.window_id, &notice.thread_id)
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ForbiddenExtensionCapability {
    Camera,
    Microphone,
    Screen,
}
pub fn extension_request(_capability: ForbiddenExtensionCapability) -> Authorization {
    Authorization::Denied
}
/// NSColorSampler requires no permission. Its host reports a sample or cancellation
/// through the picker, never through AuthorizationHost::request.
pub const COLOUR_SAMPLER_REQUIRES_AUTHORIZATION: bool = false;
