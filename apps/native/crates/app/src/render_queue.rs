//! The render queue's worker owns the profile index and frozen export inputs.
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::{HashMap, HashSet},
    fs::{self, File},
    io::{self, BufReader, BufWriter, Write},
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock, mpsc},
};

const MAX_INDEX_BYTES: usize = 64 * 1024;
const MAX_HISTORY: usize = 20;
const MAX_NAME_BYTES: usize = 160;

type Operation = Box<dyn FnOnce(&mut Queue) + Send>;
pub type Reply<T> = async_channel::Receiver<Result<T, String>>;

#[derive(Clone)]
pub struct Store(mpsc::Sender<Operation>);

impl Store {
    fn open(path: PathBuf) -> Self {
        let (send, receive) = mpsc::channel::<Operation>();
        if let Err(error) = std::thread::Builder::new()
            .name("render-queue".into())
            .spawn(move || {
                let mut queue = Queue::load(path);
                for operation in receive {
                    operation(&mut queue);
                }
            })
        {
            eprintln!("Could not start render queue worker: {error}");
        }
        Self(send)
    }

    pub fn submit<T: Send + 'static>(
        &self,
        operation: impl FnOnce(&mut Queue) -> Result<T, String> + Send + 'static,
    ) -> Reply<T> {
        let (send, receive) = async_channel::bounded(1);
        let _ = self.0.send(Box::new(move |queue| {
            let _ = send.try_send(operation(queue));
        }));
        receive
    }

    pub async fn run<T: Send + 'static>(
        &self,
        operation: impl FnOnce(&mut Queue) -> Result<T, String> + Send + 'static,
    ) -> Result<T, String> {
        self.submit(operation)
            .recv()
            .await
            .map_err(|_| "Render queue worker stopped".to_string())?
    }
}

pub fn store(profile: &Path) -> Store {
    static STORES: OnceLock<Mutex<HashMap<PathBuf, Store>>> = OnceLock::new();
    STORES
        .get_or_init(Default::default)
        .lock()
        .unwrap()
        .entry(profile.to_owned())
        .or_insert_with(|| Store::open(profile.join("render-queue.json")))
        .clone()
}

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
enum Status {
    Pending,
    Rendering,
    Interrupted,
    Done,
    Failed,
    Cancelled,
}

impl Status {
    fn has_snapshot(self) -> bool {
        matches!(self, Self::Pending | Self::Rendering | Self::Interrupted)
    }
}

#[derive(Clone, Default)]
struct Details {
    format: String,
    profile: Option<String>,
    destination: String,
    progress: f64,
    error: String,
    cancel: bool,
}

impl Details {
    fn from_input(input: &Value) -> Self {
        Self {
            format: input["options"]["format"]
                .as_str()
                .unwrap_or_default()
                .into(),
            profile: input["options"]["profile"].as_str().map(str::to_owned),
            destination: input["destination"].as_str().unwrap_or_default().into(),
            ..Default::default()
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
struct Row {
    id: u64,
    name: String,
    status: Status,
    #[serde(default, skip_serializing_if = "is_zero")]
    finished_at: u64,
    #[serde(skip)]
    details: Details,
}

fn is_zero(value: &u64) -> bool {
    *value == 0
}

impl Row {
    fn new(id: u64, status: Status, input: &Value) -> Self {
        let name = input["name"].as_str().unwrap_or("Untitled");
        Self {
            id,
            name: name[..name.floor_char_boundary(MAX_NAME_BYTES)].to_owned(),
            status,
            finished_at: 0,
            details: Details::from_input(input),
        }
    }

    fn settle(&mut self, status: Status, error: String) {
        self.status = status;
        self.finished_at = now();
        self.details.error = error;
        self.details.cancel = false;
    }
}

#[derive(Clone, Default, Serialize, Deserialize)]
struct Index {
    last_id: u64,
    rows: Vec<Row>,
}

impl Index {
    fn prune_history(&mut self) {
        let mut history: Vec<_> = self
            .rows
            .iter()
            .filter(|row| !row.status.has_snapshot())
            .map(|row| (row.finished_at, row.id))
            .collect();
        if history.len() <= MAX_HISTORY {
            return;
        }
        history.sort_unstable();
        let cutoff = history[history.len() - MAX_HISTORY];
        self.rows
            .retain(|row| row.status.has_snapshot() || (row.finished_at, row.id) >= cutoff);
    }
}

#[derive(Deserialize)]
struct LegacyRow {
    id: u64,
    status: Status,
    input: Value,
    #[serde(default)]
    finished_at: u64,
}

pub struct Queue {
    path: PathBuf,
    index: Index,
    load_error: Option<String>,
}

impl Queue {
    fn load(path: PathBuf) -> Self {
        let mut queue = Self {
            path,
            index: Index::default(),
            load_error: None,
        };
        if let Err(error) = queue.read_index() {
            queue.load_error = Some(format!("Could not read render queue: {error}"));
        } else {
            queue.remove_orphaned_snapshots();
        }
        queue
    }

    fn read_index(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        let bytes = match fs::read(&self.path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(error.into()),
        };
        if bytes.iter().find(|byte| !byte.is_ascii_whitespace()) == Some(&b'[') {
            let legacy: Vec<LegacyRow> = serde_json::from_slice(&bytes)?;
            for old in legacy {
                let mut row = Row::new(old.id, old.status, &old.input);
                row.finished_at = old.finished_at;
                if row.status.has_snapshot() {
                    row.status = Status::Interrupted;
                    self.write_snapshot(row.id, &old.input)?;
                }
                self.index.last_id = self.index.last_id.max(row.id);
                self.index.rows.push(row);
            }
            self.index.prune_history();
            self.save()?;
        } else {
            self.index = serde_json::from_slice(&bytes)?;
            for row in &mut self.index.rows {
                if row.status.has_snapshot() {
                    row.status = Status::Interrupted;
                }
            }
        }
        Ok(())
    }

    fn snapshot_dir(&self) -> PathBuf {
        self.path.with_file_name("render-jobs")
    }

    fn snapshot_path(&self, id: u64) -> PathBuf {
        self.snapshot_dir().join(format!("{id}.pmv-snapshot"))
    }

    fn write_snapshot(&self, id: u64, input: &Value) -> Result<(), String> {
        write_atomic(&self.snapshot_path(id), |writer| {
            serde_json::to_writer(writer, input)?;
            Ok(())
        })
        .map_err(|error| format!("Could not save export snapshot: {error}"))
    }

    fn remove_snapshot(&self, id: u64) {
        if let Err(error) = fs::remove_file(self.snapshot_path(id))
            && error.kind() != io::ErrorKind::NotFound
        {
            eprintln!("Could not remove render queue snapshot: {error}");
        }
    }

    fn remove_orphaned_snapshots(&self) {
        let Ok(entries) = fs::read_dir(self.snapshot_dir()) else {
            return;
        };
        let live: HashSet<_> = self
            .index
            .rows
            .iter()
            .filter(|row| row.status.has_snapshot())
            .map(|row| row.id)
            .collect();
        for entry in entries.flatten() {
            if let Some(id) = entry
                .file_name()
                .to_str()
                .and_then(|name| name.strip_suffix(".pmv-snapshot"))
                .and_then(|id| id.parse::<u64>().ok())
                && !live.contains(&id)
            {
                self.remove_snapshot(id);
            }
        }
    }

    fn save(&self) -> Result<(), String> {
        if let Some(error) = &self.load_error {
            return Err(error.clone());
        }
        let bytes = serde_json::to_vec(&self.index).map_err(|error| error.to_string())?;
        if bytes.len() > MAX_INDEX_BYTES {
            return Err("The render queue is full. Remove an interrupted export or render the waiting jobs first.".into());
        }
        write_atomic(&self.path, |writer| {
            writer.write_all(&bytes)?;
            Ok(())
        })
        .map_err(|error| format!("Could not save render queue: {error}"))
    }

    fn change<T>(&mut self, f: impl FnOnce(&mut Index) -> Result<T, String>) -> Result<T, String> {
        let before = self.index.clone();
        let result = f(&mut self.index).and_then(|value| {
            self.index.prune_history();
            self.save()?;
            Ok(value)
        });
        if result.is_err() {
            self.index = before;
        }
        result
    }

    pub fn add(&mut self, input: Value) -> Result<u64, String> {
        self.insert(input, Status::Pending)
    }

    pub fn add_rendering(&mut self, input: Value) -> Result<u64, String> {
        self.insert(input, Status::Rendering)
    }

    fn insert(&mut self, input: Value, status: Status) -> Result<u64, String> {
        if let Some(error) = &self.load_error {
            return Err(error.clone());
        }
        let id = self
            .index
            .last_id
            .checked_add(1)
            .ok_or("Render queue ran out of IDs")?;
        let row = Row::new(id, status, &input);
        self.write_snapshot(id, &input)?;
        let result = self.change(|index| {
            index.last_id = id;
            index.rows.push(row);
            Ok(id)
        });
        if result.is_err() {
            self.remove_snapshot(id);
        }
        result
    }

    pub fn take_pending(&mut self) -> Result<Option<(u64, Value)>, String> {
        if self
            .index
            .rows
            .iter()
            .any(|row| row.status == Status::Rendering)
        {
            return Ok(None);
        }
        let Some(id) = self
            .index
            .rows
            .iter()
            .find(|row| row.status == Status::Pending)
            .map(|row| row.id)
        else {
            return Ok(None);
        };
        let input = File::open(self.snapshot_path(id))
            .map_err(|error| error.to_string())
            .and_then(|file| {
                serde_json::from_reader(BufReader::new(file)).map_err(|error| error.to_string())
            });
        let input = match input {
            Ok(input) => input,
            Err(error) => {
                let error = format!("Could not read export snapshot: {error}");
                self.finish(id, "Failed", error.clone())?;
                return Err(error);
            }
        };
        self.change(|index| {
            let row = find_row(&mut index.rows, id)?;
            row.details = Details::from_input(&input);
            row.status = Status::Rendering;
            Ok(Some((id, input)))
        })
    }

    pub fn finish(&mut self, id: u64, status: &str, error: String) -> Result<(), String> {
        let status = match status {
            "Done" => Status::Done,
            "Failed" => Status::Failed,
            "Cancelled" => Status::Cancelled,
            _ => return Err("Invalid export outcome".into()),
        };
        let row = find_row(&mut self.index.rows, id)?;
        row.settle(status, error);
        self.index.prune_history();
        // The worker has already finished. Release the in-memory reservation
        // even if saving fails. Keep its snapshot until a new index is saved.
        self.save()?;
        self.remove_orphaned_snapshots();
        Ok(())
    }

    fn move_pending(rows: &mut [Row], id: u64, earlier: bool) -> Result<(), String> {
        let from = rows
            .iter()
            .position(|row| row.id == id && row.status == Status::Pending)
            .ok_or("Only waiting jobs can be moved")?;
        let pending = |at: &usize| rows[*at].status == Status::Pending;
        let to = if earlier {
            (0..from).rev().find(pending)
        } else {
            (from + 1..rows.len()).find(pending)
        };
        if let Some(to) = to {
            rows.swap(from, to);
        }
        Ok(())
    }

    pub fn action(&mut self, id: u64, action: &str) -> Result<(), String> {
        if action == "cancel"
            && let Ok(row) = find_row(&mut self.index.rows, id)
            && row.status == Status::Rendering
        {
            row.details.cancel = true;
            return Ok(());
        }
        self.change(|index| {
            let rows = &mut index.rows;
            match action {
                "clear" => rows.retain(|row| row.status.has_snapshot()),
                "moveUp" | "moveDown" => Self::move_pending(rows, id, action == "moveUp")?,
                _ => {
                    let row = find_row(rows, id)?;
                    match action {
                        "retry" if row.status == Status::Interrupted => {
                            row.status = Status::Pending
                        }
                        "cancel" if row.status == Status::Pending => {
                            row.settle(Status::Cancelled, String::new())
                        }
                        "remove" if row.status != Status::Rendering => {
                            rows.retain(|row| row.id != id)
                        }
                        _ => return Err("This queue action is unavailable".into()),
                    }
                }
            }
            Ok(())
        })?;
        self.remove_orphaned_snapshots();
        Ok(())
    }

    pub fn cancelled(&self, id: u64) -> bool {
        self.index
            .rows
            .iter()
            .any(|row| row.id == id && row.details.cancel)
    }

    pub fn progress(&mut self, id: u64, progress: f64) {
        if let Ok(row) = find_row(&mut self.index.rows, id) {
            row.details.progress = progress.clamp(0., 1.);
        }
    }

    pub fn completed_path(&self, id: u64) -> Option<PathBuf> {
        self.index
            .rows
            .iter()
            .find(|row| {
                row.id == id && row.status == Status::Done && !row.details.destination.is_empty()
            })
            .map(|row| PathBuf::from(&row.details.destination))
    }

    pub fn projection(&self) -> Value {
        let rows: Vec<_> = self.index.rows.iter().map(|row| {
            json!({
                "id": row.id, "name": row.name, "status": row.status, "finishedAt": row.finished_at,
                "format": row.details.format, "profile": row.details.profile,
                "path": row.details.destination, "error": row.details.error,
                "progress": if row.status == Status::Done { 1. } else { row.details.progress },
            })
        }).collect();
        json!(rows)
    }
}

fn find_row(rows: &mut [Row], id: u64) -> Result<&mut Row, String> {
    rows.iter_mut()
        .find(|row| row.id == id)
        .ok_or_else(|| "Queue job missing".into())
}

fn write_atomic(
    path: &Path,
    write: impl FnOnce(&mut BufWriter<&File>) -> Result<(), Box<dyn std::error::Error>>,
) -> Result<(), Box<dyn std::error::Error>> {
    let parent = path.parent().ok_or("Missing render queue directory")?;
    fs::create_dir_all(parent)?;
    let temp = tempfile::NamedTempFile::new_in(parent)?;
    let mut writer = BufWriter::new(temp.as_file());
    write(&mut writer)?;
    writer.flush()?;
    drop(writer);
    temp.as_file().sync_all()?;
    temp.persist(path)?;
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn export_queue_watchdog_can_post_work_while_disk_worker_is_busy() {
        use std::time::{Duration, Instant};
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("queue.json");
        let store = Store::open(path.clone());
        let caller = std::thread::current().id();
        let (entered_tx, entered_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let blocked = store.submit(move |_| {
            assert_ne!(std::thread::current().id(), caller);
            entered_tx.send(()).unwrap();
            release_rx.recv_timeout(Duration::from_secs(3)).unwrap();
            Ok(())
        });
        entered_rx.recv_timeout(Duration::from_secs(3)).unwrap();
        let began = Instant::now();
        let add = store.submit(|queue| queue.add(json!({"name":"Waiting"})));
        let list = store.submit(|queue| Ok(queue.projection()));
        assert!(began.elapsed() < Duration::from_secs(1));
        assert!(!path.exists());
        assert!(list.is_empty());
        release_tx.send(()).unwrap();
        blocked.recv_blocking().unwrap().unwrap();
        assert_eq!(add.recv_blocking().unwrap().unwrap(), 1);
        assert_eq!(list.recv_blocking().unwrap().unwrap()[0]["name"], "Waiting");
    }

    #[test]
    fn export_queue_migrates_legacy_jobs_without_retaining_finished_input() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("queue.json");
        let legacy = json!([
            {"id": 1, "status": "Done", "finished_at": 100, "input": {
                "name": "Completed", "document": "private finished input", "destination": "/private/completed.mp4"
            }},
            {"id": 2, "status": "Pending", "input": {"name": "Waiting", "document": "frozen source"}},
            {"id": 3, "status": "Rendering", "input": {"name": "Was rendering"}}
        ]);
        fs::write(&path, legacy.to_string()).unwrap();
        let mut queue = Queue::load(path.clone());
        assert!(queue.load_error.is_none());
        assert!(!queue.snapshot_path(1).exists());
        assert!(queue.snapshot_path(3).exists());
        assert!(queue.take_pending().unwrap().is_none());
        let index = fs::read_to_string(&path).unwrap();
        assert!(!index.contains("private"));
        assert!(!index.contains("frozen source"));
        assert_eq!(queue.projection()[1]["status"], "Interrupted");
        assert_eq!(queue.projection()[2]["status"], "Interrupted");
        queue.action(2, "retry").unwrap();
        assert_eq!(
            queue.take_pending().unwrap().unwrap(),
            (2, legacy[1]["input"].clone())
        );
        queue.finish(2, "Failed", "Missing media".into()).unwrap();
        assert!(!queue.snapshot_path(2).exists());
        queue.action(3, "remove").unwrap();
        assert!(!queue.snapshot_path(3).exists());
        assert_eq!(queue.add(json!({"name":"New export"})).unwrap(), 4);
    }

    #[test]
    fn export_queue_index_stays_small_and_keeps_only_twenty_history_rows() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("queue.json");
        let mut queue = Queue::load(path.clone());
        let contents = "frozen-project-data".repeat(8_192);
        for index in 0..50 {
            let id = queue
                .add(json!({"name": format!("Export {index}"), "document": contents}))
                .unwrap();
            queue.finish(id, "Done", String::new()).unwrap();
        }
        let index = std::fs::read_to_string(&path).unwrap();
        assert!(
            index.len() <= 64 * 1024,
            "queue index is {} bytes",
            index.len()
        );
        assert!(!index.contains("frozen-project-data"));
        let rows = Queue::load(path).projection();
        let rows = rows.as_array().unwrap();
        assert_eq!(rows.len(), 20);
        assert_eq!(rows.last().unwrap()["name"], "Export 49");
    }

    #[test]
    fn export_queue_keeps_a_waiting_project_outside_the_index() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("queue.json");
        let mut queue = Queue::load(path.clone());
        let id = queue
            .add(json!({"name": "Export", "document": "frozen-project-data"}))
            .unwrap();
        assert!(
            !std::fs::read_to_string(&path)
                .unwrap()
                .contains("frozen-project-data")
        );
        let snapshot = dir
            .path()
            .join("render-jobs")
            .join(format!("{id}.pmv-snapshot"));
        assert!(
            std::fs::read_to_string(&snapshot)
                .unwrap()
                .contains("frozen-project-data")
        );
        queue.finish(id, "Cancelled", String::new()).unwrap();
        assert!(!snapshot.exists());
    }

    #[test]
    fn export_queue_does_not_start_waiting_jobs_from_an_earlier_session() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("queue.json");
        let mut queue = Queue::load(path.clone());
        let running = queue.add(json!({"name": "Rendering"})).unwrap();
        queue.take_pending().unwrap();
        let waiting = queue.add(json!({"name": "Waiting"})).unwrap();
        let mut queue = Queue::load(path);
        assert_eq!(queue.projection()[0]["status"], "Interrupted");
        assert_eq!(queue.projection()[1]["status"], "Interrupted");
        assert!(queue.take_pending().unwrap().is_none());
        queue.action(waiting, "retry").unwrap();
        assert_eq!(queue.take_pending().unwrap().unwrap().0, waiting);
        queue.action(running, "remove").unwrap();
    }

    #[test]
    fn round_trip_order_cancel_and_history() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("queue.json");
        let mut q = Queue::load(path.clone());
        let a = q.add(json!({"name":"First", "document":"frozen"})).unwrap();
        let b = q.add(json!({"name":"Second"})).unwrap();
        let c = q.add(json!({"name":"Third"})).unwrap();
        assert_eq!(q.take_pending().unwrap().unwrap().0, a);
        assert!(q.take_pending().unwrap().is_none());
        q.finish(a, "Failed", "Missing media".into()).unwrap();
        assert!(q.action(a, "retry").is_err());
        q.action(b, "cancel").unwrap();
        assert!(q.index.rows.iter().find(|r| r.id == b).unwrap().finished_at > 0);
        assert_eq!(q.take_pending().unwrap().unwrap().0, c);
        q.action(c, "cancel").unwrap();
        assert!(q.cancelled(c));
        q.finish(c, "Cancelled", String::new()).unwrap();
        let loaded = Queue::load(path);
        assert_eq!(loaded.projection()[0]["name"], "First");
        assert_eq!(loaded.projection()[0]["status"], "Failed");
        assert!(loaded.projection()[0]["finishedAt"].as_u64().unwrap() > 0);
        assert!(loaded.projection()[0].get("document").is_none());
        assert!(!loaded.snapshot_path(a).exists());
        q.action(b, "remove").unwrap();
        assert_eq!(q.index.rows.len(), 2);
    }
    #[test]
    fn pending_order_can_change_while_a_job_renders_and_survives_reload() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("queue.json");
        let mut queue = Queue::load(file.clone());
        let done = queue.add(json!({"name":"Previous export"})).unwrap();
        queue.take_pending().unwrap();
        queue.finish(done, "Done", String::new()).unwrap();
        let running = queue.add(json!({"name":"Current export"})).unwrap();
        let first = queue.add(json!({"name":"First waiting"})).unwrap();
        let second = queue.add(json!({"name":"Second waiting"})).unwrap();
        assert_eq!(queue.take_pending().unwrap().unwrap().0, running);
        assert!(queue.action(running, "moveDown").is_err());
        assert!(queue.action(done, "moveDown").is_err());
        queue.action(second, "moveUp").unwrap();
        assert_eq!(
            queue
                .index
                .rows
                .iter()
                .map(|row| row.id)
                .collect::<Vec<_>>(),
            [done, running, second, first]
        );
        assert_eq!(queue.index.rows[1].status, Status::Rendering);
        assert!(queue.take_pending().unwrap().is_none());
        assert_eq!(
            Queue::load(file)
                .index
                .rows
                .iter()
                .map(|row| row.id)
                .collect::<Vec<_>>(),
            [done, running, second, first]
        );
        queue.finish(running, "Done", String::new()).unwrap();
        assert_eq!(queue.take_pending().unwrap().unwrap().0, second);
    }

    #[test]
    fn relaunch_restores_interrupted_job_for_explicit_retry() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("queue.json");
        let mut q = Queue::load(path.clone());
        let id = q
            .add(json!({"name":"Interrupted", "document": "frozen"}))
            .unwrap();
        q.take_pending().unwrap();
        q.progress(id, 0.75);
        drop(q);
        let mut q = Queue::load(path);
        assert_eq!(q.index.rows[0].status, Status::Interrupted);
        assert_eq!(q.index.rows[0].details.progress, 0.);
        assert!(q.take_pending().unwrap().is_none());
        q.action(id, "retry").unwrap();
        let (taken, input) = q.take_pending().unwrap().unwrap();
        assert_eq!(taken, id);
        assert_eq!(input["document"], "frozen");
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
        assert!(q.index.rows[0].finished_at > 0);
        let running = q.add(json!({"name":"Running"})).unwrap();
        q.take_pending().unwrap();
        let pending = q.add(json!({"name":"Pending"})).unwrap();
        q.action(0, "clear").unwrap();
        assert_eq!(
            q.index.rows.iter().map(|r| r.id).collect::<Vec<_>>(),
            vec![running, pending]
        );
    }
    #[test]
    fn export_queue_index_write_failure_rolls_back_the_new_snapshot() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("queue.json");
        let mut queue = Queue::load(path.clone());
        fs::create_dir(&path).unwrap();
        assert!(
            queue
                .add(json!({"name":"New export", "document":"frozen"}))
                .is_err()
        );
        assert!(queue.index.rows.is_empty());
        assert!(!queue.snapshot_path(1).exists());
        fs::remove_dir(&path).unwrap();
        assert_eq!(queue.add(json!({"name":"Retry"})).unwrap(), 1);
    }

    #[test]
    fn broken_store_is_preserved() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("queue.json");
        std::fs::write(&path, b"broken").unwrap();
        let mut q = Queue::load(path.clone());
        assert!(q.add(json!({})).is_err());
        assert!(q.index.rows.is_empty());
        assert_eq!(std::fs::read(path).unwrap(), b"broken");
    }
}
