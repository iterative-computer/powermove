//! Worker-owned, disposable project authority. Never opens the user's profile.
use pm_kernel::{
    Call, Caller, Direction, EditCall, HistoryCall, Kernel, KernelConfig, OpenSource, Origin,
    Reply, Request, SelectionCall, WorkspaceCall, WriteBody,
};
use pm_runtime_v8::{Error, Result};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

const PROJECT: &str = "extcheck";
pub struct KernelAdapter {
    kernel: Kernel,
    handles: BTreeMap<String, u64>,
    pins: BTreeSet<String>,
    replies: BTreeMap<u64, (Value, Value)>,
}
fn model(value: &Value) -> pm_model::Value {
    pm_model::json::parse(&value.to_string()).expect("JSON model input")
}
fn wire(value: &pm_model::Value) -> Value {
    serde_json::from_str(&pm_model::json::to_canonical_string(value)).expect("JSON model output")
}
fn ops(args: &Value) -> Result<Vec<pm_model::Op>> {
    args["ops"]
        .as_array()
        .ok_or(Error::Invalid("operation array"))?
        .iter()
        .map(|v| {
            pm_model::Op::from_value(&model(v)).map_err(|_| Error::Invalid("invalid operation"))
        })
        .collect()
}
fn label(args: &Value) -> String {
    args["label"].as_str().unwrap_or("Edit").into()
}
impl KernelAdapter {
    pub fn new(initial: &Value) -> Result<Self> {
        let mut kernel = Kernel::new(KernelConfig::default());
        kernel
            .open(PROJECT, OpenSource::Untitled(model(&initial["document"])))
            .map_err(|e| Error::Host(e.to_string()))?;
        kernel
            .project_mut(PROJECT)
            .unwrap()
            .set_workspace(model(&initial["workspace"]));
        Ok(Self {
            kernel,
            handles: BTreeMap::new(),
            pins: BTreeSet::new(),
            replies: BTreeMap::new(),
        })
    }
    pub fn publication(&self) -> Value {
        let p = self.kernel.project(PROJECT).unwrap();
        let store = p.store();
        let history = store.history();
        json!({"revision":store.revision(),"document":wire(&pm_model::canonical::realize_project(store.snapshot().document.to_value().unwrap())),
            "selection":{"layers":store.selection().layers(),"keys":store.selection().keys(),"chan":store.selection().channel()},
            "workspace":wire(p.workspace()),"facts":{"historyBusy":p.open_entry().is_some(),
            "canUndo":history.can_undo(),"canRedo":history.can_redo(),"editLive":p.source_edit_live(),"label":history.undo_label()}})
    }
    pub fn pin(&mut self, handle: &str, pin: bool) {
        if pin {
            self.pins.insert(handle.into());
        } else {
            self.pins.remove(handle);
        }
    }
    fn execute(&mut self, txid: u64, call: Call) -> Result<Value> {
        let request = Request::new(Caller::realm(1, 1, Origin::Extension), call).with_txid(txid);
        let handled = self
            .kernel
            .handle(PROJECT, request)
            .map_err(|e| Error::Host(e.to_string()))?;
        let ack = handled
            .reply
            .ok_or(Error::Invalid("unexpected cross-realm continuation"))?;
        let mut result = match ack.reply {
            Reply::Unit => json!({}),
            Reply::Applied { .. } => json!({}),
            Reply::Selection(selection) => {
                json!({"value":{"layers":selection.layers(),"keys":selection.keys(),"chan":selection.channel()}})
            }
            Reply::Bool(value) => json!({"value":value}),
            Reply::Before(value) => {
                json!({"value":value.map(|s|pm_model::json::to_canonical_string(&pm_model::canonical::realize_project(s.document.to_value().unwrap())))})
            }
            Reply::Entry(value) => json!({"value":value.map(|n|n.to_string())}),
            Reply::Edit(value) => {
                json!({"value":{"ok":value.ok,"message":value.message.unwrap_or_default()}})
            }
            Reply::Workspace(value) => json!({"value":wire(&value)}),
            Reply::Continuation(c) => {
                let handle = self
                    .handles
                    .iter()
                    .find(|(_, n)| **n == c.handle)
                    .map(|(h, _)| h)
                    .ok_or(Error::Invalid("unknown external handle"))?;
                json!({"continuation":{"handle":handle,"phase":match c.direction {Direction::Undo=>"undo",Direction::Redo=>"redo"},"token":c.ticket.to_string()}})
            }
            Reply::Throw(error) => json!({"error":{"code":"Rejected","message":error.to_string()}}),
            _ => return Err(Error::Invalid("unsupported kernel reply")),
        };
        let retired: Vec<_> = self
            .kernel
            .project_mut(PROJECT)
            .unwrap()
            .take_outbox()
            .into_iter()
            .filter_map(|m| match m {
                pm_kernel::RealmMessage::Cleanup { handle, .. } => self
                    .handles
                    .iter()
                    .find(|(_, n)| **n == handle)
                    .map(|(h, _)| h.clone()),
                _ => None,
            })
            .collect();
        result["publication"] = self.publication();
        if !retired.is_empty() {
            result["retired"] = json!(retired);
        }
        Ok(result)
    }
    pub fn call(&mut self, request: &Value) -> Result<Value> {
        if request["realm"] != "extcheck" || request["generation"] != 1 {
            return Err(Error::Invalid("kernel realm identity"));
        }
        let txid = request["txid"]
            .as_u64()
            .filter(|n| *n > 0)
            .ok_or(Error::Invalid("transaction id"))?;
        if let Some((original, reply)) = self.replies.get(&txid) {
            if original != request {
                return Err(Error::Invalid(
                    "transaction id reused with different request",
                ));
            }
            return Ok(reply.clone());
        }
        if self.replies.len() >= 4096 {
            return Err(Error::Quota("kernel acknowledgements"));
        }
        let before = self.publication();
        let mut result = match self.call_uncached(request, txid) {
            Ok(value) => value,
            Err(error) => {
                json!({"error":{"code":"Unavailable","message":error.to_string()},"publication":self.publication()})
            }
        };
        let after = &result["publication"];
        let mut events = vec![];
        if before["revision"] != after["revision"] {
            events.push(json!({"name":"project:changed","value":{"kind":"values"}}));
        }
        if before["selection"] != after["selection"] {
            events.push(json!({"name":"selection"}));
            events.push(json!({"name":"sel"}));
        }
        if before["workspace"] != after["workspace"] {
            events.push(json!({"name":"workspace:changed"}));
        }
        if !events.is_empty() {
            result["publication"]["events"] = json!(events);
        }
        self.replies.insert(txid, (request.clone(), result.clone()));
        Ok(result)
    }
    fn call_uncached(&mut self, request: &Value, txid: u64) -> Result<Value> {
        let args = &request["args"];
        let method = request["method"]
            .as_str()
            .ok_or(Error::Invalid("kernel method"))?;
        let p = self.kernel.project(PROJECT).unwrap();
        let read = match method {
            "history.busy" => Some(json!(p.open_entry().is_some())),
            "history.canUndo" => Some(json!(p.store().history().can_undo())),
            "history.canRedo" => Some(json!(p.store().history().can_redo())),
            "history.label" => Some(json!(p.store().history().undo_label())),
            "workspace.prepare" | "document.touch" => Some(Value::Null),
            "edit.admitMutation" => Some(
                json!({"ok":!p.source_edit_live(),"message":if p.source_edit_live(){"A source edit is already active"}else{""}}),
            ),
            _ => None,
        };
        if let Some(value) = read {
            return Ok(json!({"value":value,"publication":self.publication()}));
        }
        let call = match method {
            "document.directWrite" => {
                let before = p.store().snapshot().document.to_value().unwrap();
                let mut after = Some(pm_model::canonical::realize_project(before.clone()));
                pm_model::ops::apply_ops(&mut after, &ops(args)?)
                    .map_err(|_| Error::Invalid("invalid stored-project operation"))?;
                let canonical = after.map(pm_kernel::model::to_canonical);
                Call::DirectWrite {
                    ops: pm_model::ops::diff_ops(Some(&before), canonical.as_ref()),
                }
            }
            "history.begin" => HistoryCall::Begin { label: label(args) }.into(),
            "history.commit" => HistoryCall::Commit {
                label: args["label"].as_str().map(str::to_owned),
            }
            .into(),
            "history.cancel" => HistoryCall::Cancel.into(),
            "history.rollback" => HistoryCall::Rollback.into(),
            "history.undo" => HistoryCall::Undo.into(),
            "history.redo" => HistoryCall::Redo.into(),
            "history.clear" => HistoryCall::Clear.into(),
            "history.undoIfTop" => HistoryCall::UndoIfTop {
                entry: args["id"]
                    .as_str()
                    .and_then(|n| n.parse().ok())
                    .ok_or(Error::Invalid("history entry"))?,
            }
            .into(),
            "history.external" => {
                let handle = args["handle"]
                    .as_str()
                    .ok_or(Error::Invalid("external handle"))?;
                if !self.pins.contains(handle) {
                    return Err(Error::Invalid("unpinned external handle"));
                }
                let next = self.handles.len() as u64 + 1;
                let handle = *self.handles.entry(handle.into()).or_insert(next);
                HistoryCall::External {
                    label: label(args),
                    handle,
                    bytes: args["bytes"].as_u64().unwrap_or(256) as usize,
                    cleanup: args["cleanup"] == true,
                }
                .into()
            }
            "history.externalDone" => Call::ExternalDone {
                ticket: args["token"]
                    .as_str()
                    .and_then(|n| n.parse().ok())
                    .ok_or(Error::Invalid("external ticket"))?,
                ok: args["failed"] != true,
                message: args["message"].as_str().map(str::to_owned),
            },
            "edit.begin" => EditCall::Begin { label: label(args) }.into(),
            "edit.commit" => EditCall::Commit {
                label: args["label"].as_str().map(str::to_owned),
            }
            .into(),
            "edit.cancel" => EditCall::Cancel.into(),
            "edit.apply" | "edit.dispatch" => {
                let values = if let Some(a) = args["commands"].as_array() {
                    a.clone()
                } else {
                    vec![args["commands"].clone()]
                };
                let publication = self.publication();
                let selection: Vec<_> = p.store().selection().layers().to_vec();
                let batch = match pm_mcp::commands::plan(
                    &publication["document"],
                    &selection,
                    &values,
                    0.,
                ) {
                    Ok(batch) => batch,
                    Err(error) => {
                        return Ok(
                            json!({"value":{"ok":false,"message":error.message},"publication":publication}),
                        );
                    }
                };
                let body = WriteBody::Commands(batch.commands);
                if method == "edit.apply" {
                    EditCall::Apply {
                        body,
                        base_revision: args["meta"]["baseRevision"].as_u64(),
                        label: args["meta"]["label"].as_str().map(str::to_owned),
                    }
                    .into()
                } else {
                    EditCall::Dispatch { body }.into()
                }
            }
            "workspace.mutate" | "workspace.replace" => WorkspaceCall::Mutate {
                next: model(&args["next"]),
                threw: None,
            }
            .into(),
            "selection.set" => {
                // Each part is assigned by the native authority; no unvalidated mirror state.
                let selection = &args["selection"];
                let strings = |key: &str| -> Vec<String> {
                    selection[key]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(|v| v.as_str().map(str::to_owned))
                        .collect()
                };
                let calls = [
                    SelectionCall::Layers {
                        ids: strings("layers"),
                        add: false,
                    },
                    SelectionCall::Keys(strings("keys")),
                    SelectionCall::Channel(selection["chan"].as_str().map(str::to_owned)),
                ];
                // The outer immutable request cache deduplicates this owner turn.
                for call in calls {
                    let handled = self
                        .kernel
                        .handle(
                            PROJECT,
                            Request::new(Caller::realm(1, 1, Origin::Extension), call),
                        )
                        .map_err(|e| Error::Host(e.to_string()))?;
                    if let Some(pm_kernel::Ack {
                        reply: Reply::Throw(error),
                        ..
                    }) = handled.reply
                    {
                        return Ok(
                            json!({"error":{"code":"Rejected","message":error.to_string()},"publication":self.publication()}),
                        );
                    }
                }
                return Ok(json!({"publication":self.publication()}));
            }
            "realm.failed" => Call::RealmFailed,
            _ => return Err(Error::Host(format!("kernel method not attached: {method}"))),
        };
        self.execute(txid, call)
    }
    pub fn query(&mut self, input: &Value) -> Result<Value> {
        if input["method"] != "engine.query" {
            return Err(Error::Host("synchronous query adapter not attached".into()));
        }
        let result = self
            .kernel
            .project_mut(PROJECT)
            .unwrap()
            .store_mut()
            .queries()
            .query(&model(&input["args"]["request"]))
            .map_err(|e| Error::Host(e.to_string()))?;
        Ok(wire(&result))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn request(txid: u64, method: &str, args: Value) -> Value {
        json!({"realm":"extcheck","generation":1,"txid":txid,"method":method,"args":args})
    }
    #[test]
    fn retries_preserve_stored_composition_writes_and_reject_changed_requests() {
        let mut adapter = KernelAdapter::new(&crate::initial()).unwrap();
        adapter
            .call(&request(1, "history.begin", json!({"label":"Synthetic"})))
            .unwrap();
        let write = request(
            2,
            "document.directWrite",
            json!({"ops":[{"path":["markers"],"exists":true,"value":[{"id":"synthetic-marker","t":0,"name":"Synthetic"}]}]}),
        );
        let reply = adapter.call(&write).unwrap();
        assert_eq!(
            reply["publication"]["document"]["markers"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        assert_eq!(adapter.call(&write).unwrap(), reply);
        let mut changed = write.clone();
        changed["args"]["ops"] = json!([]);
        assert!(adapter.call(&changed).is_err());
        adapter
            .call(&request(3, "history.commit", json!({})))
            .unwrap();
        let undo = adapter
            .call(&request(4, "history.undo", json!({})))
            .unwrap();
        assert_eq!(undo["value"], true);
        assert_eq!(undo["publication"]["document"]["markers"], json!([]));
    }
    #[test]
    fn native_selection_guard_is_never_acknowledged_as_success() {
        let mut adapter = KernelAdapter::new(&crate::initial()).unwrap();
        let result = adapter
            .call(&request(
                1,
                "selection.set",
                json!({"selection":{"layers":["synthetic-missing"],"keys":[],"chan":null}}),
            ))
            .unwrap();
        assert_eq!(result["error"]["code"], "Rejected");
        assert_eq!(result["publication"]["selection"]["layers"], json!([]));
    }
}
