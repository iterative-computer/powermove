//! Legacy Codex transport used only if the App Server never initializes.
use super::{Action, Handle, Machine, ProcessSpec, QueuedAction, spawn_process};
use crate::{Effects, Event, RunResult, codex};
use std::{
    sync::mpsc::{Receiver, RecvTimeoutError, SyncSender, TryRecvError},
    time::Duration,
};

pub(crate) struct Direction {
    pub prompt: String,
    pub images: Vec<String>,
}

type LaunchTurn = Box<dyn FnMut(Option<String>, Vec<Direction>) -> Result<Handle, String> + Send>;

pub(super) struct StartupFallback {
    pub run_id: String,
    pub directions: Vec<Direction>,
    launch_turn: LaunchTurn,
}

pub(crate) fn spawn_project(
    spec: ProcessSpec,
    run: codex::Run,
    launch_turn: impl FnMut(Option<String>, Vec<Direction>) -> Result<Handle, String> + Send + 'static,
) -> Result<Handle, String> {
    let run_id = run.id.clone();
    let mut client = codex::Client::default();
    let initial = client.run(run, 0)?;
    let fallback = StartupFallback {
        run_id,
        directions: Vec::new(),
        launch_turn: Box::new(launch_turn),
    };
    Ok(spawn_process(
        spec,
        Machine::Codex(client),
        initial,
        Some(fallback),
    ))
}

pub(super) fn startup_error<'a>(
    fallback: &mut Option<StartupFallback>,
    effects: &'a Effects,
) -> Option<&'a str> {
    fallback.as_ref()?;
    if effects.events.contains(&Event::Ready) {
        *fallback = None;
        return None;
    }
    effects.events.iter().find_map(|event| match event {
        Event::Stopped(reason) => Some(reason.as_str()),
        _ => None,
    })
}

impl StartupFallback {
    pub(super) fn run(
        mut self,
        actions: Receiver<QueuedAction>,
        events: SyncSender<Event>,
        reason: &str,
    ) {
        // A cancelled startup or a closed panel should never open another worker.
        loop {
            match actions.try_recv() {
                Ok(QueuedAction {
                    action: Action::Cancel(run),
                    reply,
                }) if run == self.run_id => {
                    if let Some(reply) = reply {
                        let _ = reply.try_send(true);
                    }
                    let _ = events.send(Event::Completed {
                        run,
                        result: RunResult::failure("The Codex run was cancelled.", true),
                    });
                    return;
                }
                Ok(action) => {
                    if !self.queue(action) {
                        return;
                    }
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => return,
            }
        }
        eprintln!(
            "Codex App Server failed to initialize, using the CLI: {}",
            crate::trace::truncate(reason, 600)
        );
        if let Err(error) = self.drive(&actions, &events) {
            let _ = events.send(Event::Stopped(error));
        }
    }

    fn drive(
        &mut self,
        actions: &Receiver<QueuedAction>,
        events: &SyncSender<Event>,
    ) -> Result<(), String> {
        let mut handle = (self.launch_turn)(None, Vec::new())?;
        let mut session = None;
        loop {
            match actions.recv_timeout(Duration::from_millis(10)) {
                Ok(QueuedAction {
                    action: Action::Cancel(run),
                    reply,
                }) if run == self.run_id => {
                    self.directions.clear();
                    let sent = handle.enqueue(Action::Cancel(run), reply.clone()).is_ok();
                    if !sent && let Some(reply) = reply {
                        let _ = reply.try_send(false);
                    }
                }
                Ok(action) => {
                    if !self.queue(action) {
                        return Ok(());
                    }
                    // Read all waiting directions before choosing the next turn.
                    continue;
                }
                Err(RecvTimeoutError::Disconnected) => return Ok(()),
                Err(RecvTimeoutError::Timeout) => {}
            }
            loop {
                let event = match handle.try_event() {
                    Ok(event) => event,
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => return Err(crate::RUNTIME_STOPPED.into()),
                };
                match &event {
                    Event::Session { id, .. } => session = Some(id.clone()),
                    Event::Completed {
                        result: RunResult::Success { .. },
                        ..
                    } if !self.directions.is_empty() => {
                        let directions = std::mem::take(&mut self.directions);
                        handle = (self.launch_turn)(session.clone(), directions)?;
                        break;
                    }
                    Event::Completed { .. } => {
                        let _ = events.send(event);
                        return Ok(());
                    }
                    Event::Stopped(_) => {
                        let _ = events.send(event);
                        return Ok(());
                    }
                    _ => {}
                }
                if events.send(event).is_err() {
                    return Ok(());
                }
            }
        }
    }

    fn queue(&mut self, queued: QueuedAction) -> bool {
        let accepted = match queued.action {
            Action::Steer {
                run,
                prompt,
                images,
            } if run == self.run_id => {
                self.directions.push(Direction { prompt, images });
                true
            }
            Action::Shutdown => return false,
            _ => false,
        };
        if let Some(reply) = queued.reply {
            let _ = reply.try_send(accepted);
        }
        true
    }
}
