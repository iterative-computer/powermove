//! Fenced background-task execution for a frozen sequence of engine frames.
use crate::{
    cache::RenderCache,
    cache_frame::{FrameSource, GraphIdentity},
    proxy::Control,
    render::RenderWork,
    storage::invalid,
    tasks::{
        BackgroundTasks, ProgressUnit, TaskFailure, TaskFailureCode, TaskKind, TaskState, Ticket,
    },
};
use std::io;
#[derive(Clone, Debug)]
pub struct RenderFillWork {
    ticket: Ticket,
    project: String,
    identity: GraphIdentity,
    frames: Vec<RenderWork>,
    signature: [u8; 32],
    floor: u64,
}
#[derive(Debug)]
pub enum FillOutcome {
    Ready,
    Stopped(Control),
    Failed(TaskFailure),
}
#[derive(Debug)]
pub struct FillReport {
    ticket: Ticket,
    signature: [u8; 32],
    pub completed: u64,
    pub outcome: FillOutcome,
}
impl RenderFillWork {
    /// Kernel actor. Every frame uses one complete graph/media identity. Titles,
    /// scheduling priority and window ownership come from the existing task.
    pub fn prepare(
        tasks: &mut BackgroundTasks,
        ticket: Ticket,
        project: String,
        identity: GraphIdentity,
        frames: Vec<RenderWork>,
    ) -> io::Result<Self> {
        let state = tasks
            .worker_state(ticket)
            .map_err(|e| invalid(&format!("render ticket: {e:?}")))?;
        let task = tasks
            .get(ticket.id)
            .ok_or_else(|| invalid("unknown render task"))?;
        if state != TaskState::Running
            || task.kind != TaskKind::Render
            || task.project != project
            || task.unit != Some(ProgressUnit::Frames)
            || frames.is_empty()
            || (task.total != 0 && task.total != frames.len() as u64)
            || task.completed > frames.len() as u64
        {
            return Err(invalid("render task admission mismatch"));
        }
        let mut hash = blake3::Hasher::new_derive_key("powermove render fill v1");
        for work in &frames {
            if work.identity() != &identity {
                return Err(invalid("mixed render graph identities"));
            }
            hash.update(work.key()?.digest()?.as_bytes());
        }
        Ok(Self {
            ticket,
            project,
            identity,
            frames,
            signature: *hash.finalize().as_bytes(),
            floor: task.completed,
        })
    }
    /// Worker. Publish progress/control through the host actor channel. Completed
    /// cache entries survive stop/restart; a restart can reuse exact cache hits.
    pub fn run(
        &self,
        source: &mut dyn FrameSource,
        cache: &RenderCache,
        mut observe: impl FnMut(u64) -> Control,
    ) -> FillReport {
        let mut completed = self.floor;
        let outcome = (|| -> io::Result<FillOutcome> {
            for (i, frame) in self.frames.iter().enumerate() {
                let control = observe(completed);
                if control != Control::Continue {
                    return Ok(FillOutcome::Stopped(control));
                }
                frame.run(source, cache, None)?;
                completed = completed.max(i as u64 + 1);
            }
            let control = observe(completed);
            Ok(if control == Control::Continue {
                FillOutcome::Ready
            } else {
                FillOutcome::Stopped(control)
            })
        })()
        .unwrap_or_else(|e| {
            FillOutcome::Failed(TaskFailure {
                code: if e.kind() == io::ErrorKind::StorageFull {
                    TaskFailureCode::DiskFull
                } else {
                    TaskFailureCode::Unknown
                },
                diagnostic: e.to_string(),
            })
        });
        FillReport {
            ticket: self.ticket,
            signature: self.signature,
            completed,
            outcome,
        }
    }
    /// Kernel actor receipt. Call on current work/current engine identity. A
    /// pending control wins after worker resources return, even if graph changed.
    pub fn accept(
        &self,
        report: &FillReport,
        tasks: &mut BackgroundTasks,
        current: &GraphIdentity,
    ) -> io::Result<()> {
        if report.ticket != self.ticket || report.signature != self.signature {
            return Err(invalid("foreign render result"));
        }
        let state = tasks
            .worker_state(self.ticket)
            .map_err(|e| invalid(&format!("render ticket: {e:?}")))?;
        let task = tasks
            .get(self.ticket.id)
            .ok_or_else(|| invalid("unknown render task"))?;
        if task.kind != TaskKind::Render || task.project != self.project {
            return Err(invalid("foreign render task"));
        }
        if current != &self.identity
            && !matches!(
                state,
                TaskState::PauseRequested | TaskState::CancelRequested
            )
        {
            return Err(invalid("stale render publication"));
        }
        if !matches!(
            state,
            TaskState::PauseRequested | TaskState::CancelRequested
        ) && matches!(report.outcome, FillOutcome::Stopped(_))
        {
            return Err(invalid("render stopped without actor control request"));
        }
        tasks
            .progress(self.ticket, report.completed, self.frames.len() as u64)
            .map_err(|e| invalid(&format!("render progress: {e:?}")))?;
        if matches!(
            state,
            TaskState::PauseRequested | TaskState::CancelRequested
        ) {
            return tasks
                .settle_control(self.ticket)
                .map_err(|e| invalid(&format!("render control: {e:?}")));
        }
        let result = match &report.outcome {
            FillOutcome::Ready => Ok(()),
            FillOutcome::Failed(e) => Err(e.clone()),
            FillOutcome::Stopped(_) => {
                return Err(invalid("render stopped without actor control request"));
            }
        };
        tasks
            .finish(self.ticket, result)
            .map_err(|e| invalid(&format!("render finish: {e:?}")))
    }
}
