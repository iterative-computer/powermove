//! A realm's kernel channel on the shared actor. Reads never serialize the
//! document: a reply carries a document only when its revision moved, and then
//! as operations against the last one sent, or whole when those can't express it.
use pm_kernel::{
    Ack, Call, Caller, Direction, EditCall, HistoryCall, KernelHandle, Origin, Published,
    QueryMirror, Reply, Request, SelectionCall, WorkspaceCall, WriteBody, Writer,
    state::{StateOp, StateTransaction},
};
use pm_model::{Op, PathPart, canonical};
use pm_runtime_v8::{Error, Result};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

fn model(value: &Value) -> pm_model::Value {
    pm_model::json::parse(&value.to_string()).expect("JSON model input")
}
fn wire(value: &pm_model::Value) -> Value {
    serde_json::from_str(&pm_model::json::to_canonical_string(value)).expect("JSON model output")
}
fn ops(args: &Value) -> Result<Vec<Op>> {
    args["ops"]
        .as_array()
        .ok_or(Error::Invalid("operation array"))?
        .iter()
        .map(|v| Op::from_value(&model(v)).map_err(|_| Error::Invalid("invalid operation")))
        .collect()
}
fn label(args: &Value) -> String {
    args["label"].as_str().unwrap_or("Edit").into()
}
fn realized(published: &Published) -> pm_model::Value {
    canonical::realize_project(
        published
            .snapshot
            .document
            .to_value()
            .unwrap_or(pm_model::Value::Null),
    )
}
/// A frozen document's text (an export's snapshot) as the realm's mirror
/// shows a project.
pub(crate) fn realized_wire(document: &str) -> std::result::Result<String, String> {
    let value =
        pm_model::json::parse(document).map_err(|_| "The export's project is unreadable")?;
    Ok(pm_model::json::to_canonical_string(
        &canonical::realize_project(value),
    ))
}
fn selection_wire(selection: &pm_kernel::Selection) -> Value {
    json!({"layers":selection.layers(),"keys":selection.keys(),"chan":selection.channel()})
}

/// Stored-tree operations as seen through the realized open composition.
/// `None`: this change moves the realized root, so the caller sends it whole.
fn realize_op(op: &Op, open: Option<&str>) -> Option<Value> {
    let map = |path: &[PathPart]| -> Option<Vec<PathPart>> {
        match path {
            [] => None,
            [PathPart::Key(root), rest @ ..] if root == "comps" => match rest {
                [] => None,
                [PathPart::Key(id), inner @ ..] if open.is_some_and(|o| id == o) => match inner {
                    [PathPart::Key(field), tail @ ..] => {
                        let key = if field == "name" {
                            "compName"
                        } else if field == "id" {
                            return None;
                        } else {
                            field
                                .as_str()
                                .filter(|f| canonical::COMP_FIELDS.contains(f))?
                        };
                        let mut out = vec![PathPart::from(key)];
                        out.extend_from_slice(tail);
                        Some(out)
                    }
                    _ => None,
                },
                _ => Some(path.to_vec()),
            },
            [PathPart::Key(root), ..] if root == "compId" => None,
            _ => Some(path.to_vec()),
        }
    };
    let mut op = op.clone();
    match &mut op {
        Op::Leaf(patch) => patch.path = map(&patch.path)?,
        Op::Order { path, .. } | Op::Insert { path, .. } | Op::Delete { path, .. } => {
            *path = map(path)?
        }
    }
    Some(wire(&op.to_value()))
}

/// What the realm last saw.
struct Sent {
    document_id: u64,
    revision: u64,
    open: Option<String>,
    selection: Value,
    workspace: pm_model::Value,
}

pub struct KernelAdapter {
    kernel: KernelHandle,
    project: String,
    realm: String,
    caller: Caller,
    generation: u32,
    sent: Option<Sent>,
    /// The first publication as JSON text, built before the realm asks for it
    /// so that its cost is not JavaScript time.
    staged: Option<String>,
    handles: BTreeMap<String, u64>,
    pins: BTreeSet<String>,
    replies: BTreeMap<u64, (Value, Value)>,
    mutations: u64,
    mirror: Option<(u64, QueryMirror)>,
}
impl KernelAdapter {
    /// `writer` is the kernel identity of this realm's run: a rebuilt realm is a
    /// new generation, so its transaction ids can never meet an old one's. The
    /// realm itself always speaks as generation 1 of its own short life.
    pub fn new(kernel: KernelHandle, project: &str, realm: &str, writer: Writer) -> Self {
        Self {
            kernel,
            project: project.into(),
            realm: realm.into(),
            caller: Caller::realm(writer.realm, writer.generation, Origin::Extension),
            generation: 1,
            sent: None,
            staged: None,
            handles: BTreeMap::new(),
            pins: BTreeSet::new(),
            replies: BTreeMap::new(),
            mutations: 0,
            mirror: None,
        }
    }
    fn published(&self) -> Result<Arc<Published>> {
        self.kernel
            .published(&self.project)
            .ok_or_else(|| Error::Host("project closed".into()))
    }
    fn facts(p: &Published) -> Value {
        json!({"historyBusy":p.entry.is_some(),"canUndo":p.history.can_undo,"canRedo":p.history.can_redo,
            "editLive":p.entry.as_ref().is_some_and(|e|e.source_edit_live),"label":p.history.undo_label})
    }
    /// The whole first publication. Later ones go through `publication`.
    /// `ext.*` settings as the realm's storage object: durable, and already the
    /// keys the shipped app wrote.
    pub fn storage(&self) -> Value {
        let state = self.kernel.state();
        let values = state
            .settings
            .iter()
            .filter(|(key, _)| key.starts_with("ext."))
            .map(|(key, value)| (key.clone(), wire(value)));
        Value::Object(values.collect())
    }
    pub fn store(&self, key: &str, value: &Value) -> Result<()> {
        let value = pm_model::json::parse(&value.to_string())
            .map_err(|_| Error::Invalid("storage value"))?;
        self.kernel
            .state_transaction(StateTransaction {
                expected_revision: None,
                ops: vec![StateOp::Setting {
                    key: key.to_owned(),
                    value: Some(value),
                }],
            })
            .wait()
            .ok_or(Error::Closed)?
            .map(|_| ())
            .map_err(|error| Error::Host(error.to_string()))
    }
    pub fn snapshot(&mut self) -> Result<Value> {
        let p = self.published()?;
        let document = realized(&p);
        let selection = selection_wire(&p.selection);
        let publication = json!({"revision":p.revision(),"document":wire(&document),
            "selection":selection,"workspace":wire(&p.workspace),"facts":Self::facts(&p)});
        self.sent = Some(Sent {
            document_id: p.snapshot.doc_id,
            revision: p.revision(),
            open: p.snapshot.open_comp(),
            selection,
            workspace: p.workspace.clone(),
        });
        Ok(publication)
    }
    fn document_delta(&self, p: &Published, sent: &Sent) -> Option<Vec<Value>> {
        let open = p.snapshot.open_comp();
        if sent.document_id != p.snapshot.doc_id || sent.open != open {
            return None;
        }
        let mut revision = sent.revision;
        let mut out = vec![];
        for change in p.changes.iter().filter(|c| c.revision > sent.revision) {
            if change.base_revision != revision || change.doc_id != p.snapshot.doc_id {
                return None;
            }
            revision = change.revision;
            for op in change.ops.iter() {
                out.push(realize_op(op, open.as_deref())?);
            }
        }
        (revision == p.revision()).then_some(out)
    }
    /// Builds the first publication now; the realm's first `publication` takes it.
    /// The document goes straight from the model to text: a pro project is
    /// hundreds of megabytes of JSON, and a parsed copy of it on the way
    /// costs gigabytes the allocator keeps.
    pub fn stage(&mut self) -> Result<()> {
        let p = self.published()?;
        let document = pm_model::json::to_canonical_string(&realized(&p));
        let selection = selection_wire(&p.selection);
        let rest = json!({"revision":p.revision(),"selection":selection,
            "workspace":wire(&p.workspace),"facts":Self::facts(&p)})
        .to_string();
        let mut text = String::with_capacity(document.len() + rest.len() + 16);
        text.push_str("{\"document\":");
        text.push_str(&document);
        text.push(',');
        text.push_str(&rest[1..]);
        drop(document);
        self.sent = Some(Sent {
            document_id: p.snapshot.doc_id,
            revision: p.revision(),
            open: p.snapshot.open_comp(),
            selection,
            workspace: p.workspace.clone(),
        });
        self.staged = Some(text);
        Ok(())
    }
    pub fn take_staged(&mut self) -> Option<String> {
        self.staged.take()
    }
    /// Facts always; document, selection and workspace only when they moved.
    pub fn publication(&mut self) -> Result<Value> {
        let p = self.published()?;
        let Some(sent) = self.sent.take() else {
            return self.snapshot();
        };
        let mut out = json!({"revision":p.revision(),"facts":Self::facts(&p)});
        let mut events = vec![];
        let selection = selection_wire(&p.selection);
        let moved = sent.revision != p.revision() || sent.document_id != p.snapshot.doc_id;
        if moved {
            match self.document_delta(&p, &sent) {
                Some(delta) => out["ops"] = json!(delta),
                None => out["document"] = wire(&realized(&p)),
            }
            events.push(json!({"name":"project:changed","value":{"kind":"values"}}));
        }
        if selection != sent.selection {
            out["selection"] = selection.clone();
            events.push(json!({"name":"sel"}));
        }
        if p.workspace != sent.workspace {
            out["workspace"] = wire(&p.workspace);
            events.push(json!({"name":"workspace:changed"}));
        }
        if !events.is_empty() {
            out["events"] = json!(events);
        }
        self.sent = Some(Sent {
            document_id: p.snapshot.doc_id,
            revision: p.revision(),
            open: p.snapshot.open_comp(),
            selection,
            workspace: p.workspace.clone(),
        });
        Ok(out)
    }
    pub fn pin(&mut self, handle: &str, pin: bool) {
        if pin {
            self.pins.insert(handle.into());
        } else {
            self.pins.remove(handle);
        }
    }
    /// Revision and timing facts for a transport without a document copy.
    /// The published document and the composition the viewer renders.
    pub fn document(&self) -> Result<(Arc<pm_model::Document>, String)> {
        let p = self.published()?;
        Ok((p.snapshot.document.clone(), p.snapshot.render_comp()))
    }
    /// The open composition's width and height.
    pub fn size(&self) -> Result<(f64, f64)> {
        let p = self.published()?;
        let root = p.snapshot.root().ok_or(Error::Invalid("no open project"))?;
        let open = p.snapshot.open_comp();
        let comp = pm_kernel::model::comp_node(root, open.as_deref()).unwrap_or(root);
        let number = |key: &str, default: f64| {
            comp.get(key)
                .and_then(|n| n.to_value().as_f64())
                .unwrap_or(default)
        };
        Ok((number("w", 1920.), number("h", 1080.)))
    }
    pub fn timebase(&self) -> Result<crate::transport::TimeBase> {
        let p = self.published()?;
        Ok(crate::transport::TimeBase::of(&p))
    }
    fn execute(&mut self, txid: u64, call: Call) -> Result<Value> {
        let request = Request::new(self.caller, call).with_txid(txid);
        let ack = self
            .kernel
            .call_blocking(&self.project, request)
            .map_err(|e| Error::Host(e.to_string()))?;
        self.reply(ack)
    }
    /// A later step of one composite request. The kernel keeps one acknowledgement
    /// per txid, so only the first step carries it; `replies` makes the whole
    /// request idempotent.
    fn step(&mut self, call: Call) -> Result<Value> {
        let ack = self
            .kernel
            .call_blocking(&self.project, Request::new(self.caller, call))
            .map_err(|e| Error::Host(e.to_string()))?;
        self.reply(ack)
    }
    /// This realm is gone: its open entry records whole, its later calls are
    /// refused and its external entries become undo barriers.
    pub fn seal(&self) {
        let request = Request::new(self.caller, Call::RealmFailed);
        let _ = self.kernel.call_blocking(&self.project, request);
    }
    fn reply(&mut self, ack: Ack) -> Result<Value> {
        let mut result = match ack.reply {
            Reply::Unit | Reply::Applied { .. } => json!({}),
            Reply::Selection(selection) => json!({"value":selection_wire(&selection)}),
            Reply::Bool(value) => json!({"value":value}),
            Reply::Before(value) => {
                json!({"value":value.map(|s|pm_model::json::to_canonical_string(&canonical::realize_project(s.document.to_value().unwrap_or(pm_model::Value::Null))))})
            }
            Reply::Entry(value) => json!({"value":value.map(|n|n.to_string())}),
            Reply::Edit(value) => {
                json!({"value":{"ok":value.ok,"message":value.message.unwrap_or_default()}})
            }
            Reply::Workspace(value) => json!({"value":wire(&value)}),
            Reply::Continuation(c) => self.continuation(&c)?,
            Reply::Throw(error) => json!({"error":{"code":"Rejected","message":error.to_string()}}),
            _ => return Err(Error::Invalid("unsupported kernel reply")),
        };
        result["publication"] = self.publication()?;
        Ok(result)
    }
    pub fn handle_name(&self, number: u64) -> Option<String> {
        self.handles
            .iter()
            .find(|(_, n)| **n == number)
            .map(|(h, _)| h.clone())
    }
    /// Reply form of an external closure the realm must run.
    pub fn continuation(&self, c: &pm_kernel::Continuation) -> Result<Value> {
        let handle = self
            .handle_name(c.handle)
            .ok_or(Error::Invalid("unknown external handle"))?;
        Ok(
            json!({"continuation":{"handle":handle,"phase":match c.direction {Direction::Undo=>"undo",Direction::Redo=>"redo"},"token":c.ticket.to_string()}}),
        )
    }
    pub fn call(&mut self, request: &Value, time: f64) -> Result<Value> {
        if request["realm"] != self.realm.as_str() || request["generation"] != self.generation {
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
        let reply = match self.call_uncached(request, txid, time) {
            Ok(value) => value,
            Err(error) => {
                json!({"error":{"code":"Unavailable","message":error.to_string()},"publication":self.publication()?})
            }
        };
        // A lost acknowledgement retries only the newest requests.
        if self.replies.len() >= 256 {
            self.replies.pop_first();
        }
        self.replies.insert(txid, (request.clone(), reply.clone()));
        Ok(reply)
    }
    fn plan(
        &mut self,
        args: &Value,
        p: &Published,
        time: f64,
    ) -> Result<Option<Vec<pm_kernel::Command>>> {
        let values = if let Some(a) = args["commands"].as_array() {
            a.clone()
        } else {
            vec![args["commands"].clone()]
        };
        let document = wire(&realized(p));
        let state = self.kernel.state();
        match pm_mcp::commands::plan_with_definitions(
            &document,
            p.selection.layers(),
            &values,
            time,
            Some(&state),
        ) {
            Ok(batch) => Ok(Some(batch.commands)),
            Err(error) => Err(Error::Host(error.message)),
        }
    }
    fn select(&mut self, selection: &Value) -> Result<Option<Value>> {
        let strings = |key: &str| -> Vec<String> {
            selection[key]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|v| v.as_str().map(str::to_owned))
                .collect()
        };
        for call in [
            SelectionCall::Layers {
                ids: strings("layers"),
                add: false,
            },
            SelectionCall::Keys(strings("keys")),
            SelectionCall::Channel(selection["chan"].as_str().map(str::to_owned)),
        ] {
            let ack = self
                .kernel
                .call_blocking(&self.project, Request::new(self.caller, call))
                .map_err(|e| Error::Host(e.to_string()))?;
            if let Reply::Throw(error) = ack.reply {
                return Ok(Some(
                    json!({"error":{"code":"Rejected","message":error.to_string()},"publication":self.publication()?}),
                ));
            }
        }
        Ok(None)
    }
    /// Raw `PM.proj` writes are expressed on the realized document.
    fn direct_write(&self, args: &Value, p: &Published) -> Result<Call> {
        let requested = ops(args)?;
        let open = p.snapshot.open_comp();
        let mapped: Option<Vec<Op>> = requested
            .iter()
            .map(|op| {
                let path = op.path();
                if path.is_empty()
                    || matches!(&path[0], PathPart::Key(k) if k == "comps" || k == "compId")
                {
                    return None;
                }
                let target = canonical::canonical_path(path, open.as_deref());
                let mut out = op.clone();
                match &mut out {
                    Op::Leaf(patch) => patch.path = target,
                    Op::Order { path, .. } | Op::Insert { path, .. } | Op::Delete { path, .. } => {
                        *path = target
                    }
                }
                Some(out)
            })
            .collect();
        if let Some(ops) = mapped {
            return Ok(Call::DirectWrite { ops });
        }
        // Root replacements and writes through `comps`: rebuild the stored form.
        let before = p
            .snapshot
            .document
            .to_value()
            .ok_or(Error::Invalid("empty document"))?;
        let mut after = Some(canonical::realize_project(before.clone()));
        pm_model::ops::apply_ops(&mut after, &requested)
            .map_err(|_| Error::Invalid("invalid stored-project operation"))?;
        let canonical = after.map(pm_kernel::model::to_canonical);
        Ok(Call::DirectWrite {
            ops: pm_model::ops::diff_ops(Some(&before), canonical.as_ref()),
        })
    }
    fn call_uncached(&mut self, request: &Value, txid: u64, time: f64) -> Result<Value> {
        let args = &request["args"];
        let method = request["method"]
            .as_str()
            .ok_or(Error::Invalid("kernel method"))?;
        let p = self.published()?;
        let read = match method {
            "history.busy" => Some(json!(p.entry.is_some())),
            "history.canUndo" => Some(json!(p.history.can_undo)),
            "history.canRedo" => Some(json!(p.history.can_redo)),
            "history.label" => Some(json!(p.history.undo_label)),
            "history.traceState" => Some(json!({"depth":p.entry.as_ref().map_or(0,|e|e.depth),
                "index":p.history.applied as i64-1,"pending":p.entry.as_ref().map(|e|json!({"label":e.label})),
                "entries":vec![json!({}); p.history.entries],"totalBytes":0})),
            "workspace.prepare" | "document.touch" => Some(Value::Null),
            "edit.admitMutation" => {
                let live = p.entry.as_ref().is_some_and(|e| e.source_edit_live);
                Some(
                    json!({"ok":!live,"message":if live {"A source edit is already active"} else {""}}),
                )
            }
            _ => None,
        };
        if let Some(value) = read {
            return Ok(json!({"value":value,"publication":self.publication()?}));
        }
        let call: Call = match method {
            "document.directWrite" => self.direct_write(args, &p)?,
            "history.begin" => HistoryCall::Begin { label: label(args) }.into(),
            "history.commit" => HistoryCall::Commit {
                label: args["label"].as_str().map(str::to_owned),
            }
            .into(),
            "history.cancel" => HistoryCall::Cancel.into(),
            "history.rollback" => HistoryCall::Rollback.into(),
            "history.selection" => {
                let side = |key: &str| {
                    let strings = |field: &str| {
                        args[key][field]
                            .as_array()
                            .into_iter()
                            .flatten()
                            .filter_map(|v| v.as_str().map(str::to_owned))
                            .collect()
                    };
                    pm_kernel::Selection::from_parts(
                        strings("layers"),
                        strings("keys"),
                        args[key]["chan"].as_str().map(str::to_owned),
                    )
                };
                HistoryCall::Selection {
                    before: side("before"),
                    after: side("after"),
                }
                .into()
            }
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
            "edit.mutateBegin" => {
                // A lease is the open entry: nested synchronous writes fold into it.
                self.mutations += 1;
                let lease = self.mutations;
                let mut reply =
                    self.execute(txid, HistoryCall::Begin { label: label(args) }.into())?;
                if reply.get("error").is_none() {
                    reply["value"] = json!({"ok":true,"message":"","data":{"lease":lease}});
                }
                return Ok(reply);
            }
            "edit.mutateFinish" => {
                let write = self.direct_write(args, &p)?;
                let written = self.execute(txid, write)?;
                if written.get("error").is_some() {
                    let _ = self.step(HistoryCall::Rollback.into());
                    let message = written["error"]["message"].clone();
                    return Ok(
                        json!({"value":{"ok":false,"message":message},"publication":self.publication()?}),
                    );
                }
                let mut reply = self.step(
                    HistoryCall::Commit {
                        label: args["label"].as_str().map(str::to_owned),
                    }
                    .into(),
                )?;
                if reply.get("error").is_none() {
                    reply["value"] = json!({"ok":true,"message":""});
                }
                return Ok(reply);
            }
            "edit.mutateAbort" => HistoryCall::Rollback.into(),
            "edit.apply" | "edit.dispatch" => {
                let commands = match self.plan(args, &p, time) {
                    Ok(commands) => commands.unwrap_or_default(),
                    Err(Error::Host(message)) => {
                        return Ok(
                            json!({"value":{"ok":false,"message":message},"publication":self.publication()?}),
                        );
                    }
                    Err(error) => return Err(error),
                };
                let body = WriteBody::Commands(commands);
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
                return Ok(match self.select(&args["selection"])? {
                    Some(rejected) => rejected,
                    None => json!({"publication":self.publication()?}),
                });
            }
            "realm.failed" => Call::RealmFailed,
            _ => return Err(Error::Host(format!("kernel method not attached: {method}"))),
        };
        self.execute(txid, call)
    }
    /// Geometry answers from a query mirror at the published revision.
    pub fn query(&mut self, input: &Value) -> Result<Value> {
        let p = self.published()?;
        let mirror = match &mut self.mirror {
            Some(mirror) if mirror.0 == p.snapshot.doc_id => {
                mirror.1.sync(&p);
                &mut mirror.1
            }
            slot => &mut slot.insert((p.snapshot.doc_id, QueryMirror::new(&p))).1,
        };
        let args = &input["args"];
        let time = args["time"].as_f64().unwrap_or(0.);
        let open = p.snapshot.open_comp();
        match input["method"].as_str() {
            Some("engine.bounds") => {
                let layer = args["layer"].as_str().ok_or(Error::Invalid("layer id"))?;
                let bounds = mirror.queries().bounds(open.as_deref(), layer, time);
                Ok(bounds.ok().flatten().map_or(Value::Null, |b| {
                    json!({"x0":b.x0,"y0":b.y0,"x1":b.x1,"y1":b.y1,"w":b.w,"h":b.h,"ax":b.ax,"ay":b.ay})
                }))
            }
            Some("engine.pick") => {
                let hit = mirror.queries().pick(
                    open.as_deref(),
                    args["x"].as_f64().ok_or(Error::Invalid("pick x"))?,
                    args["y"].as_f64().ok_or(Error::Invalid("pick y"))?,
                    time,
                    &Default::default(),
                );
                Ok(hit.map_or(Value::Null, |hit| json!(hit.layer_id)))
            }
            method => Err(Error::Host(format!(
                "{} is not attached",
                method.unwrap_or("query")
            ))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pm_kernel::{KernelActor, KernelConfig, OpenSource};
    fn request(txid: u64, method: &str, args: Value) -> Value {
        json!({"realm":"compat","generation":1,"txid":txid,"method":method,"args":args})
    }
    fn adapter() -> (KernelActor, KernelAdapter) {
        let actor = KernelActor::spawn(KernelConfig::default());
        let initial = crate::initial();
        let handle = actor.handle();
        handle
            .open("compat", OpenSource::Untitled(model(&initial["document"])))
            .wait()
            .unwrap()
            .unwrap();
        let mut adapter = KernelAdapter::new(
            handle,
            "compat",
            "compat",
            Writer {
                realm: 2,
                generation: 1,
            },
        );
        adapter.snapshot().unwrap();
        (actor, adapter)
    }
    #[test]
    fn the_staged_first_publication_is_the_snapshot_as_text() {
        let (actor, mut adapter) = adapter();
        let snapshot = adapter.snapshot().unwrap();
        adapter.sent = None;
        adapter.stage().unwrap();
        let staged: Value = serde_json::from_str(&adapter.take_staged().unwrap()).unwrap();
        assert_eq!(staged, snapshot);
        assert!(adapter.sent.is_some(), "later publications are deltas");
        drop(actor);
    }
    #[test]
    fn reads_send_no_document_and_writes_send_only_their_operations() {
        let (actor, mut adapter) = adapter();
        let read = adapter
            .call(&request(1, "history.busy", json!({})), 0.)
            .unwrap();
        assert_eq!(read["value"], false);
        assert!(read["publication"].get("document").is_none());
        assert!(read["publication"].get("ops").is_none());
        adapter
            .call(
                &request(2, "history.begin", json!({"label":"Synthetic"})),
                0.,
            )
            .unwrap();
        let write = request(
            3,
            "document.directWrite",
            json!({"ops":[{"path":["markers"],"exists":true,"value":[{"id":"synthetic-marker","t":0,"name":"Synthetic"}]}]}),
        );
        let reply = adapter.call(&write, 0.).unwrap();
        assert!(reply["publication"].get("document").is_none());
        assert_eq!(
            reply["publication"]["ops"][0]["path"],
            json!(["markers"]),
            "stored comps path maps back to the realized field"
        );
        assert_eq!(adapter.call(&write, 0.).unwrap(), reply);
        let mut changed = write.clone();
        changed["args"]["ops"] = json!([]);
        assert!(adapter.call(&changed, 0.).is_err());
        adapter
            .call(&request(4, "history.commit", json!({})), 0.)
            .unwrap();
        let undo = adapter
            .call(&request(5, "history.undo", json!({})), 0.)
            .unwrap();
        assert_eq!(undo["value"], true);
        assert_eq!(undo["publication"]["ops"][0]["path"], json!(["markers"]));
        drop(adapter);
        actor.shutdown();
    }
    #[test]
    fn a_mutation_commits_its_entry_and_a_failed_one_rolls_back() {
        let (actor, mut adapter) = adapter();
        let begin = |txid| request(txid, "edit.mutateBegin", json!({"label":"Synthetic"}));
        let marker = json!({"ops":[{"path":["markers"],"exists":true,"value":[{"id":"synthetic-marker","t":0,"name":"Synthetic"}]}],"label":"Synthetic"});
        adapter.call(&begin(1), 0.).unwrap();
        let finish = request(2, "edit.mutateFinish", marker);
        let reply = adapter.call(&finish, 0.).unwrap();
        assert_eq!(reply["value"]["ok"], true);
        // The retry is the stored reply, not a second commit.
        assert_eq!(adapter.call(&finish, 0.).unwrap()["value"]["ok"], true);
        let busy = |adapter: &mut KernelAdapter, txid| {
            adapter
                .call(&request(txid, "history.busy", json!({})), 0.)
                .unwrap()["value"]
                .clone()
        };
        assert_eq!(busy(&mut adapter, 3), false, "the entry closed");
        let undo = adapter
            .call(&request(4, "history.undo", json!({})), 0.)
            .unwrap();
        assert_eq!(undo["value"], true);
        adapter.call(&begin(5), 0.).unwrap();
        let bad = json!({"ops":[{"path":["markers",0,"t"],"exists":true,"value":1}]});
        let reply = adapter
            .call(&request(6, "edit.mutateFinish", bad), 0.)
            .unwrap();
        assert_eq!(reply["value"]["ok"], false);
        assert_eq!(busy(&mut adapter, 7), false, "the failed entry rolled back");
        drop(adapter);
        actor.shutdown();
    }
    #[test]
    fn native_selection_guard_is_never_acknowledged_as_success() {
        let (actor, mut adapter) = adapter();
        let result = adapter
            .call(
                &request(
                    1,
                    "selection.set",
                    json!({"selection":{"layers":["synthetic-missing"],"keys":[],"chan":null}}),
                ),
                0.,
            )
            .unwrap();
        assert_eq!(result["error"]["code"], "Rejected");
        assert!(result["publication"].get("selection").is_none());
        drop(adapter);
        actor.shutdown();
    }
}
