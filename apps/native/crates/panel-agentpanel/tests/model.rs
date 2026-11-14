//! Tagged behaviors: threads.ts:139–239; spatial.ts:828–865,971–1061,
//! 1702–1738,1762–1975; QuestionCard.svelte:17–45; ToolActivity.svelte:22–62.
use pm_agents::{Access, Provider, persistence::Store};
use pm_panel_agentpanel::{
    Error,
    activity::{Rows, Trace},
    composer::Attachment,
    input::Catalog,
    model::*,
    view::Follow,
};
use serde_json::{Value, json};
use std::{collections::BTreeMap, io, sync::Arc};
#[derive(Default)]
struct Memory {
    values: BTreeMap<String, Value>,
    writes: Vec<String>,
    fail: bool,
}
impl Store for Memory {
    fn get(&mut self, key: &str) -> io::Result<Option<Value>> {
        Ok(self.values.get(key).cloned())
    }
    fn set(&mut self, key: &str, value: &Value) -> io::Result<()> {
        if self.fail {
            return Err(io::Error::other("synthetic disk failure"));
        }
        self.writes.push(key.into());
        self.values.insert(key.into(), value.clone());
        Ok(())
    }
}
fn panel() -> Panel {
    Panel::load(&mut Memory::default(), "p", "t", 0).unwrap()
}
fn start(p: &mut Panel) {
    p.draft_mut("t")
        .unwrap()
        .insert_text("make a title", 1, false);
    p.submit("t", "run1", "request1", 10).unwrap();
}
fn event(p: &mut Panel, seq: u64, value: Value) {
    p.receive("t", "run1", seq, Update::Trace(value), 10 + seq)
        .unwrap();
}
fn question(secret: bool, transport: &str) -> Value {
    json!({"kind":"question","itemId":"q","requestId":"request1","transport":transport,"blocking":true,"questions":[{"id":"first","question":"Choose","multiSelect":true,"allowOther":true,"secret":secret,"options":[{"label":"A"},{"label":"B"}]}]})
}
#[test]
fn background_thread_preserves_drafts_run_and_streaming_without_switching_focus() {
    let mut p = panel();
    start(&mut p);
    p.new_thread("other", 11).unwrap();
    p.draft_mut("other")
        .unwrap()
        .insert_text("unsent", 12, false);
    event(
        &mut p,
        1,
        json!({"kind":"answer","text":"background result"}),
    );
    assert_eq!(p.active_id(), "other");
    assert_eq!(p.background_runs(), 1);
    p.switch("t").unwrap();
    assert_eq!(
        p.session("t").unwrap().trace.rows.values().next().unwrap()["text"],
        "background result"
    );
    p.switch("other").unwrap();
    assert_eq!(p.session("other").unwrap().draft.request_text(), "unsent");
}
#[test]
fn new_thread_reuses_a_blank_thread() {
    let mut p = panel();
    assert_eq!(p.new_thread("unused", 1).unwrap(), "t");
    p.draft_mut("t").unwrap().insert_text("x", 2, false);
    assert_eq!(p.new_thread("new", 3).unwrap(), "new");
    assert_eq!(p.new_thread("unused2", 4).unwrap(), "new");
    assert_eq!(p.summaries().len(), 2);
}
#[test]
fn project_models_do_not_leak_text_or_drafts() {
    let mut store = Memory::default();
    let mut a = Panel::load(&mut store, "a", "t", 0).unwrap();
    let mut b = Panel::load(&mut store, "b", "t", 0).unwrap();
    a.draft_mut("t").unwrap().insert_text("only a", 1, false);
    a.save(&mut store, 2).unwrap();
    b.save(&mut store, 2).unwrap();
    assert_eq!(
        store.values["agentThreads.a"]["threads"][0]["composerDraft"],
        "only a"
    );
    assert_eq!(
        store.values["agentThreads.b"]["threads"][0]["composerDraft"],
        ""
    );
}
#[test]
fn persistence_roundtrip_preserves_unknown_fields_and_detached_payloads() {
    let mut store = Memory::default();
    store.values.insert("agentThreads.p".into(),json!({"version":1,"activeId":"t","futureArchive":{"v":7},"threads":[{"id":"t","title":"Kept","updatedAt":0,"conversation":[],"composerDraft":"L😀R","attachments":[{"id":"a","name":"A","type":"application/octet-stream","promptOffset":1,"payloadStored":true,"future":9}],"futureThread":{"resume":"opaque"}}]}));
    store.values.insert(
        "agentAttachment.a".into(),
        json!({"dataBase64":"AP8A","content":"\u{0000}�\u{0000}"}),
    );
    let mut p = Panel::load(&mut store, "p", "blank", 1).unwrap();
    assert_eq!(
        p.session("t").unwrap().draft.request_text(),
        "L[Attachment: A]😀R"
    );
    p.save(&mut store, 2).unwrap();
    let archive = &store.values["agentThreads.p"];
    assert_eq!(archive["futureArchive"]["v"], 7);
    assert_eq!(archive["threads"][0]["futureThread"]["resume"], "opaque");
    assert_eq!(store.values["agentAttachment.a"]["dataBase64"], "AP8A");
    let restored = Panel::load(&mut store, "p", "blank", 3).unwrap();
    assert_eq!(
        restored.session("t").unwrap().draft.legacy().1[0]["future"],
        9
    );
}
#[test]
fn failed_save_remains_visible_and_retry_keeps_work() {
    let mut p = panel();
    p.draft_mut("t").unwrap().insert_text("keep me", 1, false);
    let mut store = Memory {
        fail: true,
        ..Default::default()
    };
    assert!(p.save(&mut store, 2).is_err());
    assert!(p.save_error.is_some());
    assert_eq!(p.session("t").unwrap().draft.request_text(), "keep me");
    store.fail = false;
    p.save(&mut store, 3).unwrap();
    assert!(p.save_error.is_none());
}
#[test]
fn unknown_or_malformed_archive_is_never_silently_reset() {
    for archive in [
        json!({"version":2,"opaque":"keep"}),
        json!({"version":1,"activeId":"x","threads":[{"id":"x","conversation":[],"composerDraft":7}]}),
    ] {
        let mut store = Memory::default();
        store
            .values
            .insert("agentThreads.p".into(), archive.clone());
        assert!(Panel::load(&mut store, "p", "t", 0).is_err());
        assert!(store.writes.is_empty());
        assert_eq!(store.values["agentThreads.p"], archive);
    }
}
#[test]
fn confirmed_remove_archives_complete_record_and_never_deletes_payload() {
    let mut p = panel();
    p.draft_mut("t").unwrap().insert_text("archive", 1, false);
    p.add_attachments(
        "t",
        vec![Attachment::from_record(json!({"id":"a","name":"A","dataBase64":"AA=="})).unwrap()],
        2,
    )
    .unwrap();
    assert!(matches!(
        p.archive_thread("t", "replacement", false, 3)
            .unwrap()
            .unwrap()
            .kind,
        EffectKind::ConfirmArchive
    ));
    p.archive_thread("t", "replacement", true, 3).unwrap();
    let mut store = Memory::default();
    p.save(&mut store, 4).unwrap();
    assert_eq!(p.active_id(), "replacement");
    assert_eq!(
        store.values["agentThreads.p"]["archivedThreads"][0]["composerDraft"],
        "archive"
    );
    assert_eq!(store.values["agentAttachment.a"]["dataBase64"], "AA==");
}
#[test]
fn streaming_rows_stay_stable_and_duplicate_events_do_not_append() {
    let mut p = panel();
    start(&mut p);
    event(&mut p, 1, json!({"kind":"answer","text":"Hello"}));
    let id = p.session("t").unwrap().trace.rows.at(0).unwrap().id.clone();
    event(&mut p, 2, json!({"kind":"answer","text":" world"}));
    assert!(
        !p.receive(
            "t",
            "run1",
            2,
            Update::Trace(json!({"kind":"answer","text":"duplicate"})),
            14
        )
        .unwrap()
    );
    assert_eq!(p.session("t").unwrap().trace.rows.len(), 1);
    assert_eq!(
        p.session("t").unwrap().trace.rows.get(&id).unwrap().value["text"],
        "Hello world"
    );
    assert!(
        p.receive("t", "run1", 4, Update::Progress("gap".into()), 15)
            .is_err()
    );
}
#[test]
fn steering_archives_reasoning_and_completions_update_original_tool() {
    let mut p = panel();
    start(&mut p);
    event(&mut p, 1, json!({"kind":"thought","text":"why"}));
    event(
        &mut p,
        2,
        json!({"kind":"tool-start","itemId":"tool","toolName":"read","label":"Reading"}),
    );
    p.draft_mut("t").unwrap().insert_text("steer", 20, false);
    let effect = p.submit("t", "run1", "request1", 21).unwrap();
    assert!(matches!(effect.kind, EffectKind::Send { steer: true, .. }));
    assert_eq!(p.session("t").unwrap().run.as_ref().unwrap().started_at, 10);
    event(
        &mut p,
        3,
        json!({"kind":"tool-end","itemId":"tool","output":"complete","isError":false}),
    );
    let trace = p.session("t").unwrap().conversation.at(1).unwrap();
    assert_eq!(trace.value["steps"][0]["label"], "why");
    assert_eq!(trace.value["steps"][1]["status"], "done");
    assert_eq!(
        p.session("t").unwrap().conversation.at(2).unwrap().value["text"],
        "steer"
    );
}
#[test]
fn completion_does_not_duplicate_streamed_reply_and_followup_keeps_prior_turns() {
    let mut p = panel();
    start(&mut p);
    event(&mut p, 1, json!({"kind":"answer","text":"answer"}));
    p.receive(
        "t",
        "run1",
        2,
        Update::Complete {
            text: "answer".into(),
            error: false,
            cancelled: false,
        },
        30,
    )
    .unwrap();
    assert_eq!(p.session("t").unwrap().conversation.len(), 2);
    p.draft_mut("t").unwrap().insert_text("next", 31, false);
    p.submit("t", "run2", "r2", 32).unwrap();
    assert_eq!(
        p.session("t").unwrap().conversation.at(1).unwrap().value["steps"][0]["text"],
        "answer"
    );
    assert_eq!(
        p.receive("t", "run1", 3, Update::Progress("late".into()), 40),
        Err(Error::Stale)
    );
}
#[test]
fn stop_targets_one_thread_and_failure_can_retry() {
    let mut p = panel();
    start(&mut p);
    let effect = p.stop("t").unwrap();
    assert_eq!(effect.target.run.as_deref(), Some("run1"));
    assert!(p.stop("t").is_err());
    p.acknowledge(effect.id, false).unwrap();
    assert!(p.session("t").unwrap().stop_error.is_some());
    let retry = p.stop("t").unwrap();
    p.acknowledge(retry.id, true).unwrap();
    assert!(p.session("t").unwrap().run.is_some());
    p.receive(
        "t",
        "run1",
        1,
        Update::Complete {
            text: String::new(),
            error: false,
            cancelled: true,
        },
        30,
    )
    .unwrap();
    assert!(p.session("t").unwrap().run.is_none());
    assert_eq!(p.raw_archive()["threads"][0]["lastRunStatus"], "cancelled");
}
#[test]
fn approval_answers_are_explicit_masked_and_failed_delivery_closes_question() {
    let mut p = panel();
    start(&mut p);
    event(&mut p, 1, question(true, "reply"));
    assert!(p.answer("t", "q", false).is_err());
    p.choose_answer("t", "q", "first", "A").unwrap();
    p.choose_answer("t", "q", "first", "B").unwrap();
    p.type_answer("t", "q", "first", "synthetic-private-answer".into())
        .unwrap();
    let effect = p.answer("t", "q", false).unwrap();
    assert!(!format!("{effect:?}").contains("synthetic-private-answer"));
    assert_eq!(
        p.session("t").unwrap().question("q").unwrap()["answers"]["first"],
        "••••••"
    );
    let mut store = Memory::default();
    p.save(&mut store, 20).unwrap();
    assert!(
        !serde_json::to_string(&store.values)
            .unwrap()
            .contains("synthetic-private-answer")
    );
    assert!(p.answer("t", "q", false).is_err());
    p.acknowledge(effect.id, false).unwrap();
    assert_eq!(
        p.session("t").unwrap().question("q").unwrap()["status"],
        "closed"
    );
    assert!(
        p.session("t")
            .unwrap()
            .question("q")
            .unwrap()
            .get("answers")
            .is_none()
    );
}
#[test]
fn question_skip_only_reply_transport_and_message_answer_preserves_composer() {
    let mut p = panel();
    start(&mut p);
    event(&mut p, 1, question(false, "message"));
    p.draft_mut("t").unwrap().insert_text("unsent", 20, false);
    assert!(p.answer("t", "q", true).is_err());
    p.type_answer("t", "q", "first", "typed".into()).unwrap();
    let effect = p.answer("t", "q", false).unwrap();
    assert!(matches!(effect.kind, EffectKind::Send { steer: true, .. }));
    assert_eq!(p.session("t").unwrap().draft.request_text(), "unsent");
}
#[test]
fn computer_access_needs_matching_consent_and_late_grants_cannot_override_choice() {
    let mut p = panel();
    let effect = p.set_access("t", "full").unwrap().unwrap();
    assert_eq!(p.session("t").unwrap().settings.access, Access::Editor);
    p.set_access("t", "editor").unwrap();
    assert!(p.acknowledge(effect.id, true).is_err());
    let effect = p.set_access("t", "full").unwrap().unwrap();
    p.acknowledge(effect.id, false).unwrap();
    assert_eq!(p.session("t").unwrap().settings.access, Access::Editor);
    let effect = p.set_access("t", "full").unwrap().unwrap();
    p.acknowledge(effect.id, true).unwrap();
    assert_eq!(p.session("t").unwrap().settings.access, Access::Computer);
    assert!(matches!(
        p.set_access("t", "supervised").unwrap().unwrap().kind,
        EffectKind::RevokeComputerConsent
    ));
}
#[test]
fn restored_computer_intent_is_not_a_live_authority() {
    let mut p = panel();
    let effect = p.set_access("t", "full").unwrap().unwrap();
    p.acknowledge(effect.id, true).unwrap();
    let mut store = Memory::default();
    p.save(&mut store, 1).unwrap();
    let mut restored = Panel::load(&mut store, "p", "blank", 2).unwrap();
    assert_eq!(
        restored.session("t").unwrap().settings.access,
        Access::Project
    );
    restored
        .set_catalog(
            "t",
            Catalog {
                models: vec![("m".into(), "Model".into())],
                efforts: vec!["high".into()],
                model_efforts: vec![],
                ..Default::default()
            },
        )
        .unwrap();
    restored.set_model("t", "m".into(), "high".into()).unwrap();
    restored.save(&mut store, 3).unwrap();
    assert_eq!(
        store.values["agentThreads.p"]["threads"][0]["access"],
        "computer"
    );
}

#[test]
fn provider_cannot_switch_during_run_and_model_uses_catalog_ids() {
    let mut p = panel();
    let catalog = Catalog {
        providers: vec![("claude".into(), "Claude".into())],
        models: vec![("sonnet".into(), "Sonnet".into())],
        efforts: vec!["high".into()],
        model_efforts: vec![],
        claude: true,
        resolves: vec![],
    };
    p.set_provider("t", Provider::Claude, "sonnet".into(), catalog.clone())
        .unwrap();
    p.set_model("t", "claude-custom".into(), "high".into())
        .unwrap();
    assert!(
        p.set_model("t", "unavailable".into(), "high".into())
            .is_err()
    );
    start(&mut p);
    assert!(
        p.set_provider("t", Provider::Claude, "sonnet".into(), catalog)
            .is_err()
    );
}
#[test]
fn plan_apply_blocks_switches_but_failure_preserves_preview_and_draft() {
    let mut p = panel();
    start(&mut p);
    p.receive(
        "t",
        "run1",
        1,
        Update::Plan(json!({"kind":"scene","commands":[]})),
        11,
    )
    .unwrap();
    let effect = p.apply_plan("t").unwrap();
    assert!(p.new_thread("new", 12).is_err());
    assert!(p.draft_mut("t").is_err());
    p.acknowledge(effect.id, false).unwrap();
    assert_eq!(p.session("t").unwrap().phase, Phase::Preview);
    assert!(p.session("t").unwrap().plan.is_some());
    p.dismiss_plan("t").unwrap();
    assert!(p.session("t").unwrap().plan.is_none());
}
#[test]
fn artifacts_import_exactly_once_and_failed_import_is_retryable() {
    let mut p = panel();
    start(&mut p);
    p.receive(
        "t",
        "run1",
        1,
        Update::Result(json!({"artifacts":[{"path":"synthetic/result.png","name":"result.png"}]})),
        11,
    )
    .unwrap();
    let e = p.artifact("t", "synthetic/result.png", true).unwrap();
    assert!(p.artifact("t", "synthetic/result.png", true).is_err());
    p.acknowledge(e.id, false).unwrap();
    let e = p.artifact("t", "synthetic/result.png", true).unwrap();
    p.acknowledge(e.id, true).unwrap();
    assert!(p.artifact("t", "synthetic/result.png", true).is_err());
    assert!(matches!(
        p.artifact("t", "synthetic/result.png", false).unwrap().kind,
        EffectKind::RevealArtifact(_)
    ));
}
#[test]
fn six_attachment_limit_does_not_reject_existing_larger_archives() {
    let mut p = panel();
    let files = (0..6)
        .map(|i| {
            Attachment::from_record(json!({"id":format!("a{i}"),"name":"A","dataBase64":""}))
                .unwrap()
        })
        .collect();
    p.add_attachments("t", files, 1).unwrap();
    assert!(
        p.add_attachments(
            "t",
            vec![
                Attachment::from_record(json!({"id":"extra","name":"E","dataBase64":""})).unwrap()
            ],
            2
        )
        .is_err()
    );
    let missing =
        Attachment::from_record(json!({"id":"missing","name":"M","payloadStored":true})).unwrap();
    let mut p = panel();
    p.add_attachments("t", vec![missing], 1).unwrap();
    assert!(matches!(
        p.submit("t", "r", "q", 2),
        Err(Error::MissingPayload(_))
    ));
    assert_eq!(p.session("t").unwrap().draft.attachments().count(), 1);
}
#[test]
fn retry_uses_stable_message_id_and_leaves_transcript_intact() {
    let mut p = panel();
    start(&mut p);
    p.receive(
        "t",
        "run1",
        1,
        Update::Complete {
            text: "failed".into(),
            error: true,
            cancelled: false,
        },
        20,
    )
    .unwrap();
    let e = p.retry("t", Some("message-1"), false).unwrap();
    assert!(matches!(e.kind,EffectKind::Retry{ref message,..} if message=="message-0"));
    assert_eq!(p.session("t").unwrap().conversation.len(), 2);
    assert!(p.retry("t", Some("message-1"), true).is_err());
}
#[test]
fn row_windows_scale_to_sixty_thousand_and_noop_keeps_arc_identity() {
    let mut rows = Rows::default();
    for i in 0..60_001 {
        rows.upsert(format!("r{i}"), json!({"text":i})).unwrap();
    }
    let old = rows.get("r30000").unwrap().value.clone();
    assert!(!rows.upsert("r30000".into(), json!({"text":30000})).unwrap());
    assert!(Arc::ptr_eq(&old, &rows.get("r30000").unwrap().value));
    assert_eq!(rows.page(59_990..60_010).len(), 11);
    assert_eq!(rows.page(usize::MAX..usize::MAX).len(), 0);
    assert_eq!(rows.get("r60000").unwrap().value["text"], 60000);
}
#[test]
fn full_activity_is_retained_while_thoughts_settle_and_tools_update_by_id() {
    let mut trace = Trace::default();
    trace
        .receive(&json!({"kind":"thought","text":"x".repeat(10_000)}), 1)
        .unwrap();
    trace
        .receive(
            &json!({"kind":"tool-start","itemId":"tool","label":"read","toolName":"read"}),
            2,
        )
        .unwrap();
    assert_eq!(trace.rows.at(0).unwrap().value["live"], false);
    trace
        .receive(
            &json!({"kind":"tool-end","itemId":"tool","output":"y".repeat(10_000)}),
            3,
        )
        .unwrap();
    assert_eq!(
        trace.rows.get("tool").unwrap().value["output"]
            .as_str()
            .unwrap()
            .len(),
        10_000
    );
    assert_eq!(
        trace.rows.at(0).unwrap().value["label"]
            .as_str()
            .unwrap()
            .len(),
        10_000
    );
}
#[test]
fn subagent_result_updates_in_place_and_stop_is_fenced() {
    let mut p = panel();
    start(&mut p);
    event(
        &mut p,
        1,
        json!({"kind":"task","task":{"taskId":"child","requestId":"request1","title":"Review","status":"waiting","progress":"wait"}}),
    );
    let e = p.stop_task("t", "child").unwrap();
    assert!(matches!(e.kind,EffectKind::StopTask{ref task,..} if task=="child"));
    assert!(p.stop_task("t", "child").is_err());
    p.acknowledge(e.id, false).unwrap();
    event(
        &mut p,
        2,
        json!({"kind":"task","task":{"taskId":"child","requestId":"request1","title":"Review","status":"completed","summary":"{\"summary\":\"Done\",\"notes\":[\"Note\"]}"}}),
    );
    assert_eq!(p.session("t").unwrap().trace.rows.len(), 1);
    assert_eq!(
        p.session("t")
            .unwrap()
            .trace
            .rows
            .get("child")
            .unwrap()
            .value["output"],
        "Done\n\nNote"
    );
    assert!(p.stop_task("t", "child").is_err());
}
#[test]
fn progress_is_bounded_and_does_not_reset_run_timer() {
    let mut p = panel();
    start(&mut p);
    for i in 1..=25 {
        p.receive(
            "t",
            "run1",
            i,
            Update::Progress(format!("step {i}")),
            i + 10,
        )
        .unwrap();
    }
    assert_eq!(p.session("t").unwrap().progress.len(), 20);
    assert_eq!(p.session("t").unwrap().elapsed_ms(100), Some(90));
}
#[test]
fn follow_state_is_independent_of_streamed_updates() {
    let mut p = panel();
    start(&mut p);
    p.follow = Follow {
        has_text_selection: true,
        ..Default::default()
    };
    event(&mut p, 1, json!({"kind":"answer","text":"text"}));
    assert!(!p.follow.should_follow(true));
}

#[test]
fn archive_of_running_thread_waits_for_settlement_then_keeps_final_output() {
    let mut p = panel();
    start(&mut p);
    let effect = p
        .archive_thread("t", "replacement", true, 12)
        .unwrap()
        .unwrap();
    assert!(matches!(effect.kind, EffectKind::Stop));
    assert!(p.session("t").unwrap().archiving);
    event(
        &mut p,
        1,
        json!({"kind":"answer","text":"last acknowledged output"}),
    );
    p.receive(
        "t",
        "run1",
        2,
        Update::Complete {
            text: String::new(),
            error: false,
            cancelled: true,
        },
        20,
    )
    .unwrap();
    assert_eq!(p.active_id(), "replacement");
    assert_eq!(
        p.raw_archive()["archivedThreads"][0]["conversation"][1]["steps"][0]["text"],
        "last acknowledged output"
    );
}
#[test]
fn restored_generated_title_clock_and_opaque_preferences_are_not_rewritten() {
    let mut store = Memory::default();
    store.values.insert("agentThreads.p".into(),json!({"version":1,"activeId":"t","threads":[{"id":"t","title":"Generated title","updatedAt":9,"conversation":[{"role":"user","text":"the original prompt"}],"composerDraft":"","attachments":[],"access":"computer","scope":"future:opaque","future":true}]}));
    let mut p = Panel::load(&mut store, "p", "blank", 10).unwrap();
    p.save(&mut store, 99).unwrap();
    let thread = &store.values["agentThreads.p"]["threads"][0];
    assert_eq!(thread["title"], "Generated title");
    assert_eq!(thread["updatedAt"], 9);
    assert_eq!(thread["access"], "computer");
    assert_eq!(thread["scope"], "future:opaque");
}
#[test]
fn held_question_in_restored_transcript_is_closed_without_dropping_record() {
    let mut store = Memory::default();
    store.values.insert("agentThreads.p".into(),json!({"version":1,"activeId":"t","threads":[{"id":"t","title":"Title","updatedAt":0,"composerDraft":"","attachments":[],"conversation":[{"role":"trace","steps":[{"kind":"question","id":"q","status":"open","transport":"reply","requestId":"old","questions":[{"id":"x","question":"Continue?"}]}]}]}]}));
    let p = Panel::load(&mut store, "p", "blank", 1).unwrap();
    assert_eq!(
        p.session("t").unwrap().question("q").unwrap()["status"],
        "closed"
    );
    assert_eq!(
        p.session("t").unwrap().question("q").unwrap()["requestId"],
        "old"
    );
}
#[test]
fn empty_recognized_archive_opens_a_blank_without_losing_unknown_fields() {
    let mut store = Memory::default();
    store.values.insert(
        "agentThreads.p".into(),
        json!({"version":1,"activeId":"old","threads":[],"future":"kept"}),
    );
    let p = Panel::load(&mut store, "p", "blank", 1).unwrap();
    assert_eq!(p.active_id(), "blank");
    assert_eq!(p.raw_archive()["future"], "kept");
}
#[test]
fn effect_tokens_cannot_collide_between_project_models() {
    let mut a = panel();
    let mut b = panel();
    let a = a.set_access("t", "full").unwrap().unwrap();
    let old = a.id;
    let b_effect = b.set_access("t", "full").unwrap().unwrap();
    assert_ne!(old, b_effect.id);
    assert_eq!(b.acknowledge(old, true), Err(Error::Stale));
}
#[test]
fn activity_groups_keep_chronology_and_mixed_failures_semantic() {
    use pm_panel_agentpanel::activity::{GroupIndex, GroupKind, WorkStatus};
    let mut rows = Rows::default();
    for (id, value) in [
        ("a", json!({"kind":"text","text":"before"})),
        ("b", json!({"kind":"tool","status":"error"})),
        ("c", json!({"kind":"tool","status":"done"})),
        (
            "d",
            json!({"kind":"thought","label":"reasoning","live":true}),
        ),
        ("e", json!({"kind":"tool","status":"running","task":{}})),
    ] {
        rows.upsert(id.into(), value).unwrap();
    }
    let groups = GroupIndex::build(&rows);
    assert_eq!(groups.len(), 4);
    let work = &groups.page(1..2)[0];
    assert_eq!(work.kind, GroupKind::Tools);
    assert_eq!(work.rows, 1..3);
    assert_eq!(work.status, WorkStatus::Partial);
    assert_eq!(work.success_count, 1);
    assert_eq!(groups.newest_active.as_deref(), Some("e"));
}

#[test]
fn nested_archived_trace_is_windowed_by_step_even_with_replayed_ids() {
    use pm_panel_agentpanel::activity::TranscriptIndex;
    let mut messages = Rows::default();
    messages.upsert("message-0".into(),json!({"role":"trace","steps":(0..60_001).map(|i|json!({"id":"replayed","kind":"tool","output":i})).collect::<Vec<_>>()})).unwrap();
    let index = TranscriptIndex::build(&messages);
    assert_eq!(index.len(), 60_001);
    let page = index.page(59_990..60_010);
    assert_eq!(page.len(), 11);
    assert_ne!(page[0].id, page[1].id);
    assert_eq!(
        index.resolve(&page[10], &messages).unwrap()["output"],
        60_000
    );
    assert_eq!(index.get(&page[0].id), Some(&page[0]));
}
#[test]
fn generated_title_is_guarded_and_not_overwritten_by_later_save() {
    let mut p = panel();
    start(&mut p);
    assert!(
        !p.generated_title("t", "other request", "Wrong", 12)
            .unwrap()
    );
    assert!(
        p.generated_title("t", "make a title", " ## A\n useful title “ ", 13)
            .unwrap()
    );
    let mut store = Memory::default();
    p.save(&mut store, 14).unwrap();
    assert_eq!(
        store.values["agentThreads.p"]["threads"][0]["title"],
        "A useful title"
    );
    assert_eq!(
        store.values["agentThreads.p"]["threads"][0]["updatedAt"],
        13
    );
}
#[test]
fn attachment_actions_return_preview_or_reveal_and_restore_focus_by_id() {
    let mut p = panel();
    p.add_attachments("t",vec![Attachment::from_record(json!({"id":"image","name":"Image","type":"image/png","dataUrl":"data:image/png;base64,AA=="})).unwrap(),Attachment::from_record(json!({"id":"file","name":"File","type":"application/octet-stream","dataBase64":"AP8="})).unwrap()],1).unwrap();
    assert!(
        matches!(p.open_attachment("t","image").unwrap().kind,EffectKind::OpenAttachment{preview:true,ref restore_focus,..} if restore_focus=="image")
    );
    assert!(matches!(
        p.open_attachment("t", "file").unwrap().kind,
        EffectKind::OpenAttachment { preview: false, .. }
    ));
    let inspect = p.inspect("t").unwrap().to_string();
    assert!(!inspect.contains("base64"));
    assert!(!inspect.contains("AP8="));
    assert!(inspect.contains("payloadAvailable"));
}
#[test]
fn complete_keeps_review_plan_and_result_undo_ack_survives_run_settlement() {
    let mut p = panel();
    start(&mut p);
    p.receive("t", "run1", 1, Update::Plan(json!({"kind":"scene"})), 11)
        .unwrap();
    p.receive(
        "t",
        "run1",
        2,
        Update::Complete {
            text: String::new(),
            error: false,
            cancelled: false,
        },
        12,
    )
    .unwrap();
    assert_eq!(p.session("t").unwrap().phase, Phase::Preview);
    let e = p.apply_plan("t").unwrap();
    p.acknowledge(e.id, false).unwrap();
    assert_eq!(p.session("t").unwrap().phase, Phase::Preview);
}

#[test]
fn result_artifact_references_survive_keep_followup_and_restart() {
    let mut p = panel();
    start(&mut p);
    p.receive(
        "t",
        "run1",
        1,
        Update::Result(
            json!({"artifacts":[{"path":"synthetic/only-copy.mov","name":"Output"}],"future":true}),
        ),
        11,
    )
    .unwrap();
    p.receive(
        "t",
        "run1",
        2,
        Update::Complete {
            text: "done".into(),
            error: false,
            cancelled: false,
        },
        12,
    )
    .unwrap();
    let mut store = Memory::default();
    p.save(&mut store, 13).unwrap();
    p.keep_result("t").unwrap();
    p.save(&mut store, 14).unwrap();
    let restored = Panel::load(&mut store, "p", "blank", 15).unwrap();
    let record = restored
        .session("t")
        .unwrap()
        .conversation
        .values()
        .find(|m| m["nativeResult"].is_object())
        .unwrap();
    assert_eq!(
        record["nativeResult"]["artifacts"][0]["path"],
        "synthetic/only-copy.mov"
    );
    assert_eq!(record["nativeResult"]["future"], true);
}
#[test]
fn media_nonfinal_outcomes_keep_truthful_status_and_full_original_output() {
    use pm_panel_agentpanel::activity::media_outcome;
    assert_eq!(
        media_outcome(
            "mcp__pm__transcribe_media",
            &json!("Error: {\"status\":\"model-required\"}")
        ),
        Some(json!({"state":"needs-model"}))
    );
    assert_eq!(
        media_outcome(
            "transcribe_media",
            &json!("{\"status\":\"transcribing\",\"progress\":0.42}")
        ),
        Some(json!({"state":"pending","progress":0.42}))
    );
    assert!(
        media_outcome(
            "transcribe_media",
            &json!("{\"text\":\"quoted\",\"status\":\"transcribing\"}")
        )
        .is_none()
    );
    let mut trace = Trace::default();
    trace.receive(&json!({"kind":"tool-start","itemId":"x","toolName":"transcribe_media","label":"Transcribing"}),0).unwrap();
    trace.receive(&json!({"kind":"tool-end","itemId":"x","isError":true,"output":"{\"status\":\"model-required\"}"}),1).unwrap();
    assert_eq!(trace.rows.get("x").unwrap().value["status"], "done");
    assert_eq!(trace.rows.get("x").unwrap().value["outcome"], "needs-model");
}
#[test]
fn single_choice_can_be_toggled_off_and_typed_answer_is_combined() {
    use pm_panel_agentpanel::questions::QuestionDraft;
    let step = json!({"status":"open","transport":"reply","questions":[{"id":"q","multiSelect":false,"allowOther":true,"options":[{"label":"A"},{"label":"B"}]}]});
    let mut draft = QuestionDraft::default();
    draft.choose(&step, "q", "A").unwrap();
    draft.choose(&step, "q", "B").unwrap();
    draft.type_answer(&step, "q", " typed ".into()).unwrap();
    assert_eq!(draft.submit(&step, false).unwrap()["q"], vec!["B", "typed"]);
    draft.choose(&step, "q", "B").unwrap();
    assert_eq!(draft.submit(&step, false).unwrap()["q"], vec!["typed"]);
    assert!(draft.submit(&step, true).unwrap().is_empty());
}

#[test]
fn composer_text_answers_blocking_question_using_run_request_fallback() {
    let mut p = panel();
    start(&mut p);
    let mut q = question(false, "reply");
    q.as_object_mut().unwrap().remove("requestId");
    event(&mut p, 1, q);
    p.draft_mut("t")
        .unwrap()
        .insert_text("the answer", 20, false);
    let e = p.submit("t", "run1", "request1", 21).unwrap();
    assert!(
        matches!(e.kind,EffectKind::Answer{ref request_id,ref answers,..} if request_id.as_deref()==Some("request1") && answers.0["first"]==vec!["the answer"])
    );
    assert_eq!(p.session("t").unwrap().conversation.len(), 1);
    assert_eq!(p.session("t").unwrap().draft.request_text(), "");
}
#[test]
fn message_question_answer_starts_new_run_when_idle_and_preserves_secret_draft() {
    let mut p = panel();
    start(&mut p);
    event(&mut p, 1, question(true, "message"));
    p.receive(
        "t",
        "run1",
        2,
        Update::Complete {
            text: "done".into(),
            error: false,
            cancelled: false,
        },
        20,
    )
    .unwrap();
    p.draft_mut("t")
        .unwrap()
        .insert_text("unsent composer", 21, false);
    p.type_answer("t", "q", "first", "private synthetic value".into())
        .unwrap();
    let e = p.answer_at("t", "q", false, 22).unwrap();
    assert!(
        matches!(e.kind,EffectKind::Send{steer:false,ref text,..} if text.contains("(answered privately)")&&!text.contains("private synthetic value"))
    );
    assert_eq!(
        p.session("t").unwrap().draft.request_text(),
        "unsent composer"
    );
    assert!(p.session("t").unwrap().run.is_some());
}
#[test]
fn nonblocking_held_question_survives_steering_checkpoint_and_closes_at_completion() {
    let mut p = panel();
    start(&mut p);
    let mut q = question(false, "reply");
    q["blocking"] = json!(false);
    event(&mut p, 1, q);
    p.draft_mut("t").unwrap().insert_text("continue", 20, false);
    p.submit("t", "run1", "request1", 21).unwrap();
    assert_eq!(
        p.session("t").unwrap().question("q").unwrap()["status"],
        "open"
    );
    p.receive(
        "t",
        "run1",
        2,
        Update::Complete {
            text: "done".into(),
            error: false,
            cancelled: false,
        },
        30,
    )
    .unwrap();
    assert_eq!(
        p.session("t").unwrap().question("q").unwrap()["status"],
        "closed"
    );
}

#[test]
fn background_launch_never_steals_visible_blank_thread_or_inherits_computer_grant() {
    let mut p = panel();
    let consent = p.set_access("t", "full").unwrap().unwrap();
    p.acknowledge(consent.id, true).unwrap();
    p.launch_background("background", 10).unwrap();
    assert_eq!(p.active_id(), "t");
    assert_eq!(
        p.session("background").unwrap().settings.access,
        Access::Project
    );
    let e = p
        .send_message(
            "background",
            "plan the edit",
            &[],
            "background-run",
            "request",
            11,
        )
        .unwrap();
    assert_eq!(e.target.thread, "background");
    assert_eq!(p.active_id(), "t");
    assert_eq!(p.background_runs(), 1);
    assert!(p.launch_background("background", 12).is_err());
}
#[test]
fn agent_send_keeps_visible_draft_selection_files_and_undo() {
    let mut p = panel();
    p.draft_mut("t").unwrap().insert_text("my draft", 1, false);
    p.draft_mut("t").unwrap().select(0..2).unwrap();
    p.send_message("t", "background direction", &[], "run1", "request1", 10)
        .unwrap();
    assert_eq!(p.session("t").unwrap().draft.request_text(), "my draft");
    assert_eq!(p.session("t").unwrap().draft.selection(), 0..2);
    assert!(p.draft_mut("t").unwrap().undo(false));
    assert_eq!(p.session("t").unwrap().draft.request_text(), "");
}
#[test]
fn subagent_stop_still_targets_the_archived_row_after_steering_update() {
    let mut p = panel();
    start(&mut p);
    let task = json!({"kind":"task","task":{"taskId":"child","requestId":"request1","title":"Review","status":"running"}});
    event(&mut p, 1, task.clone());
    p.draft_mut("t").unwrap().insert_text("steer", 20, false);
    p.submit("t", "run1", "request1", 21).unwrap();
    event(&mut p, 2, task);
    assert!(
        matches!(p.stop_task("t","child").unwrap().kind,EffectKind::StopTask{ref task,..} if task=="child")
    );
}

#[test]
fn imported_seven_file_draft_remains_readable_despite_new_message_limit() {
    let mut store = Memory::default();
    store.values.insert("agentThreads.p".into(),json!({"version":1,"activeId":"t","threads":[{"id":"t","title":"Legacy","updatedAt":1,"conversation":[],"composerDraft":"","attachments":(0..7).map(|i|json!({"id":format!("a{i}"),"name":format!("file{i}"),"dataBase64":""})).collect::<Vec<_>>()}]}));
    let p = Panel::load(&mut store, "p", "blank", 2).unwrap();
    assert_eq!(p.session("t").unwrap().draft.attachments().count(), 7);
}
#[test]
fn duplicate_active_run_id_is_refused_without_consuming_another_threads_draft() {
    let mut p = panel();
    start(&mut p);
    p.launch_background("other", 10).unwrap();
    p.draft_mut("other")
        .unwrap()
        .insert_text("retain", 11, false);
    assert!(p.submit("other", "run1", "other-request", 12).is_err());
    assert_eq!(p.session("other").unwrap().draft.request_text(), "retain");
    assert_eq!(p.session("other").unwrap().conversation.len(), 0);
}
#[test]
fn shell_drag_and_pin_effects_leave_live_conversation_and_draft_owned_by_same_thread() {
    let mut p = panel();
    start(&mut p);
    p.draft_mut("t").unwrap().insert_text("unsent", 12, false);
    for action in [
        ShellAction::BeginDrag,
        ShellAction::CancelDrag,
        ShellAction::CommitDrag {
            dock: Some("right".into()),
        },
        ShellAction::Pin,
        ShellAction::Close,
        ShellAction::Open,
    ] {
        let e = p.host_action("t", action).unwrap();
        assert_eq!(e.target.thread, "t");
        assert_eq!(e.target.run.as_deref(), Some("run1"));
        assert_eq!(p.session("t").unwrap().draft.request_text(), "unsent");
        assert_eq!(p.session("t").unwrap().conversation.len(), 1);
    }
}

#[test]
fn new_ui_thread_preserves_provider_model_effort_and_requires_its_own_computer_grant() {
    let mut p = panel();
    p.set_provider(
        "t",
        Provider::Claude,
        "sonnet".into(),
        Catalog {
            providers: vec![("claude".into(), "Claude".into())],
            models: vec![("sonnet".into(), "Sonnet".into())],
            efforts: vec!["high".into()],
            model_efforts: vec![],
            claude: true,
            resolves: vec![],
        },
    )
    .unwrap();
    p.set_model("t", "sonnet".into(), "high".into()).unwrap();
    let consent = p.set_access("t", "full").unwrap().unwrap();
    p.acknowledge(consent.id, true).unwrap();
    p.draft_mut("t").unwrap().insert_text("old draft", 1, false);
    p.new_thread("new", 2).unwrap();
    let s = p.session("new").unwrap();
    assert_eq!(s.settings.provider, Provider::Claude);
    assert_eq!(s.settings.model, "sonnet");
    assert_eq!(s.settings.effort, "high");
    assert_eq!(s.settings.access, Access::Project);
}

#[test]
fn rejected_answer_after_same_run_settles_closes_its_archived_acknowledgement() {
    let mut p = panel();
    start(&mut p);
    event(&mut p, 1, question(false, "reply"));
    p.choose_answer("t", "q", "first", "A").unwrap();
    let effect = p.answer("t", "q", false).unwrap();
    p.receive(
        "t",
        "run1",
        2,
        Update::Complete {
            text: "finished".into(),
            error: false,
            cancelled: false,
        },
        20,
    )
    .unwrap();
    p.acknowledge(effect.id, false).unwrap();
    let q = p.session("t").unwrap().question("q").unwrap();
    assert_eq!(q["status"], "closed");
    assert!(q.get("answers").is_none());
}

#[test]
fn switching_models_keeps_only_an_effort_the_new_model_takes() {
    let mut p = panel();
    let catalog = Catalog {
        providers: vec![("chatgpt".into(), "ChatGPT".into())],
        models: vec![
            ("fast".into(), "Fast".into()),
            ("deep".into(), "Deep".into()),
        ],
        efforts: vec!["".into(), "low".into(), "xhigh".into()],
        model_efforts: vec![
            ("fast".into(), vec!["low".into()]),
            ("deep".into(), vec!["low".into(), "xhigh".into()]),
        ],
        claude: false,
        resolves: vec![],
    };
    p.set_provider("t", Provider::Chatgpt, "deep".into(), catalog)
        .unwrap();
    p.set_model("t", "deep".into(), "xhigh".into()).unwrap();
    // The carried effort doesn't exist on Fast: it falls back to Default.
    p.set_model("t", "fast".into(), "xhigh".into()).unwrap();
    let settings = &p.session("t").unwrap().settings;
    assert_eq!(
        (settings.model.as_str(), settings.effort.as_str()),
        ("fast", "")
    );
    // Asking Fast for an effort it lacks fails.
    assert!(p.set_model("t", "fast".into(), "xhigh".into()).is_err());
    p.set_model("t", "fast".into(), "low".into()).unwrap();
}

#[test]
fn a_thread_saved_with_no_model_or_default_reads_as_the_providers_default_model() {
    // The pickers list only real models; a saved "" / null / "default" (and
    // Claude's resolved id) resolve to one, while the saved value is kept so
    // the provider still picks its own default.
    let catalog = Catalog {
        providers: vec![("claude".into(), "Claude".into())],
        models: vec![
            ("opus".into(), "Opus".into()),
            ("sonnet".into(), "Sonnet".into()),
        ],
        efforts: vec!["".into(), "low".into(), "max".into()],
        model_efforts: vec![
            ("opus".into(), vec!["low".into(), "max".into()]),
            ("sonnet".into(), vec!["low".into()]),
        ],
        claude: true,
        resolves: vec![
            ("".into(), "opus".into()),
            ("claude-opus-5-5".into(), "opus".into()),
        ],
    };
    assert!(
        !catalog
            .models
            .iter()
            .any(|(id, label)| id.is_empty() || label.contains("Default"))
    );
    let mut p = panel();
    let mut store = Memory::default();
    p.save(&mut store, 1).unwrap();
    for (saved, shown) in [
        (Value::Null, "opus"),
        (json!(""), "opus"),
        (json!("default"), "opus"),
        (json!("claude-opus-5-5"), "opus"),
        (json!("sonnet"), "sonnet"),
    ] {
        let mut store = Memory {
            values: store.values.clone(),
            ..Default::default()
        };
        let record = &mut store.values.get_mut("agentThreads.p").unwrap()["threads"][0];
        record["provider"] = json!("claude");
        record["model"] = saved.clone();
        let mut restored = Panel::load(&mut store, "p", "blank", 2).unwrap();
        restored.set_catalog("t", catalog.clone()).unwrap();
        let settings = &restored.session("t").unwrap().settings;
        assert_eq!(catalog.resolve(&settings.model), shown, "{saved}");
        // Its efforts are the resolved model's own.
        assert_eq!(
            catalog.efforts_for(&settings.model).len(),
            if shown == "opus" { 3 } else { 2 },
            "{saved}"
        );
        // Effort changes on the resolved model are accepted.
        restored.set_model("t", shown.into(), "low".into()).unwrap();
    }
    // A provider with no listing yet keeps the saved value: nothing to name.
    assert_eq!(Catalog::default().resolve(""), "");
    // No default flagged: the first listed model is the one that runs.
    let unflagged = Catalog {
        resolves: vec![],
        ..catalog.clone()
    };
    assert_eq!(unflagged.resolve(""), "opus");
    assert_eq!(unflagged.resolve("unknown-model"), "unknown-model");
}
