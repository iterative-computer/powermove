use pm_agents::{Access, Event, RunResult, history::Receipt, persistence::Store};
use pm_kernel::{KernelActor, KernelConfig, persistence::DiskFormat, session::ShellCall};
use pm_panel_agentpanel::{model::Panel, store::KernelStore};
use serde_json::json;
use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};
struct Temp(PathBuf);
impl Temp {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let root = std::env::temp_dir().join(format!(
            "pm-agent-actor-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        Self(root.canonicalize().unwrap())
    }
    fn actor(&self) -> KernelActor {
        let actor = KernelActor::spawn(KernelConfig::default());
        actor
            .handle()
            .shell(ShellCall::AttachLibrary {
                profile: self.0.clone(),
                format: DiskFormat::V1_2,
                electron_profile: None,
            })
            .wait()
            .unwrap()
            .unwrap();
        actor
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}
#[test]
fn real_actor_threads_drafts_and_exact_receipts_survive_shutdown_reopen() {
    let temp = Temp::new();
    let actor = temp.actor();
    let mut store = KernelStore::bind(actor.handle()).unwrap();
    let mut panel = Panel::load(&mut store, "p", "thread", 0).unwrap();
    panel
        .draft_mut("thread")
        .unwrap()
        .insert_text("Exact authored brief 🎬", 1, false);
    panel.submit("thread", "run", "request", 10).unwrap();
    panel.save(&mut store, 10).unwrap();
    panel
        .provider_event(
            &mut store,
            "thread",
            &Receipt {
                source: "run".into(),
                sequence: 0,
                event: Event::Completed {
                    run: "run".into(),
                    result: RunResult::success("**Done**".into(), Access::Editor),
                },
            },
            20,
        )
        .unwrap();
    let archive = panel.raw_archive().clone();
    drop(store);
    actor.shutdown();
    let actor = temp.actor();
    let mut store = KernelStore::bind(actor.handle()).unwrap();
    let reopened = Panel::load(&mut store, "p", "blank", 100).unwrap();
    assert_eq!(reopened.raw_archive(), &archive);
    assert_eq!(
        reopened.raw_archive()["threads"][0]["conversation"][0]["text"],
        "Exact authored brief 🎬"
    );
    actor.shutdown();
}
#[test]
fn stale_panel_writer_cannot_overwrite_foreign_thread_or_unknown_fields() {
    let temp = Temp::new();
    let actor = temp.actor();
    let mut first = KernelStore::bind(actor.handle()).unwrap();
    let mut stale = KernelStore::bind(actor.handle()).unwrap();
    assert!(first.get("agentThreads.p").unwrap().is_none());
    assert!(stale.get("agentThreads.p").unwrap().is_none());
    let value = json!({"opaque":{"keep":true}});
    first.set("agentThreads.p", &value).unwrap();
    assert!(stale.set("agentThreads.p", &json!({"lost":true})).is_err());
    assert_eq!(first.get("agentThreads.p").unwrap(), Some(value));
    actor.shutdown();
}
#[test]
fn unattached_memory_actor_and_unobserved_or_nonagent_writes_are_refused() {
    let actor = KernelActor::spawn(KernelConfig::default());
    assert!(KernelStore::bind(actor.handle()).is_err());
    actor.shutdown();
    let temp = Temp::new();
    let actor = temp.actor();
    let mut store = KernelStore::bind(actor.handle()).unwrap();
    assert!(store.set("agentThreads.p", &json!({})).is_err());
    assert!(store.get("project.p").is_err());
    assert!(store.set("project.p", &json!({})).is_err());
    actor.shutdown();
}

#[test]
fn migrated_profiles_append_messages_without_rewriting_settings_or_payloads() {
    use pm_agents::{
        ApprovalMode,
        approvals::Authority,
        claude,
        transport::{self, Machine, ProcessSpec},
    };
    use pm_panel_agentpanel::{
        model::{EffectKind, Update},
        runtime::ProviderRuns,
        v4::{Context, Window, dispatch_at},
    };
    use std::{
        fs::File,
        io::Write,
        time::{Duration, Instant},
    };

    for legacy_attachment_mib in [0, 96] {
        let temp = Temp::new();
        let history = (0..12_000)
            .map(|i| {
                json!({
                    "role":"user","text":format!("Synthetic message {i} {}", "x".repeat(160))
                })
            })
            .collect::<Vec<_>>();
        let archive = json!({"version":1,"activeId":"t","future":{"preserve":true},"threads":[{
            "id":"t","title":"Synthetic thread","updatedAt":0,"attachments":[],
            "composerDraft":"", "conversation":history
        }]});
        let mut legacy = json!({"settings":{
            "agentThreads.synthetic":archive,
            "ext.builtin-application-agent":{
                "agentThreads.synthetic":{"distinct":true},
                "future":"preserve"
            }
        }, "extensions":[]});
        if legacy_attachment_mib > 0 {
            let large = "QUJD".repeat(24 * 1024 * 1024 / 4);
            for i in 0..4 {
                legacy["settings"][format!("agentAttachment.synthetic-{i}")] =
                    json!({"dataBase64":large});
            }
        }
        let legacy = pm_model::json::parse(&legacy.to_string()).unwrap();
        let record = temp.0.join("kernel-state.json");
        // Match the former persistence barrier: canonical serialization and a
        // durable atomic replacement, before the provider could be called.
        let baseline = Instant::now();
        let bytes = pm_model::json::to_canonical_string(&legacy);
        let staged = temp.0.join("seed.tmp");
        let mut file = File::create(&staged).unwrap();
        file.write_all(bytes.as_bytes()).unwrap();
        file.sync_all().unwrap();
        drop(file);
        fs::rename(staged, &record).unwrap();
        File::open(&temp.0).unwrap().sync_all().unwrap();
        let before = baseline.elapsed();
        let legacy_len = bytes.len();
        drop((bytes, legacy));

        let settings_path = temp.0.join("store/ext.builtin-application-agent.json");
        let expected_current = if legacy_attachment_mib > 0 {
            // .23 already applied the snapshot. Its current store setting must
            // win, while the retained Agent data is still available to import.
            fs::create_dir_all(settings_path.parent().unwrap()).unwrap();
            fs::write(&settings_path, br#"{"future":"changed on .23"}"#).unwrap();
            fs::write(temp.0.join("kernel-state.applied"), b"").unwrap();
            "changed on .23"
        } else {
            "preserve"
        };
        let actor = temp.actor();
        let kernel = actor.handle();
        assert!(!record.exists(), "the applied legacy record was retired");
        let settings = fs::read(&settings_path).unwrap();
        assert!(settings.len() < 1024);
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&settings).unwrap(),
            json!({"future":expected_current})
        );
        assert_eq!(
            fs::metadata(temp.0.join("kernel-state.pre-agent-storage.json"))
                .unwrap()
                .len(),
            legacy_len as u64
        );
        assert!(
            !kernel
                .state()
                .settings
                .contains_key("agentThreads.synthetic")
        );
        let mut store = KernelStore::bind(kernel.clone()).unwrap();
        let mut panel = Panel::load(&mut store, "synthetic", "blank", 0).unwrap();
        assert!(panel.raw_archive()["future"]["preserve"] == true);
        let journal = temp.0.join("Agent Threads/Projects/synthetic.jsonl");
        let first_len = fs::metadata(&journal).unwrap().len();
        let payload = temp.0.join("Agent Attachments/Projects/synthetic-0.json");
        let modified = (legacy_attachment_mib > 0).then(|| {
            let metadata = fs::metadata(&payload).unwrap();
            assert!(metadata.len() > 24 * 1024 * 1024);
            metadata.modified().unwrap()
        });
        let send = json!({"sequence":1,"target":{"project":"synthetic","thread":"t","run":null},
            "action":{"type":"key","composerEpoch":0,"key":{"key":"Enter","value":"Move the headline"}}});
        let start = Instant::now();
        let effect = dispatch_at(
            &mut panel,
            1,
            &send,
            Context {
                run: "run",
                request: "request",
                new_thread: "new",
                now: 1,
                popover_open: true,
                catalogs: &|_| Default::default(),
            },
        )
        .unwrap()
        .unwrap();
        let view = Window::new(&panel, "t").unwrap();
        let count = view.count(&panel).unwrap();
        let bubble = view.snapshot(&panel, count - 1..count, 1).unwrap();
        let published = start.elapsed();
        assert_eq!(
            bubble["rows"]["items"][0]["value"]["text"],
            "Move the headline"
        );
        assert_eq!(bubble["draft"]["text"], "");
        assert!(effect.message_id().is_some());
        let receipt = temp.0.join("fake-provider-dispatched");
        let EffectKind::Send { text, .. } = &effect.kind else {
            panic!("expected a provider request");
        };
        let request = claude::user_message(text, &[]);
        let mut runs = ProviderRuns::default();
        runs.start_admitted(&mut panel, &effect, 1, || {
            Ok(transport::spawn(
                ProcessSpec {
                    executable: "/bin/sh".into(),
                    args: vec![
                        "-c".into(),
                        r#"IFS= read -r request && case "$request" in
                            *'Move the headline'*) printf dispatched > "$1" ;;
                            *) exit 1 ;;
                        esac"#
                            .into(),
                        "agent-send-fixture".into(),
                        receipt.to_str().unwrap().into(),
                    ],
                    cwd: temp.0.clone(),
                    env: Default::default(),
                },
                Machine::Claude(claude::Client::new(
                    "run".into(),
                    Authority::new(Access::Editor, ApprovalMode::Auto, false).unwrap(),
                    0,
                )),
                vec![request],
            ))
        })
        .unwrap();
        while !receipt.is_file() {
            assert!(
                start.elapsed() < Duration::from_secs(3),
                "fake transport did not receive the request"
            );
            std::thread::sleep(Duration::from_millis(1));
        }
        let dispatched = start.elapsed();
        assert_eq!(fs::metadata(&journal).unwrap().len(), first_len);
        let checkpoint = Instant::now();
        panel.save(&mut store, 1).unwrap();
        let saved = checkpoint.elapsed();
        let written = fs::metadata(&journal).unwrap().len() - first_len;
        assert!(
            written > 0 && written < 2048,
            "journal appended {written} bytes"
        );
        let mut last_message = "Move the headline".to_owned();
        if legacy_attachment_mib > 0 {
            let complete = || Update::Complete {
                text: String::new(),
                error: false,
                cancelled: false,
            };
            panel.receive("t", "run", 1, complete(), 2).unwrap();
            for i in 2..=100 {
                let run = format!("sample-run-{i}");
                let request = format!("sample-request-{i}");
                last_message = format!("Synthetic send {i}");
                let now = i * 3;
                panel
                    .draft_mut("t")
                    .unwrap()
                    .insert_text(&last_message, now, false);
                panel.submit("t", &run, &request, now).unwrap();
                panel.receive("t", &run, 1, complete(), now + 1).unwrap();
                panel.save(&mut store, now + 1).unwrap();
            }
            let journal_size = fs::metadata(&journal).unwrap().len();
            let growth = journal_size - first_len;
            assert!(growth < 128 * 1024, "100 sends appended {growth} bytes");
            println!(
                "100 sends on the migrated profile: journal {journal_size} bytes, up {growth} bytes from {first_len}"
            );
        }
        assert!(!record.exists());
        assert_eq!(fs::read(&settings_path).unwrap(), settings);
        if let Some(modified) = modified {
            assert_eq!(
                fs::metadata(&payload).unwrap().modified().unwrap(),
                modified
            );
        }
        let mut journal_file = fs::OpenOptions::new().append(true).open(journal).unwrap();
        journal_file.write_all(b"{\"Change\":").unwrap();
        journal_file.sync_all().unwrap();
        drop(journal_file);
        actor.shutdown();
        let actor = temp.actor();
        let mut store = KernelStore::bind(actor.handle()).unwrap();
        let archive = store.get("agentThreads.synthetic").unwrap().unwrap();
        assert_eq!(
            archive["threads"][0]["conversation"]
                .as_array()
                .unwrap()
                .last()
                .unwrap()["text"],
            last_message
        );
        use pm_kernel::profile::{ProfileCall, ProfileReply};
        let application = actor
            .handle()
            .profile(ProfileCall::AgentGet {
                application: true,
                key: "agentThreads.synthetic".into(),
            })
            .wait()
            .unwrap()
            .unwrap();
        assert!(
            matches!(application, ProfileReply::AgentValue(Some(value)) if value["distinct"] == true)
        );
        assert!(
            actor
                .handle()
                .state()
                .settings
                .contains_key("ext.builtin-application-agent")
        );
        actor.shutdown();
        println!(
            "legacy payload {legacy_attachment_mib} MiB: legacy settings barrier {before:?}, optimistic model/row {published:?}, fake transport dispatched {dispatched:?}, background checkpoint {saved:?}, appended {written} bytes"
        );
    }
}

#[test]
fn attachment_files_remain_while_any_archive_references_them() {
    use pm_kernel::profile::{ProfileCall, ProfileReply};
    let temp = Temp::new();
    let actor = temp.actor();
    let kernel = actor.handle();
    let put = |key: &str, value| {
        let reply = kernel
            .profile(ProfileCall::AgentSet {
                application: false,
                key: key.into(),
                expected: None,
                value,
            })
            .wait()
            .unwrap()
            .unwrap();
        assert!(matches!(reply, ProfileReply::Changed));
    };
    let attachment = json!({"id":"shared","payloadStored":true});
    let thread = json!({"id":"t","attachments":[attachment]});
    let path = temp.0.join("Agent Attachments/Projects/shared.json");
    put("agentAttachment.shared", json!({"dataBase64":"AA=="}));
    put(
        "agentThreads.a",
        json!({"threads":[thread],"archivedThreads":[]}),
    );
    put(
        "agentThreads.b",
        json!({"threads":[],"archivedThreads":[thread]}),
    );
    put("agentThreads.a", json!({"threads":[],"archivedThreads":[]}));
    assert!(path.is_file(), "archived history still owns the payload");
    put("agentThreads.b", json!({"threads":[],"archivedThreads":[]}));
    assert!(!path.exists(), "the last durable reference was removed");
    actor.shutdown();
}
