//! The S4 layout corpus: cases, running the level 1 layout on them, and
//! comparing the result with a Canvas2D capture (plan section 12's gate:
//! identical line breaks, advances within 0.25 px).
//!
//! The corpus and the capture come from `capture/` (a hidden Electron host
//! that measures every case with the legacy code itself); the checked-in
//! expectations are in `tests/fixtures/`.

use crate::answers::{Answers, ReplayResolver};
use crate::compat::{Compat, GlyphLayout, Measure, TextContent, wrap};
use crate::font::{FontBytes, FontResolver};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Variable fonts the corpus drives through the `FontFace` path. The capture
/// page loads these files; the registry must resolve the same families to
/// the same files (`variable_face`).
pub const VARIABLE_SOURCES: &[(&str, &str)] = &[
    ("Skia", "/System/Library/Fonts/Supplemental/Skia.ttf"),
    ("STIX Two Text", "/System/Library/Fonts/Supplemental/STIXTwoText.ttf"),
    ("Kefa III", "/System/Library/Fonts/Supplemental/KefaIII.ttf"),
    ("Noto Sans Syriac", "/System/Library/Fonts/Supplemental/NotoSansSyriac-Regular.ttf"),
    ("System Font", "/System/Library/Fonts/SFNS.ttf"),
];

pub const TOLERANCE_PX: f64 = 0.25;

#[derive(Clone, Debug)]
pub struct Case {
    pub id: String,
    pub group: String,
    pub d: TextContent,
}

pub fn parse_case(v: &Value) -> Case {
    Case {
        id: v["id"].as_str().unwrap_or_default().to_string(),
        group: v["group"].as_str().unwrap_or_default().to_string(),
        d: TextContent::from_json(&v["d"]),
    }
}

/// `JSON.stringify` writes lone surrogates as `\udXXX` escapes, which
/// serde_json rejects. Paired surrogates are written as raw UTF-8, so every
/// surrogate escape is a lone one; map it to U+FFFD. A lone surrogate is one
/// UTF-16 unit, as U+FFFD is, so offsets are unchanged.
pub fn sanitize_lone_surrogates(json: &str) -> String {
    let b = json.as_bytes();
    let mut out = String::with_capacity(json.len());
    let mut i = 0;
    let mut last = 0;
    while i < b.len() {
        if b[i] == b'\\' {
            if i + 5 < b.len() && b[i + 1] == b'u' && (b[i + 2] == b'd' || b[i + 2] == b'D') && matches!(b[i + 3], b'8'..=b'9' | b'a'..=b'f' | b'A'..=b'F') {
                out.push_str(&json[last..i]);
                out.push_str("\\ufffd");
                i += 6;
                last = i;
                continue;
            }
            i += 2;
            continue;
        }
        i += 1;
    }
    out.push_str(&json[last..]);
    out
}

/// Cases from the corpus JSON array. The capture tool writes lone surrogates
/// that JSON cannot carry; the corpus keeps them as `\udXXX` escapes and the
/// cases carry U+FFFD instead (`sanitize_lone_surrogates`).
pub fn load_corpus(json: &str) -> Vec<Case> {
    let v: Value = serde_json::from_str(&sanitize_lone_surrogates(json)).expect("corpus json");
    v.as_array().expect("corpus array").iter().map(parse_case).collect()
}

/// Layout output for one case, in the capture's shape.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct CaseOutput {
    pub id: String,
    pub lines: Vec<(String, usize)>,
    /// UTF-16 units of each line.
    pub line_units: Vec<Vec<u16>>,
    /// Per line, `[sourceIndex, width(prefix)]` (x relative to the line's
    /// alignment start).
    pub caret: Vec<Vec<(usize, f64)>>,
    pub layout: GlyphLayout,
}

pub fn run_case<R: FontResolver>(compat: &mut Compat<R>, case: &Case) -> CaseOutput {
    let lh = case.d.line_height();
    let mut m = compat.measurer(&case.d);
    let lines = wrap::text_source_lines(&case.d, &mut m, lh);
    let caret = lines
        .iter()
        .map(|line| {
            let mut out = vec![(line.start, 0.0)];
            for r in wrap::graphemes(&line.text) {
                out.push((line.start + r.end, m.width(&line.text[..r.end])));
            }
            out
        })
        .collect();
    let layout = wrap::glyph_layout(&case.d, &mut m);
    CaseOutput {
        id: case.id.clone(),
        lines: lines.iter().map(|l| (String::from_utf16_lossy(&l.text), l.start)).collect(),
        line_units: lines.into_iter().map(|l| l.text).collect(),
        caret,
        layout,
    }
}

/// Lays the corpus out with a replay resolver over `answers` (the code
/// `pm-wasm` runs). `font` returns the bytes for a face source and the face
/// index to read them at (0 for subsets). Returns every case's layout and
/// the queries the answers lacked.
pub fn run_replay(answers: &Answers, mut font: impl FnMut(&str, u32) -> (FontBytes, u32), cases: &[Case]) -> (Vec<CaseOutput>, usize, usize) {
    let mut resolver = ReplayResolver::new(answers);
    for (blob, index) in answers.sources() {
        let (data, read_index) = font(&blob, index);
        resolver.add_font(&blob, index, data, read_index);
    }
    let mut compat = Compat::new(resolver);
    let outputs = cases.iter().map(|c| run_case(&mut compat, c)).collect();
    let resolver = compat.into_resolver();
    (outputs, resolver.misses.len(), resolver.pending_blobs.len())
}

/// Comparison of one case with the capture.
#[derive(Clone, Debug, Default, Serialize)]
pub struct CaseDiff {
    pub id: String,
    pub group: String,
    pub breaks_identical: bool,
    /// Largest |dx| over caret boundaries and unit x/w/y, in px.
    pub max_advance_error: f64,
    pub detail: Vec<String>,
}

impl CaseDiff {
    pub fn passes(&self) -> bool {
        self.breaks_identical && self.max_advance_error <= TOLERANCE_PX
    }
}

fn js_units(s: &str) -> Vec<u16> {
    s.encode_utf16().collect()
}

/// Compares a case's layout with its capture (`{lines, caret, layout}` as
/// the capture page writes them).
pub fn compare(case: &Case, out: &CaseOutput, expected: &Value) -> CaseDiff {
    let mut diff = CaseDiff { id: case.id.clone(), group: case.group.clone(), breaks_identical: true, ..Default::default() };
    if let Some(e) = expected.get("error") {
        diff.breaks_identical = false;
        diff.detail.push(format!("capture error: {e}"));
        return diff;
    }
    let exp_lines = expected["lines"].as_array().cloned().unwrap_or_default();
    if exp_lines.len() != out.lines.len() {
        diff.breaks_identical = false;
        diff.detail.push(format!("line count {} vs canvas {}", out.lines.len(), exp_lines.len()));
    }
    for (i, (l, units)) in out.lines.iter().zip(out.line_units.iter()).enumerate() {
        let Some(e) = exp_lines.get(i) else { break };
        let et = e["text"].as_str().unwrap_or_default();
        let es = e["start"].as_u64().unwrap_or(u64::MAX) as usize;
        let ours = String::from_utf16_lossy(units);
        if js_units(et) != js_units(&ours) || es != l.1 {
            diff.breaks_identical = false;
            diff.detail.push(format!("line {i}: {ours:?}@{} vs canvas {et:?}@{es}", l.1));
        }
    }
    let mut max: f64 = 0.0;
    let note = |what: String, a: f64, b: f64, diff: &mut CaseDiff, max: &mut f64| {
        let e = (a - b).abs();
        let e = if e.is_nan() { f64::INFINITY } else { e };
        if e > *max {
            *max = e;
        }
        if e > TOLERANCE_PX && diff.detail.len() < 12 {
            diff.detail.push(format!("{what}: {a} vs canvas {b} (|d| {e:.4})"));
        }
    };
    if diff.breaks_identical {
        let exp_caret = expected["caret"].as_array().cloned().unwrap_or_default();
        for (li, line) in out.caret.iter().enumerate() {
            let Some(el) = exp_caret.get(li).and_then(|v| v.as_array()) else {
                diff.breaks_identical = false;
                diff.detail.push(format!("caret line {li} missing from capture"));
                continue;
            };
            if el.len() != line.len() {
                diff.breaks_identical = false;
                diff.detail.push(format!("caret line {li}: {} boundaries vs canvas {}", line.len(), el.len()));
            }
            for (bi, (idx, x)) in line.iter().enumerate() {
                let Some(eb) = el.get(bi) else { continue };
                if eb[0].as_u64() != Some(*idx as u64) {
                    diff.breaks_identical = false;
                    diff.detail.push(format!("caret {li}.{bi} index {idx} vs {}", eb[0]));
                }
                note(format!("caret {li}.{bi}"), *x, eb[1].as_f64().unwrap_or(f64::NAN), &mut diff, &mut max);
            }
        }
        let layout = &expected["layout"];
        for (kind, ours) in [("characters", &out.layout.characters), ("words", &out.layout.words), ("lines", &out.layout.lines)] {
            let theirs = layout[kind].as_array().cloned().unwrap_or_default();
            if theirs.len() != ours.len() {
                diff.breaks_identical = false;
                diff.detail.push(format!("{kind} count {} vs canvas {}", ours.len(), theirs.len()));
                continue;
            }
            for (i, (u, e)) in ours.iter().zip(theirs.iter()).enumerate() {
                if e["sourceStart"].as_u64() != Some(u.source_start as u64) || e["line"].as_u64() != Some(u.line as u64) {
                    diff.breaks_identical = false;
                    diff.detail.push(format!("{kind} {i} source {} line {} vs canvas {} {}", u.source_start, u.line, e["sourceStart"], e["line"]));
                }
                note(format!("{kind} {i} x"), u.x, e["x"].as_f64().unwrap_or(f64::NAN), &mut diff, &mut max);
                note(format!("{kind} {i} w"), u.w, e["w"].as_f64().unwrap_or(f64::NAN), &mut diff, &mut max);
                note(format!("{kind} {i} y"), u.y, e["y"].as_f64().unwrap_or(f64::NAN), &mut diff, &mut max);
            }
        }
    }
    diff.max_advance_error = max;
    diff
}

/// Per-group and overall results of a corpus comparison.
#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Summary {
    pub cases: usize,
    pub identical_breaks: usize,
    pub within_tolerance: usize,
    pub max_advance_error: f64,
    pub p99_advance_error: f64,
    pub groups: std::collections::BTreeMap<String, (usize, usize, usize, f64)>,
}

pub fn summarize(diffs: &[CaseDiff]) -> Summary {
    let mut s = Summary { cases: diffs.len(), ..Default::default() };
    let mut errors = Vec::new();
    for d in diffs {
        let g = s.groups.entry(d.group.clone()).or_default();
        g.0 += 1;
        if d.breaks_identical {
            s.identical_breaks += 1;
            g.1 += 1;
            errors.push(d.max_advance_error);
            if d.max_advance_error <= TOLERANCE_PX {
                s.within_tolerance += 1;
                g.2 += 1;
            }
            g.3 = g.3.max(d.max_advance_error);
        }
    }
    errors.sort_by(|a, b| a.total_cmp(b));
    s.max_advance_error = errors.last().copied().unwrap_or(0.0);
    s.p99_advance_error = errors.get(((errors.len() as f64 - 1.0) * 0.99).round().max(0.0) as usize).copied().unwrap_or(0.0);
    s
}
