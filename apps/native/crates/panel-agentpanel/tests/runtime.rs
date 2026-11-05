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
    model::{Effect, Panel},
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
