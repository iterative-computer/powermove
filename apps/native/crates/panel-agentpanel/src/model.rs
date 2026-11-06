//! Thread-scoped model. External work is a fenced effect, not a success-shaped
//! placeholder. Store is supplied by the kernel's storage owner on a worker.
use crate::{
    Error, Result,
    activity::{Rows, Trace},
    composer::{Attachment, Draft},
    input::Catalog,
    questions::{Answers, QuestionDraft, shown_answers},
    view::{Follow, ThreadSummary},
};
use pm_agents::{
    Access, ApprovalMode, Provider,
    persistence::{Store, Threads},
};
use serde_json::{Value, json};
use std::{
    collections::{HashMap, HashSet},
    io,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Idle,
    Prompt,
    Running,
    Preview,
    Result,
}
#[derive(Clone, Debug)]
pub struct Run {
    pub id: String,
    pub request_id: String,
    pub started_at: u64,
    pub sequence: u64,
    pub stopping: bool,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Target {
    pub project: String,
    pub thread: String,
    pub run: Option<String>,
}
#[derive(Clone, Debug)]
pub struct Settings {
    pub provider: Provider,
    pub model: String,
    pub effort: String,
    pub access: Access,
    pub approval: ApprovalMode,
    pub scope: String,
    pub auto_apply_panels: bool,
}
impl Settings {
    fn from_record(v: &Value) -> Self {
        Self {
            provider: match v["provider"].as_str() {
                Some("claude") => Provider::Claude,
                Some("compatible") => Provider::Compatible,
                _ => Provider::Chatgpt,
            },
            model: v["model"].as_str().unwrap_or_default().into(),
            effort: v["reasoningEffort"].as_str().unwrap_or_default().into(),
            // A persisted computer grant is intent, never live authority.
            access: if matches!(v["access"].as_str(), Some("project" | "computer")) {
                Access::Project
            } else {
                Access::Editor
            },
            approval: if v["approvalMode"] == "supervised" {
                ApprovalMode::Supervised
            } else {
                ApprovalMode::Auto
            },
            scope: v["scope"].as_str().unwrap_or("workspace").into(),
            auto_apply_panels: v["autoApplyPanels"].as_bool().unwrap_or(true),
        }
    }
}
#[derive(Clone, Debug)]
pub struct Session {
    pub draft: Draft,
    pub conversation: Rows,
    pub trace: Trace,
    pub run: Option<Run>,
    last_run: Option<String>,
    settings_dirty: bool,
    access_dirty: bool,
    pub settings: Settings,
    pub catalog: Catalog,
    pub slash: crate::input::SlashMenu,
    pub follow: Follow,
    pub phase: Phase,
    pub applying: bool,
    pub archiving: bool,
    pub activity: String,
    pub progress: Vec<String>,
    pub plan: Option<Value>,
    pub result: Option<Value>,
    result_message: Option<String>,
    pub stop_error: Option<String>,
    pub disclosures: HashMap<String, bool>,
    pub(crate) questions: HashMap<String, QuestionDraft>,
    next_message: u64,
}
impl Session {
    fn from_record(v: &Value) -> Result<Self> {
        if v.get("provider").is_some_and(|p| {
            !p.is_null() && !matches!(p.as_str(), Some("chatgpt" | "claude" | "compatible"))
        }) {
            return Err(Error::Invalid("unsupported provider record"));
        }
        let files = v["attachments"].as_array().cloned().unwrap_or_default();
        let mut conversation = Rows::default();
        let messages = v["conversation"]
            .as_array()
            .ok_or(Error::Invalid("conversation"))?;
        for (i, message) in messages.iter().enumerate() {
            let mut message = message.clone();
            if let Some(steps) = message["steps"].as_array_mut() {
                for step in steps {
                    if step["kind"] == "question"
                        && step["transport"] == "reply"
                        && step["status"] == "open"
                    {
                        step["status"] = json!("closed");
                    }
                }
            }
            conversation.upsert(format!("message-{i}"), message)?;
        }
        let mut trace = Trace::default();
        // A restart keeps checkpoints visible but does not invent a live process.
        for step in v["nativeLiveTrace"].as_array().into_iter().flatten() {
            let id = step["id"].as_str().ok_or(Error::Invalid("trace id"))?;
            if trace.rows.get(id).is_some() {
                return Err(Error::Invalid("duplicate trace id"));
            }
            trace.rows.upsert(id.into(), step.clone())?;
        }
        let mut s = Self {
            draft: Draft::from_legacy(
                v["composerDraft"]
                    .as_str()
                    .ok_or(Error::Invalid("composer draft"))?,
                &files,
            )?,
            conversation,
            trace,
            run: None,
            last_run: v["lastRunId"].as_str().map(str::to_owned),
            settings_dirty: false,
            access_dirty: false,
            settings: Settings::from_record(v),
            catalog: Catalog::default(),
            slash: crate::input::SlashMenu::default(),
            follow: Follow::default(),
            phase: if messages.is_empty() {
                Phase::Idle
            } else {
                Phase::Prompt
            },
            applying: false,
            archiving: false,
            activity: String::new(),
            progress: vec![],
            plan: None,
            result: None,
            result_message: None,
            stop_error: None,
            disclosures: HashMap::new(),
            questions: HashMap::new(),
            next_message: messages.len() as u64,
        };
        s.trace.seal(v["updatedAt"].as_u64().unwrap_or_default())?;
        s.trace.close_held_questions()?;
        Ok(s)
    }
    fn append(&mut self, value: Value) -> Result<String> {
        let id = format!("message-{}", self.next_message);
        self.next_message += 1;
        self.conversation.upsert(id.clone(), value)?;
        Ok(id)
    }
    /// A question the run is blocked on; the composer's text answers it.
    pub fn needs_answer(&self) -> bool {
        self.trace
            .rows
            .values()
            .any(|row| row["kind"] == "question" && row["status"] == "open")
    }
    pub fn elapsed_ms(&self, now: u64) -> Option<u64> {
        self.run.as_ref().map(|r| now.saturating_sub(r.started_at))
    }
    pub fn composer_disabled(&self) -> bool {
        self.applying
    }
    pub fn disclosure_open(&self, id: &str, live_group: bool) -> bool {
        self.disclosures.get(id).copied().unwrap_or_else(|| {
            self.trace.rows.get(id).is_some_and(|r| {
                r.value["kind"] == "thought" && r.value["live"] == true
                    || r.value["task"].is_object() && r.value["status"] == "error"
            }) || live_group
        })
    }
    fn archive_trace(&mut self, now: u64, steering: bool) -> Result<()> {
        self.trace.seal(now)?;
        if !steering {
            self.trace.close_held_questions()?;
        }
        if !self.trace.rows.is_empty() {
            let duration = self.run.as_ref().map(|r| now.saturating_sub(r.started_at));
            self.append(json!({"role":"trace","steps":self.trace.rows.values().collect::<Vec<_>>(),"steering":steering,"durationMs":duration}))?;
            self.trace = Trace::default();
        }
        Ok(())
    }
    fn archive_result(&mut self) -> Result<()> {
        if let Some(result) = self.result.clone() {
            if let Some(id) = &self.result_message {
                self.conversation
                    .update(id, |v| v["nativeResult"] = result)?;
            } else {
                self.result_message =
                    Some(self.append(json!({"role":"assistant","nativeResult":result}))?);
            }
        }
        Ok(())
    }
    fn step(&self, id: &str) -> Option<&Value> {
        self.trace
            .rows
            .get(id)
            .map(|r| r.value.as_ref())
            .or_else(|| {
                self.conversation
                    .values()
                    .flat_map(|m| m["steps"].as_array().into_iter().flatten())
                    .find(|s| s["id"] == id)
            })
    }
    pub fn question(&self, id: &str) -> Option<&Value> {
        self.trace
            .rows
            .get(id)
            .filter(|r| r.value["kind"] == "question")
            .map(|r| r.value.as_ref())
            .or_else(|| {
                self.conversation
                    .values()
                    .flat_map(|m| m["steps"].as_array().into_iter().flatten())
                    .find(|s| s["id"] == id && s["kind"] == "question")
            })
    }
    fn change_question(&mut self, id: &str, f: impl FnOnce(&mut Value)) -> Result<()> {
        if self.trace.rows.get(id).is_some() {
            self.trace.rows.update(id, f)?;
            return Ok(());
        }
        let message = self
            .conversation
            .page(0..self.conversation.len())
            .iter()
            .find(|r| {
                r.value["steps"]
                    .as_array()
                    .is_some_and(|a| a.iter().any(|s| s["id"] == id))
            })
            .map(|r| r.id.clone())
            .ok_or_else(|| Error::UnknownId(id.into()))?;
        self.conversation.update(&message, |v| {
            if let Some(step) = v["steps"]
                .as_array_mut()
                .and_then(|a| a.iter_mut().find(|s| s["id"] == id))
            {
                f(step);
            }
        })?;
        Ok(())
    }
}

/// Raw answers are deliberately excluded from Debug and never persisted.
#[derive(Clone)]
pub struct PrivateAnswers(pub Answers);
impl std::fmt::Debug for PrivateAnswers {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("PrivateAnswers(<redacted>)")
    }
}
#[derive(Clone, Debug)]
pub enum EffectKind {
    Send {
        text: String,
        attachments: Vec<Value>,
        settings: Settings,
        steer: bool,
    },
    Stop,
    Answer {
        question: String,
        request_id: Option<String>,
        transport: String,
        answers: PrivateAnswers,
    },
    StopTask {
        task: String,
        request_id: String,
    },
    ComputerConsent,
    RevokeComputerConsent,
    ApplyPlan(Value),
    UndoResult(Value),
    OpenAttachment {
        attachment: Value,
        preview: bool,
        restore_focus: String,
    },
    RevealArtifact(Value),
    ImportArtifact(Value),
    SelectProvider(String),
    NewThread,
    OpenSettings,
    ChooseFiles,
    Shell(ShellAction),
    ConfirmArchive,
    Retry {
        message: String,
        continue_with_project: bool,
    },
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ShellAction {
    Open,
    Close,
    Pin,
    BeginDrag,
    CancelDrag,
    CommitDrag { dock: Option<String> },
    OpenGlobal,
    ImportWorkspace,
    RepairExtension(String),
}
#[derive(Clone, Debug)]
pub struct Effect {
    pub id: u64,
    pub target: Target,
    pub kind: EffectKind,
}
#[derive(Clone, Debug)]
enum Pending {
    Answer { target: Target, question: String },
    Consent(Target),
    Stop(Target),
    Task { target: Target, task: String },
    Apply(Target),
    Undo(Target),
    Import { target: Target, path: String },
}
#[derive(Clone, Debug)]
pub enum Update {
    Trace(Value),
    Progress(String),
    Complete {
        text: String,
        error: bool,
        cancelled: bool,
    },
    Plan(Value),
    Result(Value),
}
#[derive(Clone)]
pub struct Panel {
    project: String,
    archive: Threads,
    sessions: HashMap<String, Session>,
    pending: HashMap<u64, Pending>,
    archives_after_stop: HashMap<String, String>,
    retired_runs: HashSet<String>,
    /// When the oldest change not yet saved was made: streamed reply text and
    /// draft typing wait for a checkpoint instead of a durable write each.
    unsaved_since: Option<u64>,
    pub save_error: Option<String>,
    pub follow: Follow,
}
/// How far streamed text and typing may run ahead of the durable archive.
pub const CHECKPOINT_MS: u64 = 1000;
impl Panel {
    /// Worker-only: storage IO is synchronous by trait contract. Never call from
    /// a main-thread input handler. Unknown archive fields remain in Threads.
    pub fn load(store: &mut impl Store, project: &str, blank: &str, now: u64) -> Result<Self> {
        let mut archive = Threads::load(store, project, blank, now).map_err(host)?;
        if archive.raw()["threads"]
            .as_array()
            .is_some_and(Vec::is_empty)
        {
            archive.create(blank, now).map_err(host)?;
        }
        if archive.raw()["threads"]
            .as_array()
            .into_iter()
            .flatten()
            .any(|r| {
                matches!(
                    r["lastRunStatus"].as_str(),
                    Some("queued" | "running" | "waiting")
                )
            })
        {
            archive.update(store, |value| {
                for record in value["threads"].as_array_mut().expect("validated threads") {
                    if !matches!(record["lastRunStatus"].as_str(), Some("queued" | "running" | "waiting")) { continue; }
                    if record.get("nativeInterruptedRuns").is_none() { record["nativeInterruptedRuns"] = json!([]); }
                    let previous = json!({"run":record["lastRunId"],"status":record["lastRunStatus"],"observedAt":now});
                    record["nativeInterruptedRuns"].as_array_mut()
                        .ok_or_else(|| io::Error::other("Unknown interrupted-run archive shape"))?.push(previous);
                    record["lastRunStatus"] = json!("interrupted");
                }
                Ok(())
            }).map_err(host)?;
        }
        let hydrated = archive.hydrated(store).map_err(host)?;
        let mut sessions = HashMap::new();
        for thread in hydrated["threads"]
            .as_array()
            .ok_or(Error::Invalid("threads"))?
        {
            let id = thread["id"].as_str().ok_or(Error::Invalid("thread id"))?;
            sessions.insert(id.into(), Session::from_record(thread)?);
        }
        Ok(Self {
            project: project.into(),
            archive,
            sessions,
            pending: HashMap::new(),
            archives_after_stop: HashMap::new(),
            retired_runs: hydrated["threads"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|v| v["lastRunId"].as_str().map(str::to_owned))
                .collect(),
            unsaved_since: None,
            save_error: None,
            follow: Follow::default(),
        })
    }
    pub fn project(&self) -> &str {
        &self.project
    }
    pub fn active_id(&self) -> &str {
        self.archive.active_id()
    }
    pub fn session(&self, id: &str) -> Result<&Session> {
        self.sessions
            .get(id)
            .ok_or_else(|| Error::UnknownId(id.into()))
    }
    pub fn draft_mut(&mut self, id: &str) -> Result<&mut Draft> {
        let s = self
            .sessions
            .get_mut(id)
            .ok_or_else(|| Error::UnknownId(id.into()))?;
        if s.applying {
            return Err(Error::Busy);
        }
        s.slash.changed();
        Ok(&mut s.draft)
    }
    pub(crate) fn session_mut(&mut self, id: &str) -> Result<&mut Session> {
        self.sessions
            .get_mut(id)
            .ok_or_else(|| Error::UnknownId(id.into()))
    }
    pub(crate) fn target(&self, id: &str) -> Result<Target> {
        Ok(Target {
            project: self.project.clone(),
            thread: id.into(),
            run: self.session(id)?.run.as_ref().map(|r| r.id.clone()),
        })
    }
    pub(crate) fn effect(&mut self, target: Target, kind: EffectKind) -> Effect {
        static NEXT_EFFECT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        Effect {
            id: NEXT_EFFECT.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            target,
            kind,
        }
    }
    fn no_apply(&self) -> Result<()> {
        if self.sessions.values().any(|s| s.applying) {
            Err(Error::Busy)
        } else {
            Ok(())
        }
    }
    fn capture(&mut self, id: &str, now: u64) -> Result<()> {
        let s = self.session(id)?;
        let (draft, attachments) = s.draft.legacy();
        let conversation = s.conversation.values().cloned().collect::<Vec<_>>();
        let trace = s.trace.rows.values().cloned().collect::<Vec<_>>();
        let run = s.run.as_ref().map(|r| r.id.clone());
        let settings = s.settings.clone();
        let settings_dirty = s.settings_dirty;
        let s_access_dirty = s.access_dirty;
        let record = self
            .archive
            .thread_mut(id)
            .ok_or_else(|| Error::UnknownId(id.into()))?;
        let touched = record["composerDraft"].as_str() != Some(&draft)
            || record["conversation"].as_array().map(Vec::len) != Some(conversation.len());
        record["composerDraft"] = json!(draft);
        record["attachments"] = json!(attachments);
        record["conversation"] = json!(conversation);
        record["nativeLiveTrace"] = json!(trace);
        if settings_dirty {
            record["provider"] = json!(settings.provider.name());
            record["model"] = json!(settings.model);
            record["reasoningEffort"] = json!(settings.effort);
            record["scope"] = json!(settings.scope);
            if s_access_dirty || record.get("access").is_none() {
                record["access"] = json!(settings.access);
            }
            record["approvalMode"] = json!(settings.approval);
            record["autoApplyPanels"] = json!(settings.auto_apply_panels);
        }
        if touched {
            record["updatedAt"] = json!(now);
        }
        if let Some(run) = run {
            record["lastRunId"] = json!(run);
            record["lastRunStatus"] = json!("running");
        }
        // Provider title generation is shared with pm-agents, not invented copy.
        if !conversation.is_empty()
            && (record["title"] == "New thread"
                || record["title"].as_str().is_none_or(str::is_empty))
        {
            record["title"] = json!(pm_agents::conversation::title(&conversation));
        }
        Ok(())
    }
    pub fn save(&mut self, store: &mut impl Store, now: u64) -> Result<()> {
        let ids: Vec<_> = self.sessions.keys().cloned().collect();
        for id in ids {
            self.session_mut(&id)?.archive_result()?;
            self.capture(&id, now)?;
        }
        match self.archive.save(store) {
            Ok(()) => {
                self.save_error = None;
                self.unsaved_since = None;
                Ok(())
            }
            Err(e) => {
                self.save_error = Some(e.to_string());
                Err(host(e))
            }
        }
    }
    /// Keep a change in memory until the next checkpoint or save.
    pub fn defer_save(&mut self, now: u64) {
        self.unsaved_since.get_or_insert(now);
    }
    pub fn unsaved(&self) -> bool {
        self.unsaved_since.is_some()
    }
    /// Save deferred changes once they are `CHECKPOINT_MS` old. True if it saved.
    pub fn checkpoint(&mut self, store: &mut impl Store, now: u64) -> Result<bool> {
        match self.unsaved_since {
            Some(since) if now.saturating_sub(since) >= CHECKPOINT_MS => {
                self.save(store, now).map(|()| true)
            }
            _ => Ok(false),
        }
    }
    /// Publish the provider observation and its panel projection together. This
    /// runs on the storage owner's serial worker, never on a UI callback.
    /// Failed publication leaves the original model intact for an exact retry.
    /// Streamed fragments are the exception: they change the projection in
    /// memory, keep no receipt, and reach the archive at the next checkpoint or
    /// with the next event that saves.
    pub fn provider_event(
        &mut self,
        store: &mut impl Store,
        thread: &str,
        receipt: &pm_agents::history::Receipt,
        now: u64,
    ) -> Result<bool> {
        let streamed = receipt.event.streamed();
        // A process may finish reaping after its conversation was archived.
        // Retain those observations on that exact archived run, never a new thread.
        if self.archive.thread(thread).is_none() {
            if streamed {
                return Ok(false);
            }
            let mut next = self.clone();
            next.archive
                .update(store, |archive| {
                    let record = archive["archivedThreads"]
                        .as_array_mut()
                        .and_then(|threads| {
                            threads
                                .iter_mut()
                                .find(|r| r["id"] == thread && r["lastRunId"] == receipt.source)
                        })
                        .ok_or_else(|| io::Error::other("Unknown archived provider run"))?;
                    pm_agents::history::append(record, std::slice::from_ref(receipt))
                })
                .map_err(host)?;
            *self = next;
            return Ok(true);
        }
        let record = self.archive.thread(thread).ok_or(Error::Stale)?;
        let old = record["nativeRunEvents"].as_array();
        if let Some(previous) = old
            .into_iter()
            .flatten()
            .find(|v| v["source"] == receipt.source && v["sequence"] == receipt.sequence)
        {
            if *previous != serde_json::to_value(receipt).map_err(|e| host(e.into()))? {
                return Err(Error::Invalid("provider receipt conflict"));
            }
            return Ok(false);
        }
        let mut next = self.clone();
        let run = next.session(thread)?.run.as_ref();
        let current = run.is_some_and(|r| r.id == receipt.source);
        let settled = record["nativeRunEvents"]
            .as_array()
            .into_iter()
            .flatten()
            .any(|r| {
                r["source"] == receipt.source
                    && matches!(r["event"]["kind"].as_str(), Some("completed" | "stopped"))
            });
        if !current
            && !settled
            && next.session(thread)?.last_run.as_deref() != Some(&receipt.source)
        {
            return Err(Error::Stale);
        }
        // Cleanup from a retired worker can arrive after the next turn starts.
        // Preserve it as inspection; never apply it to the new run's projection.
        let update = if !current {
            None
        } else {
            match &receipt.event {
                pm_agents::Event::Trace { value, .. } => Some(Update::Trace(value.clone())),
                pm_agents::Event::Progress { text, .. } => Some(Update::Progress(text.clone())),
                pm_agents::Event::Completed { result, .. } => Some(match result {
                    pm_agents::RunResult::Success { text, .. } => Update::Complete {
                        text: pm_agents::launch::reply_text(text),
                        error: false,
                        cancelled: false,
                    },
                    pm_agents::RunResult::Failure {
                        error, cancelled, ..
                    } => Update::Complete {
                        text: error.clone(),
                        error: true,
                        cancelled: *cancelled,
                    },
                }),
                pm_agents::Event::Stopped(reason) if run.is_some() => Some(Update::Complete {
                    text: reason.clone(),
                    error: true,
                    cancelled: false,
                }),
                _ => None,
            }
        };
        if streamed && !current {
            return Ok(false);
        }
        let archive_after = next.archives_after_stop.remove(thread);
        if let Some(update) = update {
            let sequence = next
                .session(thread)?
                .run
                .as_ref()
                .ok_or(Error::Stale)?
                .sequence
                + 1;
            next.receive(thread, &receipt.source, sequence, update, now)?;
        }
        if streamed {
            if let Some(replacement) = archive_after {
                next.archives_after_stop.insert(thread.into(), replacement);
            }
            next.defer_save(now);
            *self = next;
            return Ok(true);
        }
        next.capture(thread, now)?;
        pm_agents::history::append(
            next.archive.thread_mut(thread).ok_or(Error::Stale)?,
            std::slice::from_ref(receipt),
        )
        .map_err(host)?;
        // Archive only after retaining the terminal receipt in the full record.
        if let Some(replacement) = archive_after {
            if next.session(thread)?.run.is_none() {
                next.archive_thread(thread, &replacement, true, now)?;
            } else {
                next.archives_after_stop.insert(thread.into(), replacement);
            }
        }
        if let Err(error) = next.save(store, now) {
            self.save_error = next.save_error;
            return Err(error);
        }
        *self = next;
        Ok(true)
    }
    pub fn raw_archive(&self) -> &Value {
        self.archive.raw()
    }
    pub fn switch(&mut self, id: &str) -> Result<()> {
        self.no_apply()?;
        if !self.archive.select(id) {
            return Err(Error::UnknownId(id.into()));
        }
        self.follow.switch_thread();
        Ok(())
    }
    pub fn new_thread(&mut self, id: &str, now: u64) -> Result<String> {
        self.no_apply()?;
        let active = self.session(self.active_id())?;
        let mut settings = active.settings.clone();
        if settings.access == Access::Computer {
            settings.access = Access::Project;
        }
        let catalog = active.catalog.clone();
        let ids: Vec<_> = self.sessions.keys().cloned().collect();
        for id in ids {
            self.capture(&id, now)?;
        }
        let record = self.archive.create(id, now).map_err(host)?.clone();
        let actual = record["id"].as_str().expect("validated").to_owned();
        if !self.sessions.contains_key(&actual) {
            let mut session = Session::from_record(&record)?;
            session.settings = settings;
            session.settings_dirty = true;
            session.catalog = catalog;
            self.sessions.insert(actual.clone(), session);
        }
        self.follow.switch_thread();
        Ok(actual)
    }
    pub fn summaries(&self) -> Vec<ThreadSummary> {
        self.archive.raw()["threads"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|v| {
                let id = v["id"].as_str()?;
                Some(ThreadSummary {
                    id: id.into(),
                    title: v["title"].as_str().unwrap_or_default().into(),
                    busy: self.sessions.get(id).is_some_and(|s| s.run.is_some()),
                    updated_at: v["updatedAt"].as_u64().unwrap_or_default(),
                })
            })
            .collect()
    }
    pub fn background_runs(&self) -> usize {
        self.sessions
            .iter()
            .filter(|(id, s)| id.as_str() != self.active_id() && s.run.is_some())
            .count()
    }
    /// Confirmation is a semantic host effect. N4: confirmed removal archives;
    /// active runs must first settle so their final work still has an owner.
    pub fn archive_thread(
        &mut self,
        id: &str,
        replacement: &str,
        confirmed: bool,
        now: u64,
    ) -> Result<Option<Effect>> {
        self.no_apply()?;
        let target = self.target(id)?;
        if !confirmed {
            return Ok(Some(self.effect(target, EffectKind::ConfirmArchive)));
        }
        if replacement == id
            || self.archive.thread(replacement).is_some()
            || replacement.is_empty()
            || replacement.len() > 120
            || !replacement
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-')
        {
            return Err(Error::Invalid("replacement id"));
        }
        if let Some(run) = &self.session(id)?.run {
            let stopping = run.stopping;
            let effect = if stopping { None } else { Some(self.stop(id)?) };
            self.archives_after_stop
                .insert(id.into(), replacement.into());
            self.session_mut(id)?.archiving = true;
            return Ok(effect);
        }
        self.capture(id, now)?;
        self.archive
            .archive_thread(id, replacement, now)
            .map_err(host)?;
        self.sessions.remove(id);
        let active = self.active_id().to_owned();
        if !self.sessions.contains_key(&active) {
            let record = self.archive.thread(&active).expect("active");
            self.sessions.insert(active, Session::from_record(record)?);
        }
        Ok(None)
    }
    pub fn add_attachments(&mut self, id: &str, files: Vec<Attachment>, now: u64) -> Result<()> {
        let draft = self.draft_mut(id)?;
        let new_ids: HashSet<_> = files
            .iter()
            .map(Attachment::id)
            .chain(draft.attachments().map(Attachment::id))
            .collect();
        if new_ids.len() > 6 {
            return Err(Error::Invalid("six attachments per message"));
        }
        draft.add_files(files, now)
    }
    pub fn submit(&mut self, id: &str, run_id: &str, request_id: &str, now: u64) -> Result<Effect> {
        let target = self.target(id)?;
        if target.run.is_none()
            && self
                .sessions
                .values()
                .any(|s| s.run.as_ref().is_some_and(|r| r.id == run_id))
        {
            return Err(Error::Invalid("run identity already active"));
        }
        let current = self.session(id)?;
        if !current.applying
            && current.run.is_some()
            && target.run.as_deref() == Some(run_id)
            && current.draft.attachments().next().is_none()
            && !current.draft.legacy().0.trim().is_empty()
        {
            let waiting = current
                .trace
                .rows
                .values()
                .chain(
                    current
                        .conversation
                        .values()
                        .flat_map(|m| m["steps"].as_array().into_iter().flatten()),
                )
                .find(|q| {
                    q["kind"] == "question"
                        && q["status"] == "open"
                        && q["transport"] == "reply"
                        && q["blocking"] == true
                })
                .cloned();
            if let Some(question) = waiting {
                let text = current.draft.legacy().0.trim().to_owned();
                let answers = question["questions"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|q| q["id"].as_str().map(|id| (id.into(), vec![text.clone()])))
                    .collect();
                let effect = self.deliver_answer(
                    id,
                    question["id"]
                        .as_str()
                        .ok_or(Error::Invalid("question id"))?,
                    answers,
                    now,
                )?;
                self.session_mut(id)?.draft.clear_after_send();
                return Ok(effect);
            }
        }
        let session = self.sessions.get_mut(id).expect("target validated");
        if session.applying || !session.draft.sendable() {
            return Err(Error::Busy);
        }
        if run_id.is_empty()
            || request_id.is_empty()
            || target.run.is_none() && self.retired_runs.contains(run_id)
        {
            return Err(Error::Invalid("run identity"));
        }
        let files: Vec<_> = session
            .draft
            .attachments()
            .map(|f| f.record().clone())
            .collect();
        if let Some(file) = session.draft.attachments().find(|f| !f.payload_available()) {
            return Err(Error::MissingPayload(file.id().into()));
        }
        let text = session.draft.request_text();
        let steer = session.run.is_some();
        if steer && target.run.as_deref() != Some(run_id) {
            return Err(Error::Stale);
        }
        // Archive steering checkpoints before the next user turn, retaining all
        // reasoning and work. Tool updates can still find archived step ids.
        if steer {
            session.archive_trace(now, true)?;
        }
        session.append(json!({"role":"user","text":text,"attachments":files,"steering":steer}))?;
        session.archive_result()?;
        session.draft.clear_after_send();
        session.plan = None;
        session.result = None;
        session.result_message = None;
        if !steer {
            session.last_run = Some(run_id.into());
            session.run = Some(Run {
                id: run_id.into(),
                request_id: request_id.into(),
                started_at: now,
                sequence: 0,
                stopping: false,
            });
            session.progress.clear();
        }
        session.phase = Phase::Running;
        let settings = session.settings.clone();
        self.capture(id, now)?;
        let target = self.target(id)?;
        Ok(self.effect(
            target,
            EffectKind::Send {
                text,
                attachments: files,
                settings,
                steer,
            },
        ))
    }
    pub fn receive(
        &mut self,
        id: &str,
        run_id: &str,
        sequence: u64,
        update: Update,
        now: u64,
    ) -> Result<bool> {
        let s = self
            .sessions
            .get_mut(id)
            .ok_or_else(|| Error::UnknownId(id.into()))?;
        let run = s.run.as_ref().ok_or(Error::Stale)?;
        if run.id != run_id {
            return Err(Error::Stale);
        }
        if sequence <= run.sequence {
            return Ok(false);
        }
        if sequence != run.sequence + 1 {
            return Err(Error::Invalid("stream sequence gap"));
        }
        match update {
            Update::Trace(mut event) => {
                if event["kind"] == "question"
                    && event["transport"] == "reply"
                    && event["requestId"].as_str().is_none_or(str::is_empty)
                {
                    event["requestId"] = json!(s.run.as_ref().expect("checked run").request_id);
                }
                // Task/tool/question completions may target a sealed steering
                // checkpoint. Update there in place rather than duplicating it.
                let step_id = event["itemId"]
                    .as_str()
                    .or_else(|| event["task"]["taskId"].as_str())
                    .map(str::to_owned);
                let mut archived = false;
                if let Some(step_id) = &step_id
                    && s.trace.rows.get(step_id).is_none()
                {
                    let row = s
                        .conversation
                        .page(0..s.conversation.len())
                        .iter()
                        .find(|r| {
                            r.value["steps"]
                                .as_array()
                                .is_some_and(|a| a.iter().any(|v| v["id"] == *step_id))
                        })
                        .map(|r| r.id.clone());
                    if let Some(row) = row {
                        let value = s.conversation.get(&row).expect("found").value.clone();
                        let mut trace = Trace::default();
                        for step in value["steps"].as_array().expect("found") {
                            trace.rows.upsert(
                                step["id"].as_str().ok_or(Error::Invalid("step id"))?.into(),
                                step.clone(),
                            )?;
                        }
                        trace.receive(&event, now)?;
                        s.conversation.update(&row, |v| {
                            v["steps"] = json!(trace.rows.values().collect::<Vec<_>>())
                        })?;
                        archived = true;
                    }
                }
                if !archived {
                    s.trace.receive(&event, now)?;
                }
            }
            Update::Progress(text) => {
                s.activity = text.clone();
                if !text.is_empty() && s.progress.last() != Some(&text) {
                    s.progress.push(text);
                    if s.progress.len() > 20 {
                        s.progress.remove(0);
                    }
                }
            }
            Update::Complete {
                text,
                error,
                cancelled,
            } => {
                let spoke = s.trace.rows.values().any(|v| {
                    v["kind"] == "text" && v["text"].as_str().is_some_and(|s| !s.trim().is_empty())
                });
                s.archive_trace(now, false)?;
                let held: Vec<_> = s
                    .conversation
                    .values()
                    .flat_map(|m| m["steps"].as_array().into_iter().flatten())
                    .filter(|q| {
                        q["kind"] == "question"
                            && q["status"] == "open"
                            && q["transport"] == "reply"
                    })
                    .filter_map(|q| q["id"].as_str().map(str::to_owned))
                    .collect();
                for id in held {
                    s.change_question(&id, |q| q["status"] = json!("closed"))?;
                }
                // A failure is retained even if partial text was streamed.
                if error || !spoke && !text.is_empty() {
                    s.append(json!({"role":"assistant","text":text,"error":error}))?;
                }
                self.retired_runs.insert(run_id.into());
                s.run = None;
                s.activity.clear();
                s.phase = if s.plan.is_some() {
                    Phase::Preview
                } else if s.result.is_some() {
                    Phase::Result
                } else {
                    Phase::Prompt
                };
                s.stop_error = None;
                let record = self.archive.thread_mut(id).expect("session record");
                record["lastRunStatus"] = json!(if cancelled {
                    "cancelled"
                } else if error {
                    "failed"
                } else {
                    "completed"
                });
            }
            Update::Plan(plan) => {
                s.plan = Some(plan);
                s.phase = Phase::Preview;
            }
            Update::Result(result) => {
                s.archive_result()?;
                s.result_message = None;
                s.result = Some(result);
                s.phase = Phase::Result;
            }
        }
        if let Some(run) = &mut s.run {
            run.sequence = sequence;
        }
        if s.run.is_none()
            && let Some(replacement) = self.archives_after_stop.remove(id)
        {
            self.archive_thread(id, &replacement, true, now)?;
        }
        Ok(true)
    }
    pub fn stop(&mut self, id: &str) -> Result<Effect> {
        let target = self.target(id)?;
        let run = self
            .sessions
            .get_mut(id)
            .expect("validated")
            .run
            .as_mut()
            .ok_or(Error::Stale)?;
        if run.stopping {
            return Err(Error::Busy);
        }
        run.stopping = true;
        let effect = self.effect(target.clone(), EffectKind::Stop);
        self.pending.insert(effect.id, Pending::Stop(target));
        Ok(effect)
    }
    pub fn choose_answer(
        &mut self,
        thread: &str,
        step: &str,
        question: &str,
        option: &str,
    ) -> Result<()> {
        let s = self
            .sessions
            .get_mut(thread)
            .ok_or_else(|| Error::UnknownId(thread.into()))?;
        let value = s
            .question(step)
            .ok_or_else(|| Error::UnknownId(step.into()))?
            .clone();
        s.questions
            .entry(step.into())
            .or_default()
            .choose(&value, question, option)
    }
    pub fn type_answer(
        &mut self,
        thread: &str,
        step: &str,
        question: &str,
        text: String,
    ) -> Result<()> {
        let s = self
            .sessions
            .get_mut(thread)
            .ok_or_else(|| Error::UnknownId(thread.into()))?;
        let value = s
            .question(step)
            .ok_or_else(|| Error::UnknownId(step.into()))?
            .clone();
        s.questions
            .entry(step.into())
            .or_default()
            .type_answer(&value, question, text)
    }
    pub fn answer(&mut self, thread: &str, step: &str, skip: bool) -> Result<Effect> {
        let now = self
            .archive
            .thread(thread)
            .and_then(|v| v["updatedAt"].as_u64())
            .unwrap_or_default();
        self.answer_at(thread, step, skip, now)
    }
    pub fn answer_at(&mut self, thread: &str, step: &str, skip: bool, now: u64) -> Result<Effect> {
        let s = self.session(thread)?;
        let value = s
            .question(step)
            .ok_or_else(|| Error::UnknownId(step.into()))?;
        let answers = s
            .questions
            .get(step)
            .cloned()
            .unwrap_or_default()
            .submit(value, skip)?;
        self.deliver_answer(thread, step, answers, now)
    }
    fn deliver_answer(
        &mut self,
        thread: &str,
        step: &str,
        answers: Answers,
        now: u64,
    ) -> Result<Effect> {
        let target = self.target(thread)?;
        let s = self.session_mut(thread)?;
        let value = s
            .question(step)
            .ok_or_else(|| Error::UnknownId(step.into()))?
            .clone();
        if value["status"] != "open" || value["transport"] == "reply" && target.run.is_none() {
            return Err(Error::Stale);
        }
        let shown = shown_answers(&value, &answers);
        s.change_question(step, |v| {
            v["status"] = json!("answered");
            v["answers"] = shown;
        })?;
        s.questions.remove(step);
        let effect = if value["transport"] == "message" {
            // Exact tagged follow-up grammar, including private-answer masking.
            let questions = value["questions"]
                .as_array()
                .ok_or(Error::Invalid("questions"))?;
            let reply = questions
                .iter()
                .map(|q| {
                    let id = q["id"].as_str().unwrap_or_default();
                    let response = if q["secret"] == true {
                        "(answered privately)".into()
                    } else {
                        let a = answers.get(id).map(|a| a.join(", ")).unwrap_or_default();
                        if a.is_empty() { "No answer".into() } else { a }
                    };
                    format!(
                        "{}\n→ {}",
                        q["question"].as_str().unwrap_or_default(),
                        response
                    )
                })
                .collect::<Vec<_>>()
                .join("\n\n");
            let text = format!(
                "Answering your question{}:\n\n{}",
                if questions.len() == 1 { "" } else { "s" },
                reply
            );
            let draft = std::mem::take(&mut self.session_mut(thread)?.draft);
            self.session_mut(thread)?.draft = Draft::from_legacy(&text, &[])?;
            let (run, request) = if let Some(r) = &self.session(thread)?.run {
                (r.id.clone(), r.request_id.clone())
            } else {
                static NEXT_RUN: std::sync::atomic::AtomicU64 =
                    std::sync::atomic::AtomicU64::new(1);
                let id = NEXT_RUN.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                (
                    format!("agentpanel-reply-run-{id}"),
                    format!("agentpanel-reply-request-{id}"),
                )
            };
            let result = self.submit(thread, &run, &request, now);
            self.session_mut(thread)?.draft = draft;
            match result {
                Ok(effect) => effect,
                Err(e) => {
                    self.session_mut(thread)?
                        .change_question(step, |q| *q = value)?;
                    return Err(e);
                }
            }
        } else {
            self.effect(
                target,
                EffectKind::Answer {
                    question: step.into(),
                    request_id: value["requestId"].as_str().map(str::to_owned),
                    transport: "reply".into(),
                    answers: PrivateAnswers(answers),
                },
            )
        };
        self.pending.insert(
            effect.id,
            Pending::Answer {
                target: effect.target.clone(),
                question: step.into(),
            },
        );
        Ok(effect)
    }
    /// Effect completions carry the exact id issued by this model. Late replies
    /// cannot grant access to a newer run or acknowledge another thread's work.
    pub fn acknowledge(&mut self, id: u64, accepted: bool) -> Result<()> {
        let pending = self.pending.remove(&id).ok_or(Error::Stale)?;
        let target = match &pending {
            Pending::Answer { target, .. }
            | Pending::Consent(target)
            | Pending::Stop(target)
            | Pending::Task { target, .. }
            | Pending::Apply(target)
            | Pending::Undo(target)
            | Pending::Import { target, .. } => target,
        };
        let s = self.sessions.get_mut(&target.thread).ok_or(Error::Stale)?;
        let current_run = s.run.as_ref().map(|r| r.id.clone());
        let settled_host_action = matches!(
            &pending,
            Pending::Apply(_) | Pending::Undo(_) | Pending::Import { .. } | Pending::Answer { .. }
        ) && current_run.is_none()
            && target.run == s.last_run;
        if target.run != current_run && !settled_host_action {
            return Err(Error::Stale);
        }
        match pending {
            Pending::Answer { question, .. } if !accepted => s.change_question(&question, |v| {
                v["status"] = json!("closed");
                v.as_object_mut().expect("object").remove("answers");
            })?,
            Pending::Consent(_) if accepted => {
                s.settings.access = Access::Computer;
                s.settings_dirty = true;
                s.access_dirty = true;
            }
            Pending::Stop(_) if !accepted => {
                if let Some(run) = &mut s.run {
                    run.stopping = false;
                }
                s.stop_error = Some("stop_failed".into());
            }
            Pending::Task { task, .. } => {
                s.disclosures.remove(&format!("stopping:{task}"));
                if !accepted {
                    s.stop_error = Some(task);
                }
            }
            Pending::Apply(_) => {
                s.applying = false;
                s.phase = if accepted {
                    s.plan = None;
                    Phase::Result
                } else {
                    Phase::Preview
                };
            }
            Pending::Undo(_) if accepted => {
                s.archive_result()?;
                s.result_message = None;
                s.result = None;
                s.phase = Phase::Prompt;
            }
            Pending::Import { path, .. } => {
                if let Some(result) = &mut s.result
                    && let Some(a) = result["artifacts"]
                        .as_array_mut()
                        .and_then(|a| a.iter_mut().find(|v| v["path"] == path))
                {
                    a["importing"] = json!(false);
                    a["imported"] = json!(accepted);
                }
            }
            _ => {}
        }
        Ok(())
    }
    pub fn set_provider(
        &mut self,
        id: &str,
        provider: Provider,
        model: String,
        catalog: Catalog,
    ) -> Result<()> {
        let s = self
            .sessions
            .get_mut(id)
            .ok_or_else(|| Error::UnknownId(id.into()))?;
        if s.run.is_some() || s.applying {
            return Err(Error::Busy);
        }
        if !catalog.providers.iter().any(|(p, _)| p == provider.name()) {
            return Err(Error::Invalid("provider"));
        }
        if !(catalog.models.iter().any(|(m, _)| *m == model)
            || provider == Provider::Claude && crate::input::valid_claude_id(&model))
        {
            return Err(Error::Invalid("provider model"));
        }
        s.settings_dirty = true;
        s.settings.provider = provider;
        s.settings.model = model;
        s.settings.effort.clear();
        s.catalog = catalog;
        Ok(())
    }
    pub fn set_model(&mut self, id: &str, model: String, effort: String) -> Result<()> {
        let s = self
            .sessions
            .get_mut(id)
            .ok_or_else(|| Error::UnknownId(id.into()))?;
        if !s.catalog.efforts.contains(&effort)
            || !(s.catalog.models.iter().any(|(m, _)| *m == model)
                || s.settings.provider == Provider::Claude && crate::input::valid_claude_id(&model))
        {
            return Err(Error::Invalid("model or effort"));
        }
        s.settings_dirty = true;
        s.settings.model = model;
        s.settings.effort = effort;
        Ok(())
    }
    pub fn set_catalog(&mut self, id: &str, catalog: Catalog) -> Result<()> {
        self.sessions
            .get_mut(id)
            .ok_or_else(|| Error::UnknownId(id.into()))?
            .catalog = catalog;
        Ok(())
    }
    pub fn set_access(&mut self, id: &str, permission: &str) -> Result<Option<Effect>> {
        let target = self.target(id)?;
        if permission == "full" {
            if self.session(id)?.settings.access == Access::Computer {
                return Ok(None);
            }
            if self
                .pending
                .values()
                .any(|p| matches!(p, Pending::Consent(t) if t.thread == id))
            {
                return Err(Error::Busy);
            }
            let effect = self.effect(target.clone(), EffectKind::ComputerConsent);
            self.pending.insert(effect.id, Pending::Consent(target));
            return Ok(Some(effect));
        }
        let s = self.sessions.get_mut(id).expect("validated");
        let revoke = s.settings.access == Access::Computer;
        match permission {
            "editor" => s.settings.access = Access::Editor,
            "supervised" | "auto" | "project" => {
                s.settings.access = Access::Project;
                if permission != "project" {
                    s.settings.approval = if permission == "supervised" {
                        ApprovalMode::Supervised
                    } else {
                        ApprovalMode::Auto
                    };
                }
            }
            _ => return Err(Error::Invalid("access permission")),
        }
        s.settings_dirty = true;
        s.access_dirty = true;
        // Invalidate pending consent when the person's choice changes.
        self.pending
            .retain(|_, p| !matches!(p, Pending::Consent(t) if t.thread == id));
        Ok(revoke.then(|| self.effect(target, EffectKind::RevokeComputerConsent)))
    }
    pub fn set_scope(&mut self, id: &str, scope: &str, registered: &[String]) -> Result<()> {
        let s = self
            .sessions
            .get_mut(id)
            .ok_or_else(|| Error::UnknownId(id.into()))?;
        s.settings_dirty = true;
        s.settings.scope = normalize_scope(scope, registered);
        Ok(())
    }
    pub fn toggle_auto_apply_panels(&mut self, id: &str) -> Result<()> {
        let s = self
            .sessions
            .get_mut(id)
            .ok_or_else(|| Error::UnknownId(id.into()))?;
        s.settings_dirty = true;
        s.settings.auto_apply_panels = !s.settings.auto_apply_panels;
        Ok(())
    }
    pub fn disclose(&mut self, thread: &str, row: &str, open: bool) -> Result<()> {
        let s = self
            .sessions
            .get_mut(thread)
            .ok_or_else(|| Error::UnknownId(thread.into()))?;
        if s.trace.rows.get(row).is_none() && s.conversation.get(row).is_none() {
            return Err(Error::UnknownId(row.into()));
        }
        s.disclosures.insert(row.into(), open);
        Ok(())
    }
    pub fn apply_plan(&mut self, thread: &str) -> Result<Effect> {
        self.no_apply()?;
        let target = self.target(thread)?;
        let s = self.sessions.get_mut(thread).expect("validated");
        let plan = s.plan.clone().ok_or(Error::Stale)?;
        s.applying = true;
        let effect = self.effect(target.clone(), EffectKind::ApplyPlan(plan));
        self.pending.insert(effect.id, Pending::Apply(target));
        Ok(effect)
    }
    pub fn dismiss_plan(&mut self, thread: &str) -> Result<()> {
        let s = self
            .sessions
            .get_mut(thread)
            .ok_or_else(|| Error::UnknownId(thread.into()))?;
        if s.applying {
            return Err(Error::Busy);
        }
        s.plan = None;
        s.phase = if s.run.is_some() {
            Phase::Running
        } else {
            Phase::Prompt
        };
        Ok(())
    }
    pub fn keep_result(&mut self, thread: &str) -> Result<()> {
        let s = self
            .sessions
            .get_mut(thread)
            .ok_or_else(|| Error::UnknownId(thread.into()))?;
        s.archive_result()?;
        s.result = None;
        s.result_message = None;
        s.phase = if s.run.is_some() {
            Phase::Running
        } else {
            Phase::Prompt
        };
        Ok(())
    }
    pub fn undo_result(&mut self, thread: &str) -> Result<Effect> {
        let target = self.target(thread)?;
        let result = self.session(thread)?.result.clone().ok_or(Error::Stale)?;
        let effect = self.effect(target.clone(), EffectKind::UndoResult(result));
        self.pending.insert(effect.id, Pending::Undo(target));
        Ok(effect)
    }
    pub fn stop_task(&mut self, thread: &str, row: &str) -> Result<Effect> {
        let target = self.target(thread)?;
        let s = self.sessions.get_mut(thread).expect("validated");
        let value = s
            .step(row)
            .ok_or_else(|| Error::UnknownId(row.into()))?
            .clone();
        if value["status"] != "running" || !value["task"].is_object() {
            return Err(Error::Stale);
        }
        if s.disclosures.contains_key(&format!("stopping:{row}")) {
            return Err(Error::Busy);
        }
        let request_id = value["task"]["requestId"]
            .as_str()
            .ok_or(Error::Invalid("task request"))?
            .into();
        let task = value["task"]["taskId"]
            .as_str()
            .ok_or(Error::Invalid("task id"))?
            .into();
        s.disclosures.insert(format!("stopping:{row}"), true);
        let effect = self.effect(target.clone(), EffectKind::StopTask { task, request_id });
        self.pending.insert(
            effect.id,
            Pending::Task {
                target,
                task: row.into(),
            },
        );
        Ok(effect)
    }
    pub fn open_attachment(&mut self, thread: &str, id: &str) -> Result<Effect> {
        let target = self.target(thread)?;
        let file = self
            .session(thread)?
            .draft
            .attachment(id)
            .ok_or_else(|| Error::UnknownId(id.into()))?;
        if !file.previewable()
            && !file.payload_available()
            && !file.record()["sourcePath"].is_string()
        {
            return Err(Error::MissingPayload(id.into()));
        }
        let kind = EffectKind::OpenAttachment {
            attachment: file.record().clone(),
            preview: file.previewable(),
            restore_focus: id.into(),
        };
        Ok(self.effect(target, kind))
    }
    pub fn artifact(&mut self, thread: &str, path: &str, import: bool) -> Result<Effect> {
        let target = self.target(thread)?;
        let s = self.sessions.get_mut(thread).expect("validated");
        let artifact = s
            .result
            .as_mut()
            .and_then(|r| r["artifacts"].as_array_mut())
            .and_then(|a| a.iter_mut().find(|v| v["path"] == path))
            .ok_or_else(|| Error::UnknownId(path.into()))?;
        if import && (artifact["importing"] == true || artifact["imported"] == true) {
            return Err(Error::Busy);
        }
        if import {
            artifact["importing"] = json!(true);
        }
        let kind = if import {
            EffectKind::ImportArtifact(artifact.clone())
        } else {
            EffectKind::RevealArtifact(artifact.clone())
        };
        let effect = self.effect(target.clone(), kind);
        if import {
            self.pending.insert(
                effect.id,
                Pending::Import {
                    target,
                    path: path.into(),
                },
            );
        }
        Ok(effect)
    }
    /// Retry refers to a stable message id, never a mutable rendered index.
    pub fn retry(
        &mut self,
        thread: &str,
        message: Option<&str>,
        continue_with_project: bool,
    ) -> Result<Effect> {
        let target = self.target(thread)?;
        let s = self.session(thread)?;
        if s.run.is_some() {
            return Err(Error::Busy);
        }
        let end = if let Some(id) = message {
            let row = s
                .conversation
                .get(id)
                .ok_or_else(|| Error::UnknownId(id.into()))?;
            if continue_with_project && row.value["requiresProject"] != true {
                return Err(Error::Invalid("project continuation"));
            }
            s.conversation
                .page(0..s.conversation.len())
                .iter()
                .position(|r| r.id == id)
                .expect("found")
        } else {
            s.conversation.len()
        };
        let user = s
            .conversation
            .page(0..end)
            .iter()
            .rev()
            .find(|r| r.value["role"] == "user")
            .ok_or(Error::Invalid("retry has no user turn"))?
            .id
            .clone();
        Ok(self.effect(
            target,
            EffectKind::Retry {
                message: user,
                continue_with_project,
            },
        ))
    }
    pub fn host_settings(&mut self, thread: &str) -> Result<Effect> {
        let target = self.target(thread)?;
        Ok(self.effect(target, EffectKind::OpenSettings))
    }
    pub fn host_action(&mut self, thread: &str, action: ShellAction) -> Result<Effect> {
        let target = self.target(thread)?;
        Ok(self.effect(target, EffectKind::Shell(action)))
    }
}
fn host(e: io::Error) -> Error {
    Error::Host(e.to_string())
}
/// Source: panel-focus.ts:24–40. Registered hidden panels remain valid targets.
pub fn normalize_scope(scope: &str, registered: &[String]) -> String {
    if scope == "composition" {
        return scope.into();
    }
    let Some(ids) = scope
        .strip_prefix("panel:")
        .or_else(|| scope.strip_prefix("panels:"))
    else {
        return "workspace".into();
    };
    let mut valid = vec![];
    let mut seen = HashSet::new();
    for id in ids
        .split(',')
        .filter(|s| !s.is_empty())
        .filter(|id| seen.insert(*id))
        .take(32)
    {
        if registered.iter().any(|s| s == id) {
            valid.push(id);
        }
    }
    if valid.is_empty() {
        "workspace".into()
    } else {
        format!(
            "{}:{}",
            if valid.len() == 1 { "panel" } else { "panels" },
            valid.join(",")
        )
    }
}

impl Panel {
    /// Small machine-readable inspection surface. Payloads and private answer
    /// drafts are intentionally separate; row pages are requested by id/range.
    pub fn inspect(&self, thread: &str) -> Result<Value> {
        let s = self.session(thread)?;
        Ok(
            json!({"projectId":self.project,"threadId":thread,"phase":format!("{:?}",s.phase).to_lowercase(),"runId":s.run.as_ref().map(|r|&r.id),"messageCount":s.conversation.len(),"liveRowCount":s.trace.rows.len(),"backgroundRuns":self.background_runs(),"sendable":s.draft.sendable(),"composerDisabled":s.composer_disabled(),"applying":s.applying,"archiving":s.archiving,"stopError":s.stop_error,"saveError":self.save_error,"attachments":s.draft.attachments().map(|a|json!({"id":a.id(),"name":a.name(),"type":a.record()["type"],"payloadAvailable":a.payload_available()})).collect::<Vec<_>>()}),
        )
    }
    /// Source: spatial.ts:710–727, threads.ts:100–108. An old asynchronous title
    /// response cannot rename a conversation with a different original request.
    pub fn generated_title(
        &mut self,
        thread: &str,
        first_request: &str,
        value: &str,
        now: u64,
    ) -> Result<bool> {
        let s = self.session(thread)?;
        let original = s
            .conversation
            .values()
            .find(|m| m["role"] == "user")
            .and_then(|m| m["text"].as_str());
        if original != Some(first_request) {
            return Ok(false);
        }
        let flat = value.split_whitespace().collect::<Vec<_>>().join(" ");
        let trimmed = flat.trim_matches(|c: char| c.is_whitespace() || "#>*`\"“”".contains(c));
        let title = pm_agents::trace::truncate(trimmed, 64).trim().to_owned();
        if title.is_empty() {
            return Ok(false);
        }
        let record = self.archive.thread_mut(thread).ok_or(Error::Stale)?;
        record["title"] = json!(title);
        record["updatedAt"] = json!(now);
        Ok(true)
    }
}

impl Panel {
    /// Agent/MCP follow-up without replacing the person's visible draft, files,
    /// selection or local Undo. It uses the exact same send/steer admission.
    pub fn send_message(
        &mut self,
        thread: &str,
        text: &str,
        files: &[Value],
        run: &str,
        request: &str,
        now: u64,
    ) -> Result<Effect> {
        let message = Draft::from_legacy(text, files)?;
        let s = self.session_mut(thread)?;
        let draft = std::mem::replace(&mut s.draft, message);
        let result = self.submit(thread, run, request, now);
        self.session_mut(thread)?.draft = draft;
        result
    }
    /// Ordinary background conversation creation never selects/reuses the visible
    /// blank thread. Source: spatial.ts thread_launch path, orchestration-spec.ts:52.
    /// Computer consent is intentionally not inherited (one-run authority).
    pub fn launch_background(&mut self, id: &str, now: u64) -> Result<()> {
        self.no_apply()?;
        if id.is_empty()
            || id.len() > 120
            || !id
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-')
        {
            return Err(Error::Invalid("thread id"));
        }
        if self.archive.raw()["threads"]
            .as_array()
            .into_iter()
            .flatten()
            .chain(
                self.archive.raw()["archivedThreads"]
                    .as_array()
                    .into_iter()
                    .flatten(),
            )
            .any(|t| t["id"] == id)
        {
            return Err(Error::Invalid("duplicate thread id"));
        }
        let active = self.session(self.active_id())?;
        let mut settings = active.settings.clone();
        if settings.access == Access::Computer {
            settings.access = Access::Project;
        }
        let catalog = active.catalog.clone();
        let record = pm_agents::persistence::new_thread(id, now);
        let mut session = Session::from_record(&record)?;
        session.settings = settings;
        session.settings_dirty = true;
        session.catalog = catalog;
        let mut archive = self.archive.raw().clone();
        archive["threads"]
            .as_array_mut()
            .ok_or(Error::Invalid("threads"))?
            .insert(0, record);
        struct Seed(Value);
        impl Store for Seed {
            fn get(&mut self, _: &str) -> io::Result<Option<Value>> {
                Ok(Some(self.0.clone()))
            }
            fn set(&mut self, _: &str, _: &Value) -> io::Result<()> {
                Err(io::Error::other("read_only_seed"))
            }
        }
        let archive = Threads::load(&mut Seed(archive), &self.project, id, now).map_err(host)?;
        self.archive = archive;
        self.sessions.insert(id.into(), session);
        Ok(())
    }
}
