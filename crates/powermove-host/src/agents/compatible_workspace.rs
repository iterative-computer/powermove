//! Filesystem and shell tools for compatible models in an autonomous run
//! (port of `compatible-workspace.ts`).
//!
//! Codex and Claude bring their own file and shell tools; an API model gets
//! these instead. Paths stay inside the run's workspace in Project access,
//! including through links. Windows has no command sandbox for these runs, so
//! `run_command` exists only with Computer access; a Project run asks the
//! person through `run_outside_sandbox` instead.

use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Result, anyhow, bail};
use bytes::Bytes;
use parking_lot::Mutex;
use serde_json::{Value, json};
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;

use super::process;
use super::tools::Content;
use super::trace;
use super::workspace;

const COMMAND_TIMEOUT_MS: u64 = 600_000;
const MAX_BACKGROUND_JOBS: usize = 4;
const OUTPUT_CHARS: usize = 20_000;
const MAX_READ_BYTES: u64 = 8 * 1024 * 1024;
const MAX_WRITE_CHARS: usize = 1_000_000;
pub const COMPLETE_TASK: &str = "complete_task";

fn object(properties: Value, required: &[&str]) -> Value {
    json!({ "type": "object", "additionalProperties": false, "properties": properties, "required": required })
}

/// The tool specs (`{ name, description, inputSchema }`) a run gets.
pub fn tool_specs(commands: bool, result_schema: &Value) -> Vec<Value> {
    let mut tools = vec![
        json!({ "name": "list_files", "description": "List a workspace directory. Paths may be absolute or relative to the workspace.", "inputSchema": object(json!({ "path": { "type": "string" } }), &["path"]) }),
        json!({ "name": "read_file", "description": "Read a UTF-8 file, or return an image for visual inspection. Use offset/limit to page large text files. Read shipped API types and samples before implementing extensions.", "inputSchema": object(json!({ "path": { "type": "string" }, "offset": { "type": "integer", "minimum": 0 }, "limit": { "type": "integer", "minimum": 1, "maximum": 100_000 } }), &["path"]) }),
        json!({ "name": "write_file", "description": "Create or replace a UTF-8 file. Creates parent directories. Write extensions only in the supplied staging directory and deliverables in the run artifact directory. Read existing files before replacing them.", "inputSchema": object(json!({ "path": { "type": "string" }, "text": { "type": "string", "maxLength": MAX_WRITE_CHARS } }), &["path", "text"]) }),
    ];
    if commands {
        tools.push(json!({ "name": "run_command", "description": "Run a PowerShell command in the project workspace. Use for searching, scripts, tests, downloads and web research. Commands time out after 30 seconds unless timeoutMs allows up to 600 seconds. For longer work such as renders or installs, set background: true and follow the job with command_status. Background jobs stop when complete_task runs or the run ends. Output is bounded. Never repeat a timed-out mutation without inspecting its result.", "inputSchema": object(json!({ "command": { "type": "string", "maxLength": 100_000 }, "timeoutMs": { "type": "integer", "minimum": 1, "maximum": COMMAND_TIMEOUT_MS }, "background": { "type": "boolean" } }), &["command"]) }));
        tools.push(json!({ "name": "command_status", "description": "Check a background job from run_command: its state, exit code and recent output. Set waitMs to wait up to 120 seconds for it to finish, or stop: true to stop it.", "inputSchema": object(json!({ "jobId": { "type": "string" }, "waitMs": { "type": "integer", "minimum": 0, "maximum": 120_000 }, "stop": { "type": "boolean" } }), &["jobId"]) }));
    }
    tools.push(json!({ "name": COMPLETE_TASK, "description": "Finish the run with its summary, typed project commands, artifacts and all staged extension changes. Return commands: [] for edits already applied through live tools. If this tool fails, repair the reported problem and call it again.", "inputSchema": result_schema }));
    tools
}

/// `complete_task` arguments that match its schema's shape.
pub fn check_result(value: &Value) -> Result<()> {
    let array = |key: &str| value[key].as_array();
    let valid = value["summary"].is_string()
        && array("commands").is_some_and(|commands| commands.len() <= 80 && commands.iter().all(Value::is_string))
        && array("extensions").is_some_and(|extensions| extensions.len() <= 32)
        && array("artifacts").is_some()
        && array("notes").is_some()
        && array("externalActions").is_some();
    if !valid {
        bail!("complete_task requires summary, commands, artifacts, extensions, notes and externalActions matching its schema.");
    }
    Ok(())
}

#[derive(Default)]
struct JobState {
    running: bool,
    state: &'static str,
    exit_code: Option<i32>,
    output: String,
    truncated: bool,
}

struct Job {
    cancel: CancellationToken,
    state: Arc<Mutex<JobState>>,
    done: watch::Receiver<bool>,
}

/// One run's workspace tools.
pub struct Tools {
    root: PathBuf,
    computer: bool,
    /// Every command of the run stops with it.
    cancel: CancellationToken,
    jobs: Mutex<Vec<Job>>,
}

impl Drop for Tools {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}

fn bounded(args: &Value, key: &str, default: u64, min: u64, max: u64) -> Result<u64> {
    match &args[key] {
        Value::Null => Ok(default),
        value => value.as_u64().filter(|value| (min..=max).contains(value)).ok_or_else(|| anyhow!("Expected an integer between {min} and {max}.")),
    }
}

fn flag(args: &Value, key: &str) -> Result<bool> {
    match &args[key] {
        Value::Null => Ok(false),
        Value::Bool(value) => Ok(*value),
        _ => bail!("{key} must be true or false."),
    }
}

fn text(value: Value) -> Vec<Content> {
    vec![Content::Text(value.to_string())]
}

/// `..` and `.` resolved without touching the filesystem.
fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            other => out.push(other),
        }
    }
    out
}

fn tail(buffer: &mut String, line: &str, truncated: &mut bool) {
    buffer.push_str(line);
    buffer.push('\n');
    let count = buffer.chars().count();
    if count > OUTPUT_CHARS {
        *truncated = true;
        *buffer = buffer.chars().skip(count - OUTPUT_CHARS).collect();
    }
}

impl Tools {
    /// `computer`: the run has Computer access (unrestricted paths, `run_command`).
    pub fn new(root: &Path, computer: bool) -> Result<Self> {
        Ok(Self { root: dunce::canonicalize(root)?, computer, cancel: CancellationToken::new(), jobs: Mutex::default() })
    }

    pub fn has(&self, name: &str) -> bool {
        matches!(name, "list_files" | "read_file" | "write_file" | COMPLETE_TASK) || (self.computer && matches!(name, "run_command" | "command_status"))
    }

    /// Stop every command and background job of the run.
    pub fn stop_commands(&self) {
        for job in self.jobs.lock().iter() {
            job.cancel.cancel();
        }
    }

    /// Resolve existing ancestors too, so links cannot redirect a write.
    fn resolve(&self, file: &Value) -> Result<PathBuf> {
        let Some(file) = file.as_str().filter(|file| !file.is_empty() && !file.contains('\0')) else { bail!("Provide a valid file path.") };
        let candidate = normalize(&self.root.join(file));
        if self.computer {
            return Ok(candidate);
        }
        let contained = |path: &Path| path.starts_with(&self.root);
        if !contained(&candidate) {
            bail!("This path is outside the project workspace.");
        }
        let mut ancestor = candidate.as_path();
        loop {
            match dunce::canonicalize(ancestor) {
                Ok(real) if contained(&real) => return Ok(candidate),
                Ok(_) => bail!("This path leaves the project workspace through a link."),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => ancestor = ancestor.parent().ok_or_else(|| anyhow!("This path is outside the project workspace."))?,
                Err(error) => return Err(error.into()),
            }
        }
    }

    pub async fn call(&self, name: &str, args: &Value, cancel: &CancellationToken) -> Result<Vec<Content>> {
        match name {
            "list_files" => {
                let directory = self.resolve(&args["path"])?;
                let mut entries = tokio::fs::read_dir(&directory).await?;
                let mut listed = Vec::new();
                let mut total = 0;
                while let Some(entry) = entries.next_entry().await? {
                    total += 1;
                    if listed.len() < 1_000 {
                        listed.push(json!({ "name": entry.file_name().to_string_lossy(), "directory": entry.file_type().await.is_ok_and(|kind| kind.is_dir()) }));
                    }
                }
                Ok(text(json!({ "entries": listed, "total": total })))
            }
            "read_file" => {
                let file = self.resolve(&args["path"])?;
                let info = tokio::fs::metadata(&file).await?;
                if !info.is_file() || info.len() > MAX_READ_BYTES {
                    bail!("Read a regular file no larger than 8 MB; use another tool to inspect larger files.");
                }
                let data = tokio::fs::read(&file).await?;
                let mime = workspace::mime_type(&file);
                if mime == "image/png" || mime == "image/jpeg" {
                    return Ok(vec![Content::Image { data: Bytes::from(data), mime: mime.into() }]);
                }
                let source = String::from_utf8_lossy(&data);
                let total = source.chars().count() as u64;
                let offset = bounded(args, "offset", 0, 0, total)?;
                let limit = bounded(args, "limit", 30_000, 1, 100_000)?;
                let slice: String = source.chars().skip(offset as usize).take(limit as usize).collect();
                Ok(text(json!({ "text": slice, "offset": offset, "totalChars": total })))
            }
            "write_file" => {
                let Some(body) = args["text"].as_str().filter(|body| body.chars().count() <= MAX_WRITE_CHARS) else { bail!("Provide text no larger than 1,000,000 characters.") };
                let file = self.resolve(&args["path"])?;
                if let Some(parent) = file.parent() {
                    tokio::fs::create_dir_all(parent).await?;
                }
                // Re-check after creating parents: a link planted meanwhile is refused.
                let file = self.resolve(&args["path"])?;
                tokio::fs::write(&file, body).await?;
                Ok(text(json!({ "path": file, "bytes": body.len() })))
            }
            "run_command" if self.computer => {
                let Some(command) = args["command"].as_str().filter(|command| !command.trim().is_empty() && command.chars().count() <= 100_000) else {
                    bail!("Provide a nonempty command, at most 100,000 characters.");
                };
                let background = flag(args, "background")?;
                if !background {
                    let timeout = bounded(args, "timeoutMs", 30_000, 1, COMMAND_TIMEOUT_MS)?;
                    let job = self.start(command, timeout);
                    let mut done = job.done.clone();
                    tokio::select! {
                        _ = done.wait_for(|done| *done) => {}
                        _ = cancel.cancelled() => job.cancel.cancel(),
                    }
                    let state = job.state.lock();
                    return Ok(text(json!({ "output": state.output.trim_end(), "exitCode": state.exit_code, "truncated": state.truncated, "state": state.state })));
                }
                let mut jobs = self.jobs.lock();
                if jobs.iter().filter(|job| job.state.lock().running).count() >= MAX_BACKGROUND_JOBS {
                    bail!("At most {MAX_BACKGROUND_JOBS} background jobs can run at once. Wait for or stop one with command_status.");
                }
                let timeout = bounded(args, "timeoutMs", COMMAND_TIMEOUT_MS, 1, COMMAND_TIMEOUT_MS)?;
                jobs.push(self.start(command, timeout));
                Ok(text(json!({ "jobId": format!("job-{}", jobs.len()), "state": "running" })))
            }
            "command_status" if self.computer => {
                let index = args["jobId"].as_str().and_then(|id| id.strip_prefix("job-")).and_then(|index| index.parse::<usize>().ok()).filter(|index| *index >= 1);
                let (job_cancel, state, mut done) = {
                    let jobs = self.jobs.lock();
                    let job = index.and_then(|index| jobs.get(index - 1)).ok_or_else(|| anyhow!("Unknown background job. Use a jobId returned by run_command."))?;
                    (job.cancel.clone(), job.state.clone(), job.done.clone())
                };
                let wait = bounded(args, "waitMs", 0, 0, 120_000)?;
                if flag(args, "stop")? {
                    job_cancel.cancel();
                    let _ = tokio::time::timeout(Duration::from_secs(10), done.wait_for(|done| *done)).await;
                } else if wait > 0 {
                    tokio::select! {
                        _ = done.wait_for(|done| *done) => {}
                        _ = tokio::time::sleep(Duration::from_millis(wait)) => {}
                        _ = cancel.cancelled() => bail!("The run was stopped."),
                    }
                }
                let state = state.lock();
                Ok(text(json!({ "jobId": args["jobId"], "state": state.state, "exitCode": state.exit_code, "output": state.output.trim_end(), "truncated": state.truncated })))
            }
            _ => bail!("Unknown workspace tool: {name}"),
        }
    }

    fn start(&self, command: &str, timeout_ms: u64) -> Job {
        let cancel = self.cancel.child_token();
        let state = Arc::new(Mutex::new(JobState { running: true, state: "running", ..JobState::default() }));
        let (finished, done) = watch::channel(false);
        let args = vec!["-NoProfile".to_owned(), "-NonInteractive".to_owned(), "-Command".to_owned(), command.to_owned()];
        let mut process = super::runtime::command(Path::new("powershell"), &args, Some(&self.root), &[], &[]);
        process.stdin(std::process::Stdio::null());
        {
            let cancel = cancel.clone();
            let state = state.clone();
            tokio::spawn(async move {
                let exit = process::run(process, &cancel, Duration::from_millis(timeout_ms), |_| {}, |line| {
                    let mut state = state.lock();
                    let JobState { output, truncated, .. } = &mut *state;
                    tail(output, line, truncated);
                })
                .await;
                let mut state = state.lock();
                if let Some(error) = &exit.spawn_error {
                    state.output = format!("The command could not start: {error}");
                }
                let stderr = trace::truncate(exit.stderr.trim(), OUTPUT_CHARS / 4);
                if !stderr.is_empty() {
                    state.output.push_str(&stderr);
                }
                state.running = false;
                state.exit_code = exit.code;
                state.state = if exit.timed_out { "timed out" } else if exit.cancelled { "stopped" } else { "exited" };
                drop(state);
                finished.send_replace(true);
            });
        }
        Job { cancel, state, done }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn project_paths_stay_in_the_workspace() {
        let directory = tempfile::tempdir().unwrap();
        let tools = Tools::new(directory.path(), false).unwrap();
        let cancel = CancellationToken::new();
        tools.call("write_file", &json!({ "path": "a/b.txt", "text": "hello" }), &cancel).await.unwrap();
        let read = tools.call("read_file", &json!({ "path": "a/b.txt" }), &cancel).await.unwrap();
        assert!(matches!(&read[0], Content::Text(text) if text.contains("hello")));
        assert!(tools.call("read_file", &json!({ "path": "../outside.txt" }), &cancel).await.unwrap_err().to_string().contains("outside"));
        assert!(tools.call("run_command", &json!({ "command": "echo hi" }), &cancel).await.is_err(), "Project runs have no unsandboxed shell");
        assert!(!tools.has("run_command"));
        assert!(check_result(&json!({ "summary": "s", "commands": [], "artifacts": [], "extensions": [], "notes": [], "externalActions": [] })).is_ok());
        assert!(check_result(&json!({ "summary": "s" })).is_err());
    }
}
