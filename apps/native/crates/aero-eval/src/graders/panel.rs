//! A panel the run built or fixed: it opens, shows the text it should, and a
//! scripted probe (select, seek, click, type) has the effect it should. The
//! effect is judged against the state just before the probe's steps (project,
//! markers, playhead and frames), so whatever the run left behind cannot
//! stand in for what the panel does. The runner executes the probe through
//! the host's panel tools; this grader only reads what was observed.
use super::project::{Assertion, check_all};
use super::{Context, Expect};
use crate::verdict::Verdict;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PanelSpec {
    /// Panel id, or `$new:panel`.
    pub panel: String,
    #[serde(default)]
    pub probe: Probe,
    /// Text the panel must show after the probe (case-insensitive).
    #[serde(default)]
    pub contains: Vec<String>,
    /// For each group, the panel shows at least one of its texts (one value
    /// written in any of its usual formats).
    #[serde(default)]
    pub contains_any: Vec<Vec<String>>,
    #[serde(default)]
    pub playhead: Option<Box<Expect>>,
    /// Project assertions after the probe's steps, against the project just
    /// before them.
    #[serde(default)]
    pub after: Vec<Assertion>,
    /// Frames rendered after the steps, against the same times rendered just
    /// before them (the baseline of these checks).
    #[serde(default)]
    pub frames: Vec<super::frames::StatsSpec>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Probe {
    /// Layers to select (by name) before the steps.
    pub select: Vec<String>,
    pub steps: Vec<Step>,
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
    if o.steps.len() != spec.probe.steps.len() {
        v.missing(format!(
            "{} of {} probe steps were run",
            o.steps.len(),
            spec.probe.steps.len()
        ));
    }
    for (step, result) in spec.probe.steps.iter().zip(&o.steps) {
        v.check(result == "ok", format!("{step:?}: {result}"));
    }
    let text = o.text.join("\n").to_lowercase();
    for want in &spec.contains {
        v.check(
            text.contains(&want.to_lowercase()),
            format!("panel shows \"{want}\""),
        );
    }
    for group in &spec.contains_any {
        v.check(
            group.iter().any(|w| text.contains(&w.to_lowercase())),
            format!("panel shows one of {group:?}"),
        );
    }
    if let Some(e) = &spec.playhead {
        match o.playhead {
            None => v.missing("the host did not report the playhead"),
            Some(t) => v.check(
                e.holds(Some(&Expect::num(t))),
                format!("playhead {t:.3}s, expected {}", e.describe()),
            ),
        }
    }
    if !spec.after.is_empty() {
        match (&o.project_after, &o.project_before) {
            (None, _) => v.missing("the project after the probe was not collected"),
            (_, None) => v.missing("the project before the probe was not collected"),
            (Some(p), Some(b)) => check_all(
                &spec.after,
                crate::pmv::project_of(p),
                Some(crate::pmv::project_of(b)),
                cx,
                &mut v,
            ),
        }
    }
    for f in &spec.frames {
        let w = super::frames::grade_stats_on(
            f,
            cx,
            &o.frames,
            &o.baseline_frames,
            Verdict::new("panel"),
        );
        v.evidence
            .extend(w.evidence.iter().map(|e| format!("frames: {e}")));
        match w.status {
            crate::verdict::Status::Fail => v.status = crate::verdict::Status::Fail,
            crate::verdict::Status::Unavailable if v.status == crate::verdict::Status::Pass => {
                v.status = crate::verdict::Status::Unavailable
            }
            _ => {}
        }
    }
    v
}

impl PanelSpec {
    /// Times the probe renders before and after its steps.
    pub fn frame_times(&self) -> Vec<f64> {
        self.frames.iter().flat_map(|f| f.times.clone()).collect()
    }
    pub fn seeks(&self) -> bool {
        self.probe.steps.iter().any(|s| matches!(s, Step::Seek(_)))
    }
}
