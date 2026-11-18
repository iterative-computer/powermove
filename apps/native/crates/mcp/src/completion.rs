//! Private host receipts. Provider output never supplies rollback snapshots.
use crate::{Automation, Error, Result, commands};
use pm_kernel::Origin;
use serde_json::{Value, json};

pub(crate) struct Receipt {
    project: String,
    thread: Option<String>,
    document: Option<u64>,
    before: Option<pm_kernel::Snapshot>,
    expected: u64,
    pub(crate) changed: bool,
    pub(crate) unsafe_undo: bool,
    pub(crate) extensions: Vec<String>,
    pub(crate) extension_projects: std::collections::BTreeMap<String, String>,
    pub(crate) health_ids: Vec<String>,
    pub(crate) changes: std::collections::BTreeMap<String, String>,
    limits: Vec<String>,
}
/// Rows preserve the extension/panel pair from live registrations; ids are
/// never inferred from display names or change-set ids.
fn extension_results(
    changes: &std::collections::BTreeMap<String, String>,
    workspace: &Value,
) -> Vec<Value> {
    let mut rows = vec![];
    for (id, action) in changes {
        let record = workspace["extensions"]
            .as_array()
            .and_then(|items| items.iter().find(|item| item["id"] == *id));
        let name = record
            .and_then(|record| record["manifest"]["name"].as_str())
            .unwrap_or(id);
        let status = if action == "removed" {
            "removed"
        } else if record
            .is_some_and(|record| record["enabled"] == true && record["health"]["state"] == "ok")
        {
            "ready"
        } else {
            "error"
        };
        let panels = workspace["extensionPanels"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|pair| pair["extensionId"] == *id && action != "removed")
            .filter_map(|pair| pair["panelId"].as_str())
            .collect::<Vec<_>>();
        if panels.is_empty() {
            rows.push(json!({"extensionId":id,"panelId":null,"name":name,"action":action,"status":status}));
        } else {
            for panel in panels {
                rows.push(json!({"extensionId":id,"panelId":panel,"name":name,"action":action,"status":status}));
            }
        }
    }
    rows
}
fn document_scope(project: &str) -> Option<&str> {
    (!matches!(project, "" | "builtin-application" | "powermove-global")).then_some(project)
}
impl Automation {
    pub(crate) fn completion_receipt_mut(&mut self, run: &str) -> Option<&mut Receipt> {
        let root = self
            .completion_owners
            .get(run)
            .map(String::as_str)
            .unwrap_or(run);
        self.completions.get_mut(root)
    }
    pub(crate) fn completion_edit(
        &mut self,
        run: &str,
        before: u64,
        after: u64,
        interleaved: bool,
    ) {
        if let Some(receipt) = self.completion_receipt_mut(run) {
            receipt.unsafe_undo |= before != receipt.expected || interleaved;
            receipt.expected = after;
            receipt.changed |= before != after;
            if receipt.document.is_none() && before != after {
                receipt.unsafe_undo = true;
                let limit = "Composition edits from this app thread have no bound document receipt. Applied edits were preserved; use the editor Undo command.";
                if !receipt.limits.iter().any(|old| old == limit) {
                    receipt.limits.push(limit.into());
                }
            }
        }
    }
    /// Host-only API: never registered as an MCP tool or exposed to a provider.
    pub fn completion_action(
        &mut self,
        operation: &str,
        project: &str,
        run: &str,
        payload: &Value,
    ) -> Result<Value> {
        if operation == "bindChild" {
            let root = payload["rootRunId"]
                .as_str()
                .ok_or_else(|| Error::invalid("Missing parent receipt"))?;
            let receipt = self
                .completions
                .get(root)
                .ok_or_else(|| Error::conflict("Parent run recovery is unavailable"))?;
            let document = document_scope(project)
                .map(|project| self.original_view(project).map(|view| view.document))
                .transpose()?;
            if receipt.project != project || receipt.document != document {
                return Err(Error::conflict("The child belongs to another document"));
            }
            if self.completion_owners.len() >= 4096 {
                let receipt = self.completions.get_mut(root).expect("validated");
                receipt.limits.push("Delegated changes exceeded the automatic recovery receipt limit. Use the editor Undo command.".into());
                receipt.unsafe_undo = true;
                receipt.changed = true;
                return Err(Error::conflict("Child recovery receipt limit reached"));
            }
            self.completion_owners.insert(run.into(), root.into());
            return Ok(Value::Null);
        }
        if operation == "openPanel" {
            let extension = payload["extensionId"]
                .as_str()
                .ok_or_else(|| Error::invalid("Missing extension id"))?;
            let panel = payload["panelId"]
                .as_str()
                .ok_or_else(|| Error::invalid("Missing contributed panel id"))?;
            let surfaces = self
                .surfaces
                .as_ref()
                .ok_or_else(|| Error::unavailable("Live panel host"))?;
            let workspace =
                surfaces.call("get_workspace_state", document_scope(project), &json!({}))?;
            let pair_live = workspace["extensionPanels"]
                .as_array()
                .is_some_and(|pairs| {
                    pairs
                        .iter()
                        .any(|pair| pair["extensionId"] == extension && pair["panelId"] == panel)
                });
            let healthy = workspace["extensions"].as_array().is_some_and(|items| {
                items.iter().any(|item| {
                    item["id"] == extension
                        && item["enabled"] == true
                        && item["health"]["state"] == "ok"
                })
            });
            if !pair_live || !healthy {
                return Err(Error::conflict(
                    "This extension panel is no longer available.",
                ));
            }
            return surfaces.call(
                "open_panel",
                document_scope(project),
                &json!({"panelId":panel}),
            );
        }
        if operation == "keep" {
            self.completions.remove(run);
            self.completion_owners.retain(|_, owner| owner != run);
            return Ok(Value::Null);
        }
        if operation == "begin" {
            let thread = payload["threadId"].as_str().map(str::to_owned);
            // Sending the next task is an implicit Keep for this conversation.
            // Its archived rows and ordinary editor history remain available.
            if let Some(thread) = &thread {
                self.completions.retain(|id, receipt| {
                    id == run
                        || receipt.project != project
                        || receipt.thread.as_ref() != Some(thread)
                });
                let retained = &self.completions;
                self.completion_owners
                    .retain(|_, owner| retained.contains_key(owner));
            }
            if self.completions.len() >= 128 {
                return Err(Error::conflict(
                    "Too many retained recovery receipts. Keep or undo earlier results first.",
                ));
            }
            if self.completions.contains_key(run) {
                return Ok(Value::Null);
            }
            let view = document_scope(project)
                .map(|project| self.original_view(project))
                .transpose()?;
            let (document, before, expected) = match view {
                Some(view) => (Some(view.document), Some(view.snapshot), view.revision),
                None => (None, None, 0),
            };
            self.completions.insert(
                run.into(),
                Receipt {
                    project: project.into(),
                    thread,
                    document,
                    before,
                    expected,
                    changed: false,
                    unsafe_undo: false,
                    extensions: vec![],
                    extension_projects: Default::default(),
                    health_ids: vec![],
                    changes: Default::default(),
                    limits: vec![],
                },
            );
            return Ok(Value::Null);
        }
        if operation == "apply" {
            if document_scope(project).is_none() {
                return Err(Error::invalid(
                    "Open the originating project before applying a native plan.",
                ));
            }
            let revision = payload["revision"].as_u64().ok_or_else(|| Error::invalid("This plan needs a revision-bound native commands preview. Request a fresh plan."))?;
            if self.revision(project)? != revision {
                return Err(Error::conflict(
                    "The project changed after this plan. Request a fresh plan.",
                ));
            }
            let raw = payload["commands"].as_array().ok_or_else(|| {
                Error::invalid("This plan has no native editable commands. Request a native plan.")
            })?;
            let batch = commands::plan_with_definitions(
                &self.read_document(project)?,
                &self.view(project)?.selection.layers().to_vec(),
                raw,
                self.playhead(project),
                Some(&self.state()),
            )?;
            let receipt = self.completions.get(run).ok_or_else(|| {
                Error::conflict(
                    "This plan has no originating run receipt. Request a fresh native plan.",
                )
            })?;
            let document = document_scope(project)
                .map(|project| self.original_view(project).map(|view| view.document))
                .transpose()?;
            if receipt.project != project || receipt.document != document {
                return Err(Error::conflict(
                    "This plan belongs to another document. Nothing was applied.",
                ));
            }
            let ack = self.apply_commands(
                project,
                batch.commands,
                "Agent plan",
                Origin::Agent,
                revision,
                999,
            )?;
            self.completion_edit(run, revision, ack.revision, ack.folded.is_some());
            // The native batch is already one history transaction. Do not
            // squash across an active gesture owned by another editor.
            let mut result = json!({"runId":run,"summary":if ack.revision == revision { "This plan made no changes." } else { "Applied agent plan" },"changed":ack.revision != revision});
            if ack.folded.is_some() {
                result["verificationError"] = json!(
                    "The plan was applied alongside other edits. Use the editor Undo command for that shared history group."
                );
            }
            return Ok(result);
        }
        let receipt = self.completions.get(run).ok_or_else(|| {
            Error::conflict("This result has no recovery receipt. Use the editor Undo command.")
        })?;
        let view = document_scope(project)
            .map(|project| self.original_view(project))
            .transpose()?;
        let revision = view.as_ref().map_or(0, |view| view.revision);
        if receipt.project != project || receipt.document != view.as_ref().map(|view| view.document)
        {
            return Err(Error::conflict(
                "This result belongs to another document. Nothing was restored.",
            ));
        }
        match operation {
            "verify" => {
                let changed = receipt.changed || !receipt.extensions.is_empty();
                let mut errors = receipt
                    .limits
                    .iter()
                    .map(|limit| json!(limit))
                    .collect::<Vec<_>>();
                let mut images = vec![];
                let mut workspace_evidence = Value::Null;
                if changed {
                    if let Some(before) = &receipt.before {
                        match self.render(project, &json!({"width":480})) {
                            Ok(out) => {
                                images = out
                                .images
                                .into_iter()
                                .map(|image| json!({"data":image.data,"mimeType":image.mime_type}))
                                .collect();
                                if images.is_empty() {
                                    errors.push(json!("Final frame capture returned no images"));
                                }
                            }
                            Err(e) => errors.push(json!(format!("Final frames: {e}"))),
                        }
                        let document = self.read_document(project)?;
                        let original = crate::automation::document_json(before)?;
                        for (id, asset) in document["assets"].as_object().into_iter().flatten() {
                            if original["assets"][id] == *asset {
                                continue;
                            }
                            match &self.media_registry {
                                Some(registry) => {
                                    let state = registry.state(project, id);
                                    if !state.available || state.loading || state.error.is_some() {
                                        errors.push(json!(format!(
                                            "Imported asset {id}: {}",
                                            state.error.unwrap_or_else(|| "not ready".into())
                                        )));
                                    }
                                }
                                None => errors.push(json!(format!(
                                    "Imported asset {id}: health check unavailable"
                                ))),
                            }
                        }
                    } else if receipt.changed {
                        errors.push(json!("Composition verification is unavailable without a bound document. Applied edits were preserved."));
                    }
                    if !receipt.health_ids.is_empty() {
                        match &self.surfaces {
                            Some(surface) => {
                                match surface.call(
                                    "get_workspace_state",
                                    document_scope(project),
                                    &json!({}),
                                ) {
                                    Ok(workspace) => {
                                        workspace_evidence = workspace.clone();
                                        // Preserve real health/errors in the provider's review input.
                                        if workspace["errors"]
                                            .as_array()
                                            .is_some_and(|e| !e.is_empty())
                                        {
                                            errors.push(workspace["errors"].clone());
                                        }
                                        if workspace["extensions"]
                                            .as_array()
                                            .is_none_or(|items| items.is_empty())
                                        {
                                            errors.push(json!(
                                                "Live extension health was unavailable"
                                            ));
                                        } else {
                                            for id in &receipt.health_ids {
                                                match workspace["extensions"].as_array().unwrap().iter().find(|item| item["id"] == *id) {
                                                    Some(item) if item["enabled"] == true && item["health"]["state"] == "ok" => {},
                                                    Some(item) => errors.push(item.clone()),
                                                    None => errors.push(json!(format!("Extension {id} was not found after publication"))),
                                                }
                                            }
                                        }
                                    }
                                    Err(e) => errors.push(json!(format!("Extension health: {e}"))),
                                }
                            }
                            None => errors.push(json!("Live extension health check unavailable")),
                        }
                    }
                }
                if let Some(initial) = &view {
                    let latest = self.original_view(project)?;
                    if latest.document != initial.document {
                        return Err(Error::conflict(
                            "The originating document changed during final verification. Applied edits were preserved; retry from that project.",
                        ));
                    }
                    if changed && latest.revision != revision {
                        errors.push(json!("The composition changed during final verification. Applied edits were preserved; review the latest state."));
                    }
                }
                Ok(
                    json!({"changed":changed,"revision":revision,"errors":errors,"images":images,"extensionChangeSets":receipt.extensions,"extensions":extension_results(&receipt.changes,&workspace_evidence)}),
                )
            }
            "undo" => {
                if !receipt.limits.is_empty() {
                    return Err(Error::conflict(receipt.limits.join(" ")));
                }
                if receipt.unsafe_undo || receipt.expected != revision {
                    return Err(Error::conflict(
                        "Other edits occurred during or after this run. Nothing was restored. Use the editor Undo command to preserve that work.",
                    ));
                }
                let sets = receipt.extensions.clone();
                let before = receipt
                    .before
                    .as_ref()
                    .map(crate::automation::document_json)
                    .transpose()?;
                let journals = receipt.extension_projects.clone();
                let changed = receipt.changed;
                let expected = receipt.expected;
                // Extension restore checks the exact live tree and refuses newer edits.
                for set in sets.iter().rev() {
                    let host = self
                        .extensions
                        .as_ref()
                        .ok_or_else(|| Error::unavailable("Extension recovery"))?;
                    crate::extensions::restore(host.as_ref(), journals.get(set).map(String::as_str).unwrap_or(project), set).map_err(|mut error| {
                        if self.completions[run].extensions.len() < sets.len() {
                            error.message = format!("Some extension changes were restored before recovery stopped: {}. Project edits were preserved.",error.message);
                        }
                        error
                    })?;
                    self.completions.get_mut(run).unwrap().extensions.pop();
                }
                if changed {
                    let before = before.ok_or_else(|| {
                        Error::conflict(
                            "This run has no document checkpoint. Use the editor Undo command.",
                        )
                    })?;
                    self.restore(project, before, expected, 999).map_err(|mut error| {
                        if !sets.is_empty() { error.message = format!("Extension changes were restored, but project recovery stopped: {}. Later edits were preserved.",error.message); }
                        error
                    })?;
                }
                self.completions.remove(run);
                self.completion_owners.retain(|_, owner| owner != run);
                Ok(json!({"undone":true}))
            }
            _ => Err(Error::invalid("Unknown completion action")),
        }
    }
}
