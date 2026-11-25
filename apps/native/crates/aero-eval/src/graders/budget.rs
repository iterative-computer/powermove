//! An effect's (or transition's) frame time under its budget.
use super::Context;
use crate::verdict::Verdict;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FrameTimeSpec {
    /// Effect or transition id, or `$new:effect` / `$new:transition`.
    pub target: String,
    /// Milliseconds the target may add to one frame at the composition size.
    pub budget_ms: f64,
    /// Which percentile of the samples must be under the budget (default p50).
    #[serde(default = "p50")]
    pub percentile: f64,
    /// Fewer samples than this is not evidence.
    #[serde(default = "five")]
    pub min_samples: usize,
}

fn p50() -> f64 {
    50.0
}
fn five() -> usize {
    5
}

pub fn grade(spec: &FrameTimeSpec, cx: &Context) -> Verdict {
    let mut v = Verdict::new("frame-time");
    let ids = cx.resolve(&spec.target);
    if ids.is_empty() {
        v.check(false, format!("no extension registered {}", spec.target));
        return v;
    }
    let Some(t) = cx
        .artifacts
        .frame_times
        .iter()
        .find(|t| ids.contains(&t.target))
    else {
        v.missing(format!("no frame-time samples for {}", ids.join(" or ")));
        return v;
    };
    if t.ms.len() < spec.min_samples || t.ms.iter().any(|x| !x.is_finite()) {
        v.missing(format!(
            "{} has {} usable samples, need {}",
            t.target,
            t.ms.len(),
            spec.min_samples
        ));
        return v;
    }
    let p = crate::image::percentile(&t.ms, spec.percentile);
    v.score = Some(p);
    v.check(
        p <= spec.budget_ms,
        format!(
            "{} p{} {p:.2} ms over {} samples (budget {} ms; {})",
            t.target,
            spec.percentile,
            t.ms.len(),
            spec.budget_ms,
            t.method
        ),
    );
    v
}
