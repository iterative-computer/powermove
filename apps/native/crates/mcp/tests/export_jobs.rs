use pm_mcp::{
    Automation, McpSession, Result,
    export_jobs::{ExportJobs, Progress},
    services::{ExportChild, FrozenProject, VideoExportRequest},
};
use serde_json::{Value, json};
use std::{
    fs,
    io::Write,
    path::PathBuf,
    process::{Command, Stdio},
    sync::{Arc, Condvar, Mutex, mpsc},
    time::Duration,
};
#[derive(Default)]
struct Gate(Mutex<bool>, Condvar);
impl Gate {
    fn release(&self) {
        *self.0.lock().unwrap() = true;
        self.1.notify_all()
    }
    fn wait(&self) {
        drop(self.1.wait_while(self.0.lock().unwrap(), |v| !*v).unwrap());
    }
}
struct Child {
    gate: Gate,
    progress_gate: Gate,
    progress_started: Mutex<Option<mpsc::Sender<()>>>,
    progress: bool,
    calls: Mutex<Vec<VideoExportRequest>>,
}
impl Child {
    fn new(progress: bool) -> Self {
        let progress_gate = Gate::default();
        progress_gate.release();
        Self {
            gate: Gate::default(),
            progress_gate,
            progress_started: Mutex::new(None),
            progress,
            calls: Mutex::new(vec![]),
        }
    }
}
impl ExportChild for Child {
    fn video(&self, r: &VideoExportRequest) -> Result<PathBuf> {
        self.calls.lock().unwrap().push(r.clone());
        self.gate.wait();
        fs::create_dir_all(&r.directory)?;
        let path = r.directory.join("synthetic-export.bin");
        let mut out = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)?;
        out.write_all(b"Synthetic worker output; not a video conformance witness")?;
        Ok(path)
    }
    fn video_progress(&self, _: &FrozenProject) -> Option<Progress> {
        if let Some(signal) = self.progress_started.lock().unwrap().take() {
            signal.send(()).unwrap();
        }
        self.progress_gate.wait();
        self.progress.then_some(Progress {
            frames: 10,
            total: Some(20),
            encoder: Some("synthetic".into()),
            seconds: 2.0,
        })
    }
}
struct Fixture {
    root: PathBuf,
    host: Arc<Mutex<Automation>>,
    child: Arc<Child>,
}
impl Fixture {
    fn new(progress: bool) -> Self {
        let root = std::env::temp_dir().join(pm_kernel::new_id("mcp-export-job"));
        fs::create_dir_all(&root).unwrap();
        let root = root.canonicalize().unwrap();
        let mut host = Automation::new(root.join("profile"), root.join("exports")).unwrap();
        host.create_project(&json!({"name":"Synthetic job"}))
            .unwrap();
        let child = Arc::new(Child::new(progress));
        host.export_child = child.clone();
        host.export_jobs = Arc::new(ExportJobs::with_limits(
            Duration::ZERO,
            Duration::from_secs(12 * 60 * 60),
        ));
        Self {
            root,
            host: Arc::new(Mutex::new(host)),
            child,
        }
    }
    fn session(&self) -> McpSession {
        let mut s = McpSession::new(self.host.clone());
        s.dispatch("start_session", &json!({"prompt":"Synthetic export job"}))
            .unwrap();
        s
    }
    fn drain(&self) {
        self.child.gate.release();
        let jobs = self.host.lock().unwrap().export_jobs.clone();
        jobs.drain(Duration::from_secs(5)).unwrap();
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.child.gate.release();
        self.child.progress_gate.release();
        if let Ok(host) = self.host.lock() {
            let _ = host.export_jobs.drain(Duration::from_secs(5));
        }
        let _ = fs::remove_dir_all(&self.root);
    }
}
fn oracle(input: Value) -> Value {
    let mut child = Command::new("node")
        .arg(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("scripts/export-job-pairs.mjs"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(input.to_string().as_bytes())
        .unwrap();
    let out = child.wait_with_output().unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).unwrap()
}
#[test]
fn running_progress_shape_and_missing_progress_match_main_then_result_is_once() {
    for progress in [false, true] {
        let f = Fixture::new(progress);
        let mut s = f.session();
        let running = s.dispatch("export_video", &json!({})).unwrap();
        let id = running["jobId"].as_str().unwrap();
        let expected = oracle(
            json!({"jobId":id,"arguments":{"jobId":id},"progress":if progress{json!({"frames":10,"total":20,"encoder":"synthetic","seconds":2})}else{Value::Null}}),
        );
        let expected: Value =
            serde_json::from_str(expected["result"]["content"][0]["text"].as_str().unwrap())
                .unwrap();
        assert_eq!(running, expected);
        f.drain();
        let result = s.dispatch("export_video", &json!({"jobId":id})).unwrap();
        assert!(PathBuf::from(result["path"].as_str().unwrap()).is_file());
        assert_eq!(
            s.dispatch("export_video", &json!({"jobId":id}))
                .unwrap_err()
                .message,
            "Unknown export job. A finished job is reported once; check the export folder."
        );
        assert_eq!(f.child.calls.lock().unwrap().len(), 1);
    }
}
#[test]
fn resume_argument_errors_and_second_export_refusal_match_main() {
    let f = Fixture::new(false);
    let mut s = f.session();
    let running = s.dispatch("export_video", &json!({})).unwrap();
    let id = running["jobId"].as_str().unwrap();
    for args in [
        json!({"jobId":1}),
        json!({"jobId":id,"format":"mp4"}),
        json!({"jobId":"missing"}),
        json!({"name":"Second"}),
    ] {
        let expected = oracle(json!({"jobId":id,"arguments":args}));
        assert_eq!(
            s.dispatch("export_video", &args).unwrap_err().message,
            expected["error"]
        );
    }
    f.drain();
    assert_eq!(f.child.calls.lock().unwrap().len(), 1);
}
#[test]
fn session_finish_does_not_cancel_an_owned_export() {
    let f = Fixture::new(false);
    let mut s = f.session();
    let running = s.dispatch("export_video", &json!({})).unwrap();
    s.dispatch("finish_session", &json!({"commit":false}))
        .unwrap();
    let mut next = f.session();
    assert_eq!(
        next.dispatch("export_video", &json!({"jobId":running["jobId"]}))
            .unwrap()["status"],
        "running"
    );
    f.drain();
    assert!(
        next.dispatch("export_video", &json!({"jobId":running["jobId"]}))
            .unwrap()["path"]
            .is_string()
    );
}
#[test]
fn waiting_releases_host_mutex_and_document_replacement_never_admits_old_job() {
    let f = Fixture::new(false);
    *f.child.progress_gate.0.lock().unwrap() = false;
    let (tx, rx) = mpsc::channel();
    *f.child.progress_started.lock().unwrap() = Some(tx);
    let shared = f.host.clone();
    let waiter = std::thread::spawn(move || {
        let mut s = McpSession::new(shared);
        s.dispatch("start_session", &json!({"prompt":"Waiting export"}))
            .unwrap();
        s.dispatch("export_video", &json!({}))
    });
    rx.recv_timeout(Duration::from_secs(5)).unwrap();
    {
        let mut host = f
            .host
            .try_lock()
            .expect("MCP must release the kernel-host mutex before waiting/progress");
        host.create_project(&json!({"name":"Replacement"})).unwrap();
    }
    // Completion/draining must also proceed while the host's progress callback
    // is blocked; no internal result mutex may be held across that callback.
    f.child.gate.release();
    let jobs = f.host.lock().unwrap().export_jobs.clone();
    jobs.drain(Duration::from_secs(5)).unwrap();
    f.child.progress_gate.release();
    assert!(waiter.join().unwrap().is_err());
    let mut next = f.session();
    assert_eq!(
        next.dispatch("export_video", &json!({"jobId":"not-owner"}))
            .unwrap_err()
            .message,
        "Unknown export job. A finished job is reported once; check the export folder."
    );
    f.drain();
}
#[test]
fn deadline_error_keeps_pending_worker_visible_to_drain_without_claiming_cancellation() {
    let f = Fixture::new(false);
    f.host.lock().unwrap().export_jobs =
        Arc::new(ExportJobs::with_limits(Duration::ZERO, Duration::ZERO));
    let mut s = f.session();
    assert!(
        s.dispatch("export_video", &json!({}))
            .unwrap_err()
            .message
            .starts_with("Powermove timed out while running __mcp_export_video.")
    );
    let jobs = f.host.lock().unwrap().export_jobs.clone();
    assert!(jobs.drain(Duration::ZERO).is_err());
    f.drain();
    assert!(jobs.drain(Duration::ZERO).is_ok());
}
