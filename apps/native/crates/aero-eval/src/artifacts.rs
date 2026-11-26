//! Evidence the runner collects for one run, the graders' only input. It is
//! saved as `artifacts.json` (plus frame PNGs) in the run folder, so a run can
//! be graded again later, e.g. after the owner's spot-check is added.
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Artifacts {
    /// Root project document before the harness ran (the fixture as opened).
    pub project_before: Option<Value>,
    /// Root project document after the run, read back from the saved file.
    pub project_after: Option<Value>,
    pub extensions: ExtensionReport,
    /// Frames rendered after the run, at the times the task's graders name.
    pub frames: Vec<Frame>,
    /// The same times rendered from the fixture before the run.
    pub baseline_frames: Vec<Frame>,
    pub frame_times: Vec<FrameTiming>,
    pub panels: Vec<PanelObservation>,
    pub theme: Option<ThemeObservation>,
    /// The fresh-context evaluator's rubric scores, when the task has a rubric.
    pub evaluation: Option<Evaluation>,
    /// The owner's spot-check, added by hand after the run.
    pub spot_check: Option<SpotCheck>,
    /// Collection problems (a tool the host does not offer yet, a failed
    /// render). Graders that needed the evidence report Unavailable.
    pub notes: Vec<String>,
    /// Evidence the host cannot supply at all (the runner's capability
    /// check); graders that need it answer "unavailable: host lacks …".
    pub host_lacks: Vec<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ExtensionReport {
    /// Extensions whose folder in the profile was added or changed by the run
    /// (tree hashes before and after), plus any removed ones.
    pub changed: Vec<ChangedExtension>,
    /// Windows the host reports; "every window" means each of these.
    pub windows: Vec<String>,
    pub health: Vec<ExtensionHealth>,
    /// Registered contributions after the run, with their owning extension.
    pub registered: Vec<Registration>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ChangedExtension {
    pub id: String,
    /// `created`, `updated` or `removed`.
    pub action: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ExtensionHealth {
    pub id: String,
    pub window: String,
    /// `ok` when loaded; anything else is a load failure.
    pub state: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Registration {
    /// `effect`, `transition`, `panel`, `theme`, `command`.
    pub kind: String,
    pub id: String,
    pub extension: String,
    #[serde(default)]
    pub params: usize,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Frame {
    pub time: f64,
    /// PNG file, relative to the run folder (or absolute).
    pub png: PathBuf,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FrameTiming {
    /// The effect or transition id the samples measure.
    pub target: String,
    /// Milliseconds the target adds per frame, one sample per measurement.
    pub ms: Vec<f64>,
    /// How the samples were taken (shown in evidence).
    pub method: String,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PanelObservation {
    pub panel: String,
    pub opened: bool,
    /// One per probe of the grader, in order.
    pub probes: Vec<ProbeObservation>,
    pub capture: Option<PathBuf>,
}

/// What one probe observed.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ProbeObservation {
    /// Visible text of the panel after the steps.
    pub text: Vec<String>,
    /// The text of each element of the panel state after the steps (every
    /// object or list in the host's tree, its displayed strings joined), so
    /// a grader can tell which values share a row.
    pub rows: Vec<String>,
    /// Result of each step, in order (`ok` or the error).
    pub steps: Vec<String>,
    /// Playhead time after the steps, when the host reports it.
    pub playhead: Option<f64>,
    /// Project document just before the steps.
    pub project_before: Option<Value>,
    /// Project document after the steps.
    pub project_after: Option<Value>,
    /// Frames rendered just before and after the steps.
    pub baseline_frames: Vec<Frame>,
    pub frames: Vec<Frame>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ThemeObservation {
    pub active: String,
    pub extension: Option<String>,
    /// Resolved CSS custom properties of the active theme.
    pub tokens: std::collections::BTreeMap<String, String>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Evaluation {
    /// Harness and model of the evaluator session (never the generator's).
    pub evaluator: String,
    pub rubric: String,
    pub scores: std::collections::BTreeMap<String, CriterionScore>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CriterionScore {
    pub score: f64,
    /// What the evaluator saw that justifies the score.
    pub evidence: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SpotCheck {
    pub by: String,
    pub pass: bool,
    #[serde(default)]
    pub note: String,
    /// Would a person need to fix anything before using the result?
    #[serde(default)]
    pub needs_fix: Option<bool>,
}

impl Artifacts {
    /// Read a run's artifacts (and a spot-check beside them) from the run
    /// folder, following no link.
    pub fn load(run_dir: &Path) -> Result<Self, String> {
        let dir = crate::runner::paths::Anchor::open(run_dir)?;
        let at = |n: &str| run_dir.join(n).display().to_string();
        let bytes = dir.read(Path::new("artifacts.json"))?;
        let mut a: Artifacts =
            serde_json::from_slice(&bytes).map_err(|e| format!("{}: {e}", at("artifacts.json")))?;
        // A spot-check written later by hand lives beside the artifacts.
        if dir.entries()?.iter().any(|(n, _)| n == "spot-check.json") {
            let bytes = dir.read(Path::new("spot-check.json"))?;
            a.spot_check = Some(
                serde_json::from_slice(&bytes)
                    .map_err(|e| format!("{}: {e}", at("spot-check.json")))?,
            );
        }
        Ok(a)
    }
    /// Write `artifacts.json` into the run folder; it must not exist yet (a
    /// file or link a run component left there is never written through).
    pub fn save(&self, run_dir: &Path) -> Result<(), String> {
        let text = serde_json::to_vec_pretty(self).map_err(|e| e.to_string())?;
        crate::runner::paths::Anchor::open(run_dir)?.write_new(Path::new("artifacts.json"), &text)
    }
}
