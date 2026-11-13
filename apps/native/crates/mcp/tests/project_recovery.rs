use pm_kernel::{
    KernelActor, KernelConfig,
    locks::OpenDecision,
    persistence::DiskFormat,
    session::{ShellCall, ShellReply},
};
use pm_mcp::{Automation, McpSession, app_host::Host};
use serde_json::json;
use std::{
    path::Path,
    sync::{Arc, Mutex},
};

fn host(root: &Path, attached: bool) -> (Arc<Mutex<Automation>>, Option<KernelActor>) {
    if !attached {
        return (
            Arc::new(Mutex::new(
                Automation::new(root.into(), root.join("exports")).unwrap(),
            )),
            None,
        );
    }
    let actor = KernelActor::spawn(KernelConfig {
        journal_root: Some(root.join("journals")),
        backup_root: Some(root.join("backups")),
        ..Default::default()
    });
    assert!(matches!(
        actor
            .handle()
            .shell(ShellCall::AttachLibrary {
                profile: root.into(),
                format: DiskFormat::V1_2,
                electron_profile: None,
            })
            .wait()
            .unwrap()
            .unwrap(),
        ShellReply::Library(OpenDecision::Writable)
    ));
    let host = Host::new(actor.handle(), root.into());
    (
        Arc::new(Mutex::new(Automation::attached(host, root.join("exports")))),
        Some(actor),
    )
}
fn create_edited(session: &mut McpSession) -> String {
    let id = session
        .dispatch(
            "create_project",
            &json!({"name":"Synthetic recovery fixture"}),
        )
        .unwrap()["projectId"]
        .as_str()
        .unwrap()
        .to_owned();
    session
        .dispatch(
            "start_session",
            &json!({"prompt":"Synthetic recovery check"}),
        )
        .unwrap();
    session.dispatch("apply_commands", &json!({"commands":[{"type":"add_layer","id":"synthetic","layerType":"solid","name":"Synthetic"}]})).unwrap();
    // Match the reported order: export before finish_session groups the undo.
    session.dispatch("save_project", &json!({})).unwrap();
    session
        .dispatch("finish_session", &json!({"commit":true}))
        .unwrap();
    id
}
fn layer_count(session: &mut McpSession) -> u64 {
    session
        .dispatch(
            "start_session",
            &json!({"prompt":"Inspect recovered state"}),
        )
        .unwrap();
    let state = session
        .dispatch("get_project_state", &json!({"indexOnly":true}))
        .unwrap();
    session
        .dispatch("finish_session", &json!({"commit":true}))
        .unwrap();
    state["layerCount"].as_u64().unwrap()
}
fn stop(host: Arc<Mutex<Automation>>, actor: Option<KernelActor>, clean: bool) {
    if clean {
        host.lock().unwrap().close_headless().unwrap();
    }
    drop(host);
    if let Some(actor) = actor {
        actor.shutdown();
    }
}

#[test]
fn clean_headless_save_and_close_reopen_without_recovery() {
    for attached in [false, true] {
        let root = std::env::temp_dir().join(pm_kernel::new_id("clean-mcp-reopen"));
        let (automation, actor) = host(&root, attached);
        let mut session = McpSession::new(automation.clone());
        let id = create_edited(&mut session);
        drop(session);
        stop(automation, actor, true);
        let (automation, actor) = host(&root, attached);
        let mut session = McpSession::new(automation.clone());
        let opened = session
            .dispatch("open_project", &json!({"projectId":id}))
            .unwrap();
        assert_eq!(opened["projectId"], id);
        assert_ne!(opened["recoveryRequired"], true, "{opened}");
        assert_eq!(layer_count(&mut session), 1);
        drop(session);
        stop(automation, actor, true);
        std::fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn mcp_recovery_can_be_accepted_or_declined_with_a_project_bound_ticket() {
    for attached in [false, true] {
        for decision in ["recover", "discard", "open_as_copy"] {
            let root = std::env::temp_dir().join(pm_kernel::new_id("mcp-recovery-choice"));
            let (automation, actor) = host(&root, attached);
            let mut session = McpSession::new(automation.clone());
            let id = create_edited(&mut session);
            drop(session);
            // Simulate interrupted host shutdown: durable edits, no clean close.
            stop(automation, actor, false);
            let (automation, actor) = host(&root, attached);
            let mut session = McpSession::new(automation.clone());
            let required = session
                .dispatch("open_project", &json!({"projectId":id}))
                .unwrap();
            assert_eq!(required["recoveryRequired"], true, "{required}");
            let ticket = &required["ticket"];
            let listed = session.dispatch("list_projects", &json!({})).unwrap();
            assert_eq!(listed["pendingRecoveries"][0]["ticket"], *ticket);
            for (project, choice) in [
                ("another-project", decision),
                (id.as_str(), "open_saved_version"),
            ] {
                assert!(session.dispatch("open_project", &json!({"projectId":project,"recovery":{"ticket":ticket,"decision":choice}})).is_err());
            }
            let args = json!({"projectId":id,"recovery":{"ticket":ticket,"decision":decision}});
            let opened = session.dispatch("open_project", &args).unwrap();
            assert_eq!(opened["projectId"], id);
            assert_ne!(opened["recoveryRequired"], true);
            assert!(
                session.dispatch("open_project", &args).is_err(),
                "ticket cannot be reused"
            );
            assert_eq!(layer_count(&mut session), u64::from(decision != "discard"));
            drop(session);
            stop(automation, actor, true);
            std::fs::remove_dir_all(root).unwrap();
        }
    }
}
