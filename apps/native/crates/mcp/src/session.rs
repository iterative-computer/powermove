use crate::{Automation, Error, Result, catalog, commands, failure, json_value, success, views};
use pm_kernel::session::{ShellCall, ShellReply};
use pm_kernel::{HistoryMark, Origin};
use serde_json::{Value, json};
use std::{
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU32, Ordering},
    },
};

struct Transaction {
    snapshot: Value,
    mark: HistoryMark,
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
    committed: bool,
}
/// One connection owns one session and workspace. No session credential is
/// exposed in MCP results. Drop/disconnect attempts guarded rollback.
pub struct McpSession {
    host: Arc<Mutex<Automation>>,
    active: Option<Active>,
    writer: u32,
    closed: bool,
}
impl McpSession {
    pub fn new(host: Arc<Mutex<Automation>>) -> Self {
        static NEXT: AtomicU32 = AtomicU32::new(1000);
        Self {
            host,
            active: None,
            writer: NEXT.fetch_add(1, Ordering::Relaxed),
            closed: false,
        }
    }
    pub fn tools(&self) -> Value {
        json!(catalog::tools())
    }
    pub fn call(&mut self, name: &str, args: Value) -> Value {
        match self.dispatch(name, &args) {
            Ok(v) => success(v),
            Err(e) => failure(e),
        }
    }
    pub fn dispatch(&mut self, name: &str, args: &Value) -> Result<Value> {
        if self.closed {
            return Err(Error::invalid("This MCP connection is closed."));
        }
        catalog::validate(name, args)?;
        if name == "start_session" {
            return self.start(args);
        }
        if name == "finish_session" {
            if self.active.is_none() {
                return Err(Error::invalid("No MCP session is active."));
            }
            return self.finish(args["commit"] == true);
        }
        let shared = self.host.clone();
        let mut host = shared
            .lock()
            .map_err(|_| Error::unavailable("headless host stopped"))?;
        if ["list_projects", "create_project", "open_project"].contains(&name) {
            if name != "list_projects" && self.active.is_some() {
                return Err(Error::invalid(
                    "Finish the current session before changing projects.",
                ));
            }
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
        if session.access == "editor" && !catalog::permits("inspection", name) {
            return Err(Error::invalid("This tool requires project access."));
        }
        if session.context == "app" && !catalog::permits("app", name) {
            return Err(Error::invalid(
                "This tool requires project access and an attached project.",
            ));
        }
        if name == "manage_project" {
            if let Some(id) = &session.project {
                assert_project(session, &host, id)?;
            }
            return self.manage(&mut host, args);
        }
        if !catalog::IMPLEMENTED.contains(&name) {
            return Err(Error::unavailable(catalog::backing(name)));
        }
        let session = self.active.as_mut().unwrap();
        let id = session
            .project
            .as_ref()
            .ok_or_else(|| Error::invalid("This tool requires an attached project."))?
            .clone();
        assert_project(session, &host, &id)?;
        if name == "get_project_state" || name == "get_timeline" {
            let result = if name == "get_project_state" {
                views::project_state(&host, &id, args)?
            } else {
                views::timeline(&host, &id, args)?
            };
            refresh(session, &host, &id)?;
            return Ok(result);
        }
        match name {
            "select_layers" => {
                let ids = args["layerIds"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|v| v.as_str().unwrap().to_owned())
                    .collect::<Vec<_>>();
                Ok(json!({"selection":host.select_layers(&id,&ids,args["add"]==true)?}))
            }
            "save_project" => host.save_project(&id),
            "apply_commands" | "edit_layer_strips" => {
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
                let label = args["label"]
                    .as_str()
                    .filter(|s| !s.trim().is_empty())
                    .unwrap_or("Agent composition edit")
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
                let batch = if strips.is_none() {
                    Some(commands::plan_with_definitions(
                        &p,
                        &selection,
                        args["commands"].as_array().unwrap(),
                        host.playhead(&id),
                        Some(host.kernel().state()),
                    )?)
                } else {
                    None
                };
                if session.transaction.is_none() {
                    session.transaction = Some(Transaction {
                        snapshot: p,
                        mark: host.history_mark(&id)?,
                        revision,
                        changed: false,
                        interleaved: false,
                        label: label.clone(),
                    });
                }
                let (ack, descriptions, results, strips_result) = if let Some(plan) = strips {
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
                    host.select_layers(&id, &batch.selection, false)?;
                    (ack, batch.descriptions, batch.results, None)
                };
                let tx = session.transaction.as_mut().unwrap();
                tx.revision = ack.revision;
                tx.label = label.clone();
                tx.changed |= ack.revision != revision;
                // Folded foreign gestures retain their own normal undo entry.
                tx.interleaved |= ack.folded.is_some();
                let mut result = json!({"applied":descriptions,"clip":null,"results":results,"message":label,"revision":ack.revision});
                if let Some(notice) = ack.notice {
                    result["notice"] = json!(format!("{notice:?}"));
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
    fn start(&mut self, args: &Value) -> Result<Value> {
        if self.active.is_some() {
            return Err(Error::invalid("Finish the current MCP session first."));
        }
        let access = args["access"].as_str().unwrap_or("project");
        let context = args["context"].as_str().unwrap_or("project");
        if context == "app" && access == "editor" {
            return Err(Error::invalid("App sessions require project access."));
        }
        let host = self
            .host
            .lock()
            .map_err(|_| Error::unavailable("headless host stopped"))?;
        let project = if context == "project" {
            Some(host.active.clone().ok_or_else(|| {
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
            .map(|id| host.store(id).map(|s| s.snapshot().doc_id))
            .transpose()?;
        let run_id = format!("mcp-{}", pm_kernel::new_id("run"));
        let mut result = json!({"runId":run_id,"projectId":project.as_deref().unwrap_or("powermove-global"),"access":access});
        let workspace = if access == "editor" {
            None
        } else {
            let root = host.profile.join("mcp-runs").join(&run_id);
            // The runtime/sandbox lane will populate the complete API pack and
            // extension baseline. No executable scripting is enabled in P1.
            for name in ["extensions", "api", "artifacts", "inputs"] {
                std::fs::create_dir_all(root.join(name))?;
            }
            let input = if let Some(ref id) = project {
                host.read_document(id)?
            } else {
                json!({})
            };
            std::fs::write(
                root.join("inputs/powermove-project.json"),
                input.to_string(),
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
                "Read get_project_state before edits. Use apply_commands for typed, editable commands. finish_session commit:true keeps edits; false or disconnect attempts guarded rollback. Engine, panels, agents, Store, user MCP servers and scripting sandbox are not available in this build. api/mcp-tools.json lists the frozen public contract; unsupported tools return a structured error. Extension staging is inert until the runtime lane is connected."
            );
            Some(root)
        };
        self.active = Some(Active {
            project,
            document,
            access: access.into(),
            context: context.into(),
            baseline,
            transaction: None,
            workspace,
            committed: false,
        });
        Ok(result)
    }
    pub fn finish(&mut self, commit: bool) -> Result<Value> {
        let Some(mut session) = self.active.take() else {
            return Ok(json!({"changed":false}));
        };
        let result = (|| {
            let mut host = self
                .host
                .lock()
                .map_err(|_| Error::unavailable("headless host stopped"))?;
            let Some(id) = session.project.clone() else {
                return Ok(json!({"changed":session.committed}));
            };
            assert_project(&session, &host, &id)?;
            let Some(tx) = session.transaction.as_ref() else {
                return Ok(json!({"changed":session.committed}));
            };
            let changed = tx.changed;
            let mut history_id = None;
            if !commit {
                rollback(&mut session, &mut host, &id, self.writer)?;
            } else if changed && !tx.interleaved && tx.revision == host.revision(&id)? {
                history_id = host.squash(&id, &tx.mark, &format!("Agent · {}", tx.label))?;
            }
            Ok(
                json!({"changed":session.committed||commit&&changed,"historyId":history_id.map(|id|id.to_string())}),
            )
        })();
        if let Some(root) = session.workspace {
            let cleanup = if commit {
                std::fs::remove_dir_all(root.join("extensions"))
            } else {
                std::fs::remove_dir_all(root)
            };
            if let Err(e) = cleanup
                && result.is_ok()
            {
                return Err(e.into());
            }
        }
        result
    }
    fn manage(&mut self, host: &mut Automation, a: &Value) -> Result<Value> {
        let operation = a["operation"].as_str().unwrap();
        let id = a["projectId"]
            .as_str()
            .map(str::to_owned)
            .or_else(|| host.active.clone());
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
            return Ok(json!({"activeProjectId":host.active,"projects":rows,"trash":trash}));
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
            if tx.changed && !tx.interleaved && host.revision(&project)? == tx.revision {
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
                if host.active.as_deref() != Some(&id) {
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
        Ok(json!({"projectId":id,"operation":operation,"activeProjectId":host.active}))
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
    if host.active.as_deref() != Some(id)
        || host.store(id).ok().map(|s| s.snapshot().doc_id) != session.document
    {
        return Err(Error::conflict(
            "The active project changed or reopened. All work was preserved. Start a new agent run in this project.",
        ));
    }
    Ok(())
}
fn refresh(session: &mut Active, host: &Automation, id: &str) -> Result<()> {
    let revision = host.revision(id)?;
    session.baseline = revision;
    if let Some(tx) = &mut session.transaction
        && tx.revision != revision
    {
        if tx.changed {
            tx.interleaved = true;
        } else {
            tx.snapshot = host.read_document(id)?;
            tx.mark = host.history_mark(id)?;
        }
        tx.revision = revision;
    }
    Ok(())
}
fn rollback(session: &mut Active, host: &mut Automation, id: &str, writer: u32) -> Result<()> {
    let Some(tx) = session.transaction.as_mut() else {
        return Ok(());
    };
    if tx.interleaved {
        return Err(Error::conflict(
            "Other edits occurred during this run. Use the editor Undo command to undo agent changes without restoring over that work.",
        ));
    }
    if !tx.changed {
        return Ok(());
    }
    host.restore(id, tx.snapshot.clone(), tx.revision, writer)?;
    tx.snapshot = host.read_document(id)?;
    tx.revision = host.revision(id)?;
    tx.mark = host.history_mark(id)?;
    tx.changed = false;
    session.baseline = tx.revision;
    Ok(())
}
