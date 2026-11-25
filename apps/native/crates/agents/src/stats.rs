//! Per-run counts for product telemetry (`agent_run_finished`): turns, tool
//! calls and the tokens a harness reports. Counts only, never anything a run
//! says. Bounded: a run never taken is dropped once more than `MAX` are kept.
use std::collections::{HashMap, HashSet};
use std::sync::Mutex;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct RunStats {
    pub turns: u32,
    pub tool_calls: u32,
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    tools: HashSet<String>,
    /// Codex: the usage of the turn in progress, added when it completes.
    turn_usage: Option<(u64, u64)>,
}

const MAX: usize = 256;
static RUNS: Mutex<Option<HashMap<String, RunStats>>> = Mutex::new(None);

fn with(run: &str, f: impl FnOnce(&mut RunStats)) {
    let Ok(mut runs) = RUNS.lock() else { return };
    let runs = runs.get_or_insert_with(HashMap::new);
    if !runs.contains_key(run) && runs.len() >= MAX {
        return;
    }
    f(runs.entry(run.to_owned()).or_default());
}

fn add(total: &mut Option<u64>, n: u64) {
    *total = Some(total.unwrap_or(0) + n);
}

/// One model turn finished.
pub fn turn(run: &str) {
    turns(run, 1);
}

/// Model turns a harness reported at once.
pub fn turns(run: &str, n: u32) {
    with(run, |s| s.turns = s.turns.saturating_add(n));
}

/// A tool call started (counted once per item).
pub fn tool_call(run: &str, item: &str) {
    with(run, |s| {
        if s.tools.insert(item.to_owned()) {
            s.tool_calls += 1;
        }
    });
}

/// Tokens a turn used, as the provider reported them.
pub fn tokens(run: &str, input: Option<u64>, output: Option<u64>) {
    with(run, |s| {
        if let Some(n) = input {
            add(&mut s.input_tokens, n);
        }
        if let Some(n) = output {
            add(&mut s.output_tokens, n);
        }
    });
}

/// Codex reports the running turn's usage as it goes; the last report counts.
pub fn turn_usage(run: &str, input: u64, output: u64) {
    with(run, |s| s.turn_usage = Some((input, output)));
}

/// Codex: a turn completed (its last usage report is added).
pub fn turn_completed(run: &str) {
    with(run, |s| {
        s.turns += 1;
        if let Some((input, output)) = s.turn_usage.take() {
            add(&mut s.input_tokens, input);
            add(&mut s.output_tokens, output);
        }
    });
}

/// The run's counts, forgotten here.
pub fn take(run: &str) -> RunStats {
    RUNS.lock()
        .ok()
        .and_then(|mut runs| runs.as_mut()?.remove(run))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    #[test]
    fn counts_turns_distinct_tool_calls_and_tokens_per_run() {
        let run = "stats-test-run";
        super::tool_call(run, "a");
        super::tool_call(run, "a");
        super::tool_call(run, "b");
        super::turn(run);
        super::tokens(run, Some(100), Some(20));
        super::turn_usage(run, 5, 1);
        super::turn_usage(run, 50, 10);
        super::turn_completed(run);
        let s = super::take(run);
        assert_eq!((s.turns, s.tool_calls), (2, 2));
        assert_eq!((s.input_tokens, s.output_tokens), (Some(150), Some(30)));
        assert_eq!(super::take(run), super::RunStats::default());
    }
}
