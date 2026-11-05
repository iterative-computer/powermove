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
