//! One JSON per run (`run.json` in the run folder).
use crate::verdict::{Status, Verdict};
use serde::{Deserialize, Serialize};

pub const FORMAT: &str = "aero-eval-run";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RunRecord {
    pub format: String,
    pub suite_version: u32,
    pub task: String,
    pub kind: String,
    pub harness: String,
    pub model: String,
    pub effort: String,
    /// Repeat number (0-based) and position in the interleaved batch.
    pub rep: u32,
    pub order: u32,
    pub started_unix_ms: u64,
    pub wall_ms: u64,
    /// Every grader passed, within the time and step limits.
    pub success: bool,
    /// Every frame grader passed; None when the task has none.
    pub renders_correct: Option<bool>,
    /// Some grader lacked evidence (never counted as success).
    pub unavailable: bool,
    pub tokens: Usage,
    pub tool_calls: u32,
    pub retries: u32,
    /// Completion-gate continuations; None for harnesses without a gate.
    pub gate_continuations: Option<u32>,
    pub timed_out: bool,
    pub over_step_limit: bool,
    pub exit_code: Option<i32>,
    /// Would a person need to fix anything (from the owner's spot-check).
    pub needs_fix: Option<bool>,
    pub verdicts: Vec<Verdict>,
    /// The note under which a real-model run was approved; None for replays.
    pub approved: Option<String>,
    /// The recording a replayed run was fed from.
    pub replay: Option<String>,
    /// Why the run could not complete (setup or harness failure).
    pub error: Option<String>,
}

/// Tokens, normalised across providers: `input` counts every prompt token
/// (cached ones included), `cached` the subset served from cache.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Usage {
    pub input: u64,
    pub cached: u64,
    pub output: u64,
}

impl Usage {
    pub fn total(&self) -> u64 {
        self.input + self.output
    }
    pub fn add(&mut self, o: Usage) {
        self.input += o.input;
        self.cached += o.cached;
        self.output += o.output;
    }
}

impl RunRecord {
    /// Fill the outcome fields from the verdicts and limits.
    pub fn conclude(&mut self, judges_render: &[bool]) {
        self.unavailable = self
            .verdicts
            .iter()
            .any(|v| v.status == Status::Unavailable);
        let all_pass = !self.verdicts.is_empty() && self.verdicts.iter().all(Verdict::passed);
        self.success = all_pass && !self.timed_out && !self.over_step_limit && self.error.is_none();
        let render: Vec<&Verdict> = self
            .verdicts
            .iter()
            .zip(judges_render)
            .filter(|(_, r)| **r)
            .map(|(v, _)| v)
            .collect();
        self.renders_correct = (!render.is_empty()).then(|| render.iter().all(|v| v.passed()));
    }
}
