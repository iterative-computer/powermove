//! A grader's answer: structure in, verdict and evidence out.
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Status {
    Pass,
    Fail,
    /// The evidence the grader needs was not collected (no reference frame,
    /// no evaluator verdict, a host tool missing). Never counts as success.
    Unavailable,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Verdict {
    pub grader: String,
    pub status: Status,
    /// One line per check: what was asserted and what was observed.
    pub evidence: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub score: Option<f64>,
}

impl Verdict {
    pub fn new(grader: impl Into<String>) -> Self {
        Self {
            grader: grader.into(),
            status: Status::Pass,
            evidence: Vec::new(),
            score: None,
        }
    }
    pub fn unavailable(grader: impl Into<String>, why: impl Into<String>) -> Self {
        Self {
            grader: grader.into(),
            status: Status::Unavailable,
            evidence: vec![why.into()],
            score: None,
        }
    }
    /// Record one check. A failure turns a pass into a fail; unavailable stays.
    pub fn check(&mut self, ok: bool, line: impl Into<String>) {
        let line = line.into();
        self.evidence
            .push(format!("{} {line}", if ok { "ok:" } else { "FAIL:" }));
        if !ok && self.status == Status::Pass {
            self.status = Status::Fail;
        }
    }
    pub fn missing(&mut self, line: impl Into<String>) {
        self.evidence.push(format!("missing: {}", line.into()));
        // A definite failure outranks missing evidence.
        if self.status == Status::Pass {
            self.status = Status::Unavailable;
        }
    }
    pub fn passed(&self) -> bool {
        self.status == Status::Pass
    }
}
