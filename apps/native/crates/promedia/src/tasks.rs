//! Deterministic, cooperative background queue. Host owns execution and persistence.
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, VecDeque};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct TaskId(pub u64);
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TaskKind {
    Proxy,
    Optimized,
    Render,
    Import,
    Export,
    Purge,
    /// Generate Captions: speech heard, in seconds of source audio.
    Captions,
}
impl TaskKind {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Proxy => "Creating Proxy Media",
            Self::Optimized => "Creating Optimized Media",
            Self::Render => "Rendering",
            Self::Import => "Importing",
            Self::Export => "Exporting",
            Self::Purge => "Freeing Disk Space",
            Self::Captions => "Generating Captions",
        }
    }
    pub fn symbol(&self) -> &'static str {
        match self {
            Self::Proxy => "film.stack",
            Self::Optimized => "film.stack.fill",
            Self::Render => "square.stack.3d.down.right",
            Self::Import => "square.and.arrow.down",
            Self::Export => "square.and.arrow.up",
            Self::Purge => "trash",
            Self::Captions => "captions.bubble",
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProgressUnit {
    Frames,
    Bytes,
    Items,
    Seconds,
}
/// UI progress must pattern-match this type: unknown totals have no percentage.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TaskProgress {
    Indeterminate {
        completed: u64,
        unit: Option<ProgressUnit>,
    },
    Determinate {
        completed: u64,
        total: u64,
        unit: Option<ProgressUnit>,
    },
}
/// Admission requires authored media/composition title and explicit counter units.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TaskSpec {
    pub title: String,
    pub kind: TaskKind,
    pub unit: ProgressUnit,
    /// Scheduler input only; deliberately absent from TaskPresentation.
    pub priority: i32,
    pub total: u64,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum TaskFailureCode {
    SourceOffline,
    DiskFull,
    CodecUnsupported,
    Cancelled,
    #[default]
    Unknown,
}
impl TaskFailureCode {
    pub fn ui_message(self) -> Option<&'static str> {
        match self {
            // Copy from the design re-review (2026-11-01); sentence case, `danger`.
            Self::SourceOffline => Some("The source media is offline. Reconnect it and try again."),
            Self::DiskFull => Some("Not enough disk space. Free some space and try again."),
            Self::CodecUnsupported => Some("This media’s format isn’t supported."),
            Self::Unknown => Some("The task couldn’t be completed. Try again."),
            // Not an error to the user: the row leaves the list like a completed task.
            Self::Cancelled => None,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TaskFailure {
    pub code: TaskFailureCode,
    /// Worker diagnostics, retained for logs only, never presentation text.
    pub diagnostic: String,
}
impl From<String> for TaskFailure {
    fn from(diagnostic: String) -> Self {
        Self {
            code: TaskFailureCode::Unknown,
            diagnostic,
        }
    }
}
impl From<&str> for TaskFailure {
    fn from(diagnostic: &str) -> Self {
        diagnostic.to_owned().into()
    }
}
/// Safe binding boundary: no priority, raw diagnostic or enum-name UI copy.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TaskPresentation<'a> {
    pub title: &'a str,
    pub kind_label: &'static str,
    pub symbol: &'static str,
    pub progress: TaskProgress,
    pub transition_label: Option<&'static str>,
    pub pause_resume_label: Option<&'static str>,
    pub pause_resume_enabled: bool,
    pub cancel_enabled: bool,
    pub error_code: Option<TaskFailureCode>,
    pub error_message: Option<&'static str>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TaskState {
    Queued,
    Running,
    PauseRequested,
    Paused,
    CancelRequested,
    Cancelled,
    Completed,
    Failed,
}
impl TaskState {
    pub fn terminal(self) -> bool {
        matches!(self, Self::Cancelled | Self::Completed | Self::Failed)
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Task {
    pub id: TaskId,
    pub window: String,
    pub project: String,
    pub kind: TaskKind,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub unit: Option<ProgressUnit>,
    pub priority: i32,
    pub state: TaskState,
    pub completed: u64,
    pub total: u64,
    /// Legacy diagnostic field is preserved in serialization, never UI bindings.
    error: Option<String>,
    #[serde(default)]
    error_code: Option<TaskFailureCode>,
    generation: u64,
}
impl Task {
    pub fn diagnostic_for_log(&self) -> Option<&str> {
        self.error.as_deref()
    }
    /// Legacy records carrying only a raw diagnostic classify as `Unknown`.
    fn failure_code(&self) -> Option<TaskFailureCode> {
        self.error_code
            .or_else(|| self.error.as_ref().map(|_| TaskFailureCode::Unknown))
    }
    pub fn presentation(&self) -> TaskPresentation<'_> {
        let transitional = matches!(
            self.state,
            TaskState::PauseRequested | TaskState::CancelRequested
        );
        TaskPresentation {
            title: &self.title,
            kind_label: self.kind.label(),
            symbol: self.kind.symbol(),
            progress: if self.total == 0 {
                TaskProgress::Indeterminate {
                    completed: self.completed,
                    unit: self.unit,
                }
            } else {
                TaskProgress::Determinate {
                    completed: self.completed,
                    total: self.total,
                    unit: self.unit,
                }
            },
            transition_label: match self.state {
                TaskState::PauseRequested => Some("Pausing…"),
                TaskState::CancelRequested => Some("Cancelling…"),
                _ => None,
            },
            pause_resume_label: match self.state {
                TaskState::Paused => Some("Resume"),
                TaskState::Queued
                | TaskState::Running
                | TaskState::PauseRequested
                | TaskState::CancelRequested => Some("Pause"),
                _ => None,
            },
            pause_resume_enabled: !transitional && !self.state.terminal(),
            cancel_enabled: !transitional && !self.state.terminal(),
            error_code: self.failure_code(),
            error_message: self.failure_code().and_then(TaskFailureCode::ui_message),
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Ticket {
    pub id: TaskId,
    generation: u64,
    epoch: u64,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TaskEvent {
    pub sequence: u64,
    pub task: Task,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WindowEvents {
    /// When true, replace the window model from snapshot; events were compacted.
    pub reset: bool,
    pub cursor: u64,
    pub events: Vec<TaskEvent>,
    pub snapshot: Vec<Task>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TaskError {
    Unknown,
    InvalidTransition,
    InvalidProgress,
    StaleTicket,
    EmptyIdentity,
    Exhausted,
}
pub struct BackgroundTasks {
    epoch: u64,
    tasks: BTreeMap<TaskId, Task>,
    events: VecDeque<TaskEvent>,
    sequence: u64,
    next: u64,
    floor: u64,
}
impl Default for BackgroundTasks {
    fn default() -> Self {
        Self {
            epoch: crate::next_epoch(),
            tasks: BTreeMap::new(),
            events: VecDeque::new(),
            sequence: 0,
            next: 0,
            floor: 0,
        }
    }
}
impl BackgroundTasks {
    fn publish(&mut self, id: TaskId) {
        self.sequence += 1;
        self.events.push_back(TaskEvent {
            sequence: self.sequence,
            task: self.tasks[&id].clone(),
        });
        while self.events.len() > 1024 {
            if let Some(event) = self.events.pop_front() {
                self.floor = event.sequence;
            }
        }
    }
    pub fn enqueue(
        &mut self,
        window: String,
        project: String,
        spec: TaskSpec,
    ) -> Result<TaskId, TaskError> {
        if window.is_empty() || project.is_empty() || spec.title.is_empty() {
            return Err(TaskError::EmptyIdentity);
        }
        self.next = self.next.checked_add(1).ok_or(TaskError::Exhausted)?;
        let id = TaskId(self.next);
        self.tasks.insert(
            id,
            Task {
                id,
                window,
                project,
                kind: spec.kind,
                title: spec.title,
                unit: Some(spec.unit),
                priority: spec.priority,
                state: TaskState::Queued,
                completed: 0,
                total: spec.total,
                error: None,
                error_code: None,
                generation: 0,
            },
        );
        self.publish(id);
        Ok(id)
    }
    /// Grows with every change to any task; a watcher redraws when it moves.
    pub fn sequence(&self) -> u64 {
        self.sequence
    }
    pub fn get(&self, id: TaskId) -> Option<&Task> {
        self.tasks.get(&id)
    }
    /// Every task, in id (queue) order: the Background Tasks window lists
    /// all of them, grouped by project.
    pub fn all(&self) -> Vec<Task> {
        self.tasks.values().cloned().collect()
    }
    pub fn snapshot(&self, window: &str) -> Vec<Task> {
        self.tasks
            .values()
            .filter(|t| t.window == window)
            .cloned()
            .collect()
    }
    pub fn events(&self, window: &str, after: u64) -> WindowEvents {
        let reset = after > self.sequence || after < self.floor;
        WindowEvents {
            reset,
            cursor: self.sequence,
            events: if reset {
                Vec::new()
            } else {
                self.events
                    .iter()
                    .filter(|e| e.sequence > after && e.task.window == window)
                    .cloned()
                    .collect()
            },
            snapshot: if reset {
                self.snapshot(window)
            } else {
                Vec::new()
            },
        }
    }
    /// Higher priority first, then FIFO task id. No preemption of active codec calls.
    pub fn start_next(&mut self) -> Option<Ticket> {
        let id = self
            .tasks
            .values()
            .filter(|t| t.state == TaskState::Queued)
            .min_by_key(|t| (std::cmp::Reverse(t.priority), t.id))?
            .id;
        let task = self.tasks.get_mut(&id)?;
        task.state = TaskState::Running;
        task.generation += 1;
        let ticket = Ticket {
            id,
            generation: task.generation,
            epoch: self.epoch,
        };
        self.publish(id);
        Some(ticket)
    }
    pub fn priority(&mut self, id: TaskId, priority: i32) -> Result<(), TaskError> {
        let task = self.tasks.get_mut(&id).ok_or(TaskError::Unknown)?;
        if task.state.terminal() {
            return Err(TaskError::InvalidTransition);
        }
        task.priority = priority;
        self.publish(id);
        Ok(())
    }
    pub fn pause(&mut self, id: TaskId) -> Result<(), TaskError> {
        let t = self.tasks.get_mut(&id).ok_or(TaskError::Unknown)?;
        t.state = match t.state {
            TaskState::Queued => TaskState::Paused,
            TaskState::Running => TaskState::PauseRequested,
            _ => return Err(TaskError::InvalidTransition),
        };
        self.publish(id);
        Ok(())
    }
    pub fn resume(&mut self, id: TaskId) -> Result<(), TaskError> {
        let t = self.tasks.get_mut(&id).ok_or(TaskError::Unknown)?;
        if t.state != TaskState::Paused {
            return Err(TaskError::InvalidTransition);
        }
        t.state = TaskState::Queued;
        self.publish(id);
        Ok(())
    }
    pub fn cancel(&mut self, id: TaskId) -> Result<(), TaskError> {
        let t = self.tasks.get_mut(&id).ok_or(TaskError::Unknown)?;
        t.state = match t.state {
            TaskState::Queued | TaskState::Paused => TaskState::Cancelled,
            TaskState::Running | TaskState::PauseRequested => TaskState::CancelRequested,
            _ => return Err(TaskError::InvalidTransition),
        };
        self.publish(id);
        Ok(())
    }
    fn worker(&mut self, ticket: Ticket) -> Result<&mut Task, TaskError> {
        if ticket.epoch != self.epoch {
            return Err(TaskError::StaleTicket);
        }
        let t = self.tasks.get_mut(&ticket.id).ok_or(TaskError::Unknown)?;
        if t.generation != ticket.generation
            || !matches!(
                t.state,
                TaskState::Running | TaskState::PauseRequested | TaskState::CancelRequested
            )
        {
            return Err(TaskError::StaleTicket);
        }
        Ok(t)
    }
    /// Inspect a live ticket on the kernel actor before dispatch/progress/control.
    pub fn worker_state(&mut self, ticket: Ticket) -> Result<TaskState, TaskError> {
        self.worker(ticket).map(|task| task.state)
    }
    /// Requested control remains visible until the worker has released its resources.
    pub fn settle_control(&mut self, ticket: Ticket) -> Result<(), TaskError> {
        let t = self.worker(ticket)?;
        t.state = match t.state {
            TaskState::PauseRequested => TaskState::Paused,
            TaskState::CancelRequested => TaskState::Cancelled,
            _ => return Err(TaskError::InvalidTransition),
        };
        self.publish(ticket.id);
        Ok(())
    }
    pub fn progress(
        &mut self,
        ticket: Ticket,
        completed: u64,
        total: u64,
    ) -> Result<(), TaskError> {
        let t = self.worker(ticket)?;
        if completed < t.completed
            || (total != 0 && completed > total)
            || (t.total != 0 && t.total != total)
        {
            return Err(TaskError::InvalidProgress);
        }
        t.completed = completed;
        t.total = total;
        self.publish(ticket.id);
        Ok(())
    }
    /// A completion racing pause/cancel cannot publish an output as successful.
    pub fn finish(
        &mut self,
        ticket: Ticket,
        result: Result<(), TaskFailure>,
    ) -> Result<(), TaskError> {
        let t = self.worker(ticket)?;
        if t.state != TaskState::Running {
            return Err(TaskError::InvalidTransition);
        }
        match result {
            Ok(()) => {
                if t.total != 0 {
                    t.completed = t.total;
                }
                t.state = TaskState::Completed;
            }
            Err(e) => {
                t.error_code = Some(e.code);
                t.error = Some(e.diagnostic);
                t.state = TaskState::Failed;
            }
        }
        self.publish(ticket.id);
        Ok(())
    }
    /// Explicit retirement bounds the model; caller archives terminal facts first.
    pub fn retire(&mut self, id: TaskId) -> Result<Task, TaskError> {
        if !self
            .tasks
            .get(&id)
            .ok_or(TaskError::Unknown)?
            .state
            .terminal()
        {
            return Err(TaskError::InvalidTransition);
        }
        // Removal is represented by reset on the next poll, avoiding ghost rows.
        self.events.clear();
        self.sequence += 1;
        self.floor = self.sequence;
        Ok(self.tasks.remove(&id).expect("validated task"))
    }
}
