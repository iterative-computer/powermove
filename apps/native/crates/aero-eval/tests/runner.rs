//! The runner end to end against the fake host and fake CLIs (no app build,
//! no model), plus the harness adapters' argv, environment and stream parsing.
use pm_aero_eval::record::Usage;
use pm_aero_eval::runner::harness::{
    self, AeroHarness, ClaudeParser, CodexParser, Harness, HarnessId, HostLink, RunContext,
    StreamParser,
};
use pm_aero_eval::runner::{self, Config, Slot};
use pm_aero_eval::suite::{Limits, Suite};
use pm_aero_eval::verdict::Status;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

const FAKE: &str = env!("CARGO_BIN_EXE_aero-eval-fake-host");

fn copy(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for e in std::fs::read_dir(from).unwrap() {
        let e = e.unwrap();
        if e.path().is_dir() {
            copy(&e.path(), &to.join(e.file_name()));
        } else {
            std::fs::copy(e.path(), to.join(e.file_name())).unwrap();
        }
    }
}

fn setup() -> (tempfile::TempDir, Config) {
    let t = tempfile::tempdir().unwrap();
    let data = t.path().join("data");
    copy(&pm_aero_eval::data_root(), &data);
    let cfg = Config {
        data_root: data,
        mcp_bin: FAKE.into(),
        harness_bin: Some(FAKE.into()),
        out: t.path().join("out"),
        model: "fake-model".into(),
        effort: "high".into(),
        repeats: 1,
        approved: Some("unit test with fake CLIs; no model is reached".into()),
        replay: None,
        ablate: vec![],
    };
    (t, cfg)
}

fn slot(task: &str, harness: HarnessId, rep: u32) -> Slot {
    Slot {
        order: 0,
        rep,
        task: task.into(),
        harness,
    }
}

#[test]
fn a_correct_cut_through_codex_succeeds_and_is_recorded() {
    let (_t, cfg) = setup();
    let suite = Suite::load(&cfg.data_root).unwrap();
    let task = suite.task("cut-drop-flubbed-take").unwrap();
    let r = runner::run_slot(&suite, task, &slot(&task.id, HarnessId::Codex, 0), &cfg);
    assert_eq!(r.error, None);
    assert!(r.success, "{:#?}", r.verdicts);
    assert_eq!(
        r.tokens,
        Usage {
            input: 1000,
            cached: 400,
            output: 50
        }
    );
    assert_eq!(
        (r.tool_calls, r.retries, r.gate_continuations),
        (1, 1, None)
    );
    assert_eq!(
        r.approved.as_deref(),
        Some("unit test with fake CLIs; no model is reached")
    );
    let dir = cfg.out.join(&task.id).join("codex-fake-model-high-r0");
    for f in [
        "run.json",
        "artifacts.json",
        "transcript.ndjson",
        "home/env.txt",
    ] {
        assert!(dir.join(f).exists(), "{f}");
    }
    // The CLI saw an isolated home and nothing else from this environment.
    let env = std::fs::read_to_string(dir.join("home/env.txt")).unwrap();
    let home = std::path::absolute(dir.join("home")).unwrap();
    assert!(env.contains(&format!("HOME={}", home.display())), "{env}");
    assert!(env.contains(&format!("CODEX_HOME={}/codex", home.display())));
    for leaked in [
        "USER=",
        "SSH_AUTH_SOCK=",
        "OPENAI_API_KEY=",
        "ANTHROPIC_API_KEY=",
    ] {
        assert!(
            !env.lines().any(|l| l.starts_with(leaked)),
            "{leaked} leaked: {env}"
        );
    }
    // The seeded profile held the fixture as a file entry, untouched in data/.
    let pins = pm_aero_eval::suite::load_pins(&cfg.data_root).unwrap();
    assert_eq!(
        pm_aero_eval::suite::tree_hash(&cfg.data_root.join("fixtures/projects/interview.pmv"))
            .unwrap(),
        pins["projects/interview.pmv"]
    );
}

#[test]
fn doing_nothing_fails_with_evidence_and_claude_code_is_parsed() {
    let (_t, cfg) = setup();
    let suite = Suite::load(&cfg.data_root).unwrap();
    let task = suite.task("cut-drop-flubbed-take").unwrap();
    let none = runner::run_slot(&suite, task, &slot(&task.id, HarnessId::None, 0), &cfg);
    assert_eq!(none.error, None);
    assert!(!none.success);
    let ev = none.verdicts[0].evidence.join("\n");
    assert!(
        ev.contains("FAIL: the flubbed take is gone: 1 layers match"),
        "{ev}"
    );

    let claude = runner::run_slot(
        &suite,
        task,
        &slot(&task.id, HarnessId::ClaudeCode, 0),
        &cfg,
    );
    assert!(claude.success, "{:#?} {:?}", claude.verdicts, claude.error);
    assert_eq!(
        claude.tokens,
        Usage {
            input: 1000,
            cached: 600,
            output: 40
        }
    );
    assert_eq!(claude.tool_calls, 1);
    let argv: Vec<String> = serde_json::from_str(
        &std::fs::read_to_string(
            cfg.out
                .join(&task.id)
                .join("claude-code-fake-model-high-r0/home/argv.json"),
        )
        .unwrap(),
    )
    .unwrap();
    assert!(
        argv.windows(2)
            .any(|w| w[0] == "--max-turns" && w[1] == "60")
    );
    assert!(argv.contains(&"--strict-mcp-config".to_string()));
}

#[test]
fn aero_runs_inside_the_host_and_reports_gate_continuations() {
    let (_t, cfg) = setup();
    let suite = Suite::load(&cfg.data_root).unwrap();
    let task = suite.task("cut-drop-flubbed-take").unwrap();
    let r = runner::run_slot(&suite, task, &slot(&task.id, HarnessId::Aero, 0), &cfg);
    assert!(r.success, "{:#?} {:?}", r.verdicts, r.error);
    assert_eq!(
        r.tokens,
        Usage {
            input: 1500,
            cached: 1100,
            output: 50
        }
    );
    assert_eq!((r.tool_calls, r.gate_continuations), (2, Some(1)));
}

#[test]
fn the_time_limit_stops_a_hung_harness() {
    let (_t, cfg) = setup();
    let mut suite = Suite::load(&cfg.data_root).unwrap();
    let task = suite
        .tasks
        .iter_mut()
        .find(|t| t.id == "cut-tighten-takes")
        .unwrap();
    task.limits = Limits {
        wall_secs: 1,
        steps: 80,
    };
    let task = task.clone();
    let started = std::time::Instant::now();
    let r = runner::run_slot(&suite, &task, &slot(&task.id, HarnessId::Codex, 0), &cfg);
    assert!(started.elapsed().as_secs() < 20);
    assert!(r.timed_out && !r.success);
}

#[test]
fn real_model_runs_need_an_approval_and_runs_never_reuse_a_folder() {
    let (_t, mut cfg) = setup();
    cfg.approved = None;
    let err = runner::guard(HarnessId::Codex, &cfg).unwrap_err();
    assert!(err.contains("Chief's OK"), "{err}");
    runner::guard(HarnessId::None, &cfg).unwrap();
    cfg.replay = Some("x.json".into());
    runner::guard(HarnessId::Aero, &cfg).unwrap();
    cfg.replay = None;
    cfg.approved = Some("ok".into());
    assert!(
        runner::guard(HarnessId::Codex, &cfg).is_err(),
        "a two-letter note is not an approval"
    );

    let (_t, cfg) = setup();
    let suite = Suite::load(&cfg.data_root).unwrap();
    let task = suite.task("cut-drop-flubbed-take").unwrap();
    let s = slot(&task.id, HarnessId::None, 0);
    runner::run_slot(&suite, task, &s, &cfg);
    let again = runner::run_slot(&suite, task, &s, &cfg);
    assert!(again.error.unwrap().contains("clean folder"));
}

#[test]
fn a_fix_task_seeds_its_broken_extension_and_regrades_with_a_spot_check() {
    let (_t, cfg) = setup();
    let suite = Suite::load(&cfg.data_root).unwrap();
    let task = suite.task("fix-softglow-compile").unwrap();
    let r = runner::run_slot(&suite, task, &slot(&task.id, HarnessId::None, 0), &cfg);
    let dir = cfg.out.join(&task.id).join("none-fake-model-high-r0");
    assert!(dir.join("profile/extensions/softglow/index.ts").exists());
    assert!(!r.success);
    // Nothing was changed, so the fix is not there.
    let load = r
        .verdicts
        .iter()
        .find(|v| v.grader == "extensions-load")
        .unwrap();
    assert_eq!(load.status, Status::Fail);
    assert!(
        load.evidence
            .iter()
            .any(|e| e.contains("softglow was changed"))
    );

    // Grade-look task: a spot-check added later decides the rubric on regrade.
    let task = suite.task("grade-late-afternoon").unwrap();
    runner::run_slot(&suite, task, &slot(&task.id, HarnessId::None, 0), &cfg);
    let dir = cfg.out.join(&task.id).join("none-fake-model-high-r0");
    std::fs::write(
        dir.join("spot-check.json"),
        json!({"by": "owner", "pass": false, "note": "flat", "needs_fix": true}).to_string(),
    )
    .unwrap();
    let r = runner::regrade(&suite, &dir, &cfg.data_root).unwrap();
    let rubric = r.verdicts.iter().find(|v| v.grader == "rubric").unwrap();
    assert_eq!(rubric.status, Status::Fail);
    assert_eq!(r.needs_fix, Some(true));

    let runs = runner::load_runs(&cfg.out).unwrap();
    let s = pm_aero_eval::stats::summarize(&runs, suite.version);
    assert!(
        s.groups
            .iter()
            .any(|g| g.task == "all" && g.harness == "none" && g.runs == 2)
    );
}

#[test]
fn a_missing_host_binary_is_an_error_not_a_crash() {
    let (_t, mut cfg) = setup();
    cfg.mcp_bin = PathBuf::from("/nonexistent/powermove-mcp");
    let suite = Suite::load(&cfg.data_root).unwrap();
    let task = suite.task("add-vignette").unwrap();
    let r = runner::run_slot(&suite, task, &slot(&task.id, HarnessId::None, 0), &cfg);
    assert!(r.error.unwrap().contains("/nonexistent/powermove-mcp"));
    assert!(!r.success);
}

fn cx<'a>(limits: &'a Limits, dir: &'a Path, base: Option<String>) -> RunContext<'a> {
    RunContext {
        prompt: "Put a soft vignette on the plate.",
        model: "gpt-6.2-sol",
        effort: "high",
        limits,
        workdir: dir,
        home: dir,
        profile: dir,
        mcp_command: Path::new("/opt/powermove-mcp"),
        mcp_args: vec!["--powermove-mcp".into(), "--profile".into(), "/p".into()],
        bin: None,
        model_base_url: base,
        transcript: dir,
        ablate: &[],
    }
}

#[test]
fn replays_point_each_cli_at_the_loopback_endpoint_with_a_dummy_key() {
    let limits = Limits {
        wall_secs: 60,
        steps: 40,
    };
    let dir = Path::new("/tmp/run");
    let c = cx(&limits, dir, Some("http://127.0.0.1:5555".into()));
    let argv = harness::codex_argv(&c);
    assert!(argv.iter().any(|a| a.contains("model_providers.replay") && a.contains("http://127.0.0.1:5555/v1")));
    assert!(argv.contains(&"model_provider=\"replay\"".to_string()));
    assert!(argv.contains(&"mcp_servers.powermove.command=\"/opt/powermove-mcp\"".to_string()));
    let env = harness::cli_env(HarnessId::ClaudeCode, &c);
    assert!(env.contains(&("ANTHROPIC_BASE_URL".into(), "http://127.0.0.1:5555".into())));
    assert!(env.contains(&("ANTHROPIC_API_KEY".into(), "replay-no-key".into())));
    // Without a replay, no key of any kind is put in the environment.
    let c = cx(&limits, dir, None);
    for id in [HarnessId::Codex, HarnessId::ClaudeCode] {
        assert!(
            harness::cli_env(id, &c)
                .iter()
                .all(|(k, _)| !k.ends_with("_KEY") && !k.ends_with("_URL"))
        );
    }
    assert!(
        !harness::codex_argv(&c)
            .iter()
            .any(|a| a.contains("model_provider"))
    );
}

#[test]
fn stream_parsers_count_usage_calls_and_retries() {
    let mut p = CodexParser::default();
    for l in [
        json!({"type": "item.completed", "item": {"type": "reasoning"}}),
        json!({"type": "item.completed", "item": {"type": "mcp_tool_call"}}),
        json!({"type": "item.completed", "item": {"type": "command_execution"}}),
        json!({"type": "error", "message": "Reconnecting... 2/5"}),
        json!({"type": "error", "message": "stream closed"}),
        json!({"type": "turn.completed", "usage": {"input_tokens": 10, "cached_input_tokens": 4, "output_tokens": 2}}),
        json!({"type": "turn.completed", "usage": {"input_tokens": 20, "cached_input_tokens": 8, "output_tokens": 3}}),
        json!({"type": "item.completed", "item": {"type": "agent_message", "text": "ok"}}),
    ] {
        p.event(&l);
    }
    let o = p.outcome();
    assert_eq!(
        (o.tool_calls, o.retries, o.usage),
        (
            2,
            1,
            Usage {
                input: 30,
                cached: 12,
                output: 5
            }
        )
    );
    assert_eq!(o.final_text, "ok");

    let mut p = ClaudeParser::default();
    let tu = |id: &str| json!({"type": "assistant", "message": {"content": [{"type": "text"}, {"type": "tool_use", "id": id}]}});
    for l in [
        tu("a"),
        tu("a"),
        tu("b"),
        json!({"type": "system", "subtype": "api_retry"}),
        json!({"type": "result", "result": "fin", "usage": {"input_tokens": 5, "cache_creation_input_tokens": 7, "cache_read_input_tokens": 11, "output_tokens": 3}}),
    ] {
        p.event(&l);
    }
    let o = p.outcome();
    assert_eq!(
        (o.tool_calls, o.retries, o.usage),
        (
            2,
            1,
            Usage {
                input: 23,
                cached: 11,
                output: 3
            }
        )
    );
}

struct NoAero;
impl HostLink for NoAero {
    fn request(&mut self, method: &str, _: Value) -> Result<Value, String> {
        Err(format!("{method}: Method not found (-32601)"))
    }
    fn take_notifications(&mut self) -> Vec<Value> {
        Vec::new()
    }
}

#[test]
fn until_lanes_a_and_b_land_the_aero_adapter_says_so() {
    let limits = Limits {
        wall_secs: 60,
        steps: 40,
    };
    let t = tempfile::tempdir().unwrap();
    let transcript = t.path().join("t.ndjson");
    let mut c = cx(&limits, t.path(), None);
    c.transcript = &transcript;
    let err = AeroHarness.run(&c, &mut NoAero).unwrap_err();
    assert!(err.contains("lanes A and B"), "{err}");
}
