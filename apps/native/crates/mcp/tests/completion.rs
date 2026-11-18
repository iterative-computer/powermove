//! Synthetic fixtures only: no provider, window or user's profile.
use pm_mcp::{Automation, McpSession};
use serde_json::json;
use std::sync::{Arc, Mutex};
fn fixture() -> (tempfile::TempDir, Arc<Mutex<Automation>>, String) {
    let root = tempfile::tempdir().unwrap();
    let host = Arc::new(Mutex::new(
        Automation::new(root.path().into(), root.path().join("exports")).unwrap(),
    ));
    let id = host
        .lock()
        .unwrap()
        .create_project(&json!({"name":"Completion fixture"}))
        .unwrap()["projectId"]
        .as_str()
        .unwrap()
        .to_owned();
    host.lock()
        .unwrap()
        .completion_action("begin", &id, "run", &json!({}))
        .unwrap();
    (root, host, id)
}
#[test]
fn apply_plan_and_undo_use_a_host_receipt() {
    let (_root, host, id) = fixture();
    let mut host = host.lock().unwrap();
    let rev = host.revision(&id).unwrap();
    let result = host.completion_action("apply", &id, "run", &json!({"revision":rev,"commands":[{"type":"add_layer","id":"agent","layerType":"solid"}]})).unwrap();
    assert_eq!(result["changed"], true);
    assert_eq!(
        host.read_document(&id).unwrap()["layers"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    host.completion_action("undo", &id, "run", &result).unwrap();
    assert_eq!(
        host.read_document(&id).unwrap()["layers"]
            .as_array()
            .unwrap()
            .len(),
        0
    );
}
#[test]
fn undo_refuses_a_later_human_edit_without_changing_anything() {
    let (_root, host, id) = fixture();
    let mut h = host.lock().unwrap();
    let rev = h.revision(&id).unwrap();
    let receipt = h.completion_action("apply", &id, "run", &json!({"revision":rev,"commands":[{"type":"add_layer","id":"agent","layerType":"solid"}]})).unwrap();
    drop(h);
    let mut human = McpSession::new(host.clone());
    human
        .dispatch("start_session", &json!({"prompt":"Human fixture"}))
        .unwrap();
    human
        .dispatch(
            "apply_commands",
            &json!({"commands":[{"type":"add_layer","id":"human","layerType":"solid"}]}),
        )
        .unwrap();
    human.finish(true).unwrap();
    let mut h = host.lock().unwrap();
    let before = h.read_document(&id).unwrap();
    let error = h
        .completion_action("undo", &id, "run", &receipt)
        .unwrap_err();
    assert!(error.message.contains("Other edits"));
    assert_eq!(h.read_document(&id).unwrap(), before);
}
#[test]
fn completed_run_verifies_frames_and_reports_unavailable_checks() {
    let (_root, host, id) = fixture();
    let mut h = host.lock().unwrap();
    let rev = h.revision(&id).unwrap();
    h.completion_action(
        "apply",
        &id,
        "run",
        &json!({"revision":rev,"commands":[{"type":"add_layer","id":"agent","layerType":"solid"}]}),
    )
    .unwrap();
    let report = h
        .completion_action("verify", &id, "run", &json!({}))
        .unwrap();
    assert_eq!(report["changed"], true);
    assert!(!report["errors"].as_array().unwrap().is_empty());
}
#[test]
fn stale_plan_applies_nothing() {
    let (_root, host, id) = fixture();
    let mut h = host.lock().unwrap();
    assert!(h.completion_action("apply", &id, "run", &json!({"revision":999,"commands":[{"type":"add_layer","id":"agent","layerType":"solid"}]})).is_err());
    assert!(
        h.read_document(&id).unwrap()["layers"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}

struct Frames(std::sync::atomic::AtomicUsize);
impl pm_mcp::EngineHook for Frames {
    fn capture(&self, request: &pm_mcp::CaptureRequest) -> pm_mcp::Result<serde_json::Value> {
        self.0.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        assert!(request.times.len() <= 5);
        assert_eq!(request.width, 480);
        Ok(
            json!({"images":request.times.iter().map(|_| "data:image/png;base64,AA==").collect::<Vec<_>>()}),
        )
    }
}
fn owned_session(host: &Arc<Mutex<Automation>>, id: &str) -> McpSession {
    let mut h = host.lock().unwrap();
    h.agent_grants = Some(Arc::new(|_| Some(("run".into(), "project".into()))));
    h.completion_action("begin", id, "run", &json!({})).unwrap();
    drop(h);
    let mut session = McpSession::new(host.clone());
    session.set_agent_run("synthetic-token");
    session
        .dispatch("start_session", &json!({"prompt":"Synthetic agent edit","context":if id == "builtin-application" {"app"} else {"project"}}))
        .unwrap();
    session
}
#[test]
fn committed_tool_edits_capture_actual_final_frames_and_undo() {
    let (_root, host, id) = fixture();
    let frames = Arc::new(Frames(Default::default()));
    host.lock().unwrap().engine = Some(frames.clone());
    let mut session = owned_session(&host, &id);
    session
        .dispatch(
            "apply_commands",
            &json!({"commands":[{"type":"add_layer","id":"agent","layerType":"solid"}]}),
        )
        .unwrap();
    session.finish(true).unwrap();
    let mut h = host.lock().unwrap();
    let report = h
        .completion_action("verify", &id, "run", &json!({}))
        .unwrap();
    assert_eq!(report["changed"], true);
    assert!(report["errors"].as_array().unwrap().is_empty(), "{report}");
    assert!(!report["images"].as_array().unwrap().is_empty());
    assert_eq!(frames.0.load(std::sync::atomic::Ordering::Relaxed), 1);
    h.completion_action("undo", &id, "run", &report).unwrap();
    assert!(
        h.read_document(&id).unwrap()["layers"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}
#[test]
fn coediting_before_a_repair_refuses_snapshot_undo() {
    let (_root, host, id) = fixture();
    let mut agent = owned_session(&host, &id);
    agent
        .dispatch(
            "apply_commands",
            &json!({"commands":[{"type":"add_layer","id":"agent","layerType":"solid"}]}),
        )
        .unwrap();
    let mut human = McpSession::new(host.clone());
    human
        .dispatch("start_session", &json!({"prompt":"Human"}))
        .unwrap();
    human
        .dispatch(
            "apply_commands",
            &json!({"commands":[{"type":"add_layer","id":"human","layerType":"solid"}]}),
        )
        .unwrap();
    human.finish(true).unwrap();
    agent.dispatch("get_project_state", &json!({})).unwrap();
    agent
        .dispatch(
            "apply_commands",
            &json!({"commands":[{"type":"add_layer","id":"repair","layerType":"solid"}]}),
        )
        .unwrap();
    agent.finish(true).unwrap();
    let mut h = host.lock().unwrap();
    let before = h.read_document(&id).unwrap();
    assert!(
        h.completion_action("undo", &id, "run", &json!({}))
            .unwrap_err()
            .message
            .contains("Other edits")
    );
    assert_eq!(h.read_document(&id).unwrap(), before);
}
#[test]
fn keeping_a_result_retains_editor_history_and_drops_its_private_receipt() {
    let (_root, host, id) = fixture();
    let mut session = owned_session(&host, &id);
    session
        .dispatch(
            "apply_commands",
            &json!({"commands":[{"type":"add_layer","id":"agent","layerType":"solid"}]}),
        )
        .unwrap();
    session.finish(true).unwrap();
    let mut h = host.lock().unwrap();
    h.completion_action("keep", &id, "run", &json!({})).unwrap();
    h.undo(&id, pm_kernel::Origin::Mcp, 101).unwrap();
    assert!(
        h.read_document(&id).unwrap()["layers"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert!(h.completion_action("undo", &id, "run", &json!({})).is_err());
}

struct Extensions(std::path::PathBuf);
impl pm_mcp::extensions::ExtensionHost for Extensions {
    fn roots(&self) -> Vec<std::path::PathBuf> {
        vec![self.0.join("live")]
    }
    fn history(&self) -> std::path::PathBuf {
        self.0.join("history")
    }
    fn builtins(&self) -> Option<std::path::PathBuf> {
        None
    }
    fn manifest(&self, raw: &serde_json::Value) -> Result<serde_json::Value, String> {
        Ok(raw.clone())
    }
    fn check(&self, _: &std::path::Path, _: &serde_json::Value) -> Result<(), String> {
        Ok(())
    }
    fn reload(&self, _: &[String]) {}
    fn api_pack(&self) -> Vec<(String, String)> {
        vec![]
    }
}
struct Health(bool);
impl pm_mcp::SurfaceHook for Health {
    fn call(
        &self,
        tool: &str,
        _: Option<&str>,
        _: &serde_json::Value,
    ) -> pm_mcp::Result<serde_json::Value> {
        if tool == "open_panel" {
            return Ok(json!({"opened":"contributed-panel"}));
        }
        assert_eq!(tool, "get_workspace_state");
        Ok(
            json!({"errors":[],"extensionPanels":[{"extensionId":"fixture-panel","panelId":"contributed-panel"}],"extensions":[{"id":"fixture-panel","enabled":true,"health":{"state":if self.0 {"ok"} else {"activation-error"}}}]}),
        )
    }
}
fn publish_fixture(
    host: &Arc<Mutex<Automation>>,
    id: &str,
    root: &std::path::Path,
) -> serde_json::Value {
    host.lock().unwrap().extensions = Some(Arc::new(Extensions(root.into())));
    host.lock().unwrap().engine = Some(Arc::new(Frames(Default::default())));
    let mut session = owned_session(host, id);
    let stages = std::fs::read_dir(root.join("mcp-runs"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect::<Vec<_>>();
    let stage = stages[0].join("extensions/fixture-panel");
    std::fs::create_dir_all(&stage).unwrap();
    std::fs::write(stage.join("manifest.json"),json!({"id":"fixture-panel","name":"Fixture panel","version":"1.0.0","apiVersion":4,"entry":"index.tsx"}).to_string()).unwrap();
    std::fs::write(stage.join("index.tsx"), "export const fixture = true;").unwrap();
    let receipt=session.dispatch("publish_extensions",&json!({"extensions":[{"id":"fixture-panel","action":"created","summary":"Synthetic panel"}]})).unwrap();
    session.finish(true).unwrap();
    receipt
}
#[test]
fn extension_results_check_live_health_and_restore_the_published_change_set() {
    let (root, host, id) = fixture();
    let published = publish_fixture(&host, &id, root.path());
    assert!(published["changeSetId"].is_string());
    let mut h = host.lock().unwrap();
    h.surfaces = Some(Arc::new(Health(false)));
    let failed = h
        .completion_action("verify", &id, "run", &json!({}))
        .unwrap();
    assert_eq!(failed["changed"], true);
    assert!(!failed["errors"].as_array().unwrap().is_empty());
    h.surfaces = Some(Arc::new(Health(true)));
    let ready = h
        .completion_action("verify", &id, "run", &json!({}))
        .unwrap();
    assert!(ready["errors"].as_array().unwrap().is_empty(), "{ready}");
    assert_eq!(ready["extensions"][0]["extensionId"], "fixture-panel");
    assert_eq!(ready["extensions"][0]["panelId"], "contributed-panel");
    assert_eq!(ready["extensions"][0]["status"], "ready");
    assert!(
        h.completion_action(
            "openPanel",
            &id,
            "run",
            &json!({"extensionId":"fixture-panel","panelId":"fixture-panel"})
        )
        .is_err()
    );
    assert_eq!(
        h.completion_action(
            "openPanel",
            &id,
            "run",
            &json!({"extensionId":"fixture-panel","panelId":"contributed-panel"})
        )
        .unwrap()["opened"],
        "contributed-panel"
    );
    h.completion_action("undo", &id, "run", &ready).unwrap();
    assert!(!root.path().join("live/fixture-panel").exists());
}
#[test]
fn extension_undo_refuses_later_authored_files() {
    let (root, host, id) = fixture();
    publish_fixture(&host, &id, root.path());
    let path = root.path().join("live/fixture-panel/index.tsx");
    std::fs::write(&path, "export const human = true;").unwrap();
    let error = host
        .lock()
        .unwrap()
        .completion_action("undo", &id, "run", &json!({}))
        .unwrap_err();
    assert!(error.message.contains("Newer extension changes"), "{error}");
    assert_eq!(
        std::fs::read_to_string(path).unwrap(),
        "export const human = true;"
    );
}

#[test]
fn child_edits_are_verified_and_recovered_by_the_parent_receipt() {
    let (_root, host, id) = fixture();
    let frames = Arc::new(Frames(Default::default()));
    {
        let mut h = host.lock().unwrap();
        h.engine = Some(frames.clone());
        h.completion_action("bindChild", &id, "child-run", &json!({"rootRunId":"run"}))
            .unwrap();
        h.agent_grants = Some(Arc::new(|_| Some(("child-run".into(), "project".into()))));
    }
    let mut child = McpSession::new(host.clone());
    child.set_agent_run("synthetic-child-token");
    child
        .dispatch(
            "start_session",
            &json!({"prompt":"Synthetic delegated edit"}),
        )
        .unwrap();
    child
        .dispatch(
            "apply_commands",
            &json!({"commands":[{"type":"add_layer","id":"child-layer","layerType":"solid"}]}),
        )
        .unwrap();
    child.finish(true).unwrap();
    let mut h = host.lock().unwrap();
    let report = h
        .completion_action("verify", &id, "run", &json!({}))
        .unwrap();
    assert_eq!(report["changed"], true);
    assert_eq!(frames.0.load(std::sync::atomic::Ordering::Relaxed), 1);
    h.completion_action("undo", &id, "run", &report).unwrap();
    assert!(
        h.read_document(&id).unwrap()["layers"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}

#[test]
fn starting_the_next_task_releases_only_that_threads_old_receipt() {
    let (_root, host, id) = fixture();
    let mut h = host.lock().unwrap();
    h.completion_action("begin", &id, "first", &json!({"threadId":"thread-a"}))
        .unwrap();
    h.completion_action("begin", &id, "other", &json!({"threadId":"thread-b"}))
        .unwrap();
    let rev = h.revision(&id).unwrap();
    h.completion_action(
        "apply",
        &id,
        "first",
        &json!({"revision":rev,"commands":[{"type":"add_layer","id":"agent","layerType":"solid"}]}),
    )
    .unwrap();
    h.completion_action("begin", &id, "next", &json!({"threadId":"thread-a"}))
        .unwrap();
    assert!(
        h.completion_action("undo", &id, "first", &json!({}))
            .unwrap_err()
            .message
            .contains("no recovery receipt")
    );
    assert!(
        h.completion_action("verify", &id, "other", &json!({}))
            .is_ok()
    );
    h.undo(&id, pm_kernel::Origin::Mcp, 101).unwrap();
    assert!(
        h.read_document(&id).unwrap()["layers"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}

struct AppHealth;
impl pm_mcp::SurfaceHook for AppHealth {
    fn call(
        &self,
        tool: &str,
        project: Option<&str>,
        args: &serde_json::Value,
    ) -> pm_mcp::Result<serde_json::Value> {
        assert!(
            project.is_none(),
            "App extensions have no composition target"
        );
        <Health as pm_mcp::SurfaceHook>::call(&Health(true), tool, project, args)
    }
}
#[test]
fn app_level_extensions_verify_and_undo_without_a_composition() {
    let root = tempfile::tempdir().unwrap();
    let host = Arc::new(Mutex::new(
        Automation::new(root.path().into(), root.path().join("exports")).unwrap(),
    ));
    publish_fixture(&host, "builtin-application", root.path());
    let mut h = host.lock().unwrap();
    h.surfaces = Some(Arc::new(AppHealth));
    let result = h
        .completion_action("verify", "builtin-application", "run", &json!({}))
        .unwrap();
    assert_eq!(result["changed"], true);
    assert!(result["images"].as_array().unwrap().is_empty());
    assert!(result["errors"].as_array().unwrap().is_empty(), "{result}");
    assert_eq!(result["extensions"][0]["extensionId"], "fixture-panel");
    assert_eq!(result["extensions"][0]["panelId"], "contributed-panel");
    h.completion_action("undo", "builtin-application", "run", &result)
        .unwrap();
    assert!(!root.path().join("live/fixture-panel").exists());
}
#[test]
fn app_chat_without_edits_needs_no_composition_receipt() {
    let root = tempfile::tempdir().unwrap();
    let mut h = Automation::new(root.path().into(), root.path().join("exports")).unwrap();
    h.completion_action("begin", "builtin-application", "run", &json!({}))
        .unwrap();
    let result = h
        .completion_action("verify", "builtin-application", "run", &json!({}))
        .unwrap();
    assert_eq!(result["changed"], false);
    assert!(result["errors"].as_array().unwrap().is_empty());
}

/// Forces a post-promotion staging failure without consuming disk space.
struct RenewalFailure {
    inner: Extensions,
    failed: Arc<std::sync::atomic::AtomicBool>,
}
impl pm_mcp::extensions::ExtensionHost for RenewalFailure {
    fn roots(&self) -> Vec<std::path::PathBuf> {
        if self.failed.load(std::sync::atomic::Ordering::Relaxed) {
            vec![]
        } else {
            <Extensions as pm_mcp::extensions::ExtensionHost>::roots(&self.inner)
        }
    }
    fn history(&self) -> std::path::PathBuf {
        <Extensions as pm_mcp::extensions::ExtensionHost>::history(&self.inner)
    }
    fn builtins(&self) -> Option<std::path::PathBuf> {
        None
    }
    fn manifest(&self, raw: &serde_json::Value) -> Result<serde_json::Value, String> {
        Ok(raw.clone())
    }
    fn check(&self, _: &std::path::Path, _: &serde_json::Value) -> Result<(), String> {
        Ok(())
    }
    fn reload(&self, _: &[String]) {
        self.failed
            .store(true, std::sync::atomic::Ordering::Relaxed);
    }
    fn api_pack(&self) -> Vec<(String, String)> {
        vec![]
    }
}
#[test]
fn committed_extension_receipt_survives_next_stage_failure() {
    let root = tempfile::tempdir().unwrap();
    let host = Arc::new(Mutex::new(
        Automation::new(root.path().into(), root.path().join("exports")).unwrap(),
    ));
    let failed = Arc::new(std::sync::atomic::AtomicBool::new(false));
    host.lock().unwrap().extensions = Some(Arc::new(RenewalFailure {
        inner: Extensions(root.path().into()),
        failed: failed.clone(),
    }));
    let mut session = owned_session(&host, "builtin-application");
    let workspace = std::fs::read_dir(root.path().join("mcp-runs"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let stage = workspace.join("extensions/fixture-panel");
    std::fs::create_dir_all(&stage).unwrap();
    std::fs::write(stage.join("manifest.json"),json!({"id":"fixture-panel","name":"Fixture panel","version":"1.0.0","apiVersion":4,"entry":"index.tsx"}).to_string()).unwrap();
    std::fs::write(stage.join("index.tsx"), "export const fixture=true;").unwrap();
    assert!(session.dispatch("publish_extensions",&json!({"extensions":[{"id":"fixture-panel","action":"created","summary":"Synthetic panel"}]})).is_err());
    assert!(
        root.path().join("live/fixture-panel/index.tsx").exists(),
        "promotion committed before the staged renewal failed"
    );
    failed.store(false, std::sync::atomic::Ordering::Relaxed);
    session.finish(true).unwrap();
    let mut h = host.lock().unwrap();
    h.surfaces = Some(Arc::new(AppHealth));
    let result = h
        .completion_action("verify", "builtin-application", "run", &json!({}))
        .unwrap();
    assert_eq!(result["extensionChangeSets"].as_array().unwrap().len(), 1);
    h.completion_action("undo", "builtin-application", "run", &result)
        .unwrap();
    assert!(!root.path().join("live/fixture-panel").exists());
}

struct MediaReadiness {
    ready: std::sync::atomic::AtomicBool,
    checked: Mutex<Vec<String>>,
}
impl pm_mcp::media::MediaRegistry for MediaReadiness {
    fn state(&self, _: &str, asset: &str) -> pm_mcp::media::MediaState {
        self.checked.lock().unwrap().push(asset.into());
        pm_mcp::media::MediaState {
            available: self.ready.load(std::sync::atomic::Ordering::Relaxed),
            loading: !self.ready.load(std::sync::atomic::Ordering::Relaxed),
            ..Default::default()
        }
    }
}
#[test]
fn verification_rechecks_new_media_until_ready_without_rechecking_unchanged_assets() {
    let (_root, host, id) = fixture();
    let mut h = host.lock().unwrap();
    let add_asset = |id: &str| {
        pm_kernel::MediaCommand::AddAsset {
            asset_id: id.into(),
            asset: pm_model::json::parse(
                &json!({"id":id,"name":id,"kind":"video","dur":1,"persisted":true}).to_string(),
            )
            .unwrap(),
        }
        .into()
    };
    let revision = h.revision(&id).unwrap();
    h.apply_commands(
        &id,
        vec![add_asset("old")],
        "Existing media",
        pm_kernel::Origin::Mcp,
        revision,
        101,
    )
    .unwrap();
    h.completion_action("keep", &id, "run", &json!({})).unwrap();
    h.completion_action("begin", &id, "run", &json!({}))
        .unwrap();
    // Model an import through the ordinary kernel media admission path.
    let revision = h.revision(&id).unwrap();
    h.apply_commands(
        &id,
        vec![add_asset("new")],
        "Imported media",
        pm_kernel::Origin::Agent,
        revision,
        102,
    )
    .unwrap();
    let revision = h.revision(&id).unwrap();
    h.completion_action("apply", &id, "run", &json!({"revision":revision,"commands":[{"type":"add_layer","id":"agent","layerType":"solid"}]})).unwrap();
    h.engine = Some(Arc::new(Frames(Default::default())));
    let registry = Arc::new(MediaReadiness {
        ready: Default::default(),
        checked: Default::default(),
    });
    h.media_registry = Some(registry.clone());
    let pending = h
        .completion_action("verify", &id, "run", &json!({}))
        .unwrap();
    assert!(pending["errors"].as_array().unwrap().iter().any(|error| {
        error
            .as_str()
            .is_some_and(|text| text.contains("Imported asset new"))
    }));
    registry
        .ready
        .store(true, std::sync::atomic::Ordering::Relaxed);
    let ready = h
        .completion_action("verify", &id, "run", &json!({}))
        .unwrap();
    assert!(ready["errors"].as_array().unwrap().is_empty(), "{ready}");
    assert_eq!(*registry.checked.lock().unwrap(), vec!["new", "new"]);
}
