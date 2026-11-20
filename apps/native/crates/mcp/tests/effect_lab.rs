use pm_mcp::{Automation, EngineHook, McpSession, catalog, effect_lab, services::ToolOutput};
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};

#[test]
fn effect_lab_is_advertised() {
    let tools = catalog::served_tools();
    assert!(tools.iter().any(|tool| tool["name"] == "render_effect_lab"));
}

struct RendererStub;
impl EngineHook for RendererStub {
    fn capture(&self, _: &pm_mcp::CaptureRequest) -> pm_mcp::Result<Value> {
        panic!("effect lab must use its own engine adapter")
    }

    fn render_effect_lab(
        &self,
        request: &effect_lab::Request,
        context: &effect_lab::Context,
        _: &(dyn Fn() -> bool + Sync),
    ) -> pm_mcp::Result<ToolOutput> {
        assert!(context.project.is_none());
        assert_eq!(request.effect_id, "blur");
        assert_eq!(request.parameter_sets.len(), 3);
        Ok(ToolOutput::text(json!({"effectId":"blur","cells":[
            {"index":1,"parameters":{"amount":0}},
            {"index":2,"parameters":{"amount":6}},
            {"index":3,"parameters":{"amount":24}}
        ]})))
    }
}

fn session(path: &std::path::Path) -> (McpSession, Arc<Mutex<Automation>>) {
    let mut automation = Automation::new(path.to_owned(), path.join("exports")).unwrap();
    automation.engine = Some(Arc::new(RendererStub));
    let host = Arc::new(Mutex::new(automation));
    (McpSession::new(host.clone()), host)
}

fn arguments() -> Value {
    json!({"effectId":"blur","width":128,"height":128,"parameterSets":[{"amount":0},{"amount":6},{"amount":24}],
        "backdrop":{"type":"checker","colors":["#202020","#E0E0E0"],"tileSize":16}})
}

#[test]
fn editor_can_render_without_a_project_but_calls_without_editor_access_are_refused() {
    let tmp = tempfile::tempdir().unwrap();
    let (mut session, host) = session(tmp.path());
    let args = arguments();
    let denied = session.call("render_effect_lab", args.clone());
    assert_eq!(denied["isError"], true);
    assert!(
        denied["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("start_session")
    );

    host.lock().unwrap().agent_grants = Some(Arc::new(|token| match token {
        "planning" => Some(("synthetic-run".into(), "none".into())),
        "editing" => Some(("synthetic-run".into(), "editor".into())),
        _ => None,
    }));
    session.set_agent_run("planning");
    let start = json!({"context":"app","prompt":"Preview an effect"});
    let mut elevated = start.clone();
    elevated["access"] = json!("project");
    assert_eq!(session.call("start_session", elevated)["isError"], false);
    let denied = session.call("render_effect_lab", args.clone());
    assert_eq!(denied["isError"], true);
    assert!(
        denied["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("Editor access")
    );
    assert_eq!(
        session.call("finish_session", json!({"commit":false}))["isError"],
        false
    );

    session.set_agent_run("editing");
    assert_eq!(session.call("start_session", start)["isError"], false);
    let allowed = session.call("render_effect_lab", args);
    assert_eq!(allowed["isError"], false, "{allowed}");
    assert_eq!(
        allowed["structuredContent"]["cells"][1]["parameters"]["amount"],
        6
    );
    let backdrop = json!({"type":"composition","compositionId":"other","time":0});
    let mut composition = arguments();
    composition["backdrop"] = backdrop;
    let denied = session.call("render_effect_lab", composition);
    assert_eq!(denied["isError"], true);
    assert!(
        denied["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("Open the project")
    );
    assert_eq!(session.call("get_timeline", json!({}))["isError"], true);
}

#[test]
fn render_bounds_and_workspace_paths_are_enforced() {
    let mut args = arguments();
    args["parameterSets"] = json!(vec![json!({}); 24]);
    args["width"] = json!(1280);
    args["height"] = json!(1280);
    assert!(
        effect_lab::Request::parse(&args)
            .unwrap_err()
            .message
            .contains("8-megapixel")
    );

    let tmp = tempfile::tempdir().unwrap();
    let workspace = tmp.path().join("workspace");
    std::fs::create_dir(&workspace).unwrap();
    std::fs::write(workspace.join("safe.png"), b"image").unwrap();
    std::fs::write(tmp.path().join("private.png"), b"outside").unwrap();
    std::os::unix::fs::symlink(tmp.path(), workspace.join("outside")).unwrap();
    let context = effect_lab::Context {
        project: None,
        workspace: Some(workspace),
        agent_workspace: None,
        stage: None,
    };
    let read = |path| context.read_image(std::path::Path::new(path));
    assert_eq!(read("safe.png").unwrap(), b"image");
    assert!(read("../private.png").is_err());
    assert!(read("outside/private.png").is_err());
    assert!(context.read_image(&tmp.path().join("private.png")).is_err());
}
