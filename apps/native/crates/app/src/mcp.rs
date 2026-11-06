//! App-owned MCP connections. IO stays on workers; every write uses the app actor.
use pm_kernel::{session::*, *};
use pm_mcp::{Error, Result, catalog, commands, transport::ToolConnection};
use serde_json::{Value, json};
use std::{
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU32, Ordering},
    },
};

pub struct Host {
    pub kernel: KernelHandle,
    pub profile: PathBuf,
    current: Mutex<Option<SessionHandle>>,
    operations: Arc<AtomicBool>,
    pub closing: AtomicBool,
    changed: Mutex<Vec<async_channel::Sender<()>>>,
}
/// An owned, nonblocking fence for app session transitions. Never held by a UI gesture.
pub struct Operation(Arc<AtomicBool>);
impl Drop for Operation {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}
impl Host {
    pub fn new(kernel: KernelHandle, profile: PathBuf) -> (Arc<Self>, async_channel::Receiver<()>) {
        let (changed, rx) = async_channel::bounded(1);
        (
            Arc::new(Self {
                kernel,
                profile,
                current: Mutex::new(None),
                operations: Arc::new(AtomicBool::new(false)),
                closing: AtomicBool::new(false),
                changed: Mutex::new(vec![changed]),
            }),
            rx,
        )
    }
    pub fn try_operation(&self) -> Option<Operation> {
        self.operations
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .ok()
            .map(|_| Operation(self.operations.clone()))
    }
    pub fn current(&self) -> Option<SessionHandle> {
        self.current.lock().unwrap().clone()
    }
    pub fn bind(&self, session: Option<SessionHandle>) {
        {
            let mut current = self.current.lock().unwrap();
            if *current == session {
                return;
            }
            *current = session;
        }
        self.changed
            .lock()
            .unwrap()
            .retain(|tx| !matches!(tx.try_send(()), Err(async_channel::TrySendError::Closed(_))));
    }
    pub fn subscribe(&self) -> async_channel::Receiver<()> {
        let (tx, rx) = async_channel::bounded(1);
        let _ = tx.try_send(());
        self.changed.lock().unwrap().push(tx);
        rx
    }
    fn shell(&self, call: ShellCall) -> Result<ShellReply> {
        self.kernel
            .shell(call)
            .wait()
            .ok_or_else(stopped)?
            .map_err(Error::kernel)
    }
    pub fn open(&self, id: String, source: SessionSource) -> Result<Value> {
        let reply = self.shell(ShellCall::PrepareOpen {
            id,
            source,
            electron_profile: None,
        })?;
        // Windowless clients never choose a recovery policy on the user's behalf.
        let ShellReply::Opened {
            session,
            decision: pm_kernel::locks::OpenDecision::Writable,
            ..
        } = reply
        else {
            return Err(Error::kernel(reply));
        };
        if let Some(old) = self.current().filter(|s| {
            s != &session
                && self
                    .kernel
                    .published(&s.project)
                    .is_some_and(|p| p.session.as_ref().map(|m| &m.handle) == Some(s))
        }) {
            match self.shell(ShellCall::Close(old)) {
                Ok(ShellReply::Closed {
                    recovery_copy: None,
                }) => {}
                other => {
                    // Do not strand the newly opened project if replacing the old one fails.
                    let _ = self.shell(ShellCall::DeferClose(session));
                    return Err(Error::kernel(other));
                }
            }
        }
        self.bind(Some(session.clone()));
        let p = self
            .kernel
            .published(&session.project)
            .ok_or_else(stopped)?;
        Ok(identity(&p))
    }
    fn export(&self, session: SessionHandle, document: u64) -> Result<Value> {
        let exports = self.profile.join("exports");
        std::fs::create_dir_all(&exports)?;
        let path = exports.join(format!("{}.pmv", pm_kernel::new_id("export")));
        let plan = self
            .kernel
            .prepare_export(session.clone(), document, path.clone())
            .wait()
            .ok_or_else(stopped)?
            .map_err(Error::kernel)?;
        let receipt = plan.run().map_err(Error::kernel)?;
        if !receipt.unavailable_links.is_empty() {
            return Err(Error::unavailable(
                "original media unavailable for complete portable project export",
            ));
        }
        Ok(json!({"projectId":session.project,"path":path}))
    }
    fn save_project(&self, session: SessionHandle, document: u64) -> Result<Value> {
        let published = self
            .kernel
            .published(&session.project)
            .ok_or_else(stopped)?;
        if published.snapshot.doc_id != document {
            return Err(Error::invalid("The document was closed or replaced."));
        }
        if published.path.is_none() {
            return self.export(session, document);
        }
        let save = self
            .kernel
            .prepare_document_save(session.clone(), document)
            .wait()
            .ok_or_else(stopped)?
            .map_err(Error::kernel)?;
        let reply = self
            .kernel
            .finish_save(save.run())
            .wait()
            .ok_or_else(stopped)?
            .map_err(Error::kernel)?;
        match reply {
            ShellReply::Saved {
                session,
                recovery_copy: None,
            } => {
                let published = self
                    .kernel
                    .published(&session.project)
                    .ok_or_else(stopped)?;
                Ok(json!({"projectId":session.project,"path":published.path}))
            }
            other => Err(Error::kernel(other)),
        }
    }
    fn save(&self, session: SessionHandle, path: Option<PathBuf>) -> Result<Value> {
        let p = self
            .kernel
            .published(&session.project)
            .ok_or_else(stopped)?;
        let file = path.is_some() || p.path.is_some();
        let reply = if file {
            let save = self
                .kernel
                .prepare_save(session, path)
                .wait()
                .ok_or_else(stopped)?
                .map_err(Error::kernel)?;
            self.kernel
                .finish_save(save.run())
                .wait()
                .ok_or_else(stopped)?
                .map_err(Error::kernel)?
        } else {
            self.shell(ShellCall::Save(session))?
        };
        match reply {
            ShellReply::Saved {
                session,
                recovery_copy: None,
            } => {
                let p = self
                    .kernel
                    .published(&session.project)
                    .ok_or_else(stopped)?;
                Ok(json!({"projectId":session.project,"path":p.path}))
            }
            other => Err(Error::kernel(other)),
        }
    }
}
fn stopped() -> Error {
    Error::unavailable("app kernel stopped")
}
fn model(v: &Value) -> pm_model::Value {
    pm_model::json::parse(&v.to_string()).unwrap()
}
fn json_value(v: &pm_model::Value) -> Value {
    serde_json::from_str(&pm_model::json::to_canonical_string(v)).unwrap()
}
fn identity(p: &Published) -> Value {
    let name = p
        .snapshot
        .root()
        .and_then(|root| root.get("name"))
        .and_then(|node| match node.kind() {
            pm_model::NodeKind::String(text) => text.as_str(),
            _ => None,
        });
    json!({"projectId":p.project,"name":name})
}
fn document(p: &Published) -> Value {
    let mut v = json_value(&pm_model::canonical::realize_project(
        p.snapshot.document.to_value().unwrap(),
    ));
    v["revision"] = json!(p.revision());
    v
}
struct Edit {
    before: Value,
    mark: ActorHistoryMark,
    revision: u64,
    changed: bool,
    interleaved: bool,
    label: String,
}
struct Run {
    session: Option<SessionHandle>,
    document: Option<u64>,
    baseline: u64,
    editor: bool,
    committed: bool,
    workspace: Option<PathBuf>,
    edit: Option<Edit>,
}
pub struct Connection {
    host: Arc<Host>,
    actor: ActorHandle,
    run: Option<Run>,
}
impl Connection {
    pub fn new(host: Arc<Host>) -> Self {
        static NEXT: AtomicU32 = AtomicU32::new(100_000);
        let actor = host.kernel.for_actor(Caller::realm(
            NEXT.fetch_add(1, Ordering::Relaxed),
            1,
            Origin::Mcp,
        ));
        Self {
            host,
            actor,
            run: None,
        }
    }
    fn call(&self, id: &str, call: impl Into<Call>) -> Result<Ack> {
        let document = self
            .run
            .as_ref()
            .and_then(|run| run.document)
            .or_else(|| self.host.kernel.published(id).map(|p| p.snapshot.doc_id))
            .ok_or_else(|| Error::conflict("Project closed before the request was admitted"))?;
        let ack = self
            .actor
            .kernel
            .call(
                id,
                Request::new(self.actor.caller, call).with_document(document),
            )
            .wait()
            .ok_or_else(stopped)?
            .map_err(Error::kernel)?;
        if let Reply::Throw(e) = &ack.reply {
            return Err(Error::kernel(e));
        }
        Ok(ack)
    }
    fn published(&self) -> Result<Arc<Published>> {
        let run = self
            .run
            .as_ref()
            .ok_or_else(|| Error::invalid("Start an MCP session first."))?;
        let session = run
            .session
            .as_ref()
            .ok_or_else(|| Error::invalid("This tool requires an attached project."))?;
        let p = self
            .host
            .kernel
            .published(&session.project)
            .ok_or_else(|| Error::conflict("Project closed; start a new session."))?;
        if self.host.current().as_ref() != Some(session)
            || p.session.as_ref().map(|s| &s.handle) != Some(session)
            || Some(p.snapshot.doc_id) != run.document
        {
            return Err(Error::conflict(
                "Project changed or reopened; start a new session. All work was preserved.",
            ));
        }
        Ok(p)
    }
    fn mark(&self, id: &str) -> Result<ActorHistoryMark> {
        match self.call(id, HistoryCall::Mark)?.reply {
            Reply::Mark(mark) => Ok(mark),
            other => Err(Error::kernel(other)),
        }
    }
    fn start(&mut self, a: &Value) -> Result<Value> {
        if self.run.is_some() {
            return Err(Error::invalid("Finish the current MCP session first."));
        }
        let editor = a["access"] == "editor";
        let app = a["context"] == "app";
        if app && editor {
            return Err(Error::invalid("App sessions require project access."));
        }
        let session = if app {
            None
        } else {
            Some(
                self.host
                    .current()
                    .ok_or_else(|| Error::invalid("Create or open a project first."))?,
            )
        };
        let p = session
            .as_ref()
            .map(|s| self.host.kernel.published(&s.project).ok_or_else(stopped))
            .transpose()?;
        let run_id = pm_kernel::new_id("mcp-run");
        let mut result = json!({"runId":run_id,"projectId":session.as_ref().map(|s|s.project.as_str()).unwrap_or("powermove-global"),"access":if editor{"editor"}else{"project"}});
        let workspace = if editor {
            None
        } else {
            let root = self.host.profile.join("mcp-runs").join(&run_id);
            for dir in ["extensions", "api", "artifacts", "inputs"] {
                std::fs::create_dir_all(root.join(dir))?;
            }
            std::fs::write(
                root.join("inputs/powermove-project.json"),
                p.as_ref()
                    .map(|p| {
                        let mut value = pm_model::canonical::realize_project(
                            p.snapshot.document.to_value().unwrap(),
                        );
                        value
                            .as_object_mut()
                            .unwrap()
                            .insert("revision", pm_model::Value::Number(p.revision() as f64));
                        pm_model::json::to_canonical_string(&pm_mcp::memory::agent_snapshot_model(
                            &value,
                        ))
                    })
                    .unwrap_or_else(|| "{}".into()),
            )?;
            std::fs::write(
                root.join("api/mcp-tools.json"),
                json!(catalog::tools()).to_string(),
            )?;
            result["workspace"] = json!(root);
            result["stagingDirectory"] = json!(root.join("extensions"));
            result["apiPackDirectory"] = json!(root.join("api"));
            result["artifactsDirectory"] = json!(root.join("artifacts"));
            result["instructions"] = json!(
                "Read get_project_state before edits. finish_session commit:true keeps edits; false or disconnect attempts guarded rollback. Extension staging is inert; runtime, rendering and surface tools remain pending."
            );
            Some(root)
        };
        self.run = Some(Run {
            session,
            document: p.as_ref().map(|p| p.snapshot.doc_id),
            baseline: p.as_ref().map_or(0, |p| p.revision()),
            editor,
            committed: false,
            workspace,
            edit: None,
        });
        Ok(result)
    }
    fn rollback(&mut self) -> Result<()> {
        let p = self.published()?;
        let Some(edit) = self.run.as_ref().unwrap().edit.as_ref() else {
            return Ok(());
        };
        if !edit.changed {
            return Ok(());
        }
        if edit.interleaved || edit.revision != p.revision() {
            return Err(Error::conflict(
                "Other edits occurred. Use Undo; rollback would overwrite work.",
            ));
        }
        let ack = self.call(
            &p.project,
            Call::Restore {
                project: model(&edit.before),
                label: "Roll back agent changes".into(),
                expected_revision: edit.revision,
                expected_document: p.snapshot.doc_id,
            },
        )?;
        self.run.as_mut().unwrap().baseline = ack.revision;
        self.run.as_mut().unwrap().edit = None;
        Ok(())
    }
    fn finish(&mut self, commit: bool) -> Result<Value> {
        let result = (|| {
            let run = self
                .run
                .as_ref()
                .ok_or_else(|| Error::invalid("No MCP session is active."))?;
            let committed = run.committed;
            if run.session.is_none() {
                return Ok(json!({"changed":committed}));
            }
            let p = self.published()?;
            let Some(edit) = self.run.as_ref().unwrap().edit.as_ref() else {
                return Ok(json!({"changed":committed}));
            };
            let changed = edit.changed;
            let mut history = None;
            if !commit {
                self.rollback()?;
            } else if changed && !edit.interleaved && edit.revision == p.revision() {
                let ack = self.call(
                    &p.project,
                    HistoryCall::Squash {
                        mark: edit.mark.clone(),
                        label: format!("Agent · {}", edit.label),
                        expected_revision: edit.revision,
                    },
                )?;
                if let Reply::Entry(id) = ack.reply {
                    history = id;
                }
            }
            Ok(
                json!({"changed":committed||commit&&changed,"historyId":history.map(|id|id.to_string())}),
            )
        })();
        if let Some(run) = self.run.take()
            && let Some(root) = run.workspace
        {
            let cleanup = if commit {
                root.join("extensions")
            } else {
                root
            };
            if let Err(e) = std::fs::remove_dir_all(cleanup)
                && result.is_ok()
            {
                return Err(e.into());
            }
        }
        result
    }
    fn project_memory(&mut self, a: &Value) -> Result<pm_model::Value> {
        catalog::validate("project_memory", a)?;
        let _operation = self.host.try_operation().ok_or_else(|| {
            Error::conflict("A project operation is in progress. Inspect and retry.")
        })?;
        if self.host.closing.load(Ordering::Acquire) {
            return Err(Error::conflict(
                "The app is closing. Finish or cancel the close first.",
            ));
        }
        let run = self
            .run
            .as_ref()
            .ok_or_else(|| Error::invalid("Start an MCP session first."))?;
        if run.editor && !catalog::permits("inspection", "project_memory") {
            return Err(Error::invalid("Editor access is inspection-only."));
        }
        if run.session.is_none() {
            return Err(Error::invalid("This tool requires an attached project."));
        }
        let p = self.published()?;
        match pm_mcp::memory_admission::prepare(
            p.snapshot
                .root()
                .ok_or_else(|| Error::invalid("Missing document"))?,
            a,
        )? {
            pm_mcp::memory_admission::Prepared::Read(value) => Ok(value),
            pm_mcp::memory_admission::Prepared::Write(call) => {
                pm_mcp::memory_admission::result(self.call(&p.project, call)?)
            }
        }
    }
    pub fn dispatch(&mut self, name: &str, a: &Value) -> Result<Value> {
        if name == "project_memory" {
            return self
                .project_memory(a)
                .and_then(pm_mcp::memory_admission::unicode);
        }
        catalog::validate(name, a)?;
        // UI open/close and MCP requests fence session identity without blocking
        // the main thread. Ordinary UI edits/gestures never acquire this fence.
        let _operation = self.host.try_operation().ok_or_else(|| {
            Error::conflict("A project operation is in progress. Inspect and retry.")
        })?;
        if self.host.closing.load(Ordering::Acquire) {
            return Err(Error::conflict(
                "The app is closing. Finish or cancel the close first.",
            ));
        }
        if name == "start_session" {
            return self.start(a);
        }
        if name == "finish_session" {
            return self.finish(a["commit"] == true);
        }
        if name == "list_projects" {
            let ShellReply::Recent(rows) = self.host.shell(ShellCall::Recent)? else {
                return Err(stopped());
            };
            return Ok(
                json!({"currentProjectId":self.host.current().map(|s|s.project),"projects":rows.iter().map(json_value).collect::<Vec<_>>()}),
            );
        }
        if name == "create_project" || name == "open_project" {
            if self.run.is_some() {
                return Err(Error::invalid(
                    "Finish the current session before changing projects.",
                ));
            }
            return self.project_action(name, a);
        }
        let run = self
            .run
            .as_ref()
            .ok_or_else(|| Error::invalid("Start an MCP session first."))?;
        if run.editor && !catalog::permits("inspection", name) {
            return Err(Error::invalid("Editor access is inspection-only."));
        }
        if run.session.is_none() && !catalog::permits("app", name) {
            return Err(Error::invalid("This tool requires an attached project."));
        }
        if !catalog::IMPLEMENTED.contains(&name) {
            return Err(Error::unavailable(catalog::backing(name)));
        }
        if name == "manage_project" {
            return self.manage(a);
        }
        let p = self.published()?;
        let id = &p.project;
        if name == "save_project" {
            return self
                .host
                .save_project(run.session.clone().unwrap(), p.snapshot.doc_id);
        }
        let doc = document(&p);
        let time = p
            .ui_state
            .transport
            .get("time")
            .and_then(pm_model::Value::as_f64)
            .unwrap_or(0.);
        let selection = json!({"layers":p.selection.layers(),"keys":p.selection.keys(),"chan":p.selection.channel()});
        match name {
            "get_project_state" | "get_timeline" => {
                let result = if name == "get_project_state" {
                    pm_mcp::views::project_state_data(doc, selection, time, p.revision(), a)?
                } else {
                    pm_mcp::views::timeline_data(doc, selection, time, p.revision(), a)?
                };
                let reset = self
                    .run
                    .as_ref()
                    .unwrap()
                    .edit
                    .as_ref()
                    .is_some_and(|e| !e.changed && e.revision != p.revision());
                let mark = if reset { Some(self.mark(id)?) } else { None };
                let run = self.run.as_mut().unwrap();
                run.baseline = p.revision();
                if let Some(edit) = &mut run.edit
                    && edit.revision != p.revision()
                {
                    edit.interleaved |= edit.changed;
                    if let Some(mark) = mark {
                        edit.mark = mark;
                        edit.before = document(&p);
                    }
                    edit.revision = p.revision();
                }
                Ok(result)
            }
            "select_layers" => {
                let ids = a["layerIds"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|v| v.as_str().unwrap().to_owned())
                    .collect();
                self.call(
                    id,
                    SelectionCall::Layers {
                        ids,
                        add: a["add"] == true,
                    },
                )?;
                let p = self.published()?;
                Ok(
                    json!({"selection":{"layers":p.selection.layers(),"keys":p.selection.keys(),"chan":p.selection.channel()}}),
                )
            }
            "rollback_changes" => {
                self.rollback()?;
                Ok(json!({"rolledBack":true,"revision":self.published()?.revision()}))
            }
            "apply_commands" | "edit_layer_strips" => {
                let run = self.run.as_ref().unwrap();
                let expected = run.edit.as_ref().map_or(run.baseline, |e| e.revision);
                if expected != p.revision() {
                    return Err(Error::conflict(
                        "Project changed. Read get_project_state and retry.",
                    ));
                }
                let label = a["label"]
                    .as_str()
                    .filter(|s| !s.trim().is_empty())
                    .unwrap_or("Agent composition edit")
                    .trim()
                    .chars()
                    .take(80)
                    .collect::<String>();
                let mut strip_state = false;
                let (body, selection, mut result) = if name == "apply_commands" {
                    let batch = commands::plan_with_definitions(
                        &doc,
                        p.selection.layers(),
                        a["commands"].as_array().unwrap(),
                        time,
                        Some(&self.host.kernel.state()),
                    )?;
                    (
                        WriteBody::Commands(batch.commands),
                        Some(batch.selection),
                        json!({"applied":batch.descriptions,"clip":null,"results":batch.results,"message":label}),
                    )
                } else {
                    let plan = pm_mcp::strips::plan(&doc, a, time)?;
                    strip_state = plan.state;
                    (
                        plan.body,
                        None,
                        json!({"applied":[],"results":plan.results,"message":label,"strips":plan.result}),
                    )
                };
                if self.run.as_ref().unwrap().edit.is_none() {
                    let mark = self.mark(id)?;
                    self.run.as_mut().unwrap().edit = Some(Edit {
                        before: doc,
                        mark,
                        revision: expected,
                        changed: false,
                        interleaved: false,
                        label: label.clone(),
                    });
                }
                let ack = self.call(
                    id,
                    Call::Transact {
                        label: Some(label.clone()),
                        body,
                        expected_revision: Some(expected),
                    },
                )?;
                // Record an applied acknowledgement before any follow-up can fail.
                let edit = self.run.as_mut().unwrap().edit.as_mut().unwrap();
                edit.changed |= ack.revision != expected;
                edit.revision = ack.revision;
                edit.interleaved |= ack.folded.is_some();
                edit.label = label;
                result["revision"] = json!(ack.revision);
                if strip_state {
                    result["strips"] = json!({"results":result["results"],"revision":ack.revision});
                }
                if let Some(notice) = ack.notice {
                    result["notice"] = json!(format!("{notice:?}"));
                }
                if let Some(ids) = selection {
                    self.call(id, SelectionCall::Layers { ids, add: false })?;
                }
                Ok(result)
            }
            _ => Err(Error::unavailable(catalog::backing(name))),
        }
    }
    fn project_action(&self, name: &str, a: &Value) -> Result<Value> {
        if name == "open_project" {
            let id = a["projectId"].as_str().unwrap();
            if id.is_empty()
                || !id
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-')
            {
                return Err(Error::invalid("Invalid projectId"));
            }
            if self.host.current().is_some_and(|s| s.project == id) {
                let p = self.host.kernel.published(id).ok_or_else(stopped)?;
                return Ok(identity(&p));
            }
            return self.host.open(
                id.into(),
                SessionSource::Library {
                    profile: self.host.profile.clone(),
                    project: id.into(),
                },
            );
        }
        let id = pm_kernel::new_id("P");
        let name = a["name"]
            .as_str()
            .filter(|s| !s.trim().is_empty())
            .ok_or_else(|| Error::invalid("name is required"))?
            .trim();
        let duration = a["duration"].as_f64().unwrap_or(10.);
        let value = json!({"id":id,"compId":pm_kernel::new_id("C"),"compName":"Main","comps":{},"name":name,"w":a["width"].as_u64().unwrap_or(1920),"h":a["height"].as_u64().unwrap_or(1080),"fps":a["fps"].as_f64().unwrap_or(30.),"dur":duration,"work":[0,duration],"bg":"#000000","assets":{},"layers":[],"params":{},"markers":[]});
        let mut result = self.host.open(id, SessionSource::New(model(&value)))?;
        self.host.save(self.host.current().unwrap(), None)?;
        for (k, v) in [
            ("width", "w"),
            ("height", "h"),
            ("fps", "fps"),
            ("duration", "dur"),
        ] {
            result[k] = value[v].clone();
        }
        Ok(result)
    }
    fn manage(&mut self, a: &Value) -> Result<Value> {
        if self.run.as_ref().unwrap().session.is_some() {
            self.published()?;
        }
        let operation = a["operation"].as_str().unwrap();
        if operation == "list" {
            let ShellReply::Recent(rows) = self.host.shell(ShellCall::Recent)? else {
                return Err(stopped());
            };
            let trash = pm_kernel::LibraryStore::open(&self.host.profile)
                .read("projectTrash")
                .map_err(Error::kernel)?
                .map(|v| json_value(&v))
                .unwrap_or(json!([]));
            return Ok(
                json!({"activeProjectId":self.host.current().map(|s|s.project),"projects":rows.iter().map(json_value).collect::<Vec<_>>(),"trash":trash}),
            );
        }
        if ["create", "open"].contains(&operation) && self.run.as_ref().unwrap().session.is_some() {
            return Err(Error::invalid("Change projects from an app session."));
        }
        // Library operations commit the previous agent batch, as in the standalone host.
        if self.run.as_ref().unwrap().edit.is_some() {
            let p = self.published()?;
            let edit = self.run.as_ref().unwrap().edit.as_ref().unwrap();
            if edit.changed && !edit.interleaved && edit.revision == p.revision() {
                self.call(
                    &p.project,
                    HistoryCall::Squash {
                        mark: edit.mark.clone(),
                        label: format!("Agent · {}", edit.label),
                        expected_revision: edit.revision,
                    },
                )?;
            }
            let changed = edit.changed;
            self.run.as_mut().unwrap().committed |= changed;
            self.run.as_mut().unwrap().edit = None;
        }
        let id = a["projectId"]
            .as_str()
            .map(str::to_owned)
            .or_else(|| self.host.current().map(|s| s.project));
        if operation == "create" {
            let result=self.project_action("create_project",&json!({"name":a["name"],"width":a["w"],"height":a["h"],"fps":a["fps"],"duration":a["dur"]}))?;
            self.run.as_mut().unwrap().committed = true;
            return Ok(result);
        }
        let id = id.ok_or_else(|| Error::invalid("projectId is required"))?;
        match operation {
            "open" => {
                let result = self.project_action("open_project", &json!({"projectId":id}))?;
                self.run.as_mut().unwrap().committed = true;
                return Ok(result);
            }
            "save" => {
                let session = self
                    .host
                    .current()
                    .filter(|s| s.project == id)
                    .ok_or_else(|| Error::invalid("Project is not active"))?;
                self.host.save(session, None)?;
            }
            "rename" => {
                if self.host.current().is_none_or(|s| s.project != id) {
                    return Err(Error::unavailable(
                        "rename unopened library slot through kernel shell",
                    ));
                }
                let p = self.host.kernel.published(&id).ok_or_else(stopped)?;
                let name = a["name"]
                    .as_str()
                    .filter(|s| !s.trim().is_empty())
                    .ok_or_else(|| Error::invalid("name is required"))?;
                self.call(
                    &id,
                    Call::Transact {
                        label: Some("Rename Project".into()),
                        body: WriteBody::Commands(vec![
                            CompCommand::Set {
                                patch: Box::new(pm_kernel::commands::comps::CompPatch {
                                    name: Some(name.into()),
                                    ..Default::default()
                                }),
                            }
                            .into(),
                        ]),
                        expected_revision: Some(p.revision()),
                    },
                )?;
                self.host
                    .save(p.session.as_ref().ok_or_else(stopped)?.handle.clone(), None)?;
            }
            "duplicate" => {
                let new = pm_kernel::new_id("P");
                let name = a["name"]
                    .as_str()
                    .filter(|s| !s.trim().is_empty())
                    .ok_or_else(|| Error::invalid("name is required"))?;
                self.host.shell(ShellCall::Duplicate {
                    project: id,
                    id: new.clone(),
                    name: name.into(),
                })?;
                self.run.as_mut().unwrap().committed = true;
                return Ok(json!({"projectId":new,"name":name,"active":false}));
            }
            "trash" | "restore" => {
                let reply = self.host.shell(if operation == "trash" {
                    ShellCall::Trash(id.clone())
                } else {
                    ShellCall::Restore(id.clone())
                })?;
                if !matches!(reply, ShellReply::Changed(true)) {
                    return Err(Error::kernel(reply));
                }
            }
            _ => return Err(Error::invalid("Unknown project operation")),
        }
        self.run.as_mut().unwrap().committed = true;
        if let Some(s) = self.run.as_ref().unwrap().session.as_ref() {
            self.run.as_mut().unwrap().baseline = self
                .host
                .kernel
                .published(&s.project)
                .ok_or_else(stopped)?
                .revision();
        }
        Ok(
            json!({"projectId":id,"operation":operation,"activeProjectId":self.host.current().map(|s|s.project)}),
        )
    }
}
impl ToolConnection for Connection {
    fn list(&mut self) -> Result<Value> {
        Ok(json!(catalog::tools()))
    }
    fn invoke(&mut self, name: &str, a: Value) -> Result<Value> {
        if name == "project_memory" {
            return Ok(match self.project_memory(&a) {
                Ok(value) => pm_mcp::success_model(&value),
                Err(error) => pm_mcp::failure(error),
            });
        }
        Ok(match self.dispatch(name, &a) {
            Ok(v) => pm_mcp::success(v),
            Err(e) => pm_mcp::failure(e),
        })
    }
    fn disconnect(&mut self) -> Result<()> {
        if self.run.is_some() {
            self.finish(false)?;
        }
        Ok(())
    }
}
impl Drop for Connection {
    fn drop(&mut self) {
        if let Err(e) = self.disconnect() {
            eprintln!("MCP disconnect: {e}");
        }
    }
}

pub fn headless(profile: PathBuf, launch: Option<PathBuf>) -> anyhow::Result<()> {
    std::thread::spawn(move || -> anyhow::Result<()> {
        let actor = KernelActor::spawn(KernelConfig {
            journal_root: Some(profile.join("journals")),
            backup_root: Some(profile.join("backups")),
            ..Default::default()
        });
        let (host, _) = Host::new(actor.handle(), profile.clone());
        attach(&host.kernel, &profile)?;
        if let Some(path) = launch {
            host.open(super::session::stable_id(&path), SessionSource::File(path))?;
        }
        let shared = host.clone();
        let server = pm_mcp::transport::LoopbackServer::start_with(&profile, move || {
            Connection::new(shared.clone())
        })?;
        let result = pm_mcp::transport::serve_stdio(
            std::io::stdin().lock(),
            std::io::stdout().lock(),
            Connection::new(host),
        );
        drop(server);
        actor.shutdown();
        result.map_err(Into::into)
    })
    .join()
    .map_err(|_| anyhow::anyhow!("MCP worker panicked"))?
}
pub fn attach(kernel: &KernelHandle, profile: &std::path::Path) -> anyhow::Result<()> {
    match kernel
        .shell(ShellCall::AttachLibrary {
            profile: profile.to_owned(),
            format: pm_kernel::persistence::DiskFormat::V1_2,
            electron_profile: None,
        })
        .wait()
    {
        Some(Ok(ShellReply::Library(pm_kernel::locks::OpenDecision::Writable))) => Ok(()),
        other => anyhow::bail!("Cannot attach isolated native library: {other:?}"),
    }
}

#[cfg(test)]
#[path = "mcp_tests.rs"]
mod tests;
