//! S045/S046/N005. Adapters over the existing queue/storage; no parallel scheduler.
use super::super::{RecordHost, Records, invalid, replace, unavailable};
use crate::{Result, ServiceError, ServiceErrorCode, storage::ServiceStorage};
use pm_promedia::{
    storage::{Storage, StorageClass, Usage},
    tasks::{BackgroundTasks, Task, TaskId, TaskState},
};
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
};

fn task_error(error: pm_promedia::tasks::TaskError) -> ServiceError {
    ServiceError::message(
        ServiceErrorCode::InvalidInput,
        format!("task boundary: {error:?}"),
    )
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TaskAction {
    Pause,
    Resume,
    Cancel,
}
pub fn act(
    queue: &mut BackgroundTasks,
    window_id: &str,
    task_id: TaskId,
    action: TaskAction,
) -> Result<()> {
    let task = queue.get(task_id).ok_or_else(|| invalid("task missing"))?;
    if task.window != window_id {
        return Err(invalid("task belongs to another window"));
    }
    match action {
        TaskAction::Pause => queue.pause(task_id),
        TaskAction::Resume => queue.resume(task_id),
        TaskAction::Cancel => queue.cancel(task_id),
    }
    .map_err(task_error)
}
#[derive(Default)]
pub struct TaskRows {
    pub rows: BTreeMap<TaskId, Task>,
    pub cursor: u64,
}
impl TaskRows {
    pub fn refresh(&mut self, queue: &BackgroundTasks, window_id: &str) {
        // Always include a snapshot on first subscription (cursor 0 alone may
        // otherwise miss rows after a queue was restored).
        let events = queue.events(window_id, self.cursor);
        if self.cursor == 0 || events.reset {
            self.rows = queue
                .snapshot(window_id)
                .into_iter()
                .map(|t| (t.id, t))
                .collect();
        } else {
            for event in events.events {
                self.rows.insert(event.task.id, event.task);
            }
        }
        self.cursor = events.cursor;
    }
    /// Parallel tasks retain their own units. An unknown total keeps the overall
    /// summary indeterminate, as in the tagged pill; never average frames and bytes.
    pub fn overall_fraction(&self) -> Option<f64> {
        let running: Vec<_> = self.rows.values().filter(|t| !t.state.terminal()).collect();
        if running.is_empty() || running.iter().any(|t| t.total == 0) {
            return None;
        }
        Some(
            running
                .iter()
                .map(|t| t.completed as f64 / t.total as f64)
                .sum::<f64>()
                / running.len() as f64,
        )
    }
}
pub trait TaskArchive {
    fn retain(&mut self, task: &Task) -> Result<()>;
}
impl TaskArchive for ServiceStorage {
    fn retain(&mut self, task: &Task) -> Result<()> {
        self.set(
            &format!("services.task-archive.{}", task.id.0),
            serde_json::to_value(task)?,
        )?;
        Ok(())
    }
}
pub fn retire(
    queue: &mut BackgroundTasks,
    archive: &mut impl TaskArchive,
    window_id: &str,
    task_id: TaskId,
) -> Result<()> {
    let task = queue.get(task_id).ok_or_else(|| invalid("task missing"))?;
    if task.window != window_id || !task.state.terminal() {
        return Err(invalid("task cannot retire"));
    }
    archive.retain(task)?; // Only after durable receipt may the row be retired.
    queue.retire(task_id).map_err(task_error)?;
    Ok(())
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QueueAction {
    Retry,
    Archive,
}
pub fn legacy_queue_action(
    host: &mut impl RecordHost,
    records: &Records,
    job_id: &str,
    action: QueueAction,
) -> Result<u64> {
    records.validate()?;
    let job = records.find(job_id)?;
    if job["status"] == "rendering" {
        return Err(unavailable("render job still running"));
    }
    let mut after = records.entries.clone();
    match action {
        QueueAction::Archive => after.retain(|v| v["id"] != job_id),
        QueueAction::Retry => {
            if job["status"] != "failed" && job["status"] != "cancelled" {
                return Err(invalid("only failed or cancelled jobs retry"));
            }
            let job = after
                .iter_mut()
                .find(|v| v["id"] == job_id)
                .ok_or_else(|| invalid("job missing"))?;
            job["status"] = "pending".into();
            job["error"] = "".into();
        }
    }
    replace(host, "renderQueue", records, &after)
}
/// Start jobs against their frozen project/options. Never swap the live editor's
/// project as the tagged queue did. Host opens an isolated engine-service session.
pub trait RenderQueueHost {
    fn render_snapshot(&mut self, job_id: &str, project: &Value, options: &Value) -> Result<()>;
}
pub fn run_legacy_job(
    host: &mut impl RenderQueueHost,
    records: &Records,
    job_id: &str,
) -> Result<()> {
    records.validate()?;
    let job = records.find(job_id)?;
    if job["status"] != "pending" || !job["project"].is_object() || !job["options"].is_object() {
        return Err(invalid("invalid queued render snapshot"));
    }
    host.render_snapshot(job_id, &job["project"], &job["options"])
}
pub trait StorageClosure {
    /// Complete closure includes originals/renditions held by project, history,
    /// trash, takes, agents, rollback and in-flight jobs; errors refuse archival.
    fn referenced(&mut self, class: StorageClass, name: &str) -> Result<bool>;
}
pub fn archive_storage(
    storage: &Storage,
    closure: &mut impl StorageClosure,
    class: StorageClass,
    name: &str,
) -> Result<PathBuf> {
    let referenced = closure.referenced(class, name)?;
    storage
        .purge(class, name, referenced)
        .map_err(ServiceError::from)
}
pub fn storage_usage(storage: &Storage) -> Result<BTreeMap<StorageClass, Usage>> {
    [
        StorageClass::Originals,
        StorageClass::Proxies,
        StorageClass::Optimized,
        StorageClass::Renders,
    ]
    .into_iter()
    .map(|class| {
        storage
            .usage(class)
            .map(|usage| (class, usage))
            .map_err(ServiceError::from)
    })
    .collect()
}
pub fn active_ids(rows: &TaskRows) -> BTreeSet<TaskId> {
    rows.rows
        .values()
        .filter(|t| {
            !matches!(
                t.state,
                TaskState::Completed | TaskState::Failed | TaskState::Cancelled
            )
        })
        .map(|t| t.id)
        .collect()
}
