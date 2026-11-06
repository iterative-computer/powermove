//! Worker-side application adapter; typed plans and actual actor receipts only.
use pm_kernel::*;
use pm_panel_inspector::{
    EffectClipboard, Inspector,
    v4_effects::{EffectsPanel, Presentation},
};
use pm_surface::{
    host_api::*,
    host_data::{KernelData, QuerySnapshot},
};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU32, Ordering},
        mpsc::SyncSender,
    },
};
pub(crate) fn wire(value: &pm_model::Value) -> Value {
    serde_json::from_str(&pm_model::json::to_canonical_string(value)).expect("model JSON")
}
pub(crate) fn model(value: &Value) -> Result<pm_model::Value, String> {
    pm_model::json::parse(&value.to_string()).map_err(|e| format!("{e:?}"))
}
static NEXT_WRITER: AtomicU32 = AtomicU32::new(10_000);
struct Inflight {
    request: HostRequest,
    reply: SyncSender<HostAck>,
    pending: Pending<Result<Ack, KernelFailure>>,
    plan: Option<pm_panel_inspector::v4_effects::Plan>,
}
struct StateInflight {
    request: HostRequest,
    reply: SyncSender<HostAck>,
    pending: Pending<Result<state::StateAck, KernelFailure>>,
}
pub(crate) struct Backend {
    pub(crate) kernel: KernelHandle,
    pub(crate) identity: Identity,
    pub(crate) caller: Caller,
    pub(crate) model: Option<Inspector>,
    pub(crate) application: bool,
    /// The document session this worker was mounted on; a reopen never rebinds.
    document: Option<u64>,
    panel: EffectsPanel,
    extension: String,
    bindings: BTreeMap<String, Vec<String>>,
    binding_owners: BTreeMap<String, (u64, Arc<[String]>)>,
    binding_selection: Option<(u64, Arc<[String]>)>,
    pending: Vec<Inflight>,
    states: Vec<StateInflight>,
    retired: bool,
    pub(crate) native: super::models::Models,
    pub(crate) panel_id: String,
    pub(crate) service_revision: u64,
    timeline_waiter: Option<(HostRequest, SyncSender<HostAck>)>,
}
impl Backend {
    pub fn new(
        kernel: KernelHandle,
        identity: Identity,
        panel: &str,
        application: bool,
    ) -> Result<Self, String> {
        let published = if application {
            None
        } else {
            kernel.published(&identity.project)
        };
        if !application && published.is_none() {
            return Err("project closed".into());
        }
        let mut presentation = Presentation::default();
        for (id, label) in [
            ("shape", "Shape"),
            ("solid", "Solid"),
            ("text", "Text"),
            ("image", "Image"),
            ("video", "Video"),
            ("audio", "Audio"),
            ("group", "Group"),
            ("null", "Null"),
            ("precomp", "Composition"),
        ] {
            presentation
                .layer_type_labels
                .insert(id.into(), label.into());
        }
        let realm = NEXT_WRITER.fetch_add(1, Ordering::Relaxed);
        if realm == u32::MAX {
            return Err("writer identity exhausted".into());
        }
        let native = super::models::Models::new(
            kernel.clone(),
            &identity.project,
            Caller::realm(realm, 1, Origin::Interface),
            application,
        )?;
        let document = published.as_ref().map(|p| p.snapshot.doc_id);
        Ok(Self {
            kernel,
            identity,
            application,
            document,
            caller: Caller::realm(realm, 1, Origin::Interface),
            model: published.map(|p| Inspector::new(p, EffectClipboard::default())),
            panel: EffectsPanel::new(presentation, 0..96),
            extension: format!("ext.builtin.{panel}"),
            native,
            panel_id: panel.into(),
            service_revision: 0,
            timeline_waiter: None,
            bindings: BTreeMap::new(),
            binding_owners: BTreeMap::new(),
            binding_selection: None,
            pending: vec![],
            states: vec![],
            retired: false,
        })
    }
    pub(crate) fn published(&self) -> Option<Arc<Published>> {
        if self.application {
            None
        } else {
            self.kernel.published(&self.identity.project)
                .filter(|p| Some(p.snapshot.doc_id) == self.document)
        }
    }
    pub(crate) fn sync(&mut self) -> Result<(), String> {
        let published = self.published();
        if self.model.is_none() {
            let stamp = self.kernel.state().revision.to_string();
            if self.native.stamp != stamp {
                self.native.stamp = stamp;
                self.native.settings_sequence += 1;
                self.native.agent_sequence += 1;
            }
            return Ok(());
        }
        let published = published.ok_or("project closed")?;
        let stamp = format!(
            "{}:{}:{}:{}:{:?}",
            published.snapshot.doc_id,
            published.revision(),
            published.metadata_revision,
            self.kernel.state().revision,
            published.selection.primary()
        );
        if self.native.stamp != stamp {
            self.native.stamp = stamp;
            self.native.agent_sequence += 1;
            self.native.settings_sequence += 1;
            if !self.native.export.is_null() {
                self.native.export["sequence"] =
                    json!(self.native.export["sequence"].as_u64().unwrap_or(0) + 1);
            }
        }
        self.model
            .as_mut()
            .ok_or("No open project")?
            .sync(published);
        let state = self.kernel.state();
        let mut latest = BTreeMap::new();
        for registration in state.registries.get("effects").into_iter().flatten() {
            latest.insert(registration.id.clone(), wire(&registration.definition));
        }
        for id in self
            .model
            .as_mut()
            .ok_or("No open project")?
            .effect_definitions()
            .keys()
            .filter(|id| !latest.contains_key(*id))
            .cloned()
            .collect::<Vec<_>>()
        {
            self.model
                .as_mut()
                .ok_or("No open project")?
                .remove_definition(&id);
        }
        self.panel.presentation.parameters.clear();
        for (id, d) in latest {
            let mut params = vec![];
            for p in d["params"].as_array().into_iter().flatten() {
                let Some(key) = p["k"].as_str() else {
                    continue;
                };
                let kind = p["type"].as_str().unwrap_or("number");
                params.push(pm_panel_inspector::ParamDefinition {
                    key: key.into(),
                    kind: kind.into(),
                    default: model(p.get("def").unwrap_or(&Value::Null))?,
                    min: p["min"].as_f64(),
                    max: p["max"].as_f64(),
                    reference: matches!(kind, "layer" | "texture"),
                });
                if let Some(label) = p["label"].as_str() {
                    self.panel.presentation.parameters.insert(
                        (id.clone(), key.into()),
                        pm_panel_inspector::v4_effects::ParameterPresentation {
                            label: label.into(),
                            control: match kind {
                                "number" => "number",
                                "toggle" => "toggle",
                                "color" => "color",
                                "text" => "text",
                                "select" => "choice",
                                _ => "opaque",
                            }
                            .into(),
                            step: p["step"].as_f64(),
                            unit: p["unit"].as_str().map(str::to_owned),
                            precision: p["precision"].as_u64().map(|n| n as usize),
                            options: p["options"]
                                .as_array()
                                .map(|a| a.iter().map(model).collect())
                                .transpose()?
                                .unwrap_or_default(),
                        },
                    );
                }
            }
            self.model
                .as_mut()
                .ok_or("No open project")?
                .register_effect(pm_panel_inspector::EffectDefinition {
                    effect_type: id.clone(),
                    version: d["version"].as_u64().unwrap_or(1) as u32,
                    label: d["label"].as_str().unwrap_or(&id).into(),
                    params,
                    source_processing: d["sourceProcessing"].as_bool().unwrap_or(false),
                    backdrop: d["backdrop"].as_bool().unwrap_or(false),
                });
        }
        Ok(())
    }
    pub(crate) fn projection(&mut self) -> Result<Value, String> {
        self.sync()?;
        self.panel
            .project(
                self.model.as_mut().ok_or("No open project")?,
                self.native.facts["time"].as_f64().unwrap_or(0.),
            )
            .map(|v| wire(&v))
            .map_err(|e| e.to_string())
    }
    pub fn registry(&self) -> Value {
        let state = self.kernel.state();
        let mut items = BTreeMap::new();
        let mut groups = std::collections::BTreeSet::new();
        if let Some(registry) = state.registries.get("effects") {
            for entry in registry {
                let d = wire(&entry.definition);
                let group = d["group"].as_str().unwrap_or("");
                groups.insert(group.to_owned());
                items.insert(entry.id.clone(), json!({"id":entry.id,"label":d["label"].as_str().unwrap_or(&entry.id), "group":group, "kind":"effect","installed":true}));
            }
        }
        json!({"items":items.into_values().collect::<Vec<_>>(),"groups":groups})
    }
    pub fn target(&mut self) -> Result<Value, String> {
        self.sync()?;
        let Some(id) = self.model.as_ref().ok_or("No open project")?.primary() else {
            return Ok(Value::Null);
        };
        if !self.model.as_ref().ok_or("No open project")?.editable(id) {
            return Ok(Value::Null);
        }
        let layer = self
            .model
            .as_ref()
            .ok_or("No open project")?
            .layer(id)
            .map_err(|e| e.to_string())?;
        if layer.get("type").and_then(pm_model::Value::as_str) == Some("audio") {
            return Ok(Value::Null);
        }
        let fx = wire(&layer)["fx"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|f| f["type"].as_str().map(str::to_owned))
            .collect::<Vec<_>>();
        Ok(json!({"id":id,"effects":fx}))
    }
    pub(crate) fn validate_binding_owner(&self, handle: &str) -> Result<(), String> {
        let (document, selection) = self
            .binding_owners
            .get(handle)
            .ok_or("Unregistered binding")?;
        let inspector = self.model.as_ref().ok_or("No open project")?;
        if *document != inspector.document_id() || selection.as_ref() != inspector.selected_layers()
        {
            return Err("Binding selection changed".into());
        }
        Ok(())
    }
    pub(crate) fn parameter_write(
        &mut self,
        handle: &str,
        value: pm_model::Value,
        time: f64,
    ) -> Result<Request, String> {
        self.validate_binding_owner(handle)?;
        let channels = self
            .bindings
            .get(handle)
            .ok_or("Disposed effect binding")?
            .clone();
        let projection = self.projection()?;
        let row = projection["rows"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|r| {
                r["kind"] == "parameter"
                    && r["parameter"]["binding"] == handle
                    && r["parameter"]["readonly"] == false
                    && channels == [r["parameter"]["id"].as_str().unwrap_or("").to_owned()]
            })
            .ok_or("Effect binding no longer editable")?;
        let address = model(&row["parameter"]["id"])?;
        let property = pm_panel_inspector::PropertyId::from_value(
            &pm_model::json::parse(address.as_str().ok_or("Effect property id")?)
                .map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
        self.model
            .as_mut()
            .ok_or("No open project")?
            .set(
                self.caller,
                pm_panel_inspector::Binding::Property(property),
                value,
                time,
            )
            .map_err(|e| e.to_string())
    }
    pub fn local_fields(&mut self) {
        self.bindings.insert("effects.search".into(), vec![]);
        self.bindings.insert("effects.scope".into(), vec![]);
    }
    fn reply(
        &self,
        request: HostRequest,
        tx: SyncSender<HostAck>,
        result: Result<Value, String>,
    ) -> Result<(), String> {
        let revision = self
            .published()
            .map(|p| p.revision())
            .unwrap_or(request.expected_revision);
        let state_revision = request
            .expected_state_revision
            .map(|_| self.kernel.state().revision);
        let (value, error) = match result {
            Ok(v) => (Some(v), None),
            Err(e) => (None, Some(e)),
        };
        tx.try_send(HostAck {
            version: HOST_API_VERSION,
            realm: request.realm,
            generation: request.generation,
            project: request.project,
            request_id: request.request_id,
            revision,
            state_revision,
            accepted: error.is_none(),
            value,
            error,
        })
        .map_err(|e| e.to_string())
    }
    pub fn tick(&mut self) -> Result<(), String> {
        if let Some(result) = self.poll_timeline()
            && let Some((request, reply)) = self.timeline_waiter.take()
        {
            self.reply(request, reply, result)?;
        }
        let mut index = 0;
        while index < self.pending.len() {
            let Some(result) = self.pending[index].pending.try_take() else {
                index += 1;
                continue;
            };
            let item = self.pending.remove(index);
            self.sync()?;
            let result = result.map_err(|e| format!("{e:?}")).and_then(|ack| {
                if let Reply::Throw(error) = &ack.reply {
                    return Err(format!("{error:?}"));
                }
                if matches!(&ack.reply, Reply::Memory { .. })
                    && let Some(pm_kernel::Notice::NotDurable { message }) = &ack.notice
                {
                    return Err(format!("Notes remain applied but could not be saved: {message}. Read the latest notes before retrying."));
                }
                if let Some(plan) = item.plan {
                    self.panel
                        .acknowledge(self.model.as_mut().ok_or("No open project")?, plan, &ack)
                        .map_err(|e| e.to_string())?;
                }
                match &item.request.operation {
                    HostOperation::Command { command, .. } => self.command_reply(command),
                    _ => self.projection(),
                }
            });
            self.reply(item.request, item.reply, result)?;
        }
        let mut index = 0;
        while index < self.states.len() {
            let Some(result) = self.states[index].pending.try_take() else {
                index += 1;
                continue;
            };
            let item = self.states.remove(index);
            let result =
                result
                    .map_err(|e| format!("{e:?}"))
                    .and_then(|_| match &item.request.operation {
                        HostOperation::Command { command, .. } => self.command_reply(command),
                        _ => Ok(Value::Null),
                    });
            self.reply(item.request, item.reply, result)?;
        }
        Ok(())
    }
}
pub(crate) struct Data(pub Arc<Mutex<Backend>>);
impl KernelData for Data {
    fn query(
        &mut self,
        identity: &Identity,
        query: &Query,
        channels: Option<&[String]>,
        _: Option<f64>,
    ) -> Result<QuerySnapshot, String> {
        let mut b = self.0.lock().unwrap();
        if b.retired || identity != &b.identity {
            return Err("retired/foreign data service".into());
        }
        b.sync()?;
        let p = b.published();
        let revision = p.as_ref().map_or(0, |p| p.revision());
        let value = if let Some(value) = b.extra_query(query, channels)? {
            value
        } else {
            match query {
                Query::Evaluated if channels == Some(&["panel:layer-effects".into()][..]) => {
                    b.projection()?
                }
                Query::Project => {
                    let p = p.as_ref().ok_or("No open project")?;
                    json!({"project":p.project,"document":p.snapshot.doc_id.to_string(),"revision":p.revision()})
                }
                Query::Selection => {
                    let p = p.as_ref().ok_or("No open project")?;
                    json!({"primary":p.selection.primary(),"count":p.selection.layers().len()})
                }
                _ => return Err("unknown built-in query channel".into()),
            }
        };
        b.service_revision = revision;
        Ok(QuerySnapshot { revision, value })
    }
    fn enqueue(&mut self, request: HostRequest, reply: SyncSender<HostAck>) -> Result<(), String> {
        let mut b = self.0.lock().unwrap();
        if b.retired {
            return Err("built-in retired".into());
        }
        b.sync()?;
        let revision = b.published().as_ref().map_or(0, |p| p.revision());
        let needs_current = match &request.operation {
            HostOperation::Edit { .. } => true,
            HostOperation::Command { command, .. } => !matches!(
                command.as_str(),
                "profile.get" | "panel.agent.read" | "export.plan" | "panel.project"
            ),
            _ => false,
        };
        if needs_current && request.expected_revision != revision {
            return b.reply(
                request,
                reply,
                Err("Kernel changed; retry from its latest publication".into()),
            );
        }
        let mut plan = None;
        let mut kernel = None;
        let mut asynchronous = false;
        let result = (|| -> Result<Value, String> {
            match &request.operation {
                HostOperation::RegisterBinding { handle, channels } => {
                    let projection = b.projection()?;
                    let valid = projection["rows"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .any(|r| {
                            r["kind"] == "parameter"
                                && r["parameter"]["binding"] == *handle
                                && channels
                                    == &vec![r["parameter"]["id"].as_str().unwrap_or("").to_owned()]
                        });
                    let valid = valid || b.native.properties.valid(handle, channels);
                    if !valid {
                        return Err("unknown model binding".into());
                    }
                    let inspector = b.model.as_ref().ok_or("No open project")?;
                    if !b
                        .binding_selection
                        .as_ref()
                        .is_some_and(|(doc, selection)| {
                            *doc == inspector.document_id()
                                && selection.as_ref() == inspector.selected_layers()
                        })
                    {
                        b.binding_selection =
                            Some((inspector.document_id(), inspector.selected_layers().into()));
                    }
                    let owner = b
                        .binding_selection
                        .as_ref()
                        .ok_or("No binding selection")?
                        .clone();
                    b.binding_owners.insert(handle.clone(), owner);
                    b.bindings.insert(handle.clone(), channels.clone());
                }
                HostOperation::DisposeBinding { handle } => {
                    b.bindings.remove(handle);
                    b.binding_owners.remove(handle);
                }
                HostOperation::GetSetting { key } => {
                    let state = b.kernel.state();
                    return Ok(state
                        .settings
                        .get(&b.extension)
                        .and_then(|v| v.get(key))
                        .map(wire)
                        .unwrap_or(Value::Null));
                }
                HostOperation::SetSetting { key, value } => {
                    let state = b.kernel.state();
                    if request.expected_state_revision != Some(state.revision) {
                        return Err("Kernel settings changed".into());
                    }
                    let mut storage = state
                        .settings
                        .get(&b.extension)
                        .cloned()
                        .unwrap_or(pm_model::Value::Object(Default::default()));
                    storage
                        .as_object_mut()
                        .ok_or("extension storage is not an object")?
                        .insert(key.clone(), model(value)?);
                    let pending = b.kernel.state_transaction(state::StateTransaction {
                        expected_revision: Some(state.revision),
                        ops: vec![state::StateOp::Setting {
                            key: b.extension.clone(),
                            value: Some(storage),
                        }],
                    });
                    b.states.push(StateInflight {
                        request: request.clone(),
                        reply: reply.clone(),
                        pending,
                    });
                    asynchronous = true;
                }
                HostOperation::Edit { label, commands } => {
                    let mut all = vec![];
                    for command in commands {
                        if command.command != "effect.add" {
                            return Err("Unregistered built-in edit".into());
                        }
                        let target = command.args["target"].as_str().ok_or("effect target")?;
                        if b.model.as_ref().ok_or("No open project")?.primary() != Some(target) {
                            return Err("Effect target selection changed".into());
                        }
                        let definition = command.args["definition"]
                            .as_str()
                            .ok_or("effect definition")?;
                        let caller = b.caller;
                        let request = b
                            .model
                            .as_mut()
                            .ok_or("No open project")?
                            .add_effect(caller, target, definition)
                            .map_err(|e| e.to_string())?;
                        if let pm_kernel::Call::Transact {
                            body: pm_kernel::WriteBody::Commands(commands),
                            ..
                        } = request.call
                        {
                            all.extend(commands);
                        }
                    }
                    kernel = Some(pm_kernel::Request::new(
                        b.caller,
                        pm_kernel::Call::Transact {
                            label: Some(label.clone()),
                            body: pm_kernel::WriteBody::Commands(all),
                            expected_revision: Some(request.expected_revision),
                        },
                    ));
                }
                HostOperation::Command { command, args } if command == "panel.effect-controls" => {
                    let caller = b.caller;
                    let time = b.time();
                    let Backend {
                        panel, model: m, ..
                    } = &mut *b;
                    let mut next = panel
                        .plan(
                            m.as_mut().ok_or("No open project")?,
                            caller,
                            &model(args)?,
                            time,
                        )
                        .map_err(|e| e.to_string())?;
                    kernel = next.request.take();
                    plan = Some(next);
                }
                HostOperation::Command { command, args } => {
                    match b.native_command(command, args)? {
                        super::models::NativeCommand::Value(value) => return Ok(value),
                        super::models::NativeCommand::Kernel(request) => kernel = Some(request),
                        super::models::NativeCommand::State(tx) => {
                            let pending = b.kernel.state_transaction(tx);
                            b.states.push(StateInflight {
                                request: request.clone(),
                                reply: reply.clone(),
                                pending,
                            });
                            asynchronous = true;
                        }
                        super::models::NativeCommand::Timeline(intent) => {
                            b.start_timeline(intent)?;
                            b.timeline_waiter = Some((request.clone(), reply.clone()));
                            asynchronous = true;
                        }
                    }
                }
                _ => return Err("unregistered built-in host operation".into()),
            }
            Ok(Value::Null)
        })();
        if let Some(mut next) = kernel {
            next.txid = Some(request.request_id);
            if let Call::Transact {
                expected_revision, ..
            } = &mut next.call
            {
                *expected_revision = Some(request.expected_revision);
            }
            let pending = b.kernel.call(&b.identity.project, next);
            b.pending.push(Inflight {
                request,
                reply,
                pending,
                plan,
            });
            return Ok(());
        }
        if asynchronous {
            return Ok(());
        }
        b.reply(request, reply, result)
    }
    fn retire(&mut self, identity: &Identity) -> Result<(), String> {
        let mut b = self.0.lock().unwrap();
        if &b.identity != identity {
            return Err("foreign retirement".into());
        }
        if !b.retired {
            drop(
                b.kernel
                    .call(&identity.project, Request::new(b.caller, Call::RealmFailed)),
            );
            b.bindings.clear();
            b.binding_owners.clear();
            b.binding_selection = None;
            b.retired = true;
        }
        Ok(())
    }
}

pub(crate) fn kind_label(kind: &str) -> &'static str {
    match kind {
        "shape" => "Shape",
        "solid" => "Solid",
        "text" => "Text",
        "audio" => "Audio",
        "video" => "Video",
        "image" => "Image",
        "group" => "Group",
        "null" => "Null",
        "precomp" => "Composition",
        "camera" => "Camera",
        _ => "Layer",
    }
}
