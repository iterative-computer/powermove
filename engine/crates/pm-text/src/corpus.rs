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
use crate::registry::ProjectDescriptors;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

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
            if i + 5 < b.len()
                && b[i + 1] == b'u'
                && (b[i + 2] == b'd' || b[i + 2] == b'D')
                && matches!(b[i + 3], b'8'..=b'9' | b'a'..=b'f' | b'A'..=b'F')
            {
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

/// A project font the corpus registers (`FontFace(family, file, {weight, style})`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ProjectFontSpec {
    pub family: String,
    pub path: String,
    /// CSS `weight` descriptor: one weight or a `lo hi` range.
    pub weight: String,
    /// `normal` or `italic`.
    pub style: String,
}

impl ProjectFontSpec {
    pub fn descriptors(&self) -> ProjectDescriptors {
        ProjectDescriptors::from_css(&self.weight, &self.style, "").unwrap_or_default()
    }
}

/// The corpus file `capture/corpus.ts` writes.
#[derive(Clone, Debug)]
pub struct CorpusFile {
    /// `PM.fontSource` sources for the variable-font path, by family.
    pub variable_sources: BTreeMap<String, String>,
    pub project_fonts: Vec<ProjectFontSpec>,
    pub cases: Vec<Case>,
}

impl CorpusFile {
    /// Parses the corpus JSON (lone surrogates are mapped to U+FFFD, see
    /// `sanitize_lone_surrogates`).
    pub fn parse(json: &str) -> Result<CorpusFile, serde_json::Error> {
        let v: Value = serde_json::from_str(&sanitize_lone_surrogates(json))?;
        Self::from_value(&v)
    }

    pub fn from_value(v: &Value) -> Result<CorpusFile, serde_json::Error> {
        Ok(CorpusFile {
            variable_sources: serde_json::from_value(v["variableSources"].clone())?,
            project_fonts: serde_json::from_value(v["projectFonts"].clone())?,
            cases: v["cases"]
                .as_array()
                .map(|a| a.iter().map(parse_case).collect())
                .unwrap_or_default(),
        })
    }
}

/// The native registry configured as the capture page was: the corpus's
/// variable sources (`PM.fontSource`) and project fonts (`FontFace`s).
#[cfg(all(target_os = "macos", feature = "system"))]
pub fn native_registry(corpus: &CorpusFile) -> Result<crate::registry::CoreTextRegistry, String> {
    let mut r = crate::registry::CoreTextRegistry::new();
    for p in &corpus.project_fonts {
        let bytes = std::fs::read(&p.path).map_err(|e| format!("{}: {e}", p.path))?;
        r.add_project_font(&p.family, &bytes, 0, p.descriptors())
            .map_err(|e| format!("{}: {e}", p.path))?;
    }
    for (family, path) in &corpus.variable_sources {
        let bytes = std::fs::read(path).map_err(|e| format!("{path}: {e}"))?;
        r.set_font_source(family, &bytes)
            .map_err(|e| format!("{path}: {e}"))?;
    }
    Ok(r)
}

/// The corpus cases of a corpus file, for hosts that only lay cases out.
pub fn load_corpus(json: &str) -> Vec<Case> {
    CorpusFile::parse(json).expect("corpus json").cases
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
        lines: lines
            .iter()
            .map(|l| (String::from_utf16_lossy(&l.text), l.start))
            .collect(),
        line_units: lines.into_iter().map(|l| l.text).collect(),
        caret,
        layout,
    }
}

/// Lays the corpus out with a replay resolver over `answers` (the code
/// `pm-wasm` runs). `font` returns the bytes for a face source and the face
/// index to read them at (0 for subsets). Returns every case's layout and
/// the queries the answers lacked.
pub fn run_replay(
    answers: &Answers,
    mut font: impl FnMut(&str, u32) -> (FontBytes, u32),
    cases: &[Case],
) -> (Vec<CaseOutput>, usize, usize) {
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

/// A unit as the fixture stores it: `[x, y, w, line, sourceStart]`, with
/// x, y, and w in units of 1/`ROUND` px.
pub type UnitRow = [i64; 5];

/// What Canvas2D produced for one case, in the fixture's compact shape.
/// Positions are integers in 1/`ROUND` px (rounding adds at most 0.00005 px,
/// far below the 0.25 px tolerance and the 0.001 px the Blink port reaches).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Expected {
    /// Lines as `[text, sourceStart]`.
    pub lines: Vec<(String, usize)>,
    /// Per line, `[sourceIndex, width(prefix)]` (width in 1/`ROUND` px).
    pub caret: Vec<Vec<(usize, i64)>>,
    pub characters: Vec<UnitRow>,
    pub words: Vec<UnitRow>,
    #[serde(rename = "lineUnits")]
    pub line_units: Vec<UnitRow>,
}

pub const ROUND: f64 = 10_000.0;

fn fixed(v: f64) -> i64 {
    (v * ROUND).round() as i64
}

fn px(v: i64) -> f64 {
    v as f64 / ROUND
}

impl Expected {
    /// From one result of the capture page (`{lines, caret, layout}`).
    pub fn from_capture(r: &Value) -> Option<Expected> {
        if r.get("error").is_some() {
            return None;
        }
        let units = |kind: &str| -> Vec<UnitRow> {
            r["layout"][kind]
                .as_array()
                .map(|a| {
                    a.iter()
                        .map(|u| {
                            let n = |k: &str| u[k].as_f64().unwrap_or(f64::NAN);
                            let i = |k: &str| u[k].as_i64().unwrap_or(-1);
                            [
                                fixed(n("x")),
                                fixed(n("y")),
                                fixed(n("w")),
                                i("line"),
                                i("sourceStart"),
                            ]
                        })
                        .collect()
                })
                .unwrap_or_default()
        };
        Some(Expected {
            lines: r["lines"]
                .as_array()?
                .iter()
                .map(|l| {
                    (
                        l["text"].as_str().unwrap_or_default().to_string(),
                        l["start"].as_u64().unwrap_or(u64::MAX) as usize,
                    )
                })
                .collect(),
            caret: r["caret"]
                .as_array()?
                .iter()
                .map(|line| {
                    line.as_array()
                        .map(|b| {
                            b.iter()
                                .map(|p| {
                                    (
                                        p[0].as_u64().unwrap_or(u64::MAX) as usize,
                                        fixed(p[1].as_f64().unwrap_or(f64::NAN)),
                                    )
                                })
                                .collect()
                        })
                        .unwrap_or_default()
                })
                .collect(),
            characters: units("characters"),
            words: units("words"),
            line_units: units("lines"),
        })
    }
}

fn js_units(s: &str) -> Vec<u16> {
    s.encode_utf16().collect()
}

/// Compares a case's layout with what Canvas2D produced.
pub fn compare(case: &Case, out: &CaseOutput, expected: &Expected) -> CaseDiff {
    let mut diff = CaseDiff {
        id: case.id.clone(),
        group: case.group.clone(),
        breaks_identical: true,
        ..Default::default()
    };
    if expected.lines.len() != out.lines.len() {
        diff.breaks_identical = false;
        diff.detail.push(format!(
            "line count {} vs canvas {}",
            out.lines.len(),
            expected.lines.len()
        ));
    }
    for (i, (l, units)) in out.lines.iter().zip(out.line_units.iter()).enumerate() {
        let Some((et, es)) = expected.lines.get(i) else {
            break;
        };
        let ours = String::from_utf16_lossy(units);
        if js_units(et) != js_units(&ours) || *es != l.1 {
            diff.breaks_identical = false;
            diff.detail
                .push(format!("line {i}: {ours:?}@{} vs canvas {et:?}@{es}", l.1));
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
            diff.detail
                .push(format!("{what}: {a} vs canvas {b} (|d| {e:.4})"));
        }
    };
    if diff.breaks_identical {
        for (li, line) in out.caret.iter().enumerate() {
            let Some(el) = expected.caret.get(li) else {
                diff.breaks_identical = false;
                diff.detail
                    .push(format!("caret line {li} missing from capture"));
                continue;
            };
            if el.len() != line.len() {
                diff.breaks_identical = false;
                diff.detail.push(format!(
                    "caret line {li}: {} boundaries vs canvas {}",
                    line.len(),
                    el.len()
                ));
            }
            for (bi, ((idx, x), (eidx, ex))) in line.iter().zip(el.iter()).enumerate() {
                if idx != eidx {
                    diff.breaks_identical = false;
                    diff.detail
                        .push(format!("caret {li}.{bi} index {idx} vs {eidx}"));
                }
                note(format!("caret {li}.{bi}"), *x, px(*ex), &mut diff, &mut max);
            }
        }
        for (kind, ours, theirs) in [
            ("characters", &out.layout.characters, &expected.characters),
            ("words", &out.layout.words, &expected.words),
            ("lines", &out.layout.lines, &expected.line_units),
        ] {
            if theirs.len() != ours.len() {
                diff.breaks_identical = false;
                diff.detail.push(format!(
                    "{kind} count {} vs canvas {}",
                    ours.len(),
                    theirs.len()
                ));
                continue;
            }
            for (i, (u, e)) in ours.iter().zip(theirs.iter()).enumerate() {
                if e[4] != u.source_start as i64 || e[3] != u.line as i64 {
                    diff.breaks_identical = false;
                    diff.detail.push(format!(
                        "{kind} {i} source {} line {} vs canvas {} {}",
                        u.source_start, u.line, e[4], e[3]
                    ));
                }
                note(format!("{kind} {i} x"), u.x, px(e[0]), &mut diff, &mut max);
                note(format!("{kind} {i} y"), u.y, px(e[1]), &mut diff, &mut max);
                note(format!("{kind} {i} w"), u.w, px(e[2]), &mut diff, &mut max);
            }
        }
    }
    diff.max_advance_error = max;
    diff
}

/// Units in columns, each column delta-coded from the previous unit (x,
/// line, and source start) or absolute (w), so the fixture compresses well.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct PackedUnits {
    pub x: Vec<i64>,
    pub w: Vec<i64>,
    pub line: Vec<i64>,
    pub src: Vec<i64>,
}

/// `Expected` as the fixture stores it. Every unit on a row has the same y
/// (`row × lineHeight`), so y is stored once per row (`None` for rows
/// without units).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Packed {
    pub lines: Vec<(String, usize)>,
    pub ys: Vec<Option<i64>>,
    /// Per line, delta-coded `[index, x]` pairs as two columns.
    pub caret: Vec<(Vec<i64>, Vec<i64>)>,
    pub characters: PackedUnits,
    pub words: PackedUnits,
    #[serde(rename = "lineUnits")]
    pub line_units: PackedUnits,
}

fn deltas(v: impl Iterator<Item = i64>) -> Vec<i64> {
    let mut prev = 0;
    v.map(|x| {
        let d = x - prev;
        prev = x;
        d
    })
    .collect()
}

fn sums(v: &[i64]) -> Vec<i64> {
    let mut acc = 0;
    v.iter()
        .map(|d| {
            acc += d;
            acc
        })
        .collect()
}

impl Packed {
    /// Packs a capture result. Fails if units on one row disagree on y.
    pub fn pack(e: &Expected) -> Result<Packed, String> {
        let rows = e.lines.len();
        let mut ys: Vec<Option<i64>> = vec![None; rows];
        for u in e.characters.iter().chain(&e.words).chain(&e.line_units) {
            let row = u[3] as usize;
            let slot = ys
                .get_mut(row)
                .ok_or_else(|| format!("unit on row {row} of {rows}"))?;
            match slot {
                Some(y) if *y != u[1] => {
                    return Err(format!("row {row} has units at y {} and {}", y, u[1]));
                }
                _ => *slot = Some(u[1]),
            }
        }
        let units = |rows: &[UnitRow]| PackedUnits {
            x: deltas(rows.iter().map(|r| r[0])),
            w: rows.iter().map(|r| r[2]).collect(),
            line: deltas(rows.iter().map(|r| r[3])),
            src: deltas(rows.iter().map(|r| r[4])),
        };
        Ok(Packed {
            lines: e.lines.clone(),
            ys,
            caret: e
                .caret
                .iter()
                .map(|l| {
                    (
                        deltas(l.iter().map(|p| p.0 as i64)),
                        deltas(l.iter().map(|p| p.1)),
                    )
                })
                .collect(),
            characters: units(&e.characters),
            words: units(&e.words),
            line_units: units(&e.line_units),
        })
    }

    pub fn unpack(&self) -> Expected {
        let units = |p: &PackedUnits| -> Vec<UnitRow> {
            let (x, line, src) = (sums(&p.x), sums(&p.line), sums(&p.src));
            (0..p.w.len())
                .map(|i| {
                    let y = self
                        .ys
                        .get(line[i] as usize)
                        .copied()
                        .flatten()
                        .unwrap_or(i64::MIN);
                    [x[i], y, p.w[i], line[i], src[i]]
                })
                .collect()
        };
        Expected {
            lines: self.lines.clone(),
            caret: self
                .caret
                .iter()
                .map(|(i, x)| {
                    sums(i)
                        .into_iter()
                        .zip(sums(x))
                        .map(|(i, x)| (i as usize, x))
                        .collect()
                })
                .collect(),
            characters: units(&self.characters),
            words: units(&self.words),
            line_units: units(&self.line_units),
        }
    }
}

/// One case of the checked-in fixture: the case and what Canvas2D produced.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FixtureCase {
    pub id: String,
    pub group: String,
    /// The text content exactly as the capture page received it.
    pub d: Value,
    pub expected: Packed,
}

/// A font file the native run read, to tell font drift from layout drift.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FontFile {
    pub path: String,
    pub bytes: u64,
}

/// The checked-in Canvas2D expectations (`tests/fixtures/canvas2d-corpus.json.gz`).
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Fixture {
    pub user_agent: String,
    pub os: String,
    pub variable_sources: BTreeMap<String, String>,
    pub project_fonts: Vec<ProjectFontSpec>,
    /// Every font file the native registry read while laying the corpus out.
    pub font_files: Vec<FontFile>,
    pub cases: Vec<FixtureCase>,
}

impl Fixture {
    /// The corpus the fixture was captured from.
    pub fn corpus(&self) -> CorpusFile {
        CorpusFile {
            variable_sources: self.variable_sources.clone(),
            project_fonts: self.project_fonts.clone(),
            cases: self
                .cases
                .iter()
                .map(|c| Case {
                    id: c.id.clone(),
                    group: c.group.clone(),
                    d: TextContent::from_json(&c.d),
                })
                .collect(),
        }
    }

    /// Font files whose size changed or that are missing on this machine.
    pub fn font_drift(&self) -> Vec<String> {
        self.font_files
            .iter()
            .filter_map(|f| match std::fs::metadata(&f.path) {
                Ok(m) if m.len() == f.bytes => None,
                Ok(m) => Some(format!(
                    "{} is {} bytes, captured with {}",
                    f.path,
                    m.len(),
                    f.bytes
                )),
                Err(e) => Some(format!("{}: {e}", f.path)),
            })
            .collect()
    }
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
    let mut s = Summary {
        cases: diffs.len(),
        ..Default::default()
    };
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
    s.p99_advance_error = errors
        .get(((errors.len() as f64 - 1.0) * 0.99).round().max(0.0) as usize)
        .copied()
        .unwrap_or(0.0);
    s
}

/// Metrics-and-shaping subsets for every face source the answers name, by
/// `answers::subset_key`. `font` returns a source's full bytes.
pub fn subsets_for(
    answers: &Answers,
    mut font: impl FnMut(&str) -> Option<FontBytes>,
) -> Result<BTreeMap<String, Vec<u8>>, String> {
    let mut out = BTreeMap::new();
    for (blob, index) in answers.sources() {
        let data = font(&blob).ok_or_else(|| format!("no bytes for blob {blob}"))?;
        let (bytes, _) = crate::subset::metrics_subset(data.as_slice(), index)
            .map_err(|e| format!("{blob}.{index}: {e}"))?;
        out.insert(crate::answers::subset_key(&blob, index), bytes);
    }
    Ok(out)
}

/// A native corpus run over the CoreText registry, with every registry
/// answer recorded.
#[cfg(all(target_os = "macos", feature = "system"))]
pub struct NativeRun {
    pub outputs: Vec<CaseOutput>,
    pub answers: Answers,
    pub registry: crate::registry::CoreTextRegistry,
    pub seconds: f64,
}

/// Lays the corpus out natively, in corpus order (Blink's fallback cache
/// depends on it), as the capture page measured it. Must run on the main
/// thread.
#[cfg(all(target_os = "macos", feature = "system"))]
pub fn run_native(corpus: &CorpusFile) -> Result<NativeRun, String> {
    let registry = native_registry(corpus)?;
    let mut compat = Compat::new(crate::answers::Recorder::new(registry));
    let started = std::time::Instant::now();
    let outputs = corpus
        .cases
        .iter()
        .map(|c| run_case(&mut compat, c))
        .collect();
    let seconds = started.elapsed().as_secs_f64();
    let recorder = compat.into_resolver();
    let answers = recorder.answers();
    Ok(NativeRun {
        outputs,
        answers,
        registry: recorder.inner,
        seconds,
    })
}

/// What level 2 layout needs to lay the corpus out: the fallback manifest
/// and every face, keyed by `answers::subset_key`, with the family to add it
/// under, in the order to add them.
#[cfg(feature = "level2")]
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Level2Inputs {
    pub manifest: crate::registry::FallbackManifest,
    pub fonts: Vec<Level2Font>,
}

#[cfg(feature = "level2")]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Level2Font {
    pub key: String,
    pub family: String,
}

/// Level 2 inputs for the corpus from the native registry: the manifest
/// for the system UI font and the faces of every family the cases name.
#[cfg(all(target_os = "macos", feature = "system", feature = "level2"))]
pub fn level2_inputs(
    registry: &mut crate::registry::CoreTextRegistry,
    cases: &[Case],
) -> Level2Inputs {
    let manifest = registry.fallback_manifest("system-ui");
    let mut fonts: Vec<Level2Font> = Vec::new();
    let mut push = |family: &str, blob: &str, index: u32| {
        let f = Level2Font {
            key: crate::answers::subset_key(blob, index),
            family: family.to_string(),
        };
        if !fonts.contains(&f) {
            fonts.push(f);
        }
    };
    for f in &manifest.faces {
        push(&f.family, &f.source.blob, f.source.index);
    }
    let families: std::collections::BTreeSet<String> = cases
        .iter()
        .filter_map(|c| {
            c.d.font
                .as_deref()
                .map(str::trim)
                .filter(|f| !f.is_empty())
                .map(str::to_string)
        })
        .collect();
    for family in families {
        for source in registry.family_faces(&family) {
            push(&family, &source.blob, source.index);
        }
    }
    Level2Inputs { manifest, fonts }
}

/// Lays the cases out with level 2 over the inputs' fonts. `font` returns
/// the bytes for a key (subsets in `pm-wasm` and in the identity check).
#[cfg(feature = "level2")]
pub fn run_level2(
    inputs: &Level2Inputs,
    mut font: impl FnMut(&str) -> Option<FontBytes>,
    cases: &[Case],
) -> Result<Vec<(String, crate::level2::Level2Layout)>, String> {
    let mut l2 = crate::level2::Level2::new();
    for f in &inputs.fonts {
        let data = font(&f.key).ok_or_else(|| format!("no font {}", f.key))?;
        l2.add_font(&f.key, data, &f.family);
    }
    l2.set_manifest(&inputs.manifest);
    Ok(cases
        .iter()
        .map(|c| (c.id.clone(), l2.layout(&c.d)))
        .collect())
}

impl Fixture {
    pub fn from_json(json: &str) -> Result<Fixture, serde_json::Error> {
        serde_json::from_str(&sanitize_lone_surrogates(json))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packing_round_trips() {
        let e = Expected {
            lines: vec![("ab".into(), 0), ("".into(), 3), ("c".into(), 4)],
            caret: vec![
                vec![(0, 0), (1, 1234), (2, 2468)],
                vec![(3, 0)],
                vec![(4, 0), (5, 999)],
            ],
            characters: vec![
                [0, 0, 1234, 0, 0],
                [1234, 0, 1234, 0, 1],
                [-5, 4600, 999, 2, 4],
            ],
            words: vec![[0, 0, 2468, 0, 0], [-5, 4600, 999, 2, 4]],
            line_units: vec![[0, 0, 2468, 0, 0], [0, 4600, 999, 2, 4]],
        };
        let p = Packed::pack(&e).unwrap();
        assert_eq!(p.ys, vec![Some(0), None, Some(4600)]);
        assert_eq!(p.unpack(), e);
        let mut bad = e.clone();
        bad.words[0][1] = 7;
        assert!(Packed::pack(&bad).is_err());
    }
}
