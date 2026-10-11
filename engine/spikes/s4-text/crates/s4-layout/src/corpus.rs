//! Corpus cases, running the level 1 layout on one case, and comparing the
//! result with today's Canvas2D capture.

use crate::canvas::CanvasMeasurer;
use crate::font::{FaceId, FontResolver, PreparedCache, tag};
use crate::legacy::{self, GlyphLayout, SourceLine, TextContent};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

/// Variable fonts the corpus drives through the `FontFace` path; the capture
/// page loads the same files (capture/page.ts `VARIABLE_SOURCES`).
pub const VARIABLE_SOURCES: &[(&str, &str)] = &[
    ("Skia", "/System/Library/Fonts/Supplemental/Skia.ttf"),
    ("STIX Two Text", "/System/Library/Fonts/Supplemental/STIXTwoText.ttf"),
    ("Kefa III", "/System/Library/Fonts/Supplemental/KefaIII.ttf"),
    ("Noto Sans Syriac", "/System/Library/Fonts/Supplemental/NotoSansSyriac-Regular.ttf"),
    ("System Font", "/System/Library/Fonts/SFNS.ttf"),
];

#[derive(Clone, Debug)]
pub struct Case {
    pub id: String,
    pub group: String,
    pub d: TextContent,
}

fn is_axis_tag(t: &str) -> bool {
    t.len() == 4 && t.bytes().all(|b| (0x20..=0x7E).contains(&b))
}

pub fn parse_case(v: &Value) -> Case {
    let id = v["id"].as_str().unwrap_or_default().to_string();
    let group = v["group"].as_str().unwrap_or_default().to_string();
    let dv = &v["d"];
    let mut d: TextContent = serde_json::from_value(dv.clone()).expect("text content");
    if let Some(obj) = dv.as_object() {
        let mut axes: Vec<(String, f64)> = obj
            .iter()
            .filter_map(|(k, v)| {
                let t = k.strip_prefix("fontAxis.")?;
                (is_axis_tag(t) && v.is_number()).then(|| (t.to_string(), v.as_f64().unwrap()))
            })
            .collect();
        axes.sort_by(|a, b| a.0.cmp(&b.0));
        d.axes = axes;
    }
    Case { id, group, d }
}

/// `JSON.stringify` writes lone surrogates as `\\udXXX` escapes, which
/// serde_json rejects. Paired surrogates are written as raw UTF-8, so every
/// surrogate escape is a lone one; map it to U+FFFD (as `String::from_utf16_lossy`
/// does on our side).
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

pub fn load_corpus(json: &str) -> Vec<Case> {
    let v: Value = serde_json::from_str(json).expect("corpus json");
    v.as_array().expect("corpus array").iter().map(parse_case).collect()
}

/// Layout output for one case, in the capture's shape.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct CaseOutput {
    pub id: String,
    pub lines: Vec<(String, usize)>,
    /// UTF-16 units of each line (lone surrogates survive here).
    pub line_units: Vec<Vec<u16>>,
    pub caret: Vec<Vec<(usize, f64)>>,
    pub layout: GlyphLayout,
}

/// Resolves the `FontFace` a case's variable axes would register.
pub fn custom_for(d: &TextContent, custom_faces: &HashMap<String, FaceId>) -> Option<(FaceId, Vec<(u32, f32)>)> {
    if d.axes.is_empty() {
        return None;
    }
    let family = d.font.as_deref()?.trim();
    let face = *custom_faces.get(family)?;
    let settings = d.axes.iter().map(|(t, v)| (tag(t.as_bytes().try_into().unwrap()), *v as f32)).collect();
    Some((face, settings))
}

pub fn run_case(resolver: &mut dyn FontResolver, cache: &mut PreparedCache, case: &Case, custom_faces: &HashMap<String, FaceId>) -> CaseOutput {
    let custom = custom_for(&case.d, custom_faces);
    let mut m = CanvasMeasurer::new(resolver, cache, &case.d, custom);
    let lh = case.d.line_height();
    let lines: Vec<SourceLine> = legacy::text_source_lines(&case.d, &mut m, lh);
    let caret = legacy::caret_boundaries(&case.d, &mut m);
    let layout = legacy::glyph_layout(&case.d, &mut m);
    CaseOutput {
        id: case.id.clone(),
        lines: lines.iter().map(|l| (String::from_utf16_lossy(&l.text), l.start)).collect(),
        line_units: lines.into_iter().map(|l| l.text).collect(),
        caret,
        layout,
    }
}

/// Comparison of one case with the capture.
#[derive(Clone, Debug, Default, Serialize)]
pub struct CaseDiff {
    pub id: String,
    pub group: String,
    pub breaks_identical: bool,
    /// Largest |dx| over caret boundaries and unit x/w, in px.
    pub max_advance_error: f64,
    pub detail: Vec<String>,
}

fn js_units(s: &str) -> Vec<u16> {
    s.encode_utf16().collect()
}

pub fn compare(case: &Case, out: &CaseOutput, expected: &Value) -> CaseDiff {
    let mut diff = CaseDiff { id: case.id.clone(), group: case.group.clone(), breaks_identical: true, ..Default::default() };
    let exp_lines = expected["lines"].as_array().cloned().unwrap_or_default();
    if exp_lines.len() != out.lines.len() {
        diff.breaks_identical = false;
        diff.detail.push(format!("line count {} vs canvas {}", out.lines.len(), exp_lines.len()));
    }
    for (i, (l, units)) in out.lines.iter().zip(out.line_units.iter()).enumerate() {
        let Some(e) = exp_lines.get(i) else { break };
        // The capture went through JSON, which replaces lone surrogates.
        let et = e["text"].as_str().unwrap_or_default();
        let es = e["start"].as_u64().unwrap_or(u64::MAX) as usize;
        let ours = String::from_utf16_lossy(units);
        if js_units(et) != js_units(&ours) || es != l.1 {
            diff.breaks_identical = false;
            diff.detail.push(format!("line {i}: {:?}@{} vs canvas {:?}@{}", ours, l.1, et, es));
        }
    }
    let mut max: f64 = 0.0;
    let note = |what: String, a: f64, b: f64, diff: &mut CaseDiff, max: &mut f64| {
        let e = (a - b).abs();
        if e > *max {
            *max = e;
        }
        if e > 0.25 && diff.detail.len() < 12 {
            diff.detail.push(format!("{what}: {a} vs canvas {b} (|d| {e:.4})"));
        }
    };
    if diff.breaks_identical {
        let exp_caret = expected["caret"].as_array().cloned().unwrap_or_default();
        for (li, line) in out.caret.iter().enumerate() {
            let Some(el) = exp_caret.get(li).and_then(|v| v.as_array()) else { continue };
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
