//! In-process Aero calls to the same tool bodies used by MCP.
use super::*;
use crate::extensions::{self, Stage};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    path::{Component, Path},
};

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
        if access != pm_agents::Access::Editor
            && let Some(host) = guard.extensions.as_deref()
        {
            active.stage = Some(extensions::prepare(
                host,
                &scope.join("extensions"),
                project,
                &active.run,
            )?);
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
        })
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
        let mut result = match op {
            "summary" | "layer" | "markers" => self.session.dispatch("get_project_state", args)?,
            "timeline" => self.session.dispatch("get_timeline", args)?,
            "media" => self.session.dispatch("list_media", args)?,
            _ => {
                return Err(Error::invalid(
                    "Choose project op summary, timeline, layer, media or markers.",
                ));
            }
        };
        let revision = self.session.active.as_ref().unwrap().baseline;
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
        let document = self.session.active.as_ref().unwrap().document;
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
        let previous_focus = self.session.active.as_ref().unwrap().focus.clone();
        let label = input.args["label"]
            .as_str()
            .filter(|label| !label.trim().is_empty())
            .unwrap_or("Aero edit");
        let label = label.trim().chars().take(80).collect::<String>();
        let writer = self.session.writer;
        let lease = if backing == "select_layers" {
            None
        } else {
            let mut host = Automation::fenced(&self.session.host, document)?;
            Some(host.begin_checked_edit(&self.project, writer, &label, expected)?)
        };

        self.session.aero_revision = Some(expected);
        let edit = self.session.dispatch(backing, &input.args);
        self.session.aero_revision = None;
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
            result["checkFrames"] = json!([self.session.active.as_ref().unwrap().time]);
            Ok(result)
        });

        let active = self.session.active.as_mut().unwrap();
        let mut host = Automation::fenced(&self.session.host, document)?;
        let checked_revision = result
            .as_ref()
            .ok()
            .and_then(|value| value["revision"].as_u64())
            .unwrap_or(expected);
        let commit = result.is_ok() && checked_revision == host.revision(&self.project)?;
        let revision = match lease {
            Some(lease) => host.finish_checked_edit(&self.project, writer, lease, commit)?,
            None => host.revision(&self.project)?,
        };
        active.transaction = None;
        active.baseline = revision;
        self.undo_safe &= expected == self.own_revision && commit;
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
            Ok(_) => Err(Error::conflict(
                "The project changed during verification. Read its current state before retrying.",
            )),
            Err(error) => Err(error),
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
                let mut result = self.session.dispatch("publish_extensions", &declared)?;
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
                    let document = self.session.active.as_ref().unwrap().document;
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
        let document = self.session.active.as_ref().unwrap().document;
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
            let active = self.session.active.as_mut().unwrap();
            let stage = active
                .stage
                .as_ref()
                .ok_or_else(|| Error::unavailable("session extension staging"))?;
            active.stage = Some(extensions::prepare(
                host.as_ref(),
                &stage.staging,
                &self.project,
                &active.run,
            )?);
            return Ok(result);
        }
        if op != "session" {
            return Err(Error::invalid("Choose undo op session or extension."));
        }
        let active = self.session.active.as_mut().unwrap();
        let mut host = Automation::fenced(&self.session.host, active.document)?;
        if !self.undo_safe || host.revision(&self.project)? != self.own_revision {
            return Err(Error::conflict(
                "Another editor changed the project. Use the editor's Undo history to keep those edits.",
            ));
        }
        if self.origin.revision != self.own_revision {
            self.own_revision = host.restore(
                &self.project,
                crate::automation::document_json(&self.origin)?,
                self.own_revision,
                self.session.writer,
            )?;
            active.baseline = self.own_revision;
        }
        Ok(json!({"revision":self.own_revision,"undone":"session project edits"}))
    }

    fn check(&mut self, check: &Check) -> Result<bool> {
        match check {
            Check::Project => Ok(self.project_check(None)?["ok"] == true),
            Check::State { pointer, equals } => {
                let active = self.session.active.as_ref().unwrap();
                let host = Automation::fenced(&self.session.host, active.document)?;
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
