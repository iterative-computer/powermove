//! The runner: one task on one harness from a clean lane profile, or a batch
//! with interleaved repeats. Each run seeds a fresh profile with the fixture,
//! starts the headless host, captures the baseline, runs the harness, collects
//! evidence, grades it and writes `run.json` + `artifacts.json`.
pub mod collect;
pub mod harness;
pub mod mcp;

use crate::artifacts::Artifacts;
use crate::graders::{Context, GraderSpec};
use crate::record::{FORMAT, RunRecord, Usage};
use crate::suite::{Suite, Task};
use harness::{HarnessId, RunContext};
use std::path::{Path, PathBuf};

/// One batch's settings.
#[derive(Clone, Debug)]
pub struct Config {
    pub data_root: PathBuf,
    /// `powermove-mcp`, built elsewhere (the runner never builds it).
    pub mcp_bin: PathBuf,
    /// The harness CLI (`codex`, `claude`) for those harnesses.
    pub harness_bin: Option<PathBuf>,
    pub out: PathBuf,
    pub model: String,
    pub effort: String,
    pub repeats: u32,
    /// Required for any run that would reach a real model: who approved the
    /// batch and when. Recorded in every run.
    pub approved: Option<String>,
    /// Replay a recording instead of a model (no approval needed).
    pub replay: Option<PathBuf>,
    pub ablate: Vec<String>,
}

/// Refuse a run that would use a paid model without an approval note.
pub fn guard(harness: HarnessId, cfg: &Config) -> Result<(), String> {
    if harness.uses_model() && cfg.replay.is_none() && cfg.approved.is_none() {
        return Err(format!(
            "{} would run a real model: real-model batches need the Chief's OK (pass --approved \"<who, when>\") or use --replay <recording>",
            harness.as_str()
        ));
    }
    if let Some(a) = &cfg.approved
        && a.trim().len() < 8
    {
        return Err("--approved needs a real note (who approved, when)".into());
    }
    Ok(())
}

/// One slot of a batch.
#[derive(Clone, Debug, PartialEq)]
pub struct Slot {
    pub order: u32,
    pub rep: u32,
    pub task: String,
    pub harness: HarnessId,
}

/// Interleave repeats: every repeat runs every task on every harness before
/// the next repeat starts, and the harness order rotates by repeat and task so
/// each harness goes first equally often (drift in load or provider latency
/// hits all harnesses alike).
pub fn plan(tasks: &[&Task], harnesses: &[HarnessId], repeats: u32) -> Vec<Slot> {
    let mut out = Vec::new();
    let h = harnesses.len().max(1);
    for rep in 0..repeats {
        for (ti, t) in tasks.iter().enumerate() {
            for k in 0..harnesses.len() {
                let harness = harnesses[(k + rep as usize + ti) % h];
                out.push(Slot {
                    order: out.len() as u32,
                    rep,
                    task: t.id.clone(),
                    harness,
                });
            }
        }
    }
    out
}

pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Grade collected artifacts against a task's graders.
pub fn grade(
    task: &Task,
    artifacts: &Artifacts,
    run_dir: &Path,
    data_root: &Path,
) -> Vec<crate::verdict::Verdict> {
    let cx = Context {
        artifacts,
        run_dir,
        data_root,
    };
    task.graders.iter().map(|g| g.grade(&cx)).collect()
}

fn judges_render(task: &Task) -> Vec<bool> {
    task.graders.iter().map(GraderSpec::judges_render).collect()
}

/// Run one slot. Setup or harness failures still produce a record (with
/// `error`), so a batch never silently drops a run.
pub fn run_slot(suite: &Suite, task: &Task, slot: &Slot, cfg: &Config) -> RunRecord {
    let run_dir = cfg.out.join(&task.id).join(format!(
        "{}-{}-{}-r{}",
        slot.harness.as_str(),
        cfg.model,
        cfg.effort,
        slot.rep
    ));
    let mut record = RunRecord {
        format: FORMAT.into(),
        suite_version: suite.version,
        task: task.id.clone(),
        kind: task.kind.as_str().into(),
        harness: slot.harness.as_str().into(),
        model: cfg.model.clone(),
        effort: cfg.effort.clone(),
        rep: slot.rep,
        order: slot.order,
        started_unix_ms: now_ms(),
        wall_ms: 0,
        success: false,
        renders_correct: None,
        unavailable: false,
        tokens: Usage::default(),
        tool_calls: 0,
        retries: 0,
        gate_continuations: None,
        timed_out: false,
        over_step_limit: false,
        exit_code: None,
        needs_fix: None,
        verdicts: Vec::new(),
        approved: cfg.approved.clone(),
        replay: cfg
            .replay
            .as_ref()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().into_owned()),
        error: None,
    };
    if let Err(e) = execute(task, slot.harness, cfg, &run_dir, &mut record) {
        record.error = Some(e);
    }
    record.conclude(&judges_render(task));
    let _ = std::fs::create_dir_all(&run_dir);
    let _ = std::fs::write(
        run_dir.join("run.json"),
        serde_json::to_vec_pretty(&record).unwrap_or_default(),
    );
    record
}

fn execute(
    task: &Task,
    harness_id: HarnessId,
    cfg: &Config,
    run_dir: &Path,
    record: &mut RunRecord,
) -> Result<(), String> {
    guard(harness_id, cfg)?;
    if run_dir.exists() {
        return Err(format!(
            "{} already exists: every run starts from a clean folder",
            run_dir.display()
        ));
    }
    let dirs = collect::RunDirs::create(run_dir)?;
    let project_id = collect::seed_profile(task, &cfg.data_root, &dirs)?;
    let mut host = mcp::Host::spawn(&cfg.mcp_bin, &dirs.profile, &dirs.exports, &dirs.host_log)?;
    let mut artifacts = Artifacts::default();
    let result = (|| -> Result<(), String> {
        let baseline =
            collect::baseline(task, &project_id, &dirs, &mut host.client, &mut artifacts)?;
        let replay = match &cfg.replay {
            Some(p) => Some(crate::replay::replayer::ReplayServer::start(
                crate::replay::replayer::Replayer::new(
                    crate::replay::Recording::load(p)?,
                    vec![dirs.root.display().to_string()],
                ),
            )?),
            None => None,
        };
        let cx = RunContext {
            prompt: &task.prompt,
            model: &cfg.model,
            effort: &cfg.effort,
            limits: &task.limits,
            workdir: &dirs.work,
            home: &dirs.home,
            profile: &dirs.profile,
            mcp_command: &cfg.mcp_bin,
            mcp_args: vec![
                "--powermove-mcp".into(),
                "--profile".into(),
                dirs.profile.display().to_string(),
            ],
            bin: cfg.harness_bin.as_deref(),
            model_base_url: replay.as_ref().map(|r| r.origin()),
            transcript: &dirs.root.join("transcript.ndjson"),
            ablate: &cfg.ablate,
        };
        let outcome = harness::for_id(harness_id).run(&cx, &mut host.client);
        if let Some(r) = replay {
            let report = r.finish();
            std::fs::write(
                dirs.root.join("replay-report.json"),
                serde_json::to_vec_pretty(&report).unwrap_or_default(),
            )
            .map_err(|e| e.to_string())?;
            if !report.passed {
                artifacts.notes.push(format!(
                    "replay diverged or was incomplete: {}/{} exchanges",
                    report.consumed, report.exchanges
                ));
            }
        }
        let outcome = outcome?;
        record.wall_ms = outcome.wall_ms;
        record.tokens = outcome.usage;
        record.tool_calls = outcome.tool_calls;
        record.retries = outcome.retries;
        record.gate_continuations = outcome.gate_continuations;
        record.timed_out = outcome.timed_out;
        record.exit_code = outcome.exit_code;
        record.over_step_limit = outcome.tool_calls > task.limits.steps;
        collect::after(
            task,
            &project_id,
            &dirs,
            &baseline,
            &mut host.client,
            &mut artifacts,
        );
        Ok(())
    })();
    host.close(std::time::Duration::from_secs(10));
    artifacts.save(run_dir)?;
    result?;
    record.verdicts = grade(task, &artifacts, run_dir, &cfg.data_root);
    record.needs_fix = artifacts.spot_check.as_ref().and_then(|s| s.needs_fix);
    Ok(())
}

/// Grade a finished run again from its saved artifacts (e.g. after the owner
/// adds `spot-check.json`), rewriting `run.json`.
pub fn regrade(suite: &Suite, run_dir: &Path, data_root: &Path) -> Result<RunRecord, String> {
    let bytes = std::fs::read(run_dir.join("run.json")).map_err(|e| e.to_string())?;
    let mut record: RunRecord = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    let task = suite
        .task(&record.task)
        .ok_or_else(|| format!("task {} is not in this suite", record.task))?;
    if record.suite_version != suite.version {
        return Err(format!(
            "run is from suite v{}, this is v{}",
            record.suite_version, suite.version
        ));
    }
    let artifacts = Artifacts::load(run_dir)?;
    record.verdicts = grade(task, &artifacts, run_dir, data_root);
    record.needs_fix = artifacts.spot_check.as_ref().and_then(|s| s.needs_fix);
    record.conclude(&judges_render(task));
    std::fs::write(
        run_dir.join("run.json"),
        serde_json::to_vec_pretty(&record).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    Ok(record)
}

/// Every `run.json` under a folder.
pub fn load_runs(dir: &Path) -> Result<Vec<RunRecord>, String> {
    let mut out = Vec::new();
    fn walk(d: &Path, out: &mut Vec<RunRecord>) -> Result<(), String> {
        for e in std::fs::read_dir(d).map_err(|e| format!("{}: {e}", d.display()))? {
            let p = e.map_err(|e| e.to_string())?.path();
            if p.is_dir() {
                walk(&p, out)?;
            } else if p.file_name().is_some_and(|n| n == "run.json") {
                let bytes = std::fs::read(&p).map_err(|e| e.to_string())?;
                out.push(
                    serde_json::from_slice(&bytes).map_err(|e| format!("{}: {e}", p.display()))?,
                );
            }
        }
        Ok(())
    }
    walk(dir, &mut out)?;
    out.sort_by_key(|r| (r.started_unix_ms, r.order));
    Ok(out)
}
