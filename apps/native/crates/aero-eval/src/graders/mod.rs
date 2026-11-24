//! Graders: structure in, verdict and evidence out. Every grader reads only
//! [`Artifacts`](crate::artifacts::Artifacts), so a run can be graded again.
pub mod budget;
pub mod extensions;
pub mod frames;
pub mod panel;
pub mod project;
pub mod rubric;
pub mod theme;

use crate::artifacts::Artifacts;
use crate::verdict::Verdict;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::{Path, PathBuf};

/// One grader of a task, as written in the suite file.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "grader", rename_all = "kebab-case")]
pub enum GraderSpec {
    Project(project::ProjectSpec),
    ExtensionsLoad(extensions::LoadSpec),
    Frames(frames::FramesSpec),
    FrameStats(frames::StatsSpec),
    FrameTime(budget::FrameTimeSpec),
    Panel(panel::PanelSpec),
    Theme(theme::ThemeSpec),
    Rubric(rubric::RubricSpec),
}

impl GraderSpec {
    pub fn name(&self) -> &'static str {
        match self {
            GraderSpec::Project(_) => "project",
            GraderSpec::ExtensionsLoad(_) => "extensions-load",
            GraderSpec::Frames(_) => "frames",
            GraderSpec::FrameStats(_) => "frame-stats",
            GraderSpec::FrameTime(_) => "frame-time",
            GraderSpec::Panel(_) => "panel",
            GraderSpec::Theme(_) => "theme",
            GraderSpec::Rubric(_) => "rubric",
        }
    }
    /// Graders that judge rendered pixels decide "renders correctly".
    pub fn judges_render(&self) -> bool {
        matches!(self, GraderSpec::Frames(_) | GraderSpec::FrameStats(_))
    }
    /// Frame times this grader needs rendered (the runner renders the union).
    pub fn frame_times(&self) -> Vec<f64> {
        match self {
            GraderSpec::Frames(s) => s.times.clone(),
            GraderSpec::FrameStats(s) => s.times.clone(),
            _ => Vec::new(),
        }
    }
    pub fn grade(&self, cx: &Context) -> Verdict {
        match self {
            GraderSpec::Project(s) => project::grade(s, cx),
            GraderSpec::ExtensionsLoad(s) => extensions::grade(s, cx),
            GraderSpec::Frames(s) => frames::grade_reference(s, cx),
            GraderSpec::FrameStats(s) => frames::grade_stats(s, cx),
            GraderSpec::FrameTime(s) => budget::grade(s, cx),
            GraderSpec::Panel(s) => panel::grade(s, cx),
            GraderSpec::Theme(s) => theme::grade(s, cx),
            GraderSpec::Rubric(s) => rubric::grade(s, cx),
        }
    }
}

/// What a grader may read.
pub struct Context<'a> {
    pub artifacts: &'a Artifacts,
    /// Frame PNG paths in the artifacts are relative to this folder.
    pub run_dir: &'a Path,
    /// The suite's data folder (references, rubrics).
    pub data_root: &'a Path,
}

impl Context<'_> {
    pub fn path(&self, p: &Path) -> PathBuf {
        if p.is_absolute() {
            p.to_path_buf()
        } else {
            self.run_dir.join(p)
        }
    }
    /// Resolve an id that may be `$new:<kind>`: any contribution of that kind
    /// registered by an extension the run created or updated.
    pub fn resolve(&self, id: &str) -> Vec<String> {
        let Some(kind) = id.strip_prefix("$new:") else {
            return vec![id.to_owned()];
        };
        let ext = &self.artifacts.extensions;
        ext.registered
            .iter()
            .filter(|r| {
                r.kind == kind
                    && ext
                        .changed
                        .iter()
                        .any(|c| c.id == r.extension && c.action != "removed")
            })
            .map(|r| r.id.clone())
            .collect()
    }
}

/// An expectation on one value. Every field given must hold.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Expect {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub eq: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ne: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lt: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub le: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gt: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ge: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub approx: Option<f64>,
    /// Tolerance for `approx` (default 1e-6).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tol: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub between: Option<[f64; 2]>,
    /// Case-insensitive substring of a string, or an element of an array.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contains: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exists: Option<bool>,
    /// Array length at least this.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub len_ge: Option<usize>,
}

impl Expect {
    pub fn num(v: f64) -> Value {
        serde_json::Number::from_f64(v).map_or(Value::Null, Value::Number)
    }
    /// Check `actual` (None = absent). Returns whether it holds.
    pub fn holds(&self, actual: Option<&Value>) -> bool {
        if let Some(want) = self.exists
            && want != actual.is_some()
        {
            return false;
        }
        let Some(a) = actual else {
            return self.exists == Some(false);
        };
        let n = number(a);
        let num_ok = |f: &dyn Fn(f64) -> bool| n.is_some_and(f);
        if let Some(e) = &self.eq
            && !json_eq(a, e)
        {
            return false;
        }
        if let Some(e) = &self.ne
            && json_eq(a, e)
        {
            return false;
        }
        if let Some(x) = self.lt
            && !num_ok(&|n| n < x)
        {
            return false;
        }
        if let Some(x) = self.le
            && !num_ok(&|n| n <= x)
        {
            return false;
        }
        if let Some(x) = self.gt
            && !num_ok(&|n| n > x)
        {
            return false;
        }
        if let Some(x) = self.ge
            && !num_ok(&|n| n >= x)
        {
            return false;
        }
        if let Some(x) = self.approx {
            let tol = self.tol.unwrap_or(1e-6);
            if !num_ok(&|n| (n - x).abs() <= tol) {
                return false;
            }
        }
        if let Some([lo, hi]) = self.between
            && !num_ok(&|n| n >= lo && n <= hi)
        {
            return false;
        }
        if let Some(c) = &self.contains {
            let ok = match (a, c) {
                (Value::String(s), Value::String(c)) => {
                    s.to_lowercase().contains(&c.to_lowercase())
                }
                (Value::Array(items), c) => items.iter().any(|i| json_eq(i, c)),
                _ => false,
            };
            if !ok {
                return false;
            }
        }
        if let Some(n) = self.len_ge
            && a.as_array().is_none_or(|v| v.len() < n)
        {
            return false;
        }
        true
    }
    /// The expectation in words, for evidence lines.
    pub fn describe(&self) -> String {
        serde_json::to_string(self).unwrap_or_default()
    }
}

/// A number, also out of a `{v, kf, expr}` channel.
pub fn number(v: &Value) -> Option<f64> {
    v.as_f64().or_else(|| v.get("v").and_then(Value::as_f64))
}

/// Numbers compare by value (1 == 1.0); channels compare by their static value.
fn json_eq(a: &Value, b: &Value) -> bool {
    match (number(a), b.as_f64()) {
        (Some(x), Some(y)) => (x - y).abs() <= 1e-9,
        _ => a == b || a.get("v").is_some_and(|v| v == b),
    }
}

/// Short display of an observed value.
pub fn show(v: Option<&Value>) -> String {
    match v {
        None => "absent".into(),
        Some(v) => {
            let s = v.to_string();
            if s.len() > 120 {
                format!("{}…", &s[..s.floor_char_boundary(117)])
            } else {
                s
            }
        }
    }
}
