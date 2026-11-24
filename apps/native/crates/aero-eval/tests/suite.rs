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
    assert_eq!((i.estimate, i.lo, i.hi), (3.0, 1.0, 5.0));
    // n = 10 at 90 %: [x2, x9] (coverage 97.9 %; [x3, x8] covers only 89 %).
    let v: Vec<f64> = (1..=10).map(f64::from).collect();
    let i = stats::median_interval(&v, 0.90).unwrap();
    assert_eq!((i.estimate, i.lo, i.hi), (5.5, 2.0, 9.0));
    // n = 20: [x6, x15] (coverage 95.9 %).
    let v: Vec<f64> = (1..=20).map(f64::from).collect();
    let i = stats::median_interval(&v, 0.90).unwrap();
    assert_eq!((i.lo, i.hi), (6.0, 15.0));
    assert!(stats::median_interval(&[], 0.9).is_none());
}

fn run(harness: &str, task: &str, success: bool, wall: u64, tokens: u64) -> RunRecord {
    RunRecord {
        format: "aero-eval-run".into(),
        suite_version: 1,
        task: task.into(),
        kind: "cut".into(),
        harness: harness.into(),
        model: "gpt-6.2-sol".into(),
        effort: "high".into(),
        rep: 0,
        order: 0,
        started_unix_ms: 0,
        wall_ms: wall,
        success,
        renders_correct: None,
        unavailable: false,
        tokens: Usage {
            input: tokens,
            cached: tokens / 2,
            output: tokens / 10,
        },
        tool_calls: 5,
        retries: 0,
        gate_continuations: (harness == "aero").then_some(1),
        timed_out: false,
        over_step_limit: false,
        exit_code: Some(0),
        needs_fix: None,
        verdicts: vec![],
        approved: Some("test".into()),
        replay: None,
        error: None,
    }
}

#[test]
fn the_bar_needs_separated_intervals_and_no_worse_cost() {
    let mut runs = Vec::new();
    for i in 0..20 {
        runs.push(run("aero", "t", i < 19, 60_000, 50_000));
        runs.push(run("codex", "t", i < 8, 90_000, 80_000));
    }
    let s = stats::summarize(&runs, 1);
    assert_eq!(s.bar.len(), 1);
    assert!(s.bar[0].better, "{:?}", s.bar[0].reasons);
    // Same success, or more tokens: not better.
    let mut runs2: Vec<RunRecord> = runs
        .iter()
        .filter(|r| r.harness == "codex")
        .cloned()
        .collect();
    for i in 0..20 {
        runs2.push(run("aero", "t", i < 19, 60_000, 90_000));
    }
    let s = stats::summarize(&runs2, 1);
    assert!(!s.bar[0].better);
    assert!(
        s.bar[0]
            .reasons
            .iter()
            .any(|r| r.contains("tokens") && r.contains("worse"))
    );
    let mut close = Vec::new();
    for i in 0..5 {
        close.push(run("aero", "t", i < 5, 60_000, 50_000));
        close.push(run("codex", "t", i < 3, 90_000, 80_000));
    }
    assert!(
        !stats::summarize(&close, 1).bar[0].better,
        "5 runs cannot separate 100 % from 60 %"
    );
}

#[test]
fn summaries_exclude_other_suite_versions_and_group_by_task() {
    let mut runs = vec![run("aero", "a", true, 1, 1), run("aero", "b", false, 1, 1)];
    let mut old = run("aero", "a", true, 1, 1);
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
