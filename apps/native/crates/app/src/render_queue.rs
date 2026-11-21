//! Profile-owned render queue. Frozen inputs stay private in the profile; UI
//! projections contain only labels, destinations and lifecycle facts.
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    io::{BufWriter, Write},
    path::{Path, PathBuf},
    sync::{Arc, Mutex, OnceLock, mpsc},
};
fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
pub type Store = Arc<Mutex<Queue>>;
static STORES: OnceLock<Mutex<HashMap<PathBuf, Store>>> = OnceLock::new();

pub fn shutdown() {
    if let Some(stores) = STORES.get() {
        for queue in stores.lock().unwrap().values() {
            queue.lock().unwrap().writer.stop();
        }
    }
}

pub fn store(profile: &Path) -> Store {
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
    #[serde(with = "shared_input")]
    input: Arc<Value>,
    #[serde(default)]
    cancel: bool,
    #[serde(default)]
    finished_at: u64,
}
pub struct Queue {
    writer: Writer,
    rows: Vec<Row>,
    load_error: Option<String>,
}

mod shared_input {
    use super::*;
    pub fn serialize<S: serde::Serializer>(value: &Arc<Value>, s: S) -> Result<S::Ok, S::Error> {
        value.as_ref().serialize(s)
    }
    pub fn deserialize<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Arc<Value>, D::Error> {
        Value::deserialize(d).map(Arc::new)
    }
}

enum WriteRequest {
    Snapshot(Vec<Row>, Vec<Row>),
    #[cfg(test)]
    Flush(mpsc::Sender<()>),
}

struct Writer {
    sender: Option<mpsc::Sender<WriteRequest>>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Writer {
    fn new(path: PathBuf) -> Self {
        let (sender, receiver) = mpsc::channel();
        let thread = std::thread::Builder::new()
            .name("pm-render-queue-io".into())
            .spawn(move || Self::run(path, receiver))
            .expect("start render queue writer");
        Self {
            sender: Some(sender),
            thread: Some(thread),
        }
    }

    fn run(path: PathBuf, receiver: mpsc::Receiver<WriteRequest>) {
        Self::run_with(receiver, |rows| {
            if let Err(error) = Self::write(&path, rows) {
                eprintln!("Could not save render queue: {error}");
            }
        });
    }

    fn run_with(receiver: mpsc::Receiver<WriteRequest>, mut write: impl FnMut(&[Row])) {
        while let Ok(request) = receiver.recv() {
            let mut latest = None;
            #[allow(unused_mut)]
            let mut flush: Option<mpsc::Sender<()>> = None;
            let mut next = Some(request);
            while let Some(request) = next {
                match request {
                    WriteRequest::Snapshot(rows, retired) => {
                        drop(retired);
                        latest = Some(rows);
                    }
                    #[cfg(test)]
                    WriteRequest::Flush(done) => {
                        flush = Some(done);
                        break;
                    }
                }
                next = receiver.try_recv().ok();
            }
            if let Some(rows) = latest {
                write(&rows);
            }
            if let Some(done) = flush {
                let _ = done.send(());
            }
        }
    }

    fn write(path: &Path, rows: &[Row]) -> anyhow::Result<()> {
        std::fs::create_dir_all(path.parent().unwrap())?;
        let tmp = path.with_extension("json.tmp");
        let mut file = BufWriter::new(std::fs::File::create(&tmp)?);
        serde_json::to_writer(&mut file, rows)?;
        file.flush()?;
        file.get_ref().sync_all()?;
        std::fs::rename(tmp, path)?;
        Ok(())
    }

    fn send(&self, request: WriteRequest) {
        if self
            .sender
            .as_ref()
            .is_none_or(|sender| sender.send(request).is_err())
        {
            eprintln!("Could not save render queue: writer stopped");
        }
    }

    #[cfg(test)]
    fn flush(&self) {
        let (done, wait) = mpsc::channel();
        self.send(WriteRequest::Flush(done));
        wait.recv().unwrap();
    }

    fn stop(&mut self) {
        self.sender.take();
        if let Some(thread) = self.thread.take() {
            if thread.join().is_err() {
                eprintln!("Render queue writer panicked");
            }
        }
    }
}

impl Drop for Writer {
    fn drop(&mut self) {
        self.stop();
    }
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
            writer: Writer::new(path),
            rows,
            load_error,
        }
    }
    fn change<T>(
        &mut self,
        f: impl FnOnce(&mut Vec<Row>) -> Result<T, String>,
    ) -> Result<T, String> {
        if let Some(error) = &self.load_error {
            return Err(error.clone());
        }
        let before = self.rows.clone();
        let value = match f(&mut self.rows) {
            Ok(value) => value,
            Err(error) => {
                self.rows = before;
                return Err(error);
            }
        };
        self.writer
            .send(WriteRequest::Snapshot(self.rows.clone(), before));
        Ok(value)
    }
    pub fn add(&mut self, input: Value) -> Result<u64, String> {
        self.change(|rows| {
            let id = rows.iter().map(|r| r.id).max().unwrap_or(0) + 1;
            rows.push(Row {
                id,
                status: "Pending".into(),
                error: String::new(),
                progress: 0.,
                input: Arc::new(input),
                cancel: false,
                finished_at: 0,
            });
            Ok(id)
        })
    }
    pub fn take_pending(&mut self) -> Result<Option<(u64, Arc<Value>)>, String> {
        if self.rows.iter().any(|r| r.status == "Rendering")
            || !self.rows.iter().any(|r| r.status == "Pending")
        {
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
    fn move_pending(rows: &mut [Row], id: u64, earlier: bool) -> Result<(), String> {
        let from = rows
            .iter()
            .position(|row| row.id == id && row.status == "Pending")
            .ok_or("Only waiting jobs can be moved")?;
        let pending = |at: &usize| rows[*at].status == "Pending";
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
        self.change(|rows| {
            if action == "clear" {
                rows.retain(|r| r.status == "Pending" || r.status == "Rendering");
                return Ok(());
            }
            if matches!(action, "moveUp" | "moveDown") {
                return Self::move_pending(rows, id, action == "moveUp");
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
        assert!(q.rows.iter().find(|r| r.id == b).unwrap().finished_at > 0);
        assert_eq!(q.take_pending().unwrap().unwrap().0, c);
        q.action(c, "cancel").unwrap();
        assert!(q.cancelled(c));
        q.finish(c, "Cancelled", String::new()).unwrap();
        q.writer.flush();
        let loaded = Queue::load(path);
        assert_eq!(loaded.projection(), q.projection());
        assert_eq!(loaded.rows[0].input["document"], "frozen");
        assert_eq!(loaded.rows[0].status, "Done");
        q.action(b, "remove").unwrap();
        assert_eq!(q.rows.len(), 2);
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
            queue.rows.iter().map(|row| row.id).collect::<Vec<_>>(),
            [done, running, second, first]
        );
        assert_eq!(queue.rows[1].status, "Rendering");
        assert!(queue.take_pending().unwrap().is_none());
        queue.writer.flush();
        assert_eq!(
            Queue::load(file)
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
    fn broken_store_is_preserved() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("queue.json");
        std::fs::write(&path, b"broken").unwrap();
        let mut q = Queue::load(path.clone());
        assert!(q.add(json!({})).is_err());
        assert!(q.rows.is_empty());
        assert_eq!(std::fs::read(path).unwrap(), b"broken");
    }

    #[test]
    fn snapshots_coalesce_in_order_and_drop_drains_the_last_write() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("queue.json");
        let (send, receive) = mpsc::channel();
        let mut q = Queue::load(dir.path().join("unused.json"));
        q.add(json!({"document":"frozen"})).unwrap();
        for n in 0..100 {
            q.rows[0].error = n.to_string();
            send.send(WriteRequest::Snapshot(q.rows.clone(), vec![]))
                .unwrap();
        }
        drop(send);
        let mut writes = 0;
        Writer::run_with(receive, |rows| {
            writes += 1;
            Writer::write(&path, rows).unwrap();
        });
        assert_eq!(writes, 1);
        assert_eq!(Queue::load(path).rows[0].error, "99");
        let path = dir.path().join("drop.json");
        let mut q = Queue::load(path.clone());
        let id = q.add(json!({"document":"last"})).unwrap();
        q.finish(id, "Done", String::new()).unwrap();
        drop(q);
        let wire: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(wire[0]["input"], json!({"document":"last"}));
        assert_eq!(Queue::load(path).rows[0].status, "Done");
    }

    #[test]
    fn snapshots_arriving_during_a_write_never_go_backwards() {
        let dir = tempfile::tempdir().unwrap();
        let mut q = Queue::load(dir.path().join("queue.json"));
        q.add(json!({})).unwrap();
        let (send, receive) = mpsc::channel();
        let (started, wait_started) = mpsc::channel();
        let (resume, wait_resume) = mpsc::channel();
        send.send(WriteRequest::Snapshot(q.rows.clone(), vec![]))
            .unwrap();
        let caller = std::thread::current().id();
        let worker = std::thread::spawn(move || {
            let mut written = vec![];
            Writer::run_with(receive, |rows| {
                assert_ne!(std::thread::current().id(), caller);
                if written.is_empty() {
                    started.send(()).unwrap();
                    wait_resume.recv().unwrap();
                }
                written.push(rows[0].error.clone());
            });
            written
        });
        wait_started.recv().unwrap();
        for n in 1..=100 {
            q.rows[0].error = n.to_string();
            send.send(WriteRequest::Snapshot(q.rows.clone(), vec![]))
                .unwrap();
        }
        drop(send);
        resume.send(()).unwrap();
        assert_eq!(worker.join().unwrap(), ["", "100"]);
    }

    #[test]
    fn write_failure_keeps_memory_authoritative_and_writer_recovers() {
        let dir = tempfile::tempdir().unwrap();
        let parent = dir.path().join("blocked");
        std::fs::create_dir(&parent).unwrap();
        let mut q = Queue::load(parent.join("queue.json"));
        std::fs::remove_dir(&parent).unwrap();
        std::fs::write(&parent, b"file").unwrap();
        let id = q.add(json!({"name":"retained"})).unwrap();
        q.writer.flush();
        assert_eq!(q.rows.len(), 1);
        std::fs::remove_file(&parent).unwrap();
        q.finish(id, "Done", String::new()).unwrap();
        q.writer.flush();
        assert_eq!(
            Queue::load(parent.join("queue.json")).rows[0].status,
            "Done"
        );
    }

    #[test]
    fn idle_take_pending_does_not_schedule_a_write() {
        let dir = tempfile::tempdir().unwrap();
        let mut q = Queue::load(dir.path().join("queue.json"));
        let (send, receive) = mpsc::channel();
        q.writer.stop();
        q.writer.sender = Some(send);
        assert!(q.take_pending().unwrap().is_none());
        assert!(matches!(receive.try_recv(), Err(mpsc::TryRecvError::Empty)));
    }

    #[test]
    fn existing_wire_format_loads_and_stays_unchanged() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("queue.json");
        let bytes = br#"[{"id":1,"status":"Done","error":"","progress":1.0,"input":{"document":"{\"layers\":[]}"},"cancel":false,"finished_at":12}]"#;
        std::fs::write(&path, bytes).unwrap();
        let mut q = Queue::load(path.clone());
        q.finish(1, "Rendering", String::new()).unwrap();
        q.rows[0].status = "Done".into();
        q.writer
            .send(WriteRequest::Snapshot(q.rows.clone(), vec![]));
        q.writer.flush();
        assert_eq!(std::fs::read(path).unwrap(), bytes);
    }

    #[test]
    fn large_history_transition_is_pointer_only() {
        let dir = tempfile::tempdir().unwrap();
        let mut q = Queue::load(dir.path().join("queue.json"));
        for id in 1..=20 {
            q.rows.push(Row {
                id,
                status: if id == 1 { "Failed" } else { "Done" }.into(),
                error: String::new(),
                progress: 1.,
                input: Arc::new(json!({"document":format!("{{\"layers\":[{}null]}}",
                    "{\"name\":\"synthetic\",\"value\":0},".repeat(70_000))})),
                cancel: false,
                finished_at: 1,
            });
        }
        let inputs: Vec<_> = q.rows.iter().map(|r| r.input.clone()).collect();
        let began = std::time::Instant::now();
        q.action(1, "retry").unwrap();
        let elapsed = began.elapsed();
        println!("new caller transition, 20 x ~2 MiB: {elapsed:?}");
        assert!(
            elapsed < std::time::Duration::from_millis(100),
            "{elapsed:?}"
        );
        for (row, input) in q.rows.iter().zip(inputs) {
            assert!(Arc::ptr_eq(&row.input, &input));
        }

        // Reproduce the previous change/action/save path, including its deep
        // rollback copy and unbuffered serializer, on the same payload.
        #[derive(Clone, Serialize, Deserialize)]
        struct LegacyRow {
            id: u64,
            status: String,
            error: String,
            progress: f64,
            input: Value,
            cancel: bool,
            finished_at: u64,
        }
        let mut old: Vec<LegacyRow> = q
            .rows
            .iter()
            .map(|r| serde_json::from_value(serde_json::to_value(r).unwrap()).unwrap())
            .collect();
        old[0].status = "Failed".into();
        let began = std::time::Instant::now();
        let before = old.clone();
        old[0].status = "Pending".into();
        let path = dir.path().join("old.json");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let tmp = path.with_extension("json.tmp");
        let file = std::fs::File::create(&tmp).unwrap();
        serde_json::to_writer(&file, &old).unwrap();
        file.sync_all().unwrap();
        std::fs::rename(tmp, path).unwrap();
        drop(before);
        println!("old caller transition, 20 x ~2 MiB: {:?}", began.elapsed());
    }
}
