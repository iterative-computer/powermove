use pm_agents::{
    Access, ApprovalMode, Event, RunResult,
    approvals::Authority,
    claude, codex,
    history::Receipt,
    persistence::Store,
    transport::{self, Machine, ProcessSpec},
};
use pm_panel_agentpanel::{
    Error,
    model::{CHECKPOINT_MS, Effect, Panel},
    runtime::ProviderRuns,
};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    io,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
    thread,
    time::{Duration, Instant},
};
#[derive(Default)]
struct Memory {
    data: BTreeMap<String, Value>,
    fail: bool,
    writes: usize,
}
impl Store for Memory {
    fn get(&mut self, key: &str) -> io::Result<Option<Value>> {
        Ok(self.data.get(key).cloned())
    }
    fn set(&mut self, key: &str, value: &Value) -> io::Result<()> {
        if self.fail {
            return Err(io::Error::other("injected storage failure"));
        }
        self.data.insert(key.into(), value.clone());
        self.writes += 1;
        Ok(())
    }
}
struct Temp(PathBuf);
impl Temp {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "pm-panel-worker-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&path).unwrap();
        Self(path.canonicalize().unwrap())
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).unwrap();
    }
}
fn setup() -> (Memory, Panel, Effect) {
    let mut store = Memory::default();
    let mut panel = Panel::load(&mut store, "p", "t", 0).unwrap();
    panel
        .draft_mut("t")
        .unwrap()
        .insert_text("Original brief", 1, false);
    let effect = panel.submit("t", "run", "request", 10).unwrap();
    (store, panel, effect)
}
fn spec(temp: &Temp, mode: &str) -> ProcessSpec {
    ProcessSpec {
        executable: "python3".into(),
        args: vec![
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("fixtures/provider-worker.py")
                .to_str()
                .unwrap()
                .into(),
            mode.into(),
            temp.0.join("interrupt").to_str().unwrap().into(),
        ],
        cwd: temp.0.clone(),
        env: BTreeMap::from([("PATH".into(), std::env::var("PATH").unwrap())]),
    }
}
fn peer(temp: &Temp, mode: &str) -> transport::Handle {
    transport::spawn(
        spec(temp, mode),
        Machine::Claude(claude::Client::new(
            "run".into(),
            Authority::new(Access::Editor, ApprovalMode::Auto, false).unwrap(),
            0,
        )),
        vec![],
    )
}
fn drain(
    runs: &mut ProviderRuns,
    panel: &mut Panel,
    store: &mut impl Store,
    until: impl Fn(&Panel) -> bool,
) {
    let end = Instant::now() + Duration::from_secs(5);
    while Instant::now() < end {
        runs.poll(panel, store, 100, 256).unwrap();
        if until(panel) {
            return;
        }
        thread::sleep(Duration::from_millis(2));
    }
    panic!("provider publication timed out");
}
#[test]
fn no_process_launch_before_durable_submission_and_retry_launches_once() {
    let temp = Temp::new();
    let (mut store, mut panel, effect) = setup();
    let mut runs = ProviderRuns::default();
    store.fail = true;
    assert!(
        runs.start(&mut panel, &mut store, &effect, 10, || panic!(
            "must not launch"
        ))
        .is_err()
    );
    assert!(runs.is_empty());
    store.fail = false;
    runs.start(&mut panel, &mut store, &effect, 10, || {
        Ok(peer(&temp, "claude"))
    })
    .unwrap();
    assert_eq!(
        store.data["agentThreads.p"]["threads"][0]["lastRunStatus"],
        "running"
    );
    assert_eq!(
        runs.start(&mut panel, &mut store, &effect, 11, || panic!(
            "duplicate launch"
        )),
        Err(Error::Busy)
    );
}
#[test]
fn real_pipe_question_answer_background_stream_and_reopen_preserve_receipts() {
    let temp = Temp::new();
    let (mut store, mut panel, effect) = setup();
    let mut runs = ProviderRuns::default();
    runs.start(&mut panel, &mut store, &effect, 10, || {
        Ok(peer(&temp, "claude"))
    })
    .unwrap();
    panel.new_thread("other", 12).unwrap();
    drain(&mut runs, &mut panel, &mut store, |p| {
        p.session("t").unwrap().question("question").is_some()
    });
    panel
        .choose_answer("t", "question", "question-0", "Soft")
        .unwrap();
    let answer = panel.answer("t", "question", false).unwrap();
    runs.dispatch(&mut panel, &mut store, &answer, 100).unwrap();
    drain(&mut runs, &mut panel, &mut store, |p| {
        p.session("t").unwrap().run.is_none()
    });
    let end = Instant::now() + Duration::from_secs(5);
    while !runs.is_empty() && Instant::now() < end {
        runs.poll(&mut panel, &mut store, 120, 256).unwrap();
        thread::sleep(Duration::from_millis(2));
    }
    assert!(runs.is_empty());
    assert_eq!(panel.active_id(), "other");
    // S114: the host hears the send, the question and the finished turn.
    let notices = runs.take_notices();
    assert_eq!(
        notices.first(),
        Some(&pm_panel_agentpanel::runtime::Notice::Sent { thread: "t".into() })
    );
    assert!(notices.iter().any(|n| matches!(
        n,
        pm_panel_agentpanel::runtime::Notice::Question { thread, .. } if thread == "t"
    )));
    assert!(notices.iter().any(|n| matches!(
        n,
        pm_panel_agentpanel::runtime::Notice::Finished { thread, stopped: false, .. } if thread == "t"
    )));
    assert!(runs.take_notices().is_empty());
    let archived = panel.raw_archive().clone();
    let receipts = archived["threads"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["id"] == "t")
        .unwrap()["nativeRunEvents"]
        .as_array()
        .unwrap();
    assert!(receipts.iter().any(|r| r["event"]["kind"] == "session"));
    assert!(receipts.iter().any(|r| r["event"]["kind"] == "completed"));
    assert!(receipts.iter().any(|r| r["event"]["kind"] == "stopped"));
    let reopened = Panel::load(&mut store, "p", "blank", 200).unwrap();
    assert_eq!(reopened.raw_archive(), &archived);
    assert!(reopened.session("t").unwrap().run.is_none());
    assert!(
        reopened
            .session("t")
            .unwrap()
            .conversation
            .values()
            .any(|m| m["steps"]
                .as_array()
                .is_some_and(|s| s.iter().any(|v| v["text"] == "**Done**")))
    );
}
#[test]
fn failed_event_publication_keeps_exact_event_for_retry_before_later_delivery() {
    let temp = Temp::new();
    let (mut store, mut panel, effect) = setup();
    let mut runs = ProviderRuns::default();
    runs.start(&mut panel, &mut store, &effect, 10, || {
        Ok(peer(&temp, "claude"))
    })
    .unwrap();
    store.fail = true;
    let before = panel.raw_archive().clone();
    let end = Instant::now() + Duration::from_secs(5);
    loop {
        if runs.poll(&mut panel, &mut store, 20, 1).is_err() {
            break;
        }
        assert!(Instant::now() < end);
        thread::sleep(Duration::from_millis(2));
    }
    assert_eq!(panel.raw_archive(), &before);
    assert!(panel.session("t").unwrap().question("question").is_none());
    store.fail = false;
    assert_eq!(runs.poll(&mut panel, &mut store, 20, 1).unwrap(), 1);
    assert!(panel.session("t").unwrap().question("question").is_some());
    assert_eq!(
        panel.raw_archive()["threads"][0]["nativeRunEvents"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
}
#[test]
fn receipt_conflicts_fail_atomically_and_startup_failure_becomes_durable_terminal() {
    let (mut store, mut panel, _) = setup();
    panel.save(&mut store, 10).unwrap();
    let mut receipt = Receipt {
        source: "run".into(),
        sequence: 0,
        event: Event::Ready,
    };
    assert!(panel.provider_event(&mut store, "t", &receipt, 11).unwrap());
    assert!(!panel.provider_event(&mut store, "t", &receipt, 11).unwrap());
    receipt.event = Event::Warning("different".into());
    let before = panel.raw_archive().clone();
    assert!(panel.provider_event(&mut store, "t", &receipt, 12).is_err());
    assert_eq!(panel.raw_archive(), &before);
    receipt.sequence = 1;
    receipt.event = Event::Stopped("Could not start agent runtime".into());
    panel.provider_event(&mut store, "t", &receipt, 13).unwrap();
    assert!(panel.session("t").unwrap().run.is_none());
    assert_eq!(panel.raw_archive()["threads"][0]["lastRunStatus"], "failed");
}
#[test]
fn restart_keeps_interrupted_trace_and_never_restores_computer_authority() {
    let (mut store, mut panel, _) = setup();
    let receipt = Receipt {
        source: "run".into(),
        sequence: 0,
        event: Event::Trace {
            run: "run".into(),
            value: json!({"kind":"answer","text":"Partial original"}),
        },
    };
    panel.provider_event(&mut store, "t", &receipt, 12).unwrap();
    assert!(!panel.checkpoint(&mut store, 11 + CHECKPOINT_MS).unwrap());
    assert!(panel.checkpoint(&mut store, 12 + CHECKPOINT_MS).unwrap());
    let reopened = Panel::load(&mut store, "p", "blank", 200).unwrap();
    assert!(reopened.session("t").unwrap().run.is_none());
    assert_eq!(
        reopened
            .session("t")
            .unwrap()
            .trace
            .rows
            .values()
            .next()
            .unwrap()["text"],
        "Partial original"
    );
    assert_ne!(
        reopened.session("t").unwrap().settings.access,
        Access::Computer
    );
}
#[test]
fn a_long_stream_checkpoints_instead_of_saving_every_fragment() {
    let (mut store, mut panel, _) = setup();
    panel.save(&mut store, 10).unwrap();
    let before = store.writes;
    // 2,000 tokens over four seconds, then the reply completes.
    for sequence in 0..2000u64 {
        let receipt = Receipt {
            source: "run".into(),
            sequence,
            event: Event::Trace {
                run: "run".into(),
                value: json!({"kind":"answer","text":"x"}),
            },
        };
        let now = 10 + sequence * 2;
        assert!(
            panel
                .provider_event(&mut store, "t", &receipt, now)
                .unwrap()
        );
        panel.checkpoint(&mut store, now).unwrap();
    }
    assert!(
        store.writes - before <= 4,
        "{} writes",
        store.writes - before
    );
    let receipt = Receipt {
        source: "run".into(),
        sequence: 2000,
        event: Event::Completed {
            run: "run".into(),
            result: RunResult::success("Done".into(), Access::Editor),
        },
    };
    panel
        .provider_event(&mut store, "t", &receipt, 4100)
        .unwrap();
    assert!(!panel.unsaved());
    let receipts = panel.raw_archive()["threads"][0]["nativeRunEvents"]
        .as_array()
        .unwrap();
    assert_eq!(receipts.len(), 1, "fragments keep no receipt");
    let reopened = Panel::load(&mut store, "p", "blank", 5000).unwrap();
    assert_eq!(reopened.raw_archive(), panel.raw_archive());
}
#[test]
fn effects_the_host_cannot_carry_out_settle_instead_of_stranding_the_thread() {
    let (mut store, mut panel, effect) = setup();
    let mut runs = ProviderRuns::default();
    // A Stop for a run with no process: acknowledged as refused, Stop again usable.
    let stop = panel.stop("t").unwrap();
    let error = runs
        .dispatch(&mut panel, &mut store, &stop, 11)
        .unwrap_err();
    runs.refuse(&mut panel, &mut store, &stop, 11, &error)
        .unwrap();
    let session = panel.session("t").unwrap();
    assert!(!session.run.as_ref().unwrap().stopping);
    assert!(session.stop_error.is_some());
    // A start that never launched fails its run, in words, durably.
    let error = Error::Host("The agent's workspace could not be created.".into());
    runs.refuse(&mut panel, &mut store, &effect, 12, &error)
        .unwrap();
    assert!(panel.session("t").unwrap().run.is_none());
    let thread = &store.data["agentThreads.p"]["threads"][0];
    assert_eq!(thread["lastRunStatus"], "failed");
    assert!(
        thread
            .to_string()
            .contains("workspace could not be created")
    );
    assert!(!thread.to_string().contains("Host("));
}
#[test]
fn forged_run_identity_never_changes_conversation() {
    let (mut store, mut panel, _) = setup();
    let before = panel.raw_archive().clone();
    let receipt = Receipt {
        source: "run".into(),
        sequence: 0,
        event: Event::Completed {
            run: "other-run".into(),
            result: RunResult::success("forged".into(), Access::Editor),
        },
    };
    assert!(panel.provider_event(&mut store, "t", &receipt, 20).is_err());
    assert_eq!(panel.raw_archive(), &before);
    assert!(panel.session("t").unwrap().run.is_some());
}

#[test]
fn real_codex_stop_is_nonterminal_until_interrupt_acknowledgement() {
    let temp = Temp::new();
    let (mut store, mut panel, effect) = setup();
    let mut runs = ProviderRuns::default();
    runs.start(&mut panel, &mut store, &effect, 10, || {
        let handle = transport::spawn(
            spec(&temp, "codex"),
            Machine::Codex(codex::Client::default()),
            vec![],
        );
        let request = serde_json::from_value(
            json!({"id":"run","cwd":temp.0,"prompt":"Original brief","imagePaths":[]}),
        )
        .unwrap();
        handle
            .send(transport::Action::Run(Box::new(request)))
            .ok()
            .unwrap();
        Ok(handle)
    })
    .unwrap();
    drain(&mut runs, &mut panel, &mut store, |p| {
        !p.session("t").unwrap().trace.rows.is_empty()
    });
    let stop = panel.stop("t").unwrap();
    runs.dispatch(&mut panel, &mut store, &stop, 100).unwrap();
    let end = Instant::now() + Duration::from_secs(5);
    while !temp.0.join("interrupt").exists() {
        runs.poll(&mut panel, &mut store, 101, 256).unwrap();
        assert!(Instant::now() < end);
        thread::sleep(Duration::from_millis(2));
    }
    assert!(panel.session("t").unwrap().run.as_ref().unwrap().stopping);
    assert!(
        !panel.raw_archive()["threads"][0]["nativeRunEvents"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["event"]["kind"] == "completed")
    );
    std::fs::write(temp.0.join("interrupt.release"), b"release").unwrap();
    drain(&mut runs, &mut panel, &mut store, |p| {
        p.session("t").unwrap().run.is_none()
    });
    assert_eq!(
        panel.raw_archive()["threads"][0]["lastRunStatus"],
        "cancelled"
    );
}

#[test]
fn recovery_status_is_durable_idempotent_and_failed_recovery_keeps_original_archive() {
    let (mut store, mut panel, _) = setup();
    panel.save(&mut store, 10).unwrap();
    let before = store.data["agentThreads.p"].clone();
    store.fail = true;
    assert!(Panel::load(&mut store, "p", "blank", 200).is_err());
    assert_eq!(store.data["agentThreads.p"], before);
    store.fail = false;
    let reopened = Panel::load(&mut store, "p", "blank", 200).unwrap();
    assert_eq!(
        reopened.raw_archive()["threads"][0]["lastRunStatus"],
        "interrupted"
    );
    assert_eq!(
        reopened.raw_archive()["threads"][0]["nativeInterruptedRuns"],
        json!([{"run":"run","status":"running","observedAt":200}])
    );
    let archive = reopened.raw_archive().clone();
    assert_eq!(
        Panel::load(&mut store, "p", "blank", 300)
            .unwrap()
            .raw_archive(),
        &archive
    );
}
#[test]
fn late_cleanup_is_inspection_after_new_turn_and_archival_keeps_terminal_receipt() {
    let (mut store, mut panel, _) = setup();
    let completed = Receipt {
        source: "run".into(),
        sequence: 0,
        event: Event::Completed {
            run: "run".into(),
            result: RunResult::success("Done".into(), Access::Editor),
        },
    };
    panel
        .provider_event(&mut store, "t", &completed, 20)
        .unwrap();
    panel
        .draft_mut("t")
        .unwrap()
        .insert_text("Next brief", 21, false);
    panel.submit("t", "run2", "request2", 22).unwrap();
    let stopped = Receipt {
        source: "run".into(),
        sequence: 1,
        event: Event::Stopped("old cleanup".into()),
    };
    panel.provider_event(&mut store, "t", &stopped, 23).unwrap();
    assert_eq!(panel.session("t").unwrap().run.as_ref().unwrap().id, "run2");
    panel.archive_thread("t", "replacement", true, 24).unwrap();
    let cancelled = Receipt {
        source: "run2".into(),
        sequence: 0,
        event: Event::Completed {
            run: "run2".into(),
            result: RunResult::failure("cancelled", true),
        },
    };
    panel
        .provider_event(&mut store, "t", &cancelled, 25)
        .unwrap();
    let cleanup = Receipt {
        source: "run2".into(),
        sequence: 1,
        event: Event::Stopped("reaped".into()),
    };
    panel.provider_event(&mut store, "t", &cleanup, 26).unwrap();
    let archived = &panel.raw_archive()["archivedThreads"][0];
    assert_eq!(archived["id"], "t");
    assert_eq!(archived["nativeRunEvents"].as_array().unwrap().len(), 4);
    assert!(panel.session("t").is_err());
}

#[test]
fn failed_action_ack_publication_holds_stream_until_exact_retry() {
    let temp = Temp::new();
    let (mut store, mut panel, effect) = setup();
    let mut runs = ProviderRuns::default();
    runs.start(&mut panel, &mut store, &effect, 10, || {
        Ok(peer(&temp, "claude"))
    })
    .unwrap();
    drain(&mut runs, &mut panel, &mut store, |p| {
        p.session("t").unwrap().question("question").is_some()
    });
    panel
        .choose_answer("t", "question", "question-0", "Soft")
        .unwrap();
    let answer = panel.answer("t", "question", false).unwrap();
    runs.dispatch(&mut panel, &mut store, &answer, 100).unwrap();
    let before = panel.raw_archive().clone();
    store.fail = true;
    let end = Instant::now() + Duration::from_secs(5);
    loop {
        if runs.poll(&mut panel, &mut store, 101, 256).is_err() {
            break;
        }
        assert!(Instant::now() < end);
        thread::sleep(Duration::from_millis(2));
    }
    assert_eq!(panel.raw_archive(), &before);
    assert!(panel.session("t").unwrap().run.is_some());
    store.fail = false;
    drain(&mut runs, &mut panel, &mut store, |p| {
        p.session("t").unwrap().run.is_none()
    });
    assert_eq!(
        panel.raw_archive()["threads"][0]["lastRunStatus"],
        "completed"
    );
}

#[test]
fn real_provider_pipes_publish_through_single_kernel_writer_and_reopen() {
    use pm_kernel::{KernelActor, KernelConfig, persistence::DiskFormat, session::ShellCall};
    use pm_panel_agentpanel::store::KernelStore;
    let temp = Temp::new();
    let profile = temp.0.join("profile");
    std::fs::create_dir(&profile).unwrap();
    let open = || {
        let actor = KernelActor::spawn(KernelConfig::default());
        actor
            .handle()
            .shell(ShellCall::AttachLibrary {
                profile: profile.clone(),
                format: DiskFormat::V1_2,
                electron_profile: None,
            })
            .wait()
            .unwrap()
            .unwrap();
        actor
    };
    let actor = open();
    let mut store = KernelStore::bind(actor.handle()).unwrap();
    let mut panel = Panel::load(&mut store, "p", "t", 0).unwrap();
    panel
        .draft_mut("t")
        .unwrap()
        .insert_text("Original brief", 1, false);
    let effect = panel.submit("t", "run", "request", 10).unwrap();
    let mut runs = ProviderRuns::default();
    runs.start(&mut panel, &mut store, &effect, 10, || {
        Ok(peer(&temp, "claude"))
    })
    .unwrap();
    drain(&mut runs, &mut panel, &mut store, |p| {
        p.session("t").unwrap().question("question").is_some()
    });
    panel
        .choose_answer("t", "question", "question-0", "Soft")
        .unwrap();
    let answer = panel.answer("t", "question", false).unwrap();
    runs.dispatch(&mut panel, &mut store, &answer, 100).unwrap();
    drain(&mut runs, &mut panel, &mut store, |p| {
        p.session("t").unwrap().run.is_none()
    });
    let end = Instant::now() + Duration::from_secs(5);
    while !runs.is_empty() {
        assert!(Instant::now() < end);
        runs.poll(&mut panel, &mut store, 120, 256).unwrap();
        thread::sleep(Duration::from_millis(2));
    }
    let archive = panel.raw_archive().clone();
    drop(runs);
    drop(store);
    actor.shutdown();
    let actor = open();
    let mut store = KernelStore::bind(actor.handle()).unwrap();
    assert_eq!(
        Panel::load(&mut store, "p", "blank", 200)
            .unwrap()
            .raw_archive(),
        &archive
    );
    actor.shutdown();
}

#[test]
fn claude_steer_with_attachments_is_dispatchable() {
    let temp = Temp::new();
    let (mut store, mut panel, effect) = setup();
    let mut runs = ProviderRuns::default();
    runs.start(&mut panel, &mut store, &effect, 10, || {
        Ok(peer(&temp, "idle"))
    })
    .unwrap();
    assert!(runs.steerable("run"));
    panel
        .draft_mut("t")
        .unwrap()
        .insert_text("Make it blue", 11, false);
    let mut steer = panel.submit("t", "run", "direction", 12).unwrap();
    if let pm_panel_agentpanel::model::EffectKind::Send { attachments, .. } = &mut steer.kind {
        attachments.push(json!({"name":"reference.txt","text":"blue"}));
    }
    runs.dispatch(&mut panel, &mut store, &steer, 12).unwrap();
    assert!(
        panel
            .session("t")
            .unwrap()
            .conversation
            .values()
            .any(|m| m["steering"] == true)
    );
}

#[test]
fn compatible_steer_with_and_without_attachments_is_dispatchable() {
    use pm_agents::compatible::{self, ChatHost, Saved};
    use std::sync::{Arc, Mutex, atomic::AtomicBool, mpsc};
    #[derive(Debug)]
    struct Held(Mutex<mpsc::Receiver<()>>);
    impl ChatHost for Held {
        fn saved(&self) -> Option<Saved> {
            let _ = self.0.lock().unwrap().recv_timeout(Duration::from_secs(5));
            None
        }
        fn key(&self, _: &str) -> Option<String> {
            None
        }
        fn stream(
            &self,
            _: &str,
            _: Option<&str>,
            _: Vec<u8>,
            _: &AtomicBool,
            _: &mut dyn FnMut(&[u8]) -> Result<(), String>,
        ) -> Result<u16, String> {
            panic!("no network")
        }
    }
    for attached in [false, true] {
        let (mut store, mut panel, effect) = setup();
        let mut runs = ProviderRuns::default();
        let (release, held) = mpsc::channel();
        let handle = compatible::start(
            Arc::new(Held(Mutex::new(held))),
            compatible::Run {
                id: "run".into(),
                prompt: "Original brief".into(),
                model: None,
                effort: None,
                access: Access::Editor,
                instructions: String::new(),
                images: vec![],
                tools: ("unused".into(), vec![]),
                tool_env: Default::default(),
                allowed: vec![],
            },
        );
        runs.start(&mut panel, &mut store, &effect, 10, || Ok(handle))
            .unwrap();
        panel
            .draft_mut("t")
            .unwrap()
            .insert_text("Make it blue", 11, false);
        let mut steer = panel.submit("t", "run", "direction", 12).unwrap();
        if attached
            && let pm_panel_agentpanel::model::EffectKind::Send { attachments, .. } =
                &mut steer.kind
        {
            attachments.push(json!({"name":"reference.txt","text":"blue"}));
        }
        let result = runs.dispatch(&mut panel, &mut store, &steer, 12);
        release.send(()).unwrap();
        result.unwrap();
    }
}

#[test]
fn rejected_steer_restarts_with_original_and_directions() {
    let temp = Temp::new();
    let (mut store, mut panel, effect) = setup();
    let mut runs = ProviderRuns::default();
    let mismatched = transport::spawn(
        spec(&temp, "idle"),
        Machine::Claude(claude::Client::new(
            "other-run".into(),
            Authority::new(Access::Editor, ApprovalMode::Auto, false).unwrap(),
            0,
        )),
        vec![],
    );
    runs.start(&mut panel, &mut store, &effect, 10, || Ok(mismatched))
        .unwrap();
    panel
        .draft_mut("t")
        .unwrap()
        .insert_text("Make it blue", 11, false);
    let steer = panel.submit("t", "run", "direction", 12).unwrap();
    let prompt = runs.continued_request("run", "Make it blue").unwrap();
    assert!(prompt.contains("ORIGINAL REQUEST\nOriginal brief"));
    assert!(prompt.contains("NEW DIRECTION\nMake it blue"));
    let restarted = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let called = restarted.clone();
    let restart_spec = spec(&temp, "idle");
    runs.steer(
        &mut panel,
        &mut store,
        &steer,
        12,
        "Make it blue".into(),
        vec![],
        Box::new(move || {
            called.store(true, Ordering::SeqCst);
            Ok(transport::spawn(
                restart_spec,
                Machine::Claude(claude::Client::new(
                    "run".into(),
                    Authority::new(Access::Editor, ApprovalMode::Auto, false).unwrap(),
                    0,
                )),
                vec![claude::user_message(&prompt, &[])],
            ))
        }),
    )
    .unwrap();
    let end = Instant::now() + Duration::from_secs(3);
    while !restarted.load(Ordering::SeqCst) && Instant::now() < end {
        runs.poll(&mut panel, &mut store, 13, 256).unwrap();
        thread::sleep(Duration::from_millis(2));
    }
    assert!(restarted.load(Ordering::SeqCst));
    let next = runs.continued_request("run", "Keep it short").unwrap();
    assert!(next.contains("Original brief"));
    assert!(next.contains("Make it blue\n\nKeep it short"));
    assert!(panel.session("t").unwrap().run.is_some());
}
#[test]
fn failed_steer_shows_a_notice() {
    let temp = Temp::new();
    let (mut store, mut panel, effect) = setup();
    let mut runs = ProviderRuns::default();
    runs.start(&mut panel, &mut store, &effect, 10, || {
        Ok(peer(&temp, "idle"))
    })
    .unwrap();
    panel
        .draft_mut("t")
        .unwrap()
        .insert_text("Make it blue", 11, false);
    let steer = panel.submit("t", "run", "direction", 12).unwrap();
    runs.refuse(&mut panel, &mut store, &steer, 13, &Error::Busy)
        .unwrap();
    runs.poll(&mut panel, &mut store, 14, 256).unwrap();
    assert!(
        format!("{:?}", panel.session("t").unwrap().trace.rows).contains("could not be delivered")
    );
}

#[test]
fn exec_steer_restarts_and_preserves_attachment_metadata() {
    let temp = Temp::new();
    let (mut store, mut panel, mut effect) = setup();
    let context = json!({"kind":"layer","id":"original-target"});
    if let pm_panel_agentpanel::model::EffectKind::Send { attachments, .. } = &mut effect.kind {
        attachments
            .push(json!({"name":"original.txt","content":"original", "targetContext":context}));
    }
    let handle = transport::spawn(
        spec(&temp, "idle"),
        Machine::CodexExec {
            client: pm_agents::codex_exec::Client::new(
                "run".into(),
                Authority::new(Access::Project, ApprovalMode::Auto, false).unwrap(),
                0,
            ),
            output: temp.0.join("result"),
        },
        vec![],
    );
    assert!(handle.supports_steering());
    assert!(!handle.supports_live_steering());
    let mut runs = ProviderRuns::default();
    runs.start(&mut panel, &mut store, &effect, 10, || Ok(handle))
        .unwrap();
    panel
        .draft_mut("t")
        .unwrap()
        .insert_text("Make it blue", 11, false);
    let mut steer = panel.submit("t", "run", "direction", 12).unwrap();
    if let pm_panel_agentpanel::model::EffectKind::Send { attachments, .. } = &mut steer.kind {
        attachments
            .push(json!({"name":"new.txt","content":"blue","targetContext":{"id":"new-target"}}));
    }
    let continued = runs.continuation_effect(&steer).unwrap();
    if let pm_panel_agentpanel::model::EffectKind::Send { attachments, .. } = &continued.kind {
        assert_eq!(attachments.len(), 2);
        assert_eq!(attachments[0]["targetContext"], context);
        assert_eq!(attachments[1]["targetContext"]["id"], "new-target");
    } else {
        panic!("expected send");
    }
    let prompt = runs.continued_request("run", "Make it blue").unwrap();
    assert!(prompt.contains("ORIGINAL REQUEST\nOriginal brief"));
    assert!(prompt.contains("NEW DIRECTION\nMake it blue"));
    let restarted = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let called = restarted.clone();
    let restart_spec = spec(&temp, "idle");
    runs.steer(
        &mut panel,
        &mut store,
        &steer,
        12,
        "Make it blue".into(),
        vec![],
        Box::new(move || {
            called.store(true, Ordering::SeqCst);
            Ok(transport::spawn(
                restart_spec,
                Machine::Claude(claude::Client::new(
                    "run".into(),
                    Authority::new(Access::Editor, ApprovalMode::Auto, false).unwrap(),
                    0,
                )),
                vec![],
            ))
        }),
    )
    .unwrap();
    assert!(restarted.load(Ordering::SeqCst));
    assert!(runs.steerable("run"));
}

#[test]
fn claude_queued_steer_publishes_last_result_after_stdin_closes() {
    let temp = Temp::new();
    let (mut store, mut panel, effect) = setup();
    let mut runs = ProviderRuns::default();
    runs.start(&mut panel, &mut store, &effect, 10, || {
        Ok(peer(&temp, "claude-steer"))
    })
    .unwrap();
    panel
        .draft_mut("t")
        .unwrap()
        .insert_text("Make it blue", 11, false);
    let steer = panel.submit("t", "run", "direction", 12).unwrap();
    runs.steer(
        &mut panel,
        &mut store,
        &steer,
        12,
        "Make it blue".into(),
        vec![],
        Box::new(|| panic!("live stdin must accept the steer")),
    )
    .unwrap();
    drain(&mut runs, &mut panel, &mut store, |p| {
        p.session("t").unwrap().run.is_none()
    });
    let settled = runs.take_settled();
    assert_eq!(settled.len(), 1);
    assert_eq!(
        settled[0].1,
        RunResult::success("Blue answer".into(), Access::Editor)
    );
}

#[test]
fn steering_a_held_turn_discards_its_previous_completion() {
    let temp = Temp::new();
    let (mut store, mut panel, effect) = setup();
    let mut runs = ProviderRuns::default();
    runs.start(&mut panel, &mut store, &effect, 10, || {
        Ok(peer(&temp, "idle"))
    })
    .unwrap();
    assert!(runs.orchestrate("run"));
    runs.inject(
        "run",
        Event::Completed {
            run: "run".into(),
            result: RunResult::success("Old answer".into(), Access::Editor),
        },
    );
    runs.poll(&mut panel, &mut store, 11, 16).unwrap();
    assert!(runs.held("run"));
    panel
        .draft_mut("t")
        .unwrap()
        .insert_text("Make it blue", 12, false);
    let steer = panel.submit("t", "run", "direction", 13).unwrap();
    let replacement = peer(&temp, "idle");
    runs.steer(
        &mut panel,
        &mut store,
        &steer,
        13,
        "Make it blue".into(),
        vec![],
        Box::new(move || Ok(replacement)),
    )
    .unwrap();
    assert!(!runs.held("run"));
    assert!(runs.take_ended().is_empty());
    runs.release("run");
    runs.poll(&mut panel, &mut store, 14, 16).unwrap();
    assert!(panel.session("t").unwrap().run.is_some());
    assert!(runs.take_settled().is_empty());
}

#[test]
fn rejected_steer_with_failed_restart_reports_a_quiet_notice() {
    let temp = Temp::new();
    let (mut store, mut panel, effect) = setup();
    let mut runs = ProviderRuns::default();
    let mismatched = transport::spawn(
        spec(&temp, "idle"),
        Machine::Claude(claude::Client::new(
            "other-run".into(),
            Authority::new(Access::Editor, ApprovalMode::Auto, false).unwrap(),
            0,
        )),
        vec![],
    );
    runs.start(&mut panel, &mut store, &effect, 10, || Ok(mismatched))
        .unwrap();
    panel
        .draft_mut("t")
        .unwrap()
        .insert_text("Make it blue", 11, false);
    let steer = panel.submit("t", "run", "direction", 12).unwrap();
    runs.steer(
        &mut panel,
        &mut store,
        &steer,
        12,
        "Make it blue".into(),
        vec![],
        Box::new(|| Err(Error::Host("synthetic restart refused".into()))),
    )
    .unwrap();
    drain(&mut runs, &mut panel, &mut store, |p| {
        format!("{:?}", p.session("t").unwrap().trace.rows).contains("could not be delivered")
    });
    let rows = format!("{:?}", panel.session("t").unwrap().trace.rows);
    assert!(rows.contains("synthetic restart refused"));
    assert!(rows.contains("Please send it again"));
    assert!(panel.session("t").unwrap().run.is_some());
}
