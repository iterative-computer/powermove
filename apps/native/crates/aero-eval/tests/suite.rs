//! The committed suite, its fixtures and the statistics.
use pm_aero_eval::record::{RunRecord, Usage};
use pm_aero_eval::runner::harness::HarnessId;
use pm_aero_eval::stats::{self, Z90};
use pm_aero_eval::suite::{Kind, Suite, prompt_problems, tool_names};
use std::collections::BTreeMap;

fn catalog() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../mcp/fixtures/catalog-v1.2.0.json")
}

#[test]
fn the_committed_suite_passes_every_check() {
    let data = pm_aero_eval::data_root();
    let suite = Suite::load(&data).unwrap();
    let problems = suite.validate(&data, &tool_names(&catalog()).unwrap());
    assert!(problems.is_empty(), "{problems:#?}");
    let mut per: BTreeMap<Kind, usize> = BTreeMap::new();
    for t in &suite.tasks {
        *per.entry(t.kind).or_default() += 1;
    }
    for k in Kind::ALL {
        assert!(per[&k] >= 2, "{k:?}");
    }
    assert_eq!(suite.tasks.len(), 19);
}

#[test]
fn a_changed_fixture_or_a_hinting_prompt_is_caught() {
    let t = tempfile::tempdir().unwrap();
    let data = pm_aero_eval::data_root();
    copy(&data, t.path());
    // Edit a fixture without regenerating the pins.
    let p = t.path().join("fixtures/extensions/mono/index.ts");
    let mut s = std::fs::read_to_string(&p).unwrap();
    s.push_str("// tweak\n");
    std::fs::write(&p, s).unwrap();
    let mut suite = Suite::load(t.path()).unwrap();
    suite.tasks[0].prompt =
        "Use edit_layer_strips to delete Take 2 and register a manifest.".into();
    suite
        .tasks
        .retain(|t| t.kind != Kind::Restyle || t.id == "restyle-projector");
    let problems = suite
        .validate(t.path(), &tool_names(&catalog()).unwrap())
        .join("\n");
    assert!(
        problems.contains("fixture extensions/mono changed"),
        "{problems}"
    );
    assert!(
        problems.contains("names the tool edit_layer_strips"),
        "{problems}"
    );
    assert!(problems.contains("\"register\""), "{problems}");
    assert!(problems.contains("\"manifest\""), "{problems}");
    assert!(problems.contains("kind restyle has 1 tasks"), "{problems}");
}

fn copy(from: &std::path::Path, to: &std::path::Path) {
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

#[test]
fn prompts_read_like_a_person() {
    assert!(prompt_problems("Put a soft vignette on the plate.", &[]).is_empty());
    assert!(!prompt_problems("Write GLSL for a uniform blur", &[]).is_empty());
    assert!(!prompt_problems("call render_frames", &["render_frames".into()]).is_empty());
    assert!(!prompt_problems(&"long ".repeat(100), &[]).is_empty());
}

#[test]
fn wilson_intervals_match_known_values() {
    // 8 of 10 at 90 %: Wilson [0.5408, 0.9314].
    let i = stats::wilson(8, 10, Z90);
    assert!(
        (i.lo - 0.5408).abs() < 1e-3 && (i.hi - 0.9314).abs() < 1e-3,
        "{i:?}"
    );
    let i = stats::wilson(0, 5, Z90);
    assert!(i.lo.abs() < 1e-12, "{i:?}");
    assert!((i.hi - 0.3511).abs() < 1e-3, "{i:?}");
    let i = stats::wilson(5, 5, Z90);
    assert_eq!(i.hi, 1.0);
}

#[test]
fn median_intervals_use_order_statistics() {
    // n = 5 at 90 %: [x1, x5] (coverage 93.75 %).
    let i = stats::median_interval(&[5.0, 1.0, 3.0, 2.0, 4.0], 0.90).unwrap();
    assert_eq!((i.estimate, i.lo, i.hi), (3.0, Some(1.0), Some(5.0)));
    assert!((i.coverage.unwrap() - 0.9375).abs() < 1e-12);
    // n = 10 at 90 %: [x2, x9] (coverage 97.9 %; [x3, x8] covers only 89 %).
    let v: Vec<f64> = (1..=10).map(f64::from).collect();
    let i = stats::median_interval(&v, 0.90).unwrap();
    assert_eq!((i.estimate, i.lo, i.hi), (5.5, Some(2.0), Some(9.0)));
    // n = 20: [x6, x15] (coverage 95.9 %).
    let v: Vec<f64> = (1..=20).map(f64::from).collect();
    let i = stats::median_interval(&v, 0.90).unwrap();
    assert_eq!((i.lo, i.hi), (Some(6.0), Some(15.0)));
    assert!(stats::median_interval(&[], 0.9).is_none());
}

#[test]
fn median_bounds_stay_exact_for_large_batches_and_are_unavailable_below_the_minimum() {
    // Review scenario: 1,024 observations 0..1023 gave [476, 547] (the
    // binomial coefficients overflowed); the exact 90 % interval is [485, 538].
    let v: Vec<f64> = (0..1024).map(f64::from).collect();
    let i = stats::median_interval(&v, 0.90).unwrap();
    assert_eq!((i.lo, i.hi), (Some(485.0), Some(538.0)));
    assert!(
        i.coverage.unwrap() >= 0.90 && i.coverage.unwrap() < 0.91,
        "{i:?}"
    );
    // Much larger still works.
    let v: Vec<f64> = (0..100_000).map(f64::from).collect();
    let i = stats::median_interval(&v, 0.90).unwrap();
    assert!(
        i.lo.unwrap() > 49_700.0 && i.hi.unwrap() < 50_300.0,
        "{i:?}"
    );
    // Four observations cannot reach 90 % (the range covers 87.5 %): no bounds.
    let i = stats::median_interval(&[1.0, 2.0, 3.0, 4.0], 0.90).unwrap();
    assert_eq!(
        (i.estimate, i.lo, i.hi, i.coverage),
        (2.5, None, None, None)
    );
}

struct Run {
    harness: &'static str,
    task: &'static str,
    success: bool,
    wall: Option<u64>,
    tokens: Option<u64>,
    mode: &'static str,
    ablation: Vec<String>,
}

fn run(harness: &'static str, task: &'static str, success: bool, wall: u64, tokens: u64) -> Run {
    Run {
        harness,
        task,
        success,
        wall: Some(wall),
        tokens: Some(tokens),
        mode: "live",
        ablation: vec![],
    }
}

static NEXT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

impl Run {
    fn record(self) -> RunRecord {
        let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        RunRecord {
            format: "aero-eval-run".into(),
            suite_version: 1,
            task: self.task.into(),
            kind: "cut".into(),
            harness: self.harness.into(),
            model: "gpt-6.2-sol".into(),
            effort: "high".into(),
            mode: self.mode.into(),
            cohort: "c1".into(),
            ablation: self.ablation,
            run_id: format!("run-{n}"),
            rep: 0,
            order: n,
            started_unix_ms: 0,
            wall_ms: self.wall,
            success: self.success,
            renders_correct: None,
            unavailable: false,
            tokens: self.tokens.map(|t| Usage {
                input: t,
                cached: t / 2,
                output: t / 10,
            }),
            tool_calls: 5,
            retries: 0,
            gate_continuations: (self.harness == "aero").then_some(1),
            timed_out: false,
            over_step_limit: false,
            exit_code: Some(0),
            harness_completed: Some(true),
            stop_reason: None,
            replay_passed: None,
            needs_fix: None,
            verdicts: vec![],
            approved: Some("test".into()),
            replay: None,
            error: None,
        }
    }
}

/// `n` runs per task per harness over `tasks`; counting each harness's runs
/// across tasks, Aero succeeds at `a` of every 20 and Codex at `c`.
fn batch(tasks: &[&'static str], n: u32, a: u32, c: u32) -> Vec<RunRecord> {
    let mut out = Vec::new();
    let mut k = 0;
    for t in tasks {
        for _ in 0..n {
            out.push(run("aero", t, k % 20 < a, 60_000, 50_000).record());
            out.push(run("codex", t, k % 20 < c, 90_000, 80_000).record());
            k += 1;
        }
    }
    out
}

use stats::Bar;

#[test]
fn the_bar_needs_separated_intervals_and_no_worse_cost() {
    let runs = batch(&["a", "b", "c", "d"], 5, 19, 8);
    let s = stats::summarize(&runs, 1);
    assert_eq!(s.bar.len(), 1);
    assert_eq!(s.bar[0].verdict, Bar::Better, "{:#?}", s.bar[0]);
    // Same success, or more tokens: not better.
    let mut runs2: Vec<RunRecord> = runs
        .iter()
        .filter(|r| r.harness == "codex")
        .cloned()
        .collect();
    for t in ["a", "b", "c", "d"] {
        for i in 0..5 {
            runs2.push(run("aero", t, i < 19, 60_000, 90_000).record());
        }
    }
    let s = stats::summarize(&runs2, 1);
    assert_eq!(s.bar[0].verdict, Bar::NotShown);
    assert!(
        s.bar[0].comparisons[0]
            .reasons
            .iter()
            .any(|r| r.contains("tokens") && r.contains("worse"))
    );
    // Five runs a side cannot separate 100 % from 60 %.
    let close = batch(&["a"], 5, 20, 3);
    assert_eq!(stats::summarize(&close, 1).bar[0].verdict, Bar::NotShown);
}

#[test]
fn the_bar_refuses_incomparable_samples() {
    // Review scenario: three Aero successes on one cut against three Codex
    // failures on another effect, replay mixed with live, gave BETTER.
    let mut runs = Vec::new();
    for _ in 0..3 {
        runs.push(run("aero", "cut", true, 1, 1).record());
        runs.push(run("codex", "effect", false, 1, 1).record());
        let mut r = run("aero", "cut", true, 1, 1);
        r.mode = "replay";
        runs.push(r.record());
    }
    let b = &stats::summarize(&runs, 1).bar[0];
    assert_eq!(b.verdict, Bar::Unavailable, "{b:#?}");
    let why = b.comparisons[0].reasons.join("\n");
    assert!(
        why.contains("coverage differs") && why.contains("fewer than 5"),
        "{why}"
    );

    // Matched tasks, but too few repeats on one side.
    let mut runs = batch(&["a", "b"], 5, 20, 0);
    let mut kept = 0;
    runs.retain(|r| {
        if r.harness == "codex" && r.task == "b" {
            kept += 1;
            kept <= 3
        } else {
            true
        }
    });
    assert_eq!(stats::summarize(&runs, 1).bar[0].verdict, Bar::Unavailable);

    // A run without usage telemetry is unavailable, never zero cost.
    let mut runs = batch(&["a"], 20, 20, 0);
    let mut r = run("aero", "a", true, 1, 1);
    r.tokens = None;
    runs.push(r.record());
    let b = &stats::summarize(&runs, 1).bar[0];
    assert_eq!(b.verdict, Bar::Unavailable);
    assert!(
        b.comparisons[0]
            .reasons
            .iter()
            .any(|r| r.contains("no observed"))
    );
    let g = stats::summarize(&runs, 1)
        .groups
        .into_iter()
        .find(|g| g.harness == "aero" && g.task == "all")
        .unwrap();
    assert_eq!((g.runs, g.tokens_unobserved), (21, 1));

    // Ablations, replays and `none` never enter the bar, and never pool.
    let mut runs = batch(&["a"], 20, 20, 0);
    for mode in ["replay", "none"] {
        let mut r = run(
            if mode == "none" { "none" } else { "aero" },
            "a",
            false,
            1,
            1,
        );
        r.mode = mode;
        runs.push(r.record());
    }
    let mut r = run("aero", "a", false, 1, 1);
    r.ablation = vec!["completion-gate".into()];
    runs.push(r.record());
    let s = stats::summarize(&runs, 1);
    assert_eq!(s.bar[0].verdict, Bar::Better, "{:#?}", s.bar[0]);
    assert_eq!(s.bar[0].comparisons.len(), 1, "none is never a comparator");
    let aero: Vec<_> = s
        .groups
        .iter()
        .filter(|g| g.harness == "aero" && g.task == "all")
        .collect();
    assert_eq!(aero.len(), 3, "live, replay and the ablation stay apart");
    // Duplicated records count once.
    let mut dup = batch(&["a"], 5, 5, 0);
    dup.push(dup[0].clone());
    let s = stats::summarize(&dup, 1);
    assert_eq!(s.excluded.len(), 1);
}

#[test]
fn one_verdict_per_model_needs_every_comparison() {
    let mut runs = batch(&["a"], 20, 20, 0);
    // Claude Code on the same model beats Aero: the model verdict fails.
    for i in 0..20 {
        runs.push(run("claude-code", "a", i < 20, 10_000, 10_000).record());
    }
    let s = stats::summarize(&runs, 1);
    assert_eq!(s.bar.len(), 1);
    assert_eq!(s.bar[0].verdict, Bar::NotShown);
    assert_eq!(s.bar[0].comparisons.len(), 2);
    // Aero alone is not shown better than nothing.
    let alone: Vec<RunRecord> = (0..5)
        .map(|_| run("aero", "a", true, 1, 1).record())
        .collect();
    assert_eq!(stats::summarize(&alone, 1).bar[0].verdict, Bar::Unavailable);
}

#[test]
fn summaries_exclude_other_suite_versions_and_group_by_task() {
    let mut runs = vec![
        run("aero", "a", true, 1, 1).record(),
        run("aero", "b", false, 1, 1).record(),
    ];
    let mut old = run("aero", "a", true, 1, 1).record();
    old.suite_version = 0;
    runs.push(old);
    let s = stats::summarize(&runs, 1);
    assert_eq!(s.excluded.len(), 1);
    let all = s.groups.iter().find(|g| g.task == "all").unwrap();
    assert_eq!((all.runs, all.successes), (2, 1));
    assert_eq!(all.gate_continuations_mean, Some(1.0));
    assert!(s.groups.iter().any(|g| g.task == "a") && s.groups.iter().any(|g| g.task == "b"));
    assert!(stats::render(&s).contains("excluded"));
}

#[test]
fn batches_interleave_and_rotate_harness_order() {
    let data = pm_aero_eval::data_root();
    let suite = Suite::load(&data).unwrap();
    let tasks: Vec<_> = suite.tasks.iter().take(3).collect();
    let hs = [HarnessId::Aero, HarnessId::Codex];
    let plan = pm_aero_eval::runner::plan(&tasks, &hs, 4);
    assert_eq!(plan.len(), 3 * 2 * 4);
    // Every repeat finishes before the next starts.
    for w in plan.windows(2) {
        assert!(w[0].rep <= w[1].rep);
    }
    // Each harness goes first in its pair equally often.
    let firsts = plan
        .chunks(2)
        .filter(|c| c[0].harness == HarnessId::Aero)
        .count();
    assert_eq!(firsts, plan.len() / 4);
    for c in plan.chunks(2) {
        assert_eq!(c[0].task, c[1].task);
        assert_ne!(c[0].harness, c[1].harness);
    }
}
