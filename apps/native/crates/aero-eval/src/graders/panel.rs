//! A panel the run built or fixed: it opens, and every control the prompt
//! names is exercised by its own probe (select, seek, click, type) and has
//! the effect it should. Each probe's effect is judged against the state just
//! before its steps (project, markers, playhead and frames), so whatever the
//! run or an earlier probe left behind cannot stand in for what this control
//! does. Rows pair what belongs together (a marker and its time, a layer and
//! its note): a row must show its own pair, not a neighbour's. The runner
//! executes the probes through the host's panel tools; this grader only reads
//! what was observed.
use super::project::{Assertion, check_all};
use super::{Context, Expect};
use crate::artifacts::ProbeObservation;
use crate::verdict::{Status, Verdict};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PanelSpec {
    /// Panel id, or `$new:panel`.
    pub panel: String,
    /// One probe per control the prompt names, run in order on the same
    /// panel (each starts from the state the previous one left).
    pub probes: Vec<Probe>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Probe {
    /// What this probe exercises, for evidence lines.
    pub name: String,
    /// Layers to select (by name) before the steps.
    pub select: Vec<String>,
    pub steps: Vec<Step>,
    /// Text the panel must show after the steps (case-insensitive).
    pub contains: Vec<String>,
    /// Rows the panel must show after the steps, each pairing its name with
    /// its own time or text.
    pub rows: Vec<Row>,
    pub playhead: Option<Box<Expect>>,
    /// Project assertions after the steps, against the project just before
    /// them.
    pub after: Vec<Assertion>,
    /// Frames rendered after the steps, against the same times rendered just
    /// before them (the baseline of these checks).
    pub frames: Vec<super::frames::StatsSpec>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Step {
    /// Move the playhead to this time (seconds).
    Seek(f64),
    /// Press the control whose label is this text.
    Click(String),
    /// Type into the field with this label.
    Type { field: String, text: String },
}

/// One row of a list panel: an element of the panel that names `name` and
/// none of the probe's other row names, and shows `time` (in any usual
/// display format) and/or `text`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Row {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub time: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
}

pub fn grade(spec: &PanelSpec, cx: &Context) -> Verdict {
    let mut v = Verdict::new("panel");
    let ids = cx.resolve(&spec.panel);
    if ids.is_empty() {
        v.check(false, format!("no extension registered {}", spec.panel));
        return v;
    }
    let Some(o) = cx.artifacts.panels.iter().find(|o| ids.contains(&o.panel)) else {
        v.missing(format!("panel {} was not observed", ids.join(" or ")));
        return v;
    };
    v.check(o.opened, format!("{} opens", o.panel));
    if !o.opened {
        return v;
    }
    if o.probes.len() != spec.probes.len() {
        v.missing(format!(
            "{} of {} probes were run",
            o.probes.len(),
            spec.probes.len()
        ));
    }
    for (i, (p, ob)) in spec.probes.iter().zip(&o.probes).enumerate() {
        let label = if p.name.is_empty() {
            format!("probe {}", i + 1)
        } else {
            p.name.clone()
        };
        // Each probe is judged on its own; a failing control fails the
        // panel even when another probe lacked evidence.
        let mut w = Verdict::new("panel");
        grade_probe(p, ob, &label, cx, &mut w);
        merge(&mut v, w, "");
    }
    v
}

fn grade_probe(p: &Probe, o: &ProbeObservation, label: &str, cx: &Context, v: &mut Verdict) {
    if o.steps.len() != p.steps.len() {
        v.missing(format!(
            "{label}: {} of {} steps were run",
            o.steps.len(),
            p.steps.len()
        ));
    }
    for (step, result) in p.steps.iter().zip(&o.steps) {
        v.check(result == "ok", format!("{label}: {step:?}: {result}"));
    }
    let text = o.text.join("\n").to_lowercase();
    for want in &p.contains {
        v.check(
            text.contains(&want.to_lowercase()),
            format!("{label}: panel shows \"{want}\""),
        );
    }
    for r in &p.rows {
        let (ok, line) = row_shown(r, &p.rows, &o.rows);
        v.check(ok, format!("{label}: {line}"));
    }
    if let Some(e) = &p.playhead {
        match o.playhead {
            None => v.missing(format!("{label}: the host did not report the playhead")),
            Some(t) => v.check(
                e.holds(Some(&Expect::num(t))),
                format!("{label}: playhead {t:.3}s, expected {}", e.describe()),
            ),
        }
    }
    if !p.after.is_empty() {
        match (&o.project_after, &o.project_before) {
            (None, _) => v.missing(format!(
                "{label}: the project after the steps was not collected"
            )),
            (_, None) => v.missing(format!(
                "{label}: the project before the steps was not collected"
            )),
            (Some(a), Some(b)) => {
                let mut w = Verdict::new("panel");
                check_all(
                    &p.after,
                    crate::pmv::project_of(a),
                    Some(crate::pmv::project_of(b)),
                    cx,
                    &mut w,
                );
                merge(v, w, label);
            }
        }
    }
    for f in &p.frames {
        let w = super::frames::grade_stats_on(
            f,
            cx,
            &o.frames,
            &o.baseline_frames,
            Verdict::new("panel"),
        );
        merge(v, w, &format!("{label}: frames"));
    }
}

fn merge(v: &mut Verdict, w: Verdict, label: &str) {
    v.evidence.extend(w.evidence.iter().map(|e| {
        if label.is_empty() {
            e.clone()
        } else {
            format!("{label}: {e}")
        }
    }));
    match w.status {
        Status::Fail => v.status = Status::Fail,
        Status::Unavailable if v.status == Status::Pass => v.status = Status::Unavailable,
        _ => {}
    }
}

/// Whether one row of the panel pairs `r.name` with its own time and text.
fn row_shown(r: &Row, all: &[Row], rows: &[String]) -> (bool, String) {
    let lower = |s: &str| s.to_lowercase();
    let others: Vec<&Row> = all.iter().filter(|o| o.name != r.name).collect();
    let mut nearest = None;
    for row in rows {
        let l = lower(row);
        if !l.contains(&lower(&r.name)) || others.iter().any(|o| l.contains(&lower(&o.name))) {
            continue;
        }
        nearest.get_or_insert(row.as_str());
        let time_ok = r.time.is_none_or(|t| {
            let tokens = time_tokens(row);
            let own = tokens.iter().any(|k| k.matches(t));
            // A token that reads as another row's time and not this one's.
            let foreign = tokens.iter().any(|k| {
                !k.matches(t)
                    && others
                        .iter()
                        .any(|o| o.time.is_some_and(|ot| k.matches(ot)))
            });
            own && !foreign
        });
        let text_ok = r.text.as_deref().is_none_or(|x| l.contains(&lower(x)));
        if time_ok && text_ok {
            return (true, format!("row \"{}\" shows {}", row.trim(), want(r)));
        }
    }
    (
        false,
        match nearest {
            Some(row) => format!(
                "row \"{}\" does not show {} (expected {})",
                row.trim(),
                want(r),
                want(r)
            ),
            None => format!(
                "no row shows {} apart from the other rows ({} rows seen)",
                r.name,
                rows.len()
            ),
        },
    )
}

fn want(r: &Row) -> String {
    let mut s = vec![format!("\"{}\"", r.name)];
    if let Some(t) = r.time {
        s.push(format!("at {t}s"));
    }
    if let Some(x) = &r.text {
        s.push(format!("with \"{x}\""));
    }
    s.join(" ")
}

/// A time as a panel may display it, read as the range of times that
/// display stands for.
#[derive(Clone, Debug, PartialEq)]
pub struct TimeToken {
    /// Half-open ranges [lo, hi) of seconds the token can mean.
    pub ranges: Vec<(f64, f64)>,
}

impl TimeToken {
    pub fn matches(&self, t: f64) -> bool {
        self.ranges
            .iter()
            .any(|(lo, hi)| t >= *lo - 1e-9 && t < *hi)
    }
}

/// Frame rates a timecode's last field may count.
const FPS: [f64; 4] = [24.0, 25.0, 30.0, 60.0];

/// Times in a row's text: `2.5`, `2.5s`, `5s`, `0:02`, `0:02.5`, `1:02:03`
/// and SMPTE-style `00:00:02:15` (frames at a common rate). A bare integer is
/// not a time (it could be anything). Whole-second displays may be rounded
/// down or to the nearest second; decimals by half their last digit.
pub fn time_tokens(text: &str) -> Vec<TimeToken> {
    let chars: Vec<char> = text.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        if !chars[i].is_ascii_digit() || (i > 0 && chars[i - 1].is_alphanumeric()) {
            i += 1;
            continue;
        }
        let start = i;
        while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == ':' || chars[i] == '.') {
            i += 1;
        }
        let mut tok: String = chars[start..i].iter().collect();
        while tok.ends_with([':', '.']) {
            tok.pop();
        }
        let secs_suffix = chars.get(i).is_some_and(|c| *c == 's')
            && !chars.get(i + 1).is_some_and(|c| c.is_alphanumeric());
        if let Some(t) = parse_time(&tok, secs_suffix) {
            out.push(t);
        }
    }
    out
}

fn parse_time(tok: &str, secs_suffix: bool) -> Option<TimeToken> {
    let parts: Vec<&str> = tok.split(':').collect();
    if parts.iter().any(|p| p.is_empty()) {
        return None;
    }
    let (last, head) = parts.split_last()?;
    if head.iter().any(|p| !p.bytes().all(|b| b.is_ascii_digit())) {
        return None;
    }
    let (whole, frac) = match last.split_once('.') {
        Some((w, f)) if !f.contains('.') && !w.is_empty() && !f.is_empty() => (w, Some(f)),
        Some(_) => return None,
        None => (*last, None),
    };
    if !whole.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let last_v: f64 = last.parse().ok()?;
    let unit = frac.map_or(1.0, |f| 10f64.powi(-(f.len() as i32)));
    // A display of `v` with this resolution: truncated or rounded.
    let shown = |v: f64| (v - unit / 2.0, v + unit);
    let nums: Vec<f64> = head.iter().map(|p| p.parse().unwrap_or(0.0)).collect();
    let mut ranges = Vec::new();
    match nums.len() {
        0 if frac.is_some() || secs_suffix => ranges.push(shown(last_v)),
        0 => return None,
        1 | 2 => {
            // m:ss(.f) or h:mm:ss(.f)
            if last_v < 60.0 && nums.iter().skip(1).all(|n| *n < 60.0) {
                let v = nums.iter().fold(0.0, |acc, n| acc * 60.0 + n) * 60.0 + last_v;
                ranges.push(shown(v));
            }
            // s:ff or m:ss:ff (frames)
            if frac.is_none() {
                ranges.extend(frames_range(&nums, last_v));
            }
        }
        3 if frac.is_none() => ranges.extend(frames_range(&nums, last_v)),
        _ => return None,
    }
    (!ranges.is_empty()).then_some(TimeToken { ranges })
}

/// `…:ss:ff` read at each common frame rate: [t − ½ frame, t + 1 frame).
fn frames_range(head: &[f64], frames: f64) -> Vec<(f64, f64)> {
    let secs = head.iter().fold(0.0, |acc, n| acc * 60.0 + n);
    FPS.iter()
        .filter(|fps| frames < **fps)
        .map(|fps| {
            let t = secs + frames / fps;
            (t - 0.5 / fps, t + 1.0 / fps)
        })
        .collect()
}

impl PanelSpec {
    /// Every time any probe renders before and after its steps.
    pub fn frame_times(&self) -> Vec<f64> {
        self.probes
            .iter()
            .flat_map(|p| p.frames.iter().flat_map(|f| f.times.clone()))
            .collect()
    }
    pub fn seeks(&self) -> bool {
        self.probes
            .iter()
            .any(|p| p.steps.iter().any(|s| matches!(s, Step::Seek(_))))
    }
    pub fn renders(&self) -> bool {
        self.probes.iter().any(|p| !p.frames.is_empty())
    }
}

impl Probe {
    pub fn frame_times(&self) -> Vec<f64> {
        self.frames.iter().flat_map(|f| f.times.clone()).collect()
    }
}
