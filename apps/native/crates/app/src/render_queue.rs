//! Profile-owned render queue. Frozen inputs stay private in the profile; UI
//! projections contain only labels, destinations and lifecycle facts.
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, OnceLock},
};
fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
pub type Store = Arc<Mutex<Queue>>;
pub fn store(profile: &Path) -> Store {
    static STORES: OnceLock<Mutex<HashMap<PathBuf, Store>>> = OnceLock::new();
    STORES
        .get_or_init(Default::default)
        .lock()
        .unwrap()
        .entry(profile.to_owned())
        .or_insert_with(|| Arc::new(Mutex::new(Queue::load(profile.join("render-queue.json")))))
        .clone()
}
#[derive(Clone, Serialize, Deserialize)]
struct Row {
    id: u64,
    status: String,
    error: String,
    progress: f64,
    input: Value,
    #[serde(default)]
    cancel: bool,
    #[serde(default)]
    finished_at: u64,
}
pub struct Queue {
    path: PathBuf,
    rows: Vec<Row>,
    load_error: Option<String>,
}
impl Queue {
    fn load(path: PathBuf) -> Self {
        let (mut rows, load_error): (Vec<Row>, _) = match std::fs::read(&path) {
            Ok(bytes) => match serde_json::from_slice(&bytes) {
                Ok(rows) => (rows, None),
                Err(e) => (vec![], Some(format!("Could not read render queue: {e}"))),
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => (vec![], None),
            Err(e) => (vec![], Some(format!("Could not read render queue: {e}"))),
        };
        for row in &mut rows {
            if row.status == "Rendering" {
                row.status = "Pending".into();
                row.progress = 0.;
                row.cancel = false;
            }
        }
        Self {
            path,
            rows,
            load_error,
        }
    }
    fn save(&self) -> Result<(), String> {
        if let Some(error) = &self.load_error {
            return Err(error.clone());
        }
        let save = || -> Result<(), Box<dyn std::error::Error>> {
            std::fs::create_dir_all(self.path.parent().unwrap())?;
            let tmp = self.path.with_extension("json.tmp");
            let file = std::fs::File::create(&tmp)?;
            serde_json::to_writer(&file, &self.rows)?;
            file.sync_all()?;
            std::fs::rename(tmp, &self.path)?;
            Ok(())
        };
        save().map_err(|e| format!("Could not save render queue: {e}"))
    }
    fn change<T>(
        &mut self,
        f: impl FnOnce(&mut Vec<Row>) -> Result<T, String>,
    ) -> Result<T, String> {
        let before = self.rows.clone();
        let result = f(&mut self.rows).and_then(|value| self.save().map(|_| value));
        if result.is_err() {
            self.rows = before;
        }
        result
    }
    pub fn add(&mut self, input: Value) -> Result<u64, String> {
        self.change(|rows| {
            let id = rows.iter().map(|r| r.id).max().unwrap_or(0) + 1;
            rows.push(Row {
                id,
                status: "Pending".into(),
                error: String::new(),
                progress: 0.,
                input,
                cancel: false,
                finished_at: 0,
            });
            Ok(id)
        })
    }
    pub fn take_pending(&mut self) -> Result<Option<(u64, Value)>, String> {
        if self.rows.iter().any(|r| r.status == "Rendering") {
            return Ok(None);
        }
        self.change(|rows| {
            Ok(rows.iter_mut().find(|r| r.status == "Pending").map(|row| {
                row.status = "Rendering".into();
                (row.id, row.input.clone())
            }))
        })
    }
    pub fn finish(&mut self, id: u64, status: &str, error: String) -> Result<(), String> {
        self.change(|rows| {
            let row = rows
                .iter_mut()
                .find(|r| r.id == id)
                .ok_or("Queue job missing")?;
            row.status = status.into();
            row.error = error;
            if status != "Rendering" {
                row.finished_at = now();
            }
            if status == "Done" {
                row.progress = 1.;
            }
            Ok(())
        })
    }
    pub fn action(&mut self, id: u64, action: &str) -> Result<(), String> {
        self.change(|rows| {
            if action == "clear" {
                rows.retain(|r| r.status == "Pending" || r.status == "Rendering");
                return Ok(());
            }
            let row = rows
                .iter_mut()
                .find(|r| r.id == id)
                .ok_or("Queue job missing")?;
            match action {
                "retry" if row.status == "Failed" || row.status == "Cancelled" => {
                    row.status = "Pending".into();
                    row.error.clear();
                    row.progress = 0.;
                    row.cancel = false;
                    row.finished_at = 0;
                }
                "cancel" if row.status == "Rendering" => row.cancel = true,
                "cancel" if row.status == "Pending" => {
                    row.status = "Cancelled".into();
                    row.finished_at = now();
                }
                "remove" if row.status != "Rendering" => rows.retain(|r| r.id != id),
                _ => return Err("This queue action is unavailable".into()),
            }
            Ok(())
        })
    }
    pub fn cancelled(&self, id: u64) -> bool {
        self.rows.iter().any(|r| r.id == id && r.cancel)
    }
    pub fn progress(&mut self, id: u64, progress: f64) {
        if let Some(row) = self.rows.iter_mut().find(|r| r.id == id) {
            row.progress = progress.clamp(0., 1.);
        }
    }
    pub fn projection(&self) -> Value {
        json!(self.rows.iter().map(|r|json!({"id":r.id,"name":r.input["name"],"format":r.input["options"]["format"],
            "profile":r.input["options"]["profile"],"finishedAt":r.finished_at,"path":r.input["destination"],"status":r.status,"progress":r.progress,"error":r.error})).collect::<Vec<_>>())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn round_trip_order_retry_cancel_and_history() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("queue.json");
        let mut q = Queue::load(path.clone());
        let a = q.add(json!({"name":"First", "document":"frozen"})).unwrap();
        let b = q.add(json!({"name":"Second"})).unwrap();
        let c = q.add(json!({"name":"Third"})).unwrap();
        assert_eq!(q.take_pending().unwrap().unwrap().0, a);
        assert!(q.take_pending().unwrap().is_none());
        q.finish(a, "Failed", "Missing media".into()).unwrap();
        q.action(a, "retry").unwrap();
        assert_eq!(q.take_pending().unwrap().unwrap().0, a);
        q.finish(a, "Done", String::new()).unwrap();
        q.action(b, "cancel").unwrap();
        assert_eq!(q.take_pending().unwrap().unwrap().0, c);
        q.action(c, "cancel").unwrap();
        assert!(q.cancelled(c));
        q.finish(c, "Cancelled", String::new()).unwrap();
        let loaded = Queue::load(path);
        assert_eq!(loaded.projection(), q.projection());
        assert_eq!(loaded.rows[0].input["document"], "frozen");
        assert_eq!(loaded.rows[0].status, "Done");
        q.action(b, "remove").unwrap();
        assert_eq!(q.rows.len(), 2);
    }
    #[test]
    fn relaunch_restores_interrupted_job_as_pending() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("queue.json");
        let mut q = Queue::load(path.clone());
        let id = q.add(json!({"name":"Interrupted"})).unwrap();
        q.take_pending().unwrap();
        q.progress(id, 0.75);
        drop(q);
        let mut q = Queue::load(path);
        assert_eq!(q.rows[0].status, "Pending");
        assert_eq!(q.rows[0].progress, 0.);
        assert_eq!(q.take_pending().unwrap().unwrap().0, id);
    }
    #[test]
    fn cancelling_a_running_job_waits_for_settlement_before_the_next_job() {
        let dir = tempfile::tempdir().unwrap();
        let mut q = Queue::load(dir.path().join("queue.json"));
        let first = q.add(json!({"name":"First"})).unwrap();
        let second = q.add(json!({"name":"Second"})).unwrap();
        q.take_pending().unwrap();
        assert!(q.action(first, "remove").is_err());
        assert!(q.action(first, "retry").is_err());
        q.action(first, "cancel").unwrap();
        assert!(q.cancelled(first));
        assert!(q.take_pending().unwrap().is_none());
        q.finish(first, "Cancelled", String::new()).unwrap();
        assert_eq!(q.take_pending().unwrap().unwrap().0, second);
    }
    #[test]
    fn clear_history_keeps_pending_and_running_jobs_and_completion_time() {
        let dir = tempfile::tempdir().unwrap();
        let mut q = Queue::load(dir.path().join("queue.json"));
        let done = q.add(json!({"name":"Done"})).unwrap();
        q.take_pending().unwrap();
        q.finish(done, "Done", String::new()).unwrap();
        assert!(q.rows[0].finished_at > 0);
        let running = q.add(json!({"name":"Running"})).unwrap();
        q.take_pending().unwrap();
        let pending = q.add(json!({"name":"Pending"})).unwrap();
        q.action(0, "clear").unwrap();
        assert_eq!(
            q.rows.iter().map(|r| r.id).collect::<Vec<_>>(),
            vec![running, pending]
        );
    }
    #[test]
    fn broken_store_is_preserved_and_write_failure_rolls_back() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("queue.json");
        std::fs::write(&path, b"broken").unwrap();
        let mut q = Queue::load(path.clone());
        assert!(q.add(json!({})).is_err());
        assert!(q.rows.is_empty());
        assert_eq!(std::fs::read(path).unwrap(), b"broken");
    }
}
