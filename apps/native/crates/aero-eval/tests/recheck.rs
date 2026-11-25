//! Sol's targeted re-check (fix round 2), one named test per probe. Every
//! test here uses only interfaces the crate had at 4e0aacde2 (artifacts as
//! JSON, the suite, the stats, the stream parsers, the transport, the runner
//! with a script harness, the replay server), so it builds on that head and
//! fails there, and passes on this one. Panel observations carry both the
//! old single-probe fields and the per-probe ones: each head reads its own.
mod support;

use pm_aero_eval::artifacts::Artifacts;
use pm_aero_eval::graders::{Context, GraderSpec};
use pm_aero_eval::record::{RunRecord, Usage};
use pm_aero_eval::runner::harness::{
    AeroHarness, ClaudeParser, CodexParser, Harness, HarnessId, HostLink, Provider, RunContext,
    StreamParser,
};
use pm_aero_eval::runner::mcp::{Deadlines, McpClient, OnNote, RpcError};
use pm_aero_eval::runner::{self, Config, Slot};
use pm_aero_eval::stats::{self, Bar};
use pm_aero_eval::suite::{Limits, Suite, Task};
use pm_aero_eval::verdict::{Status, Verdict};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

const FAKE: &str = env!("CARGO_BIN_EXE_aero-eval-fake-host");
const CUT: &str = "cut-drop-flubbed-take";

// ------------------------------------------------------------ P1-2 graders

fn fixture(name: &str) -> Value {
    let p = pm_aero_eval::data_root()
        .join("fixtures/projects")
        .join(name);
    pm_aero_eval::pmv::read_document(&std::fs::read(p).unwrap()).unwrap()
}

fn set_layer(doc: &mut Value, name: &str, path: &str, v: Value) {
    let l = doc["proj"]["layers"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|l| l["name"] == name)
        .unwrap();
    *l.pointer_mut(path).unwrap() = v;
}

/// A run that created a panel extension `tool` with `panel`, a positive
/// owner spot-check, and the given panel observation.
fn panel_artifacts(project: &Value, panel: &str, obs: Value) -> Value {
    json!({
        "project_before": project, "project_after": project,
        "extensions": {
            "changed": [{"id": "tool", "action": "created"}],
            "windows": ["headless"],
            "health": [{"id": "tool", "window": "headless", "state": "ok"}],
            "registered": [{"kind": "panel", "id": panel, "extension": "tool", "params": 0}]
        },
        "panels": [obs],
        "spot_check": {"by": "owner", "pass": true, "note": "", "needs_fix": false}
    })
}

fn grade(task: &str, a: Value, run_dir: &Path) -> Vec<Verdict> {
    let a: Artifacts = serde_json::from_value(a).unwrap();
    let data = pm_aero_eval::data_root();
    let s = Suite::load(&data).unwrap();
    let t = s.task(task).unwrap();
    let cx = Context {
        artifacts: &a,
        run_dir,
        data_root: &data,
    };
    t.graders
        .iter()
        .map(|g: &GraderSpec| g.grade(&cx))
        .collect()
}

fn assert_passes(task: &str, a: Value, dir: &Path) {
    let v = grade(task, a, dir);
    assert!(
        v.iter().all(Verdict::passed),
        "{task}: the correct result failed: {v:#?}"
    );
}

fn assert_fails(task: &str, a: Value, dir: &Path, what: &str) {
    let v = grade(task, a, dir);
    assert!(
        v.iter()
            .any(|v| v.grader != "rubric" && v.status == Status::Fail),
        "{task}: {what} passed every programmatic grader: {v:#?}"
    );
}

/// Marker List as the panel shows it, as both heads read it: the old probe
/// (seek 0, click Drop) and one probe per row and per click.
fn marker_list(times: [&str; 3], jumps: [f64; 3]) -> Value {
    let doc = fixture("markers.pmv");
    let names = ["Intro", "Drop", "Outro"];
    let mut rows: Vec<String> = names
        .iter()
        .zip(times)
        .map(|(n, t)| format!("{n} {t}"))
        .collect();
    let text: Vec<String> = names
        .iter()
        .zip(times)
        .flat_map(|(n, t)| [n.to_string(), t.to_string()])
        .collect();
    rows.push(rows.join(" "));
    let click = |t: f64| {
        json!({"steps": ["ok", "ok"], "playhead": t,
                "project_before": doc, "project_after": doc})
    };
    let obs = json!({
        "panel": "tool.panel", "opened": true,
        // 4e0aacde2's single probe.
        "text": text, "steps": ["ok", "ok"], "playhead": jumps[1],
        "project_before": doc, "project_after": doc,
        // One probe per control.
        "probes": [{"text": text, "rows": rows}, click(jumps[0]), click(jumps[1]), click(jumps[2])]
    });
    panel_artifacts(&doc, "tool.panel", obs)
}

#[test]
fn recheck_marker_list_with_intro_and_outro_times_swapped_fails() {
    let t = tempfile::tempdir().unwrap();
    let task = "panel-marker-list";
    assert_passes(
        task,
        marker_list(["0:00.0", "0:02.5", "0:05.0"], [0.0, 2.5, 5.0]),
        t.path(),
    );
    // Sol: Intro and Outro show each other's times; Drop jumps correctly.
    assert_fails(
        task,
        marker_list(["0:05.0", "0:02.5", "0:00.0"], [0.0, 2.5, 5.0]),
        t.path(),
        "swapped Intro and Outro times",
    );
    // Every row is exercised: only Drop's click jumps.
    assert_fails(
        task,
        marker_list(["0:00.0", "0:02.5", "0:05.0"], [4.0, 2.5, 0.0]),
        t.path(),
        "only Drop jumps",
    );
}

/// Speed Buttons: what each press sets (None: the button is inert). The old
/// probe presses 50 % only; the new ones press 50 %, 200 % and 100 % on Clip
/// and 50 % with Clip 2 selected.
fn speed_buttons(set: [Option<f64>; 4]) -> Value {
    let doc = |clip: f64, clip2: f64| {
        let mut d = fixture("speed.pmv");
        set_layer(&mut d, "Clip", "/d/speed", json!(clip));
        set_layer(&mut d, "Clip 2", "/d/speed", json!(clip2));
        d
    };
    let mut s = (1.0, 1.0);
    let mut probes = Vec::new();
    for (i, v) in set.iter().enumerate() {
        let before = s;
        if let Some(v) = v {
            if i == 3 {
                s.1 = *v;
            } else {
                s.0 = *v;
            }
        }
        probes.push(json!({"steps": ["ok"], "text": ["50%", "100%", "200%"],
            "project_before": doc(before.0, before.1), "project_after": doc(s.0, s.1)}));
    }
    let first = probes[0].clone();
    let obs = json!({
        "panel": "tool.panel", "opened": true,
        "text": ["50%", "100%", "200%"], "steps": ["ok"],
        "project_before": first["project_before"], "project_after": first["project_after"],
        "probes": probes
    });
    panel_artifacts(&fixture("speed.pmv"), "tool.panel", obs)
}

#[test]
fn recheck_speed_buttons_with_only_50_percent_working_fails() {
    let t = tempfile::tempdir().unwrap();
    let task = "panel-speed-buttons";
    assert_passes(
        task,
        speed_buttons([Some(0.5), Some(2.0), Some(1.0), Some(0.5)]),
        t.path(),
    );
    // Sol: 50 % implemented, 100 % and 200 % inert.
    assert_fails(
        task,
        speed_buttons([Some(0.5), None, None, Some(0.5)]),
        t.path(),
        "only 50 % works",
    );
}

/// Shot Notes: typing into each layer's note field. `background_works`
/// false: only the Title row has a working note field (the old probe types
/// into Title only).
fn shot_notes(background_works: bool) -> Value {
    let doc = fixture("promo.pmv");
    let bg = if background_works {
        "Background plate wide plate"
    } else {
        "Background plate"
    };
    let step2 = if background_works {
        "ok".to_string()
    } else {
        "no control labelled \"Background plate\"".to_string()
    };
    let obs = json!({
        "panel": "shot-notes.panel", "opened": true,
        "text": ["Background plate", "Title", "hero shot"], "steps": ["ok"],
        "probes": [
            {"steps": ["ok"], "rows": ["Background plate", "Title hero shot"],
             "text": ["Background plate", "Title", "hero shot"]},
            {"steps": [step2], "rows": [bg, "Title hero shot"],
             "text": ["Background plate", "Title", "hero shot"]}
        ]
    });
    let mut a = panel_artifacts(&doc, "shot-notes.panel", obs);
    a["extensions"]["changed"] = json!([{"id": "shot-notes", "action": "updated"}]);
    a["extensions"]["health"][0]["id"] = json!("shot-notes");
    a["extensions"]["registered"][0]["extension"] = json!("shot-notes");
    a
}

#[test]
fn recheck_shot_notes_with_only_the_first_note_field_working_fails() {
    let t = tempfile::tempdir().unwrap();
    let task = "fix-shot-notes-crash";
    assert_passes(task, shot_notes(true), t.path());
    assert_fails(
        task,
        shot_notes(false),
        t.path(),
        "a note field on one layer only",
    );
}

fn png(dir: &Path, name: &str, rgb: [u8; 3]) -> String {
    std::fs::create_dir_all(dir.join("frames")).unwrap();
    let rel = format!("frames/{name}.png");
    std::fs::write(
        dir.join(&rel),
        pm_aero_eval::image::Rgba::solid(64, 36, rgb).encode_png(),
    )
    .unwrap();
    rel
}

/// Beat flash: taps at 2 s and 4 s. `follows_playhead` false: Tap always
/// drops its marker and flash at 2 s.
fn beat_flash(dir: &Path, follows_playhead: bool) -> Value {
    let with = |ts: &[f64]| {
        let mut d = fixture("promo.pmv");
        d["proj"]["markers"] = json!(
            ts.iter()
                .enumerate()
                .map(|(i, t)| json!({"id": format!("m{i}"), "t": t, "name": "Beat"}))
                .collect::<Vec<_>>()
        );
        d
    };
    let grey = [110, 110, 110];
    let mut n = 0;
    let mut frames = |at: f64, flash: f64| {
        let mut base = Vec::new();
        let mut now = Vec::new();
        for d in [-0.05, 0.0167, 0.05, 0.1167] {
            let t = ((at + d) * 10_000.0_f64).round() / 10_000.0;
            let lit = (flash..flash + 0.0667).contains(&t);
            n += 1;
            base.push(json!({"time": t, "png": png(dir, &format!("b{n}"), grey)}));
            now.push(json!({"time": t, "png": png(dir, &format!("a{n}"),
                if lit { [255, 255, 255] } else { grey })}));
        }
        (base, now)
    };
    let (b1, a1) = frames(2.0, 2.0);
    let second = if follows_playhead { 4.0 } else { 2.0 };
    let (b2, a2) = frames(4.0, second);
    let obs = json!({
        "panel": "tool.panel", "opened": true,
        "text": ["Tap"], "steps": ["ok", "ok"],
        "project_before": with(&[]), "project_after": with(&[2.0]),
        "baseline_frames": b1, "frames": a1,
        "probes": [
            {"steps": ["ok", "ok"], "project_before": with(&[]), "project_after": with(&[2.0]),
             "baseline_frames": b1, "frames": a1},
            {"steps": ["ok", "ok"], "project_before": with(&[2.0]),
             "project_after": with(&[2.0, if follows_playhead { 4.0 } else { 2.0001 }]),
             "baseline_frames": b2, "frames": a2}
        ]
    });
    panel_artifacts(&fixture("promo.pmv"), "tool.panel", obs)
}

#[test]
fn recheck_beat_flash_whose_tap_ignores_the_playhead_fails() {
    let t = tempfile::tempdir().unwrap();
    let task = "tool-beat-flash";
    assert_passes(task, beat_flash(t.path(), true), t.path());
    assert_fails(
        task,
        beat_flash(t.path(), false),
        t.path(),
        "Tap always at 2 s",
    );
}

/// Lower third kit: two adds (Ada Park, Colourist at 1 s; Ben Ode, Gaffer at
/// 3 s). `with_title` false: the typed title is dropped.
fn lower_third(with_title: bool) -> Value {
    let doc = fixture("promo.pmv");
    let slide = json!([{"t": 0.0, "v": -400}, {"t": 0.5, "v": 0}]);
    let third = |id: &str, from: f64, name: &str, title: &str| {
        let text = if with_title {
            format!("{name}\n{title}")
        } else {
            name.to_string()
        };
        json!({"id": id, "type": "text", "name": "Lower third", "from": from, "dur": 4, "on": true,
               "fx": [], "d": {"text": text}, "p": {"position.x": {"v": 0, "kf": slide}}})
    };
    let with = |ls: &[Value]| {
        let mut d = doc.clone();
        d["proj"]["layers"]
            .as_array_mut()
            .unwrap()
            .extend(ls.iter().cloned());
        d
    };
    let ada = third("lt1", 1.0, "Ada Park", "Colourist");
    let ben = third("lt2", 3.0, "Ben Ode", "Gaffer");
    let one = with(std::slice::from_ref(&ada));
    let two = with(&[ada, ben]);
    let steps = json!(["ok", "ok", "ok", "ok"]);
    let obs = json!({
        "panel": "tool.panel", "opened": true,
        "text": ["Name", "Title", "Add"], "steps": steps,
        "project_before": doc, "project_after": one,
        "probes": [
            {"steps": steps, "project_before": doc, "project_after": one},
            {"steps": steps, "project_before": one, "project_after": two}
        ]
    });
    panel_artifacts(&doc, "tool.panel", obs)
}

#[test]
fn recheck_lower_third_kit_that_drops_the_typed_title_fails() {
    let t = tempfile::tempdir().unwrap();
    let task = "tool-lower-third-kit";
    assert_passes(task, lower_third(true), t.path());
    assert_fails(task, lower_third(false), t.path(), "no title");
}

// ------------------------------------------------------------ P1-3 the bar

/// One live run record; batch position and start time follow `order`.
fn rec(harness: &str, task: &str, cohort: &str, rep: u32, order: u32, success: bool) -> RunRecord {
    RunRecord {
        format: "aero-eval-run".into(),
        suite_version: 1,
        task: task.into(),
        kind: "cut".into(),
        harness: harness.into(),
        model: "gpt-6.2-sol".into(),
        effort: "high".into(),
        mode: "live".into(),
        cohort: cohort.into(),
        ablation: vec![],
        run_id: format!("{cohort}-{order}"),
        rep,
        order,
        started_unix_ms: 1_000 + order as u64,
        wall_ms: Some(60_000),
        success,
        renders_correct: None,
        unavailable: false,
        tokens: Some(Usage {
            input: 50_000,
            cached: 10_000,
            output: 5_000,
        }),
        tool_calls: 5,
        retries: 0,
        gate_continuations: None,
        timed_out: false,
        over_step_limit: false,
        exit_code: Some(0),
        harness_completed: Some(true),
        stop_reason: None,
        replay_passed: None,
        needs_fix: None,
        verdicts: vec![],
        approved: Some("test batch".into()),
        replay: None,
        error: None,
    }
}

#[test]
fn recheck_bar_two_disjoint_cohorts_are_not_better() {
    // Aero in one batch, Codex in another: no shared, interleaved cohort.
    let mut runs = Vec::new();
    for rep in 0..5 {
        for (i, t) in ["a", "b"].iter().enumerate() {
            runs.push(rec("aero", t, "batch-1", rep, rep * 2 + i as u32, true));
            runs.push(rec("codex", t, "batch-2", rep, rep * 2 + i as u32, false));
        }
    }
    let b = &stats::summarize(&runs, 1).bar[0];
    assert_ne!(b.verdict, Bar::Better, "{b:#?}");
    assert_eq!(b.verdict, Bar::Unavailable, "{b:#?}");
}

#[test]
fn recheck_bar_equal_task_rates_with_lopsided_repeats_are_not_better() {
    // Both harnesses always pass `easy` and always fail `hard`, but Aero
    // repeats `easy` 50 times and `hard` 5, Codex the other way round
    // (91 % against 9 % pooled). One interleaved batch.
    let mut runs = Vec::new();
    let mut order = 0;
    for rep in 0..50 {
        let mut push = |h: &str, t: &str, ok: bool| {
            runs.push(rec(h, t, "batch-1", rep, order, ok));
            order += 1;
        };
        push("aero", "easy", true);
        push("codex", "hard", false);
        if rep < 5 {
            push("codex", "easy", true);
            push("aero", "hard", false);
        }
    }
    let b = &stats::summarize(&runs, 1).bar[0];
    assert_ne!(b.verdict, Bar::Better, "{b:#?}");
    assert_eq!(b.verdict, Bar::NotShown, "{b:#?}");
}

#[test]
fn recheck_codex_empty_usage_is_unavailable_not_zero() {
    let mut p = CodexParser::default();
    p.event(&json!({"type": "turn.completed", "usage": {}}));
    assert_eq!(p.outcome().usage, None);
    // One turn without counts makes the run's total unknown.
    let mut p = CodexParser::default();
    p.event(&json!({"type": "turn.completed", "usage": {"input_tokens": 10, "output_tokens": 2}}));
    p.event(&json!({"type": "turn.completed", "usage": {}}));
    assert_eq!(p.outcome().usage, None);
}

#[test]
fn recheck_claude_empty_usage_is_unavailable_not_zero() {
    let mut p = ClaudeParser::default();
    p.event(&json!({"type": "result", "subtype": "success", "result": "", "usage": {}}));
    assert_eq!(p.outcome().usage, None);
}

// ------------------------------------------------------------ runner setup

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
        cohort: "recheck".into(),
        deadlines: deadlines(),
        sandbox_read: vec![],
        sandbox_protect: vec![],
    };
    (t, cfg)
}

fn slot(task: &str, rep: u32) -> Slot {
    Slot {
        order: rep,
        rep,
        task: task.into(),
        harness: HarnessId::Codex,
    }
}

fn cut(cfg: &Config, marker: &str) -> (Suite, Task) {
    let suite = Suite::load(&cfg.data_root).unwrap();
    let mut t = suite.task(CUT).unwrap().clone();
    t.prompt = format!("{} {marker}", t.prompt);
    (suite, t)
}

fn replay(cfg: &mut Config, suite: &Suite, t: &Task, rep: u32) -> RunRecord {
    let rec = support::fake_recording("codex", &t.id, &cfg.model, &cfg.effort, &t.prompt);
    let path = cfg.out.with_file_name(format!("rec-{rep}.json"));
    rec.save(&path).unwrap();
    cfg.replay = Some(path);
    runner::run_slot(suite, t, &slot(&t.id, rep), cfg).unwrap()
}

fn artifacts(cfg: &Config, t: &Task, rep: u32) -> Value {
    let d = runner::run_dir(cfg, t, &slot(&t.id, rep));
    serde_json::from_slice(&std::fs::read(d.join("artifacts.json")).unwrap()).unwrap()
}

/// A stand-in for `codex exec` that finds the run's profile (after
/// `--add-dir`), hands the prompt to the fake host, runs `actions` with
/// `$profile` and `$run` set, and ends its turn.
fn script_harness(dir: &Path, actions: &str) -> PathBuf {
    let p = dir.join("harness.sh");
    std::fs::write(
        &p,
        format!(
            "#!/bin/sh\nprev=\"\"; profile=\"\"; last=\"\"\nfor a in \"$@\"; do\n  if [ \"$prev\" = \"--add-dir\" ]; then profile=\"$a\"; fi\n  prev=\"$a\"; last=\"$a\"\ndone\nrun=\"$(dirname \"$profile\")\"\nprintf '%s' \"$last\" > \"$profile/fake-prompt.txt\"\n{actions}\necho '{{\"type\":\"turn.completed\",\"usage\":{{\"input_tokens\":1,\"output_tokens\":1}}}}'\n"
        ),
    )
    .unwrap();
    std::fs::set_permissions(&p, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    p
}

// ------------------------------------------------------------ P1-4 isolation

#[test]
fn recheck_an_extension_root_that_is_a_symlink_is_never_walked() {
    let (t, mut cfg) = setup();
    // An extension outside the run, behind a link the harness plants as the
    // profile's whole extension root.
    let outside = t.path().join("owner-extensions");
    std::fs::create_dir_all(outside.join("stolen")).unwrap();
    std::fs::write(outside.join("stolen/index.ts"), "export default 1;").unwrap();
    cfg.harness_bin = Some(script_harness(
        t.path(),
        &format!("ln -s '{}' \"$profile/extensions\"", outside.display()),
    ));
    let (suite, task) = cut(&cfg, "");
    replay(&mut cfg, &suite, &task, 0);
    let a = artifacts(&cfg, &task, 0);
    let changed = a["extensions"]["changed"].as_array().unwrap();
    assert!(
        !changed.iter().any(|c| c["id"] == "stolen"),
        "the collector walked an extension root behind a link: {changed:?}"
    );
    assert!(
        changed.iter().any(|c| c["action"] == "refused"),
        "the linked root was not reported: {changed:?}"
    );
}

#[test]
fn recheck_a_saved_project_behind_a_symlinked_exports_folder_is_never_read() {
    let (t, mut cfg) = setup();
    // A project outside the run, reached through the run's exports folder
    // once the harness swaps that folder for a link.
    let outside = t.path().join("owner-projects");
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::copy(
        cfg.data_root.join("fixtures/projects/speed.pmv"),
        outside.join("stolen.pmv"),
    )
    .unwrap();
    cfg.harness_bin = Some(script_harness(
        t.path(),
        &format!(
            "rm -rf \"$run/exports\" && ln -s '{}' \"$run/exports\"",
            outside.display()
        ),
    ));
    let (suite, task) = cut(&cfg, "");
    let run = runner::run_dir(&cfg, &task, &slot(&task.id, 0));
    let named = std::path::absolute(&run)
        .unwrap()
        .join("exports/stolen.pmv");
    let (suite, task) = {
        let mut task = task;
        task.prompt = format!("{} [saved-outside {}]", task.prompt, named.display());
        (suite, task)
    };
    replay(&mut cfg, &suite, &task, 0);
    let a = artifacts(&cfg, &task, 0);
    assert!(
        a["project_after"].is_null(),
        "a project behind a linked exports folder was read: {}",
        a["project_after"]["proj"]["id"]
    );
    assert!(a["notes"].to_string().contains("symlink"), "{}", a["notes"]);
}

// ------------------------------------------------------------ P1-5 deadlines

#[test]
fn recheck_an_rpc_whose_write_blocks_stops_at_its_deadline() {
    // Sol: an RPC with a 50 ms deadline sat 310 ms in its write, until the
    // peer closed. The peer here never reads and closes after 310 ms.
    let (ours_w, peer_w) = std::os::unix::net::UnixStream::pair().unwrap();
    let (ours_r, peer_r) = std::os::unix::net::UnixStream::pair().unwrap();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(310));
        drop((peer_w, peer_r));
    });
    let mut c = McpClient::new(ours_r, ours_w);
    let big = "x".repeat(8 << 20);
    let started = Instant::now();
    let r = c.request_until(
        "tools/call",
        json!({"name": "apply_commands", "arguments": {"blob": big}}),
        Instant::now() + Duration::from_millis(50),
        &mut |_| None,
    );
    let took = started.elapsed();
    assert!(matches!(r, Err(RpcError::Timeout(_))), "{r:?}");
    assert!(took < Duration::from_millis(200), "{took:?}");
}

#[test]
fn recheck_cancelling_aero_on_a_host_that_stopped_reading_is_bounded() {
    // The host never reads its input: the aero/run request, its cancel and
    // the wait for an answer all end within the run's deadline plus grace.
    let (ours_w, peer_w) = std::os::unix::net::UnixStream::pair().unwrap();
    let (ours_r, peer_r) = std::os::unix::net::UnixStream::pair().unwrap();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_secs(8));
        drop((peer_w, peer_r));
    });
    let mut c = McpClient::new(ours_r, ours_w);
    let t = tempfile::tempdir().unwrap();
    let limits = Limits {
        wall_secs: 1,
        steps: 40,
    };
    let mut d = deadlines();
    d.grace = Duration::from_secs(1);
    let prompt = format!("Put a soft vignette on the plate. {}", "x".repeat(8 << 20));
    let transcript = t.path().join("t.ndjson");
    let cx = RunContext {
        prompt: &prompt,
        model: "gpt-6.2-sol",
        effort: "high",
        limits: &limits,
        workdir: t.path(),
        home: t.path(),
        profile: t.path(),
        mcp_command: Path::new("/opt/powermove-mcp"),
        mcp_args: vec![],
        bin: None,
        provider: Provider::Replay {
            base_url: "http://127.0.0.1:1".into(),
        },
        transcript: &transcript,
        ablate: &[],
        confine: None,
        deadlines: &d,
        run_id: "r1",
        project_id: "promo",
        revision: Some(1),
    };
    let started = Instant::now();
    let o = AeroHarness.run(&cx, &mut c);
    let took = started.elapsed();
    assert!(took < Duration::from_secs(4), "{took:?}");
    assert!(o.host_hung && !o.completed, "{o:?}");
}

#[test]
fn recheck_codex_counts_a_stalled_tool_call_when_it_starts() {
    let mut p = CodexParser::default();
    let item =
        json!({"id": "item_1", "type": "mcp_tool_call", "server": "powermove", "tool": "edit"});
    p.event(&json!({"type": "item.started", "item": item}));
    assert_eq!(p.steps(), 1, "a started call is a step");
    p.event(&json!({"type": "item.updated", "item": item}));
    p.event(&json!({"type": "item.completed", "item": item}));
    assert_eq!(p.steps(), 1, "once per call id");
}

#[test]
fn recheck_claude_counts_a_tool_call_when_its_block_starts() {
    let mut p = ClaudeParser::default();
    let block = json!({"type": "tool_use", "id": "tu_1", "name": "mcp__powermove__edit"});
    p.event(&json!({"type": "stream_event", "event": {"type": "content_block_start", "index": 1, "content_block": block}}));
    assert_eq!(p.steps(), 1, "a started call is a step");
    p.event(&json!({"type": "assistant", "message": {"content": [block]}}));
    assert_eq!(p.steps(), 1, "once per call id");
}

/// A host whose Aero starts tool calls that never finish.
struct Starts {
    events: Vec<Value>,
    sent: Vec<String>,
}

impl HostLink for Starts {
    fn request_until(
        &mut self,
        _: &str,
        p: Value,
        _: Instant,
        on: OnNote,
    ) -> Result<Value, RpcError> {
        for e in self.events.clone() {
            if let Some((m, _)) =
                on(&json!({"jsonrpc": "2.0", "method": "aero/event", "params": e}))
            {
                self.sent.push(m);
            }
        }
        Ok(json!({"runId": p["runId"], "finalText": "", "completed": false, "reason": "cancelled"}))
    }
    fn notify(&mut self, m: &str, _: Value) -> Result<(), String> {
        self.sent.push(m.into());
        Ok(())
    }
    fn resume(&mut self, _: Instant, _: OnNote) -> Result<Value, RpcError> {
        Err(RpcError::Protocol("nothing pending".into()))
    }
}

#[test]
fn recheck_aero_counts_a_tool_call_when_it_starts() {
    let limits = Limits {
        wall_secs: 60,
        steps: 2,
    };
    let t = tempfile::tempdir().unwrap();
    let transcript = t.path().join("t.ndjson");
    let d = deadlines();
    let cx = RunContext {
        prompt: "Put a soft vignette on the plate.",
        model: "gpt-6.2-sol",
        effort: "high",
        limits: &limits,
        workdir: t.path(),
        home: t.path(),
        profile: t.path(),
        mcp_command: Path::new("/opt/powermove-mcp"),
        mcp_args: vec![],
        bin: None,
        provider: Provider::Replay {
            base_url: "http://127.0.0.1:1".into(),
        },
        transcript: &transcript,
        ablate: &[],
        confine: None,
        deadlines: &d,
        run_id: "r1",
        project_id: "promo",
        revision: Some(1),
    };
    // Three calls start (one announced twice) and none finishes.
    let start = |seq: u64, id: &str| json!({"runId": "r1", "seq": seq, "event": {"type": "tool-started", "callId": id, "name": "edit"}});
    let mut host = Starts {
        events: vec![
            start(0, "c1"),
            start(1, "c1"),
            start(2, "c2"),
            start(3, "c3"),
        ],
        sent: vec![],
    };
    let o = AeroHarness.run(&cx, &mut host);
    assert_eq!(o.tool_calls, 3, "{o:?}");
    assert!(o.over_step_limit, "{o:?}");
    assert!(
        host.sent.iter().any(|m| m == "aero/cancel"),
        "{:?}",
        host.sent
    );
}

// ------------------------------------------------------------ network boundary

/// This Mac's IPv4 addresses other than loopback, and its global IPv6 ones.
fn host_addresses() -> (Vec<String>, Vec<String>) {
    let out = std::process::Command::new("/sbin/ifconfig")
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    let mut v4 = Vec::new();
    let mut v6 = Vec::new();
    for l in text.lines().map(str::trim) {
        let mut w = l.split_whitespace();
        match (w.next(), w.next()) {
            (Some("inet"), Some(a)) if !a.starts_with("127.") => v4.push(a.to_string()),
            (Some("inet6"), Some(a))
                if !a.contains('%') && a != "::1" && !a.starts_with("fe80") =>
            {
                v6.push(a.to_string())
            }
            _ => {}
        }
    }
    (v4, v6)
}

#[test]
fn boundary_a_replay_succeeds_and_cannot_reach_an_unrelated_loopback_listener() {
    let (_t, mut cfg) = setup();
    let other = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = other.local_addr().unwrap();
    let (suite, task) = cut(&cfg, &format!("[probe connect={addr}]"));
    let r = replay(&mut cfg, &suite, &task, 0);
    assert!(
        r.success,
        "the replay itself must succeed: {:?} {:#?}",
        r.error, r.verdicts
    );
    let d = runner::run_dir(&cfg, &task, &slot(&task.id, 0));
    let p: Value =
        serde_json::from_slice(&std::fs::read(d.join("home/probe.json")).unwrap()).unwrap();
    assert!(
        p["connect"].as_str().unwrap().contains("not permitted"),
        "a confined run reached {addr}: {p}"
    );
}

#[test]
fn boundary_a_replay_cannot_reach_a_listener_on_this_macs_own_address() {
    let (v4, _) = host_addresses();
    let ip = v4
        .first()
        .expect("this Mac has a non-loopback IPv4 address to test against");
    let (_t, mut cfg) = setup();
    let other = std::net::TcpListener::bind(format!("{ip}:0")).unwrap();
    let addr = other.local_addr().unwrap();
    let (suite, task) = cut(&cfg, &format!("[probe connect={addr}]"));
    let r = replay(&mut cfg, &suite, &task, 0);
    assert!(r.success, "{:?} {:#?}", r.error, r.verdicts);
    let d = runner::run_dir(&cfg, &task, &slot(&task.id, 0));
    let p: Value =
        serde_json::from_slice(&std::fs::read(d.join("home/probe.json")).unwrap()).unwrap();
    assert!(
        p["connect"].as_str().unwrap().contains("not permitted"),
        "a confined run reached {addr}: {p}"
    );
}

/// Bind a listening TCP socket at `ip:port` with the given options; the
/// error number when refused.
fn try_bind(ip: &str, port: u16, reuse_addr: bool, reuse_port: bool) -> Result<(), i32> {
    let ip: std::net::IpAddr = ip.parse().unwrap();
    // SAFETY: plain socket calls on a socket this function owns and closes.
    unsafe {
        let fam = if ip.is_ipv4() {
            libc::AF_INET
        } else {
            libc::AF_INET6
        };
        let s = libc::socket(fam, libc::SOCK_STREAM, 0);
        assert!(s >= 0);
        let one: libc::c_int = 1;
        let set = |opt: libc::c_int, level: libc::c_int| {
            libc::setsockopt(
                s,
                level,
                opt,
                &one as *const _ as *const libc::c_void,
                std::mem::size_of::<libc::c_int>() as libc::socklen_t,
            );
        };
        if reuse_addr {
            set(libc::SO_REUSEADDR, libc::SOL_SOCKET);
        }
        if reuse_port {
            set(libc::SO_REUSEPORT, libc::SOL_SOCKET);
        }
        let r = match ip {
            std::net::IpAddr::V4(a) => {
                let mut sa: libc::sockaddr_in = std::mem::zeroed();
                sa.sin_len = std::mem::size_of::<libc::sockaddr_in>() as u8;
                sa.sin_family = libc::AF_INET as libc::sa_family_t;
                sa.sin_port = port.to_be();
                sa.sin_addr.s_addr = u32::from(a).to_be();
                libc::bind(
                    s,
                    &sa as *const _ as *const libc::sockaddr,
                    std::mem::size_of::<libc::sockaddr_in>() as libc::socklen_t,
                )
            }
            std::net::IpAddr::V6(a) => {
                set(libc::IPV6_V6ONLY, libc::IPPROTO_IPV6);
                let mut sa: libc::sockaddr_in6 = std::mem::zeroed();
                sa.sin6_len = std::mem::size_of::<libc::sockaddr_in6>() as u8;
                sa.sin6_family = libc::AF_INET6 as libc::sa_family_t;
                sa.sin6_port = port.to_be();
                sa.sin6_addr.s6_addr = a.octets();
                libc::bind(
                    s,
                    &sa as *const _ as *const libc::sockaddr,
                    std::mem::size_of::<libc::sockaddr_in6>() as libc::socklen_t,
                )
            }
        };
        let r = if r == 0 { libc::listen(s, 1) } else { r };
        let e = std::io::Error::last_os_error().raw_os_error().unwrap_or(0);
        libc::close(s);
        if r == 0 { Ok(()) } else { Err(e) }
    }
}

#[test]
fn boundary_no_other_listener_can_take_the_replay_port() {
    let rec = support::fake_recording("codex", CUT, "fake-model", "high", "prompt");
    let server = pm_aero_eval::replay::replayer::ReplayServer::start(
        pm_aero_eval::replay::replayer::Replayer::new(rec, vec![]),
    )
    .unwrap();
    let port: u16 = server.origin().rsplit(':').next().unwrap().parse().unwrap();
    let (v4, v6) = host_addresses();
    assert!(!v4.is_empty(), "this Mac has a non-loopback address");
    let mut addrs = vec![
        "127.0.0.1".to_string(),
        "0.0.0.0".into(),
        "::1".into(),
        "::".into(),
    ];
    addrs.extend(v4);
    addrs.extend(v6);
    let mut taken = Vec::new();
    for a in &addrs {
        for (ra, rp) in [(false, false), (true, false), (false, true), (true, true)] {
            if try_bind(a, port, ra, rp).is_ok() {
                taken.push(format!("{a} (SO_REUSEADDR {ra}, SO_REUSEPORT {rp})"));
            }
        }
    }
    assert!(
        taken.is_empty(),
        "another listener took the replay port {port} on: {taken:#?}"
    );
    drop(server);
}
