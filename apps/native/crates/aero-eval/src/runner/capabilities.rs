//! What evidence the host can supply. Checked once per run, before the
//! baseline; a grader whose evidence the host lacks answers
//! "unavailable: host lacks …" instead of guessing. Tool-backed evidence is
//! read from `tools/list`; the rest must be advertised by an eval-capable
//! host in its `initialize` answer under
//! `capabilities.experimental.powermoveEval` (the host contract in
//! docs/native/aero-eval.md). The stock headless host advertises none.
use serde_json::Value;

/// Rendered frames (`render_frames`).
pub const FRAMES: &str = "frame rendering";
/// Opening, reading and pressing a panel (`open_panel`, `get_panel_state`,
/// `interact_panel`).
pub const PROBES: &str = "panel probes";
/// Each changed extension's load result in every window of the tested set.
pub const HEALTH: &str = "extension health";
/// The extension that registered each effect, transition, panel and theme.
pub const OWNERS: &str = "registration owners";
pub const TRANSITIONS: &str = "transition registrations";
pub const PANELS: &str = "panel registrations";
/// The active theme and its resolved tokens.
pub const THEMES: &str = "the active theme";
/// Moving the playhead (`set_playhead`).
pub const SEEK: &str = "seek";
/// Paired enabled/disabled timings of one target at active frames
/// (`measure_frame_time`).
pub const TIMING: &str = "paired frame timing";
/// A fresh-context evaluator session scoring a rubric (`evaluate_rubric`).
pub const EVALUATOR: &str = "a fresh evaluator";

/// Advertised flag for each capability that has no tool of its own.
const ADVERTISED: [(&str, &str); 6] = [
    (HEALTH, "extensionHealth"),
    (OWNERS, "registrationOwners"),
    (TRANSITIONS, "transitions"),
    (PANELS, "panels"),
    (THEMES, "themes"),
    (EVALUATOR, "evaluator"),
];

/// Capabilities backed by host tools.
const TOOLS: [(&str, &[&str]); 4] = [
    (FRAMES, &["render_frames"]),
    (PROBES, &["open_panel", "get_panel_state", "interact_panel"]),
    (SEEK, &["set_playhead"]),
    (TIMING, &["measure_frame_time"]),
];

/// Every capability the host lacks, by name.
pub fn lacking(initialize: &Value, tools: &[String]) -> Vec<String> {
    let flags = initialize
        .pointer("/capabilities/experimental/powermoveEval")
        .cloned()
        .unwrap_or(Value::Null);
    let mut out = Vec::new();
    for (name, needs) in TOOLS {
        if !needs.iter().all(|n| tools.iter().any(|t| t == n)) {
            out.push(name.to_owned());
        }
    }
    for (name, flag) in ADVERTISED {
        if flags.get(flag) != Some(&Value::Bool(true)) {
            out.push(name.to_owned());
        }
    }
    if flags.get("evaluator") == Some(&Value::Bool(true))
        && !tools.iter().any(|t| t == "evaluate_rubric")
    {
        out.push(EVALUATOR.to_owned());
    }
    out.sort();
    out.dedup();
    out
}
