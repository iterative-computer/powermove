//! S107–S110. Source: main/onboarding, onboarding/welcome, workspace-import.
use super::{Ticket, invalid, stale, ticket, unavailable};
use crate::{Result, ServiceErrorCode};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Timeline {
    #[default]
    Layers,
    Tracks,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Agent {
    pub provider: String,
    pub model: String,
}
#[derive(Clone, Debug, PartialEq)]
pub struct Choice {
    pub timeline: Timeline,
    pub agent: Option<Agent>,
    pub import_after_effects: bool,
    /// Exact authored choices, including future keys, retained in completion record.
    pub raw: Value,
}
impl Choice {
    pub fn parse(raw: Value) -> Result<Self> {
        if !raw.is_object() {
            return Err(invalid("onboarding choice must be an object"));
        }
        let timeline = match raw.get("timelineMode").and_then(Value::as_str) {
            None if raw.get("timelineMode").is_none() => Timeline::Layers,
            Some("layers") => Timeline::Layers,
            Some("tracks") => Timeline::Tracks,
            _ => return Err(invalid("invalid timeline mode")),
        };
        let import_after_effects = match raw.get("workspaceImport") {
            Some(Value::Null) => false,
            Some(Value::String(s)) if s == "after-effects" => true,
            _ => return Err(invalid("unsupported creative app")),
        };
        let agent = match raw.get("agent") {
            None | Some(Value::Null) => None,
            Some(v) => {
                let agent: Agent = serde_json::from_value(v.clone())?;
                let bytes = agent.model.as_bytes();
                if !["chatgpt", "claude", "compatible"].contains(&agent.provider.as_str())
                    || bytes.is_empty()
                    || bytes.len() > 120
                    || !bytes[0].is_ascii_alphanumeric()
                    || !bytes
                        .iter()
                        .all(|c| c.is_ascii_alphanumeric() || b"._:/-".contains(c))
                {
                    return Err(invalid("invalid agent choice"));
                }
                Some(agent)
            }
        };
        Ok(Self {
            timeline,
            agent,
            import_after_effects,
            raw,
        })
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    Animation,
    Welcome,
    Timeline,
    Agent,
    Transcription,
    Workspace,
    Starting,
    Complete,
}
pub trait OnboardingHost {
    fn create_editor(&mut self) -> Result<()>;
    fn timeline(&mut self, timeline: Timeline) -> Result<()>;
    fn agent(&mut self, agent: &Agent) -> Result<()>;
    fn import_workspace(&mut self) -> Result<()>;
    /// Durable marker; preserves prior generations through the native storage service.
    fn completed(&mut self, record: &Value) -> Result<()>;
    /// Download is owned by the shared model service and outlives this flow.
    fn download_transcription(&mut self, model_id: &str) -> Result<()>;
}
#[derive(Clone, Debug, PartialEq)]
pub struct Begin {
    pub ticket: Ticket,
    pub choice: Choice,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Completion {
    pub warnings: Vec<ServiceErrorCode>,
    pub marker_written: bool,
}
pub fn execute(host: &mut impl OnboardingHost, begin: &Begin) -> Result<Completion> {
    host.create_editor()?;
    let mut warnings = Vec::new();
    if let Err(e) = host.timeline(begin.choice.timeline) {
        warnings.push(e.code);
    }
    if let Some(agent) = &begin.choice.agent
        && let Err(e) = host.agent(agent)
    {
        warnings.push(e.code);
    }
    // Agent choice is applied before import; import failure leaves flow retryable.
    if begin.choice.import_after_effects {
        host.import_workspace()?;
    }
    let mut record = begin.choice.raw.clone();
    record
        .as_object_mut()
        .ok_or_else(|| invalid("invalid completion record"))?
        .insert("version".into(), json!(1));
    let marker_written = match host.completed(&record) {
        Ok(()) => true,
        Err(e) => {
            warnings.push(e.code);
            false
        }
    };
    Ok(Completion {
        warnings,
        marker_written,
    })
}
pub struct Onboarding {
    pub step: Step,
    pub first_run_pending: bool,
    active: Option<Begin>,
}
impl Onboarding {
    pub fn new(first_run_pending: bool) -> Self {
        Self {
            step: Step::Animation,
            first_run_pending,
            active: None,
        }
    }
    /// Animation completion, skip, load failure and watchdog all reach welcome.
    pub fn welcome(&mut self) {
        if self.step == Step::Animation {
            self.step = Step::Welcome;
        }
    }
    pub fn advance(&mut self) -> Result<()> {
        self.step = match self.step {
            Step::Welcome => Step::Timeline,
            Step::Timeline => Step::Agent,
            Step::Agent => Step::Transcription,
            Step::Transcription => Step::Workspace,
            _ => return Err(invalid("onboarding cannot advance")),
        };
        Ok(())
    }
    pub fn back(&mut self) -> Result<()> {
        self.step = match self.step {
            Step::Timeline => Step::Welcome,
            Step::Agent => Step::Timeline,
            Step::Transcription => Step::Agent,
            Step::Workspace => Step::Transcription,
            _ => return Err(invalid("onboarding cannot go back")),
        };
        Ok(())
    }
    pub fn begin(&mut self, choice: Choice) -> Result<Begin> {
        if self.step != Step::Workspace || self.active.is_some() {
            return Err(unavailable("onboarding already starting or incomplete"));
        }
        let request = Begin {
            ticket: ticket(),
            choice,
        };
        self.active = Some(request.clone());
        self.step = Step::Starting;
        Ok(request)
    }
    pub fn settle(&mut self, request: &Begin, result: Result<Completion>) -> Result<Completion> {
        if self.active.as_ref() != Some(request) {
            return Err(stale());
        }
        self.active = None;
        match result {
            Ok(done) => {
                self.first_run_pending = false;
                self.step = Step::Complete;
                Ok(done)
            }
            Err(e) => {
                self.step = Step::Workspace;
                Err(e)
            }
        }
    }
    /// Replay never discards or cancels a started import/download behind the worker.
    pub fn replay(&mut self) -> Result<()> {
        if self.active.is_some() {
            return Err(unavailable("onboarding work still running"));
        }
        self.step = Step::Animation;
        Ok(())
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImportHandoff {
    pub thread_id: String,
    pub original_draft: String,
}
impl ImportHandoff {
    /// Original TS guards: same app thread, unchanged exact draft, not running.
    pub fn may_resume(&self, thread_id: &str, context: &str, draft: &str, running: bool) -> bool {
        thread_id == self.thread_id && context == "app" && draft == self.original_draft && !running
    }
}
