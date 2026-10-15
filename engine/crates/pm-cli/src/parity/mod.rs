//! `pm-cli parity eval`: evaluation parity against the corpus (plan section
//! 25, "Parity" and "Infrastructure"; section 26, Phase 1 gate).
//!
//! Every project of the capture is evaluated at every captured sample time,
//! and every dumped value is compared (`items`): natively (`native`), and
//! with `--wasm` through the built pm-wasm module in Node plus the same
//! engine compiled natively, which must reply byte for byte the same
//! (`wasm`). See docs/engine/phase1-gate.md.

pub mod compare;
pub mod corpus;
pub mod fonts;
pub mod items;
pub mod native;
pub mod wasm;

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::time::Instant;

use pm_eval::text_measure::CompatMeasure;
use pm_model::Document;
use pm_model::json::Value;
use pm_text::compat::FontCacheSeed;
use pm_text::registry::CoreTextRegistry;

use compare::{Answer, Outcome, Stats, dumped_number, judge, member, text_gate};
use items::{Item, Kind};

/// Why a text layer's geometry is not compared: its layout differs from the
/// capture (the layout itself is counted as a mismatch).
pub const LAYOUT_DIFFERS: &str =
    "its text layout differs from the capture (counted under textLayout)";

pub struct Options {
    pub capture: Option<PathBuf>,
    pub only: Vec<String>,
    pub exclude: Vec<String>,
    /// Measure text as a fresh renderer (no font cache seeds).
    pub fresh_font_cache: bool,
    pub native: bool,
    pub wasm: bool,
    pub wasm_module: Option<PathBuf>,
    pub wasm_subsets: bool,
    pub node: String,
    pub json: Option<PathBuf>,
    pub verbose: bool,
    pub max_samples: usize,
}

/// The run's results.
pub struct Report {
    pub capture: PathBuf,
    pub projects: usize,
    pub dumps: usize,
    pub native: Option<Stats>,
    pub wasm: Option<Stats>,
    pub identity: Option<Identity>,
    pub font_cache: String,
    pub seconds: f64,
}

#[derive(Default)]
pub struct Identity {
    pub replies: usize,
    pub differ: usize,
    pub samples: Vec<String>,
    pub answers: usize,
    pub faces: usize,
}

/// The UI's type scale (`packages/tokens/tokens.css`: `--fs-sm`, `--fs-md`)
/// in CSS px: the app lays `system-ui` DOM text out at these sizes before a
/// canvas measures anything.
const UI_SYSTEM_UI_SIZES: [f32; 2] = [12.0, 13.0];

/// The capture renderer's font cache (`docs/engine/text.md`, "The
/// renderer's font cache"; `pm-eval/tests/corpus.rs`): at the capture's
/// device pixel ratio the UI's `system-ui` text created the renderer's 24
/// and 26 px entries.
fn capture_font_cache_seeds(manifest: Option<&Value>) -> Vec<FontCacheSeed> {
    let dpr = manifest
        .and_then(|m| member(m, "gpu"))
        .and_then(|g| member(g, "devicePixelRatio"))
        .and_then(dumped_number)
        .unwrap_or(1.0) as f32;
    if dpr == 1.0 {
        return Vec::new();
    }
    UI_SYSTEM_UI_SIZES
        .iter()
        .map(|&css| FontCacheSeed {
            family: "system-ui".into(),
            weight: 400.0,
            italic: false,
            size: pm_text::compat::canvas::effective_font_size(css * dpr),
            specified: css,
        })
        .collect()
}

fn selected(name: &str, opts: &Options) -> bool {
    (opts.only.is_empty() || opts.only.iter().any(|p| name.starts_with(p.as_str())))
        && !opts.exclude.iter().any(|p| name.starts_with(p.as_str()))
}

/// Judges one dump's answers: text layouts first (a layout that differs
/// takes the geometry that depends on it out of the comparison), then the
/// rest.
pub fn judge_dump(stats: &mut Stats, project: &str, items: &[Item], answers: &[Answer]) {
    let mut differs: BTreeSet<(String, usize)> = BTreeSet::new();
    let label = |it: &Item| format!("{project} {}", it.label);
    for (it, a) in items.iter().zip(answers) {
        if !matches!(it.kind, Kind::TextLayout) {
            continue;
        }
        if let Answer::Unanswered(reason) = a {
            stats.skip(it.category, reason);
            continue;
        }
        let (outcome, err, why) = judge(a, &it.want, it.legacy_threw.is_some(), it.rule);
        if let Answer::Value(v) = a {
            text_gate(&mut stats.text, &compare::plain(v), &it.want, || label(it));
        }
        if outcome != Outcome::Match {
            if let Some(i) = it.layer {
                differs.insert((it.path.clone(), i));
            }
            *stats
                .layout_fonts
                .entry(it.font.clone().unwrap_or_else(|| "(none)".into()))
                .or_default() += 1;
        }
        stats.record(it.category, outcome, err, || {
            format!("{}: {why}", label(it))
        });
    }
    for (it, a) in items.iter().zip(answers) {
        if matches!(it.kind, Kind::TextLayout) {
            continue;
        }
        if it
            .text_deps
            .iter()
            .any(|i| differs.contains(&(it.path.clone(), *i)))
        {
            stats.skip(it.category, LAYOUT_DIFFERS);
            continue;
        }
        if let Answer::Unanswered(reason) = a {
            stats.skip(it.category, reason);
            continue;
        }
        if let (Some(want), Answer::Threw(got)) = (&it.legacy_threw, a) {
            let same = want == got;
            stats.record(
                "throw messages",
                if same {
                    Outcome::Match
                } else {
                    Outcome::Mismatch
                },
                0.0,
                || format!("{}: ours {got:?}, capture {want:?}", label(it)),
            );
        }
        let (outcome, err, why) = judge(a, &it.want, it.legacy_threw.is_some(), it.rule);
        if let (Outcome::Question(_), Answer::Value(v)) = (outcome, a) {
            let d = compare::max_abs_difference(&compare::plain(v), &it.want).unwrap_or(f64::NAN);
            let c = stats.categories.entry(it.category).or_default();
            if d.is_nan() || d > c.question_max_abs {
                c.question_max_abs = d;
            }
        }
        stats.record(it.category, outcome, err, || {
            format!("{}: {why}", label(it))
        });
    }
}

pub fn run(opts: &Options) -> Result<Report, String> {
    let start = Instant::now();
    let capture = match &opts.capture {
        Some(c) => c.clone(),
        None => corpus::default_capture().ok_or(
            "no parity corpus capture found under ~/Library/Application Support/Powermove Engine Corpus; pass --corpus",
        )?,
    };
    if !capture.join("projects").is_dir() {
        return Err(format!("{} is not a corpus capture", capture.display()));
    }
    let manifest = corpus::read_json(&capture.join("manifest.json")).ok();
    let seeds = if opts.fresh_font_cache {
        Vec::new()
    } else {
        capture_font_cache_seeds(manifest.as_ref())
    };
    let font_cache = if seeds.is_empty() {
        "fresh renderer".to_owned()
    } else {
        seeds
            .iter()
            .map(|s| {
                format!(
                    "{} {} at {} px from {} px",
                    s.family, s.weight, s.size, s.specified
                )
            })
            .collect::<Vec<_>>()
            .join(", ")
    };

    let mut measure = CompatMeasure::new(CoreTextRegistry::new());
    measure.compat().set_font_cache_seeds(seeds);
    let mut native_stats = opts
        .native
        .then(|| Stats::new(opts.verbose, opts.max_samples));
    let mut wasm_stats = opts
        .wasm
        .then(|| Stats::new(opts.verbose, opts.max_samples));
    let mut identity = opts.wasm.then(Identity::default);
    let mut font_service = opts
        .wasm
        .then(|| fonts::FontService::new(opts.wasm_subsets));
    let mut runner = if opts.wasm {
        let engine = Path::new(env!("CARGO_MANIFEST_DIR"));
        let module = opts.wasm_module.clone().unwrap_or_else(|| {
            engine.join("../../../apps/desktop/src/renderer/public/engine/pm_wasm.wasm")
        });
        if !module.is_file() {
            return Err(format!(
                "{} is missing; build it with node engine/crates/pm-wasm/scripts/build.mjs",
                module.display()
            ));
        }
        let dir = std::env::temp_dir().join(format!("pm-cli-parity-{}", std::process::id()));
        Some(wasm::NodeRunner::start(
            &opts.node,
            &engine.join("scripts/parity-wasm.mjs"),
            &module,
            dir,
        )?)
    } else {
        None
    };

    let mut projects = 0;
    let mut dumps = 0;
    for project in corpus::projects(&capture) {
        if !selected(&project.name, opts) {
            continue;
        }
        projects += 1;
        let loaded = project.loaded()?;
        let canonical = opts
            .wasm
            .then(|| pm_model::canonical::canonical_project(loaded.clone()));
        let doc = Document::new(Some(loaded), 1);
        let mut discovery = match (&canonical, font_service.as_mut()) {
            (Some(c), Some(fs)) => Some(wasm::Discovery::new(fs, c)?),
            _ => None,
        };
        let mut project_items: Vec<Vec<Item>> = Vec::new();
        for path in &project.dumps {
            let dump = corpus::read_json(path)?;
            let mut builder = items::Builder {
                measure: &mut measure,
                skipped: Vec::new(),
            };
            let items = builder
                .dump(&doc, &dump)
                .map_err(|e| format!("{} {}: {e}", project.name, path.display()))?;
            let skipped = std::mem::take(&mut builder.skipped);
            for stats in [native_stats.as_mut(), wasm_stats.as_mut()]
                .into_iter()
                .flatten()
            {
                for (c, r) in &skipped {
                    stats.skip(c, r);
                }
            }
            if let Some(stats) = native_stats.as_mut() {
                let answers = native::answer_dump(&doc, &items, &mut measure);
                judge_dump(stats, &project.name, &items, &answers);
            }
            if let Some(d) = discovery.as_mut() {
                d.dump(&items)
                    .map_err(|e| format!("{} {}: {e}", project.name, path.display()))?;
                project_items.push(items);
            }
            dumps += 1;
        }
        let finished = discovery.map(wasm::Discovery::finish);
        if let (Some(p), Some(fs), Some(runner), Some(stats), Some(id)) = (
            finished,
            font_service.as_mut(),
            runner.as_mut(),
            wasm_stats.as_mut(),
            identity.as_mut(),
        ) {
            let native = wasm::native_replies(&p, fs)?;
            let wasm = runner
                .replies(&p, fs)
                .map_err(|e| format!("{}: {e}", project.name))?;
            id.replies += native.len().max(wasm.len());
            id.answers += p.answers.answers.len();
            id.faces = id.faces.max(p.faces.len());
            for i in 0..native.len().max(wasm.len()) {
                let (a, b) = (native.get(i), wasm.get(i));
                if a != b {
                    id.differ += 1;
                    if id.samples.len() < opts.max_samples {
                        id.samples.push(format!(
                            "{} request {}: native {} wasm {}",
                            project.name,
                            p.requests.get(i).map_or("?", String::as_str),
                            a.map_or("(none)", String::as_str),
                            b.map_or("(none)", String::as_str)
                        ));
                    }
                }
            }
            for (items, plans) in project_items.iter().zip(&p.plans) {
                let answers = wasm::answers(items, plans, &wasm);
                judge_dump(stats, &project.name, items, &answers);
            }
        }
    }
    if let Some(r) = runner {
        r.close();
    }
    let report = Report {
        capture,
        projects,
        dumps,
        native: native_stats,
        wasm: wasm_stats,
        identity,
        font_cache,
        seconds: start.elapsed().as_secs_f64(),
    };
    if let Some(path) = &opts.json {
        std::fs::write(
            path,
            serde_json::to_string_pretty(&report_json(&report)).unwrap_or_default(),
        )
        .map_err(|e| format!("{}: {e}", path.display()))?;
    }
    Ok(report)
}

fn stats_json(s: &Stats) -> serde_json::Value {
    let categories: serde_json::Map<String, serde_json::Value> = s
        .categories
        .iter()
        .map(|(k, c)| {
            (
                (*k).to_owned(),
                serde_json::json!({
                    "compared": c.compared, "matched": c.matched, "allowed": c.allowed,
                    "questions": c.questions, "questionMaxAbsDifference": c.question_max_abs, "mismatched": c.mismatched,
                    "maxRelativeError": c.max_error, "samples": c.samples,
                }),
            )
        })
        .collect();
    let skipped: Vec<serde_json::Value> = s
        .skipped
        .iter()
        .map(|((c, r), n)| serde_json::json!({"category": c, "reason": r, "count": n}))
        .collect();
    serde_json::json!({
        "compared": s.compared(), "mismatched": s.mismatches(), "questions": s.questions(),
        "categories": categories, "notCompared": skipped,
        "textLayoutsDifferingByFont": s.layout_fonts,
        "textGate": {
            "layouts": s.text.layouts, "units": s.text.units,
            "lineBreaksDiffer": s.text.line_breaks_differ, "advancesOver025px": s.text.advances_over,
            "maxAdvanceErrorPx": s.text.max_advance_error, "samples": s.text.samples,
        },
    })
}

pub fn report_json(r: &Report) -> serde_json::Value {
    serde_json::json!({
        "format": "powermove-parity-eval", "version": 1,
        "capture": r.capture, "projects": r.projects, "dumps": r.dumps,
        "fontCache": r.font_cache, "seconds": r.seconds,
        "native": r.native.as_ref().map(stats_json),
        "wasm": r.wasm.as_ref().map(stats_json),
        "identity": r.identity.as_ref().map(|i| serde_json::json!({
            "replies": i.replies, "differ": i.differ, "samples": i.samples,
            "fontAnswers": i.answers, "maxFacesPerProject": i.faces,
        })),
    })
}

fn print_stats(title: &str, s: &Stats) {
    println!(
        "\n{title}: {} values compared, {} mismatched, {} tolerance questions",
        s.compared(),
        s.mismatches(),
        s.questions()
    );
    for (category, c) in &s.categories {
        let allowed: usize = c.allowed.values().sum();
        let questions: usize = c.questions.values().sum();
        println!(
            "  {category:<28} {:>8} compared {:>8} matched {:>6} allowed {:>6} questions {:>6} mismatched   max rel. error {:.3e}",
            c.compared, c.matched, allowed, questions, c.mismatched, c.max_error
        );
        for (class, n) in c.allowed.iter() {
            println!("      allowed ({class}): {n}");
        }
        for (class, n) in c.questions.iter() {
            println!(
                "      tolerance question ({class}): {n}, max absolute difference {:.3e}",
                c.question_max_abs
            );
        }
        for sample in &c.samples {
            println!("      {sample}");
        }
    }
    for ((category, reason), n) in &s.skipped {
        println!("  not compared  {category:<28} {n:>7}: {reason}");
    }
    if !s.layout_fonts.is_empty() {
        println!("  text layouts that differ from the capture, by font:");
        for (font, n) in &s.layout_fonts {
            println!("    {font:<28} {n:>7}");
        }
    }
    let t = &s.text;
    println!(
        "  text sub-gate: {} layouts ({} units): line breaks differ in {}, advances over 0.25 px in {}, max position or advance error {:.6} px",
        t.layouts, t.units, t.line_breaks_differ, t.advances_over, t.max_advance_error
    );
    for sample in &t.samples {
        println!("      {sample}");
    }
}

pub fn print_report(r: &Report) {
    println!(
        "pm-cli parity eval: {} ({} projects, {} dumps) in {:.1} s",
        r.capture.display(),
        r.projects,
        r.dumps,
        r.seconds
    );
    println!("renderer font cache (native): {}", r.font_cache);
    if let Some(s) = &r.native {
        print_stats("native (pm-eval, CoreText registry)", s);
    }
    if let Some(s) = &r.wasm {
        print_stats(
            "wasm (pm-wasm in Node, registry answers and faces from main)",
            s,
        );
    }
    if let Some(i) = &r.identity {
        println!(
            "\nnative vs wasm32 (pm-wasm compiled natively and the module in Node): {} replies, {} differ",
            i.replies, i.differ
        );
        for s in &i.samples {
            println!("      {s}");
        }
    }
}

/// The exit status: 0 when every compared value is within the plan's
/// tolerances or an allowed difference and native and wasm agree; 1 on any
/// mismatch or identity difference; 2 when only tolerance questions remain.
pub fn status(r: &Report) -> i32 {
    let stats = [r.native.as_ref(), r.wasm.as_ref()];
    let mismatches: usize = stats.iter().flatten().map(|s| s.mismatches()).sum();
    let questions: usize = stats.iter().flatten().map(|s| s.questions()).sum();
    let differ = r.identity.as_ref().map_or(0, |i| i.differ);
    if mismatches > 0 || differ > 0 {
        1
    } else if questions > 0 {
        2
    } else {
        0
    }
}
