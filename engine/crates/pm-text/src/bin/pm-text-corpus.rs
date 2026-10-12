//! The pm-text corpus tool.
//!
//! - `pm-text-corpus fixture [--root DIR] [--out FILE]` turns a Canvas2D
//!   capture (`capture/run.ts` writes `DIR/corpus.json` and
//!   `DIR/canvas2d.json`) into the checked-in expectations
//!   (`tests/fixtures/canvas2d-corpus.json.gz`).
//! - `pm-text-corpus compare [--fixture FILE] [--group G] [--id ID] [--verbose]`
//!   lays the corpus out natively and compares it with the expectations.
//! - `pm-text-corpus export [--fixture FILE] --out DIR` writes what the WASM
//!   identity check needs: the registry answers, the subsets, the corpus,
//!   and the native outputs over full fonts and over subsets.
//!
//! `DIR` defaults to `engine/target/pm-text`; the fixture to the checked-in one.

use pm_text::corpus::{self, CaseDiff, CorpusFile, Expected, Fixture, FixtureCase, FontFile, Packed};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

fn arg(name: &str) -> Option<String> {
    let args: Vec<String> = std::env::args().collect();
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1).cloned())
}

fn flag(name: &str) -> bool {
    std::env::args().any(|a| a == name)
}

fn crate_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn default_fixture() -> PathBuf {
    crate_dir().join("tests/fixtures/canvas2d-corpus.json.gz")
}

fn default_root() -> PathBuf {
    crate_dir().join("../../target/pm-text")
}

fn read_fixture(path: &Path) -> Result<Fixture, String> {
    let raw = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let json = if path.extension().is_some_and(|e| e == "gz") {
        let mut s = String::new();
        flate2::read::GzDecoder::new(&raw[..]).read_to_string(&mut s).map_err(|e| format!("{}: {e}", path.display()))?;
        s
    } else {
        String::from_utf8(raw).map_err(|e| e.to_string())?
    };
    Fixture::from_json(&json).map_err(|e| format!("{}: {e}", path.display()))
}

fn os_version() -> String {
    let out = std::process::Command::new("sw_vers").output().ok();
    out.map(|o| String::from_utf8_lossy(&o.stdout).split_whitespace().collect::<Vec<_>>().join(" ")).unwrap_or_default()
}

fn fixture(root: &Path, out: &Path) -> Result<(), String> {
    let corpus_json = std::fs::read_to_string(root.join("corpus.json")).map_err(|e| format!("corpus.json: {e} (run capture/run.ts)"))?;
    let corpus_value: Value = serde_json::from_str(&corpus::sanitize_lone_surrogates(&corpus_json)).map_err(|e| e.to_string())?;
    let corpus = CorpusFile::from_value(&corpus_value).map_err(|e| e.to_string())?;
    let canvas: Value = serde_json::from_str(&corpus::sanitize_lone_surrogates(&std::fs::read_to_string(root.join("canvas2d.json")).map_err(|e| format!("canvas2d.json: {e}"))?))
        .map_err(|e| e.to_string())?;
    let results: HashMap<&str, &Value> = canvas["results"].as_array().ok_or("canvas2d.json has no results")?.iter().map(|r| (r["id"].as_str().unwrap_or_default(), r)).collect();
    let mut cases = Vec::new();
    for (case, raw) in corpus.cases.iter().zip(corpus_value["cases"].as_array().ok_or("corpus has no cases")?) {
        let r = results.get(case.id.as_str()).ok_or_else(|| format!("{} was not captured", case.id))?;
        let expected = Expected::from_capture(r).ok_or_else(|| format!("{}: capture error {}", case.id, r["error"]))?;
        let expected = Packed::pack(&expected).map_err(|e| format!("{}: {e}", case.id))?;
        cases.push(FixtureCase { id: case.id.clone(), group: case.group.clone(), d: raw["d"].clone(), expected });
    }
    // The native run lists the font files layout reads, so a test can tell
    // font drift (an OS update) from layout drift.
    let run = corpus::run_native(&corpus)?;
    let font_files = run.registry.files_read().into_iter().map(|(p, bytes)| FontFile { path: p.to_string_lossy().into_owned(), bytes }).collect();
    let fixture = Fixture {
        user_agent: canvas["userAgent"].as_str().unwrap_or_default().to_string(),
        os: os_version(),
        variable_sources: corpus.variable_sources.clone(),
        project_fonts: corpus.project_fonts.clone(),
        font_files,
        cases,
    };
    let json = serde_json::to_vec(&fixture).map_err(|e| e.to_string())?;
    let file = std::fs::File::create(out).map_err(|e| format!("{}: {e}", out.display()))?;
    let mut gz = flate2::GzBuilder::new().mtime(0).write(file, flate2::Compression::best());
    gz.write_all(&json).map_err(|e| e.to_string())?;
    gz.finish().map_err(|e| e.to_string())?;
    println!("{} cases, {} font files; {} bytes of JSON -> {}", fixture.cases.len(), fixture.font_files.len(), json.len(), out.display());
    Ok(())
}

fn compare(fixture: &Fixture) -> Result<bool, String> {
    let drift = fixture.font_drift();
    if !drift.is_empty() {
        println!("font drift since the capture ({}):", fixture.os);
        for d in &drift {
            println!("  {d}");
        }
    }
    let mut corpus = fixture.corpus();
    let wanted: Vec<bool> = corpus.cases.iter().map(|c| arg("--group").is_none_or(|g| c.group == g) && arg("--id").is_none_or(|id| c.id == id)).collect();
    // Every case runs (the fallback cache depends on order); only the
    // selected ones are reported.
    let run = corpus::run_native(&corpus)?;
    let verbose = flag("--verbose");
    let mut diffs: Vec<CaseDiff> = Vec::new();
    for ((case, out), (fc, want)) in corpus.cases.iter().zip(&run.outputs).zip(fixture.cases.iter().zip(&wanted)) {
        if !want {
            continue;
        }
        let diff = corpus::compare(case, out, &fc.expected.unpack());
        if verbose && !diff.passes() {
            println!("{} [{}] breaks={} max={:.4}", diff.id, diff.group, diff.breaks_identical, diff.max_advance_error);
            for d in &diff.detail {
                println!("    {d}");
            }
        }
        diffs.push(diff);
    }
    corpus.cases.clear();
    let s = corpus::summarize(&diffs);
    println!(
        "cases {}: identical breaks {}, within {} px {}; max error {:.6} px (p99 {:.6}); native layout {:.2} s",
        s.cases, s.identical_breaks, corpus::TOLERANCE_PX, s.within_tolerance, s.max_advance_error, s.p99_advance_error, run.seconds
    );
    for (g, (n, ident, within, max)) in &s.groups {
        println!("  {g:12} {n:5} cases  breaks {ident:5}  within {within:5}  max {max:.6}");
    }
    Ok(s.within_tolerance == s.cases)
}

fn export(fixture: &Fixture, out: &Path) -> Result<(), String> {
    let corpus = fixture.corpus();
    let run = corpus::run_native(&corpus)?;
    std::fs::create_dir_all(out.join("subsets")).map_err(|e| e.to_string())?;
    let write = |name: &str, bytes: &[u8]| std::fs::write(out.join(name), bytes).map_err(|e| format!("{name}: {e}"));
    write("answers.json", &serde_json::to_vec(&run.answers).map_err(|e| e.to_string())?)?;
    write("native-output.json", &serde_json::to_vec(&run.outputs).map_err(|e| e.to_string())?)?;
    let corpus_json = json!({
        "variableSources": fixture.variable_sources,
        "projectFonts": fixture.project_fonts,
        "cases": fixture.cases.iter().map(|c| json!({"id": c.id, "group": c.group, "d": c.d})).collect::<Vec<_>>(),
    });
    write("corpus.json", &serde_json::to_vec(&corpus_json).map_err(|e| e.to_string())?)?;
    let started = std::time::Instant::now();
    let subsets = corpus::subsets_for(&run.answers, |b| run.registry.blob(b))?;
    let built = started.elapsed().as_secs_f64();
    let mut full = 0u64;
    let mut seen = std::collections::BTreeSet::new();
    for (blob, _) in run.answers.sources() {
        if seen.insert(blob.clone()) {
            full += run.registry.blob(&blob).map(|b| b.as_slice().len() as u64).unwrap_or(0);
        }
    }
    let sub_total: usize = subsets.values().map(|b| b.len()).sum();
    for (key, bytes) in &subsets {
        write(&format!("subsets/{key}.bin"), bytes)?;
    }
    // The native replay over subsets: the output the WASM module must match byte for byte.
    let (replayed, misses, pending) = corpus::run_replay(&run.answers, |b, i| (pm_text::font::FontBytes::new(subsets[&pm_text::answers::subset_key(b, i)].clone()), 0), &corpus.cases);
    let replay_json = serde_json::to_vec(&replayed).map_err(|e| e.to_string())?;
    write("replay-subsets-output.json", &replay_json)?;
    let identical = replayed.iter().zip(&run.outputs).filter(|(a, b)| a == b).count();
    let report = json!({
        "cases": run.outputs.len(),
        "faces": run.answers.faces.len(),
        "fontFiles": seen.len(),
        "fontBytes": full,
        "subsets": subsets.len(),
        "subsetBytes": sub_total,
        "subsetPercent": 100.0 * sub_total as f64 / full.max(1) as f64,
        "subsetSeconds": built,
        "replayIdenticalToNative": identical,
        "misses": misses,
        "pendingBlobs": pending,
    });
    write("export.json", serde_json::to_string_pretty(&report).map_err(|e| e.to_string())?.as_bytes())?;
    println!("{report}");
    if identical != run.outputs.len() || misses != 0 || pending != 0 {
        return Err("the subset replay differs from the native run".into());
    }
    Ok(())
}

fn main() -> ExitCode {
    let cmd = std::env::args().nth(1).unwrap_or_default();
    let fixture_path = arg("--fixture").map(PathBuf::from).unwrap_or_else(default_fixture);
    let result = match cmd.as_str() {
        "fixture" => fixture(&arg("--root").map(PathBuf::from).unwrap_or_else(default_root), &arg("--out").map(PathBuf::from).unwrap_or_else(default_fixture)).map(|_| true),
        "compare" => read_fixture(&fixture_path).and_then(|f| compare(&f)),
        "export" => read_fixture(&fixture_path).and_then(|f| export(&f, &arg("--out").map(PathBuf::from).unwrap_or_else(|| default_root().join("wasm"))).map(|_| true)),
        _ => Err("usage: pm-text-corpus fixture|compare|export [options]".into()),
    };
    match result {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::FAILURE,
        Err(e) => {
            eprintln!("{e}");
            ExitCode::FAILURE
        }
    }
}
