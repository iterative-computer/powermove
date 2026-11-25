//! The runner: one task on one harness from a clean lane profile, or a batch
//! with interleaved repeats. Each run reserves a fresh folder, seeds a clean
//! profile with the fixture, starts the headless host, checks what evidence
//! it can supply, captures the baseline, runs the harness, collects
//! evidence, grades it and writes `run.json` + `artifacts.json`.
//!
//! Replays and the `none` harness run under the OS boundary in [`sandbox`]
//! and fail closed; live runs are refused until batch credentials are
//! approved ([`harness::Credentials`]).
pub mod capabilities;
pub mod collect;
pub mod harness;
pub mod mcp;
pub mod paths;
pub mod proc;
pub mod sandbox;

use crate::artifacts::Artifacts;
use crate::graders::{Context, GraderSpec};
use crate::record::{FORMAT, RunRecord};
use crate::replay::Recording;
use crate::suite::{Suite, Task};
use harness::{HarnessId, Provider, RunContext};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Instant;

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
    /// Who approved a real-model batch and when; recorded in every run. Live
    /// runs are still refused until batch credentials are approved.
    pub approved: Option<String>,
    /// Replay a recording instead of a model (no approval needed).
    pub replay: Option<PathBuf>,
    pub ablate: Vec<String>,
    /// The batch's id, stored in every run (defaults to its start time).
    pub cohort: String,
    pub deadlines: mcp::Deadlines,
    /// More paths a confined run may read (a CLI's install folder).
    pub sandbox_read: Vec<PathBuf>,
    /// More roots a confined run may not read (tests add canary folders).
    pub sandbox_protect: Vec<PathBuf>,
}

impl Config {
    /// `live`, `replay` or `none`.
    pub fn mode(&self, harness: HarnessId) -> &'static str {
        if harness == HarnessId::None {
            "none"
        } else if self.replay.is_some() {
            "replay"
        } else {
            "live"
        }
    }
}

/// Refuse a run that would reach a paid model: until the Chief approves how a
/// batch supplies credentials, every live run stops here.
pub fn guard(harness: HarnessId, cfg: &Config) -> Result<(), String> {
    if let Some(a) = &cfg.approved
        && a.trim().len() < 8
    {
        return Err("--approved needs a real note (who approved, when)".into());
    }
    if cfg.mode(harness) == "live" {
        if cfg.approved.is_none() {
            return Err(format!(
                "{} would run a real model: real-model batches need the Chief's OK (--approved \"<who, when>\") or use --replay <recording>",
                harness.as_str()
            ));
        }
        use harness::Credentials;
        harness::NotApproved.env(harness, Path::new(""))?;
    }
    Ok(())
}

/// A recording replays only the slot it was made for.
pub fn check_recording(
    rec: &Recording,
    task: &Task,
    harness: HarnessId,
    cfg: &Config,
) -> Result<(), String> {
    let mut wrong = Vec::new();
    for (what, got, want) in [
        ("task", rec.task.as_str(), task.id.as_str()),
        ("harness", rec.harness.as_str(), harness.as_str()),
        ("model", rec.model.as_str(), cfg.model.as_str()),
        ("effort", rec.effort.as_str(), cfg.effort.as_str()),
    ] {
        if got != want {
            wrong.push(format!("{what} {got} (the slot is {want})"));
        }
    }
    if wrong.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "the recording does not match the slot: {}",
            wrong.join(", ")
        ))
    }
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

/// The folder of one slot.
pub fn run_dir(cfg: &Config, task: &Task, slot: &Slot) -> PathBuf {
    let mut name = format!(
        "{}-{}-{}-r{}",
        slot.harness.as_str(),
        cfg.model,
        cfg.effort,
        slot.rep
    );
    if !cfg.ablate.is_empty() {
        name.push_str(&format!("-no-{}", cfg.ablate.join("+")));
    }
    cfg.out.join(&task.id).join(name)
}

/// Reserve a run folder atomically: a folder that already exists is never
/// touched (its record stays as it was).
fn reserve(dir: &Path) -> Result<(), String> {
    let parent = dir.parent().ok_or("a run folder needs a parent")?;
    std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
    match std::fs::create_dir(dir) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Err(format!(
            "{} already exists: every run starts from a clean folder (the existing record is left untouched)",
            dir.display()
        )),
        Err(e) => Err(format!("{}: {e}", dir.display())),
    }
}

/// Write a file that must not exist yet.
fn write_new(path: &Path, bytes: &[u8]) -> Result<(), String> {
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .and_then(|mut f| f.write_all(bytes).and_then(|()| f.sync_all()))
        .map_err(|e| format!("{}: {e}", path.display()))
}

/// Run one slot. Setup or harness failures still produce a record (with
/// `error`), so a batch never silently drops a run. Err only when the run
/// folder could not be reserved or the record could not be written.
pub fn run_slot(
    suite: &Suite,
    task: &Task,
    slot: &Slot,
    cfg: &Config,
) -> Result<RunRecord, String> {
    let dir = run_dir(cfg, task, slot);
    reserve(&dir)?;
    let started = now_ms();
    let mut ablation = cfg.ablate.clone();
    ablation.sort();
    let mut record = RunRecord {
        format: FORMAT.into(),
        suite_version: suite.version,
        task: task.id.clone(),
        kind: task.kind.as_str().into(),
        harness: slot.harness.as_str().into(),
        model: cfg.model.clone(),
        effort: cfg.effort.clone(),
        mode: cfg.mode(slot.harness).into(),
        cohort: cfg.cohort.clone(),
        ablation,
        run_id: format!("{}-{}-{started}", cfg.cohort, slot.order),
        rep: slot.rep,
        order: slot.order,
        started_unix_ms: started,
        wall_ms: None,
        success: false,
        renders_correct: None,
        unavailable: false,
        tokens: None,
        tool_calls: 0,
        retries: 0,
        gate_continuations: None,
        timed_out: false,
        over_step_limit: false,
        exit_code: None,
        harness_completed: None,
        stop_reason: None,
        replay_passed: None,
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
    if let Err(e) = execute(task, slot.harness, cfg, &dir, &mut record) {
        record.error.get_or_insert(e);
    }
    record.conclude(&judges_render(task));
    write_new(
        &dir.join("run.json"),
        &serde_json::to_vec_pretty(&record).map_err(|e| e.to_string())?,
    )?;
    Ok(record)
}

fn execute(
    task: &Task,
    harness_id: HarnessId,
    cfg: &Config,
    run_dir: &Path,
    record: &mut RunRecord,
) -> Result<(), String> {
    guard(harness_id, cfg)?;
    let recording = match &cfg.replay {
        Some(p) if harness_id != HarnessId::None => {
            let r = Recording::load(p)?;
            check_recording(&r, task, harness_id, cfg)?;
            Some(r)
        }
        _ => None,
    };
    let dirs = collect::RunDirs::create(run_dir)?;
    let mut binaries: Vec<&Path> = vec![&cfg.mcp_bin];
    if harness_id == HarnessId::Codex || harness_id == HarnessId::ClaudeCode {
        binaries.extend(cfg.harness_bin.as_deref());
    }
    binaries.extend(cfg.sandbox_read.iter().map(PathBuf::as_path));
    // The replay holds its port on every interface before anything starts;
    // the run's one outbound TCP port is that one.
    let replay = match recording {
        Some(r) => Some(crate::replay::replayer::ReplayServer::start(
            crate::replay::replayer::Replayer::new(r, vec![dirs.root.display().to_string()]),
        )?),
        None => None,
    };
    let confinement = if record.mode == "live" {
        None
    } else {
        let c = sandbox::Confinement::for_run(&dirs.root, &binaries, &cfg.sandbox_protect)?;
        let c = match &replay {
            Some(r) => c.allow_tcp(r.port()),
            None => c,
        };
        c.verify()?;
        Some(c)
    };
    let project_id = collect::seed_profile(task, &cfg.data_root, &dirs)?;
    let host_env = harness::base_env(&dirs.profile, &dirs.tmp);
    let mut host = mcp::Host::spawn(
        &cfg.mcp_bin,
        &dirs.profile,
        &dirs.exports,
        &dirs.host_log,
        &host_env,
        confinement.as_ref(),
        &cfg.deadlines,
    )?;
    let mut artifacts = Artifacts::default();
    let tools = host.client.tool_names().unwrap_or_default();
    artifacts.host_lacks = capabilities::lacking(&host.init, &tools);
    let mut hung = false;
    let result = (|| -> Result<(), String> {
        host.client.budget_end = Some(Instant::now() + cfg.deadlines.collect);
        let baseline =
            collect::baseline(task, &project_id, &dirs, &mut host.client, &mut artifacts)?;
        host.client.budget_end = None;
        let provider = match (&replay, harness_id) {
            (_, HarnessId::None) => Provider::None,
            (Some(r), _) => Provider::Replay {
                base_url: r.origin(),
            },
            (None, _) => Provider::Live,
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
            provider,
            transcript: &dirs.root.join("transcript.ndjson"),
            ablate: &cfg.ablate,
            confine: confinement.as_ref(),
            deadlines: &cfg.deadlines,
            run_id: &record.run_id,
            project_id: &project_id,
            revision: baseline.revision,
        };
        let outcome = harness::for_id(harness_id).run(&cx, &mut host.client);
        // The replay's verdict is kept whatever the harness did.
        if let Some(r) = replay {
            let report = r.finish();
            record.replay_passed = Some(report.passed);
            if !report.passed {
                artifacts.notes.push(format!(
                    "replay diverged or was incomplete: {}/{} exchanges",
                    report.consumed, report.exchanges
                ));
            }
            write_new(
                &dirs.root.join("replay-report.json"),
                &serde_json::to_vec_pretty(&report).map_err(|e| e.to_string())?,
            )?;
        }
        record.wall_ms = outcome.wall_ms;
        record.tokens = outcome.usage;
        record.tool_calls = outcome.tool_calls;
        record.retries = outcome.retries;
        record.gate_continuations = outcome.gate_continuations;
        record.timed_out = outcome.timed_out;
        record.over_step_limit = outcome.over_step_limit || outcome.tool_calls > task.limits.steps;
        record.exit_code = outcome.exit_code;
        record.harness_completed = Some(outcome.completed);
        record.stop_reason = Some(outcome.stop_reason.clone());
        if let Some(e) = outcome.error {
            record.error = Some(e);
        }
        if outcome.host_hung {
            hung = true;
            artifacts
                .notes
                .push("the host stopped answering; nothing was collected after the run".into());
            return Ok(());
        }
        // Evidence is collected even when the harness failed.
        host.client.budget_end = Some(Instant::now() + cfg.deadlines.collect);
        collect::after(
            task,
            &dirs,
            &baseline,
            &mut host.client,
            &mut artifacts,
            &cfg.data_root,
        );
        Ok(())
    })();
    let stopped = if hung {
        host.kill(cfg.deadlines.grace)
    } else {
        host.close(cfg.deadlines.grace)
    };
    if stopped.stragglers {
        artifacts
            .notes
            .push("processes of the host survived its stop".into());
        record
            .error
            .get_or_insert("processes of the host survived its stop".into());
    }
    artifacts.save(run_dir)?;
    result?;
    record.verdicts = grade(task, &artifacts, run_dir, &cfg.data_root);
    record.needs_fix = artifacts.spot_check.as_ref().and_then(|s| s.needs_fix);
    Ok(())
}

/// Grade a finished run again from its saved artifacts (e.g. after the owner
/// adds `spot-check.json`), rewriting `run.json`.
pub fn regrade(suite: &Suite, run_dir: &Path, data_root: &Path) -> Result<RunRecord, String> {
    let dir = paths::Anchor::open(run_dir)?;
    let bytes = dir.read(Path::new("run.json"))?;
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
    dir.replace(
        Path::new("run.json"),
        &serde_json::to_vec_pretty(&record).map_err(|e| e.to_string())?,
    )?;
    Ok(record)
}

/// Every `run.json` under a folder.
pub fn load_runs(dir: &Path) -> Result<Vec<RunRecord>, String> {
    let mut out = Vec::new();
    fn walk(d: &Path, out: &mut Vec<RunRecord>) -> Result<(), String> {
        for e in std::fs::read_dir(d).map_err(|e| format!("{}: {e}", d.display()))? {
            let e = e.map_err(|e| e.to_string())?;
            let p = e.path();
            let t = e.file_type().map_err(|e| e.to_string())?;
            if t.is_dir() {
                walk(&p, out)?;
            } else if t.is_file() && p.file_name().is_some_and(|n| n == "run.json") {
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

/// A default cohort id for a batch started now.
pub fn new_cohort() -> String {
    format!("batch-{}", now_ms())
}
