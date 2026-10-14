//! Level 1 layout regression test against Canvas2D (plan section 12's text
//! gate): the S4 corpus plus project fonts, laid out natively over the
//! CoreText registry, must reproduce the checked-in Canvas2D capture with
//! identical line breaks and advances within 0.25 px. The same run, replayed
//! from the recorded registry answers over full fonts and over
//! metrics-and-shaping subsets, must be bit-identical to the native run
//! (the code `pm-wasm` runs), and the subsets must stay small.
//!
//! Runs without the test harness so the registry lives on the main thread
//! (AppKit's font manager). Recapture with `capture/run.ts` and
//! `pm-text-corpus fixture` after a macOS update changes the fonts.

#[cfg(all(target_os = "macos", feature = "system"))]
fn main() {
    use flate2::read::GzDecoder;
    use pm_text::answers::subset_key;
    use pm_text::corpus::{self, Fixture, TOLERANCE_PX};
    use pm_text::font::FontBytes;
    use std::io::Read;

    /// Worst error the port has shown on the corpus is 0.00101 px; the gate
    /// is 0.25 px. A layout change that lands between the two is reported as
    /// a regression even though it passes the gate.
    const REGRESSION_SENTINEL_PX: f64 = 0.01;

    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/canvas2d-corpus.json.gz");
    let mut json = String::new();
    GzDecoder::new(std::fs::File::open(&path).expect("fixture"))
        .read_to_string(&mut json)
        .expect("gzip fixture");
    let fixture = Fixture::from_json(&json).expect("fixture json");
    let drift = fixture.font_drift();
    assert!(
        drift.is_empty(),
        "fonts changed since the capture on {} ({}); recapture (capture/run.ts, then pm-text-corpus fixture):\n{}",
        fixture.os,
        fixture.user_agent,
        drift.join("\n")
    );

    let corpus = fixture.corpus();
    let run = corpus::run_native(&corpus).expect("native registry");
    let diffs: Vec<_> = corpus
        .cases
        .iter()
        .zip(&run.outputs)
        .zip(&fixture.cases)
        .map(|((c, o), f)| corpus::compare(c, o, &f.expected.unpack()))
        .collect();
    let s = corpus::summarize(&diffs);
    println!(
        "level 1 vs Canvas2D: {} cases, identical breaks {}, within {TOLERANCE_PX} px {}, max error {:.6} px (p99 {:.6}), native layout {:.2} s",
        s.cases,
        s.identical_breaks,
        s.within_tolerance,
        s.max_advance_error,
        s.p99_advance_error,
        run.seconds
    );
    for (g, (n, ident, within, max)) in &s.groups {
        println!("  {g:12} {n:5} cases  breaks {ident:5}  within {within:5}  max {max:.6}");
    }
    let failures: Vec<String> = diffs
        .iter()
        .filter(|d| !d.passes())
        .take(20)
        .map(|d| format!("{} [{}]: {}", d.id, d.group, d.detail.join("; ")))
        .collect();
    assert!(
        failures.is_empty(),
        "{} cases fail the text gate:\n{}",
        diffs.iter().filter(|d| !d.passes()).count(),
        failures.join("\n")
    );
    assert!(
        s.max_advance_error <= REGRESSION_SENTINEL_PX,
        "max error {} px passes the 0.25 px gate but regressed past {REGRESSION_SENTINEL_PX} px",
        s.max_advance_error
    );

    // Replay over full fonts: the registry boundary loses nothing.
    let full = |blob: &str, index: u32| (run.registry.blob(blob).expect("recorded blob"), index);
    let (replayed, misses, pending) = corpus::run_replay(&run.answers, full, &corpus.cases);
    assert_eq!((misses, pending), (0, 0), "replay misses and pending faces");
    let same = replayed
        .iter()
        .zip(&run.outputs)
        .filter(|(a, b)| a == b)
        .count();
    assert_eq!(
        same,
        run.outputs.len(),
        "replay over full fonts is bit-identical to the native run"
    );

    // Replay over metrics-and-shaping subsets (what main serves to pm-wasm).
    let subsets = corpus::subsets_for(&run.answers, |b| run.registry.blob(b)).expect("subsets");
    let (replayed, misses, pending) = corpus::run_replay(
        &run.answers,
        |b, i| (FontBytes::new(subsets[&subset_key(b, i)].clone()), 0),
        &corpus.cases,
    );
    assert_eq!(
        (misses, pending),
        (0, 0),
        "subset replay misses and pending faces"
    );
    let same = replayed
        .iter()
        .zip(&run.outputs)
        .filter(|(a, b)| a == b)
        .count();
    assert_eq!(
        same,
        run.outputs.len(),
        "replay over subsets is bit-identical to the native run"
    );

    // Subset sizes: each subset is smaller than its source, and the whole
    // set stays a few percent of the font files it came from.
    let mut full_bytes = 0u64;
    let mut seen = std::collections::BTreeSet::new();
    for (blob, index) in run.answers.sources() {
        let source = run.registry.blob(&blob).expect("blob").as_slice().len();
        let subset = subsets[&subset_key(&blob, index)].len();
        assert!(
            subset < source,
            "subset {blob}.{index} is {subset} bytes from {source}"
        );
        if seen.insert(blob) {
            full_bytes += source as u64;
        }
    }
    let subset_bytes: usize = subsets.values().map(Vec::len).sum();
    let percent = 100.0 * subset_bytes as f64 / full_bytes as f64;
    println!(
        "subsets: {} faces from {} font files, {:.1} MB -> {:.2} MB ({percent:.2}%)",
        subsets.len(),
        seen.len(),
        full_bytes as f64 / 1e6,
        subset_bytes as f64 / 1e6
    );
    assert!(percent < 5.0, "subsets are {percent:.2}% of their fonts");
    println!("ok");
}

#[cfg(not(all(target_os = "macos", feature = "system")))]
fn main() {
    println!("the corpus test needs the macOS CoreText registry; skipped");
}
