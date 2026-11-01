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
    pub priority: i32,
    pub state: TaskState,
    pub completed: u64,
    pub total: u64,
    pub error: Option<String>,
    generation: u64,
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
        kind: TaskKind,
        priority: i32,
        total: u64,
    ) -> Result<TaskId, TaskError> {
        if window.is_empty() || project.is_empty() {
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
                kind,
                priority,
                state: TaskState::Queued,
                completed: 0,
                total,
                error: None,
                generation: 0,
            },
        );
        self.publish(id);
        Ok(id)
    }
    pub fn get(&self, id: TaskId) -> Option<&Task> {
        self.tasks.get(&id)
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
        if completed < t.completed || completed > total || (t.total != 0 && t.total != total) {
            return Err(TaskError::InvalidProgress);
        }
        t.completed = completed;
        t.total = total;
        self.publish(ticket.id);
        Ok(())
    }
    /// A completion racing pause/cancel cannot publish an output as successful.
    pub fn finish(&mut self, ticket: Ticket, result: Result<(), String>) -> Result<(), TaskError> {
        let t = self.worker(ticket)?;
        if t.state != TaskState::Running {
            return Err(TaskError::InvalidTransition);
        }
        match result {
            Ok(()) => {
                t.completed = t.total;
                t.state = TaskState::Completed;
            }
            Err(e) => {
                t.error = Some(e);
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
