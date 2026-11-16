//! Synthetic, headless co-editing regressions.
use pm_mcp::{Automation, McpSession};
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};
fn ok(v: Value) -> Value {
    assert_eq!(v["isError"], false, "{v}");
    serde_json::from_str(v["content"][0]["text"].as_str().unwrap()).unwrap()
}
fn fixture() -> (
    std::path::PathBuf,
    Arc<Mutex<Automation>>,
    McpSession,
    String,
) {
    let root = std::env::temp_dir().join(pm_kernel::new_id("coedit"));
    let mut h = Automation::new(root.clone(), root.join("exports")).unwrap();
    let id = h
        .create_project(&json!({"name":"Synthetic coedit"}))
        .unwrap()["projectId"]
        .as_str()
        .unwrap()
        .to_owned();
    h.agent_grants = Some(Arc::new(|_| {
        Some(("synthetic-run".into(), "project".into()))
    }));
    let h = Arc::new(Mutex::new(h));
    let mut s = McpSession::new(h.clone());
    s.set_agent_run("synthetic-grant");
    ok(s.call("start_session", json!({"prompt":"Synthetic coedit"})));
    (root, h, s, id)
}
#[test]
fn agent_selection_and_creates_never_change_interface_selection() {
    let (root, h, mut s, id) = fixture();
    ok(s.call(
        "apply_commands",
        json!({"commands":[{"type":"add_layer","id":"a","layerType":"shape","name":"A"}]}),
    ));
    assert!(
        h.lock()
            .unwrap()
            .view(&id)
            .unwrap()
            .selection
            .layers()
            .is_empty()
    );
    let result = ok(s.call("select_layers", json!({"layerIds":["a"]})));
    assert_eq!(result["scope"], "agent");
    assert!(
        h.lock()
            .unwrap()
            .view(&id)
            .unwrap()
            .selection
            .layers()
            .is_empty()
    );
    ok(s.call("apply_commands",json!({"commands":[{"type":"set_property","target":"$selection","path":"opacity","value":40}]})));
    s.finish(true).unwrap();
    drop(s);
    drop(h);
    std::fs::remove_dir_all(root).unwrap();
}
#[test]
fn session_stays_bound_when_front_project_changes() {
    let (root, h, mut s, id) = fixture();
    let other = h
        .lock()
        .unwrap()
        .create_project(&json!({"name":"Other"}))
        .unwrap()["projectId"]
        .as_str()
        .unwrap()
        .to_owned();
    assert_ne!(id, other);
    ok(s.call(
        "apply_commands",
        json!({"commands":[{"type":"add_layer","id":"bound","layerType":"shape","name":"Bound"}]}),
    ));
    assert_eq!(
        h.lock().unwrap().read_document(&id).unwrap()["layers"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        h.lock().unwrap().read_document(&other).unwrap()["layers"]
            .as_array()
            .unwrap()
            .len(),
        0
    );
    s.finish(true).unwrap();
    drop(s);
    drop(h);
    std::fs::remove_dir_all(root).unwrap();
}
#[test]
fn presence_is_project_bound_and_never_stalls_a_writer() {
    let (root, h, mut s, id) = fixture();
    let rx = h.lock().unwrap().subscribe_presence();
    for i in 0..12 {
        ok(s.call("apply_commands",json!({"commands":[{"type":"add_layer","id":format!("a{i}"),"layerType":"shape","name":"Synthetic"}]})));
    }
    let events = rx.try_iter().collect::<Vec<_>>();
    assert_eq!(events.len(), 6);
    assert!(events.iter().all(|e| e.project == id));
    s.finish(true).unwrap();
    drop(s);
    drop(h);
    std::fs::remove_dir_all(root).unwrap();
}
#[test]
fn a_deleted_agent_target_returns_target_removed() {
    let (root, h, mut s, id) = fixture();
    ok(s.call(
        "apply_commands",
        json!({"commands":[{"type":"add_layer","id":"gone","layerType":"shape","name":"Gone"}]}),
    ));
    {
        let mut h = h.lock().unwrap();
        let revision = h.revision(&id).unwrap();
        h.apply_commands(
            &id,
            vec![
                pm_kernel::LayerCommand::Delete {
                    layer_ids: vec!["gone".into()],
                }
                .into(),
            ],
            "User delete",
            pm_kernel::Origin::Mcp,
            revision,
            7,
        )
        .unwrap();
    }
    let result = s.call(
        "apply_commands",
        json!({"commands":[{"type":"set_property","target":"gone","path":"opacity","value":30}]}),
    );
    assert_eq!(result["isError"], true);
    assert!(result.to_string().contains("target_removed"), "{result}");
    // Interleaved delete is preserved on disconnect.
    let _ = s.finish(true);
    drop(s);
    drop(h);
    std::fs::remove_dir_all(root).unwrap();
}

fn attached_fixture() -> (
    std::path::PathBuf,
    Arc<Mutex<Automation>>,
    pm_kernel::KernelActor,
    McpSession,
    String,
) {
    use pm_kernel::{
        KernelActor, KernelConfig,
        locks::OpenDecision,
        persistence::DiskFormat,
        session::{ShellCall, ShellReply},
    };
    let root = std::env::temp_dir().join(pm_kernel::new_id("coedit-attached"));
    std::fs::create_dir_all(&root).unwrap();
    let actor = KernelActor::spawn(KernelConfig::default());
    assert!(matches!(
        actor
            .handle()
            .shell(ShellCall::AttachLibrary {
                profile: root.clone(),
                format: DiskFormat::V1_2,
                electron_profile: None
            })
            .wait()
            .unwrap()
            .unwrap(),
        ShellReply::Library(OpenDecision::Writable)
    ));
    let app = pm_mcp::app_host::Host::new(actor.handle(), root.clone());
    let mut host = Automation::attached(app, root.join("exports"));
    let id = host
        .create_project(&json!({"name":"Synthetic held write"}))
        .unwrap()["projectId"]
        .as_str()
        .unwrap()
        .to_owned();
    host.agent_grants = Some(Arc::new(|_| {
        Some(("synthetic-run".into(), "project".into()))
    }));
    let host = Arc::new(Mutex::new(host));
    let mut session = McpSession::new(host.clone());
    session.set_agent_run("synthetic-grant");
    ok(session.call("start_session", json!({"prompt":"Synthetic hold"})));
    ok(session.call(
        "apply_commands",
        json!({"commands":[{"type":"add_layer","id":"L","layerType":"shape","name":"Synthetic"}]}),
    ));
    (root, host, actor, session, id)
}
#[test]
fn a_live_interface_lease_holds_an_agent_write_then_lands_after_release() {
    for cancel in [false, true] {
        use pm_kernel::{Caller, GestureCall, Reply, Request};
        let (root, host, actor, mut session, id) = attached_fixture();
        let kernel = actor.handle();
        let ack = kernel
            .call(
                &id,
                Request::new(
                    Caller::native(),
                    GestureCall::Begin {
                        label: "User drag".into(),
                    },
                ),
            )
            .wait()
            .unwrap()
            .unwrap();
        let Reply::Gesture(gesture) = ack.reply else {
            panic!("expected gesture");
        };
        if cancel {
            kernel
                .call(
                    &id,
                    Request::new(
                        Caller::native(),
                        GestureCall::Step {
                            gesture,
                            replace: false,
                            body: vec![pm_kernel::Command::SetStaticOpacity {
                                layer_id: "L".into(),
                                value: 17.,
                            }]
                            .into(),
                        },
                    ),
                )
                .wait()
                .unwrap()
                .unwrap();
        }
        let release = kernel.clone();
        let project = id.clone();
        let thread = std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(100));
            release
                .call(
                    &project,
                    Request::new(
                        Caller::native(),
                        if cancel {
                            GestureCall::Cancel { gesture }
                        } else {
                            GestureCall::Commit { gesture }
                        },
                    ),
                )
                .wait()
                .unwrap()
                .unwrap();
        });
        let result = session
        .dispatch(
            "apply_commands",
            &json!({"commands":[{"type":"set_property","target":"L","path":"opacity","value":40}]}),
        )
        .unwrap();
        thread.join().unwrap();
        assert_eq!(result["status"], "applied", "{result}");
        session.finish(true).unwrap();
        assert_eq!(
            host.lock().unwrap().read_document(&id).unwrap()["layers"][0]["p"]["opacity"]["v"],
            40
        );
        drop(session);
        drop(host);
        drop(actor);
        std::fs::remove_dir_all(root).unwrap();
    }
}
#[test]
fn typing_holds_after_four_seconds_and_user_wins_at_release() {
    use pm_kernel::{Call, Caller, Command, Request, WriteBody};
    let (root, host, actor, mut session, id) = attached_fixture();
    pm_mcp::coedit::touch_field(&id, "synthetic-opacity", true);
    let result = session
        .dispatch(
            "apply_commands",
            &json!({"commands":[{"type":"set_property","target":"L","path":"opacity","value":40}]}),
        )
        .unwrap();
    assert_eq!(result["status"], "held", "{result}");
    let started = std::time::Instant::now();
    let pending = session
        .dispatch("finish_session", &json!({"commit":true}))
        .unwrap();
    assert_eq!(pending["status"], "held", "{pending}");
    assert!(started.elapsed() < std::time::Duration::from_secs(6));
    actor
        .handle()
        .call(
            &id,
            Request::new(
                Caller::native(),
                Call::Transact {
                    label: Some("User value".into()),
                    body: WriteBody::Commands(vec![Command::SetStaticOpacity {
                        layer_id: "L".into(),
                        value: 33.,
                    }]),
                    expected_revision: None,
                },
            ),
        )
        .wait()
        .unwrap()
        .unwrap();
    pm_mcp::coedit::touch_field(&id, "synthetic-opacity", false);
    let result = session
        .dispatch("finish_session", &json!({"commit":true}))
        .unwrap();
    assert_eq!(result["coeditNote"]["status"], "dropped", "{result}");
    assert_eq!(
        host.lock().unwrap().read_document(&id).unwrap()["layers"][0]["p"]["opacity"]["v"],
        33
    );
    drop(session);
    drop(host);
    drop(actor);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn agent_evaluation_time_is_private_and_reported_consistently() {
    let (root, host, mut session, id) = fixture();
    let original = host.lock().unwrap().playhead(&id);
    let result = session
        .dispatch(
            "run_command",
            &json!({"command":"transport.seek","args":[1.25]}),
        )
        .unwrap();
    assert_eq!(result["scope"], "agent");
    assert_eq!(host.lock().unwrap().playhead(&id), original);
    let result = session
        .dispatch("get_project_state", &json!({"indexOnly":true}))
        .unwrap();
    assert_eq!(result["composition"]["playhead"], 1.25);
    session.finish(true).unwrap();
    drop(session);
    drop(host);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn deleting_an_attached_target_reports_target_removed() {
    use pm_kernel::{Call, Caller, LayerCommand, Request};
    let (root, host, actor, mut session, id) = attached_fixture();
    actor
        .handle()
        .call(
            &id,
            Request::new(
                Caller::native(),
                Call::Transact {
                    label: Some("User delete".into()),
                    expected_revision: None,
                    body: vec![
                        LayerCommand::Delete {
                            layer_ids: vec!["L".into()],
                        }
                        .into(),
                    ]
                    .into(),
                },
            ),
        )
        .wait()
        .unwrap()
        .unwrap();
    let error = session
        .dispatch(
            "apply_commands",
            &json!({"commands":[{"type":"set_property","target":"L","path":"opacity","value":40}]}),
        )
        .unwrap_err();
    assert_eq!(error.code, "target_removed");
    session.finish(true).unwrap();
    drop(session);
    drop(host);
    drop(actor);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn coedit_paused_worker_observes_panel_stop_without_a_resume() {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::Duration;
    let (root, host, actor, mut session, id) = attached_fixture();
    let app = host.lock().unwrap().coedit_host().unwrap();
    assert!(app.toggle_pause(&id));
    let running = Arc::new(AtomicBool::new(true));
    let worker_running = running.clone();
    let worker_app = app.clone();
    let project = id.clone();
    let (started, ready) = std::sync::mpsc::channel();
    let (send, result) = std::sync::mpsc::channel();
    let worker = std::thread::spawn(move || {
        let mut announced = false;
        let admitted = worker_app.wait_if_paused(&project, || {
            if !announced {
                started.send(()).unwrap();
                announced = true;
            }
            worker_running.load(Ordering::Acquire)
        });
        send.send(admitted).unwrap();
    });
    ready.recv_timeout(Duration::from_secs(1)).unwrap();
    assert!(result.try_recv().is_err());
    running.store(false, Ordering::Release);
    assert!(!result.recv_timeout(Duration::from_secs(1)).unwrap());
    worker.join().unwrap();
    app.resume_project(&id);
    assert!(app.wait_if_paused(&id, || true));
    session.finish(true).unwrap();
    drop(session);
    drop(host);
    drop(app);
    drop(actor);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn stopping_a_bound_run_drops_its_held_write_without_waiting_for_blur() {
    let (root, host, actor, mut session, id) = attached_fixture();
    pm_mcp::coedit::touch_field(&id, "synthetic-stop", true);
    let result = session
        .dispatch(
            "apply_commands",
            &json!({"commands":[{"type":"set_property","target":"L","path":"opacity","value":40}]}),
        )
        .unwrap();
    assert_eq!(result["status"], "held");
    let started = std::time::Instant::now();
    session.close().unwrap();
    assert!(started.elapsed() < std::time::Duration::from_secs(2));
    assert_ne!(
        host.lock().unwrap().read_document(&id).unwrap()["layers"][0]["p"]["opacity"]["v"],
        40
    );
    pm_mcp::coedit::touch_field(&id, "synthetic-stop", false);
    drop(session);
    drop(host);
    drop(actor);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn non_open_composition_commits_independently_of_user_commit_or_cancel() {
    use pm_kernel::{Call, Caller, Command, CompCommand, GestureCall, HistoryCall, Reply, Request};
    for cancel in [false, true] {
        let (root, host, actor, mut session, id) = attached_fixture();
        session.finish(true).unwrap();
        let kernel = actor.handle();
        kernel
            .call(
                &id,
                Request::new(
                    Caller::native(),
                    Call::Transact {
                        label: Some("User precomp".into()),
                        expected_revision: None,
                        body: vec![
                            CompCommand::Create {
                                comp_id: "pre".into(),
                                settings: Default::default(),
                            }
                            .into(),
                        ]
                        .into(),
                    },
                ),
            )
            .wait()
            .unwrap()
            .unwrap();
        ok(session.call("start_session", json!({"prompt":"Edit other composition"})));
        let before = kernel.published(&id).unwrap();
        let Reply::Gesture(gesture) = kernel
            .call(
                &id,
                Request::new(
                    Caller::native(),
                    GestureCall::Begin {
                        label: "User drag".into(),
                    },
                ),
            )
            .wait()
            .unwrap()
            .unwrap()
            .reply
        else {
            panic!()
        };
        kernel
            .call(
                &id,
                Request::new(
                    Caller::native(),
                    GestureCall::Step {
                        gesture,
                        replace: false,
                        body: vec![Command::SetStaticOpacity {
                            layer_id: "L".into(),
                            value: 17.,
                        }]
                        .into(),
                    },
                ),
            )
            .wait()
            .unwrap()
            .unwrap();
        let started = std::time::Instant::now();
        let result=session.dispatch("apply_commands",&json!({"compositionId":"pre","commands":[{"type":"add_layer","id":"remote","layerType":"shape"}]})).unwrap();
        assert_ne!(result["status"], "held", "{result}");
        assert!(started.elapsed() < std::time::Duration::from_secs(2));
        let during = kernel.published(&id).unwrap();
        assert!(during.entry.is_some());
        assert_eq!(during.snapshot.open_comp(), before.snapshot.open_comp());
        assert_eq!(during.selection, before.selection);
        assert_eq!(
            host.lock().unwrap().read_document(&id).unwrap()["comps"]["pre"]["layers"][0]["id"],
            "remote"
        );
        // Finishing may defer history grouping, but never waits for this drag.
        session.finish(true).unwrap();
        let app = host.lock().unwrap().coedit_host().unwrap();
        let (writer, _) = app.latest_turn(&id).unwrap();
        kernel
            .call(
                &id,
                Request::new(
                    Caller::native(),
                    if cancel {
                        GestureCall::Cancel { gesture }
                    } else {
                        GestureCall::Commit { gesture }
                    },
                ),
            )
            .wait()
            .unwrap()
            .unwrap();
        let project = host.lock().unwrap().read_document(&id).unwrap();
        assert_eq!(
            project["layers"][0]["p"]["opacity"]["v"],
            if cancel { 100 } else { 17 }
        );
        assert_eq!(project["comps"]["pre"]["layers"][0]["id"], "remote");
        let reply = kernel
            .call(
                &id,
                Request::new(
                    Caller::native(),
                    HistoryCall::UndoWriter {
                        writer,
                        keep_mine: false,
                        label: "Undo Agent".into(),
                    },
                ),
            )
            .wait()
            .unwrap()
            .unwrap()
            .reply;
        assert!(
            matches!(reply, Reply::Applied { changed: true, .. }),
            "{reply:?}"
        );
        let project = host.lock().unwrap().read_document(&id).unwrap();
        assert_eq!(
            project["layers"][0]["p"]["opacity"]["v"],
            if cancel { 100 } else { 17 }
        );
        assert_eq!(
            project["comps"]["pre"]["layers"].as_array().unwrap().len(),
            0
        );
        drop(session);
        drop(host);
        drop(app);
        drop(actor);
        std::fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn pause_keeps_the_turn_past_tool_ack_timeout_and_legacy_mcp_bypasses_it() {
    use std::time::Duration;
    let (root, host, actor, mut session, id) = attached_fixture();
    let app = host.lock().unwrap().coedit_host().unwrap();
    assert!(app.toggle_pause(&id));
    let mut legacy = McpSession::new(host.clone());
    ok(legacy.call("start_session", json!({"prompt":"Legacy client"})));
    let started = std::time::Instant::now();
    legacy.wait_for_pause().unwrap();
    assert!(started.elapsed() < Duration::from_millis(200));
    let gate = app.clone();
    let project = id.clone();
    let (send, result) = std::sync::mpsc::channel();
    let worker = std::thread::spawn(move || {
        send.send(gate.wait_if_paused(&project, || true)).unwrap();
    });
    // Pause is intentional; the four-second tool acknowledgement timeout
    // must not release it or end the provider turn.
    assert!(matches!(
        result.recv_timeout(Duration::from_millis(4200)),
        Err(std::sync::mpsc::RecvTimeoutError::Timeout)
    ));
    app.resume_project(&id);
    assert!(result.recv_timeout(Duration::from_secs(1)).unwrap());
    worker.join().unwrap();
    legacy.finish(true).unwrap();
    session.finish(true).unwrap();
    drop(legacy);
    drop(session);
    drop(host);
    drop(app);
    drop(actor);
    std::fs::remove_dir_all(root).unwrap();
}

fn ui_call(
    actor: &pm_kernel::KernelActor,
    id: &str,
    call: impl Into<pm_kernel::Call>,
) -> pm_kernel::Reply {
    let ack = actor
        .handle()
        .call(
            id,
            pm_kernel::Request::new(pm_kernel::Caller::native(), call),
        )
        .wait()
        .unwrap()
        .unwrap();
    assert!(
        !matches!(ack.reply, pm_kernel::Reply::Throw(_)),
        "{:?}",
        ack.reply
    );
    ack.reply
}
fn user_opacity(actor: &pm_kernel::KernelActor, id: &str, value: f64) {
    ui_call(
        actor,
        id,
        pm_kernel::Call::Transact {
            label: Some("User opacity".into()),
            expected_revision: None,
            body: vec![pm_kernel::Command::SetStaticOpacity {
                layer_id: "L".into(),
                value,
            }]
            .into(),
        },
    );
}

#[test]
fn ownership_survives_a_remote_write_a_cancelled_drag_and_a_composition_switch() {
    use pm_kernel::{Call, CompCommand, GestureCall, Reply};
    let (root, host, actor, mut session, id) = attached_fixture();
    session.finish(true).unwrap();
    ui_call(
        &actor,
        &id,
        Call::Transact {
            label: None,
            expected_revision: None,
            body: vec![
                CompCommand::Create {
                    comp_id: "pre".into(),
                    settings: Default::default(),
                }
                .into(),
            ]
            .into(),
        },
    );
    let document = actor.handle().published(&id).unwrap().snapshot.doc_id;
    let original = actor
        .handle()
        .published(&id)
        .unwrap()
        .snapshot
        .render_comp();
    ok(session.call("start_session", json!({"prompt":"Respect user ownership"})));
    user_opacity(&actor, &id, 33.);
    let Reply::Gesture(gesture) = ui_call(
        &actor,
        &id,
        GestureCall::Begin {
            label: "User drag".into(),
        },
    ) else {
        panic!()
    };
    ui_call(
        &actor,
        &id,
        GestureCall::Step {
            gesture,
            replace: false,
            body: vec![pm_kernel::Command::SetStaticOpacity {
                layer_id: "L".into(),
                value: 17.,
            }]
            .into(),
        },
    );
    let remote = session.dispatch("apply_commands", &json!({"compositionId":"pre","commands":[{"type":"add_layer","id":"remote","layerType":"shape"}]})).unwrap();
    assert_ne!(remote["status"], "held", "{remote}");
    ui_call(&actor, &id, GestureCall::Cancel { gesture });
    ui_call(
        &actor,
        &id,
        Call::OpenComposition {
            comp_id: "pre".into(),
            document,
        },
    );
    // Default targeting stays bound to the run's original composition.
    let result = session
        .dispatch(
            "apply_commands",
            &json!({"commands":[{"type":"set_property","target":"L","path":"opacity","value":40}]}),
        )
        .unwrap();
    assert_eq!(result["status"], "dropped", "{result}");
    assert_eq!(result["error"], "user_owned");
    assert_eq!(
        actor
            .handle()
            .published(&id)
            .unwrap()
            .snapshot
            .render_comp(),
        "pre"
    );
    let state = ok(session.call("get_project_state", json!({"compositionId":original})));
    let layer = state["layers"]
        .as_array()
        .unwrap()
        .iter()
        .find(|l| l["id"] == "L")
        .unwrap();
    let opacity = layer["properties"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["path"] == "opacity")
        .unwrap();
    assert_eq!(opacity["value"], 33);
    session.finish(true).unwrap();
    drop(session);
    drop(host);
    drop(actor);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn cancelled_provisional_edit_does_not_claim_ownership() {
    use pm_kernel::{GestureCall, Reply};
    let (root, host, actor, mut session, id) = attached_fixture();
    let Reply::Gesture(gesture) = ui_call(
        &actor,
        &id,
        GestureCall::Begin {
            label: "Cancelled drag".into(),
        },
    ) else {
        panic!()
    };
    ui_call(
        &actor,
        &id,
        GestureCall::Step {
            gesture,
            replace: false,
            body: vec![pm_kernel::Command::SetStaticOpacity {
                layer_id: "L".into(),
                value: 17.,
            }]
            .into(),
        },
    );
    ui_call(&actor, &id, GestureCall::Cancel { gesture });
    let result = session
        .dispatch(
            "apply_commands",
            &json!({"commands":[{"type":"set_property","target":"L","path":"opacity","value":40}]}),
        )
        .unwrap();
    assert_ne!(result["status"], "dropped", "{result}");
    assert_eq!(
        host.lock().unwrap().read_document(&id).unwrap()["layers"][0]["p"]["opacity"]["v"],
        40
    );
    session.finish(true).unwrap();
    drop(session);
    drop(host);
    drop(actor);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn key_presence_uses_composition_time_and_includes_auto_keyframes() {
    let (root, host, mut session, id) = fixture();
    ok(session.call("apply_commands", json!({"commands":[{"type":"add_layer","id":"offset","layerType":"shape","from":2,"duration":4}]})));
    let events = host.lock().unwrap().subscribe_presence();
    ok(session.call("apply_commands", json!({"commands":[{"type":"set_property","target":"offset","path":"opacity","mode":"keyframe","time":3,"value":40}]})));
    let first = events
        .try_iter()
        .find(|e| e.kind == pm_mcp::presence::Kind::Key)
        .unwrap();
    assert_eq!(first.times, vec![3.]);
    ok(session.call("apply_commands", json!({"commands":[{"type":"set_property","target":"offset","path":"opacity","time":3.5,"value":60}]})));
    let second = events
        .try_iter()
        .find(|e| e.kind == pm_mcp::presence::Kind::Key)
        .unwrap();
    assert_eq!(second.times, vec![3.5]);
    session.finish(true).unwrap();
    drop(session);
    drop(host);
    std::fs::remove_dir_all(root).unwrap();
    let _ = id;
}

#[test]
fn remote_layer_seen_in_state_or_selection_reports_target_removed() {
    use pm_kernel::{Call, CompCommand, LayerCommand};
    for select in [false, true] {
        let (root, host, actor, mut session, id) = attached_fixture();
        session.finish(true).unwrap();
        ui_call(
            &actor,
            &id,
            Call::Transact {
                label: None,
                expected_revision: None,
                body: vec![
                    CompCommand::Create {
                        comp_id: "pre".into(),
                        settings: Default::default(),
                    }
                    .into(),
                ]
                .into(),
            },
        );
        ok(session.call("start_session", json!({"prompt":"Setup remote layer"})));
        ok(session.call("apply_commands",json!({"compositionId":"pre","commands":[{"type":"add_layer","id":"existing","layerType":"shape"}]})));
        session.finish(true).unwrap();
        ok(session.call("start_session", json!({"prompt":"Observe remote layer"})));
        if select {
            ok(session.call(
                "select_layers",
                json!({"compositionId":"pre","layerIds":["existing"]}),
            ));
        } else {
            ok(session.call("get_project_state", json!({"compositionId":"pre"})));
        }
        let document = actor.handle().published(&id).unwrap().snapshot.doc_id;
        ui_call(
            &actor,
            &id,
            Call::OpenComposition {
                comp_id: "pre".into(),
                document,
            },
        );
        ui_call(
            &actor,
            &id,
            Call::Transact {
                label: Some("User delete".into()),
                expected_revision: None,
                body: vec![
                    LayerCommand::Delete {
                        layer_ids: vec!["existing".into()],
                    }
                    .into(),
                ]
                .into(),
            },
        );
        let result=session.call("apply_commands",json!({"compositionId":"pre","commands":[{"type":"set_property","target":"existing","path":"opacity","value":30}]}));
        assert_eq!(result["isError"], true, "{result}");
        assert!(result.to_string().contains("target_removed"), "{result}");
        session.finish(true).unwrap();
        drop(session);
        drop(host);
        drop(actor);
        std::fs::remove_dir_all(root).unwrap();
    }
}
