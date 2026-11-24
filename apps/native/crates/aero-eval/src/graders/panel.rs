//! A panel the run built or fixed: it opens, shows the text it should, and a
//! scripted probe (select, seek, click, type) has the effect it should. The
//! runner executes the probe through the host's panel tools; this grader only
//! reads what was observed.
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
    #[serde(default)]
    pub playhead: Option<Box<Expect>>,
    /// Project assertions after the probe's steps.
    #[serde(default)]
    pub after: Vec<Assertion>,
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
        match &o.project_after {
            None => v.missing("the project after the probe was not collected"),
            Some(p) => check_all(
                &spec.after,
                crate::pmv::project_of(p),
                cx.artifacts
                    .project_after
                    .as_ref()
                    .map(crate::pmv::project_of),
                cx,
                &mut v,
            ),
        }
    }
    v
}
