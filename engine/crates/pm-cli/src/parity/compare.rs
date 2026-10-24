//! Comparison of engine answers with the corpus's dumped values (plan
//! section 25, "Parity"; amendment 20).
//!
//! * Numbers: `|a − b| ≤ 1e-9 + 1e-9·|b|`; bounds within 1e-6 (absolute).
//! * Strings, booleans, `null`: exact. Objects: the same members (order
//!   carries no meaning); arrays: the same length, element by element.
//! * Non-finite numbers are dumped as `"NaN"`, `"Infinity"`, `"-Infinity"`
//!   and compare equal only to themselves; `-0` is dumped as 0.
//! * Allowed differences are counted apart, never as matches or mismatches:
//!   a rotation that differs by exactly 360° at `atan2`'s branch cut
//!   (amendment 20), a variable-speed source time in a nested composition
//!   (amendment 16).
//! * A named tolerance class passes a difference outside the plan's
//!   tolerance but within the class's own, and counts it apart: variable-font
//!   ink (CoreText's variation arithmetic against skrifa's, at most 1.19e-4
//!   px on the corpus; Phase 1 T2, decided by the orchestrator under
//!   amendment 22: invisible). Bounds and raster boxes of variable-font text
//!   take it.
//! * A "tolerance question" is a difference outside the plan's tolerance but
//!   within a named looser one: reported, never passed.

use std::collections::BTreeMap;

use pm_eval::JsValue;
use pm_model::JsString;
use pm_model::json::{Map, Value, to_canonical_string};

/// Absolute and relative tolerance.
pub type Tol = (f64, f64);

/// Section 25: evaluation.
pub const NUM: Tol = (1e-9, 1e-9);
/// Section 25: bounds within 1e-6.
pub const BOUNDS: Tol = (1e-6, 0.0);
/// Section 25, text class: advances within 0.25 px (level 1).
pub const TEXT_ADVANCE: f64 = 0.25;

pub fn member<'v>(v: &'v Value, key: &str) -> Option<&'v Value> {
    match v {
        Value::Object(m) => m.get_key(&JsString::from(key)),
        _ => None,
    }
}

/// A dumped number: numbers, or `"NaN"` / `"Infinity"` / `"-Infinity"`.
pub fn dumped_number(v: &Value) -> Option<f64> {
    match v {
        Value::Number(n) => Some(*n),
        Value::String(s) => match s.as_str()? {
            "NaN" => Some(f64::NAN),
            "Infinity" => Some(f64::INFINITY),
            "-Infinity" => Some(f64::NEG_INFINITY),
            _ => None,
        },
        _ => None,
    }
}

pub fn js_to_value(v: &JsValue) -> Value {
    match v {
        JsValue::Undefined | JsValue::Null => Value::Null,
        JsValue::Bool(b) => Value::Bool(*b),
        JsValue::Number(n) => Value::Number(*n),
        JsValue::String(s) => Value::String(s.clone()),
        JsValue::Object(v) => (**v).clone(),
    }
}

pub fn numbers(xs: &[f64]) -> Value {
    Value::Array(xs.iter().map(|x| Value::Number(*x)).collect())
}

/// What the capture's `plain()` writes for a value: non-finite numbers as
/// strings, `-0` as 0.
pub fn plain(v: &Value) -> Value {
    match v {
        Value::Number(n) if !n.is_finite() => Value::String(JsString::from(if n.is_nan() {
            "NaN"
        } else if *n > 0.0 {
            "Infinity"
        } else {
            "-Infinity"
        })),
        Value::Number(n) if *n == 0.0 => Value::Number(0.0),
        Value::Array(items) => Value::Array(items.iter().map(plain).collect()),
        Value::Object(map) => {
            Value::Object(map.iter().map(|(k, v)| (k.clone(), plain(v))).collect())
        }
        other => other.clone(),
    }
}

/// A pm-wasm reply value with its special-number markers
/// (`{"\u0000":"NaN"}`, `"undefined"`, ...) turned back into values.
pub fn revive(v: Value) -> Value {
    match v {
        Value::Object(map) => {
            if map.len() == 1
                && let Some(Value::String(s)) = map.get("\u{0}")
            {
                return match s.as_str() {
                    Some("NaN") => Value::Number(f64::NAN),
                    Some("Infinity") => Value::Number(f64::INFINITY),
                    Some("-Infinity") => Value::Number(f64::NEG_INFINITY),
                    Some("-0") => Value::Number(-0.0),
                    // `undefined`: JSON's closest form, as the capture's dump writes it.
                    _ => Value::Null,
                };
            }
            Value::Object(map.into_iter().map(|(k, v)| (k, revive(v))).collect())
        }
        Value::Array(items) => Value::Array(items.into_iter().map(revive).collect()),
        other => other,
    }
}

fn close(ours: f64, want: f64, tol: Tol) -> bool {
    if ours.is_nan() || want.is_nan() {
        return ours.is_nan() && want.is_nan();
    }
    if ours.is_infinite() || want.is_infinite() {
        return ours == want;
    }
    (ours - want).abs() <= tol.0 + tol.1 * want.abs()
}

/// Whether two numbers are the same rotation on either side of `atan2`'s
/// branch cut: 180 against −180 (each within the evaluation tolerance).
pub fn branch_cut(ours: f64, want: f64) -> bool {
    ours.is_finite()
        && want.is_finite()
        && close(ours.abs(), 180.0, NUM)
        && close(want.abs(), 180.0, NUM)
        && ours.signum() != want.signum()
        && close((ours - want).abs(), 360.0, (1e-9, 1e-9))
}

/// Deep comparison. `err` collects the largest error `|a − b| / (1 + |b|)`;
/// `cut` is set when numbers differ only at `atan2`'s branch cut (they then
/// count as equal here, and the caller decides whether that is allowed).
pub fn same(ours: &Value, want: &Value, tol: Tol, err: &mut f64, cut: &mut bool) -> bool {
    match (ours, want) {
        (Value::Number(a), _) if dumped_number(want).is_some() => {
            let b = dumped_number(want).unwrap_or(f64::NAN);
            if close(*a, b, tol) {
                if a.is_finite() && b.is_finite() {
                    let e = (a - b).abs() / (1.0 + b.abs());
                    if e > *err {
                        *err = e;
                    }
                }
                true
            } else if branch_cut(*a, b) {
                *cut = true;
                true
            } else {
                if a.is_finite() && b.is_finite() {
                    let e = (a - b).abs() / (1.0 + b.abs());
                    if e > *err {
                        *err = e;
                    }
                }
                false
            }
        }
        (Value::String(a), Value::String(b)) => a == b,
        (Value::Bool(a), Value::Bool(b)) => a == b,
        (Value::Null, Value::Null) => true,
        (Value::Array(a), Value::Array(b)) => {
            a.len() == b.len() && a.iter().zip(b).all(|(x, y)| same(x, y, tol, err, cut))
        }
        (Value::Object(a), Value::Object(b)) => {
            a.len() == b.len()
                && a.iter()
                    .all(|(k, x)| b.get_key(k).is_some_and(|y| same(x, y, tol, err, cut)))
        }
        _ => false,
    }
}

/// The largest absolute numeric difference between two values of the same
/// shape (`None` when the shapes differ).
pub fn max_abs_difference(ours: &Value, want: &Value) -> Option<f64> {
    match (ours, want) {
        (Value::Number(a), _) => {
            let b = dumped_number(want)?;
            if a.is_nan() && b.is_nan() || a == &b {
                Some(0.0)
            } else {
                Some((a - b).abs())
            }
        }
        (Value::Array(a), Value::Array(b)) if a.len() == b.len() => a
            .iter()
            .zip(b)
            .try_fold(0.0f64, |m, (x, y)| Some(m.max(max_abs_difference(x, y)?))),
        (Value::Object(a), Value::Object(b)) if a.len() == b.len() => {
            a.iter().try_fold(0.0f64, |m, (k, x)| {
                Some(m.max(max_abs_difference(x, b.get_key(k)?)?))
            })
        }
        (a, b) => (a == b).then_some(0.0),
    }
}

/// Where two values first differ (a JSON path with both sides).
pub fn first_difference(ours: &Value, want: &Value, tol: Tol, path: String) -> Option<String> {
    let (mut err, mut cut) = (0.0, false);
    if same(ours, want, tol, &mut err, &mut cut) {
        return None;
    }
    match (ours, want) {
        (Value::Object(a), Value::Object(b)) => {
            for (k, x) in a.iter() {
                let p = format!("{path}.{}", k.to_string_lossy());
                match b.get_key(k) {
                    Some(y) => {
                        if let Some(d) = first_difference(x, y, tol, p) {
                            return Some(d);
                        }
                    }
                    None => return Some(format!("{p} (only ours)")),
                }
            }
            for (k, _) in b.iter() {
                if a.get_key(k).is_none() {
                    return Some(format!("{path}.{} (only the capture)", k.to_string_lossy()));
                }
            }
            None
        }
        (Value::Array(a), Value::Array(b)) if a.len() == b.len() => a
            .iter()
            .zip(b)
            .enumerate()
            .find_map(|(i, (x, y))| first_difference(x, y, tol, format!("{path}[{i}]"))),
        _ => Some(format!(
            "{path} (ours {} vs capture {})",
            short(&to_canonical_string(ours)),
            short(&to_canonical_string(want))
        )),
    }
}

fn short(s: &str) -> String {
    if s.len() <= 240 {
        s.to_owned()
    } else {
        let mut end = 240;
        while !s.is_char_boundary(end) {
            end -= 1;
        }
        format!("{}…", &s[..end])
    }
}

/// An engine's answer to one dumped value.
#[derive(Clone, Debug)]
pub enum Answer {
    Value(Value),
    /// The engine threw (the legacy code throws there too when the dump
    /// records an error for the value).
    Threw(String),
    /// The engine does not answer this in Phase 1 (three.js geometry, model
    /// group recipes, fonts or ink a configuration lacks).
    Unanswered(&'static str),
}

/// How a value compared.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    Match,
    /// Within a named tolerance class (passes; counted apart).
    Class(&'static str),
    Allowed(&'static str),
    Question(&'static str),
    Mismatch,
    NotCompared,
}

/// Counts per category.
#[derive(Default, Clone, Debug)]
pub struct Category {
    pub compared: usize,
    pub matched: usize,
    pub allowed: BTreeMap<&'static str, usize>,
    /// Values that passed in a named tolerance class, by class.
    pub classes: BTreeMap<&'static str, usize>,
    /// Largest absolute difference among them.
    pub class_max_abs: f64,
    pub questions: BTreeMap<&'static str, usize>,
    pub mismatched: usize,
    pub max_error: f64,
    /// Largest absolute difference among tolerance questions.
    pub question_max_abs: f64,
    pub samples: Vec<String>,
}

#[derive(Default, Debug)]
pub struct Stats {
    pub categories: BTreeMap<&'static str, Category>,
    /// Values not compared, by category and reason.
    pub skipped: BTreeMap<(&'static str, &'static str), usize>,
    /// Text layouts (layer × dump) laid out differently from the capture, by font.
    pub layout_fonts: BTreeMap<String, usize>,
    pub text: TextGate,
    pub verbose: bool,
    pub max_samples: usize,
}

/// The text sub-gate (section 26 Phase 1 gate; section 25 text class): every
/// text layout's line breaks identical and its advances within 0.25 px.
#[derive(Default, Clone, Debug)]
pub struct TextGate {
    pub layouts: usize,
    pub units: usize,
    pub line_breaks_differ: usize,
    pub advances_over: usize,
    pub max_advance_error: f64,
    pub samples: Vec<String>,
}

impl Stats {
    pub fn new(verbose: bool, max_samples: usize) -> Stats {
        Stats {
            verbose,
            max_samples,
            ..Stats::default()
        }
    }

    pub fn skip(&mut self, category: &'static str, reason: &'static str) {
        *self.skipped.entry((category, reason)).or_default() += 1;
    }

    pub fn record(
        &mut self,
        category: &'static str,
        outcome: Outcome,
        err: f64,
        what: impl FnOnce() -> String,
    ) {
        let (verbose, max) = (self.verbose, self.max_samples);
        let c = self.categories.entry(category).or_default();
        c.compared += 1;
        if err > c.max_error {
            c.max_error = err;
        }
        match outcome {
            Outcome::Match => c.matched += 1,
            Outcome::Class(class) => {
                c.matched += 1;
                *c.classes.entry(class).or_default() += 1;
            }
            Outcome::Allowed(class) => *c.allowed.entry(class).or_default() += 1,
            Outcome::Question(class) => {
                *c.questions.entry(class).or_default() += 1;
                if verbose || c.samples.len() < max {
                    c.samples
                        .push(format!("[tolerance question: {class}] {}", what()));
                }
            }
            Outcome::Mismatch => {
                c.mismatched += 1;
                if verbose || c.samples.len() < max {
                    c.samples.push(what());
                }
            }
            Outcome::NotCompared => c.compared -= 1,
        }
    }

    pub fn mismatches(&self) -> usize {
        self.categories.values().map(|c| c.mismatched).sum()
    }

    pub fn questions(&self) -> usize {
        self.categories
            .values()
            .map(|c| c.questions.values().sum::<usize>())
            .sum()
    }

    pub fn compared(&self) -> usize {
        self.categories.values().map(|c| c.compared).sum()
    }
}

/// How one dumped value is judged.
#[derive(Clone, Copy, Debug)]
pub struct Rule {
    pub tol: Tol,
    /// A 180/−180 rotation difference is an allowed difference (amendment 20).
    /// Not for bounds or text, which have their own tolerances.
    pub branch_cut: bool,
    /// A looser tolerance whose differences are reported as a tolerance
    /// question instead of a mismatch.
    pub question: Option<(Tol, &'static str)>,
    /// A named tolerance class: differences within it pass, counted apart.
    pub class: Option<(Tol, &'static str)>,
    /// A difference here is an allowed difference of this class.
    pub allowed: Option<&'static str>,
}

impl Rule {
    pub const fn num() -> Rule {
        Rule {
            tol: NUM,
            branch_cut: true,
            question: None,
            class: None,
            allowed: None,
        }
    }
    pub const fn bounds() -> Rule {
        Rule {
            tol: BOUNDS,
            branch_cut: false,
            question: None,
            class: None,
            allowed: None,
        }
    }
}

/// Judges an answer against the dumped value. `legacy_threw` is set when the
/// dump records that the legacy call threw (its value is then `null`).
pub fn judge(
    answer: &Answer,
    want: &Value,
    legacy_threw: bool,
    rule: Rule,
) -> (Outcome, f64, String) {
    match answer {
        Answer::Unanswered(_) => (Outcome::NotCompared, 0.0, String::new()),
        Answer::Threw(message) => {
            if legacy_threw || matches!(want, Value::Null) {
                (Outcome::Match, 0.0, String::new())
            } else {
                (
                    Outcome::Mismatch,
                    0.0,
                    format!(
                        "ours threw {message:?}, capture {}",
                        short(&to_canonical_string(want))
                    ),
                )
            }
        }
        Answer::Value(v) => {
            let v = plain(v);
            let (mut err, mut cut) = (0.0, false);
            let ok = !legacy_threw && same(&v, want, rule.tol, &mut err, &mut cut);
            if ok && !cut {
                return (Outcome::Match, err, String::new());
            }
            let describe = || {
                let at = first_difference(&v, want, rule.tol, String::new()).unwrap_or_default();
                format!(
                    "first difference at {}: ours {} capture {}{}",
                    if at.is_empty() { "the value" } else { &at },
                    short(&to_canonical_string(&v)),
                    short(&to_canonical_string(want)),
                    if legacy_threw {
                        " (the legacy call threw)"
                    } else {
                        ""
                    }
                )
            };
            if ok && cut {
                return if rule.branch_cut {
                    (Outcome::Allowed("atan2 branch cut"), err, describe())
                } else {
                    (Outcome::Mismatch, err, describe())
                };
            }
            if legacy_threw {
                return (Outcome::Mismatch, err, describe());
            }
            if let Some(class) = rule.allowed {
                return (Outcome::Allowed(class), err, describe());
            }
            if let Some((tol, class)) = rule.class {
                let (mut e2, mut c2) = (0.0, false);
                if same(&v, want, tol, &mut e2, &mut c2) && !c2 {
                    return (Outcome::Class(class), err, describe());
                }
            }
            if let Some((loose, class)) = rule.question {
                let (mut e2, mut c2) = (0.0, false);
                if same(&v, want, loose, &mut e2, &mut c2) {
                    return (Outcome::Question(class), err, describe());
                }
            }
            (Outcome::Mismatch, err, describe())
        }
    }
}

/// The text sub-gate over one layout: identical line breaks (the same lines,
/// each with the same text and source offset, and every character on the
/// same line) and every unit's position and advance within 0.25 px.
pub fn text_gate(gate: &mut TextGate, ours: &Value, want: &Value, label: impl FnOnce() -> String) {
    gate.layouts += 1;
    let units = |v: &Value, k: &str| -> Vec<Value> {
        member(v, k)
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default()
    };
    let field = |u: &Value, k: &str| member(u, k).cloned().unwrap_or(Value::Null);
    let (ol, wl) = (units(ours, "lines"), units(want, "lines"));
    let (oc, wc) = (units(ours, "characters"), units(want, "characters"));
    let breaks_same = ol.len() == wl.len()
        && ol.iter().zip(&wl).all(|(a, b)| {
            field(a, "text") == field(b, "text")
                && field(a, "sourceStart") == field(b, "sourceStart")
        })
        && oc.len() == wc.len()
        && oc.iter().zip(&wc).all(|(a, b)| {
            field(a, "line") == field(b, "line") && field(a, "text") == field(b, "text")
        });
    let mut worst = 0.0f64;
    let mut over = false;
    for kind in ["characters", "words", "lines"] {
        let (a, b) = (units(ours, kind), units(want, kind));
        gate.units += a.len().min(b.len());
        for (x, y) in a.iter().zip(&b) {
            for k in ["x", "y", "w"] {
                let (p, q) = (
                    member(x, k).and_then(dumped_number),
                    member(y, k).and_then(dumped_number),
                );
                if let (Some(p), Some(q)) = (p, q) {
                    let e = (p - q).abs();
                    if e > worst {
                        worst = e;
                    }
                    if e > TEXT_ADVANCE {
                        over = true;
                    }
                }
            }
        }
    }
    if worst > gate.max_advance_error {
        gate.max_advance_error = worst;
    }
    if !breaks_same {
        gate.line_breaks_differ += 1;
    }
    if over {
        gate.advances_over += 1;
    }
    if (!breaks_same || over) && gate.samples.len() < 12 {
        gate.samples.push(format!(
            "{}: {}max position or advance error {worst:.4} px",
            label(),
            if breaks_same {
                ""
            } else {
                "line breaks differ; "
            }
        ));
    }
}

/// The value's JSON with object members as a `Map` (helper for builders).
pub fn object(items: impl IntoIterator<Item = (&'static str, Value)>) -> Value {
    let mut m = Map::new();
    for (k, v) in items {
        m.insert(k, v);
    }
    Value::Object(m)
}

#[cfg(test)]
mod tests {
    use super::*;
    use pm_model::json::parse;

    fn v(s: &str) -> Value {
        parse(s).unwrap()
    }

    #[test]
    fn tolerance_is_absolute_plus_relative() {
        let (mut e, mut c) = (0.0, false);
        assert!(same(
            &Value::Number(1e6 + 1e-3 * 0.9),
            &Value::Number(1e6),
            NUM,
            &mut e,
            &mut c
        ));
        assert!(!same(
            &Value::Number(1e6 + 2e-3),
            &Value::Number(1e6),
            NUM,
            &mut e,
            &mut c
        ));
        assert!(same(
            &Value::Number(1e-9),
            &Value::Number(0.0),
            NUM,
            &mut e,
            &mut c
        ));
        assert!(!same(
            &Value::Number(3e-9),
            &Value::Number(0.0),
            NUM,
            &mut e,
            &mut c
        ));
    }

    #[test]
    fn non_finite_numbers_match_their_dumped_strings() {
        let (mut e, mut c) = (0.0, false);
        assert!(same(
            &Value::Number(f64::NAN),
            &v("\"NaN\""),
            NUM,
            &mut e,
            &mut c
        ));
        assert!(same(
            &Value::Number(f64::INFINITY),
            &v("\"Infinity\""),
            NUM,
            &mut e,
            &mut c
        ));
        assert!(!same(
            &Value::Number(f64::INFINITY),
            &v("\"-Infinity\""),
            NUM,
            &mut e,
            &mut c
        ));
        assert!(!same(
            &Value::Number(0.0),
            &v("\"NaN\""),
            NUM,
            &mut e,
            &mut c
        ));
        assert_eq!(plain(&Value::Number(-0.0)), Value::Number(0.0));
    }

    #[test]
    fn objects_ignore_order_and_need_the_same_members() {
        let (mut e, mut c) = (0.0, false);
        assert!(same(
            &v(r#"{"a":1,"b":"x"}"#),
            &v(r#"{"b":"x","a":1}"#),
            NUM,
            &mut e,
            &mut c
        ));
        assert!(!same(
            &v(r#"{"a":1}"#),
            &v(r#"{"a":1,"b":null}"#),
            NUM,
            &mut e,
            &mut c
        ));
        assert!(!same(&v("[1,2]"), &v("[1,2,3]"), NUM, &mut e, &mut c));
        assert!(!same(&v("true"), &v("1"), NUM, &mut e, &mut c));
    }

    #[test]
    fn branch_cut_is_allowed_only_where_the_rule_says() {
        let a = Answer::Value(v("[1, 180]"));
        let want = v("[1, -180]");
        assert_eq!(
            judge(&a, &want, false, Rule::num()).0,
            Outcome::Allowed("atan2 branch cut")
        );
        assert_eq!(judge(&a, &want, false, Rule::bounds()).0, Outcome::Mismatch);
        assert!(!branch_cut(180.0, 180.0));
        assert!(!branch_cut(179.0, -181.0));
        assert!(branch_cut(-180.00000000000003, 180.0));
    }

    #[test]
    fn throws_match_recorded_errors() {
        assert_eq!(
            judge(&Answer::Threw("x".into()), &Value::Null, true, Rule::num()).0,
            Outcome::Match
        );
        assert_eq!(
            judge(&Answer::Threw("x".into()), &v("1"), false, Rule::num()).0,
            Outcome::Mismatch
        );
        assert_eq!(
            judge(&Answer::Value(Value::Null), &Value::Null, true, Rule::num()).0,
            Outcome::Mismatch
        );
    }

    #[test]
    fn tolerance_questions_are_never_matches() {
        let rule = Rule {
            question: Some(((2e-4, 0.0), "variable-font ink")),
            ..Rule::bounds()
        };
        let a = Answer::Value(v("1.0001"));
        assert_eq!(
            judge(&a, &v("1"), false, rule).0,
            Outcome::Question("variable-font ink")
        );
        assert_eq!(
            judge(&Answer::Value(v("1.001")), &v("1"), false, rule).0,
            Outcome::Mismatch
        );
        assert_eq!(
            judge(&Answer::Value(v("1.0000001")), &v("1"), false, rule).0,
            Outcome::Match
        );
    }

    #[test]
    fn a_tolerance_class_passes_within_its_bound_and_is_counted_apart() {
        let rule = Rule {
            class: Some(((2e-4, 0.0), "variable-font ink")),
            ..Rule::bounds()
        };
        assert_eq!(
            judge(&Answer::Value(v("1.000119")), &v("1"), false, rule).0,
            Outcome::Class("variable-font ink")
        );
        assert_eq!(
            judge(&Answer::Value(v("1.001")), &v("1"), false, rule).0,
            Outcome::Mismatch
        );
        assert_eq!(
            judge(&Answer::Value(v("1.0000001")), &v("1"), false, rule).0,
            Outcome::Match
        );
        let mut stats = Stats::new(false, 4);
        stats.record(
            "bounds (variable font)",
            Outcome::Class("variable-font ink"),
            1e-4,
            String::new,
        );
        assert_eq!(
            (stats.mismatches(), stats.questions(), stats.compared()),
            (0, 0, 1)
        );
        assert_eq!(
            stats.categories["bounds (variable font)"].classes["variable-font ink"],
            1
        );
    }

    #[test]
    fn revive_turns_markers_back_into_numbers() {
        let r = revive(v(
            r#"{"a":{"\u0000":"NaN"},"b":[{"\u0000":"-Infinity"},{"\u0000":"undefined"}]}"#,
        ));
        let (mut e, mut c) = (0.0, false);
        assert!(same(
            &plain(&r),
            &v(r#"{"a":"NaN","b":["-Infinity",null]}"#),
            NUM,
            &mut e,
            &mut c
        ));
    }

    #[test]
    fn text_gate_counts_breaks_and_advances() {
        let mut g = TextGate::default();
        let want = v(
            r#"{"characters":[{"text":"a","x":0,"y":0,"w":10,"line":0}],"words":[],"lines":[{"text":"a","x":0,"y":0,"w":10,"line":0,"sourceStart":0}]}"#,
        );
        let near = v(
            r#"{"characters":[{"text":"a","x":0,"y":0,"w":10.2,"line":0}],"words":[],"lines":[{"text":"a","x":0,"y":0,"w":10.2,"line":0,"sourceStart":0}]}"#,
        );
        let far = v(
            r#"{"characters":[{"text":"a","x":0,"y":0,"w":10.3,"line":1}],"words":[],"lines":[{"text":"a","x":0,"y":0,"w":10.3,"line":0,"sourceStart":0}]}"#,
        );
        text_gate(&mut g, &near, &want, String::new);
        assert_eq!((g.line_breaks_differ, g.advances_over), (0, 0));
        text_gate(&mut g, &far, &want, String::new);
        assert_eq!(
            (g.layouts, g.line_breaks_differ, g.advances_over),
            (2, 1, 1)
        );
    }
}
