//! The runner end to end against the fake host and fake CLIs (no app build,
//! no model): every run is a replay of a synthetic recording, confined by the
//! OS sandbox. Each review scenario is reproduced here: incomplete replays and
//! failed harnesses, isolation canaries, deadlines and process groups,
//! duplicate runs.
mod support;

use pm_aero_eval::record::{RunRecord, Usage};
use pm_aero_eval::runner::harness::{
    self, AeroHarness, ClaudeParser, CodexParser, Harness, HarnessId, HostLink, Provider,
    RunContext, StreamParser,
};
use pm_aero_eval::runner::mcp::{Deadlines, OnNote, RpcError};
use pm_aero_eval::runner::sandbox::Confinement;
use pm_aero_eval::runner::{self, Config, Slot};
use pm_aero_eval::suite::{Limits, Suite, Task};
use pm_aero_eval::verdict::Status;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

const FAKE: &str = env!("CARGO_BIN_EXE_aero-eval-fake-host");
const CLI: &str = env!("CARGO_BIN_EXE_aero-eval");
const CUT: &str = "cut-drop-flubbed-take";

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

fn deadlines() -> Deadlines {
    Deadlines {
        init: Duration::from_secs(10),
        call: Duration::from_secs(10),
        collect: Duration::from_secs(30),
        grace: Duration::from_secs(2),
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
        approved: None,
        replay: None,
        ablate: vec![],
        cohort: "test".into(),
        deadlines: deadlines(),
        sandbox_read: vec![],
        sandbox_protect: vec![],
    };
    (t, cfg)
}

fn slot(task: &str, harness: HarnessId, rep: u32) -> Slot {
    Slot {
        order: rep,
        rep,
        task: task.into(),
        harness,
    }
}

/// The cut task with an extra prompt marker and optional limits.
fn task(cfg: &Config, marker: &str, limits: Option<Limits>) -> (Suite, Task) {
    let suite = Suite::load(&cfg.data_root).unwrap();
    let mut t = suite.task(CUT).unwrap().clone();
    if !marker.is_empty() {
        t.prompt = format!("{} {marker}", t.prompt);
    }
    if let Some(l) = limits {
        t.limits = l;
    }
    (suite, t)
}

/// Replay `t` on `harness` from a recording made for the slot (the fake
/// harnesses send the prompt, so the recording is built for it).
fn replay(cfg: &mut Config, suite: &Suite, t: &Task, harness: HarnessId, rep: u32) -> RunRecord {
    let rec = support::fake_recording(harness.as_str(), &t.id, &cfg.model, &cfg.effort, &t.prompt);
    let path = cfg
        .out
        .with_file_name(format!("rec-{}-{rep}.json", harness.as_str()));
    rec.save(&path).unwrap();
    cfg.replay = Some(path);
    runner::run_slot(suite, t, &slot(&t.id, harness, rep), cfg).unwrap()
}

fn dir(cfg: &Config, t: &Task, harness: HarnessId, rep: u32) -> PathBuf {
    runner::run_dir(cfg, t, &slot(&t.id, harness, rep))
}

fn artifacts(d: &Path) -> Value {
    serde_json::from_slice(&std::fs::read(d.join("artifacts.json")).unwrap()).unwrap()
}

#[test]
fn replayed_cuts_through_every_harness_succeed_and_consume_the_recording() {
    let (_t, mut cfg) = setup();
    let (suite, t) = task(&cfg, "", None);
    for (rep, h) in [HarnessId::Codex, HarnessId::ClaudeCode, HarnessId::Aero]
        .into_iter()
        .enumerate()
    {
        let r = replay(&mut cfg, &suite, &t, h, rep as u32);
        assert_eq!(r.error, None, "{h:?}");
        assert!(r.success, "{h:?}: {:#?}", r.verdicts);
        assert_eq!(
            (r.mode.as_str(), r.replay_passed, r.harness_completed),
            ("replay", Some(true), Some(true))
        );
        assert_eq!(
            r.tokens,
            Some(Usage {
                input: 1000,
                cached: 400,
                output: 50
            }),
            "{h:?}"
        );
        assert_eq!(r.tool_calls, 1, "{h:?}");
        let report: Value = serde_json::from_slice(
            &std::fs::read(dir(&cfg, &t, h, rep as u32).join("replay-report.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(
            (report["consumed"].as_u64(), report["passed"].as_bool()),
            (Some(2), Some(true))
        );
    }
    let aero = runner::load_runs(&cfg.out)
        .unwrap()
        .into_iter()
        .find(|r| r.harness == "aero")
        .unwrap();
    assert_eq!(aero.gate_continuations, Some(1));
}

#[test]
fn a_harness_that_ignores_the_replay_fails_and_keeps_its_evidence() {
    // Review scenario: the fake completed the cut with none of the recorded
    // exchanges consumed, and the run said success.
    let (_t, mut cfg) = setup();
    for h in [HarnessId::Codex, HarnessId::Aero] {
        let marker = if h == HarnessId::Aero {
            "[aero-skip-replay]"
        } else {
            "[skip-replay]"
        };
        let (suite, t) = task(&cfg, marker, None);
        let r = replay(&mut cfg, &suite, &t, h, 0);
        assert_eq!(r.replay_passed, Some(false), "{h:?}");
        assert!(!r.success, "{h:?}");
        // The evidence stays: the cut itself grades as done.
        let d = dir(&cfg, &t, h, 0);
        assert!(artifacts(&d)["project_after"].is_object());
        assert_eq!(r.verdicts[0].status, Status::Pass, "{:#?}", r.verdicts[0]);
        let report: Value =
            serde_json::from_slice(&std::fs::read(d.join("replay-report.json")).unwrap()).unwrap();
        assert_eq!(report["consumed"], 0);
    }
}

#[test]
fn a_failed_exit_or_an_unfinished_aero_turn_is_not_a_success() {
    // Review scenarios: exit 17 after a correct cut; Aero `completed:false`.
    let (_t, mut cfg) = setup();
    let (suite, t) = task(&cfg, "[exit 17]", None);
    let r = replay(&mut cfg, &suite, &t, HarnessId::Codex, 0);
    assert_eq!((r.exit_code, r.harness_completed), (Some(17), Some(false)));
    assert_eq!(r.replay_passed, Some(true));
    assert!(!r.success);
    assert_eq!(r.stop_reason.as_deref(), Some("exited 17"));

    let (suite, t) = task(&cfg, "[aero-gate-fails]", None);
    let r = replay(&mut cfg, &suite, &t, HarnessId::Aero, 1);
    assert_eq!(r.harness_completed, Some(false));
    assert_eq!(r.stop_reason.as_deref(), Some("gate-exhausted"));
    assert!(!r.success);
}

#[test]
fn a_recording_for_another_slot_is_refused_before_anything_starts() {
    let (_t, mut cfg) = setup();
    let (suite, t) = task(&cfg, "", None);
    let mut rec = support::fake_recording("codex", CUT, "another-model", "high", &t.prompt);
    let path = cfg.out.with_file_name("other.json");
    rec.save(&path).unwrap();
    cfg.replay = Some(path.clone());
    let r = runner::run_slot(&suite, &t, &slot(CUT, HarnessId::Codex, 0), &cfg).unwrap();
    let e = r.error.unwrap();
    assert!(
        e.contains("does not match the slot") && e.contains("model another-model"),
        "{e}"
    );
    assert!(!dir(&cfg, &t, HarnessId::Codex, 0).join("profile").exists());
    // The harness and task are checked too.
    rec.model = "fake-model".into();
    rec.harness = "aero".into();
    rec.save(&path).unwrap();
    let r = runner::run_slot(&suite, &t, &slot(CUT, HarnessId::Codex, 1), &cfg).unwrap();
    assert!(r.error.unwrap().contains("harness aero"));
}

#[test]
fn the_cli_fails_a_failed_replay_and_no_component_inherits_the_environment() {
    let (t, cfg) = setup();
    let canary = "AERO_EVAL_CANARY";
    let value = "canary-7c1d9e";
    let run_cli = |data: &Path, out: &Path, rec: &Path| {
        std::process::Command::new(CLI)
            .args([
                "run",
                "--task",
                CUT,
                "--harness",
                "codex",
                "--model",
                "fake-model",
            ])
            .args([
                "--effort",
                "high",
                "--mcp-bin",
                FAKE,
                "--harness-bin",
                FAKE,
                "--cohort",
                "cli",
            ])
            .arg("--out")
            .arg(out)
            .arg("--replay")
            .arg(rec)
            .arg("--data")
            .arg(data)
            .env(canary, value)
            .env("OPENAI_API_KEY", "sk-canary-not-a-key")
            .output()
            .unwrap()
    };
    // A good replay exits 0.
    let (_, good) = task(&cfg, "", None);
    let rec = support::fake_recording("codex", CUT, "fake-model", "high", &good.prompt);
    let rec_path = t.path().join("good.json");
    rec.save(&rec_path).unwrap();
    let out = run_cli(&cfg.data_root, &t.path().join("good"), &rec_path);
    assert_eq!(
        out.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&out.stdout)
    );
    let d = t
        .path()
        .join("good")
        .join(CUT)
        .join("codex-fake-model-high-r0");
    for f in ["profile/host-env.txt", "home/env.txt"] {
        let env = std::fs::read_to_string(d.join(f)).unwrap();
        assert!(
            !env.contains(value) && !env.contains("sk-canary"),
            "{f} inherited: {env}"
        );
        for leaked in ["USER=", "SSH_AUTH_SOCK=", "LOGNAME="] {
            assert!(!env.lines().any(|l| l.starts_with(leaked)), "{f}: {leaked}");
        }
        let home = env.lines().find(|l| l.starts_with("HOME=")).unwrap();
        assert!(home.contains("codex-fake-model-high-r0/"), "{f}: {home}");
    }
    // A replay the harness ignores exits 1 (a regression gate rejects it).
    let mut suite: Value =
        serde_json::from_slice(&std::fs::read(cfg.data_root.join("suite.json")).unwrap()).unwrap();
    let cut = suite["tasks"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|t| t["id"] == CUT)
        .unwrap();
    let prompt = format!("{} [skip-replay]", cut["prompt"].as_str().unwrap());
    cut["prompt"] = json!(prompt);
    std::fs::write(cfg.data_root.join("suite.json"), suite.to_string()).unwrap();
    let rec = support::fake_recording("codex", CUT, "fake-model", "high", &prompt);
    rec.save(&rec_path).unwrap();
    let out = run_cli(&cfg.data_root, &t.path().join("bad"), &rec_path);
    assert_eq!(
        out.status.code(),
        Some(1),
        "{}",
        String::from_utf8_lossy(&out.stdout)
    );
    assert!(String::from_utf8_lossy(&out.stdout).contains("replay diverged or was incomplete"));
}

/// TEST-NET-1 (RFC 5737): an address of no machine, never this one.
const OUTSIDE: &str = "192.0.2.1:9";

fn probe(confine: Option<&Confinement>, args: &[&str]) -> Value {
    let out = pm_aero_eval::runner::sandbox::command(Path::new(FAKE), confine)
        .arg("probe")
        .args(args)
        .env_clear()
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).unwrap()
}

#[test]
fn the_os_sandbox_refuses_outside_reads_and_non_loopback_connections() {
    let t = tempfile::tempdir().unwrap();
    let (root, owner) = (t.path().join("run"), t.path().join("owner"));
    std::fs::create_dir_all(&root).unwrap();
    std::fs::create_dir_all(&owner).unwrap();
    let (secret, inside) = (owner.join("secret.txt"), root.join("ok.txt"));
    std::fs::write(&secret, "canary").unwrap();
    std::fs::write(&inside, "fine").unwrap();
    let c = Confinement::for_run(&root, &[Path::new(FAKE)], std::slice::from_ref(&owner)).unwrap();
    let s = |p: &Path| p.display().to_string();
    // The canary file outside the root: readable unconfined, refused confined.
    assert_eq!(probe(None, &["--read", &s(&secret)])["read"], "ok");
    let r = probe(Some(&c), &["--read", &s(&secret)]);
    assert!(r["read"].as_str().unwrap().starts_with("refused"), "{r}");
    assert_eq!(probe(Some(&c), &["--read", &s(&inside)])["read"], "ok");
    // Loopback is allowed.
    let lo = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = lo.local_addr().unwrap().to_string();
    assert_eq!(probe(Some(&c), &["--connect", &addr])["connect"], "ok");
    // Any other host is refused by the kernel before a packet leaves (EPERM,
    // not a timeout).
    let started = Instant::now();
    let r = probe(Some(&c), &["--connect", OUTSIDE]);
    assert!(
        r["connect"].as_str().unwrap().contains("not permitted"),
        "{r}"
    );
    assert!(started.elapsed() < Duration::from_secs(2));
    // The owner's home and /Users are always protected.
    let p = c.profile();
    assert!(
        p.contains("(deny network*)") && p.contains("(subpath \"/Users\")"),
        "{p}"
    );
    if let Some(h) = std::env::var_os("HOME") {
        let h = std::fs::canonicalize(h).unwrap();
        assert!(c.protected.contains(&h));
    }
}

#[test]
fn a_replay_run_is_confined_end_to_end() {
    let (t, mut cfg) = setup();
    let owner = t.path().join("owner");
    std::fs::create_dir_all(&owner).unwrap();
    let secret = owner.join("secret.txt");
    std::fs::write(&secret, "canary").unwrap();
    cfg.sandbox_protect = vec![owner.clone()];
    let (suite, task) = task(
        &cfg,
        &format!("[probe read={} connect={OUTSIDE}]", secret.display()),
        None,
    );
    let r = replay(&mut cfg, &suite, &task, HarnessId::Codex, 0);
    assert!(r.success, "{:?} {:#?}", r.error, r.verdicts);
    let p: Value = serde_json::from_slice(
        &std::fs::read(dir(&cfg, &task, HarnessId::Codex, 0).join("home/probe.json")).unwrap(),
    )
    .unwrap();
    assert!(p["read"].as_str().unwrap().starts_with("refused"), "{p}");
    assert!(
        p["connect"].as_str().unwrap().contains("not permitted"),
        "{p}"
    );
}

#[test]
fn paths_the_host_names_and_symlinks_are_never_read() {
    let (t, mut cfg) = setup();
    let outside = t.path().join("elsewhere.pmv");
    std::fs::copy(
        cfg.data_root.join("fixtures/projects/interview.pmv"),
        &outside,
    )
    .unwrap();
    for (rep, marker, want) in [
        (
            0,
            format!("[saved-outside {}]", outside.display()),
            "is not a file inside",
        ),
        (1, "[saved-symlink]".to_string(), "symlink"),
    ] {
        let (suite, task) = task(&cfg, &marker, None);
        let r = replay(&mut cfg, &suite, &task, HarnessId::Codex, rep);
        let a = artifacts(&dir(&cfg, &task, HarnessId::Codex, rep));
        assert!(a["project_after"].is_null(), "{marker}");
        let notes = a["notes"].to_string();
        assert!(notes.contains(want), "{marker}: {notes}");
        assert!(!r.success);
        assert_eq!(r.verdicts[0].status, Status::Unavailable);
    }
    // An extension folder that is a link: refused, unread, and a failure.
    let (suite, mut task) = task(&cfg, "[ext-symlink]", None);
    task.graders = suite.task("fix-softglow-compile").unwrap().graders.clone();
    let r = replay(&mut cfg, &suite, &task, HarnessId::Codex, 2);
    let a = artifacts(&dir(&cfg, &task, HarnessId::Codex, 2));
    assert!(
        a["extensions"]["changed"].to_string().contains("refused"),
        "{a}"
    );
    let load = r
        .verdicts
        .iter()
        .find(|v| v.grader == "extensions-load")
        .unwrap();
    assert_eq!(load.status, Status::Fail);
    assert!(
        load.evidence
            .iter()
            .any(|e| e.contains("evil contains a symlink")),
        "{load:?}"
    );
}

fn survivors(arg: &str) -> bool {
    std::process::Command::new("/usr/bin/pgrep")
        .args(["-f", &format!("^/bin/sleep {arg}$")])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

#[test]
fn a_deadline_stops_the_whole_process_group_at_once() {
    // Review scenario: a grandchild held stdout after its parent was killed.
    let (_t, mut cfg) = setup();
    let limits = Some(Limits {
        wall_secs: 1,
        steps: 60,
    });
    let (suite, t) = task(&cfg, "[holder]", limits.clone());
    let started = Instant::now();
    let r = replay(&mut cfg, &suite, &t, HarnessId::Codex, 0);
    let took = started.elapsed();
    assert!(r.timed_out && !r.success);
    assert!(took < Duration::from_secs(8), "{took:?}");
    assert!(!survivors("31.7"), "the grandchild survived");
    assert_eq!(r.error, None, "{:?}", r.error);

    // A parent that exits at once, leaving a grandchild on its output.
    let (suite, t) = task(&cfg, "[holder-exit]", None);
    let started = Instant::now();
    let r = replay(&mut cfg, &suite, &t, HarnessId::Codex, 1);
    assert!(
        started.elapsed() < Duration::from_secs(8),
        "{:?}",
        started.elapsed()
    );
    assert!(!survivors("32.3"), "the grandchild survived");
    assert!(!r.timed_out);
}

#[test]
fn steps_are_tool_calls_and_are_enforced_while_the_run_goes() {
    let limits = Some(Limits {
        wall_secs: 60,
        steps: 2,
    });
    let (_t, mut cfg) = setup();
    let (suite, t) = task(&cfg, "[steps 5]", limits.clone());
    let started = Instant::now();
    let r = replay(&mut cfg, &suite, &t, HarnessId::Codex, 0);
    assert!(
        started.elapsed() < Duration::from_secs(10),
        "{:?}",
        started.elapsed()
    );
    assert!(r.over_step_limit && !r.success);
    assert_eq!(r.stop_reason.as_deref(), Some("stopped after 2 steps"));

    // Aero: the runner cancels through the harness when the steps pass.
    let (suite, t) = task(&cfg, "[aero-steps 5]", limits);
    let r = replay(&mut cfg, &suite, &t, HarnessId::Aero, 1);
    assert!(r.over_step_limit && !r.success, "{r:?}");
    assert_eq!(r.stop_reason.as_deref(), Some("cancelled"));
}

#[test]
fn aero_is_cancelled_at_its_deadline_and_the_run_is_still_collected() {
    // Review scenario: Aero measured time only after a blocking RPC returned.
    let (_t, mut cfg) = setup();
    let (suite, t) = task(
        &cfg,
        "[aero-hang]",
        Some(Limits {
            wall_secs: 1,
            steps: 60,
        }),
    );
    let started = Instant::now();
    let r = replay(&mut cfg, &suite, &t, HarnessId::Aero, 0);
    assert!(
        started.elapsed() < Duration::from_secs(8),
        "{:?}",
        started.elapsed()
    );
    assert!(r.timed_out && !r.success);
    assert_eq!(r.stop_reason.as_deref(), Some("cancelled"));
    assert!(artifacts(&dir(&cfg, &t, HarnessId::Aero, 0))["project_after"].is_object());
}

#[test]
fn a_host_that_never_initialises_is_stopped_at_the_init_deadline() {
    let (t, mut cfg) = setup();
    let script = t.path().join("silent-host.sh");
    std::fs::write(&script, "#!/bin/sh\nexec /bin/sleep 33.1\n").unwrap();
    std::fs::set_permissions(&script, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    cfg.mcp_bin = script;
    cfg.deadlines.init = Duration::from_secs(1);
    let suite = Suite::load(&cfg.data_root).unwrap();
    let t = suite.task(CUT).unwrap();
    let started = Instant::now();
    let r = runner::run_slot(&suite, t, &slot(CUT, HarnessId::None, 0), &cfg).unwrap();
    assert!(
        started.elapsed() < Duration::from_secs(6),
        "{:?}",
        started.elapsed()
    );
    assert!(r.error.unwrap().contains("did not initialise"));
    assert!(!survivors("33.1"));
}

#[test]
fn a_duplicate_run_leaves_the_first_record_untouched() {
    // Review scenario: the refused second run overwrote the first run.json.
    let (_t, mut cfg) = setup();
    let (suite, t) = task(&cfg, "", None);
    let first = replay(&mut cfg, &suite, &t, HarnessId::Codex, 0);
    assert!(first.success);
    let path = dir(&cfg, &t, HarnessId::Codex, 0).join("run.json");
    let before = std::fs::read(&path).unwrap();
    let again = runner::run_slot(&suite, &t, &slot(CUT, HarnessId::Codex, 0), &cfg);
    assert!(again.unwrap_err().contains("left untouched"));
    assert_eq!(std::fs::read(&path).unwrap(), before);
}

#[test]
fn live_runs_are_refused_until_batch_credentials_are_approved() {
    let (_t, mut cfg) = setup();
    let err = runner::guard(HarnessId::Codex, &cfg).unwrap_err();
    assert!(err.contains("Chief's OK"), "{err}");
    cfg.approved = Some("the Chief, 2026-11-24".into());
    let err = runner::guard(HarnessId::Codex, &cfg).unwrap_err();
    assert!(err.contains("not approved yet"), "{err}");
    runner::guard(HarnessId::None, &cfg).unwrap();
    cfg.approved = Some("ok".into());
    assert!(
        runner::guard(HarnessId::None, &cfg).is_err(),
        "a two-letter note is not an approval"
    );
    cfg.approved = None;
    cfg.replay = Some("x.json".into());
    runner::guard(HarnessId::Aero, &cfg).unwrap();
}

#[test]
fn a_fix_task_seeds_its_broken_extension_and_regrades_with_a_spot_check() {
    let (_t, cfg) = setup();
    let suite = Suite::load(&cfg.data_root).unwrap();
    let task = suite.task("fix-softglow-compile").unwrap();
    let r = runner::run_slot(&suite, task, &slot(&task.id, HarnessId::None, 0), &cfg).unwrap();
    assert_eq!(r.mode, "none");
    let d = runner::run_dir(&cfg, task, &slot(&task.id, HarnessId::None, 0));
    assert!(d.join("profile/extensions/softglow/index.ts").exists());
    assert!(!r.success);
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
    runner::run_slot(&suite, task, &slot(&task.id, HarnessId::None, 0), &cfg).unwrap();
    let d = runner::run_dir(&cfg, task, &slot(&task.id, HarnessId::None, 0));
    std::fs::write(
        d.join("spot-check.json"),
        json!({"by": "owner", "pass": false, "note": "flat", "needs_fix": true}).to_string(),
    )
    .unwrap();
    let r = runner::regrade(&suite, &d, &cfg.data_root).unwrap();
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
fn graders_the_host_cannot_feed_say_what_it_lacks() {
    // The fake host has no panel probes, seek, paired timing or evaluator.
    let (_t, cfg) = setup();
    let suite = Suite::load(&cfg.data_root).unwrap();
    for (id, grader, lack) in [
        ("tool-beat-flash", "panel", "panel probes"),
        ("new-effect-halftone", "frame-time", "paired frame timing"),
        ("panel-marker-list", "rubric", "fresh evaluator"),
    ] {
        let task = suite.task(id).unwrap();
        let r = runner::run_slot(&suite, task, &slot(id, HarnessId::None, 0), &cfg).unwrap();
        let v = r.verdicts.iter().find(|v| v.grader == grader).unwrap();
        assert_eq!(v.status, Status::Unavailable, "{id}");
        assert!(
            v.evidence.iter().any(|e| e.contains(lack)),
            "{id}: {:?}",
            v.evidence
        );
    }
}

#[test]
fn a_missing_host_binary_is_an_error_not_a_crash() {
    let (_t, mut cfg) = setup();
    cfg.mcp_bin = PathBuf::from("/nonexistent/powermove-mcp");
    let suite = Suite::load(&cfg.data_root).unwrap();
    let task = suite.task("add-vignette").unwrap();
    let r = runner::run_slot(&suite, task, &slot(&task.id, HarnessId::None, 0), &cfg).unwrap();
    assert!(r.error.unwrap().contains("/nonexistent/powermove-mcp"));
    assert!(!r.success);
}

fn cx<'a>(
    limits: &'a Limits,
    dir: &'a Path,
    provider: Provider,
    d: &'a Deadlines,
) -> RunContext<'a> {
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
        provider,
        transcript: dir,
        ablate: &[],
        confine: None,
        deadlines: d,
        run_id: "r1",
        project_id: "promo",
        revision: Some(1),
    }
}

#[test]
fn replays_point_each_cli_at_the_loopback_endpoint_with_a_dummy_key() {
    let limits = Limits {
        wall_secs: 60,
        steps: 40,
    };
    let d = deadlines();
    let dir = Path::new("/tmp/run");
    let c = cx(
        &limits,
        dir,
        Provider::Replay {
            base_url: "http://127.0.0.1:5555".into(),
        },
        &d,
    );
    let argv = harness::codex_argv(&c);
    assert!(argv.iter().any(|a| a.contains("model_providers.replay") && a.contains("http://127.0.0.1:5555/v1")));
    assert!(argv.contains(&"model_provider=\"replay\"".to_string()));
    let env = harness::cli_env(HarnessId::ClaudeCode, &c);
    assert!(env.contains(&("ANTHROPIC_BASE_URL".into(), "http://127.0.0.1:5555".into())));
    assert!(env.contains(&("ANTHROPIC_API_KEY".into(), "replay-no-key".into())));
    // Turns are not steps: no turn cap is passed.
    assert!(!harness::claude_argv(&c).iter().any(|a| a == "--max-turns"));
    // Without a replay, no key of any kind is put in the environment.
    let c = cx(&limits, dir, Provider::Live, &d);
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
fn stream_parsers_count_usage_calls_retries_and_failures() {
    let mut p = CodexParser::default();
    assert_eq!(
        p.outcome().usage,
        None,
        "no usage until the stream reports it"
    );
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
            Some(Usage {
                input: 30,
                cached: 12,
                output: 5
            })
        )
    );
    assert_eq!(
        (o.final_text.as_str(), p.steps(), p.failed()),
        ("ok", 2, None)
    );
    p.event(&json!({"type": "turn.failed", "error": {"message": "quota"}}));
    assert_eq!(p.failed().as_deref(), Some("quota"));

    let mut p = ClaudeParser::default();
    let tu = |id: &str| json!({"type": "assistant", "message": {"content": [{"type": "text"}, {"type": "tool_use", "id": id}]}});
    for l in [
        tu("a"),
        tu("a"),
        tu("b"),
        json!({"type": "system", "subtype": "api_retry"}),
        json!({"type": "result", "subtype": "success", "result": "fin", "usage": {"input_tokens": 5, "cache_creation_input_tokens": 7, "cache_read_input_tokens": 11, "output_tokens": 3}}),
    ] {
        p.event(&l);
    }
    let o = p.outcome();
    assert_eq!(
        (o.tool_calls, o.retries, o.usage),
        (
            2,
            1,
            Some(Usage {
                input: 23,
                cached: 11,
                output: 3
            })
        )
    );
    assert_eq!(p.failed(), None);
    p.event(&json!({"type": "result", "subtype": "error_max_turns", "is_error": true}));
    assert_eq!(p.failed().as_deref(), Some("error_max_turns"));
}

struct NoAero;
impl HostLink for NoAero {
    fn request_until(
        &mut self,
        method: &str,
        _: Value,
        _: Instant,
        _: OnNote,
    ) -> Result<Value, RpcError> {
        Err(RpcError::Remote(
            -32601,
            format!("{method}: Method not found"),
        ))
    }
    fn notify(&mut self, _: &str, _: Value) -> Result<(), String> {
        Ok(())
    }
    fn resume(&mut self, _: Instant, _: OnNote) -> Result<Value, RpcError> {
        Err(RpcError::Protocol("nothing pending".into()))
    }
}

/// A host that answers aero/run with events from another run or out of order.
struct BadAero(Vec<Value>);
impl HostLink for BadAero {
    fn request_until(
        &mut self,
        _: &str,
        p: Value,
        _: Instant,
        on: OnNote,
    ) -> Result<Value, RpcError> {
        for e in &self.0 {
            on(&json!({"jsonrpc": "2.0", "method": "aero/event", "params": e}));
        }
        Ok(json!({"runId": p["runId"], "finalText": "", "completed": true, "reason": "completed"}))
    }
    fn notify(&mut self, _: &str, _: Value) -> Result<(), String> {
        Ok(())
    }
    fn resume(&mut self, _: Instant, _: OnNote) -> Result<Value, RpcError> {
        Err(RpcError::Protocol("nothing pending".into()))
    }
}

#[test]
fn the_aero_client_holds_the_host_to_the_wire_contract() {
    let limits = Limits {
        wall_secs: 60,
        steps: 40,
    };
    let t = tempfile::tempdir().unwrap();
    let transcript = t.path().join("t.ndjson");
    let d = deadlines();
    let mut c = cx(
        &limits,
        t.path(),
        Provider::Replay {
            base_url: "http://127.0.0.1:1".into(),
        },
        &d,
    );
    c.transcript = &transcript;
    let o = AeroHarness.run(&c, &mut NoAero);
    assert!(o.error.unwrap().contains("lanes A and B"));
    let call = |run: &str, seq: u64| json!({"runId": run, "seq": seq, "event": {"type": "tool-call", "name": "x", "ok": true}});
    let o = AeroHarness.run(&c, &mut BadAero(vec![call("r1", 0), call("other", 1)]));
    assert!(!o.completed && o.error.unwrap().contains("for run other"));
    let o = AeroHarness.run(&c, &mut BadAero(vec![call("r1", 0), call("r1", 2)]));
    assert!(!o.completed && o.error.unwrap().contains("no gaps"));
    let o = AeroHarness.run(&c, &mut BadAero(vec![call("r1", 0), call("r1", 1)]));
    assert!(o.completed && o.error.is_none() && o.usage.is_none());
    // A live run never starts without approved credentials.
    let c = cx(&limits, t.path(), Provider::Live, &d);
    assert!(
        AeroHarness
            .run(&c, &mut NoAero)
            .error
            .unwrap()
            .contains("not approved yet")
    );
}
