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
    /// `live` (a real model), `replay` (a recorded stream) or `none` (no
    /// model). Summaries never pool modes.
    pub mode: String,
    /// The batch (one `aero-eval run` invocation) the run belongs to.
    pub cohort: String,
    /// Aero subsystems switched off for this run, sorted; empty for the full
    /// harness. Summaries never pool ablations.
    pub ablation: Vec<String>,
    /// Unique per run; summaries count distinct ids.
    pub run_id: String,
    /// Repeat number (0-based) and position in the interleaved batch.
    pub rep: u32,
    pub order: u32,
    pub started_unix_ms: u64,
    /// The harness's wall time; None when it never ran.
    pub wall_ms: Option<u64>,
    /// Every grader passed, the harness completed its turn, a replay was
    /// consumed exactly, within the time and step limits.
    pub success: bool,
    /// Every frame grader passed; None when the task has none.
    pub renders_correct: Option<bool>,
    /// Some grader lacked evidence (never counted as success).
    pub unavailable: bool,
    /// Token usage the harness reported; None when it reported none (never
    /// read as zero).
    pub tokens: Option<Usage>,
    /// Steps: tool calls the model issued (the one step unit, enforced while
    /// the run goes).
    pub tool_calls: u32,
    pub retries: u32,
    /// Completion-gate continuations; None for harnesses without a gate.
    pub gate_continuations: Option<u32>,
    pub timed_out: bool,
    pub over_step_limit: bool,
    pub exit_code: Option<i32>,
    /// The harness ended its turn by itself (CLI exit 0, Aero `completed`).
    pub harness_completed: Option<bool>,
    /// Why the harness stopped, in words.
    pub stop_reason: Option<String>,
    /// Replays only: every recorded exchange was requested in order and
    /// matched.
    pub replay_passed: Option<bool>,
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
    /// Fill the outcome fields from the verdicts, the harness and the limits.
    pub fn conclude(&mut self, judges_render: &[bool]) {
        self.unavailable = self
            .verdicts
            .iter()
            .any(|v| v.status == Status::Unavailable);
        let all_pass = !self.verdicts.is_empty() && self.verdicts.iter().all(Verdict::passed);
        let replay_ok = match (self.mode.as_str(), self.replay_passed) {
            ("replay", Some(true)) => true,
            ("replay", _) => false,
            (_, p) => p != Some(false),
        };
        self.success = all_pass
            && self.harness_completed == Some(true)
            && replay_ok
            && !self.timed_out
            && !self.over_step_limit
            && self.error.is_none();
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
