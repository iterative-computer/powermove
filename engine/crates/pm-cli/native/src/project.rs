use crate::{Error, Result, error, to_json};
use pm_kernel::session::{SessionHandle, SessionSource, ShellCall, ShellReply};
use pm_kernel::{
    Call, Caller, KernelActor, KernelConfig, OpenSource, Origin, Published, Reply, Request,
    WriteBody,
};
use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

const SLOT: &str = "headless";
/// Owns the actor, never a second mutable DocumentStore. A scratch planner in
/// pm-mcp resolves references, then the actor stages the entire batch once.
pub struct Project {
    actor: Option<KernelActor>,
    pub kernel: pm_kernel::KernelHandle,
    session: Option<SessionHandle>,
    slot: String,
    source_version: Arc<pm_kernel::project_io::FileVersion>,
    scratch: Option<tempfile::TempDir>,
    pub source: PathBuf,
    applied_unsaved: bool,
    history_rejected: bool,
}
impl Project {
    pub fn open(path: &Path, editing: bool) -> Result<Self> {
        let source = path.canonicalize()?;
        let opened = pm_kernel::open_project(&source, Default::default()).map_err(Error::kernel)?;
        let source_version = opened
            .file
            .version
            .clone()
            .ok_or_else(|| error("kernel_error", "Missing source version"))?;
        let scratch = tempfile::Builder::new()
            .prefix("powermove-cli-")
            .tempdir()?;
        let actor = KernelActor::spawn(KernelConfig {
            journal_root: Some(scratch.path().join("journal")),
            ..Default::default()
        });
        let kernel = actor.handle();
        let mut project = Self {
            actor: Some(actor),
            kernel,
            session: None,
            slot: SLOT.into(),
            source_version,
            scratch: Some(scratch),
            source,
            applied_unsaved: false,
            history_rejected: false,
        };
        if editing {
            match project
                .kernel
                .shell(ShellCall::Open {
                    id: SLOT.into(),
                    source: SessionSource::File(project.source.clone()),
                    electron_profile: None,
                })
                .wait()
                .ok_or_else(|| error("kernel_error", "Actor stopped"))?
                .map_err(Error::kernel)?
            {
                ShellReply::Opened { session, info, .. } => {
                    project.slot = session.project.clone();
                    project.session = Some(session);
                    project.history_rejected = info.history_rejected;
                }
                reply => {
                    return Err(error(
                        "kernel_error",
                        format!("Open did not create a session: {reply:?}"),
                    ));
                }
            }
        } else {
            let info = project
                .kernel
                .open(SLOT, OpenSource::File(opened))
                .wait()
                .ok_or_else(|| error("kernel_error", "Actor stopped"))?
                .map_err(Error::kernel)?;
            project.history_rejected = info.history_rejected;
        }
        project.source_version.check(true).map_err(|_| {
            Error::conflict("Source changed during open; retry against the current file")
        })?;
        Ok(project)
    }
    pub fn check_source_hash(&self, expected: &str) -> Result<()> {
        if self.source_version.hash != expected {
            return Err(Error::conflict(
                "Source SHA-256 differs; no commands were applied",
            ));
        }
        self.source_version
            .check(true)
            .map_err(|_| Error::conflict("Source changed since open"))
    }
    pub fn published(&self) -> Result<Arc<Published>> {
        self.kernel
            .published(&self.slot)
            .ok_or_else(|| error("kernel_error", "Project is not published"))
    }
    pub fn document(&self) -> Result<Value> {
        let published = self.published()?;
        let value = published
            .snapshot
            .document
            .to_value()
            .ok_or_else(|| error("kernel_error", "Missing document"))?;
        let mut value = to_json(&pm_model::canonical::realize_project(value));
        value["revision"] = json!(published.revision());
        Ok(value)
    }
    pub fn composition_id(&self) -> Result<String> {
        if let Some(id) = self.published()?.snapshot.open_comp() {
            return Ok(id);
        }
        let document = self.document()?;
        let id = document["id"]
            .as_str()
            .filter(|s| !s.is_empty())
            .ok_or_else(|| Error::invalid("Project has no composition identity"))?;
        Ok(format!("@root:{id}"))
    }
    fn selection(&self) -> Result<Value> {
        let p = self.published()?;
        Ok(
            json!({"layers":p.selection.layers(), "keys":p.selection.keys(), "chan":p.selection.channel()}),
        )
    }
    pub fn info(&self, args: &Value) -> Result<Value> {
        pm_mcp::views::project_state_data(
            self.document()?,
            self.selection()?,
            0.,
            self.published()?.revision(),
            args,
        )
    }
    pub fn timeline(&self, args: &Value) -> Result<Value> {
        pm_mcp::views::timeline_data(
            self.document()?,
            self.selection()?,
            0.,
            self.published()?.revision(),
            args,
        )
    }
    pub fn check_revision(&self, expected: u64) -> Result<()> {
        let actual = self.published()?.revision();
        if expected != actual {
            return Err(Error::conflict(format!(
                "Expected revision {expected}, found {actual}; no commands were applied"
            )));
        }
        Ok(())
    }
    pub fn apply(&mut self, args: &Value, output: &Path) -> Result<Value> {
        // Check before admitting a write. SaveAs still performs its own locking
        // and version checks, so a concurrent destination creator wins safely.
        self.source_version.check(true).map_err(|_| {
            Error::conflict("Source changed before the edit; no commands were applied")
        })?;
        let output = new_destination(output)?;
        let published = self.published()?;
        let document = self.document()?;
        let batch = pm_mcp::commands::plan_with_definitions(
            &document,
            published.selection.layers(),
            args["commands"]
                .as_array()
                .ok_or_else(|| Error::invalid("commands must be an array"))?,
            0.,
            Some(&self.kernel.state()),
        )?;
        let label = args["label"]
            .as_str()
            .unwrap_or("Agent composition edit")
            .trim();
        let request = Request::new(
            Caller::realm(1, 1, Origin::Mcp),
            Call::Transact {
                label: Some(label.into()),
                body: WriteBody::Commands(batch.commands),
                expected_revision: Some(published.revision()),
            },
        );
        let ack = self
            .kernel
            .call(&self.slot, request)
            .wait()
            .ok_or_else(|| error("kernel_error", "Actor stopped"))?
            .map_err(Error::kernel)?;
        if let Reply::Throw(ref failure) = ack.reply {
            return Err(Error::kernel(failure));
        }
        self.applied_unsaved = true;
        let session = self
            .session
            .clone()
            .ok_or_else(|| error("kernel_error", "Missing edit session"))?;
        let saved = self
            .kernel
            .shell(ShellCall::SaveAs {
                session,
                path: output,
            })
            .wait()
            .ok_or_else(|| error("kernel_error", "Actor stopped during save"))?
            .map_err(Error::kernel)?;
        if !matches!(
            saved,
            ShellReply::Saved {
                recovery_copy: None,
                ..
            }
        ) {
            return Err(error(
                "kernel_error",
                format!("Save needs recovery handling: {saved:?}"),
            ));
        }
        let durable = self
            .kernel
            .shell(ShellCall::DurabilityBarrier)
            .wait()
            .ok_or_else(|| error("kernel_error", "Actor stopped during durability barrier"))?
            .map_err(Error::kernel)?;
        if !matches!(durable, ShellReply::Durable) {
            return Err(error(
                "kernel_error",
                "Save did not reach its durability barrier",
            ));
        }
        self.applied_unsaved = false;
        let mut result = json!({"applied":batch.descriptions,"clip":null,"results":batch.results,"message":label,"revision":ack.revision});
        if let Some(notice) = ack.notice {
            result["notice"] = json!(format!("{notice:?}"));
        }
        Ok(result)
    }
    /// Container/media integrity is checked by open_project. This structural
    /// report deliberately does not claim the pending MCP creative lint body.
    pub fn validate(&self) -> Result<Value> {
        let p = self.document()?;
        let mut issues = vec![];
        if self.history_rejected {
            issues.push(json!({"code":"history_rejected","message":"Saved history was preserved but could not be replayed"}));
        }
        fn layers(value: &Value, issues: &mut Vec<Value>) {
            let Some(layers) = value["layers"].as_array() else {
                issues.push(json!({"code":"missing_layers"}));
                return;
            };
            let mut ids = std::collections::HashSet::new();
            for layer in layers {
                match layer["id"].as_str().filter(|id| !id.is_empty()) {
                    Some(id) if ids.insert(id) => (),
                    _ => issues.push(
                        json!({"code":"duplicate_or_missing_layer_id","layerId":layer["id"]}),
                    ),
                }
            }
            for layer in layers {
                for key in ["parent", "group", "matteSource"] {
                    if let Some(target) = layer[key].as_str().filter(|id| !id.is_empty())
                        && !ids.contains(target)
                    {
                        issues.push(json!({"code":"missing_layer_reference","layerId":layer["id"],"field":key,"target":target}));
                    }
                }
            }
        }
        layers(&p, &mut issues);
        if let Some(comps) = p["comps"].as_object() {
            for comp in comps.values() {
                layers(comp, &mut issues);
            }
        }
        Ok(
            json!({"ok":issues.is_empty(), "issues":issues,"scope":"container-media-integrity-and-layer-references", "creativeLintAvailable":false}),
        )
    }
    pub fn cache_dir(&self) -> &Path {
        self.scratch.as_ref().expect("live scratch").path()
    }
}
impl Drop for Project {
    fn drop(&mut self) {
        if let Some(session) = self.session.take() {
            let _ = self.kernel.shell(ShellCall::DeferClose(session)).wait();
        }
        if let Some(actor) = self.actor.take() {
            actor.shutdown();
        }
        if self.applied_unsaved
            && let Some(scratch) = self.scratch.take()
        {
            let recovery = scratch.keep();
            eprintln!("{{\"recoveryJournal\":{}}}", json!(recovery));
        }
    }
}
/// Never replace an authored destination. Resolve its parent, also catching
/// dangling symlinks which Path::exists would otherwise mistake for absence.
pub fn new_destination(path: &Path) -> Result<PathBuf> {
    let absolute = std::path::absolute(path)?;
    let parent = absolute
        .parent()
        .ok_or_else(|| Error::invalid("Destination requires a parent"))?
        .canonicalize()?;
    let name = absolute
        .file_name()
        .ok_or_else(|| Error::invalid("Destination requires a filename"))?;
    let path = parent.join(name);
    match std::fs::symlink_metadata(&path) {
        Ok(_) => Err(Error::invalid(
            "Destination already exists; choose a new path",
        )),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(path),
        Err(e) => Err(e.into()),
    }
}
