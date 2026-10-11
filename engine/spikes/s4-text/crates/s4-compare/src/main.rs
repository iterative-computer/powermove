//! Runs the level 1 layout over the S4 corpus with the native CoreText
//! registry and compares every case with today's Canvas2D capture.
//!
//! s4-compare [--root out] [--group G] [--id ID] [--verbose]
//! Writes out/compare-native.json (summary and failures), out/native-output.json
//! (every case's layout, for the WASM identity check), and out/resolver-log.json
//! (the registry answers the replay resolver needs).

use s4_layout::corpus::{self, CaseDiff, CaseOutput, VARIABLE_SOURCES};
use s4_layout::fallback_cache::FallbackCached;
use s4_layout::font::{FaceId, FontBytes, FontResolver, PreparedCache};
use s4_layout::replay::{FaceRecord, LogFile};
use s4_registry::CoreTextRegistry;
use serde_json::{Value, json};
use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

fn arg(name: &str) -> Option<String> {
    let args: Vec<String> = std::env::args().collect();
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1).cloned())
}

fn flag(name: &str) -> bool {
    std::env::args().any(|a| a == name)
}

fn main() {
    if let Some(t) = arg("--shape") {
        shape_debug(&t);
        return;
    }
    let root = PathBuf::from(arg("--root").unwrap_or_else(|| "out".into()));
    let corpus_json = corpus::sanitize_lone_surrogates(&std::fs::read_to_string(root.join("corpus.json")).expect("out/corpus.json (run bun capture/run.ts measure)"));
    let canvas: Value = serde_json::from_str(&corpus::sanitize_lone_surrogates(&std::fs::read_to_string(root.join("canvas2d.json")).expect("out/canvas2d.json"))).unwrap();
    let expected: HashMap<String, Value> =
        canvas["results"].as_array().unwrap().iter().map(|r| (r["id"].as_str().unwrap().to_string(), r.clone())).collect();
    let mut cases = corpus::load_corpus(&corpus_json);
    if let Some(g) = arg("--group") {
        cases.retain(|c| c.group == g);
    }
    if let Some(id) = arg("--id") {
        cases.retain(|c| c.id == id);
    }
    let verbose = flag("--verbose");

    let mut registry = FallbackCached::new(CoreTextRegistry::new());
    let mut cache = PreparedCache::default();
    let mut custom: HashMap<String, FaceId> = HashMap::new();
    let mut custom_names: Vec<(String, FaceId)> = Vec::new();
    for (family, path) in VARIABLE_SOURCES {
        let bytes = std::fs::read(path).unwrap_or_else(|e| panic!("{path}: {e}"));
        let id = registry.register_custom(FontBytes(Arc::new(bytes)), 0);
        custom.insert(family.to_string(), id);
        custom_names.push((family.to_string(), id));
    }

    let started = Instant::now();
    let mut diffs: Vec<CaseDiff> = Vec::new();
    let mut outputs: Vec<CaseOutput> = Vec::new();
    for case in &cases {
        let out = corpus::run_case(&mut registry, &mut cache, case, &custom);
        let diff = match expected.get(&case.id) {
            Some(e) => corpus::compare(case, &out, e),
            None => CaseDiff { id: case.id.clone(), group: case.group.clone(), detail: vec!["no capture".into()], ..Default::default() },
        };
        if verbose && (!diff.breaks_identical || diff.max_advance_error > 0.25) {
            println!("{} [{}] breaks={} max={:.4}", diff.id, diff.group, diff.breaks_identical, diff.max_advance_error);
            for d in &diff.detail {
                println!("    {d}");
            }
        }
        diffs.push(diff);
        outputs.push(out);
    }
    let elapsed = started.elapsed();

    // Summary per group.
    let mut groups: BTreeMap<String, (usize, usize, usize, f64)> = BTreeMap::new();
    let mut errors: Vec<f64> = Vec::new();
    for d in &diffs {
        let g = groups.entry(d.group.clone()).or_default();
        g.0 += 1;
        if d.breaks_identical {
            g.1 += 1;
            errors.push(d.max_advance_error);
            if d.max_advance_error <= 0.25 {
                g.2 += 1;
            }
            g.3 = g.3.max(d.max_advance_error);
        }
    }
    errors.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let pct = |p: f64| errors.get(((errors.len() as f64 - 1.0) * p).round() as usize).copied().unwrap_or(0.0);
    let total = diffs.len();
    let identical = diffs.iter().filter(|d| d.breaks_identical).count();
    let pass = diffs.iter().filter(|d| d.breaks_identical && d.max_advance_error <= 0.25).count();
    println!("cases {total}: identical breaks {identical}, within 0.25 px {pass}; layout time {:.2}s", elapsed.as_secs_f64());
    println!("max advance error over identical-break cases: p50 {:.6} p99 {:.6} max {:.6}", pct(0.5), pct(0.99), pct(1.0));
    for (g, (n, ident, within, max)) in &groups {
        println!("  {g:12} {n:5} cases  breaks {ident:5}  within {within:5}  max {max:.4}");
    }

    let failures: Vec<&CaseDiff> = diffs.iter().filter(|d| !d.breaks_identical || d.max_advance_error > 0.25).collect();
    let summary = json!({
        "cases": total,
        "identicalBreaks": identical,
        "withinTolerance": pass,
        "tolerancePx": 0.25,
        "maxAdvanceError": { "p50": pct(0.5), "p99": pct(0.99), "max": pct(1.0) },
        "layoutSeconds": elapsed.as_secs_f64(),
        "groups": groups.iter().map(|(g, (n, i, w, m))| (g.clone(), json!({"cases": n, "identicalBreaks": i, "withinTolerance": w, "maxAdvanceError": m}))).collect::<serde_json::Map<_, _>>(),
        "failures": failures,
    });
    if arg("--group").is_none() && arg("--id").is_none() {
        std::fs::write(root.join("compare-native.json"), serde_json::to_string_pretty(&summary).unwrap()).unwrap();
        std::fs::write(root.join("native-output.json"), serde_json::to_string(&outputs).unwrap()).unwrap();
        write_log(&registry.inner, &custom_names, &root);
    }
}

/// Serializes the registry's answers and faces. Face bytes are referenced by
/// file path (system fonts stay on this machine and out of the repository);
/// table-assembled faces are written under out/blobs.
fn write_log(registry: &CoreTextRegistry, custom: &[(String, FaceId)], root: &std::path::Path) {
    let blobs = root.join("blobs");
    std::fs::create_dir_all(&blobs).unwrap();
    let mut by_ptr: HashMap<usize, String> = HashMap::new();
    let mut records = Vec::new();
    for (id, info) in registry.faces() {
        let ptr = Arc::as_ptr(&info.data.0) as usize;
        let name = by_ptr
            .entry(ptr)
            .or_insert_with(|| {
                let name = format!("blob-{:04}.bin", by_ptr_len(ptr, id));
                std::fs::write(blobs.join(&name), info.data.as_slice()).unwrap();
                name
            })
            .clone();
        records.push((
            id,
            FaceRecord {
                id,
                blob: name,
                index: info.index,
                coords: info.coords.clone(),
                synthetic_bold: info.synthetic_bold,
                synthetic_italic: info.synthetic_italic,
                custom: info.custom,
                synthesized_hyphens: info.synthesized_hyphens,
                postscript_name: info.postscript_name.clone(),
                typeface_key: info.typeface_key.clone(),
                fallback_key: info.fallback_key.clone(),
            },
        ));
    }
    let mut file = LogFile::from_log(&registry.log, &records);
    file.custom_names = custom.to_vec();
    std::fs::write(root.join("resolver-log.json"), serde_json::to_string(&file).unwrap()).unwrap();
}

fn by_ptr_len(_ptr: usize, id: FaceId) -> FaceId {
    id
}

/// Debug: `s4-compare --shape TEXT [--font F] [--weight W] [--size S] [--tracking T]`
/// prints Blink-port runs with their faces and advances.
pub fn shape_debug(text: &str) {
    use s4_layout::blink::measure;
    use s4_layout::legacy::TextContent;
    let d = TextContent {
        text: text.into(),
        font: arg("--font"),
        weight: arg("--weight").map(|w| w.parse().unwrap()),
        size: Some(arg("--size").map(|s| s.parse().unwrap()).unwrap_or(16.0)),
        tracking: arg("--tracking").map(|s| s.parse().unwrap()),
        ..Default::default()
    };
    let mut registry = CoreTextRegistry::new();
    let mut cache = PreparedCache::default();
    let fams = s4_layout::canvas::families(&d, None);
    let setup = measure::build_setup(&mut registry, &fams, s4_layout::canvas::font_desc(&d), s4_layout::canvas::letter_spacing(&d));
    for g in &setup.group {
        println!("group face {} {}", g.face(), registry.face(g.face()).postscript_name);
    }
    // `\uXXXX` escapes allow lone surrogates.
    let mut units: Vec<u16> = Vec::new();
    let mut rest = text;
    while let Some(i) = rest.find("\\u") {
        units.extend(rest[..i].encode_utf16());
        units.push(u16::from_str_radix(&rest[i + 2..i + 6], 16).unwrap());
        rest = &rest[i + 6..];
    }
    units.extend(rest.encode_utf16());
    let m = measure::measure(&mut registry, &mut cache, &setup, &units);
    println!("width {}", m.width);
    for item in &m.items {
        for run in &item.runs {
            let info = registry.face(run.face);
            let adv: Vec<f64> = run.glyphs.iter().map(|g| g.advance as f64 / 65536.0).collect();
            println!(
                "  run face={} {} coords={:?} synth_b={} start={} n={} width={} glyphs={:?} adv={:?}",
                run.face, info.postscript_name, info.coords, info.synthetic_bold, run.start_index, run.num_characters, run.width,
                run.glyphs.iter().map(|g| g.glyph).collect::<Vec<_>>(), adv
            );
        }
    }
}
