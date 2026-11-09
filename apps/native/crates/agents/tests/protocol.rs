mod support;
use pm_agents::{
    Access, ApprovalMode, Effects, Event, RunResult,
    approvals::{Approvals, Authority},
    claude, codex,
    protocol::Lines,
};
use serde_json::{Value, json};
use std::collections::BTreeMap;
fn run(id: &str) -> codex::Run {
    codex::Run {
        id: id.into(),
        cwd: "/workspace/agent".into(),
        prompt: "Synthetic brief".into(),
        model: None,
        reasoning_effort: None,
        schema: None,
        image_paths: vec![],
        config: json!({"mcp_servers":{}}),
        resume_thread: None,
    }
}
fn started() -> codex::Client {
    let mut c = codex::Client::default();
    c.run(run("a"), 0).unwrap();
    c.receive(json!({"id":1,"result":{}}), 0);
    c.receive(json!({"id":2,"result":{"thread":{"id":"t"}}}), 0);
    c.receive(json!({"id":3,"result":{"turn":{"id":"u"}}}), 0);
    c
}
fn completed(e: &Effects) -> usize {
    e.events
        .iter()
        .filter(|e| matches!(e, Event::Completed { .. }))
        .count()
}
#[test]
fn framing_preserves_split_multibyte_and_unterminated_tail() {
    let s = "{\"text\":\"café 🎞\"}";
    for chunk in 1..s.len() {
        let mut l = Lines::new(1000);
        let mut values = vec![];
        for bytes in s.as_bytes().chunks(chunk) {
            values.extend(l.push(bytes));
        }
        values.extend(l.finish());
        assert_eq!(values, vec![Ok(json!({"text":"café 🎞"}))]);
        assert!(l.finish().is_empty());
    }
}
#[test]
fn framing_drops_entire_oversized_event() {
    let mut l = Lines::new(20);
    assert!(l.push(b"xxxxxxxxxxxxxxxxxxxxx").is_empty());
    assert_eq!(
        l.push(b"{\"id\":1}\n{\"id\":2}\n"),
        vec![
            Err("Runtime emitted an oversized event".into()),
            Ok(json!({"id":2}))
        ]
    );
}
#[test]
fn framing_ignores_nonobjects_and_malformed() {
    let mut l = Lines::new(100);
    assert_eq!(
        l.push(b"junk\nnull\n[]\ntrue\n{\"x\":1}\r\n"),
        vec![Ok(json!({"x":1}))]
    );
}
#[test]
fn computer_authority_requires_fresh_consent() {
    for mode in [ApprovalMode::Auto, ApprovalMode::Supervised] {
        assert!(Authority::new(Access::Computer, mode, false).is_err());
        assert!(
            Authority::new(Access::Computer, mode, true)
                .unwrap()
                .computer_allowed()
        );
    }
}
#[test]
fn approval_allowance_is_scoped_to_one_run() {
    let mut a = Approvals::default();
    assert!(a.decide("run-a", &[" Allow for this run ".into()]).0);
    assert!(a.allowed("run-a"));
    assert!(!a.allowed("run-b"));
    a.close("run-a");
    assert!(!a.allowed("run-a"));
    assert!(!a.decide("run-a", &["Use another path".into()]).0);
}
#[test]
fn duplicate_run_never_starts_second_turn() {
    let mut c = started();
    assert!(c.run(run("a"), 0).is_err());
    assert!(c.run(run("b"), 0).is_ok());
}
#[test]
fn server_request_id_collision_cannot_settle_client_request() {
    let mut c = codex::Client::default();
    let first = c.run(run("a"), 0).unwrap();
    assert_eq!(first.outgoing.len(), 1);
    let e = c.receive(json!({"id":1,"method":"unknown","params":{}}), 0);
    assert_eq!(e.outgoing[0]["error"]["code"], -32601);
    let e = c.receive(json!({"id":1,"result":{}}), 0);
    assert_eq!(e.outgoing[1]["method"], "thread/start");
}
#[test]
fn server_question_with_missing_scope_is_settled() {
    let mut c = started();
    let e=c.receive(json!({"id":"q","method":"item/tool/requestUserInput","params":{"questions":[{"id":"q","question":"Why?"}],"itemId":"i"}}),0);
    assert_eq!(e.outgoing, vec![json!({"id":"q","result":{"answers":{}}})]);
}
#[test]
fn question_without_choices_still_accepts_text() {
    let mut c = started();
    let e=c.receive(json!({"id":"q","method":"item/tool/requestUserInput","params":{"threadId":"t","turnId":"u","itemId":"i","questions":[{"id":"q","question":"Why?"}]}}),0);
    assert!(
        matches!(&e.events[0],Event::Trace{value,..}if value["questions"][0]["allowOther"]==true)
    );
}
#[test]
fn cancel_drains_held_questions_and_interrupts_exact_turn() {
    let mut c = started();
    c.receive(json!({"id":"q","method":"item/tool/requestUserInput","params":{"threadId":"t","turnId":"u","itemId":"i","questions":[{"id":"q","question":"Why?"}]}}),0);
    let e = c.cancel("a", 0).unwrap();
    assert_eq!(
        e.outgoing
            .iter()
            .find(|m| m["method"] == "turn/interrupt")
            .unwrap()["params"],
        json!({"threadId":"t","turnId":"u"})
    );
    assert_eq!(e.outgoing.len(), 1);
    assert_eq!(completed(&e), 0);
    assert!(c.answer("a", "i", &BTreeMap::new()).is_none());
    let interrupt = e.outgoing[0]["id"].clone();
    let settled = c.receive(json!({"id":interrupt,"result":{}}), 1);
    assert_eq!(completed(&settled), 1);
    assert_eq!(
        settled.outgoing,
        vec![json!({"id":"q","result":{"answers":{}}})]
    );
}
#[test]
fn cancellation_before_turn_id_interrupts_late_ack_once() {
    let mut c = codex::Client::default();
    c.run(run("a"), 0).unwrap();
    c.receive(json!({"id":1,"result":{}}), 0);
    c.receive(json!({"id":2,"result":{"thread":{"id":"t"}}}), 0);
    let e = c.cancel("a", 0).unwrap();
    assert_eq!(completed(&e), 1);
    let e = c.receive(json!({"id":3,"result":{"turn":{"id":"u"}}}), 0);
    assert_eq!(e.outgoing[0]["method"], "turn/interrupt");
    assert_eq!(completed(&e), 0);
    assert_eq!(completed(&c.stopped("exit")), 0);
}
#[test]
fn cancel_during_initialize_never_creates_thread() {
    let mut c = codex::Client::default();
    c.run(run("a"), 0).unwrap();
    assert_eq!(completed(&c.cancel("a", 0).unwrap()), 1);
    assert_eq!(
        c.receive(json!({"id":1,"result":{}}), 0).outgoing,
        vec![json!({"method":"initialized","params":{}})]
    );
}
#[test]
fn queued_steering_preserves_order() {
    let mut c = codex::Client::default();
    c.run(run("a"), 0).unwrap();
    c.receive(json!({"id":1,"result":{}}), 0);
    c.receive(json!({"id":2,"result":{"thread":{"id":"t"}}}), 0);
    assert!(c.steer("a", "first", &[], 0).unwrap().outgoing.is_empty());
    c.steer("a", "second", &[], 0).unwrap();
    let e = c.receive(json!({"id":3,"result":{"turn":{"id":"u"}}}), 0);
    assert_eq!(e.outgoing[0]["params"]["input"][0]["text"], "first");
    assert_eq!(e.outgoing[1]["params"]["input"][0]["text"], "second");
}
#[test]
fn stale_turn_and_foreign_thread_cannot_finish_run() {
    let mut c = started();
    for (t, u) in [("other", "u"), ("t", "old")] {
        let e=c.receive(json!({"method":"turn/completed","params":{"threadId":t,"turn":{"id":u,"status":"completed"}}}),0);
        assert_eq!(completed(&e), 0);
    }
    let e = c.cancel("a", 0).unwrap();
    assert_eq!(completed(&e), 0);
    let id = e.outgoing.last().unwrap()["id"].clone();
    assert_eq!(completed(&c.receive(json!({"id":id,"result":{}}), 1)), 1);
}
#[test]
fn async_updates_never_replace_final_answer() {
    let mut c = started();
    for (delivery, value) in [("sync", "authored"), ("async", "status")] {
        c.receive(json!({"method":"item/completed","params":{"threadId":"t","turnId":"u","item":{"id":delivery,"type":"agentMessage","delivery":delivery,"text":value}}}),0);
    }
    let e=c.receive(json!({"method":"turn/completed","params":{"threadId":"t","turn":{"id":"u","status":"completed"}}}),0);
    assert!(
        matches!(&e.events[0],Event::Completed{result:RunResult::Success{text,..},..}if text=="authored")
    );
}
#[test]
fn compaction_exhaustion_has_actionable_result() {
    let mut c = started();
    let e=c.receive(json!({"method":"turn/completed","params":{"threadId":"t","turn":{"id":"u","status":"failed","error":{"codexErrorInfo":"contextWindowExceeded"}}}}),0);
    assert!(
        matches!(&e.events[0],Event::Completed{result:RunResult::Failure{error,..},..}if error==codex::CONTEXT_EXHAUSTED)
    );
}
#[test]
fn request_timeout_fails_run_and_cleans_pending() {
    let mut c = codex::Client::default();
    c.run(run("a"), 0).unwrap();
    let e = c.tick(30000);
    assert_eq!(completed(&e), 1);
    assert_eq!(completed(&c.tick(30001)), 0);
}
#[test]
fn explicit_resume_uses_resume_without_duplicating_history() {
    let mut c = codex::Client::default();
    let mut r = run("a");
    r.resume_thread = Some("existing".into());
    c.run(r, 0).unwrap();
    let e = c.receive(json!({"id":1,"result":{}}), 0);
    assert_eq!(e.outgoing[1]["method"], "thread/resume");
    assert_eq!(e.outgoing[1]["params"]["threadId"], "existing");
}
fn question() -> Value {
    json!({"type":"control_request","request_id":"c1","request":{"subtype":"can_use_tool","tool_name":"AskUserQuestion","tool_use_id":"i","input":{"questions":[{"question":"Original unclipped question","options":[]}]}}})
}
fn claude(access: Access) -> claude::Client {
    claude::Client::new(
        "a".into(),
        Authority::new(access, ApprovalMode::Supervised, false).unwrap(),
        0,
    )
}
#[test]
fn claude_skip_question_settles_stdio() {
    let mut c = claude(Access::Editor);
    c.receive(question());
    let e = c.answer("i", &BTreeMap::new()).unwrap();
    assert_eq!(e.outgoing[0]["response"]["response"]["behavior"], "deny");
    assert!(c.answer("i", &BTreeMap::new()).is_none());
}
#[test]
fn claude_cancel_closes_held_question() {
    let mut c = claude(Access::Editor);
    c.receive(question());
    let e = c.cancel();
    assert!(e.close_stdin);
    assert_eq!(e.outgoing[0]["response"]["response"]["behavior"], "deny");
    assert_eq!(completed(&e), 1);
    assert!(
        c.receive(json!({"type":"result","result":"late"}))
            .events
            .is_empty()
    );
}
#[test]
fn claude_control_cancel_invalidates_answer() {
    let mut c = claude(Access::Editor);
    c.receive(question());
    let e = c.receive(json!({"type":"control_cancel_request","request_id":"c1"}));
    assert!(matches!(&e.events[0],Event::Trace{value,..}if value["kind"]=="question-closed"));
    assert!(c.answer("i", &BTreeMap::new()).is_none());
}
#[test]
fn claude_project_approvals_hold_and_allow_run() {
    let mut c = claude(Access::Project);
    let ask = |id| json!({"type":"control_request","request_id":id,"request":{"subtype":"can_use_tool","tool_name":"Bash","input":{"command":"pwd","dangerouslyDisableSandbox":true}}});
    let e = c.receive(ask("c1"));
    assert!(e.outgoing.is_empty());
    let e = c
        .answer(
            "approval-c1",
            &BTreeMap::from([("decision".into(), vec!["Allow for this run".into()])]),
        )
        .unwrap();
    assert_eq!(e.outgoing[0]["response"]["response"]["behavior"], "allow");
    let e = c.receive(ask("c2"));
    assert_eq!(e.outgoing[0]["response"]["response"]["behavior"], "allow");
}
#[test]
fn claude_compaction_is_housekeeping() {
    let mut c = claude(Access::Editor);
    let a = c.receive(json!({"type":"system","subtype":"status","status":"compacting"}));
    assert_eq!(a.events.len(), 1);
    assert!(
        c.receive(json!({"type":"system","subtype":"status","status":"compacting"}))
            .events
            .is_empty()
    );
    let e = c.receive(json!({"type":"system","subtype":"compact_boundary"}));
    assert!(matches!(&e.events[0],Event::Trace{value,..}if value["kind"]=="tool-end"));
    assert_eq!(completed(&e), 0);
}
#[test]
fn claude_arguments_do_not_escalate_editor() {
    let options = claude::Options {
        authority: Authority::new(Access::Editor, ApprovalMode::Auto, false).unwrap(),
        schema: json!({}),
        model: None,
        effort: None,
        session: None,
        mcp: json!({"mcpServers":{}}),
        extensions: None,
        instructions: "synthetic".into(),
        project_tools: vec![],
        editor_tools: vec![],
    };
    let args = options.argv().unwrap();
    assert!(args.contains(&"--permission-prompt-tool".into()));
    assert!(args.contains(&"--no-session-persistence".into()));
    assert!(!args.iter().any(|s| s.contains("dangerously")));
    let settings: Value =
        serde_json::from_str(&args[args.iter().position(|a| a == "--settings").unwrap() + 1])
            .unwrap();
    assert_eq!(settings["sandbox"]["failIfUnavailable"], true);
    assert_eq!(settings["sandbox"]["autoAllowBashIfSandboxed"], false);
}
#[test]
fn cancelled_turn_reports_only_once_even_if_start_times_out() {
    let mut c = codex::Client::default();
    c.run(run("a"), 0).unwrap();
    c.receive(json!({"id":1,"result":{}}), 0);
    c.receive(json!({"id":2,"result":{"thread":{"id":"t"}}}), 0);
    assert_eq!(completed(&c.cancel("a", 0).unwrap()), 1);
    assert_eq!(completed(&c.tick(30000)), 0);
    assert_eq!(completed(&c.stopped("exit")), 0);
}
#[test]
fn codex_exec_approval_profiles_preserve_supervised_and_auto_modes() {
    for mode in [ApprovalMode::Auto, ApprovalMode::Supervised] {
        let o = pm_agents::codex_exec::Options {
            authority: Authority::new(Access::Project, mode, false).unwrap(),
            extensions: "/workspace/ext".into(),
            schema_path: "/state/schema".into(),
            output_path: "/state/output".into(),
            prompt: "--literal prompt".into(),
            instructions: "policy".into(),
            model: None,
            effort: None,
            session: Some("saved-thread".into()),
            image_paths: vec![],
            shell_network: true,
            mcp_config: vec![],
        };
        let args = o.argv().unwrap();
        assert!(args.contains(&"resume".into()));
        assert!(args.contains(&"permissions.powermove.network.enabled=true".into()));
        assert!(!args.iter().any(|s| s.contains("dangerously")));
        assert!(args.contains(&if mode == ApprovalMode::Auto {
            "approvals_reviewer=\"auto_review\"".into()
        } else {
            "approval_policy=\"never\"".into()
        }));
        let divider = args.iter().position(|a| a == "--").unwrap();
        assert_eq!(args[divider + 1], "saved-thread");
    }
}
#[test]
fn rejected_steering_does_not_destroy_active_turn() {
    let mut c = started();
    let e = c.steer("a", "update", &[], 0).unwrap();
    let id = e.outgoing[0]["id"].clone();
    let e = c.receive(
        json!({"id":id,"error":{"code":-32000,"message":"stale steer"}}),
        0,
    );
    assert_eq!(completed(&e), 0);
    let e = c.cancel("a", 0).unwrap();
    assert_eq!(completed(&e), 0);
    let id = e.outgoing.last().unwrap()["id"].clone();
    assert_eq!(completed(&c.receive(json!({"id":id,"result":{}}), 1)), 1);
}
#[test]
fn item_without_turn_identity_cannot_become_the_answer() {
    let mut c = started();
    let e=c.receive(json!({"method":"item/completed","params":{"threadId":"t","item":{"type":"agentMessage","text":"unscoped"}}}),0);
    assert!(e.events.iter().all(|e| !matches!(e, Event::Trace { .. })));
    let e=c.receive(json!({"method":"turn/completed","params":{"threadId":"t","turn":{"id":"u","status":"completed"}}}),0);
    assert!(matches!(
        &e.events[0],
        Event::Completed {
            result: RunResult::Failure { .. },
            ..
        }
    ));
}
#[test]
fn bounded_activity_formatting_keeps_unicode_and_line_structure() {
    use pm_agents::trace;
    assert_eq!(
        trace::fragment(&json!("  café\n\tline\u{7}  ")),
        "  café\n\tline  "
    );
    assert_eq!(
        trace::detail("Bash", &json!({"command":"first\r\nsecond"})),
        "first ⏎ second"
    );
    assert_eq!(trace::truncate("a🎞b", 2), "a");
    assert_eq!(
        trace::excerpt(&json!({"content":[{"text":"one\ntwo"}]})),
        "one\ntwo"
    );
    assert_eq!(
        trace::label("mcp__studio__HTTPServerTool"),
        "Http server tool"
    );
    assert_eq!(
        trace::fragment(&json!("🎞".repeat(2000)))
            .encode_utf16()
            .count(),
        2000
    );
}

#[test]
fn cancel_completion_waits_for_ack_and_survives_rejection_or_timeout() {
    for reply in [
        Some(json!({"result":{}})),
        Some(json!({"error":{"code":-1,"message":"already done"}})),
        None,
    ] {
        let mut c = started();
        let e = c.cancel("a", 10).unwrap();
        assert_eq!(completed(&e), 0);
        assert!(c.cancel("a", 11).unwrap().outgoing.is_empty());
        assert!(c.steer("a", "too late", &[], 11).is_err());
        let e = if let Some(mut reply) = reply {
            reply["id"] = e.outgoing[0]["id"].clone();
            c.receive(reply, 20)
        } else {
            assert_eq!(completed(&c.tick(30_009)), 0);
            c.tick(30_010)
        };
        assert!(
            matches!(&e.events[..], [Event::Completed { run, result:RunResult::Failure{cancelled:true,..} }] if run=="a")
        );
        assert_eq!(completed(&c.tick(3_600_001)), 0);
        assert_eq!(completed(&c.stopped("exit")), 0);
    }
}
#[test]
fn cancelled_turn_cannot_publish_late_progress_questions_or_success() {
    let mut c = started();
    c.cancel("a", 0).unwrap();
    for message in [
        json!({"method":"warning","params":{"threadId":"t","message":"Reconnecting..."}}),
        json!({"method":"item/agentMessage/delta","params":{"threadId":"t","turnId":"u","delta":"late"}}),
        json!({"method":"turn/completed","params":{"threadId":"t","turn":{"id":"u","status":"completed"}}}),
    ] {
        let e = c.receive(message, 1);
        assert!(e.events.iter().all(|e| matches!(e, Event::Notification(_))));
    }
    let e = c.receive(json!({"id":"late-q","method":"item/tool/requestUserInput","params":{"threadId":"t","turnId":"u","itemId":"late","questions":[{"id":"q","question":"Late?"}]}}),1);
    assert_eq!(
        e.outgoing,
        vec![json!({"id":"late-q","result":{"answers":{}}})]
    );
    assert_eq!(completed(&c.receive(json!({"id":4,"result":{}}), 2)), 1);
}
#[test]
fn stopping_during_interrupt_completes_once_then_reinitializes() {
    let mut c = started();
    c.cancel("a", 0).unwrap();
    assert_eq!(completed(&c.stopped("pipe closed")), 1);
    assert_eq!(completed(&c.receive(json!({"id":4,"result":{}}), 1)), 0);
    let e = c.run(run("b"), 2).unwrap();
    assert_eq!(e.outgoing[0]["method"], "initialize");
    let id = e.outgoing[0]["id"].clone();
    let e = c.receive(json!({"id":id,"result":{}}), 3);
    assert_eq!(e.outgoing[1]["method"], "thread/start");
}

#[test]
fn cancellation_settles_questions_in_authored_arrival_order_after_interrupt() {
    let mut c = started();
    for id in ["z-first", "a-second"] {
        c.receive(json!({"id":id,"method":"item/tool/requestUserInput","params":{"threadId":"t","turnId":"u","itemId":id,"questions":[{"id":"q","question":"Choose?"}]}}),0);
    }
    let e = c.cancel("a", 0).unwrap();
    assert_eq!(e.outgoing.len(), 1);
    let rpc = e.outgoing[0]["id"].clone();
    let e = c.receive(json!({"id":rpc,"result":{}}), 1);
    assert_eq!(
        e.outgoing,
        vec![
            json!({"id":"z-first","result":{"answers":{}}}),
            json!({"id":"a-second","result":{"answers":{}}})
        ]
    );
    assert_eq!(completed(&e), 1);
}
