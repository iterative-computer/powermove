//! `compat` level 1 shape and path rasters against Chromium's Canvas2D
//! (docs/engine/vector.md, "Level 1 parity").
//!
//! The references are captured by `tools/canvas-ref` from today's code
//! (`rasterShape`, `rasterPaths`, raw canvas diagnostics) in the app's
//! Electron, as the premultiplied bytes the compositor samples. The capture
//! lives outside the repository: the test reads the capture named by
//! `PM_CANVAS_REF` or the newest complete `canvas-*` capture under
//! `~/Library/Application Support/Powermove Engine Corpus/`, and passes with
//! a note when there is none.
//!
//! Bar (the brief's 2D class): at most 0.1% of a case's pixels differ by
//! more than 1 code value, and no pixel by more than 4. Every case meets the
//! first; [`KNOWN`] lists the cases that miss the second, each a handful of
//! single flipped MSAA samples, pinned so a regression or an improvement is
//! seen.

mod canvas_cases;

/// Cases with a pixel off by more than 1 code value, as measured on
/// `canvas-20261016-075306-cbd1dcfd` (Electron 44, Chromium 152, Apple M-series):
/// (id, pixels off by more than 1, largest difference).
const KNOWN: &[(&str, usize, i32)] = &[
    ("shape-ellipse-big-s2.75", 3, 64),
    ("shape-random153-s3.75", 1, 45),
    ("paths-badge-rotated-scaled-s3", 2, 57),
    ("diag-ellipse-380", 2, 64),
    // The parity corpus's composition/metadata rectangle (radius 24).
    ("shape-corpus-rect-r24-s1", 2, 64),
    // The cards of the corpus's space/groups-25d (130 × 140, radius 24),
    // which a 3D group's plane magnifies (pm-render routes level 1 shapes
    // on 2.5D planes for this; docs/engine/composite.md).
    ("shape-groups25d-card-ff6040-s1", 2, 64),
    ("shape-groups25d-card-4060ff-s1", 2, 64),
];

#[test]
fn rasters_match_chromium_canvas2d() {
    let Some(dir) = canvas_cases::capture_dir() else {
        eprintln!("note: no canvas-* capture (run tools/canvas-ref); parity not measured");
        return;
    };
    eprintln!("capture {}", dir.display());
    let (mut exact, mut within1, mut known, mut compared) = (0, 0, 0, 0);
    let mut failures = Vec::new();
    for c in canvas_cases::cases() {
        let id = c["id"].as_str().unwrap();
        let Some(result) = canvas_cases::rasterize(&c) else { continue };
        let (w, h, ours) = match result {
            Ok(r) => r,
            Err(e) => {
                failures.push(format!("{id}: {e}"));
                continue;
            }
        };
        let (rw, rh, reference) = canvas_cases::read_png(&dir.join(format!("{id}.png")));
        if (rw, rh) != (w, h) {
            failures.push(format!("{id}: size {w}x{h}, reference {rw}x{rh}"));
            continue;
        }
        compared += 1;
        let (mut over1, mut max) = (0usize, 0i32);
        for (a, b) in ours.chunks(4).zip(reference.chunks(4)) {
            let e = a.iter().zip(b).map(|(x, y)| (i32::from(*x) - i32::from(*y)).abs()).max().unwrap();
            over1 += usize::from(e > 1);
            max = max.max(e);
        }
        let pixels = (w * h) as usize;
        if over1 * 1000 > pixels {
            failures.push(format!("{id}: {over1} of {pixels} pixels off by more than 1"));
        }
        match KNOWN.iter().find(|k| k.0 == id) {
            Some(&(_, n, m)) => {
                known += 1;
                eprintln!("known {id}: {over1} pixels off by more than 1, max {max}");
                if over1 > n || max > m {
                    failures.push(format!("{id}: {over1} pixels off by more than 1 (max {max}); known {n} (max {m})"));
                } else if max <= 4 {
                    eprintln!("note: {id} now within the bar (max {max}); remove it from KNOWN");
                }
            }
            None if max > 1 => failures.push(format!("{id}: {over1} pixels off by more than 1, max {max}")),
            None => {
                if max == 0 { exact += 1 } else { within1 += 1 }
            }
        }
    }
    eprintln!("{compared} cases: {exact} exact, {within1} within 1, {known} known exceptions");
    assert!(compared > 400, "the capture covers the cases");
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
