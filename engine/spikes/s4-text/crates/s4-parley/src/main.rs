//! Can Parley measure level 1 text? Runs today's wrapping algorithm (the
//! same `legacy` port) with `measureText` widths from a Parley layout
//! instead of the Blink port, and compares every corpus case with the
//! Canvas2D capture. This is the evidence for where Parley sits in the
//! level 1 stack (see docs/engine/spikes/S4.md).
//!
//! Parley is configured the way the engine would configure it for a text
//! layer: the `fontStr(d)` family list as CSS, fontique's system collection
//! and fallback, weight `d.weight || 500`, italic, size, letter spacing from
//! tracking, explicit `fontAxis.*` variations, and `opsz` = size (Blink's
//! `font-optical-sizing: auto`). Variable fonts the corpus loads as
//! `FontFace`s are registered from the same files.
//!
//! s4-parley [--root out]  -> out/compare-parley.json

use parley::fontique::{Blob, FontInfoOverride};
use parley::{FontContext, FontFamily, FontStyle, FontVariation, FontWeight, Layout, LayoutContext, StyleProperty};
use s4_layout::corpus::{self, Case, VARIABLE_SOURCES};
use s4_layout::legacy::{self, Measure, SourceLine, TextContent};
use serde_json::{Value, json};
use std::borrow::Cow;
use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

struct ParleyMeasurer<'a> {
    fcx: &'a mut FontContext,
    lcx: &'a mut LayoutContext<()>,
    families: String,
    weight: f32,
    italic: bool,
    size: f32,
    letter_spacing: f32,
    variations: Vec<FontVariation>,
    memo: HashMap<Vec<u16>, f64>,
}

fn css_quote(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

impl<'a> ParleyMeasurer<'a> {
    fn new(fcx: &'a mut FontContext, lcx: &'a mut LayoutContext<()>, d: &TextContent, custom: Option<&str>) -> Self {
        let mut fams = Vec::new();
        if let Some(c) = custom {
            fams.push(css_quote(c));
        }
        if let Some(f) = d.font.as_deref().filter(|f| !f.is_empty()) {
            fams.push(css_quote(f));
        }
        fams.push(css_quote("SF Pro Display"));
        fams.push("-apple-system".into());
        fams.push("sans-serif".into());
        let size = d.size() as f32;
        let mut variations: Vec<FontVariation> =
            d.axes.iter().map(|(t, v)| FontVariation { tag: parley::setting::Tag::new(t.as_bytes().try_into().unwrap()), value: *v as f32 }).collect();
        if !d.axes.iter().any(|(t, _)| t == "opsz") {
            variations.push(FontVariation { tag: parley::setting::Tag::new(b"opsz"), value: size });
        }
        ParleyMeasurer {
            fcx,
            lcx,
            families: fams.join(", "),
            weight: d.weight.filter(|w| *w != 0.0 && !w.is_nan()).unwrap_or(500.0) as f32,
            italic: d.italic.unwrap_or(false),
            size,
            letter_spacing: d.tracking.filter(|t| !t.is_nan()).unwrap_or(0.0) as f32,
            variations,
            memo: HashMap::new(),
        }
    }
}

impl Measure for ParleyMeasurer<'_> {
    fn width(&mut self, text: &[u16]) -> f64 {
        if let Some(w) = self.memo.get(text) {
            return *w;
        }
        let s = String::from_utf16_lossy(text);
        let mut b = self.lcx.ranged_builder(self.fcx, &s, 1.0, false);
        b.push_default(StyleProperty::FontFamily(FontFamily::Source(Cow::Borrowed(&self.families))));
        b.push_default(StyleProperty::FontSize(self.size));
        b.push_default(StyleProperty::FontWeight(FontWeight::new(self.weight)));
        b.push_default(StyleProperty::FontStyle(if self.italic { FontStyle::Italic } else { FontStyle::Normal }));
        b.push_default(StyleProperty::LetterSpacing(self.letter_spacing));
        b.push_default(StyleProperty::FontVariations(parley::style::FontVariations::List(Cow::Borrowed(&self.variations))));
        let mut layout: Layout<()> = b.build(&s);
        layout.break_all_lines(None);
        let w = layout.full_width() as f64;
        self.memo.insert(text.to_vec(), w);
        w
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let root = PathBuf::from(args.iter().position(|a| a == "--root").map(|i| args[i + 1].clone()).unwrap_or_else(|| "out".into()));
    let cases = corpus::load_corpus(&corpus::sanitize_lone_surrogates(&std::fs::read_to_string(root.join("corpus.json")).expect("out/corpus.json")));
    let canvas: Value = serde_json::from_str(&corpus::sanitize_lone_surrogates(&std::fs::read_to_string(root.join("canvas2d.json")).expect("out/canvas2d.json"))).unwrap();
    let expected: HashMap<String, Value> =
        canvas["results"].as_array().expect("capture results").iter().map(|c| (c["id"].as_str().unwrap().to_string(), c.clone())).collect();

    let mut fcx = FontContext::new();
    let mut lcx = LayoutContext::new();
    let mut custom_names: HashMap<String, String> = HashMap::new();
    for (family, path) in VARIABLE_SOURCES {
        let Ok(bytes) = std::fs::read(path) else { continue };
        let name = format!("S4 FontFace {family}");
        let leaked: &'static str = Box::leak(name.clone().into_boxed_str());
        fcx.collection.register_fonts(Blob::new(Arc::new(bytes)), Some(FontInfoOverride { family_name: Some(leaked), ..Default::default() }));
        custom_names.insert(family.to_string(), name);
    }

    let started = Instant::now();
    #[derive(Default)]
    struct G {
        cases: usize,
        breaks: usize,
        within: usize,
        max: f64,
        errs: Vec<f64>,
    }
    let mut groups: BTreeMap<String, G> = BTreeMap::new();
    let mut failures = Vec::new();
    for case in &cases {
        let Some(exp) = expected.get(&case.id) else { continue };
        let custom = (!case.d.axes.is_empty()).then(|| case.d.font.as_deref().map(str::trim).and_then(|f| custom_names.get(f))).flatten().cloned();
        let mut m = ParleyMeasurer::new(&mut fcx, &mut lcx, &case.d, custom.as_deref());
        let out = run(&mut m, case);
        let diff = corpus::compare(case, &out, exp);
        let g = groups.entry(case.group.clone()).or_default();
        g.cases += 1;
        if diff.breaks_identical {
            g.breaks += 1;
            g.errs.push(diff.max_advance_error);
            g.max = g.max.max(diff.max_advance_error);
            if diff.max_advance_error <= 0.25 {
                g.within += 1;
            }
        }
        if !(diff.breaks_identical && diff.max_advance_error <= 0.25) {
            failures.push(json!({ "id": diff.id, "group": diff.group, "breaks": diff.breaks_identical, "max": diff.max_advance_error, "detail": diff.detail.iter().take(1).collect::<Vec<_>>() }));
        }
    }
    let elapsed = started.elapsed().as_secs_f64();
    let (mut total, mut breaks, mut within) = (0, 0, 0);
    let mut summary = serde_json::Map::new();
    for (name, g) in &mut groups {
        total += g.cases;
        breaks += g.breaks;
        within += g.within;
        g.errs.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let p50 = g.errs.get(g.errs.len() / 2).copied().unwrap_or(0.0);
        println!("  {name:<12} {:>5} cases  breaks {:>5}  within {:>5}  p50 {p50:.4}  max {:.4}", g.cases, g.breaks, g.within, g.max);
        summary.insert(name.clone(), json!({ "cases": g.cases, "identicalBreaks": g.breaks, "withinTolerance": g.within, "p50AdvanceError": p50, "maxAdvanceError": g.max }));
    }
    println!("parley: {total} cases, identical breaks {breaks}, within 0.25 px {within}; {elapsed:.2}s");
    let report = json!({ "measurer": "parley 0.12 (fontique system collection)", "cases": total, "identicalBreaks": breaks, "withinTolerance": within, "groups": summary, "failures": failures });
    std::fs::write(root.join("compare-parley.json"), serde_json::to_string_pretty(&report).unwrap()).unwrap();
}

fn run(m: &mut ParleyMeasurer, case: &Case) -> corpus::CaseOutput {
    let lh = case.d.line_height();
    let lines: Vec<SourceLine> = legacy::text_source_lines(&case.d, m, lh);
    let caret = legacy::caret_boundaries(&case.d, m);
    let layout = legacy::glyph_layout(&case.d, m);
    corpus::CaseOutput {
        id: case.id.clone(),
        lines: lines.iter().map(|l| (String::from_utf16_lossy(&l.text), l.start)).collect(),
        line_units: lines.into_iter().map(|l| l.text).collect(),
        caret,
        layout,
    }
}
