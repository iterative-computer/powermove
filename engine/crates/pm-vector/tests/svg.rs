//! SVG bucket rasters against Chromium's `<img>` rasters
//! (docs/engine/vector.md, "SVG images").
//!
//! The references are `rasterSvgAsset`'s canvases (`drawImage(img, 0, 0,
//! width, height)`) captured by `tools/canvas-ref` (see `tests/canvas2d.rs`
//! for where the capture lives). `SvgDocument::raster_compat` replays
//! Blink's SVG paint record through the Graphite device emulation.
//!
//! The level 1 gate (`SvgDocument::compat_supported_at`) admits a case only
//! when the replay is inside the 2D class (at most 1 code value on 99.9% of
//! pixels, at most 4 anywhere): every admitted case must be in the class.
//! The routed cases pin their measured residual so a change is seen; they
//! are blockers listed in the docs, never accepted differences, and they
//! stay routed until they reach the class. Text cases need a pm-text
//! resolver and are measured by `examples/svg_compat_compare`.

mod canvas_cases;

use std::path::{Path, PathBuf};

use pm_vector::render::svg::SvgDocument;

/// What the gate says at each case's own size: `None` when admitted, else
/// a fragment of the reason.
const GATE: &[(&str, Option<&str>)] = &[
    ("svg-solid-64x64", Some("multisampled pass")),
    ("svg-solid-200x200", Some("multisampled pass")),
    ("svg-solid-333x150", Some("multisampled pass")),
    ("svg-stroke-64x64", Some("strokes")),
    ("svg-stroke-200x200", Some("strokes")),
    ("svg-stroke-333x150", Some("strokes")),
    ("svg-opacity-64x64", None),
    ("svg-opacity-200x200", None),
    ("svg-opacity-333x150", None),
    ("svg-gradient-64x64", None),
    ("svg-gradient-200x200", None),
    ("svg-gradient-333x150", None),
    ("svg-clip-64x64", None),
    ("svg-clip-200x200", None),
    // The stretched circle clip is an elliptical oval: two of its four
    // uses intersect geometrically, the deferred clip draw goes into the
    // depth attachment (exact here, but in the multisampled pass).
    ("svg-clip-333x150", Some("depth attachment")),
    ("svg-evenodd-64x64", None),
    ("svg-evenodd-200x200", None),
    ("svg-evenodd-333x150", None),
    ("svg-blur-64x64", Some("filter")),
    ("svg-blur-200x200", Some("filter")),
    ("svg-blur-333x150", Some("filter")),
    ("svg-text-64x64", Some("text")),
    ("svg-text-200x200", Some("text")),
    ("svg-text-333x150", Some("text")),
    ("svg-transform-64x64", Some("strokes")),
    ("svg-transform-200x200", Some("strokes")),
    ("svg-transform-333x150", Some("strokes")),
];

/// Routed cases outside the class: (case id, largest difference, pixels off
/// by more than 1), measured on `canvas-20261016-075306-cbd1dcfd`.
const RESIDUAL: &[(&str, u8, usize)] = &[
    // Analytic draws sharing a multisampled pass with a tessellated draw:
    // pixels along analytic mesh edges where Chromium's pixel takes one
    // mesh triangle's fragment for every sample (not modeled; routed).
    ("svg-solid-333x150", 44, 4),
    ("svg-stroke-64x64", 15, 30),
    ("svg-stroke-333x150", 19, 135),
    // Gaussian blur: within 2 code values, too many pixels at 2 (routed).
    ("svg-blur-64x64", 2, 14),
    ("svg-blur-200x200", 2, 3804),
    ("svg-blur-333x150", 2, 5008),
];

#[test]
fn level1_gate_admits_only_measured_passes() {
    let mut seen = 0;
    for c in canvas_cases::cases() {
        if c["kind"] != "svg" {
            continue;
        }
        let id = c["id"].as_str().unwrap();
        let (w, h) = (c["width"].as_u64().unwrap() as u32, c["height"].as_u64().unwrap() as u32);
        let doc = SvgDocument::parse(c["svg"].as_str().unwrap().as_bytes()).unwrap();
        let verdict = doc.compat_supported_at(&[(w, h)]);
        let expected = GATE.iter().find(|g| g.0 == id).unwrap_or_else(|| panic!("{id}: not in GATE")).1;
        match (expected, &verdict) {
            (None, Ok(())) => {}
            (Some(why), Err(e)) => assert!(e.0.contains(why), "{id}: {e}"),
            _ => panic!("{id}: expected {expected:?}, got {verdict:?}"),
        }
        seen += 1;
    }
    assert_eq!(seen, GATE.len());
}

#[test]
fn bucket_rasters_against_chromium() {
    let Some(dir) = canvas_cases::capture_dir() else {
        eprintln!("note: no canvas-* capture (run tools/canvas-ref); SVG rasters not measured");
        return;
    };
    let mut failures = Vec::new();
    let (mut compared, mut admitted) = (0, 0);
    for c in canvas_cases::cases() {
        if c["kind"] != "svg" {
            continue;
        }
        let id = c["id"].as_str().unwrap();
        let doc = SvgDocument::parse(c["svg"].as_str().unwrap().as_bytes()).unwrap();
        if doc.has_text() {
            eprintln!("{id:28} has text (measured by svg_compat_compare with pm-text)");
            continue;
        }
        let (rw, rh, reference) = canvas_cases::read_png(&dir.join(format!("{id}.png")));
        let gate = doc.compat_supported_at(&[(rw, rh)]).is_ok();
        let ours = doc.raster_compat(rw, rh).unwrap_or_else(|e| panic!("{id}: {e}"));
        let (mut max, mut over1) = (0u8, 0usize);
        for (a, b) in ours.rgba.chunks(4).zip(reference.chunks(4)) {
            let e = a.iter().zip(b).map(|(x, y)| x.abs_diff(*y)).max().unwrap();
            max = max.max(e);
            over1 += usize::from(e > 1);
        }
        compared += 1;
        let pixels = (rw * rh) as usize;
        let class = max <= 4 && over1 * 1000 <= pixels;
        eprintln!(
            "{id:28} max {max} over1 {over1}/{pixels} {} {}",
            if class { "2D class" } else { "outside" },
            if gate { "drawn" } else { "routed" }
        );
        if gate {
            admitted += 1;
            if !class {
                failures.push(format!("{id}: drawn by level 1 but outside the 2D class (max {max}, {over1} pixels over 1)"));
            }
            continue;
        }
        match RESIDUAL.iter().find(|r| r.0 == id) {
            None => {}
            Some(&(_, m, n)) if class => failures.push(format!("{id}: now in the 2D class; drop its residual pin ({m}, {n})")),
            Some(&(_, m, n)) if max <= m && over1 <= n => {}
            Some(&(_, m, n)) => failures.push(format!("{id}: max {max} (pinned {m}), {over1} over 1 (pinned {n})")),
        }
    }
    assert_eq!(compared, 24, "the capture covers the SVG cases without text");
    assert_eq!(admitted, GATE.iter().filter(|g| g.1.is_none()).count());
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// The level 1 gate reference documents (`tools/canvas-ref/svg-gate-cases.ts`,
/// one per construct the source gate admits).
fn gate_cases() -> Vec<serde_json::Value> {
    let fixture: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/svg-gate-cases.json")).unwrap(),
    )
    .unwrap();
    fixture["cases"].as_array().unwrap().clone()
}

/// `PM_SVG_GATE_REF`, or the newest complete `svg-gate-*` capture under
/// `~/Library/Application Support/Powermove Engine Corpus/`.
fn gate_capture() -> Option<PathBuf> {
    if let Ok(dir) = std::env::var("PM_SVG_GATE_REF") {
        return Some(PathBuf::from(dir));
    }
    let root = Path::new(&std::env::var("HOME").ok()?).join("Library/Application Support/Powermove Engine Corpus");
    let ids: Vec<String> = gate_cases().iter().map(|c| c["id"].as_str().unwrap().to_owned()).collect();
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(root)
        .ok()?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| {
            p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with("svg-gate-"))
                && ids.iter().all(|id| p.join(format!("{id}.png")).is_file())
        })
        .collect();
    dirs.sort();
    dirs.pop()
}

/// The gate reference cases the gate refuses at their own size, with a
/// fragment of the reason (every other case is admitted). Each is outside
/// what the replay is known to draw, or a value no reference measures.
const GATE_REFUSED: &[(&str, &str)] = &[
    // A clip the classifier sends to the depth attachment (an oval clip
    // stretched by the bucket, a rotated rect clip, a path clip).
    ("gate-xform-group-clip-97x61", "depth attachment"),
    ("gate-xform-clip-rotated-", "depth attachment"),
    ("gate-clip-ellipse-", "depth attachment"),
    ("gate-clip-polygon-", "depth attachment"),
    ("gate-clip-rule-", "depth attachment"),
    // Five stops after padding (inside the class, but unmeasured beyond four).
    ("gate-grad-stops-", "four stops"),
    // A five-digit hex color (an invalid color both engines ignore, but
    // outside the measured syntaxes).
    ("gate-paint-invalid-", "#12345"),
    // rx="auto": Blink draws square corners.
    ("gate-shape-rect-radii-", "rx=\"auto\""),
    // Graphite draws filled nested rects with drawDRRect.
    ("gate-path-evenodd-style-", "nested rects"),
];

fn gate_expectation(id: &str) -> Option<&'static str> {
    GATE_REFUSED.iter().find(|(p, _)| id.starts_with(p)).map(|(_, why)| *why)
}

/// The gate's verdict on every reference document at its own size, and
/// over its asset's bucket ladder when admitted there.
#[test]
fn gate_reference_verdicts() {
    let cases = gate_cases();
    assert!(cases.len() >= 380, "{} gate reference cases", cases.len());
    let mut wrong = Vec::new();
    for c in &cases {
        let id = c["id"].as_str().unwrap();
        let (w, h) = (c["width"].as_u64().unwrap() as u32, c["height"].as_u64().unwrap() as u32);
        let doc = SvgDocument::parse(c["svg"].as_str().unwrap().as_bytes()).unwrap();
        let verdict = doc.compat_supported_at(&[(w, h)]);
        match (gate_expectation(id), &verdict) {
            (None, Ok(())) => {}
            (Some(why), Err(e)) if e.0.contains(why) => {}
            (expected, got) => wrong.push(format!("{id}: expected {expected:?}, got {got:?}")),
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

/// Every reference document the gate admits is inside the 2D class
/// against Chromium (`canvas-ref/run.ts --svg-gate`).
#[test]
fn gate_references_against_chromium() {
    let Some(dir) = gate_capture() else {
        eprintln!("note: no svg-gate-* capture (run tools/canvas-ref/run.ts --svg-gate); gate references not measured");
        return;
    };
    let mut failures = Vec::new();
    let (mut admitted, mut compared) = (0, 0);
    for c in gate_cases() {
        let id = c["id"].as_str().unwrap();
        let doc = SvgDocument::parse(c["svg"].as_str().unwrap().as_bytes()).unwrap();
        let (rw, rh, reference) = canvas_cases::read_png(&dir.join(format!("{id}.png")));
        let gate = doc.compat_supported_at(&[(rw, rh)]).is_ok();
        let ours = doc.raster_compat(rw, rh).unwrap_or_else(|e| panic!("{id}: {e}"));
        let (mut max, mut over1) = (0u8, 0usize);
        for (a, b) in ours.rgba.chunks(4).zip(reference.chunks(4)) {
            let e = a.iter().zip(b).map(|(x, y)| x.abs_diff(*y)).max().unwrap();
            max = max.max(e);
            over1 += usize::from(e > 1);
        }
        compared += 1;
        let class = max <= 4 && over1 * 1000 <= (rw * rh) as usize;
        if gate {
            admitted += 1;
            if !class {
                failures.push(format!("{id}: admitted but outside the 2D class (max {max}, {over1} pixels over 1)"));
            }
        } else {
            eprintln!("{id:44} routed: max {max}, {over1} over 1");
        }
    }
    eprintln!("{compared} gate references compared, {admitted} admitted");
    assert_eq!(compared, gate_cases().len());
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
