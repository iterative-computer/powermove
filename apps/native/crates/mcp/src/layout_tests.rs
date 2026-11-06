//! Saved-layout conformance and behaviour on an isolated corpus copy.
use super::*;
use crate::{McpSession, catalog};
use pm_kernel::state::{Registration, StateOp, StateTransaction};
use std::io::Write;
use std::process::{Command as Process, Stdio};
use std::sync::Mutex;

fn oracle(workspace: &Value, panels: &Value) -> Value {
    let mut child = Process::new("node")
        .arg(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("scripts/layout-pairs.mjs"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(
            json!({"workspace":workspace,"panels":panels})
                .to_string()
                .as_bytes(),
        )
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}
fn result(value: Value) -> Value {
    assert_eq!(value["isError"], false, "{value}");
    serde_json::from_str(value["content"][0]["text"].as_str().unwrap()).unwrap()
}

#[test]
fn saved_panel_layout_matches_tag_on_corpus_copy_and_app_context() {
    let root = std::env::temp_dir().join(pm_kernel::new_id("pm-mcp-layout"));
    {
        let mut host = Automation::new(root.clone(), root.join("exports")).unwrap();
        let corpus = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(
            "../../../../engine/crates/pm-model/tests/fixtures/corpus/compositing-blend-modes.json",
        );
        let project: Value = serde_json::from_slice(&std::fs::read(corpus).unwrap()).unwrap();
        let id = project["id"].as_str().unwrap();
        let opened = host
            .kernel_mut()
            .shell(ShellCall::Open {
                id: id.into(),
                source: SessionSource::New(model(&project)),
                electron_profile: None,
            })
            .unwrap();
        host.adopt(opened).unwrap();
        let before = host.read_document(id).unwrap();
        let revision = host.revision(id).unwrap();
        let workspace = json!({"id":"corpus-layout","name":"Corpus layout","layout":{"docks":[
            {"id":"left","size":240,"panels":[{"id":"assets","collapsed":true,"size":180},{"id":"unknown"}]},
            {"id":"center","panels":[{"id":"viewer","flex":true,"title":"Custom title"}]},
            {"id":"bottom","size":null,"panels":[{"id":"timeline","size":"invalid"}]}
        ]}});
        let panels = json!([{"id":"assets","title":"Media"},{"id":"viewer","title":"Viewer"},{"id":"timeline","title":"Timeline"}]);
        let mut ops = vec![
            StateOp::Setting {
                key: "workspaces".into(),
                value: Some(model(&json!([workspace]))),
            },
            StateOp::Setting {
                key: "workspace".into(),
                value: Some(model(&json!("corpus-layout"))),
            },
        ];
        for (i, panel) in panels.as_array().unwrap().iter().enumerate() {
            ops.push(StateOp::Register {
                registry: "panels".into(),
                entry: Registration {
                    token: format!("panel-{i}"),
                    owner: pm_kernel::tx::Writer {
                        realm: 4,
                        generation: 1,
                    },
                    id: panel["id"].as_str().unwrap().into(),
                    definition: model(panel),
                },
            });
        }
        let state_revision = host.kernel_mut().state().revision;
        host.kernel_mut()
            .state_transaction(StateTransaction {
                expected_revision: Some(state_revision),
                ops,
            })
            .unwrap();
        assert_eq!(
            host.panel_layout(None).unwrap(),
            oracle(&workspace, &panels)
        );
        // A project-specific workspace wins over the global selection.
        let local = json!({"id":"local","name":"Local","layout":{"docks":[]}});
        host.kernel_mut()
            .handle(
                id,
                Request::new(
                    Caller::realm(5, 1, Origin::Mcp),
                    pm_kernel::WorkspaceCall::Mutate {
                        next: model(&local),
                        threw: None,
                    },
                ),
            )
            .unwrap();
        assert_eq!(
            host.panel_layout(Some(id)).unwrap(),
            oracle(&local, &panels)
        );
        let shared = Arc::new(Mutex::new(host));
        let mut session = McpSession::new(shared.clone());
        result(session.call(
            "start_session",
            json!({"prompt":"Corpus layout read","access":"editor"}),
        ));
        assert_eq!(
            result(session.call("get_panel_layout", json!({}))),
            oracle(&local, &panels)
        );
        result(session.call("finish_session", json!({"commit":true})));
        result(session.call(
            "start_session",
            json!({"prompt":"App layout read","context":"app"}),
        ));
        assert_eq!(
            result(session.call("get_panel_layout", json!({}))),
            // Kernel workspace mutations also update the selected named workspace.
            oracle(&local, &panels)
        );
        result(session.call("finish_session", json!({"commit":true})));
        let host = shared.lock().unwrap();
        assert_eq!(host.revision(id).unwrap(), revision);
        assert_eq!(host.read_document(id).unwrap(), before);
    }
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn get_panel_layout_schema_is_frozen_and_rejects_unknown_arguments() {
    assert!(catalog::IMPLEMENTED.contains(&"get_panel_layout"));
    assert_eq!(
        catalog::spec("get_panel_layout").unwrap()["inputSchema"],
        json!({"type":"object","properties":{},"additionalProperties":false})
    );
    assert!(catalog::validate("get_panel_layout", &json!({})).is_ok());
    assert!(catalog::validate("get_panel_layout", &json!({"projectId":"foreign"})).is_err());
}

#[test]
fn layout_empty_and_contribution_override_disposal_match_tag() {
    let root = std::env::temp_dir().join(pm_kernel::new_id("pm-mcp-layout-stacks"));
    {
        let mut host = Automation::new(root.clone(), root.join("exports")).unwrap();
        assert_eq!(
            host.panel_layout(None).unwrap(),
            oracle(&Value::Null, &json!([]))
        );
        for (token, title) in [("base", "Base"), ("override", "Override")] {
            let state_revision = host.kernel_mut().state().revision;
            host.kernel_mut()
                .state_transaction(StateTransaction {
                    expected_revision: Some(state_revision),
                    ops: vec![StateOp::Register {
                        registry: "panels".into(),
                        entry: Registration {
                            token: token.into(),
                            owner: pm_kernel::tx::Writer {
                                realm: 7,
                                generation: 1,
                            },
                            id: "viewer".into(),
                            definition: model(&json!({"id":"viewer","title":title})),
                        },
                    }],
                })
                .unwrap();
        }
        assert_eq!(
            host.panel_layout(None).unwrap(),
            oracle(&Value::Null, &json!([{"id":"viewer","title":"Override"}]))
        );
        let state_revision = host.kernel_mut().state().revision;
        host.kernel_mut()
            .state_transaction(StateTransaction {
                expected_revision: Some(state_revision),
                ops: vec![StateOp::Dispose {
                    token: "override".into(),
                }],
            })
            .unwrap();
        assert_eq!(
            host.panel_layout(None).unwrap(),
            oracle(&Value::Null, &json!([{"id":"viewer","title":"Base"}]))
        );
        assert!(host.panel_layout(Some("closed-project")).is_err());
    }
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn corpus_project_reads_selection_rollback_and_library_management() {
    let root = std::env::temp_dir().join(pm_kernel::new_id("pm-mcp-corpus"));
    {
        let mut host = Automation::new(root.clone(), root.join("exports")).unwrap();
        let source = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(
            "../../../../engine/crates/pm-model/tests/fixtures/corpus/compositing-blend-modes.json",
        );
        let bytes = std::fs::read(&source).unwrap();
        let project: Value = serde_json::from_slice(&bytes).unwrap();
        let id = project["id"].as_str().unwrap();
        let opened = host
            .kernel_mut()
            .shell(ShellCall::Open {
                id: id.into(),
                source: SessionSource::New(model(&project)),
                electron_profile: None,
            })
            .unwrap();
        host.adopt(opened).unwrap();
        host.save_library(id).unwrap();
        let before = host.read_document(id).unwrap();
        let shared = Arc::new(Mutex::new(host));
        let mut session = McpSession::new(shared.clone());
        result(session.call("start_session", json!({"prompt":"Corpus project workflow"})));
        for tool in ["get_project_state", "get_timeline"] {
            let actual = result(session.call(tool, json!({})));
            let mut child = Process::new("node")
                .arg(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("scripts/source-pairs.mjs"))
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap();
            child
                .stdin
                .take()
                .unwrap()
                .write_all(
                    json!({"tool":tool,"project":before,"arguments":{}})
                        .to_string()
                        .as_bytes(),
                )
                .unwrap();
            let output = child.wait_with_output().unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            assert_eq!(
                actual,
                serde_json::from_slice::<Value>(&output.stdout).unwrap(),
                "{tool}"
            );
        }
        let layer = before["layers"][0]["id"].as_str().unwrap();
        let selection = result(session.call("select_layers", json!({"layerIds":[layer]})));
        assert_eq!(selection["selection"]["layers"], json!([layer]));
        result(session.call("apply_commands",json!({"commands":[{"type":"add_layer","id":"corpus-added","layerType":"solid","name":"Corpus temporary"}]})));
        result(session.call("rollback_changes", json!({})));
        let restored = shared.lock().unwrap().read_document(id).unwrap();
        let mut comparable = restored.clone();
        comparable["revision"] = before["revision"].clone();
        assert_eq!(comparable, before);
        // Saving/duplicating/trashing are kernel-owned library commit boundaries.
        let duplicate = result(session.call(
            "manage_project",
            json!({"operation":"duplicate","projectId":id,"name":"Corpus duplicate"}),
        ));
        let duplicate_id = duplicate["projectId"].as_str().unwrap();
        result(session.call(
            "manage_project",
            json!({"operation":"trash","projectId":duplicate_id}),
        ));
        let listed = result(session.call("manage_project", json!({"operation":"list"})));
        assert!(
            listed["trash"]
                .as_array()
                .unwrap()
                .iter()
                .any(|p| p["id"] == duplicate_id)
        );
        result(session.call(
            "manage_project",
            json!({"operation":"restore","projectId":duplicate_id}),
        ));
        let listed = result(session.call("manage_project", json!({"operation":"list"})));
        assert!(
            listed["projects"]
                .as_array()
                .unwrap()
                .iter()
                .any(|p| p["id"] == duplicate_id)
        );
        let saved = result(session.call("save_project", json!({})));
        assert!(PathBuf::from(saved["path"].as_str().unwrap()).is_file());
        result(session.call("finish_session", json!({"commit":true})));
        assert_eq!(
            std::fs::read(source).unwrap(),
            bytes,
            "corpus source remains untouched"
        );
    }
    std::fs::remove_dir_all(root).unwrap();
}
