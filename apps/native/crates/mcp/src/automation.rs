use crate::{Error, Result, json_value, model};
use pm_kernel::session::{SessionHandle, SessionSource, ShellCall, ShellReply};
use pm_kernel::{
    Ack, ActorHistoryMark, Call, Caller, Command, Kernel, KernelConfig, KernelEvent, KernelHandle,
    Origin, Published, Reply, Request, WriteBody,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{Arc, mpsc},
};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct AutomationEvent {
    pub project_id: String,
    pub revision: u64,
    pub kind: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct CaptureRequest {
    pub project_id: String,
    pub revision: u64,
    pub times: Vec<f64>,
    pub width: u32,
}
/// A later engine lane supplies real image content (MCP base64 image blocks).
pub trait EngineHook: Send + Sync {
    fn capture(&self, request: &CaptureRequest) -> Result<Value>;
    fn capture_snapshot(
        &self,
        request: &CaptureRequest,
        _snapshot: &crate::services::FrozenProject,
    ) -> Result<Value> {
        self.capture(request)
    }
    fn resolve_media(
        &self,
        _: &crate::services::MediaRequest,
    ) -> Result<crate::media_tools::MediaSource> {
        Err(Error::unavailable(
            "engine original media resolver and evaluated source timing",
        ))
    }
    fn probe(
        &self,
        _: &crate::services::MediaRequest,
        _: &crate::media_tools::MediaSource,
    ) -> Result<crate::media_tools::Probe> {
        Err(Error::unavailable("engine native media probe"))
    }
    fn sample(&self, _: &crate::media_tools::FrameRequest) -> Result<Vec<crate::services::Image>> {
        Err(Error::unavailable("engine native source-frame decoder"))
    }
    fn scan(&self, _: &crate::media_tools::ScanRequest) -> Result<crate::media_tools::Scan> {
        Err(Error::unavailable("engine informative source-frame scan"))
    }
    fn sheet(&self, _: &crate::media_tools::SheetRequest) -> Result<crate::services::Image> {
        Err(Error::unavailable(
            "engine labelled contact-sheet assembler",
        ))
    }
    fn composition_sheet(
        &self,
        request: &crate::media_tools::SheetRequest,
    ) -> Result<crate::media_tools::SheetOutput> {
        Ok(crate::media_tools::SheetOutput {
            image: self.sheet(request)?,
            completed: request.cells.len(),
        })
    }
    fn waveform(
        &self,
        _: &crate::media_tools::WaveformRequest,
    ) -> Result<crate::media_tools::Waveform> {
        Err(Error::unavailable(
            "engine measured waveform and loudness worker",
        ))
    }
}
/// Stable ids are owned by each surface, never derived from child order. The
/// same provider is used windowed and headless. Sensitive controls are omitted.
pub trait SurfaceHook: Send + Sync {
    fn list_panels(&self) -> Result<Value>;
    fn inspect(&self, panel_id: &str, element_id: Option<&str>) -> Result<Value>;
    fn drive(&self, panel_id: &str, element_id: &str, action: &Value) -> Result<Value>;
    fn capture(&self, panel_id: &str) -> Result<Value>;
}
/// Single owner headless host. Call on a dedicated worker; do not put this
/// behind a mutex that a UI thread waits for. It never discovers a profile.
pub struct Automation {
    kernel: Kernel,
    pub profile: PathBuf,
    pub exports: PathBuf,
    pub active: Option<String>,
    sessions: HashMap<String, SessionHandle>,
    subscribers: Vec<mpsc::Sender<AutomationEvent>>,
    pub engine: Option<Arc<dyn EngineHook>>,
    pub surfaces: Option<Arc<dyn SurfaceHook>>,
    pub media_registry: Option<Arc<dyn crate::media::MediaRegistry>>,
    /// Loaded 3D geometry from the native import/engine worker, for snapshot-bound placement and exploded views.
    pub scene3d_assets: Option<Arc<dyn pm_scene3d::geometry::AssetGeometry>>,
    pub export_child: Arc<dyn crate::services::ExportChild>,
    pub store_reads: Option<Arc<dyn crate::store::StoreRead>>,
    pub extension_sources: Option<Arc<dyn crate::extensions::ExtensionSources>>,
}
impl Automation {
    /// `profile` must be an explicit, isolated native destination, never the
    /// Electron profile. Kernel's locks and copy decisions remain authoritative.
    pub fn new(profile: PathBuf, exports: PathBuf) -> Result<Self> {
        if !profile.is_absolute() || !exports.is_absolute() {
            return Err(Error::invalid(
                "Use absolute native profile and export paths",
            ));
        }
        std::fs::create_dir_all(&profile)?;
        let mut kernel = Kernel::new(KernelConfig {
            journal_root: Some(profile.join("journals")),
            ..Default::default()
        });
        match kernel
            .shell(ShellCall::AttachLibrary {
                profile: profile.clone(),
                format: pm_kernel::persistence::DiskFormat::V1_2,
                electron_profile: None,
            })
            .map_err(Error::kernel)?
        {
            ShellReply::Library(pm_kernel::locks::OpenDecision::Writable) => {}
            other => return Err(Error::kernel(other)),
        }
        Ok(Self {
            kernel,
            profile,
            exports,
            active: None,
            sessions: HashMap::new(),
            subscribers: Vec::new(),
            engine: None,
            surfaces: None,
            media_registry: None,
            scene3d_assets: None,
            export_child: Arc::new(crate::services::NativeExportChild),
            store_reads: None,
            extension_sources: None,
        })
    }
    pub fn kernel(&self) -> &Kernel {
        &self.kernel
    }
    pub fn subscribe(&mut self) -> mpsc::Receiver<AutomationEvent> {
        let (tx, rx) = mpsc::channel();
        self.subscribers.push(tx);
        rx
    }
    fn emit(&mut self, id: &str, kind: &str) {
        let revision = self
            .kernel
            .project(id)
            .map(|p| p.store().revision())
            .unwrap_or(0);
        let event = AutomationEvent {
            project_id: id.into(),
            revision,
            kind: kind.into(),
        };
        self.subscribers.retain(|tx| tx.send(event.clone()).is_ok());
    }
    pub fn revision(&self, id: &str) -> Result<u64> {
        Ok(self.store(id)?.revision())
    }
    pub fn store(&self, id: &str) -> Result<&pm_kernel::DocumentStore> {
        self.kernel
            .project(id)
            .map(|p| p.store())
            .ok_or_else(|| Error::invalid("Project is not open"))
    }
    pub fn read_document(&self, id: &str) -> Result<Value> {
        let store = self.store(id)?;
        let mut project = json_value(&pm_model::canonical::realize_project(
            store
                .snapshot()
                .document
                .to_value()
                .ok_or_else(|| Error::invalid("Missing document"))?,
        ));
        project["revision"] = json!(store.revision());
        Ok(project)
    }
    pub fn selection(&self, id: &str) -> Result<Value> {
        let s = self.store(id)?.selection();
        Ok(json!({"layers":s.layers(),"keys":s.keys(),"chan":s.channel()}))
    }
    pub fn playhead(&self, id: &str) -> f64 {
        self.kernel
            .state()
            .ui
            .get(id)
            .and_then(|ui| ui.transport.get("time"))
            .and_then(pm_model::Value::as_f64)
            .unwrap_or(0.0)
    }
    pub fn apply_commands(
        &mut self,
        id: &str,
        commands: Vec<Command>,
        label: &str,
        origin: Origin,
        expected_revision: u64,
        writer: u32,
    ) -> Result<Ack> {
        if !matches!(origin, Origin::Mcp | Origin::Agent) {
            return Err(Error::invalid(
                "Automation edits require Mcp or Agent origin",
            ));
        }
        self.call(
            id,
            Request::new(
                Caller::realm(writer, 1, origin),
                Call::Transact {
                    label: Some(label.into()),
                    body: WriteBody::Commands(commands),
                    expected_revision: Some(expected_revision),
                },
            ),
        )
    }
    pub(crate) fn apply_body(
        &mut self,
        id: &str,
        body: WriteBody,
        label: &str,
        expected: u64,
        writer: u32,
    ) -> Result<Ack> {
        self.call(
            id,
            Request::new(
                Caller::realm(writer, 1, Origin::Mcp),
                Call::Transact {
                    label: Some(label.into()),
                    body,
                    expected_revision: Some(expected),
                },
            ),
        )
    }
    fn call(&mut self, id: &str, request: Request) -> Result<Ack> {
        let handled = self.kernel.handle(id, request).map_err(Error::kernel)?;
        let ack = handled
            .reply
            .ok_or_else(|| Error::unavailable("external history continuation runtime"))?;
        if let Reply::Throw(e) = &ack.reply {
            return Err(Error::kernel(e));
        }
        // A notice can accompany an applied edit (notably NotDurable). Keep
        // its acknowledgement so the session still tracks and can undo it.
        if let Some(notice) = &ack.notice {
            eprintln!("MCP kernel notice: {notice:?}");
            self.emit(id, &format!("notice: {notice:?}"));
        }
        self.emit(id, "published");
        Ok(ack)
    }
    pub fn select_layers(
        &mut self,
        id: &str,
        ids: &[String],
        add: bool,
        writer: u32,
    ) -> Result<Value> {
        self.call(
            id,
            Request::new(
                Caller::realm(writer, 1, Origin::Mcp),
                pm_kernel::SelectionCall::Layers {
                    ids: ids.to_vec(),
                    add,
                },
            ),
        )?;
        self.selection(id)
    }
    pub fn history_mark(&mut self, id: &str, writer: u32) -> Result<ActorHistoryMark> {
        match self
            .call(
                id,
                Request::new(
                    Caller::realm(writer, 1, Origin::Mcp),
                    pm_kernel::HistoryCall::Mark,
                ),
            )?
            .reply
        {
            Reply::Mark(mark) => Ok(mark),
            other => Err(Error::kernel(other)),
        }
    }
    pub fn squash(
        &mut self,
        id: &str,
        mark: &ActorHistoryMark,
        label: &str,
    ) -> Result<Option<u64>> {
        let revision = self.revision(id)?;
        let writer = mark.writer();
        match self
            .call(
                id,
                Request::new(
                    Caller::realm(writer.realm, writer.generation, Origin::Mcp),
                    pm_kernel::HistoryCall::Squash {
                        mark: mark.clone(),
                        label: label.into(),
                        expected_revision: revision,
                    },
                ),
            )?
            .reply
        {
            Reply::Entry(entry) => Ok(entry),
            other => Err(Error::kernel(other)),
        }
    }
    pub fn restore(&mut self, id: &str, value: Value, expected: u64, writer: u32) -> Result<u64> {
        let document = self.store(id)?.doc_id();
        let ack = self
            .call(
                id,
                Request::new(
                    Caller::realm(writer, 1, Origin::Mcp),
                    Call::Restore {
                        project: model(&value),
                        label: "Roll back agent changes".into(),
                        expected_revision: expected,
                        expected_document: document,
                    },
                ),
            )
            .map_err(|e| Error::conflict(e.message))?;
        Ok(ack.revision)
    }
    pub fn undo(&mut self, id: &str, origin: Origin, writer: u32) -> Result<Ack> {
        self.call(
            id,
            Request::new(
                Caller::realm(writer, 1, origin),
                pm_kernel::HistoryCall::Undo,
            ),
        )
    }
    pub fn capture_frames(&self, request: &CaptureRequest) -> Result<Value> {
        if self.revision(&request.project_id)? != request.revision {
            return Err(Error::conflict("Capture revision is stale"));
        }
        self.engine
            .as_ref()
            .ok_or_else(|| Error::unavailable("engine-service headless render"))?
            .capture(request)
    }
    /// The frozen layout digest reads saved data, never live surface controls.
    pub fn panel_layout(&self, project: Option<&str>) -> Result<Value> {
        let state = self.kernel.state();
        let saved = project
            .map(|id| {
                self.kernel
                    .project(id)
                    .ok_or_else(|| Error::invalid("Project is not open"))
            })
            .transpose()?
            .map(|p| json_value(p.workspace()));
        let named = state
            .settings
            .get("workspaces")
            .map(json_value)
            .unwrap_or(json!([]));
        let current = state.settings.get("workspace").map(json_value);
        let workspace = saved.as_ref().filter(|v| v.is_object()).or_else(|| {
            named
                .as_array()?
                .iter()
                .find(|v| Some(&v["id"]) == current.as_ref())
        });
        let registrations = state.registries.get("panels");
        // A contribution stack exposes only its winning provider, retaining
        // first-registration ordering just like object property replacement.
        let mut panels = Vec::<Value>::new();
        if let Some(registrations) = registrations {
            for entry in registrations {
                let definition = json_value(&entry.definition);
                let id = definition["id"].as_str().unwrap_or(&entry.id);
                if id.is_empty() {
                    continue;
                }
                let panel = json!({"id":id,"title":definition["title"].as_str().filter(|s| !s.is_empty()).unwrap_or(id)});
                if let Some(previous) = panels.iter_mut().find(|v| v["id"] == id) {
                    *previous = panel;
                } else {
                    panels.push(panel);
                }
            }
        }
        let docks = workspace.and_then(|w| w["layout"]["docks"].as_array()).map(|docks| docks.iter().map(|dock| {
            let children = dock["panels"].as_array().map(|children| children.iter().map(|panel| {
                let id = panel["id"].as_str().unwrap_or("");
                let title = panel["title"].as_str().filter(|s| !s.is_empty())
                    .or_else(|| panels.iter().find(|v| v["id"] == id).and_then(|v| v["title"].as_str()))
                    .unwrap_or(id);
                json!({"id":id,"title":title,"collapsed":panel["collapsed"] == true,"flex":panel["flex"] == true,
                    "size":panel["size"].as_f64().map(|_| panel["size"].clone())})
            }).collect::<Vec<_>>()).unwrap_or_default();
            json!({"id":dock["id"].as_str().unwrap_or(""),"size":dock["size"].as_f64().map(|_| dock["size"].clone()),"panels":children})
        }).collect::<Vec<_>>()).unwrap_or_default();
        let mut identity = json!(null);
        if let Some(workspace) = workspace {
            identity = json!({});
            for key in ["id", "name"] {
                if let Some(value) = workspace.get(key) {
                    identity[key] = value.clone();
                }
            }
        }
        Ok(json!({"workspace":identity,"docks":docks,"registeredPanels":panels}))
    }
    pub fn list_panels(&self) -> Result<Value> {
        self.surfaces
            .as_ref()
            .ok_or_else(|| Error::unavailable("shell/surface host"))?
            .list_panels()
    }
    pub fn inspect_element(&self, panel: &str, element: Option<&str>) -> Result<Value> {
        self.surfaces
            .as_ref()
            .ok_or_else(|| Error::unavailable("shell/surface host"))?
            .inspect(panel, element)
    }
    pub fn drive_element(&self, panel: &str, element: &str, action: &Value) -> Result<Value> {
        self.surfaces
            .as_ref()
            .ok_or_else(|| Error::unavailable("shell/surface host"))?
            .drive(panel, element, action)
    }
    pub fn list_projects(&mut self) -> Result<Value> {
        let rows = match self
            .kernel
            .shell(ShellCall::Recent)
            .map_err(Error::kernel)?
        {
            ShellReply::Recent(rows) => {
                rows.into_iter().map(|v| json_value(&v)).collect::<Vec<_>>()
            }
            other => return Err(Error::kernel(other)),
        };
        Ok(json!({"currentProjectId":self.active,"projects":rows}))
    }
    pub fn create_project(&mut self, args: &Value) -> Result<Value> {
        let id = pm_kernel::new_id("P");
        let name = args["name"].as_str().unwrap_or("Untitled").trim();
        if name.is_empty() {
            return Err(Error::invalid("name is required"));
        }
        let w = args["width"].as_u64().unwrap_or(1920);
        let h = args["height"].as_u64().unwrap_or(1080);
        let fps = args["fps"].as_f64().unwrap_or(30.0);
        let dur = args["duration"].as_f64().unwrap_or(10.0);
        let value = json!({"id":id,"compId":pm_kernel::new_id("C"),"compName":"Main","comps":{},"name":name,"w":w,"h":h,"fps":fps,"dur":dur,"work":[0,dur],"bg":"#000000","layers":[],"assets":{},"params":{},"markers":[]});
        let reply = self
            .kernel
            .shell(ShellCall::Open {
                id: id.clone(),
                source: SessionSource::New(model(&value)),
                electron_profile: None,
            })
            .map_err(Error::kernel)?;
        self.adopt(reply)?;
        self.save_library(&id)?;
        Ok(json!({"projectId":id,"name":name,"width":w,"height":h,"fps":fps,"duration":dur}))
    }
    fn adopt(&mut self, reply: ShellReply) -> Result<String> {
        match reply {
            ShellReply::Opened {
                session,
                decision: pm_kernel::locks::OpenDecision::Writable,
                recovery:
                    pm_kernel::session::RecoveryResult::None
                    | pm_kernel::session::RecoveryResult::Recovered { .. },
                ..
            } => {
                let id = session.project.clone();
                if let Some(previous) = self.active.clone().filter(|previous| previous != &id)
                    && let Some(handle) = self.sessions.get(&previous).cloned()
                {
                    self.kernel
                        .shell(ShellCall::Close(handle))
                        .map_err(Error::kernel)?;
                    self.sessions.remove(&previous);
                    self.emit(&previous, "closed");
                }
                self.sessions.insert(id.clone(), session);
                self.active = Some(id.clone());
                self.emit(&id, "opened");
                Ok(id)
            }
            other => Err(Error::kernel(other)),
        }
    }
    pub fn open_project(&mut self, id: &str) -> Result<Value> {
        if !id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
            || id.is_empty()
        {
            return Err(Error::invalid("Invalid projectId"));
        }
        if !self.sessions.contains_key(id) {
            let reply = self
                .kernel
                .shell(ShellCall::PrepareOpen {
                    id: id.into(),
                    source: SessionSource::Library {
                        profile: self.profile.clone(),
                        project: id.into(),
                    },
                    electron_profile: None,
                })
                .map_err(Error::kernel)?;
            self.adopt(reply)?;
        }
        self.active = Some(id.into());
        let value = self.read_document(id)?;
        Ok(json!({"projectId":id,"name":value["name"]}))
    }
    pub fn save_library(&mut self, id: &str) -> Result<()> {
        let handle = self
            .sessions
            .get(id)
            .ok_or_else(|| Error::invalid("No shell session"))?
            .clone();
        match self
            .kernel
            .shell(ShellCall::Save(handle))
            .map_err(Error::kernel)?
        {
            ShellReply::Saved {
                recovery_copy: None,
                ..
            } => Ok(()),
            other => Err(Error::kernel(other)),
        }
    }
    pub fn save_project(&mut self, id: &str) -> Result<Value> {
        std::fs::create_dir_all(&self.exports)?;
        let name = self.read_document(id)?["name"]
            .as_str()
            .unwrap_or("Project")
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '-' || c == ' ' {
                    c
                } else {
                    '_'
                }
            })
            .take(100)
            .collect::<String>();
        let session = self
            .sessions
            .get(id)
            .ok_or_else(|| Error::invalid("No shell session"))?
            .clone();
        let call = if self.store(id)?.path().is_some() {
            ShellCall::Save(session)
        } else {
            ShellCall::SaveAs {
                session,
                path: self
                    .exports
                    .join(format!("{}-{}.pmv", name, pm_kernel::new_id("export"))),
            }
        };
        // Session SaveAs owns the file lock, destination, metadata and journal
        // rebase. Writing the DocumentStore alone leaves these out of sync.
        let reply = self.kernel.shell(call).map_err(Error::kernel)?;
        if !matches!(
            reply,
            ShellReply::Saved {
                recovery_copy: None,
                ..
            }
        ) {
            return Err(Error::kernel(reply));
        }
        let path = self
            .store(id)?
            .path()
            .ok_or_else(|| Error::kernel("Missing saved path"))?;
        let result = json!({"path":path,"projectId":id});
        self.emit(id, "saved");
        Ok(result)
    }
    pub fn library_action(&mut self, call: ShellCall) -> Result<ShellReply> {
        self.kernel.shell(call).map_err(Error::kernel)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_foreign_entry_prevents_snapshot_restore_and_squash() {
        let root = std::env::temp_dir().join(pm_kernel::new_id("pm-mcp-open-entry"));
        {
            let mut host = Automation::new(root.clone(), root.join("exports")).unwrap();
            let id = host
                .create_project(&json!({"name":"Synthetic open entry"}))
                .unwrap()["projectId"]
                .as_str()
                .unwrap()
                .to_owned();
            let snapshot = host.read_document(&id).unwrap();
            let revision = host.revision(&id).unwrap();
            let mark = host.history_mark(&id, 23).unwrap();
            host.kernel
                .handle(
                    &id,
                    Request::new(
                        Caller::realm(22, 1, Origin::Interface),
                        pm_kernel::HistoryCall::Begin {
                            label: "Foreign edit".into(),
                        },
                    ),
                )
                .unwrap();
            assert_eq!(
                host.restore(&id, snapshot.clone(), revision, 23)
                    .unwrap_err()
                    .code,
                "revision_conflict"
            );
            assert_eq!(host.squash(&id, &mark, "Agent run").unwrap(), None);
            assert_eq!(host.read_document(&id).unwrap(), snapshot);
            assert!(host.kernel.project(&id).unwrap().open_entry().is_some());
        }
        std::fs::remove_dir_all(root).unwrap();
    }
}

/// Adapter to the app's existing actor. Reads are immutable; edits are awaited.
/// Selection, history and restore use the same actor messages as standalone tools.
#[derive(Clone)]
pub struct ActorAutomation {
    pub kernel: KernelHandle,
    pub engine: Option<Arc<dyn EngineHook>>,
    pub surfaces: Option<Arc<dyn SurfaceHook>>,
    pub media_registry: Option<Arc<dyn crate::media::MediaRegistry>>,
    pub export_child: Arc<dyn crate::services::ExportChild>,
}
impl ActorAutomation {
    pub fn new(kernel: KernelHandle) -> Self {
        Self {
            kernel,
            engine: None,
            surfaces: None,
            media_registry: None,
            export_child: Arc::new(crate::services::NativeExportChild),
        }
    }
    pub fn for_actor(&self, caller: Caller) -> pm_kernel::ActorHandle {
        self.kernel.for_actor(caller)
    }
    async fn actor_call(&self, id: &str, caller: Caller, call: impl Into<Call>) -> Result<Ack> {
        let ack = self
            .for_actor(caller)
            .call(id, call)
            .await
            .ok_or_else(|| Error::unavailable("kernel actor stopped"))?
            .map_err(Error::kernel)?;
        if let Reply::Throw(e) = &ack.reply {
            return Err(Error::kernel(e));
        }
        Ok(ack)
    }
    pub async fn select_layers(
        &self,
        id: &str,
        caller: Caller,
        ids: Vec<String>,
        add: bool,
    ) -> Result<Ack> {
        self.actor_call(id, caller, pm_kernel::SelectionCall::Layers { ids, add })
            .await
    }
    pub async fn history_mark(&self, id: &str, caller: Caller) -> Result<ActorHistoryMark> {
        match self
            .actor_call(id, caller, pm_kernel::HistoryCall::Mark)
            .await?
            .reply
        {
            Reply::Mark(mark) => Ok(mark),
            other => Err(Error::kernel(other)),
        }
    }
    pub async fn squash(
        &self,
        id: &str,
        caller: Caller,
        mark: ActorHistoryMark,
        expected_revision: u64,
        label: String,
    ) -> Result<Ack> {
        self.actor_call(
            id,
            caller,
            pm_kernel::HistoryCall::Squash {
                mark,
                label,
                expected_revision,
            },
        )
        .await
    }
    pub async fn restore(
        &self,
        id: &str,
        caller: Caller,
        value: Value,
        expected_revision: u64,
        expected_document: u64,
    ) -> Result<Ack> {
        self.actor_call(
            id,
            caller,
            Call::Restore {
                project: model(&value),
                label: "Roll back agent changes".into(),
                expected_revision,
                expected_document,
            },
        )
        .await
    }
    /// The caller runs SessionSave::run on its worker and returns the completion.
    pub async fn prepare_save(
        &self,
        session: SessionHandle,
        path: Option<PathBuf>,
    ) -> Result<pm_kernel::session::SessionSave> {
        self.kernel
            .prepare_save(session, path)
            .await
            .ok_or_else(|| Error::unavailable("kernel actor stopped"))?
            .map_err(Error::kernel)
    }
    pub async fn finish_save(
        &self,
        completion: pm_kernel::session::SessionSaveCompletion,
    ) -> Result<ShellReply> {
        self.kernel
            .finish_save(completion)
            .await
            .ok_or_else(|| Error::unavailable("kernel actor stopped"))?
            .map_err(Error::kernel)
    }
    pub fn inspect_project(&self, id: &str) -> Result<Arc<Published>> {
        self.kernel
            .published(id)
            .ok_or_else(|| Error::invalid("Project is not open"))
    }
    pub fn read_document(&self, id: &str) -> Result<Value> {
        let p = self.inspect_project(id)?;
        let mut value = json_value(&pm_model::canonical::realize_project(
            p.snapshot
                .document
                .to_value()
                .ok_or_else(|| Error::invalid("Missing document"))?,
        ));
        value["revision"] = json!(p.revision());
        Ok(value)
    }
    pub fn list_panels(&self) -> Result<Value> {
        self.surfaces
            .as_ref()
            .ok_or_else(|| Error::unavailable("shell/surface host"))?
            .list_panels()
    }
    pub fn inspect_element(&self, panel: &str, element: Option<&str>) -> Result<Value> {
        self.surfaces
            .as_ref()
            .ok_or_else(|| Error::unavailable("shell/surface host"))?
            .inspect(panel, element)
    }
    pub fn drive_element(&self, panel: &str, element: &str, action: &Value) -> Result<Value> {
        self.surfaces
            .as_ref()
            .ok_or_else(|| Error::unavailable("shell/surface host"))?
            .drive(panel, element, action)
    }
    pub fn capture_frames(&self, request: &CaptureRequest) -> Result<Value> {
        if self.inspect_project(&request.project_id)?.revision() != request.revision {
            return Err(Error::conflict("Capture revision is stale"));
        }
        self.engine
            .as_ref()
            .ok_or_else(|| Error::unavailable("engine-service headless render"))?
            .capture(request)
    }
    pub fn subscribe(&self) -> mpsc::Receiver<KernelEvent> {
        self.kernel.subscribe()
    }
    /// Use the application's generation-bound session, without opening another
    /// store or discovering a profile. The caller must handle recovery copies.
    pub async fn save(&self, session: SessionHandle) -> Result<ShellReply> {
        self.shell(ShellCall::Save(session)).await
    }
    pub async fn save_as(&self, session: SessionHandle, path: PathBuf) -> Result<ShellReply> {
        self.shell(ShellCall::SaveAs { session, path }).await
    }
    async fn shell(&self, call: ShellCall) -> Result<ShellReply> {
        self.kernel
            .shell(call)
            .await
            .ok_or_else(|| Error::unavailable("kernel actor stopped"))?
            .map_err(Error::kernel)
    }
    pub async fn apply_commands(
        &self,
        id: &str,
        commands: Vec<Command>,
        label: String,
        origin: Origin,
        expected_revision: u64,
        writer: u32,
    ) -> Result<Ack> {
        if !matches!(origin, Origin::Mcp | Origin::Agent) {
            return Err(Error::invalid(
                "Automation edits require Mcp or Agent origin",
            ));
        }
        let ack = self
            .kernel
            .call(
                id,
                Request::new(
                    Caller::realm(writer, 1, origin),
                    Call::Transact {
                        label: Some(label),
                        body: WriteBody::Commands(commands),
                        expected_revision: Some(expected_revision),
                    },
                ),
            )
            .await
            .ok_or_else(|| Error::unavailable("kernel actor stopped"))?
            .map_err(Error::kernel)?;
        if let Reply::Throw(ref e) = ack.reply {
            return Err(Error::kernel(e));
        }
        Ok(ack)
    }
}

#[cfg(test)]
#[path = "layout_tests.rs"]
mod layout_tests;
