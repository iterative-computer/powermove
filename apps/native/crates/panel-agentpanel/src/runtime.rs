//! Concrete process-to-panel bridge for the application's serial storage worker.
//! Uses real pm-agents handles, bounded queues and durable panel publication.
//! Shell registration, workspace staging and consent remain host-owned.
use crate::{
    Error, Result,
    model::{Effect, EffectKind, Panel, Target},
};
use pm_agents::{
    Event, RunResult,
    history::Receipt,
    persistence::Store,
    transport::{Action, Handle},
};
use std::collections::VecDeque;
use std::sync::mpsc::{Receiver, TryRecvError, TrySendError};

struct PendingAction {
    effect: Option<u64>,
    pending: Receiver<bool>,
    result: Option<bool>,
}
struct Running {
    target: Target,
    handle: Handle,
    sequence: u64,
    pending: Option<Event>,
    action: Option<PendingAction>,
    /// The run settled. Its process only has late cleanup left to report.
    finished: bool,
    /// The run delegated or watched: its turn's completion waits for the
    /// host (main's after-turn barrier) instead of settling the run.
    orchestrated: bool,
    /// A finished turn the host hasn't released or continued. Its process
    /// was shut down; nothing more is read from it.
    held: Option<Event>,
    release: bool,
    /// Host events (task rows, progress) delivered before the provider's.
    injected: VecDeque<Event>,
}
/// What the host may tell the person outside the app (S114): a run was
/// sent, finished, or asked a question.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Notice {
    Sent {
        thread: String,
    },
    Finished {
        thread: String,
        run: String,
        /// Stopped by the person: never notified.
        stopped: bool,
    },
    Question {
        thread: String,
        run: String,
        item: String,
        text: String,
    },
}
#[derive(Default)]
pub struct ProviderRuns {
    runs: Vec<Running>,
    cursor: usize,
    ended: Vec<(String, RunResult)>,
    settled: Vec<(String, RunResult)>,
    notices: Vec<Notice>,
}
impl ProviderRuns {
    pub fn len(&self) -> usize {
        self.runs.len()
    }
    pub fn is_empty(&self) -> bool {
        self.runs.is_empty()
    }
    /// Persist the original prompt/run/settings before starting any provider.
    /// The factory must launch a native worker (e.g. claude_launch::Options::launch
    /// or transport::spawn); it is not called if storage publication fails.
    pub fn start(
        &mut self,
        panel: &mut Panel,
        store: &mut impl Store,
        effect: &Effect,
        now: u64,
        launch: impl FnOnce() -> Result<Handle>,
    ) -> Result<()> {
        let EffectKind::Send { steer: false, .. } = &effect.kind else {
            return Err(Error::Invalid("provider start effect"));
        };
        Self::validate(panel, &effect.target)?;
        if self.active() >= 8
            || self.runs.iter().any(|r| {
                !r.finished
                    && r.target.thread == effect.target.thread
                    && r.target.project == effect.target.project
            })
        {
            return Err(Error::Busy);
        }
        panel.save(store, now)?;
        let handle = match launch() {
            Ok(handle) => handle,
            Err(error) => {
                let receipt = Receipt {
                    source: effect.target.run.clone().ok_or(Error::Stale)?,
                    sequence: 0,
                    event: Event::Stopped(error.to_string()),
                };
                panel.provider_event(store, &effect.target.thread, &receipt, now)?;
                return Err(error);
            }
        };
        self.runs.push(Running {
            target: effect.target.clone(),
            handle,
            sequence: 0,
            pending: None,
            action: None,
            finished: false,
            orchestrated: false,
            held: None,
            release: false,
            injected: VecDeque::new(),
        });
        self.notices.push(Notice::Sent {
            thread: effect.target.thread.clone(),
        });
        Ok(())
    }
    /// Notices since the last call, oldest first.
    pub fn take_notices(&mut self) -> Vec<Notice> {
        std::mem::take(&mut self.notices)
    }
    fn live_mut(&mut self, run: &str) -> Option<&mut Running> {
        self.runs
            .iter_mut()
            .find(|r| !r.finished && r.target.run.as_deref() == Some(run))
    }
    /// The unfinished run's target (its project and thread).
    pub fn live(&self, run: &str) -> Option<&Target> {
        self.runs
            .iter()
            .find(|r| !r.finished && r.target.run.as_deref() == Some(run))
            .map(|r| &r.target)
    }
    /// Hold this run's turn completions for the host (see `take_ended`).
    pub fn orchestrate(&mut self, run: &str) -> bool {
        self.live_mut(run).map(|r| r.orchestrated = true).is_some()
    }
    /// Turns of orchestrated runs that finished since the last call: the host
    /// releases each (`release`) or starts its next turn (`resume`).
    pub fn take_ended(&mut self) -> Vec<(String, RunResult)> {
        std::mem::take(&mut self.ended)
    }
    /// Runs whose result reached the conversation since the last call.
    pub fn take_settled(&mut self) -> Vec<(String, RunResult)> {
        std::mem::take(&mut self.settled)
    }
    pub fn held(&self, run: &str) -> bool {
        self.runs
            .iter()
            .any(|r| r.held.is_some() && r.target.run.as_deref() == Some(run))
    }
    /// Deliver the held completion: the run settles as its last turn said.
    pub fn release(&mut self, run: &str) {
        if let Some(running) = self.live_mut(run).filter(|r| r.held.is_some()) {
            running.release = true;
        }
    }
    /// The run's next turn (a continuation) runs in a new process.
    pub fn resume(&mut self, run: &str, handle: Handle) -> Result<()> {
        let running = self
            .live_mut(run)
            .filter(|r| r.held.is_some())
            .ok_or(Error::Stale)?;
        running.held = None;
        running.release = false;
        running.handle = handle;
        Ok(())
    }
    /// Queue a host event (task row, progress) on the run's conversation.
    pub fn inject(&mut self, run: &str, event: Event) -> bool {
        self.live_mut(run)
            .map(|r| r.injected.push_back(event))
            .is_some()
    }
    /// Settle an effect the host could not carry out, so the panel never waits
    /// on a process that will not answer. A start that never launched fails its
    /// run in the conversation; a refused Stop or answer is acknowledged as such.
    pub fn refuse(
        &mut self,
        panel: &mut Panel,
        store: &mut impl Store,
        effect: &Effect,
        now: u64,
        error: &Error,
    ) -> Result<()> {
        match &effect.kind {
            EffectKind::Send { steer: false, .. } => {
                let run = effect.target.run.clone().ok_or(Error::Stale)?;
                if self
                    .runs
                    .iter()
                    .any(|r| r.target.run.as_ref() == Some(&run))
                {
                    return Ok(());
                }
                let receipt = Receipt {
                    source: run,
                    sequence: 0,
                    event: Event::Stopped(error.to_string()),
                };
                panel
                    .provider_event(store, &effect.target.thread, &receipt, now)
                    .map(drop)
            }
            EffectKind::Stop | EffectKind::Answer { .. } => {
                match panel.acknowledge(effect.id, false) {
                    Ok(()) | Err(Error::Stale) => {}
                    Err(other) => return Err(other),
                }
                panel.save(store, now)
            }
            _ => Ok(()),
        }
    }
    /// Cancel every unfinished run. Each settles through its usual events.
    pub fn cancel_all(&mut self) {
        for running in self.runs.iter_mut().filter(|r| !r.finished) {
            if let Some(run) = running.target.run.clone() {
                if running.held.is_some() {
                    running.held = Some(Event::Completed {
                        run,
                        result: RunResult::failure("The run was stopped.", true),
                    });
                    running.release = true;
                } else {
                    let _ = running.handle.send(Action::Cancel(run));
                }
            }
        }
    }
    /// Only App Server takes more input during a turn.
    pub fn steerable(&self, run: &str) -> bool {
        self.runs.iter().any(|r| {
            !r.finished
                && r.held.is_none()
                && r.target.run.as_deref() == Some(run)
                && r.handle.supports_steering()
        })
    }
    /// Unfinished runs, which is what the thread lease and the UI care about.
    pub fn active(&self) -> usize {
        self.runs.iter().filter(|r| !r.finished).count()
    }
    fn validate(panel: &Panel, target: &Target) -> Result<()> {
        if panel.project() != target.project
            || target.run.is_none()
            || panel.session(&target.thread)?.run.as_ref().map(|r| &r.id) != target.run.as_ref()
        {
            return Err(Error::Stale);
        }
        Ok(())
    }
    /// Enqueue only. A successful Stop ack never publishes a terminal result;
    /// the actual interrupt/settlement event completes the panel's run.
    pub fn dispatch(
        &mut self,
        panel: &mut Panel,
        store: &mut impl Store,
        effect: &Effect,
        now: u64,
    ) -> Result<()> {
        Self::validate(panel, &effect.target)?;
        let running = self
            .runs
            .iter_mut()
            .find(|r| r.target == effect.target)
            .ok_or(Error::Stale)?;
        if running.action.is_some() {
            return Err(Error::Busy);
        }
        let run = effect.target.run.clone().ok_or(Error::Stale)?;
        // Waiting for its subagents, the run has no process: Stop settles it.
        if running.held.is_some() {
            if !matches!(effect.kind, EffectKind::Stop) {
                return Err(Error::Invalid("run is waiting for its subagents"));
            }
            let result = RunResult::failure("The run was stopped.", true);
            running.held = Some(Event::Completed {
                run: run.clone(),
                result: result.clone(),
            });
            // The host stops its subagents, then releases it.
            self.ended.push((run, result));
            panel.acknowledge(effect.id, true)?;
            return panel.save(store, now);
        }
        let (action, ack) = match &effect.kind {
            EffectKind::Stop => (Action::Cancel(run), true),
            EffectKind::Answer {
                question,
                answers,
                transport,
                ..
            } if transport == "reply" => (
                Action::Answer {
                    run,
                    item: question.clone(),
                    answers: answers.0.clone(),
                },
                true,
            ),
            EffectKind::Send {
                text,
                steer: true,
                attachments,
                ..
            } if attachments.is_empty() && running.handle.supports_steering() => (
                Action::Steer {
                    run,
                    prompt: text.clone(),
                    images: vec![],
                },
                false,
            ),
            _ => return Err(Error::Invalid("unsupported provider effect")),
        };
        panel.save(store, now)?;
        match running.handle.send_checked(action) {
            Ok(pending) => {
                running.action = Some(PendingAction {
                    effect: ack.then_some(effect.id),
                    pending,
                    result: None,
                });
                Ok(())
            }
            Err(TrySendError::Full(_)) => Err(Error::Busy),
            Err(TrySendError::Disconnected(_)) => {
                Err(Error::Host("provider worker disconnected".into()))
            }
        }
    }

    /// Poll at most `budget` observations in stable run admission order. Failed
    /// durability retains the exact event and position, blocking later delivery
    /// until retry; no publication/receipt is acknowledged or reconstructed.
    pub fn poll(
        &mut self,
        panel: &mut Panel,
        store: &mut impl Store,
        now: u64,
        budget: usize,
    ) -> Result<usize> {
        let mut delivered = 0;
        let mut empty = 0;
        while delivered < budget.min(256) && !self.runs.is_empty() {
            self.cursor %= self.runs.len();
            let running = &mut self.runs[self.cursor];
            if let Some(action) = &mut running.action {
                if action.result.is_none() {
                    action.result = match action.pending.try_recv() {
                        Ok(value) => Some(value),
                        Err(TryRecvError::Disconnected) => Some(false),
                        Err(TryRecvError::Empty) => None,
                    };
                }
                if let Some(accepted) = action.result {
                    if let Some(effect) = action.effect {
                        let mut next = panel.clone();
                        next.acknowledge(effect, accepted)?;
                        next.save(store, now)?;
                        *panel = next;
                    }
                    running.action = None;
                    if !accepted {
                        return Err(Error::Host("provider rejected action".into()));
                    }
                } else {
                    self.cursor += 1;
                    empty += 1;
                    if empty >= self.runs.len() {
                        break;
                    }
                    continue;
                }
            }
            let mut releasing = false;
            if running.pending.is_none() {
                if let Some(event) = running.injected.pop_front() {
                    running.pending = Some(event);
                } else if running.held.is_some() {
                    if running.release {
                        running.release = false;
                        running.pending = running.held.take();
                        releasing = true;
                    } else {
                        self.cursor += 1;
                        empty += 1;
                        if empty >= self.runs.len() {
                            break;
                        }
                        continue;
                    }
                }
            }
            if running.pending.is_none() {
                running.pending = match running.handle.try_event() {
                    Ok(event) => Some(event),
                    Err(TryRecvError::Empty) => None,
                    Err(TryRecvError::Disconnected) => {
                        Some(Event::Stopped("Agent runtime stopped.".into()))
                    }
                };
            }
            if let Some(Event::Completed { result, .. }) = &running.pending
                && running.orchestrated
                && !releasing
                && running.held.is_none()
            {
                // The host decides: deliver, wait for subagents, or continue.
                let run = running.target.run.clone().ok_or(Error::Stale)?;
                self.ended.push((run, result.clone()));
                running.held = running.pending.take();
                let _ = running.handle.send(Action::Shutdown);
                self.cursor += 1;
                continue;
            }
            if let Some(event) = &running.pending {
                let stopped = matches!(event, Event::Stopped(_));
                let completed = matches!(event, Event::Completed { .. });
                let run = running.target.run.clone().ok_or(Error::Stale)?;
                let thread = running.target.thread.clone();
                match event {
                    Event::Completed { result, .. } if !running.finished => {
                        self.notices.push(Notice::Finished {
                            thread,
                            run: run.clone(),
                            stopped: matches!(
                                result,
                                RunResult::Failure {
                                    cancelled: true,
                                    ..
                                }
                            ),
                        });
                        self.settled.push((run, result.clone()));
                    }
                    Event::Stopped(reason) if !running.finished => {
                        self.settled
                            .push((run, RunResult::failure(reason.clone(), false)));
                    }
                    Event::Trace { value, .. } if value["kind"] == "question" => {
                        let text = value["questions"]
                            .as_array()
                            .and_then(|q| q.first())
                            .and_then(|q| q["question"].as_str())
                            .unwrap_or_default();
                        if let Some(item) = value["itemId"].as_str() {
                            self.notices.push(Notice::Question {
                                thread,
                                run,
                                item: item.into(),
                                text: text.into(),
                            });
                        }
                    }
                    _ => {}
                }
                let receipt = Receipt {
                    source: running.target.run.clone().ok_or(Error::Stale)?,
                    sequence: running.sequence,
                    event: event.clone(),
                };
                panel.provider_event(store, &running.target.thread, &receipt, now)?;
                running.pending = None;
                running.sequence += 1;
                delivered += 1;
                empty = 0;
                if completed {
                    // App Server stays up after a turn; this run owns the process.
                    running.finished = true;
                    let _ = running.handle.send(Action::Shutdown);
                }
                if stopped {
                    self.runs.remove(self.cursor);
                } else {
                    self.cursor += 1;
                }
            } else {
                self.cursor += 1;
                empty += 1;
                if empty >= self.runs.len() {
                    break;
                }
            }
        }
        panel.checkpoint(store, now)?;
        Ok(delivered)
    }
}
