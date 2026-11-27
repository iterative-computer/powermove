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
        project: None,
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
        e.events.as_slice(),
        [Event::Completed { result: RunResult::Success { text, .. }, .. }] if text.is_empty()
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

#[test]
fn provider_turns_that_complete_without_text_are_successful() {
    let codex = started().receive(
        json!({"method":"turn/completed","params":{"threadId":"t","turn":{"id":"u","status":"completed"}}}),
        0,
    );
    let claude = claude(Access::Editor).receive(json!({"type":"result","subtype":"success"}));
    let mut exec = pm_agents::codex_exec::Client::new(
        "a".into(),
        Authority::new(Access::Editor, ApprovalMode::Supervised, false).unwrap(),
        0,
    );
    let exited = exec.exited(Some(0), None);
    for effect in [codex, claude, exited] {
        assert!(matches!(
            effect.events.as_slice(),
            [Event::Completed { result: RunResult::Success { text, .. }, .. }] if text.is_empty()
        ), "{:?}", effect.events);
    }
}

#[test]
fn nonzero_exec_exit_stays_a_failure() {
    let mut exec = pm_agents::codex_exec::Client::new(
        "a".into(),
        Authority::new(Access::Editor, ApprovalMode::Supervised, false).unwrap(),
        0,
    );
    assert!(matches!(
        exec.exited(Some(1), None).events.as_slice(),
        [Event::Completed { result: RunResult::Failure { cancelled: false, .. }, .. }]
    ));
}

#[test]
fn actual_provider_failure_keeps_its_diagnostic() {
    let mut client = started();
    let failure = "The provider stopped responding and timed out.";
    let effect = client.receive(
        json!({"method":"turn/completed","params":{"threadId":"t","turn":{"id":"u","status":"failed","error":{"message":failure}}}}),
        0,
    );
    assert!(matches!(
        effect.events.as_slice(),
        [Event::Completed { result: RunResult::Failure { error, cancelled: false, .. }, .. }]
            if error == failure
    ));
}

fn project_run(access: Access, supervised: bool) -> codex::Run {
    let mut r = run("p");
    r.schema = Some(json!({"type":"object","properties":{"reply":{"type":"string"}}}));
    r.project = Some(codex::Project { access, supervised });
    r
}
fn project_started(access: Access, supervised: bool) -> (codex::Client, Vec<Value>) {
    let mut c = codex::Client::default();
    let mut sent = c.run(project_run(access, supervised), 0).unwrap().outgoing;
    sent.extend(c.receive(json!({"id":1,"result":{}}), 0).outgoing);
    sent.extend(
        c.receive(json!({"id":2,"result":{"thread":{"id":"t"}}}), 0)
            .outgoing,
    );
    sent.extend(
        c.receive(json!({"id":3,"result":{"turn":{"id":"u"}}}), 0)
            .outgoing,
    );
    (c, sent)
}
fn sent<'a>(all: &'a [Value], method: &str) -> &'a Value {
    all.iter()
        .find(|m| m["method"] == method)
        .unwrap_or_else(|| panic!("{method} not sent: {all:?}"))
}
fn approval(c: &mut codex::Client, method: &str, id: u64) -> Effects {
    c.receive(
        json!({"id":id,"method":method,"params":{"threadId":"t","turnId":"u","itemId":"i","command":"curl example.test","reason":"needs network","permissions":{"network":{"enabled":true},"fileSystem":null}}}),
        1,
    )
}

#[test]
fn project_runs_use_the_app_server_with_the_workspace_sandbox() {
    let (_, all) = project_started(Access::Project, false);
    let thread = &sent(&all, "thread/start")["params"];
    assert_eq!(thread["approvalPolicy"], "on-request");
    assert_eq!(thread["approvalsReviewer"], "auto_review");
    assert!(thread.get("sandbox").is_none(), "{thread}");
    assert_eq!(thread["permissions"], "powermove");
    assert_eq!(thread["config"]["default_permissions"], "powermove");
    assert_eq!(
        thread["config"]["permissions"]["powermove"]["extends"],
        ":workspace"
    );
    assert_eq!(
        thread["config"]["permissions"]["powermove"]["network"]["enabled"],
        true
    );
    let turn = &sent(&all, "turn/start")["params"];
    assert_eq!(turn["approvalPolicy"], "on-request");
    assert!(turn.get("sandboxPolicy").is_none(), "{turn}");
    assert_eq!(turn["permissions"], "powermove");
    assert!(turn["outputSchema"].is_object());
}
#[test]
fn supervised_project_routes_escapes_to_the_user_and_full_access_is_unsandboxed() {
    let (_, all) = project_started(Access::Project, true);
    assert_eq!(
        sent(&all, "thread/start")["params"]["approvalsReviewer"],
        "user"
    );
    let (_, all) = project_started(Access::Computer, false);
    let thread = &sent(&all, "thread/start")["params"];
    assert_eq!(thread["approvalPolicy"], "never");
    assert_eq!(thread["sandbox"], "danger-full-access");
    assert_eq!(
        sent(&all, "turn/start")["params"]["sandboxPolicy"]["type"],
        "dangerFullAccess"
    );
}
#[test]
fn editor_runs_stay_read_only() {
    let mut c = codex::Client::default();
    let mut all = c.run(run("a"), 0).unwrap().outgoing;
    all.extend(c.receive(json!({"id":1,"result":{}}), 0).outgoing);
    let thread = &sent(&all, "thread/start")["params"];
    assert_eq!(thread["sandbox"], "read-only");
    assert!(thread.get("approvalsReviewer").is_none());
}
#[test]
fn project_steer_mid_turn_is_turn_steer_in_the_same_turn() {
    let (mut c, _) = project_started(Access::Project, false);
    let e = c.steer("p", "no, do it at 15:26", &[], 5).unwrap();
    let steer = sent(&e.outgoing, "turn/steer");
    assert_eq!(steer["params"]["expectedTurnId"], "u");
    assert_eq!(steer["params"]["threadId"], "t");
    assert_eq!(completed(&e), 0);
    // Rapid double steer: both reach the same turn, in order.
    let e = c.steer("p", "and keep it short", &[], 6).unwrap();
    assert_eq!(
        sent(&e.outgoing, "turn/steer")["params"]["input"][0]["text"],
        "and keep it short"
    );
    // The run ends once, with Project access.
    let done = c.receive(
        json!({"method":"turn/completed","params":{"threadId":"t","turn":{"id":"u","status":"completed"}}}),
        7,
    );
    assert_eq!(completed(&done), 1);
}
#[test]
fn project_steer_before_the_turn_id_is_queued_not_restarted() {
    let mut c = codex::Client::default();
    c.run(project_run(Access::Project, false), 0).unwrap();
    c.receive(json!({"id":1,"result":{}}), 0);
    c.receive(json!({"id":2,"result":{"thread":{"id":"t"}}}), 0);
    assert!(c.steer("p", "first", &[], 0).unwrap().outgoing.is_empty());
    let e = c.receive(json!({"id":3,"result":{"turn":{"id":"u"}}}), 0);
    assert_eq!(
        sent(&e.outgoing, "turn/steer")["params"]["input"][0]["text"],
        "first"
    );
}
#[test]
fn sandbox_escape_becomes_an_approval_card_and_the_answer_is_sent() {
    let (mut c, _) = project_started(Access::Project, true);
    let e = approval(&mut c, "item/commandExecution/requestApproval", 90);
    assert!(e.outgoing.is_empty(), "held for the user: {:?}", e.outgoing);
    let card = e
        .events
        .iter()
        .find_map(|ev| match ev {
            Event::Trace { value, .. } if value["kind"] == "question" => Some(value.clone()),
            _ => None,
        })
        .expect("approval card");
    assert!(
        card["questions"][0]["question"]
            .as_str()
            .unwrap()
            .contains("curl example.test")
    );
    let item = card["itemId"].as_str().unwrap().to_owned();
    let allow = BTreeMap::from([("decision".to_owned(), vec!["Allow once".to_owned()])]);
    let e = c.answer("p", &item, &allow).unwrap();
    assert_eq!(
        e.outgoing,
        vec![json!({"id":90,"result":{"decision":"accept"}})]
    );
    // Deny, and a permissions request, use their own response shapes.
    let e = approval(&mut c, "item/permissions/requestApproval", 91);
    let item = e
        .events
        .iter()
        .find_map(|ev| match ev {
            Event::Trace { value, .. } => value["itemId"].as_str().map(str::to_owned),
            _ => None,
        })
        .unwrap();
    let deny = BTreeMap::from([("decision".to_owned(), vec!["Deny".to_owned()])]);
    let e = c.answer("p", &item, &deny).unwrap();
    assert_eq!(
        e.outgoing,
        vec![json!({"id":91,"result":{"permissions":{},"scope":"turn"}})]
    );
    let e = approval(&mut c, "item/permissions/requestApproval", 92);
    let item = e
        .events
        .iter()
        .find_map(|ev| match ev {
            Event::Trace { value, .. } => value["itemId"].as_str().map(str::to_owned),
            _ => None,
        })
        .unwrap();
    let allow = BTreeMap::from([("decision".to_owned(), vec!["Allow for this run".to_owned()])]);
    let e = c.answer("p", &item, &allow).unwrap();
    assert_eq!(
        e.outgoing,
        vec![json!({"id":92,"result":{"permissions":{"network":{"enabled":true}},"scope":"turn"}})]
    );
    // "Allow for this run" settles the next request without a card.
    let e = approval(&mut c, "item/commandExecution/requestApproval", 93);
    assert_eq!(
        e.outgoing,
        vec![json!({"id":93,"result":{"decision":"accept"}})]
    );
}
#[test]
fn full_access_and_editor_runs_decline_escape_requests() {
    let (mut c, _) = project_started(Access::Computer, false);
    let e = approval(&mut c, "item/commandExecution/requestApproval", 90);
    assert_eq!(
        e.outgoing,
        vec![json!({"id":90,"result":{"decision":"decline"}})]
    );
    let mut c = started();
    let e = approval(&mut c, "item/fileChange/requestApproval", 91);
    assert_eq!(
        e.outgoing,
        vec![json!({"id":91,"result":{"decision":"decline"}})]
    );
}
#[test]
fn cancel_and_crash_settle_held_approvals_cleanly() {
    let (mut c, _) = project_started(Access::Project, true);
    approval(&mut c, "item/commandExecution/requestApproval", 90);
    let e = c.cancel("p", 2).unwrap();
    assert!(
        e.outgoing
            .contains(&json!({"id":90,"result":{"decision":"decline"}})),
        "{:?}",
        e.outgoing
    );
    let (mut c, _) = project_started(Access::Project, true);
    approval(&mut c, "item/commandExecution/requestApproval", 91);
    let e = c.stopped("Agent runtime exited without a result.");
    assert_eq!(completed(&e), 1);
    assert!(e.events.iter().any(|ev| matches!(
        ev,
        Event::Completed {
            result: RunResult::Failure { .. },
            ..
        }
    )));
}
#[test]
fn project_success_carries_its_access() {
    let (mut c, _) = project_started(Access::Project, false);
    c.receive(json!({"method":"item/completed","params":{"threadId":"t","turnId":"u","item":{"type":"agentMessage","id":"m","text":"{\"reply\":\"ok\"}"}}}), 1);
    let e = c.receive(json!({"method":"turn/completed","params":{"threadId":"t","turn":{"id":"u","status":"completed"}}}), 2);
    assert!(
        e.events.iter().any(|ev| matches!(
            ev,
            Event::Completed {
                result: RunResult::Success {
                    access: Access::Project,
                    ..
                },
                ..
            }
        )),
        "{:?}",
        e.events
    );
}
