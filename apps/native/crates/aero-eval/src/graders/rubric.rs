//! Subjective work: a rubric the fresh-context evaluator scores, anchored by
//! the owner's spot-check, which decides whenever it exists. Disagreements are
//! kept in the evidence so evaluator prompts can be tuned on them.
use super::Context;
use crate::verdict::{Status, Verdict};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RubricSpec {
    /// Rubric file under `data/`.
    pub rubric: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Rubric {
    pub id: String,
    pub title: String,
    /// Scores run from 0 to `scale` (whole numbers, anchored below).
    pub scale: u32,
    /// Weighted mean score needed to pass.
    pub pass_mean: f64,
    /// What the evaluator receives: captures, frames, the prompt. Never the
    /// generator's reasoning.
    pub evaluator_sees: Vec<String>,
    pub criteria: Vec<Criterion>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Criterion {
    pub id: String,
    pub question: String,
    #[serde(default = "one")]
    pub weight: f64,
    /// A hard criterion fails the task below `hard_min` whatever the mean.
    #[serde(default)]
    pub hard: bool,
    #[serde(default)]
    pub hard_min: Option<f64>,
    /// What each score means, keyed by score.
    pub anchors: BTreeMap<String, String>,
}

fn one() -> f64 {
    1.0
}

impl Rubric {
    pub fn load(path: &Path) -> Result<Self, String> {
        let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let r: Rubric =
            serde_json::from_slice(&bytes).map_err(|e| format!("{}: {e}", path.display()))?;
        r.validate()
            .map_err(|e| format!("{}: {e}", path.display()))?;
        Ok(r)
    }
    pub fn validate(&self) -> Result<(), String> {
        if self.criteria.is_empty() {
            return Err("a rubric needs criteria".into());
        }
        if !(0.0..=self.scale as f64).contains(&self.pass_mean) {
            return Err("pass_mean is outside the scale".into());
        }
        let mut seen = std::collections::BTreeSet::new();
        for c in &self.criteria {
            if !seen.insert(&c.id) {
                return Err(format!("criterion {} appears twice", c.id));
            }
            if c.weight <= 0.0 {
                return Err(format!("criterion {} needs a positive weight", c.id));
            }
            for s in 0..=self.scale {
                if !c.anchors.contains_key(&s.to_string()) {
                    return Err(format!("criterion {} has no anchor for score {s}", c.id));
                }
            }
        }
        Ok(())
    }
}

pub fn grade(spec: &RubricSpec, cx: &Context) -> Verdict {
    let mut v = Verdict::new("rubric");
    let rubric = match Rubric::load(&cx.data_root.join(&spec.rubric)) {
        Ok(r) => r,
        Err(e) => {
            v.check(false, format!("rubric unreadable: {e}"));
            return v;
        }
    };
    let evaluator = cx.artifacts.evaluation.as_ref().map(|e| score(&rubric, e));
    if let Some(Err(e)) = &evaluator {
        v.evidence.push(format!("evaluator: {e}"));
    }
    let evaluator = evaluator.and_then(Result::ok);
    if let Some((mean, pass, lines)) = &evaluator {
        v.evidence.extend(lines.iter().cloned());
        v.evidence.push(format!(
            "evaluator mean {mean:.2} (pass at {}): {}",
            rubric.pass_mean,
            if *pass { "pass" } else { "fail" }
        ));
        v.score = Some(*mean);
    }
    match (&cx.artifacts.spot_check, evaluator) {
        (Some(s), e) => {
            v.status = if s.pass { Status::Pass } else { Status::Fail };
            v.evidence.push(format!(
                "owner spot-check by {}: {}{}",
                s.by,
                if s.pass { "pass" } else { "fail" },
                if s.note.is_empty() {
                    String::new()
                } else {
                    format!(" ({})", s.note)
                }
            ));
            if let Some((_, pass, _)) = e {
                v.evidence.push(if pass == s.pass {
                    "evaluator agreed with the owner".into()
                } else {
                    "DISAGREEMENT: evaluator and owner differ (tune the evaluator on this run)"
                        .into()
                });
            }
        }
        (None, Some((_, pass, _))) => {
            v.status = if pass { Status::Pass } else { Status::Fail };
        }
        (None, None) => {
            v.status = Status::Unavailable;
            let lack = cx
                .artifacts
                .host_lacks
                .iter()
                .any(|l| l == crate::runner::capabilities::EVALUATOR);
            v.evidence.push(if lack {
                "unavailable: host lacks a fresh evaluator, and there is no spot-check".into()
            } else {
                "missing: no usable evaluator verdict and no spot-check".into()
            });
        }
    }
    v
}

/// Weighted mean, pass/fail and one line per criterion; an incomplete
/// evaluation (missing criterion, score off the scale, no evidence) is an error.
fn score(r: &Rubric, e: &crate::artifacts::Evaluation) -> Result<(f64, bool, Vec<String>), String> {
    if e.rubric != r.id {
        return Err(format!("scored rubric {}, expected {}", e.rubric, r.id));
    }
    let (mut sum, mut weights, mut pass) = (0.0, 0.0, true);
    let mut lines = Vec::new();
    for c in &r.criteria {
        let s = e
            .scores
            .get(&c.id)
            .ok_or_else(|| format!("criterion {} was not scored", c.id))?;
        if !(0.0..=r.scale as f64).contains(&s.score) {
            return Err(format!(
                "criterion {} scored {} off the scale",
                c.id, s.score
            ));
        }
        if s.evidence.trim().is_empty() {
            return Err(format!("criterion {} has no evidence", c.id));
        }
        let min = c.hard_min.unwrap_or(r.scale as f64 / 2.0);
        if c.hard && s.score < min {
            pass = false;
        }
        sum += s.score * c.weight;
        weights += c.weight;
        lines.push(format!(
            "{} {}: {}{} ({})",
            if c.hard && s.score < min { "✗" } else { "·" },
            c.id,
            s.score,
            if c.hard { " hard" } else { "" },
            s.evidence
        ));
    }
    let mean = sum / weights;
    Ok((mean, pass && mean >= r.pass_mean, lines))
}
