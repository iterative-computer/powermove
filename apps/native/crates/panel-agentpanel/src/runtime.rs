//! Concrete process-to-panel bridge for the application's serial storage worker.
//! Uses real pm-agents handles, bounded queues and durable panel publication.
//! Shell registration, workspace staging and consent remain host-owned.
use crate::{
    Error, Result,
    model::{Effect, EffectKind, Panel, Target},
};
use pm_agents::{
    Event,
    history::Receipt,
    persistence::Store,
    transport::{Action, Handle},
};
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
}
#[derive(Default)]
pub struct ProviderRuns {
    runs: Vec<Running>,
    cursor: usize,
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
        if self.runs.len() >= 8
            || self.runs.iter().any(|r| {
                r.target.thread == effect.target.thread && r.target.project == effect.target.project
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
                    event: Event::Stopped(format!("{error:?}")),
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
        });
        Ok(())
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
            if running.pending.is_none() {
                running.pending = match running.handle.try_event() {
                    Ok(event) => Some(event),
                    Err(TryRecvError::Empty) => None,
                    Err(TryRecvError::Disconnected) => {
                        Some(Event::Stopped("Agent runtime stopped.".into()))
                    }
                };
            }
            if let Some(event) = &running.pending {
                let stopped = matches!(event, Event::Stopped(_));
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
        Ok(delivered)
    }
}
