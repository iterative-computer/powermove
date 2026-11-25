//! In-process Aero calls to the same tool bodies used by MCP.
use super::*;
use crate::extensions::{self, Stage};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    path::{Component, Path},
    sync::atomic::{AtomicBool, Ordering},
};

/// The bound project was closed, or reopened as another document.
const GONE: &str = "The project this Aero session works on was closed or replaced, so nothing more was changed. Ask the user to reopen it and start a new turn.";
/// The person pressed Stop: nothing new starts.
const HALTED: &str = "The turn was stopped before this tool changed anything.";

#[path = "aero_tools/files.rs"]
mod files;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Input<'a> {
    pub op: &'a str,
    pub revision: Option<u64>,
    #[serde(default)]
    pub args: Value,
}

pub struct Tools {
    session: McpSession,
    pub(crate) check_edits: bool,
    project: String,
    access: pm_agents::Access,
    origin: pm_kernel::Snapshot,
    own_revision: u64,
    undo_safe: bool,
    changed_extensions: BTreeMap<String, String>,
    plan_path: PathBuf,
    plan: Vec<PlanItem>,
    stage_path: PathBuf,
    halt: Arc<AtomicBool>,
    /// Runs between an edit's write and its check (tests: a competing
    /// writer, a project closed mid-edit).
    #[cfg(test)]
    pub(crate) after_write: Option<Box<dyn FnMut(&Arc<Mutex<Automation>>) + Send>>,
}

/// The stage's baseline, kept beside it so a later turn resumes the same
/// tree instead of a fresh copy that would drop its drafts.
#[derive(Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
struct SavedStage {
    project: String,
    baseline: BTreeMap<String, String>,
}

#[derive(Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
struct PlanItem {
    id: String,
    behavior: String,
    check: Check,
    #[serde(default)]
    done: bool,
}

#[derive(Deserialize, serde::Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum Check {
    Project,
    State { pointer: String, equals: Value },
}

impl Tools {
    pub fn open(
        host: Arc<Mutex<Automation>>,
        project: &str,
        expected: Option<u64>,
        scope: &Path,
        access: pm_agents::Access,
        token: Option<&str>,
    ) -> Result<Self> {
        let mut session = McpSession::new(host);
        session.aero_project = Some(project.into());
        session.bypass_hold = true;
        if let Some(token) = token {
            session.set_agent_run(token);
        }
        session.start(&json!({"prompt":"Aero project tools"}))?;
        let active = session.active.as_mut().unwrap();
        let guard = Automation::fenced(&session.host, active.document)?;
        assert_project(active, &guard, project)?;
        let origin = guard.original_view(project)?.snapshot;
        let revision = origin.revision;
        if expected.is_some_and(|expected| expected != revision) {
            return Err(Error::conflict(format!(
                "The request names revision {}, but the project is at {revision}. Read the current revision and retry.",
                expected.unwrap()
            )));
        }
        let stage_path = scope.join("stage.json");
        if access != pm_agents::Access::Editor
            && let Some(host) = guard.extensions.as_deref()
        {
            let staging = scope.join("extensions");
            let saved = match fs::read(&stage_path) {
                Ok(bytes) => Some(serde_json::from_slice::<SavedStage>(&bytes).map_err(|e| {
                    Error::invalid(format!("Read Aero's saved extension stage: {e}"))
                })?),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
                Err(e) => return Err(e.into()),
            };
            let stage = match saved {
                Some(saved) if saved.project == project && staging.is_dir() => {
                    extensions::resume(host, &staging, project, &active.run, saved.baseline)?
                }
                _ => extensions::prepare(host, &staging, project, &active.run)?,
            };
            save_stage(&stage_path, &stage)?;
            active.stage = Some(stage);
        }
        let plan_path = scope.join("plan.json");
        let plan = match fs::read(&plan_path) {
            Ok(bytes) => serde_json::from_slice(&bytes)
                .map_err(|e| Error::invalid(format!("Read Aero's saved plan: {e}")))?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(e) => return Err(e.into()),
        };
        drop(guard);
        Ok(Self {
            session,
            check_edits: true,
            project: project.into(),
            access,
            origin,
            own_revision: revision,
            undo_safe: true,
            changed_extensions: BTreeMap::new(),
            plan_path,
            plan,
            stage_path,
            halt: Arc::new(AtomicBool::new(false)),
            #[cfg(test)]
            after_write: None,
        })
    }

    /// The flag the harness sets when the person presses Stop.
    pub fn halt_flag(&mut self, flag: Arc<AtomicBool>) {
        self.halt = flag;
    }

    fn halted(&self) -> Result<()> {
        if self.halt.load(Ordering::Acquire) {
            return Err(Error::invalid(HALTED));
        }
        Ok(())
    }

    /// The session's document is still the open one: a closed or reopened
    /// project ends the run's changes with a terminal conflict.
    fn bound(&self) -> Result<()> {
        let document = self.active()?.document;
        let host = Automation::fenced(&self.session.host, None)?;
        match host.view(&self.project) {
            Ok(view) if Some(view.document) == document => Ok(()),
            _ => Err(Error::conflict(GONE)),
        }
    }

    fn active(&self) -> Result<&Active> {
        self.session
            .active
            .as_ref()
            .ok_or_else(|| Error::conflict(GONE))
    }

    fn active_mut(&mut self) -> Result<&mut Active> {
        self.session
            .active
            .as_mut()
            .ok_or_else(|| Error::conflict(GONE))
    }

    pub fn context(&mut self) -> Result<Value> {
        let mut state = self.session.dispatch(
            "get_project_state",
            &json!({
                "indexOnly":true,"layerLimit":20
            }),
        )?;
        Ok(json!({
            "composition": state["composition"].take(),
            "selection": state["selection"].take(),
            "layers": state["layers"].take(),
            "layerCount": state["layerCount"].take(),
        }))
    }

    pub fn call(
        &mut self,
        tool: &str,
        input: &str,
        approve: &mut dyn FnMut(&str) -> bool,
    ) -> Result<Value> {
        let mut input: Input<'_> = serde_json::from_str(input).map_err(|e| {
            Error::invalid(format!(
                "{tool} input is invalid: {e}. Send op, optional revision, and an args object."
            ))
        })?;
        if input.args.is_null() {
            input.args = json!({});
        }
        if !input.args.is_object() {
            return Err(Error::invalid(
                "Pass tool-specific fields in the args object.",
            ));
        }
        use pm_agents::aero::Permission;
        match pm_agents::aero::permission(self.access, tool, input.op) {
            Permission::Allow => {}
            Permission::Approve if approve(&format!("Allow Aero to {tool} {}?", input.op)) => {}
            Permission::Approve => {
                return Err(Error::invalid(
                    "This change needs approval. Ask the user to approve it in the Agent panel.",
                ));
            }
            Permission::Deny => {
                return Err(Error::invalid(
                    "This tool is outside this run's Access. Ask the user to change Access and start a new turn.",
                ));
            }
        }
        match tool {
            "project" => self.project(input.op, &input.args),
            "edit" => self.edit(input),
            "look" => self.look(input.op, &input.args),
            "files" => files::call(self.stage()?, input.op, &input.args),
            "extension" => self.extension(input.op, &input.args),
            "panel" => self.panel(input.op, &input.args),
            "undo" => self.undo(input.op, &input.args),
            "docs" => docs(input.op, &input.args),
            "plan" => self.plan(input.op, &input.args),
            _ => Err(Error::invalid("Unknown Aero tool. Read docs with op list.")),
        }
    }

    fn project(&mut self, op: &str, args: &Value) -> Result<Value> {
        let backing = match op {
            "summary" | "layer" | "markers" => "get_project_state",
            "timeline" => "get_timeline",
            "media" => "list_media",
            _ => {
                return Err(Error::invalid(
                    "Choose project op summary, timeline, layer, media or markers.",
                ));
            }
        };
        // A view reads the document in several steps while the editor may
        // write. Its data belongs to a revision only when the revision is
        // the same right before and right after it (revisions only grow).
        let revision_now = |tools: &Self| -> Result<u64> {
            let document = tools.active()?.document;
            Automation::fenced(&tools.session.host, document)?.revision(&tools.project)
        };
        let (mut result, revision) = 'read: {
            for _ in 0..8 {
                let before = revision_now(self)?;
                let result = self.session.dispatch(backing, args)?;
                if revision_now(self)? == before {
                    break 'read (result, before);
                }
            }
            return Err(Error::conflict(
                "The project kept changing while Aero read it. Read it again in a moment.",
            ));
        };
        if op == "media" && !result.is_object() {
            result = json!({"items":result});
        }
        if op == "markers" {
            result = json!({"markers":result["markers"].take()});
        }
        result["revision"] = json!(revision);
        Ok(result)
    }

    fn edit(&mut self, input: Input<'_>) -> Result<Value> {
        let backing = match input.op {
            "commands" => "apply_commands",
            "strips" => "edit_layer_strips",
            "video" => "edit_video",
            "3d" => "edit_3d",
            "selection" => "select_layers",
            _ => {
                return Err(Error::invalid(
                    "Choose edit op commands, strips, video, 3d or selection.",
                ));
            }
        };
        let expected = input.revision.ok_or_else(|| {
            Error::invalid(
                "Edit needs a revision. Read project state and use the revision you planned on.",
            )
        })?;
        self.halted()?;
        self.bound()?;
        let document = self.active()?.document;
        {
            let host = Automation::fenced(&self.session.host, document)?;
            let actual = host.revision(&self.project)?;
            if actual != expected {
                return Err(Error::conflict(format!(
                    "The project changed from revision {expected} to {actual}. Read its current state and retry."
                )));
            }
        }
        let before = if self.check_edits {
            self.project_check(input.args.get("compositionId"))?
        } else {
            Value::Null
        };
        let previous_focus = self.active()?.focus.clone();
        let label = input.args["label"]
            .as_str()
            .filter(|label| !label.trim().is_empty())
            .unwrap_or("Aero edit");
        let label = label.trim().chars().take(80).collect::<String>();
        let writer = self.session.writer;
        // The last point where Stop cancels the edit: past it, the mutation is
        // admitted and settles as one transaction before the turn ends.
        self.halted()?;
        let lease = if backing == "select_layers" {
            None
        } else {
            let mut host = Automation::fenced(&self.session.host, document)?;
            Some(host.begin_checked_edit(&self.project, writer, &label, expected)?)
        };

        self.session.aero_revision = Some(expected);
        let edit = self.session.dispatch(backing, &input.args);
        self.session.aero_revision = None;
        #[cfg(test)]
        if let Some(hook) = self.after_write.as_mut() {
            hook(&self.session.host);
        }
        let result = edit.and_then(|mut result| {
            if result["results"]
                .as_array()
                .is_some_and(|items| items.iter().any(|item| item["ok"] == false))
            {
                return Err(Error::invalid(format!(
                    "The edit could not apply every command. Read the tool errors and retry: {}.",
                    result["results"]
                )));
            }
            let after = if self.check_edits {
                self.project_check(input.args.get("compositionId"))?
            } else {
                Value::Null
            };
            let issues = after["issues"].as_array().into_iter().flatten();
            let introduced: Vec<_> = issues
                .clone()
                .filter(|issue| {
                    issue["severity"] == "error"
                        && !before["issues"]
                            .as_array()
                            .is_some_and(|old| old.contains(issue))
                })
                .collect();
            if !introduced.is_empty() {
                return Err(Error::invalid(format!(
                    "Project check failed. Fix these issues before retrying: {}",
                    json!(introduced)
                )));
            }
            result["warnings"] = json!(
                issues
                    .filter(|issue| issue["severity"] != "error")
                    .collect::<Vec<_>>()
            );
            result["checkFrames"] = json!([self.active()?.time]);
            Ok(result)
        });

        // Settle against the document the edit began on, even when the
        // session has meanwhile let go of a closed or replaced project.
        let gone = self.session.active.is_none();
        let mut host = Automation::fenced(&self.session.host, document)?;
        let checked_revision = result
            .as_ref()
            .ok()
            .and_then(|value| value["revision"].as_u64())
            .unwrap_or(expected);
        let current = host.revision(&self.project);
        let commit = !gone && result.is_ok() && current.as_ref().ok() == Some(&checked_revision);
        let settled = match lease {
            Some(lease) => host.finish_checked_edit(&self.project, writer, lease, commit),
            None => current.map(|revision| (revision, None)),
        };
        drop(host);
        let (revision, notice) = match settled {
            Ok(settled) => settled,
            Err(_) if gone || self.session.active.is_none() => {
                return Err(Error::conflict(GONE));
            }
            Err(error) => return Err(error),
        };
        // What an abort could not take back stays applied: say so.
        let kept = notice.map(|reason| {
            format!(
                " Part of this edit stayed applied because another editor changed the same values meanwhile ({reason}); the project is at revision {revision}. Read it before continuing."
            )
        });
        let Some(active) = self.session.active.as_mut() else {
            return Err(Error::conflict(format!(
                "{GONE}{}",
                kept.unwrap_or_default()
            )));
        };
        active.transaction = None;
        active.baseline = revision;
        // A confirmed clean abort leaves the project as this run left it.
        let clean_abort = !commit && kept.is_none() && revision == expected;
        self.undo_safe &= expected == self.own_revision && (commit || clean_abort);
        self.own_revision = revision;
        if !commit {
            active.focus = previous_focus;
        }
        match result {
            Ok(mut result) if commit => {
                active.committed |= revision != expected;
                result["revision"] = json!(revision);
                Ok(result)
            }
            Ok(_) => Err(Error::conflict(format!(
                "The project changed during verification. Read its current state before retrying.{}",
                kept.unwrap_or_default()
            ))),
            Err(mut error) => {
                if let Some(kept) = kept {
                    error.message.push_str(&kept);
                }
                Err(error)
            }
        }
    }

    fn project_check(&mut self, composition: Option<&Value>) -> Result<Value> {
        self.session.dispatch(
            "check_project",
            &composition.map_or_else(|| json!({}), |id| json!({"compositionId":id})),
        )
    }

    fn look(&mut self, op: &str, args: &Value) -> Result<Value> {
        let backing = match op {
            "frames" => "render_frames",
            "sample" => "sample_media_frames",
            "contact_sheet" => "media_contact_sheet",
            "waveform" => "media_waveform",
            "panel" => "capture_panel",
            "effect_lab" => "render_effect_lab",
            _ => {
                return Err(Error::invalid(
                    "Choose look op frames, sample, contact_sheet, waveform, panel or effect_lab.",
                ));
            }
        };
        self.session.dispatch(backing, args)
    }

    fn stage(&self) -> Result<&Stage> {
        self.session
            .active
            .as_ref()
            .and_then(|session| session.stage.as_ref())
            .ok_or_else(|| Error::unavailable("session extension staging"))
    }

    fn extension(&mut self, op: &str, args: &Value) -> Result<Value> {
        match op {
            "check" => {
                let stage = self.stage()?;
                let id = args["id"]
                    .as_str()
                    .ok_or_else(|| Error::invalid("Name the staged extension id."))?;
                let host = self
                    .session
                    .host
                    .lock()
                    .unwrap()
                    .extensions
                    .clone()
                    .ok_or_else(|| Error::unavailable("extension compiler"))?;
                let scratch = tempfile::tempdir_in(stage.staging.parent().unwrap())?;
                let manifest =
                    extensions::preview_source(host.as_ref(), stage, id, scratch.path())?;
                host.check(scratch.path(), &manifest).map_err(|message| Error::invalid(
                    format!("{id} failed its compile or activation check. Fix the reported source and run extension check again. {message}")
                ))?;
                Ok(json!({"id":id,"checked":true}))
            }
            "publish" | "remove" => {
                let declared = if op == "remove" {
                    json!({"extensions":[{"id":args["id"],"action":"removed"}]})
                } else {
                    args.clone()
                };
                self.halted()?;
                self.bound()?;
                let mut result = self.session.dispatch("publish_extensions", &declared)?;
                // Publication renewed the stage from the new installed trees.
                if let Ok(stage) = self.stage() {
                    save_stage(&self.stage_path, stage)?;
                }
                if result["changeSetId"].is_string() {
                    for extension in declared["extensions"].as_array().into_iter().flatten() {
                        if let Some(name) = extension["id"].as_str() {
                            self.changed_extensions.insert(
                                name.into(),
                                extension["action"].as_str().unwrap_or("updated").into(),
                            );
                        }
                    }
                    result["health"] = self.extension_health()?;
                    let document = self.active()?.document;
                    let host = Automation::fenced(&self.session.host, document)?;
                    let view = host.view(&self.project)?;
                    let engine = host.engine.clone();
                    drop(host);
                    let ids = declared["extensions"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter(|change| change["action"] != "removed")
                        .filter_map(|change| change["id"].as_str().map(str::to_owned))
                        .collect::<Vec<_>>();
                    if let Some(engine) = engine
                        && !ids.is_empty()
                    {
                        match engine.extension_probe(
                            &self.project,
                            view.document,
                            view.revision,
                            &ids,
                        ) {
                            Ok(probes) => result["probes"] = probes,
                            Err(error) => {
                                return Err(Error::invalid(format!(
                                    "The extension was published, but its probe frame failed: {error}. Inspect the registration and publish a fix."
                                )));
                            }
                        }
                    }
                }
                Ok(result)
            }
            "fork" => self.session.dispatch("fork_builtin_extension", args),
            "rebase" => self.session.dispatch("stage_fork_rebase", args),
            _ => Err(Error::invalid(
                "Choose extension op check, publish, remove, fork or rebase.",
            )),
        }
    }

    fn extension_health(&self) -> Result<Value> {
        if self.changed_extensions.is_empty() {
            return Ok(json!([]));
        }
        let document = self.active()?.document;
        let host = Automation::fenced(&self.session.host, document)?;
        let view = host.view(&self.project)?;
        let extensions = host
            .extensions
            .clone()
            .ok_or_else(|| Error::unavailable("extension loader"))?;
        let engine = host.engine.clone();
        drop(host);
        if let Some(engine) = engine {
            engine.prepare_definitions(&self.project, view.document, view.revision)?;
        }
        Ok(json!(extensions.health()?))
    }

    fn panel(&mut self, op: &str, args: &Value) -> Result<Value> {
        let backing = match op {
            "open" => "open_panel",
            "read" => "get_panel_state",
            "interact" => "interact_panel",
            "capture" => "capture_panel",
            "begin" => "panel_begin",
            "build" => "panel_build",
            _ => {
                return Err(Error::invalid(
                    "Choose panel op open, read, interact, capture, begin or build.",
                ));
            }
        };
        let mut result = self.session.dispatch(backing, args)?;
        if op == "build" && args["event"] == "ready" {
            result["capture"] = self
                .session
                .dispatch("capture_panel", &json!({"panelId":args["panelId"]}))?;
        }
        Ok(result)
    }

    fn undo(&mut self, op: &str, args: &Value) -> Result<Value> {
        if op == "extension" {
            let id = args["changeSetId"]
                .as_str()
                .ok_or_else(|| Error::invalid("Supply the extension changeSetId."))?;
            let host = self
                .session
                .host
                .lock()
                .unwrap()
                .extensions
                .clone()
                .ok_or_else(|| Error::unavailable("extension history"))?;
            let result = extensions::restore(host.as_ref(), &self.project, id)?;
            // Only the restored extensions follow the installed trees again;
            // the session's other drafts stay staged.
            let restored: Vec<String> = result["changes"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|change| change["id"].as_str().map(str::to_owned))
                .collect();
            let stage_path = self.stage_path.clone();
            let stage = self
                .active_mut()?
                .stage
                .as_mut()
                .ok_or_else(|| Error::unavailable("session extension staging"))?;
            extensions::refresh(stage, &restored)?;
            save_stage(&stage_path, stage)?;
            return Ok(result);
        }
        if op != "session" {
            return Err(Error::invalid("Choose undo op session or extension."));
        }
        self.bound()?;
        let document = self.active()?.document;
        let mut host = Automation::fenced(&self.session.host, document)?;
        if !self.undo_safe || host.revision(&self.project)? != self.own_revision {
            return Err(Error::conflict(
                "Another editor changed the project. Use the editor's Undo history to keep those edits.",
            ));
        }
        if self.origin.revision != self.own_revision {
            // The native snapshot, not its JSON projection: authored text
            // (lone surrogates included) comes back byte for byte.
            self.own_revision = host.restore_snapshot(
                &self.project,
                &self.origin,
                self.own_revision,
                self.session.writer,
            )?;
            drop(host);
            let revision = self.own_revision;
            self.active_mut()?.baseline = revision;
        }
        Ok(json!({"revision":self.own_revision,"undone":"session project edits"}))
    }

    fn check(&mut self, check: &Check) -> Result<bool> {
        match check {
            Check::Project => Ok(self.project_check(None)?["ok"] == true),
            Check::State { pointer, equals } => {
                let document = self.active()?.document;
                let host = Automation::fenced(&self.session.host, document)?;
                Ok(host.read_document(&self.project)?.pointer(pointer) == Some(equals))
            }
        }
    }

    fn plan(&mut self, op: &str, args: &Value) -> Result<Value> {
        match op {
            "list" => {}
            "add" => {
                let mut item: PlanItem = serde_json::from_value(args.clone()).map_err(|e| {
                    Error::invalid(format!("Plan items need id, behavior and check: {e}"))
                })?;
                if item.id.is_empty()
                    || item.behavior.is_empty()
                    || self.plan.iter().any(|old| old.id == item.id)
                    || self.plan.len() >= 32
                {
                    return Err(Error::invalid(
                        "Use a unique id and a behavior. A plan holds at most 32 items.",
                    ));
                }
                item.done = false;
                let mut next = std::mem::take(&mut self.plan);
                next.push(item);
                let saved = self.save_plan(&next);
                if saved.is_err() {
                    next.pop();
                }
                self.plan = next;
                saved?;
            }
            "verify" => {
                let index = self
                    .plan
                    .iter()
                    .position(|item| !item.done)
                    .ok_or_else(|| Error::invalid("No plan item is waiting for verification."))?;
                if args["id"] != self.plan[index].id {
                    return Err(Error::invalid(
                        "Verify the first unfinished plan item before moving to the next.",
                    ));
                }
                let mut next = std::mem::take(&mut self.plan);
                let saved = self.check(&next[index].check).and_then(|ok| {
                    if !ok {
                        return Err(Error::invalid(
                            "The current item failed its check. Fix the result and verify again.",
                        ));
                    }
                    next[index].done = true;
                    self.save_plan(&next)
                });
                if saved.is_err() {
                    next[index].done = false;
                }
                self.plan = next;
                saved?;
            }
            _ => return Err(Error::invalid("Choose plan op list, add or verify.")),
        }
        Ok(
            json!({"items":self.plan,"active":self.plan.iter().find(|item| !item.done).map(|item| &item.id)}),
        )
    }

    fn save_plan(&self, plan: &[PlanItem]) -> Result<()> {
        let parent = self.plan_path.parent().unwrap();
        let mut file = tempfile::NamedTempFile::new_in(parent)?;
        serde_json::to_writer(&mut file, plan).map_err(|e| Error::invalid(e.to_string()))?;
        file.as_file().sync_all()?;
        file.persist(&self.plan_path).map_err(|e| e.error)?;
        fs::File::open(parent)?.sync_all()?;
        Ok(())
    }

    pub fn failures(&mut self) -> Vec<String> {
        let mut failures = Vec::new();
        match self.project_check(None) {
            Ok(check) if check["ok"] == true => {}
            Ok(check) => failures.push(format!(
                "Project check: {}. Fix the reported issues and check again.",
                check["issues"]
            )),
            Err(error) => failures.push(format!("Project check failed: {error}")),
        }
        let plan = std::mem::take(&mut self.plan);
        for item in plan.iter().filter(|item| item.done) {
            if !matches!(self.check(&item.check), Ok(true)) {
                failures.push(format!(
                    "Plan item {} no longer passes. Restore its check: {}.",
                    item.id, item.behavior
                ));
            }
        }
        self.plan = plan;
        if let Ok(stage) = self.stage() {
            match files::dirty(stage) {
                Ok(ids) if ids.is_empty() => {}
                Ok(ids) => failures.push(format!(
                    "Unpublished staging changes in {}. Publish them or discard the drafts.",
                    ids.join(", ")
                )),
                Err(error) => failures.push(format!("Staging check failed: {error}")),
            }
        }
        if !self.changed_extensions.is_empty() {
            match self.extension_health() {
                Ok(health) => {
                    for (id, action) in &self.changed_extensions {
                        let row = health
                            .as_array()
                            .into_iter()
                            .flatten()
                            .find(|row| row["id"] == *id);
                        if action == "removed" && row.is_none() {
                            continue;
                        }
                        if !row
                            .and_then(|row| row["health"].as_object())
                            .is_some_and(|windows| {
                                !windows.is_empty()
                                    && windows
                                        .values()
                                        .all(|window| window == "ok" || window["state"] == "ok")
                            })
                        {
                            failures.push(format!("Could not confirm {id} loaded in every window. Inspect its activation errors and publish a fix."));
                        }
                    }
                }
                Err(error) => failures.push(format!("Extension health is unavailable: {error}")),
            }
        }
        failures
    }

    pub fn finish(&mut self) -> Result<()> {
        self.session.finish(true)?;
        Ok(())
    }
}

/// Keeps the stage's baseline beside it, owner-only, replaced atomically.
fn save_stage(path: &Path, stage: &Stage) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let parent = path.parent().unwrap_or(Path::new("."));
    let mut file = tempfile::NamedTempFile::new_in(parent)?;
    file.as_file()
        .set_permissions(fs::Permissions::from_mode(0o600))?;
    serde_json::to_writer(
        &mut file,
        &SavedStage {
            project: stage.project.clone(),
            baseline: stage.baseline.clone(),
        },
    )
    .map_err(|e| Error::invalid(e.to_string()))?;
    file.as_file().sync_all()?;
    file.persist(path).map_err(|e| e.error)?;
    fs::File::open(parent)?.sync_all()?;
    Ok(())
}

fn docs(op: &str, args: &Value) -> Result<Value> {
    let name = args["topic"].as_str().unwrap_or("");
    if op == "list" {
        return Ok(
            json!({"topics":TOPICS.iter().map(|(name, purpose, _)| json!({"name":name,"purpose":purpose})).collect::<Vec<_>>() }),
        );
    }
    if op != "read" {
        return Err(Error::invalid("Choose docs op list or read."));
    }
    TOPICS
        .iter()
        .find(|(topic, _, _)| *topic == name)
        .map(|(topic, _, body)| json!({"topic":topic,"body":body}))
        .ok_or_else(|| Error::invalid("Unknown API pack topic. Call docs list."))
}

const TOPICS: &[(&str, &str, &str)] = &[
    (
        "handoff",
        "Extension workflow, effect and transition authoring",
        include_str!("../../../js/compat/api-pack/HANDOFF.md"),
    ),
    (
        "extensions",
        "Extension package shape, effect and transition APIs",
        include_str!("../../../js/compat/api-pack/EXTENSIONS.md"),
    ),
    (
        "panels",
        "Native v4 controls and panels",
        include_str!("../../../js/v4/api-pack/EXTENSIONS.md"),
    ),
    (
        "project",
        "Project types including text, 3D and colour",
        include_str!("../../../js/compat/api-pack/project.ts"),
    ),
    (
        "commands",
        "Editable commands and parameter names",
        include_str!("../../../js/compat/api-pack/commands.ts"),
    ),
    (
        "api",
        "Compatibility API declarations",
        include_str!("../../../js/compat/api-pack/api.ts"),
    ),
    (
        "sample-panel",
        "Minimal native panel",
        include_str!("../../../js/v4/api-pack/samples/hello-panel/index.tsx"),
    ),
    (
        "sample-effect",
        "Minimal gradient effect",
        include_str!("../../../js/compat/api-pack/samples/gradient-tint/index.ts"),
    ),
];

impl pm_agents::aero::Session for Tools {
    fn context(&mut self) -> std::result::Result<Value, String> {
        Tools::context(self).map_err(|e| e.to_string())
    }
    fn halt(&mut self, flag: Arc<AtomicBool>) {
        self.halt_flag(flag);
    }

    fn call(
        &mut self,
        tool: &str,
        input: &str,
        approve: &mut dyn FnMut(&str) -> bool,
    ) -> std::result::Result<pm_agents::aero::ToolResult, String> {
        use base64::Engine;
        let value = Tools::call(self, tool, input, approve).map_err(|e| e.to_string())?;
        let mut result = pm_agents::aero::ToolResult::text(value.to_string());
        if let Some(parts) = value.as_array()
            && parts.iter().any(|part| part["type"] == "image")
        {
            let mut text = Vec::new();
            for part in parts {
                if let Some(data) = part["data"].as_str().filter(|_| part["type"] == "image") {
                    let mime = part["mimeType"].as_str().unwrap_or("image/png");
                    let bytes = base64::engine::general_purpose::STANDARD
                        .decode(data)
                        .map_err(|e| format!("The renderer returned an unreadable image: {e}"))?;
                    result.images.push((mime.into(), bytes));
                } else {
                    text.push(part.clone());
                }
            }
            result.text = json!(text).to_string();
        }
        Ok(result)
    }

    fn failures(&mut self) -> Vec<String> {
        Tools::failures(self)
    }
    fn finish(&mut self) -> std::result::Result<(), String> {
        Tools::finish(self).map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixture {
        profile: tempfile::TempDir,
        host: Arc<Mutex<Automation>>,
        project: String,
        revision: u64,
    }

    fn fixture() -> Fixture {
        let profile = tempfile::Builder::new()
            .prefix("pm-aero-unit-")
            .tempdir()
            .unwrap();
        let root = profile.path();
        let mut host = Automation::new(root.into(), root.join("exports")).unwrap();
        let project = host
            .create_project(&json!({"name":"Synthetic","duration":4}))
            .unwrap()["projectId"]
            .as_str()
            .unwrap()
            .to_owned();
        let host = Arc::new(Mutex::new(host));
        let mut seed = McpSession::new(host.clone());
        seed.dispatch("start_session", &json!({"prompt":"Seed"}))
            .unwrap();
        seed.dispatch(
            "apply_commands",
            &json!({"commands":[{"type":"add_layer","id":"keeper","name":"Keep",
                "layerType":"solid","from":0,"duration":4}]}),
        )
        .unwrap();
        seed.finish(true).unwrap();
        let revision = host.lock().unwrap().revision(&project).unwrap();
        Fixture {
            profile,
            host,
            project,
            revision,
        }
    }

    fn tools(fixture: &Fixture) -> Tools {
        let scope = fixture.profile.path().join("aero");
        fs::create_dir_all(&scope).unwrap();
        Tools::open(
            fixture.host.clone(),
            &fixture.project,
            None,
            &scope,
            pm_agents::Access::Project,
            None,
        )
        .unwrap()
    }

    fn rename(revision: u64, name: &str) -> String {
        json!({"op":"commands","revision":revision,"args":{"commands":[
            {"type":"set_layer","target":"keeper","patch":{"name":name}}
        ]}})
        .to_string()
    }

    /// P1-3: another editor writes the same value between Aero's write and
    /// its check, and its write fails the check. The abort cannot take Aero's
    /// work back, and the tool says so instead of reporting a clean failure.
    #[test]
    fn a_failed_check_with_a_competing_writer_reports_what_stayed_applied() {
        let fixture = fixture();
        let mut tools = tools(&fixture);
        tools.after_write = Some(Box::new(|host| {
            let mut human = McpSession::new(host.clone());
            human
                .dispatch("start_session", &json!({"prompt":"The editor"}))
                .unwrap();
            human
                .dispatch(
                    "apply_commands",
                    &json!({"commands":[
                        {"type":"set_layer","target":"keeper","patch":{"name":"Human"}},
                        {"type":"add_layer","id":"ghost","name":"Ghost","layerType":"video",
                            "from":0,"duration":2,"content":{"asset":"ghost"}}
                    ]}),
                )
                .unwrap();
            human.finish(true).unwrap();
        }));
        let error = tools
            .call("edit", &rename(fixture.revision, "Aero"), &mut |_| false)
            .unwrap_err();
        assert!(
            error.message.contains("stayed applied"),
            "the model was not told what stayed applied: {}",
            error.message
        );
    }

    /// P1-5: the editor tries to close the project while an edit's lease
    /// is open. The kernel refuses until the edit settles; the edit settles
    /// against its own document; once the project closes, Aero's next edit
    /// ends with a terminal conflict and nothing panics.
    #[test]
    fn closing_the_project_around_an_edit_settles_it_then_ends_with_a_conflict() {
        let fixture = fixture();
        let mut tools = tools(&fixture);
        let refused = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let seen = refused.clone();
        tools.after_write = Some(Box::new(move |host| {
            let closed = host.lock().unwrap().close_headless();
            seen.store(closed.is_err(), Ordering::SeqCst);
        }));
        let edited = tools
            .call("edit", &rename(fixture.revision, "Aero"), &mut |_| false)
            .expect("the edit settles");
        assert!(
            refused.load(Ordering::SeqCst),
            "the kernel closed under a lease"
        );
        tools.after_write = None;
        fixture.host.lock().unwrap().close_headless().unwrap();
        let revision = edited["revision"].as_u64().unwrap();
        let next = tools.call("edit", &rename(revision, "Again"), &mut |_| false);
        assert_eq!(next.unwrap_err().code, "revision_conflict");
    }
}
