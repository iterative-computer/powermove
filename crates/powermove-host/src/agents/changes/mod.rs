//! Publishing the mods agents write (port of `codex/change-history.ts`,
//! `codex/validate-staged-extensions.ts`, `extensions/update-fork.ts` and
//! the extension parts of `codex/index.ts` and `agent-tools/bridge.ts`).
//!
//! An autonomous run edits a private stage (`workspace.rs`). When it
//! finishes, [`promote`] snapshots the changed folders, checks each one
//! (manifest, credential scan, the real mod compiler) and swaps them into
//! the live folder with an undo record (`history.rs`); [`refresh`] then
//! rebuilds them and sends `ext:changed`. Also here: the
//! `codex:restore-change-set`, `codex:update-fork` and `codex:rebase-prompt`
//! channels ([`install`]), the `fork_builtin_extension` and
//! `stage_fork_rebase` agent tools ([`call_stage_tool`]), and boot recovery
//! ([`recover_all`]).

mod history;
#[cfg(test)]
mod live_tests;
mod scan;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use anyhow::{Result, anyhow, bail};
use parking_lot::Mutex;
use serde_json::{Value, json};

use super::workspace::{ValidationError, safe_component};
use crate::extensions::compiler::Compiler;
use crate::extensions::{fork, manifest::parse_manifest, rebase};
use crate::ids::{is_extension_id, is_project_id};
use crate::router::Router;
pub use history::{Action, Change, Record, Stage, parse_changes, recover};

/// Fork updates keep their undo records under this pseudo-project.
const FORK_UPDATE_PROJECT_ID: &str = "fork-updates";

/// `<userData>/Agent Change History/<project>`.
pub fn history_root(user_data: &Path, project_id: &str) -> PathBuf {
    user_data.join("Agent Change History").join(safe_component(project_id, "project"))
}

/// Check the declared changes in a snapshot (`validateStagedExtensions`).
/// Every failure the agent can fix is a [`ValidationError`].
pub fn validate(staging: &Path, compiled: &Path, changes: &[Change], compiler: &Compiler) -> Result<()> {
    let repair = |message: String| -> anyhow::Error { ValidationError(message).into() };
    for change in changes {
        let id = &change.id;
        if !is_extension_id(id) {
            bail!("Invalid staged extension id.");
        }
        if change.action == Action::Removed {
            continue;
        }
        let dir = staging.join(id);
        let raw = std::fs::read(dir.join("manifest.json")).ok().and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok());
        let Some(raw) = raw else { return Err(repair(format!("{id}: provide a valid manifest.json in the current staging directory. Nothing was published."))) };
        let manifest = parse_manifest(&raw).map_err(|error| repair(format!("{id}: {error}. Nothing was published.")))?;
        if &manifest.id != id {
            return Err(repair(format!("{id}: the manifest id must match its folder. Nothing was published.")));
        }
        let blocked = scan::scan_folder(&dir)?;
        if !blocked.is_empty() {
            let lines: Vec<String> = blocked.iter().map(|finding| format!("{id}/{}:{} looks like {}. Declare it in manifest vars and read api.vars. Nothing was published.", finding.path, finding.line, scan::label(finding.kind))).collect();
            return Err(repair(lines.join("\n")));
        }
        compiler.compile(&dir, manifest.entry(), compiled).map_err(|error| repair(format!("{id} failed compilation: {error}. Repair its staged source. Nothing was published.")))?;
    }
    Ok(())
}

/// Snapshot the stage, validate it, then publish it. `None` when nothing
/// changed. Blocking. `cancelled` is checked once more before publishing.
pub fn promote(stage: &Stage, declared: &[Change], compiler: &Compiler, cancelled: impl Fn() -> bool) -> Result<Option<Record>> {
    let snapshot = history::Snapshot::take(stage, declared)?;
    validate(&snapshot.staging, &snapshot.compiled, declared, compiler)?;
    if cancelled() {
        bail!("The run was cancelled.");
    }
    history::publish(stage, &snapshot, declared)
}

/// Rebuild promoted or restored ids and send `ext:changed` (`refreshExtensions`).
pub async fn refresh(user_data: &Path, ids: Vec<String>) {
    let Some(extensions) = crate::extensions::for_profile(user_data) else { return };
    if let Err(error) = extensions.refresh(Some(ids), "reload").await {
        tracing::warn!("could not refresh promoted extensions: {error:#}");
    }
}

/// Roll back interrupted publishes before the extension registry first
/// scans the live folder. Call once at boot.
pub async fn recover_all(user_data: &Path) {
    let (user_data, live) = (user_data.to_path_buf(), user_data.join("extensions"));
    match tokio::task::spawn_blocking(move || history::recover_all(&user_data, &live)).await {
        Ok(Ok(())) => {}
        Ok(Err(error)) => tracing::warn!("could not recover interrupted extension changes: {error:#}"),
        Err(error) => tracing::warn!("extension recovery stopped: {error}"),
    }
}

async fn blocking<T: Send + 'static>(work: impl FnOnce() -> Result<T> + Send + 'static) -> Result<T> {
    tokio::task::spawn_blocking(work).await.map_err(|error| anyhow!("extension task stopped: {error}"))?
}

/* ── the fork tools ─────────────────────────────────────────── */

/// Where a run's fork tools write: its stage, the live folder they read
/// forks from, and the shipped built-ins.
#[derive(Clone)]
struct RunStage {
    staging: PathBuf,
    live: PathBuf,
    builtin: Option<PathBuf>,
}

static RUN_STAGES: LazyLock<Mutex<HashMap<String, RunStage>>> = LazyLock::new(Mutex::default);

/// Forgets the run's stage when dropped.
pub struct StageGuard(String);

impl Drop for StageGuard {
    fn drop(&mut self) {
        RUN_STAGES.lock().remove(&self.0);
    }
}

/// Make run `run_id`'s stage reachable from its fork tools while the guard lives.
pub fn track_stage(run_id: &str, user_data: &Path, staging: &Path, live: &Path) -> StageGuard {
    let builtin = crate::extensions::for_profile(user_data).map(|extensions| extensions.builtin_dir().to_path_buf());
    RUN_STAGES.lock().insert(run_id.to_owned(), RunStage { staging: staging.into(), live: live.into(), builtin });
    StageGuard(run_id.to_owned())
}

pub fn is_stage_tool(tool: &str) -> bool {
    tool == "fork_builtin_extension" || tool == "stage_fork_rebase"
}

/// `fork_builtin_extension` and `stage_fork_rebase` for run `run_id`; the tool's JSON text.
pub async fn call_stage_tool(run_id: &str, tool: &str, args: &Value) -> Result<String> {
    let arguments = args.as_object().cloned().unwrap_or_default();
    let id = arguments.get("id").and_then(Value::as_str).filter(|id| is_extension_id(id)).map(str::to_owned);
    let stage = || -> Result<(RunStage, PathBuf)> {
        let stage = RUN_STAGES.lock().get(run_id).cloned().ok_or_else(|| anyhow!("The current run extension stage is not ready."))?;
        let builtin = stage.builtin.clone().ok_or_else(|| anyhow!("Powermove built-in extensions are unavailable."))?;
        Ok((stage, std::path::absolute(builtin)?))
    };
    if tool == "fork_builtin_extension" {
        let fork_id = arguments.get("forkId");
        let valid_fork = fork_id.is_none_or(|value| value.as_str().is_some_and(is_extension_id));
        let (Some(id), true, true) = (id, valid_fork, arguments.keys().all(|key| key == "id" || key == "forkId")) else { bail!("fork_builtin_extension expects {{ id, forkId? }}.") };
        let fork_id = fork_id.and_then(Value::as_str).map(str::to_owned);
        let (stage, builtin) = stage()?;
        let forked = blocking(move || fork::fork_builtin(&builtin, &id, &std::path::absolute(&stage.staging)?, fork_id.as_deref())).await?;
        return Ok(json!({
            "dir": forked.dir,
            "forkId": forked.fork_id,
            "files": forked.files,
            "forkedFrom": forked.forked_from,
            "reminder": format!("List {} in the result's extensions array with action created.", forked.fork_id),
        })
        .to_string());
    }
    let (Some(id), 1) = (id, arguments.len()) else { bail!("stage_fork_rebase expects {{ id }} with a valid extension id.") };
    let (stage, builtin) = stage()?;
    let staged = blocking(move || rebase::stage_fork_rebase(&id, &std::path::absolute(&stage.staging)?, &std::path::absolute(&stage.live)?, &builtin)).await?;
    Ok(serde_json::to_string(&staged)?)
}

/* ── channels ───────────────────────────────────────────────── */

/// `buildRebasePrompt`.
fn rebase_prompt(info: &rebase::ForkRebaseInfo) -> String {
    let (fork_id, forked_from, base, current) = (&info.fork_id, &info.forked_from, &info.base, &info.current);
    let quoted = Value::String(fork_id.clone());
    format!(
        "Update the user fork `{fork_id}`. It was forked from `{forked_from}@{base}`, and Powermove now ships `{forked_from}@{current}`.\n\nCall `stage_fork_rebase` with {{\"id\":{quoted}}}. Work only inside the returned staging paths. Powermove has already merged straightforward changes in `mergedFiles` into `workingDir`. Resolve only the remaining `conflicts` using a three-way comparison: `baseDir` is the old base, the working copy retains the user's customizations, and `oursDir` is the shipped version. Preserve the user's changes, behavior, and intent; do not redo completed merges. Compile the result and repair any remaining validation failure. Explain each conflict resolution in your final response. When finished, return the fork in the result's `extensions` array as {{\"id\":{quoted},\"action\":\"updated\",\"summary\":\"...\"}}."
    )
}

/// Merge a stale fork onto the shipped built-in, compile it and promote it
/// with an undo record, without an agent (`updateFork`). Blocking.
fn update_fork(fork_id: &str, user_data: &Path, live: &Path, builtin: &Path, compiler: &Compiler) -> Result<Value> {
    let temporary = user_data.join("Fork Updates").join(format!("update-{}", uuid::Uuid::new_v4().simple()));
    std::fs::create_dir_all(&temporary)?;
    let result = (|| -> Result<Value> {
        let run_id = format!("fork-update-{}", uuid::Uuid::new_v4());
        let stage = Stage::prepare(live, &temporary.join("stage"), &history_root(user_data, FORK_UPDATE_PROJECT_ID), FORK_UPDATE_PROJECT_ID, &run_id)?;
        // Read from the isolated baseline; publication refuses intervening edits.
        let rebased = rebase::stage_fork_rebase(fork_id, &temporary.join("merge"), &stage.staging, builtin)?;
        if !rebased.conflicts.is_empty() {
            return Ok(json!({ "kind": "needs-agent", "reason": "conflict", "conflicts": rebased.conflicts }));
        }
        // Keep every other folder at its baseline; replace only this fork.
        std::fs::remove_dir_all(stage.staging.join(fork_id))?;
        std::fs::rename(&rebased.working_dir, stage.staging.join(fork_id))?;
        let changes = [Change { summary: Some(format!("Updated to {} {}; customizations kept.", rebased.info.forked_from, rebased.info.current)), ..Change::new(fork_id, Action::Updated) }];
        match promote(&stage, &changes, compiler, || false) {
            Ok(Some(record)) => Ok(json!({ "kind": "updated", "version": rebased.info.current, "projectId": FORK_UPDATE_PROJECT_ID, "changeSetId": record.id })),
            Ok(None) => bail!("The extension update made no changes."),
            Err(error) if error.is::<ValidationError>() => Ok(json!({ "kind": "needs-agent", "reason": "validation", "conflicts": [] })),
            Err(error) => Err(error),
        }
    })();
    let _ = std::fs::remove_dir_all(&temporary);
    result
}

fn fork_request(value: &Value, channel: &str) -> Result<String> {
    value["id"].as_str().filter(|id| is_extension_id(id)).map(str::to_owned).ok_or_else(|| anyhow!("{channel}: invalid request"))
}

/// One fork update at a time, like the Mac app's queue.
static FORK_UPDATES: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Register `codex:rebase-prompt`, `codex:update-fork` and `codex:restore-change-set`.
pub fn install(router: &mut Router) {
    router.json("codex:rebase-prompt", |call| async move {
        let id = fork_request(call.arg(0), "codex:rebase-prompt")?;
        let config = &call.host.config;
        let (live, builtin) = (std::path::absolute(config.user_data.join("extensions"))?, std::path::absolute(&config.builtin_extensions)?);
        let info = blocking(move || rebase::read_fork_rebase_info(&id, &live, &builtin)).await?;
        Ok(json!(rebase_prompt(&info)))
    });
    router.json("codex:update-fork", |call| async move {
        let id = fork_request(call.arg(0), "codex:update-fork")?;
        let config = &call.host.config;
        let user_data = config.user_data.clone();
        let (live, builtin) = (std::path::absolute(user_data.join("extensions"))?, std::path::absolute(&config.builtin_extensions)?);
        let compiler_dir = config.resources_dir.join("compiler");
        let _queue = FORK_UPDATES.lock().await;
        let (fork_id, data) = (id.clone(), user_data.clone());
        let result = blocking(move || update_fork(&fork_id, &data, &live, &builtin, &Compiler::new(compiler_dir))).await?;
        // The undo reference is returned even if the refresh fails.
        if result["kind"] == "updated" {
            refresh(&user_data, vec![id]).await;
        }
        Ok(result)
    });
    router.json("codex:restore-change-set", |call| async move {
        let value = call.arg(0);
        let project = value["projectId"].as_str().filter(|id| is_project_id(id));
        let change_set = value["changeSetId"].as_str().filter(|id| (1..=160).contains(&id.len()) && id.bytes().all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-'));
        let (Some(project), Some(change_set)) = (project, change_set) else { bail!("codex:restore-change-set: invalid change-set request") };
        let user_data = call.host.config.user_data.clone();
        let (live, history, change_set) = (user_data.join("extensions"), history_root(&user_data, project), change_set.to_owned());
        let record = blocking(move || history::restore(&live, &history, &change_set)).await?;
        refresh(&user_data, record.changes.iter().map(|change| change.id.clone()).collect()).await;
        Ok(json!({ "changeSetId": record.id, "extensions": record.changes }))
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::extensions::compiler::tests::{staged_compiler, write};

    fn manifest(id: &str) -> String {
        format!(r#"{{ "id": "{id}", "name": "Tint", "version": "1.0.0", "apiVersion": 3 }}"#)
    }

    /// Stage, validate with the real compiler, promote, and refuse what fails to compile.
    #[test]
    fn promotes_only_what_compiles() {
        let Some(compiler_dir) = staged_compiler() else { return };
        let compiler = Compiler::new(compiler_dir);
        let root = tempfile::tempdir().unwrap();
        let live = root.path().join("live");
        let stage = Stage::prepare(&live, &root.path().join("stage"), &root.path().join("history"), "p1", "run-1").unwrap();
        let created = [Change::new("tint", Action::Created)];

        write(&stage.staging.join("tint"), &[("manifest.json", &manifest("tint")), ("index.ts", "import x from 'left-pad'; export default x;")]);
        let error = promote(&stage, &created, &compiler, || false).unwrap_err();
        assert_eq!(error.downcast_ref::<ValidationError>().unwrap().0, "tint failed compilation: Unsupported bare import: left-pad. Repair its staged source. Nothing was published.");
        assert!(!live.join("tint").exists());

        write(&stage.staging.join("tint"), &[("manifest.json", &manifest("other"))]);
        assert!(promote(&stage, &created, &compiler, || false).unwrap_err().to_string().contains("the manifest id must match its folder"));

        write(&stage.staging.join("tint"), &[("manifest.json", &manifest("tint")), ("index.ts", "export default function activate() { return 1; }")]);
        assert_eq!(promote(&stage, &created, &compiler, || true).unwrap_err().to_string(), "The run was cancelled.");
        let record = promote(&stage, &created, &compiler, || false).unwrap().unwrap();
        assert_eq!(record.changes, created);
        assert!(live.join("tint").join("index.ts").is_file());
        assert!(promote(&stage, &[], &compiler, || false).is_err(), "a stale stage is refused");
    }

    #[test]
    fn refuses_staged_credentials() {
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join("tint");
        write(&dir, &[("manifest.json", &manifest("tint")), ("index.ts", &format!("const key = 'sk-{}';", "x".repeat(20)))]);
        let error = validate(root.path(), &root.path().join("out"), &[Change::new("tint", Action::Created)], &Compiler::new(root.path())).unwrap_err();
        assert_eq!(error.to_string(), "tint/index.ts:1 looks like an OpenAI API key. Declare it in manifest vars and read api.vars. Nothing was published.");
    }

    #[test]
    fn parses_reports_and_prompts_like_the_mac_app() {
        let changes = parse_changes(&json!([{ "id": "tint", "action": "created", "summary": "s" }, { "id": "Bad", "action": "created" }, { "id": "x2", "action": "moved" }])).unwrap();
        assert_eq!(serde_json::to_value(&changes).unwrap(), json!([{ "id": "tint", "action": "created", "summary": "s" }]));
        assert_eq!(parse_changes(&Value::Null), None);
        let info = rebase::ForkRebaseInfo { fork_id: "tools-fork".into(), forked_from: "tools".into(), base: "1.0.0".into(), current: "2.0.0".into() };
        let prompt = rebase_prompt(&info);
        assert!(prompt.starts_with("Update the user fork `tools-fork`. It was forked from `tools@1.0.0`, and Powermove now ships `tools@2.0.0`.\n\nCall `stage_fork_rebase` with {\"id\":\"tools-fork\"}."));
        assert!(prompt.ends_with("as {\"id\":\"tools-fork\",\"action\":\"updated\",\"summary\":\"...\"}."));
    }

    #[tokio::test]
    async fn fork_tools_write_into_the_run_stage() {
        let root = tempfile::tempdir().unwrap();
        let builtin = root.path().join("builtin");
        write(&builtin.join("tools"), &[("manifest.json", r#"{ "id": "tools", "name": "Tools", "version": "1.0.0", "apiVersion": 3 }"#), ("index.ts", "export default () => {}")]);
        let staging = root.path().join("stage");
        std::fs::create_dir_all(&staging).unwrap();
        RUN_STAGES.lock().insert("run-forks".into(), RunStage { staging: staging.clone(), live: root.path().join("live"), builtin: Some(builtin) });
        let guard = StageGuard("run-forks".into());

        let text = call_stage_tool("run-forks", "fork_builtin_extension", &json!({ "id": "tools" })).await.unwrap();
        let result: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(result["forkId"], "tools-fork");
        assert_eq!(result["reminder"], "List tools-fork in the result's extensions array with action created.");
        assert!(staging.join("tools-fork").join(".forked-from").join("index.ts").is_file());
        assert!(call_stage_tool("run-forks", "fork_builtin_extension", &json!({ "id": "tools", "extra": 1 })).await.is_err());
        let error = call_stage_tool("run-forks", "stage_fork_rebase", &json!({ "id": "tools-fork" })).await.unwrap_err();
        assert_eq!(error.to_string(), "Unknown extension: tools-fork", "rebases read forks from the live folder");
        drop(guard);
        assert_eq!(call_stage_tool("run-forks", "stage_fork_rebase", &json!({ "id": "tools-fork" })).await.unwrap_err().to_string(), "The current run extension stage is not ready.");
    }
}
