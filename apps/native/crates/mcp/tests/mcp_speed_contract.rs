use pm_mcp::{
    Automation, McpSession, Result,
    export_jobs::ExportJobs,
    services::{ExportChild, VideoDelivered, VideoExportRequest},
};
use serde_json::{Value, json};
use std::{
    fs,
    path::PathBuf,
    sync::{Arc, Condvar, Mutex, mpsc},
    time::Duration,
};

#[derive(Default)]
struct ExportGate {
    released: Mutex<bool>,
    wake: Condvar,
}

impl ExportGate {
    fn release(&self) {
        *self.released.lock().unwrap() = true;
        self.wake.notify_all();
    }
}

impl ExportChild for ExportGate {
    fn video(&self, request: &VideoExportRequest) -> Result<VideoDelivered> {
        drop(
            self.wake
                .wait_while(self.released.lock().unwrap(), |released| !*released)
                .unwrap(),
        );
        let path = request.directory.join("synthetic.mp4");
        fs::create_dir_all(&request.directory)?;
        fs::write(&path, "synthetic output")?;
        Ok(path.into())
    }
}

struct Fixture {
    root: PathBuf,
    host: Arc<Mutex<Automation>>,
    id: String,
    gate: Arc<ExportGate>,
}

impl Fixture {
    fn new(wait: Duration) -> Self {
        let root = std::env::temp_dir().join(pm_kernel::new_id("mcp-speed-contract"));
        fs::create_dir(&root).unwrap();
        let root = root.canonicalize().unwrap();
        let mut host = Automation::new(root.join("profile"), root.join("exports")).unwrap();
        let id = host
            .create_project(&json!({"name":"Synthetic MCP speed fixture"}))
            .unwrap()["projectId"]
            .as_str()
            .unwrap()
            .to_owned();
        let gate = Arc::new(ExportGate::default());
        host.export_child = gate.clone();
        host.export_jobs = Arc::new(ExportJobs::with_limits(wait, Duration::from_secs(30)));
        Self {
            root,
            host: Arc::new(Mutex::new(host)),
            id,
            gate,
        }
    }

    fn start(&self) -> McpSession {
        let mut session = McpSession::new(self.host.clone());
        session
            .dispatch("start_session", &json!({"prompt":"Synthetic speed test"}))
            .unwrap();
        session
    }

    fn layers(&self) -> Value {
        self.host.lock().unwrap().read_document(&self.id).unwrap()["layers"].clone()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.gate.release();
        let jobs = self.host.lock().unwrap().export_jobs.clone();
        let _ = jobs.drain(Duration::from_secs(5));
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn add_layer(id: &str) -> Value {
    json!({"type":"add_layer","id":id,"name":id,"layerType":"solid"})
}

#[test]
fn background_export_allows_another_call_on_the_same_connection() {
    let fixture = Fixture::new(Duration::from_secs(10));
    let mut session = fixture.start();
    let (sent, received) = mpsc::channel();
    let worker = std::thread::spawn(move || {
        let submitted = session.dispatch("export_video", &json!({"background":true}));
        if submitted
            .as_ref()
            .is_err_and(|e| e.message == "Unknown arguments.background")
        {
            // Exercise the old path on the baseline, which rejects the opt-in.
            let _ = session.dispatch("export_video", &json!({}));
        }
        let read = session.dispatch("get_project_state", &json!({"indexOnly":true}));
        sent.send((submitted, read)).unwrap();
    });
    let reply = received.recv_timeout(Duration::from_secs(2));
    fixture.gate.release();
    worker.join().unwrap();
    let (submitted, read) = reply.expect("submission and subsequent read must not wait for encode");
    let submitted = submitted.unwrap();
    assert_eq!(submitted["status"], "running");
    assert!(submitted["jobId"].is_string());
    assert!(read.is_ok());
}

#[test]
fn disconnect_preserves_checkpoint_but_rolls_back_later_edits() {
    let fixture = Fixture::new(Duration::ZERO);
    let mut session = fixture.start();
    session
        .dispatch(
            "apply_commands",
            &json!({"commands":[add_layer("before-export")]}),
        )
        .unwrap();
    let queued = session.dispatch("export_video", &json!({})).unwrap();
    assert!(queued["jobId"].is_string());
    session
        .dispatch(
            "apply_commands",
            &json!({"commands":[add_layer("after-export")]}),
        )
        .unwrap();
    session.close().unwrap();
    let layers = fixture.layers();
    assert_eq!(layers.as_array().unwrap().len(), 1);
    assert_eq!(layers[0]["id"], "before-export");
}

#[test]
fn start_has_a_bounded_layer_index_with_detail_available() {
    let fixture = Fixture::new(Duration::ZERO);
    let mut writer = fixture.start();
    let layers = (0..180)
        .map(|n| {
            let mut layer = add_layer(&format!("layer-{n}"));
            layer["name"] = json!(format!("Layer {n} {}", "\"\\long".repeat(25)));
            layer
        })
        .collect::<Vec<_>>();
    writer
        .dispatch("apply_commands", &json!({"commands":layers}))
        .unwrap();
    writer
        .dispatch("finish_session", &json!({"commit":true}))
        .unwrap();

    let mut session = McpSession::new(fixture.host.clone());
    let reply = session.call("start_session", json!({"prompt":"Compact startup"}));
    assert_eq!(reply["isError"], false);
    assert!(reply.to_string().len() < 20_000);
    let body: Value = serde_json::from_str(reply["content"][0]["text"].as_str().unwrap()).unwrap();
    let comp = &body["summary"]["compositions"][0];
    assert_eq!(
        comp["layerCount"], 180,
        "startup must include a compact layer index"
    );
    assert!(comp["id"].is_string());
    assert_eq!(comp["width"], 1920);
    assert_eq!(comp["height"], 1080);
    assert_eq!(comp["fps"], 30);
    assert_eq!(comp["layers"][0]["type"], "solid");
    assert!(comp["layers"].as_array().unwrap().len() < 180);
    assert_eq!(body["summary"]["mediaCount"], 0);
    assert!(body["summary"]["selection"]["layers"].is_array());
    session
        .dispatch("finish_session", &json!({"commit":true}))
        .unwrap();
    let full = session
        .dispatch(
            "start_session",
            &json!({"prompt":"Full startup","detail":"full"}),
        )
        .unwrap();
    assert_eq!(full["project"]["layers"].as_array().unwrap().len(), 180);
}

#[test]
fn partial_batch_reports_failed_indices_and_commits_as_one_undo() {
    let fixture = Fixture::new(Duration::ZERO);
    let mut session = fixture.start();
    let batch = session
        .dispatch(
            "apply_commands",
            &json!({"atomic":false,"commands":[
                add_layer("kept"),
                "bad command text",
                {"type":"set_property","target":"missing","path":"opacity","value":20},
                {"type":"set_layer","target":"kept","patch":{"name":"renamed"}},
                add_layer("also-kept")
            ]}),
        )
        .unwrap();
    let results = batch["results"].as_array().unwrap();
    assert_eq!(results.len(), 5);
    for (index, ok) in [true, false, false, true, true].into_iter().enumerate() {
        assert_eq!(results[index]["index"], index);
        assert_eq!(results[index]["ok"], ok);
        if !ok {
            assert!(results[index]["message"].is_string());
        }
    }
    let edited = fixture.layers();
    assert_eq!(edited.as_array().unwrap().len(), 2);
    assert!(
        edited
            .as_array()
            .unwrap()
            .iter()
            .any(|layer| layer["name"] == "renamed")
    );
    session
        .dispatch("finish_session", &json!({"commit":true}))
        .unwrap();
    fixture
        .host
        .lock()
        .unwrap()
        .undo(&fixture.id, pm_kernel::Origin::Mcp, 5)
        .unwrap();
    assert_eq!(fixture.layers(), json!([]));
}

#[test]
fn batch_results_are_compact_unless_verbose() {
    let fixture = Fixture::new(Duration::ZERO);
    let mut session = fixture.start();
    let compact = session
        .dispatch("apply_commands", &json!({"commands":[add_layer("short")]}))
        .unwrap();
    assert!(
        compact["results"][0]["data"].get("layer").is_none(),
        "default returned a full layer"
    );
    assert!(compact["results"][0].get("command").is_none());
    assert_eq!(compact["results"][0]["index"], 0);
    assert_eq!(compact["results"][0]["ok"], true);
    assert_eq!(compact["newIds"], json!(["short"]));
    assert_eq!(compact["changedLayerIds"], json!(["short"]));
    let verbose = session
        .dispatch(
            "apply_commands",
            &json!({"verbose":true,"commands":[add_layer("long")]}),
        )
        .unwrap();
    assert_eq!(verbose["results"][0]["data"]["layer"]["id"], "long");
}
