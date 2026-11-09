use crate::{Automation, Error, Result, catalog, commands, failure, json_value, success, views};
use pm_kernel::session::{ShellCall, ShellReply};
use pm_kernel::{ActorHistoryMark, Origin};
use serde_json::{Value, json};
use std::{
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU32, Ordering},
    },
};

/// Tools the running app answers on its main thread (`SurfaceHook`).
const PANEL_TOOLS: &[&str] = &[
    "open_panel",
    "get_panel_state",
    "interact_panel",
    "capture_panel",
    "computer_use_panel",
    "set_panel_layout",
    "get_workspace_state",
];
/// The shell's live docks and workspace replace the saved ones; panels the
/// window registered natively join the kernel's registrations.
fn live_layout(mut saved: Value, live: &Value) -> Value {
    if !live.is_object() {
        return saved;
    }
    for key in ["workspace", "docks"] {
        if !live[key].is_null() {
            saved[key] = live[key].clone();
        }
    }
    let mut panels = saved["registeredPanels"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    for panel in live["registeredPanels"].as_array().into_iter().flatten() {
        if !panels.iter().any(|p| p["id"] == panel["id"]) {
            panels.push(panel.clone());
        }
    }
    saved["registeredPanels"] = json!(panels);
    saved
}
/// The run (and its MCP access, "editor" or "project") a grant token names.
fn agent(token: Option<&str>, host: &Automation) -> Option<(String, String)> {
    (host.agent_grants.as_ref()?)(token?)
}
/// Main's orchestrator tools, answered by the window that runs the caller.
const ORCHESTRATION_TOOLS: &[&str] = &[
    "orchestrator_capabilities",
    "delegate_task",
    "task_status",
    "task_send",
    "task_cancel",
    "thread_watch",
    "thread_unwatch",
];
struct Transaction {
    snapshot: Value,
    mark: ActorHistoryMark,
    revision: u64,
    changed: bool,
    interleaved: bool,
    label: String,
}
struct Active {
    project: Option<String>,
    document: Option<u64>,
    access: String,
    context: String,
    baseline: u64,
    transaction: Option<Transaction>,
    workspace: Option<PathBuf>,
    /// The run's extension stage; renewed after each publication.
    stage: Option<crate::extensions::Stage>,
    run: String,
    publications: u32,
    committed: bool,
    external_actions: bool,
    /// Panel controls ran during this run (main's `panelActions`): their own
    /// editor Undo applies, so the run never snapshot-restores over them.
    panel_actions: bool,
}
/// One connection owns one session and workspace. No session credential is
/// exposed in MCP results. Drop/disconnect attempts guarded rollback.
pub struct McpSession {
    host: Arc<Mutex<Automation>>,
    active: Option<Active>,
    /// The grant token of the provider run that launched this connection
    /// (`--agent-run`), resolved through the host on every use: a token that
    /// doesn't resolve (forged, or its run ended) acts as no run.
    agent_token: Option<String>,
    writer: u32,
    closed: bool,
}
impl McpSession {
    pub fn new(host: Arc<Mutex<Automation>>) -> Self {
        static NEXT: AtomicU32 = AtomicU32::new(1000);
        Self {
            host,
            active: None,
            agent_token: None,
            writer: NEXT.fetch_add(1, Ordering::Relaxed),
            closed: false,
        }
    }
    pub fn tools(&self) -> Value {
        json!(catalog::tools())
    }
    pub fn set_agent_run(&mut self, token: &str) {
        self.agent_token = Some(token.to_owned());
    }
    pub fn call(&mut self, name: &str, args: Value) -> Value {
        if name == "project_memory" {
            return match self.project_memory(&args) {
                Ok(value) => crate::success_model(&value),
                Err(error) => failure(error),
            };
        }
        match self.dispatch(name, &args) {
            Ok(v)
                if [
                    "render_frames",
                    "probe_media",
                    "sample_media_frames",
                    "media_contact_sheet",
                    "media_waveform",
                    "capture_panel",
                    "computer_use_panel",
                ]
                .contains(&name) =>
            {
                json!({"content":v,"isError":false})
            }
            Ok(v) => success(v),
            Err(e) => failure(e),
        }
    }
    /// No schema admission, as on main: the session and memory switch refuse
    /// first, and a JavaScript slice's lone surrogate is kept (tests/wave4.rs,
    /// tests/main_memory.rs).
    fn project_memory(&mut self, args: &Value) -> Result<pm_model::Value> {
        if self.closed {
            return Err(Error::invalid("This MCP connection is closed."));
        }
        self.retire_replaced_document()?;
        let session = self
            .active
            .as_ref()
            .ok_or_else(|| Error::invalid("Call start_session before using Powermove tools."))?;
        if session.access == "editor" && !catalog::permits("inspection", "project_memory") {
            return Err(Error::invalid(
                "This planning run can only inspect the project.",
            ));
        }
        if session.context == "app" && !catalog::permits("app", "project_memory") {
            return Err(Error::invalid(
                "This agent has no project attached. Open a project to use composition tools.",
            ));
        }
        let id = session
            .project
            .as_deref()
            .ok_or_else(|| Error::invalid("This tool requires an attached project."))?;
        let mut host = Automation::fenced(&self.host, session.document)?;
        assert_project(session, &host, id)?;
        host.project_memory(id, args, self.writer)
    }
    pub fn dispatch(&mut self, name: &str, args: &Value) -> Result<Value> {
        if name == "project_memory" {
            return self
                .project_memory(args)
                .and_then(crate::memory_admission::unicode);
        }
        if self.closed {
            return Err(Error::invalid("This MCP connection is closed."));
        }
        self.retire_replaced_document()?;
        if name == "start_session" {
            return self.start(args);
        }
        if name == "finish_session" {
            crate::validation::session(name, args)?;
            if self.active.is_none() {
                return Err(Error::invalid("No MCP session is active."));
            }
            return self.finish(args["commit"] == true);
        }
        let shared = self.host.clone();
        let mut host = Automation::fenced(
            &shared,
            self.active.as_ref().and_then(|session| session.document),
        )?;
        if ["list_projects", "create_project", "open_project"].contains(&name) {
            if name != "list_projects" && self.active.is_some() {
                return Err(Error::invalid(
                    "Finish the current session before changing projects.",
                ));
            }
            catalog::validate(name, args)?;
            return match name {
                "list_projects" => host.list_projects(),
                "create_project" => host.create_project(args),
                _ => host.open_project(args["projectId"].as_str().unwrap()),
            };
        }
        let session = self
            .active
            .as_mut()
            .ok_or_else(|| Error::invalid("Call start_session before using Powermove tools."))?;
        // External-session gates run before the bridge and body parsers in the tag.
        if name == "publish_extensions" && session.access == "editor" {
            return Err(Error::invalid("Planning access cannot publish extensions."));
        }
        if name == "publish_extensions" {
            crate::validation::extensions(args)?;
            let extensions = host
                .extensions
                .clone()
                .ok_or_else(|| Error::unavailable(catalog::backing(name)))?;
            drop(host);
            return publish(session, extensions.as_ref(), args);
        }
        if name == "import_media_to_timeline"
            && (session.access == "editor" || session.context == "app")
        {
            return Err(Error::invalid(
                "This tool requires project access and an attached project.",
            ));
        }
        if catalog::spec(name).is_none() {
            return Err(Error::invalid(format!("Unknown Powermove tool: {name}")));
        }
        if session.access == "editor" && !catalog::permits("inspection", name) {
            return Err(Error::invalid(
                "This planning run can only inspect the project.",
            ));
        }
        if session.context == "app" && !catalog::permits("app", name) {
            return Err(Error::invalid(
                "This agent has no project attached. Open a project to use composition tools.",
            ));
        }
        if name == "manage_project" {
            catalog::validate(name, args)?;
            if let Some(id) = &session.project {
                assert_project(session, &host, id)?;
            }
            return self.manage(&mut host, args);
        }
        if name == "inspect_creative_workspace" {
            return host.creative.workspace(args);
        }
        if name == "inspect_creative_extension" {
            return host.creative.extension(args);
        }
        if name == "get_panel_layout" {
            if let Some(id) = &session.project {
                assert_project(session, &host, id)?;
            }
            let project = session.project.clone();
            let saved = host.panel_layout(project.as_deref())?;
            let Some(surfaces) = host.surfaces.clone() else {
                return Ok(saved);
            };
            drop(host);
            let live = surfaces.call(name, project.as_deref(), args)?;
            return Ok(live_layout(saved, &live));
        }
        if name == "validate_effect" {
            if let Some(id) = &session.project {
                assert_project(session, &host, id)?;
            }
            return crate::effect::validate(&args["definition"]);
        }
        if name == "fork_builtin_extension" || name == "stage_fork_rebase" {
            catalog::validate(name, args)?;
            if let Some(id) = &session.project {
                assert_project(session, &host, id)?;
            }
            let extensions = host
                .extensions
                .clone()
                .ok_or_else(|| Error::unavailable(catalog::backing(name)))?;
            let staging = session
                .stage
                .as_ref()
                .map(|stage| stage.staging.clone())
                .ok_or_else(|| Error::invalid("Extension staging requires project access"))?;
            drop(host);
            let id = args["id"].as_str().unwrap_or_default();
            return if name == "stage_fork_rebase" {
                crate::extensions::rebase(extensions.as_ref(), id, &staging)
            } else {
                crate::extensions::fork(extensions.as_ref(), id, args["forkId"].as_str(), &staging)
            };
        }
        if PANEL_TOOLS.contains(&name) {
            // As with the Store reads, a host without the capability says so
            // before arguments are parsed.
            if host.surfaces.is_none() {
                return Err(Error::unavailable(catalog::backing(name)));
            }
            catalog::validate(name, args)?;
            if let Some(id) = &session.project {
                assert_project(session, &host, id)?;
            }
            if ["interact_panel", "computer_use_panel"].contains(&name)
                && matches!(args["panelId"].as_str(), Some("agent" | "library"))
            {
                return Err(Error::invalid(
                    "The agent cannot operate its own conversation or permission controls.",
                ));
            }
            let surfaces = host
                .surfaces
                .clone()
                .ok_or_else(|| Error::unavailable(catalog::backing(name)))?;
            let project = session.project.clone();
            let acting = ["interact_panel", "computer_use_panel"].contains(&name);
            if acting && let Some(id) = project.as_deref() {
                let revision = host.revision(id)?;
                if session.transaction.is_none() {
                    session.transaction = Some(Transaction {
                        snapshot: host.read_document(id)?,
                        mark: host.history_mark(id, self.writer)?,
                        revision,
                        changed: false,
                        interleaved: false,
                        label: "Use panel".into(),
                    });
                }
                if session
                    .transaction
                    .as_ref()
                    .is_some_and(|tx| tx.revision != revision)
                {
                    return Err(Error::invalid(
                        "The project changed. Read get_project_state before using another panel control.",
                    ));
                }
                session.panel_actions = true;
            }
            let base = match name {
                "get_workspace_state" => Some(host.workspace_state(project.as_deref())?),
                "set_panel_layout" => Some(host.panel_layout(project.as_deref())?),
                _ => None,
            };
            drop(host);
            let live = surfaces.call(name, project.as_deref(), args);
            if acting && let Some(id) = project.as_deref() {
                // Panel handlers may edit asynchronously; later reads see it.
                let host = shared
                    .lock()
                    .map_err(|_| Error::unavailable("headless host stopped"))?;
                let revision = host.revision(id)?;
                if let Some(tx) = session.transaction.as_mut() {
                    tx.changed |= tx.snapshot["revision"].as_u64() != Some(revision);
                    tx.revision = revision;
                }
            }
            let live = live?;
            return Ok(match (name, base) {
                ("get_workspace_state", Some(mut state)) => {
                    let layout = live_layout(state["layout"].take(), &live["layout"]);
                    for (key, value) in live.as_object().into_iter().flatten() {
                        if key == "project" {
                            for (field, v) in value.as_object().into_iter().flatten() {
                                state["project"][field] = v.clone();
                            }
                        } else {
                            state[key] = value.clone();
                        }
                    }
                    state["layout"] = layout;
                    state
                }
                ("set_panel_layout", Some(saved)) => {
                    let mut result = live;
                    result["layout"] = live_layout(saved, &result["layout"]);
                    result
                }
                _ => live,
            });
        }
        if name == "publish_visual_reply" {
            // The reply joins the calling run's thread in the app.
            let surfaces = host
                .surfaces
                .clone()
                .ok_or_else(|| Error::unavailable(catalog::backing(name)))?;
            catalog::validate(name, args)?;
            let run = agent(self.agent_token.as_deref(), &host)
                .map(|(run, _)| run)
                .ok_or_else(|| {
                    Error::invalid("The visualization’s agent run is no longer active.")
                })?;
            let project = session.project.clone();
            drop(host);
            let mut call = args.clone();
            call["agentRunId"] = json!(run);
            return surfaces.call(name, project.as_deref(), &call);
        }
        if ORCHESTRATION_TOOLS.contains(&name) {
            // Tasks and watches belong to the window's Agent panel runs.
            let surfaces = host
                .surfaces
                .clone()
                .ok_or_else(|| Error::unavailable(catalog::backing(name)))?;
            catalog::validate(name, args)?;
            if let Some(id) = &session.project {
                assert_project(session, &host, id)?;
            }
            let run = agent(self.agent_token.as_deref(), &host)
                .map(|(run, _)| run)
                .ok_or_else(|| {
                    Error::invalid(
                        "Subagents and thread watches belong to a Powermove agent run. Ask in Powermove's Agent panel, or use thread_launch and thread_read.",
                    )
                })?;
            let project = session.project.clone();
            drop(host);
            let mut call = args.clone();
            call["agentRunId"] = json!(run);
            return surfaces.call(name, project.as_deref(), &call);
        }
        if ["thread_launch", "thread_send", "thread_cancel"].contains(&name) {
            // The window's Agent panel owns provider runs; only the app has one.
            let surfaces = host
                .surfaces
                .clone()
                .ok_or_else(|| Error::unavailable(catalog::backing(name)))?;
            catalog::validate(name, args)?;
            if let Some(id) = &session.project {
                assert_project(session, &host, id)?;
            }
            let project = session.project.clone();
            let caller = agent(self.agent_token.as_deref(), &host);
            drop(host);
            let mut call = args.clone();
            call["callerAccess"] = json!(session.access);
            call["callerRunId"] = json!(session.run);
            if let Some((run, _)) = caller {
                call["agentRunId"] = json!(run);
            }
            return surfaces.call(name, project.as_deref(), &call);
        }
        if ["thread_list", "thread_read"].contains(&name) {
            catalog::validate(name, args)?;
            if let Some(id) = &session.project {
                assert_project(session, &host, id)?;
            }
            let threads = host
                .threads
                .clone()
                .ok_or_else(|| Error::unavailable(catalog::backing(name)))?;
            let surfaces = host.surfaces.clone();
            let caller = agent(self.agent_token.as_deref(), &host);
            drop(host);
            let project = session.project.clone();
            let mut read = threads.read(name, project.as_deref(), args)?;
            // Reading a finished run acknowledges this run's watch on it.
            if name == "thread_read"
                && let (Some(surfaces), Some((run, _))) = (&surfaces, caller)
            {
                let mut call = args.clone();
                call["agentRunId"] = json!(run);
                let _ = surfaces.call("thread_acknowledge", project.as_deref(), &call);
            }
            // A live run's trace and open questions come from the window's panel.
            if name == "thread_read"
                && let Some(surfaces) = surfaces
                && let Ok(live) = surfaces.call("thread_live", project.as_deref(), args)
                && live.is_object()
            {
                read["activity"] = live["activity"].clone();
                read["questions"] = live["questions"].clone();
                if live["busy"] == true {
                    read["thread"]["busy"] = json!(true);
                    read["thread"]["status"] = json!("running");
                    read["thread"]["runId"] = live["runId"].clone();
                }
            }
            return Ok(read);
        }
        if ["store_search", "store_extension", "store_source"].contains(&name) {
            if let Some(id) = &session.project {
                assert_project(session, &host, id)?;
            }
            let store = host.store_reads.as_deref().ok_or_else(|| Error::unavailable(
                "native Store same-origin bounded HTTP transport (search/detail/release/tree/file)"
            ))?;
            catalog::validate(name, args)?;
            return match name {
                "store_search" => crate::store::search(store, args),
                "store_extension" => crate::store::detail(store, args),
                _ => crate::store::source(store, args),
            };
        }
        if !catalog::IMPLEMENTED.contains(&name) {
            return Err(Error::unavailable(catalog::backing(name)));
        }
        if name == "export_video" {
            let id = session
                .project
                .as_deref()
                .ok_or_else(|| Error::invalid("This tool requires an attached project."))?;
            assert_project(session, &host, id)?;
            let jobs = host.export_jobs.clone();
            let pending = jobs.prepare(
                host.freeze(id)?,
                args,
                host.export_child.clone(),
                host.exports.clone(),
            )?;
            drop(host);
            let outcome = pending.wait();
            let finished = !matches!(&outcome, Ok((_, false)));
            if finished && !jobs.consume(&pending.id)? {
                return Err(Error::invalid(
                    "Unknown export job. A finished job is reported once; check the export folder.",
                ));
            }
            let observed = self
                .host
                .lock()
                .map_err(|_| Error::unavailable("headless host stopped"))?;
            assert_project(session, &observed, id)?;
            return outcome.map(|(value, _)| value);
        }
        let prepared_selection = if name == "select_layers" {
            let project = host
                .active_project()
                .map(|id| host.read_document(&id))
                .transpose()?;
            let ids = args["layerIds"].as_array().filter(|ids| ids.iter().all(|id| {
                id.is_string() && project.as_ref().and_then(|p| p["layers"].as_array()).is_some_and(|layers| layers.iter().any(|layer| layer["id"] == *id))
            })).ok_or_else(|| Error::invalid("Select existing layer IDs from get_project_state. No selection was changed."))?;
            Some(
                ids.iter()
                    .map(|id| id.as_str().unwrap().to_owned())
                    .collect::<Vec<_>>(),
            )
        } else {
            None
        };
        let cleaned_commands = if name == "apply_commands" {
            Some(commands::clean_commands(
                args["commands"]
                    .as_array()
                    .map(Vec::as_slice)
                    .unwrap_or_default(),
            )?)
        } else {
            if name != "select_layers" {
                catalog::validate(name, args)?;
            }
            None
        };
        let session = self.active.as_mut().unwrap();
        let id = session
            .project
            .as_ref()
            .ok_or_else(|| Error::invalid("This tool requires an attached project."))?
            .clone();
        assert_project(session, &host, &id)?;
        if ["get_project_state", "get_timeline", "list_media"].contains(&name) {
            let result = if name == "get_project_state" {
                views::project_state(&host, &id, args)?
            } else if name == "list_media" {
                crate::media::list(
                    &host.read_document(&id)?,
                    &id,
                    args,
                    host.media_registry.as_deref(),
                )
            } else {
                views::timeline(&host, &id, args)?
            };
            refresh(session, &mut host, &id, self.writer)?;
            return Ok(result);
        }
        match name {
            "get_3d_scene" => {
                let result = crate::inspection::scene(&host, &id, args)?;
                refresh(session, &mut host, &id, self.writer)?;
                Ok(result)
            }
            "check_project" => crate::inspection::check(&host, &id),
            "render_frames" => host.render(&id, args)?.content(),
            "probe_media" | "sample_media_frames" | "media_contact_sheet" | "media_waveform" => {
                use crate::services::MediaOperation;
                let operation = match name {
                    "probe_media" => MediaOperation::Probe,
                    "sample_media_frames" => MediaOperation::SampleFrames,
                    "media_contact_sheet" => MediaOperation::ContactSheet,
                    _ => MediaOperation::Waveform,
                };
                host.media_tool(&id, operation, args)?.content()
            }

            "export_captions" => host.export_captions(&id, args),
            "import_media" | "replace_media" | "import_media_to_timeline" => {
                let importer = host
                    .media_importer
                    .clone()
                    .ok_or_else(|| Error::unavailable(catalog::backing(name)))?;
                let expected = session
                    .transaction
                    .as_ref()
                    .map_or(session.baseline, |t| t.revision);
                if host.revision(&id)? != expected {
                    return Err(Error::conflict(
                        "The project changed. Read get_project_state before retrying.",
                    ));
                }
                let frozen = host.freeze(&id)?;
                let replace = (name == "replace_media")
                    .then(|| args["assetId"].as_str().unwrap_or_default().to_owned());
                if let Some(asset) = replace.as_deref() {
                    if crate::media::has_locked_reference(&frozen.project, asset) {
                        return Err(Error::invalid(
                            "Unlock layers using this media before replacing it.",
                        ));
                    }
                    if args["missingOnly"] == true
                        && host
                            .media_registry
                            .as_ref()
                            .is_some_and(|registry| registry.state(&id, asset).available)
                    {
                        return Ok(json!({"status":"already_available","assetId":asset,
                            "persisted":frozen.project["assets"][asset]["persisted"] == true}));
                    }
                }
                let project = frozen.project.clone();
                let imported = importer.import(&crate::services::MediaImport {
                    frozen,
                    path: args["path"].as_str().unwrap_or_default().into(),
                    folder: args["folderId"].as_str().map(str::to_owned),
                    replace,
                    expected_fingerprint: args["expectedFingerprint"].as_str().map(str::to_owned),
                    writer: self.writer,
                })?;
                // Media Undo is separate from run grouping, like manage_media.
                session.external_actions = true;
                session.committed |= imported.revision != expected;
                session.baseline = imported.revision;
                if let Some(tx) = session.transaction.as_mut() {
                    tx.revision = imported.revision;
                }
                let mut result = json!({"status":imported.status,"assetId":imported.asset,
                    "name":imported.name,"persisted":true,"fingerprint":imported.fingerprint,
                    "storage":"copied","revision":imported.revision});
                if name == "import_media_to_timeline" {
                    let at = args["at"].as_f64().unwrap_or(0.);
                    match crate::media::layer_for(&project, &imported, at) {
                        Some((layer, command)) => {
                            drop(host);
                            let commands = json!({"commands":[command],"label":"Import Media"});
                            self.dispatch("apply_commands", &commands)?;
                            result["layerId"] = json!(layer);
                        }
                        None => {
                            result["note"] = json!("Imported. Place 3D models with edit_3d.");
                        }
                    }
                }
                Ok(result)
            }
            "manage_media" => {
                let expected = session
                    .transaction
                    .as_ref()
                    .map_or(session.baseline, |t| t.revision);
                if host.revision(&id)? != expected {
                    return Err(Error::conflict(
                        "The project changed. Read get_project_state before managing media.",
                    ));
                }
                let plan = crate::media::manage(&host.read_document(&id)?, args)?;
                let ack = host.apply_body(&id, plan.body, "Manage media", expected, self.writer)?;
                // Native media Undo is separate from run grouping and snapshot
                // rollback, exactly as the frozen contract promises.
                session.external_actions = true;
                session.committed |= ack.revision != expected;
                session.baseline = ack.revision;
                if let Some(tx) = session.transaction.as_mut() {
                    tx.revision = ack.revision;
                }
                let selected = host
                    .view(&id)?
                    .selection
                    .layers()
                    .iter()
                    .filter(|s| !plan.removed.contains(s))
                    .cloned()
                    .collect::<Vec<_>>();
                host.select_layers(&id, &selected, false, self.writer)?;
                let mut result = plan.result;
                result["changed"] = json!(ack.revision != expected);
                result["revision"] = json!(ack.revision);
                if let Some(notice) = ack.notice {
                    result["notice"] = json!(format!("{notice:?}"));
                }
                Ok(result)
            }
            "select_layers" => {
                let ids = prepared_selection.unwrap();
                Ok(
                    json!({"selection":host.select_layers(&id,&ids,args["add"]==true, self.writer)?}),
                )
            }
            "save_project" => host.save_project(&id),
            "apply_commands" | "edit_layer_strips" | "edit_video" | "edit_3d" => {
                let expected = session
                    .transaction
                    .as_ref()
                    .map_or(session.baseline, |t| t.revision);
                let revision = host.revision(&id)?;
                if expected != revision {
                    return Err(Error::conflict(format!(
                        "The project changed during the agent run (expected revision {expected}, found {revision}). No commands were applied. Read get_project_state, adjust the commands to the current state, and retry."
                    )));
                }
                let default_label = if name == "edit_3d" {
                    "Edit 3D Layers"
                } else {
                    "Agent composition edit"
                };
                let label = args["label"]
                    .as_str()
                    .filter(|s| !s.trim().is_empty())
                    .unwrap_or(default_label)
                    .trim()
                    .chars()
                    .take(80)
                    .collect::<String>();
                let p = host.read_document(&id)?;
                let selection = host.selection(&id)?["layers"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|v| v.as_str().unwrap().to_owned())
                    .collect::<Vec<_>>();
                let strips = if name == "edit_layer_strips" {
                    Some(crate::strips::plan(&p, args, host.playhead(&id))?)
                } else {
                    None
                };
                let video = if name == "edit_video" {
                    Some(crate::video::plan(
                        &p,
                        &id,
                        args,
                        host.media_registry.as_deref(),
                    )?)
                } else {
                    None
                };
                let batch = if name == "edit_3d" {
                    Some(crate::scene3d::batch(
                        &host.view(&id)?.snapshot,
                        args,
                        &selection,
                        host.playhead(&id),
                        host.scene3d_assets
                            .as_deref()
                            .unwrap_or(&pm_scene3d::geometry::NoAssets),
                    )?)
                } else if strips.is_none() && video.is_none() {
                    Some(commands::plan_cleaned_with_definitions(
                        &p,
                        &selection,
                        cleaned_commands.as_deref().unwrap(),
                        host.playhead(&id),
                        Some(&host.state()),
                    )?)
                } else {
                    None
                };
                if session.transaction.is_none() {
                    session.transaction = Some(Transaction {
                        snapshot: p.clone(),
                        mark: host.history_mark(&id, self.writer)?,
                        revision,
                        changed: false,
                        interleaved: false,
                        label: label.clone(),
                    });
                }
                let mut clip = Value::Null;
                let (ack, descriptions, results, strips_result) = if let Some(plan) = video {
                    clip = plan.clip;
                    let ack = host.apply_body(&id, plan.body, &label, expected, self.writer)?;
                    (ack, vec![], vec![], None)
                } else if let Some(plan) = strips {
                    let ack = host.apply_body(&id, plan.body, &label, expected, self.writer)?;
                    let result = if plan.state {
                        json!({"results":plan.results,"revision":ack.revision})
                    } else {
                        plan.result
                    };
                    (ack, vec![], plan.results, Some(result))
                } else {
                    let batch = batch.unwrap();
                    let ack = host.apply_commands(
                        &id,
                        batch.commands,
                        &label,
                        Origin::Mcp,
                        expected,
                        self.writer,
                    )?;
                    host.select_layers(&id, &batch.selection, false, self.writer)?;
                    (ack, batch.descriptions, batch.results, None)
                };
                let tx = session.transaction.as_mut().unwrap();
                tx.revision = ack.revision;
                tx.label = label.clone();
                tx.changed |= ack.revision != revision;
                // Folded foreign gestures retain their own normal undo entry.
                tx.interleaved |= ack.folded.is_some();
                let message = if name == "edit_video" && ack.revision == revision {
                    "No source changes"
                } else {
                    &label
                };
                let mut result = json!({"applied":descriptions,"clip":clip,"results":results,"message":message,"revision":ack.revision});
                if let Some(notice) = ack.notice {
                    result["notice"] = json!(format!("{notice:?}"));
                }
                if name == "edit_3d" {
                    let after = host.read_document(&id)?;
                    let target = crate::scene3d::result_id(&p, &after, args, &selection);
                    if let Some(ref target) = target {
                        host.select_layers(&id, std::slice::from_ref(target), false, self.writer)?;
                    }
                    result["result"] = json!({"id":target,"layerId":target});
                }
                if let Some(strips) = strips_result {
                    result["strips"] = strips;
                }
                Ok(result)
            }
            "rollback_changes" => {
                rollback(session, &mut host, &id, self.writer)?;
                Ok(json!({"rolledBack":true,"revision":host.revision(&id)?}))
            }
            _ => Err(Error::unavailable(catalog::backing(name))),
        }
    }
    /// Renderer/document replacement ends the run, not the external connection.
    /// Never restore an old snapshot into a replacement document. Preserve staged
    /// files even when archival fails; cleanup cannot replace the primary loss.
    fn retire_replaced_document(&mut self) -> Result<()> {
        let replaced = if let Some(session) = self.active.as_ref() {
            if let Some(id) = session.project.as_deref() {
                let host = self
                    .host
                    .lock()
                    .map_err(|_| Error::unavailable("headless host stopped"))?;
                host.active_project().as_deref() != Some(id)
                    || host.view(id).ok().map(|view| view.document) != session.document
            } else {
                false
            }
        } else {
            false
        };
        if replaced {
            let session = self.active.take().expect("live replaced session");
            if let Some(root) = session.workspace
                && let Err(error) = archive_workspace(&root, false)
            {
                eprintln!(
                    "MCP replaced-document workspace retained after archive failure: {error}"
                );
            }
        }
        Ok(())
    }
    fn start(&mut self, args: &Value) -> Result<Value> {
        if self.active.is_some() {
            return Err(Error::invalid("Finish the current MCP session first."));
        }
        crate::validation::session("start_session", args)?;
        let host = self
            .host
            .lock()
            .map_err(|_| Error::unavailable("headless host stopped"))?;
        // A provider run's session never exceeds that run's access.
        let granted = match self.agent_token.as_deref() {
            Some(token) => Some(
                agent(Some(token), &host)
                    .map(|(_, access)| access)
                    .ok_or_else(|| Error::invalid("This agent run is no longer active."))?,
            ),
            None => None,
        };
        let access = args["access"]
            .as_str()
            .or(granted.as_deref())
            .unwrap_or("project");
        if granted.as_deref() == Some("editor") && access != "editor" {
            return Err(Error::invalid(
                "This run has planning access: start the session with access \"editor\".",
            ));
        }
        let context = args["context"].as_str().unwrap_or("project");
        if context == "app" && access == "editor" {
            return Err(Error::invalid("App sessions require project access."));
        }
        let project = if context == "project" {
            Some(host.active_project().ok_or_else(|| {
                Error::invalid("Create or open a project before starting a project session.")
            })?)
        } else {
            None
        };
        let baseline = project
            .as_ref()
            .map(|id| host.revision(id))
            .transpose()?
            .unwrap_or(0);
        let document = project
            .as_ref()
            .map(|id| host.view(id).map(|view| view.document))
            .transpose()?;
        let run_id = format!("mcp-{}", pm_kernel::new_id("run"));
        let mut result = json!({"runId":run_id,"projectId":project.as_deref().unwrap_or("powermove-global"),"access":access});
        let workspace = if access == "editor" {
            None
        } else {
            let root = host.profile.join("mcp-runs").join(&run_id);
            for name in ["extensions", "api", "artifacts", "inputs"] {
                std::fs::create_dir_all(root.join(name))?;
            }
            let input = if let Some(ref id) = project {
                crate::memory::agent_snapshot_model(&host.read_document_model(id)?)
            } else {
                pm_model::Value::Object(pm_model::Map::new())
            };
            std::fs::write(
                root.join("inputs/powermove-project.json"),
                pm_model::json::to_canonical_string(&input),
            )?;
            std::fs::write(
                root.join("api/mcp-tools.json"),
                json!(catalog::tools()).to_string(),
            )?;
            result["workspace"] = json!(root);
            result["stagingDirectory"] = json!(root.join("extensions"));
            result["apiPackDirectory"] = json!(root.join("api"));
            result["artifactsDirectory"] = json!(root.join("artifacts"));
            let extensions = if host.extensions.is_some() {
                " To change the app itself, read api/EXTENSIONS.md, write extensions only in stagingDirectory (folder name = manifest id) and call publish_extensions: it validates, compiles and loads them into the running app."
            } else {
                " Extension staging is inert: nothing here activates."
            };
            result["instructions"] = json!(format!(
                "Read get_project_state before edits. Use apply_commands for typed, editable commands. finish_session commit:true keeps edits; false or disconnect attempts guarded rollback. api/mcp-tools.json lists the public contract. A tool this build lacks returns an error naming what is missing, so report that instead of retrying.{extensions}"
            ));
            Some(root)
        };
        let stage = match (&workspace, host.extensions.clone()) {
            (Some(root), Some(extensions)) => {
                for (name, text) in extensions.api_pack() {
                    let path = root.join("api").join(&name);
                    if name
                        .split('/')
                        .any(|part| part.is_empty() || part == "." || part == "..")
                    {
                        continue;
                    }
                    if let Some(parent) = path.parent() {
                        std::fs::create_dir_all(parent)?;
                    }
                    std::fs::write(path, text)?;
                }
                let project = project.as_deref().unwrap_or("powermove-global");
                Some(crate::extensions::prepare(
                    extensions.as_ref(),
                    &root.join("extensions"),
                    project,
                    &format!("{run_id}-1"),
                )?)
            }
            _ => None,
        };
        self.active = Some(Active {
            project,
            document,
            access: access.into(),
            context: context.into(),
            baseline,
            transaction: None,
            workspace,
            stage,
            run: run_id,
            publications: 1,
            committed: false,
            external_actions: false,
            panel_actions: false,
        });
        Ok(result)
    }
    pub fn finish(&mut self, commit: bool) -> Result<Value> {
        let Some(mut session) = self.active.take() else {
            return Ok(json!({"changed":false}));
        };
        let result = (|| {
            let mut host = Automation::fenced(&self.host, session.document)?;
            let Some(id) = session.project.clone() else {
                return Ok(json!({"changed":session.committed}));
            };
            assert_project(&session, &host, &id)?;
            let Some(tx) = session.transaction.as_ref() else {
                return Ok(json!({"changed":session.committed}));
            };
            if session.panel_actions {
                let changed =
                    tx.changed || tx.snapshot["revision"].as_u64() != Some(host.revision(&id)?);
                if !commit && changed {
                    return Err(Error::invalid(
                        "Panel actions were preserved after the run stopped. Use the editor Undo command to undo project changes.",
                    ));
                }
                return Ok(json!({"changed":session.committed || changed,"historyId":null}));
            }
            let changed = tx.changed;
            let mut history_id = None;
            if !commit && changed {
                rollback(&mut session, &mut host, &id, self.writer)?;
            } else if changed
                && !session.external_actions
                && !tx.interleaved
                && tx.revision == host.revision(&id)?
            {
                history_id = host.squash(&id, &tx.mark, &format!("Agent · {}", tx.label))?;
            }
            Ok(
                json!({"changed":session.committed||commit&&changed,"historyId":history_id.map(|id|id.to_string())}),
            )
        })();
        if let Some(root) = session.workspace {
            let cleanup = archive_workspace(&root, commit);
            if let Err(e) = cleanup
                && result.is_ok()
            {
                return Err(e);
            }
        }
        result
    }
    fn manage(&mut self, host: &mut Automation, a: &Value) -> Result<Value> {
        let operation = a["operation"].as_str().unwrap();
        let id = a["projectId"]
            .as_str()
            .map(str::to_owned)
            .or_else(|| host.active_project());
        let active = self.active.as_mut().unwrap();
        if ["create", "open"].contains(&operation) && active.context != "app" {
            return Err(Error::invalid(
                "Create or open projects from an app thread. This thread stays attached to its project.",
            ));
        }
        if operation == "list" {
            let projects = host.list_projects()?["projects"].clone();
            let trash = pm_kernel::LibraryStore::open(&host.profile)
                .read("projectTrash")
                .map_err(Error::kernel)?
                .map(|v| json_value(&v))
                .unwrap_or(json!([]));
            let rows = projects
                .as_array()
                .unwrap()
                .iter()
                .map(|p| json!({"id":p["id"],"name":p["name"],"w":p["w"],"h":p["h"],"at":p["at"]}))
                .collect::<Vec<_>>();
            let trash = trash
                .as_array()
                .unwrap_or(&vec![])
                .iter()
                .map(|p| json!({"id":p["id"],"name":p["name"]}))
                .collect::<Vec<_>>();
            return Ok(
                json!({"activeProjectId":host.active_project(),"projects":rows,"trash":trash}),
            );
        }
        let name = || {
            a["name"]
                .as_str()
                .filter(|s| !s.trim().is_empty())
                .map(str::trim)
                .ok_or_else(|| Error::invalid("name is required"))
        };
        // Library operations are immediate commit boundaries, outside rollback.
        if let (Some(project), Some(tx)) = (active.project.clone(), active.transaction.take()) {
            if tx.changed
                && !active.external_actions
                && !tx.interleaved
                && host.revision(&project)? == tx.revision
            {
                host.squash(&project, &tx.mark, &format!("Agent · {}", tx.label))?;
            }
            active.committed |= tx.changed;
        }
        if operation == "create" {
            let result=host.create_project(&json!({"name":name()?,"width":a["w"],"height":a["h"],"fps":a["fps"],"duration":a["dur"]}))?;
            active.committed = true;
            return Ok(
                json!({"projectId":result["projectId"],"name":result["name"],"active":true}),
            );
        }
        let id = id.ok_or_else(|| Error::invalid("projectId is required"))?;
        match operation {
            "open" => {
                host.open_project(&id)?;
            }
            "save" => host.save_library(&id)?,
            "rename" => {
                if host.active_project().as_deref() != Some(&id) {
                    return Err(Error::unavailable(
                        "rename unopened library slot through kernel shell",
                    ));
                }
                let revision = host.revision(&id)?;
                host.apply_commands(
                    &id,
                    vec![
                        pm_kernel::CompCommand::Set {
                            patch: Box::new(pm_kernel::commands::comps::CompPatch {
                                name: Some(name()?.into()),
                                ..Default::default()
                            }),
                        }
                        .into(),
                    ],
                    "Rename Project",
                    Origin::Mcp,
                    revision,
                    self.writer,
                )?;
                host.save_library(&id)?;
            }
            "duplicate" => {
                let new_id = pm_kernel::new_id("P");
                let n = name()?.to_owned();
                host.library_action(ShellCall::Duplicate {
                    project: id,
                    id: new_id.clone(),
                    name: n.clone(),
                })?;
                active.committed = true;
                return Ok(json!({"projectId":new_id,"name":n,"active":false}));
            }
            "trash" => match host.library_action(ShellCall::Trash(id.clone()))? {
                ShellReply::Changed(true) => {}
                other => return Err(Error::kernel(other)),
            },
            "restore" => match host.library_action(ShellCall::Restore(id.clone()))? {
                ShellReply::Changed(true) => {}
                other => return Err(Error::kernel(other)),
            },
            _ => return Err(Error::invalid("Unknown project operation")),
        }
        active.committed = true;
        if let Some(project) = &active.project {
            active.baseline = host.revision(project)?;
        }
        Ok(json!({"projectId":id,"operation":operation,"activeProjectId":host.active_project()}))
    }
    pub fn close(&mut self) -> Result<()> {
        let result = self.finish(false).map(|_| ());
        self.closed = true;
        result
    }
}
impl Drop for McpSession {
    fn drop(&mut self) {
        if !self.closed
            && let Err(e) = self.close()
        {
            eprintln!("MCP disconnect rollback: {e}");
        }
    }
}
fn assert_project(session: &Active, host: &Automation, id: &str) -> Result<()> {
    if host.active_project().as_deref() != Some(id)
        || host.view(id).ok().map(|view| view.document) != session.document
    {
        return Err(Error::conflict(
            "The active project changed or reopened. All work was preserved. Start a new agent run in this project.",
        ));
    }
    Ok(())
}
fn refresh(session: &mut Active, host: &mut Automation, id: &str, writer: u32) -> Result<()> {
    let revision = host.revision(id)?;
    session.baseline = revision;
    if let Some(tx) = &mut session.transaction
        && tx.revision != revision
    {
        if tx.changed {
            tx.interleaved = true;
        } else {
            tx.snapshot = host.read_document(id)?;
            tx.mark = host.history_mark(id, writer)?;
        }
        tx.revision = revision;
    }
    Ok(())
}
fn rollback(session: &mut Active, host: &mut Automation, id: &str, writer: u32) -> Result<()> {
    if session.external_actions {
        return Err(Error::conflict(
            "This run used media actions. Use the editor Undo command; media actions cannot be snapshot-rolled back.",
        ));
    }
    let Some(tx) = session.transaction.as_mut() else {
        return Ok(());
    };
    if tx.interleaved {
        return Err(Error::conflict(
            "Other edits occurred during this run. Use the editor Undo command to undo agent changes without restoring over that work.",
        ));
    }
    if session.panel_actions {
        return Err(Error::conflict(
            "Panel controls use their normal editor Undo. Their actions cannot be automatically rolled back; use the editor Undo command.",
        ));
    }
    if !tx.changed {
        return Ok(());
    }
    host.restore(id, tx.snapshot.clone(), tx.revision, writer)?;
    tx.snapshot = host.read_document(id)?;
    tx.revision = host.revision(id)?;
    tx.mark = host.history_mark(id, writer)?;
    tx.changed = false;
    session.baseline = tx.revision;
    Ok(())
}

/// Main's publish_extensions: validate and promote, reload the app, then
/// continue from the promoted baseline with a new run-private stage.
fn publish(
    session: &mut Active,
    extensions: &dyn crate::extensions::ExtensionHost,
    args: &Value,
) -> Result<Value> {
    let stage = session
        .stage
        .as_ref()
        .ok_or_else(|| Error::invalid("Extension staging requires project access"))?;
    let declared = args["extensions"].as_array().cloned().unwrap_or_default();
    let record = crate::extensions::publish(extensions, stage, &declared)?;
    session.publications += 1;
    let run = format!("{}-{}", session.run, session.publications);
    let next = crate::extensions::prepare(extensions, &stage.staging, &stage.project, &run)?;
    let staging = next.staging.clone();
    session.stage = Some(next);
    Ok(
        json!({"changeSetId":record.map(|r| r["id"].clone()),"extensions":declared,"stagingDirectory":staging}),
    )
}

/// N4: retiring an agent workspace must preserve its authored files. Moving
/// within the explicit profile is atomic and never follows staged symlinks.
fn archive_workspace(root: &std::path::Path, commit: bool) -> Result<()> {
    let profile = root
        .parent()
        .and_then(std::path::Path::parent)
        .ok_or_else(|| Error::invalid("Invalid run workspace"))?;
    let runs = profile.join("mcp-archives");
    std::fs::create_dir_all(&runs)?;
    let archive = runs.join(
        root.file_name()
            .ok_or_else(|| Error::invalid("Missing run identity"))?,
    );
    // Exclusive creation prevents overwriting an earlier archive on retry.
    std::fs::create_dir(&archive)?;
    if commit {
        std::fs::rename(root.join("extensions"), archive.join("extensions"))?;
    } else {
        std::fs::rename(root, archive.join("workspace"))?;
    }
    Ok(())
}
