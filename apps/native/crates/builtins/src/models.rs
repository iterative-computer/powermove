//! Native model coordination. Snapshots contain presentation facts and leased
//! identifiers, never a writable JS project or a second persistence authority.
use crate::LockShared;
use crate::{
    data::{Backend, model, wire},
    properties::Properties,
};
use pm_kernel::*;
use pm_panel_media::{Catalog, readiness::ObservedRuntime, v4::ProjectBridge};
use pm_panel_timeline::{PropertyFacts, TimelineClient};
use pm_surface::host_api::Query;
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    future::Future,
    pin::Pin,
    task::{Context, Poll, Waker},
};

type TimelineJob = Pin<
    Box<
        dyn Future<
                Output = (
                    TimelineClient,
                    Result<Vec<pm_panel_timeline::HostEffect>, String>,
                ),
            > + Send,
    >,
>;
pub(crate) struct Models {
    pub properties: Properties,
    pub timeline: Option<TimelineClient>,
    /// Where the timeline client comes from, until a panel first asks for it:
    /// indexing every channel of a large project costs seconds, and only the
    /// Timeline panel reads it.
    timeline_source: Option<(KernelHandle, String, Caller)>,
    pub viewer: pm_panel_viewerchrome::v4::Session,
    viewer_projection: Option<(
        pm_panel_viewerchrome::Stamp,
        pm_panel_viewerchrome::Viewport,
        f64,
        Value,
    )>,
    pub project: ProjectBridge,
    pub facts: Value,
    pub fields: BTreeMap<String, String>,
    pub platform: Vec<(String, Value)>,
    /// Loaded for the Agent panel only; its load settles interrupted runs.
    pub agent: Option<crate::agent_host::SharedAgent>,
    pub agent_link: std::sync::Arc<crate::agent_host::AgentLink>,
    pub stamp: String,
    pub settings_sequence: u64,
    pub export: Value,
    pub project_reply: Option<Value>,
    pub media_runtime: ObservedRuntime,
    /// Moves with every new observation, so the Project panel asks for rows
    /// again when readiness lands without a document revision.
    pub media_sequence: u64,
    /// The status line asked the Project panel to list only missing files;
    /// applied on the panel's next request.
    pub show_missing: bool,
    pub originals: std::collections::BTreeMap<String, std::path::PathBuf>,
    pub export_probe_stamp: String,
    timeline_job: Option<TimelineJob>,
    pub(crate) timeline_effect_pending: usize,
    pub(crate) timeline_effect_result: Option<Result<(), String>>,
    timeline_editor: Option<TimelineEditor>,
}
impl Models {
    /// The timeline client, made on first use ([`Models::timeline_source`]).
    pub(crate) fn ensure_timeline(&mut self) -> Result<(), String> {
        if self.timeline.is_none()
            && let Some((kernel, project, caller)) = self.timeline_source.take()
        {
            self.timeline =
                Some(TimelineClient::new(kernel, &project, caller).map_err(|e| e.to_string())?);
        }
        Ok(())
    }
    pub fn new(
        kernel: KernelHandle,
        project: &str,
        caller: Caller,
        application: bool,
        loads_agent: bool,
    ) -> Result<Self, String> {
        let timeline_source = (!application).then(|| (kernel.clone(), project.to_owned(), caller));
        let agent = if loads_agent {
            let mut store = KernelStore::new(kernel, application);
            let panel = pm_panel_agentpanel::model::Panel::load(
                &mut store,
                project,
                &new_id("thread"),
                crate::agent_host::now_ms(),
            )
            .map_err(|e| format!("{e:?}"))?;
            Some(std::sync::Arc::new(std::sync::Mutex::new(
                crate::agent_host::AgentState::new(panel),
            )))
        } else {
            None
        };
        Ok(Self {
            properties: Properties::default(),
            timeline: None,
            timeline_source,
            viewer: Default::default(),
            viewer_projection: None,
            project: ProjectBridge::default(),
            facts: json!({"time":0.,"playing":false,"stage":[640.,360.],"light":false}),
            fields: BTreeMap::new(),
            platform: vec![],
            agent,
            agent_link: Default::default(),
            stamp: String::new(),
            settings_sequence: 1,
            export: Value::Null,
            timeline_job: None,
            timeline_effect_pending: 0,
            timeline_effect_result: None,
            timeline_editor: None,
            project_reply: None,
            media_runtime: Default::default(),
            media_sequence: 0,
            show_missing: false,
            originals: Default::default(),
            export_probe_stamp: String::new(),
        })
    }
}
// A disjoint actor setting namespace avoids treating any real project ID as
// reserved. Unknown application fields are retained; concurrent writes refuse.
/// Where the application Agent panel (no project) keeps its archives.
pub const APPLICATION_AGENT_STORE: &str = "ext.builtin-application-agent";
pub(crate) struct KernelStore {
    kernel: KernelHandle,
    application: bool,
}
impl KernelStore {
    pub fn new(kernel: KernelHandle, application: bool) -> Self {
        Self {
            kernel,
            application,
        }
    }
}
impl pm_agents::persistence::Store for KernelStore {
    fn get(&mut self, key: &str) -> std::io::Result<Option<Value>> {
        let state = self.kernel.state();
        if !self.application {
            return Ok(state.settings.get(key).map(wire));
        }
        match state.settings.get(APPLICATION_AGENT_STORE).map(wire) {
            None => Ok(None),
            Some(Value::Object(fields)) => Ok(fields.get(key).cloned()),
            Some(_) => Err(std::io::Error::other(
                "Unknown application Agent archive shape",
            )),
        }
    }
    fn set(&mut self, key: &str, value: &Value) -> std::io::Result<()> {
        let state = self.kernel.state();
        let (key, value) = if self.application {
            let mut fields = match state.settings.get(APPLICATION_AGENT_STORE).map(wire) {
                None => serde_json::Map::new(),
                Some(Value::Object(fields)) => fields,
                Some(_) => {
                    return Err(std::io::Error::other(
                        "Unknown application Agent archive shape",
                    ));
                }
            };
            fields.insert(key.into(), value.clone());
            (APPLICATION_AGENT_STORE, Value::Object(fields))
        } else {
            (key, value.clone())
        };
        let value = model(&value).map_err(std::io::Error::other)?;
        self.kernel
            .state_transaction(state::StateTransaction {
                expected_revision: Some(state.revision),
                ops: vec![state::StateOp::Setting {
                    key: key.into(),
                    value: Some(value),
                }],
            })
            .wait()
            .ok_or_else(|| std::io::Error::other("kernel closed"))?
            .map_err(std::io::Error::other)?;
        Ok(())
    }
}

impl Backend {
    pub(crate) fn time(&self) -> f64 {
        self.native.facts["time"].as_f64().unwrap_or(0.)
    }
    pub(crate) fn fields(&mut self, names: &Value) -> Result<Value, String> {
        let mut fields = serde_json::Map::new();
        for name in names.as_array().ok_or("field names")? {
            let name = name.as_str().ok_or("field name")?;
            let id = format!(
                "builtin:{}:{}:{}",
                self.panel_id, self.identity.generation, name
            );
            self.native.fields.insert(id.clone(), name.into());
            fields.insert(name.into(), json!(id));
        }
        Ok(Value::Object(fields))
    }
    pub(crate) fn inspector_snapshot(&mut self) -> Result<Value, String> {
        self.sync()?;
        let time = self.time();
        let playing = self.native.facts["playing"].as_bool().unwrap_or(false);
        self.native.properties.snapshot(
            self.model.as_mut().ok_or("No open project")?,
            time,
            playing,
        )
    }
    pub(crate) fn timeline_frame(&mut self, request: &Value) -> Result<Value, String> {
        self.sync()?;
        self.native.ensure_timeline()?;
        let time = self.time();
        let playing = self.native.facts["playing"].as_bool().unwrap_or(false);
        let p = self.published().ok_or("closed")?;
        let client = self
            .native
            .timeline
            .as_mut()
            .ok_or("timeline action in flight")?;
        let first = request["first"].as_u64().ok_or("first row")? as usize;
        let visible = request["visible"]
            .as_u64()
            .filter(|n| *n > 0 && *n <= 512)
            .ok_or("visible rows")? as usize;
        self.model
            .as_mut()
            .ok_or("No open project")?
            .sync(p.clone());
        client.model.sync(&p);
        let mut frame = pm_panel_timeline::v4::project_window(&client.model, first, visible)
            .map_err(|e| e.to_string())?;
        let fps = client.model.index.fps;
        let numbers = layer_numbers(&p);
        let mut display = vec![];
        let editor_field = format!(
            "builtin:{}:{}:edit",
            self.panel_id, self.identity.generation
        );
        for row in frame["window"]["rows"].as_array().ok_or("timeline rows")? {
            let mut row = row.clone();
            row["focused"] = json!(false);
            row["hovered"] = json!(false);
            if row["kind"] == "track" {
                row["accessibilityLabel"] = row["label"].clone();
                display.push(row);
                continue;
            }
            let layer = row["layer"].as_str().ok_or("row layer")?.to_owned();
            if row["kind"] == "layer" {
                if !numbers {
                    row["index"] = Value::Null;
                }
                let l = client.model.index.layer(&layer).ok_or("layer")?;
                row["accessibilityLabel"] = json!(format!(
                    "{}, {}",
                    l.name.to_string_lossy(),
                    super::data::kind_label(&l.kind)
                ));
                row["canOpen"] = json!(!l.channels.is_empty() || l.kind == "group");
                row["transformable"] = json!(l.kind != "audio");
                row["parent"] = json!({"name":l.parent.as_deref().and_then(|p|client.model.index.layer(p)).map(|p|p.name.to_string_lossy()).unwrap_or_default(),"set":l.parent.is_some()});
                if self.native.timeline_editor == Some(TimelineEditor::Name(layer.clone())) {
                    row["nameEditor"] = json!({
                        "value": l.name.to_string_lossy(),
                        "binding": editor_field.clone(),
                    });
                }
                row["media"] = json!(if matches!(l.kind.as_str(), "video" | "audio" | "image") {
                    "loading"
                } else {
                    "ready"
                });
            } else {
                let path = row["path"].as_str().ok_or("property path")?.to_owned();
                let label = property_label(&path);
                row["label"] = json!(label);
                let channels = client.model.track_channels(&layer, &path);
                let mut values = Vec::new();
                let mut animated = false;
                let mut at_key = !channels.is_empty();
                for channel in &channels {
                    let binding = pm_panel_inspector::Binding::Property(
                        pm_panel_inspector::PropertyId::new(&layer, &channel.path),
                    );
                    let state = self
                        .model
                        .as_mut()
                        .ok_or("No open project")?
                        .binding_state(&binding, time)
                        .map_err(|e| format!("{e:?}"))?;
                    let value = wire(&state.value);
                    values.push(match value.as_f64() {
                        Some(n) => readout(n),
                        None => value.to_string(),
                    });
                    animated |= state.animated;
                    at_key &= state.key_at_playhead;
                }
                row["readout"] = json!(values.join(", "));
                row["animated"] = json!(animated);
                row["keyAtPlayhead"] = json!(at_key);
                row["previous"] = json!(
                    client
                        .model
                        .adjacent_key(-1, time, Some((&layer, &path)))
                        .is_some()
                );
                row["next"] = json!(
                    client
                        .model
                        .adjacent_key(1, time, Some((&layer, &path)))
                        .is_some()
                );
                row["accessibilityLabel"] = json!(format!(
                    "{label}, {}",
                    row["readout"].as_str().unwrap_or("")
                ));
            }
            display.push(row);
        }
        frame["window"]["rows"] = json!(display);
        let frames = (time * fps).round().max(0.) as u64;
        let nominal = fps.round().max(1.) as u64;
        let sec = frames / nominal;
        frame["graphEmpty"] = json!(
            !client
                .model
                .index
                .layers
                .iter()
                .any(|l| l.channels.iter().any(|c| !c.keys.is_empty()))
        );
        frame["playing"] = json!(playing);
        frame["timecode"] = json!(format!(
            "{:02}:{:02}:{:02}:{:02}",
            sec / 3600,
            sec / 60 % 60,
            sec % 60,
            frames % nominal
        ));
        frame["spokenTime"] = json!(format!("{sec} seconds, {} frames", frames % nominal));
        if self.native.timeline_editor == Some(TimelineEditor::Time) {
            frame["timeEditor"] = json!({"value":frame["timecode"],"binding":editor_field});
        }
        frame["frameRate"] = json!(format!("{fps} fps"));

        let title = p
            .snapshot
            .root()
            .and_then(|r| r.get("name"))
            .and_then(|n| match n.kind() {
                pm_model::NodeKind::String(s) => Some(s.to_string_lossy()),
                _ => None,
            })
            .unwrap_or_default();
        // Root-level layers are the composition the shipped app calls `root`.
        let tab = client.model.index.comp_id.as_deref().unwrap_or("root");
        frame["tabs"] = json!([{"id":tab,"name":title,"active":true,"showClose":false}]);
        frame["renderSummary"] = json!("");
        frame["feedback"] = json!({});
        Ok(frame)
    }
    /// The viewer slot (the host's `stage` fact, in points) around the
    /// composition. A pane shorter than two insets fits edge to edge.
    fn viewer_viewport(
        &self,
        vm: &pm_panel_viewerchrome::ViewModel,
    ) -> Result<pm_panel_viewerchrome::Viewport, String> {
        let side = |i: usize, default: f64| {
            self.native.facts["stage"][i]
                .as_f64()
                .filter(|v| v.is_finite())
                .unwrap_or(default)
                .max(1.)
        };
        let stage = pm_panel_viewerchrome::Size {
            width: side(0, 640.),
            height: side(1, 360.),
        };
        let inset = pm_design::Tokens::new(pm_design::Scheme::Dark, Default::default())
            .metrics
            .inset as f64;
        let room = stage.width.min(stage.height);
        let inset = if inset * 2. < room { inset } else { 0. };
        vm.viewport(stage, inset).map_err(|e| e.to_string())
    }
    pub(crate) fn viewer_snapshot(&mut self) -> Result<Value, String> {
        self.sync()?;
        let p = self.published().ok_or("closed")?;
        let mut vm = pm_panel_viewerchrome::ViewModel::read(&p, self.kernel.state().revision)
            .map_err(|e| e.to_string())?;
        // Auto's playing resolution is the engine's measurement, not a setting.
        if let Some(scale) = self.native.facts["effectiveQuality"]
            .as_f64()
            .filter(|q| (0.25..=1.).contains(q))
        {
            vm.chrome.effective_quality = scale;
        }
        let effective = vm.chrome.effective_quality;
        let viewport = self.viewer_viewport(&vm)?;
        let model = if let Some((stamp, previous_viewport, previous_effective, projection)) =
            &self.native.viewer_projection
            && *stamp == vm.stamp
            && *previous_viewport == viewport
            && *previous_effective == effective
        {
            projection.clone()
        } else {
            let projection = wire(
                &self
                    .native
                    .viewer
                    .publish(&vm, viewport)
                    .map_err(|e| e.to_string())?,
            );
            self.native.viewer_projection =
                Some((vm.stamp.clone(), viewport, effective, projection.clone()));
            projection
        };
        Ok(
            json!({"model":model,"host":{"presentation":"Docked","frame":{"resource":null,"loading":self.native.facts["frameLoading"].as_bool().unwrap_or(false),"failed":self.native.facts["frameFailed"].as_bool().unwrap_or(false)},"source":null,"interaction":"idle"}}),
        )
    }
    pub(crate) fn project_view(&mut self, request: Option<Value>) -> Result<Value, String> {
        let p = self.published().ok_or("closed")?;
        let catalog = Catalog::published(&p).map_err(|e| format!("{e:?}"))?;
        self.native
            .project
            .refresh(&catalog, &self.native.media_runtime);
        if let Some(request) = request {
            let reply = self
                .native
                .project
                .request(
                    &catalog,
                    &self.native.media_runtime,
                    serde_json::from_value(request).map_err(|e| e.to_string())?,
                )
                .map_err(|e| format!("{e:?}"))?;
            if reply
                .outcome
                .as_ref()
                .is_some_and(|o| o.edit.is_some() || !o.effects.is_empty())
            {
                return Err("Project edit requires native coordinator".into());
            }
            serde_json::to_value(reply).map_err(|e| e.to_string())
        } else {
            serde_json::to_value(
                self.native
                    .project
                    .view(&catalog, &self.native.media_runtime)
                    .map_err(|e| format!("{e:?}"))?,
            )
            .map_err(|e| e.to_string())
        }
    }
    fn project_command(&mut self, args: &Value) -> Result<NativeCommand, String> {
        let p = self.published().ok_or("closed")?;
        let catalog = Catalog::published(&p).map_err(|e| format!("{e:?}"))?;
        self.native
            .project
            .refresh(&catalog, &self.native.media_runtime);
        if std::mem::take(&mut self.native.show_missing) {
            // Project-wide: from the top folder, every missing file, flat.
            let filter = pm_panel_media::Filter {
                missing_only: true,
                ..Default::default()
            };
            self.native
                .project
                .apply_host_filter(&catalog, &self.native.media_runtime, filter)
                .map_err(|e| format!("{e:?}"))?;
        }
        let mut reply = self
            .native
            .project
            .request(
                &catalog,
                &self.native.media_runtime,
                serde_json::from_value(args.clone()).map_err(|e| e.to_string())?,
            )
            .map_err(|e| format!("{e:?}"))?;
        if let Some(outcome) = reply.outcome.as_mut() {
            for effect in outcome.effects.drain(..) {
                self.native.platform.push((
                    "project.effect".into(),
                    serde_json::to_value(pm_panel_media::HostRequest {
                        owner: catalog.stamp.clone(),
                        effect,
                    })
                    .map_err(|e| e.to_string())?,
                ));
            }
            if let Some(edit) = outcome.edit.take() {
                let prepared = catalog
                    .prepare(edit.clone())
                    .map_err(|e| format!("{e:?}"))?;
                self.native.project_reply =
                    Some(serde_json::to_value(reply).map_err(|e| e.to_string())?);
                return Ok(NativeCommand::Kernel(project_edit_request(
                    edit,
                    &prepared,
                    self.caller,
                )));
            }
        }
        Ok(NativeCommand::Value(
            serde_json::to_value(reply).map_err(|e| e.to_string())?,
        ))
    }
    pub(crate) fn agent_snapshot(&mut self) -> Result<Value, String> {
        let agent = self.native.agent.clone().ok_or("Agent panel is not open")?;
        let mut state = agent.shared();
        let active = state.panel.active_id().to_owned();
        let provider = state
            .panel
            .session(&active)
            .map_err(|e| format!("{e:?}"))?
            .settings
            .provider;
        state
            .panel
            .set_catalog(&active, self.native.agent_link.catalog(provider))
            .map_err(|e| format!("{e:?}"))?;
        let panel = &state.panel;
        let active = active.as_str();
        let window =
            pm_panel_agentpanel::v4::Window::new(panel, active).map_err(|e| format!("{e:?}"))?;
        let count = window.count(panel).map_err(|e| format!("{e:?}"))?;
        let mut snapshot = window
            .snapshot(
                panel,
                count.saturating_sub(64)..count,
                crate::agent_host::now_ms(),
            )
            .map_err(|e| format!("{e:?}"))?;
        // The images, videos and sounds each reply names, for its cards.
        let home = reply_home();
        for item in snapshot["rows"]["items"]
            .as_array_mut()
            .into_iter()
            .flatten()
        {
            let value = &mut item["value"];
            let media: Vec<Value> = reply_media(value, &home)
                .iter()
                .map(
                    |m| json!({"kind":m.kind_name(),"path":m.path,"name":m.name,"remote":m.remote}),
                )
                .collect();
            if !media.is_empty() {
                value["media"] = json!(media);
            }
        }
        let session = panel.session(active).map_err(|e| format!("{e:?}"))?;
        let (connection, reason) = self.native.agent_link.connection(provider);
        let needs_answer = session.needs_answer();
        Ok(
            json!({"generation":self.identity.generation,"sequence":self.native.agent_link.sequence(),"snapshot":snapshot,"native":{"connection":connection,"reason":reason,"presentation":"popover","atMinimumDockWidth":false,"visible":true,"extensionContext":false,"loadingHistory":false,"dropTarget":false,"needsAnswer":needs_answer,"keyboardFocus":null,"textSelected":false,"hoverRow":null,"increaseContrast":false,"reduceTransparency":false,"reduceMotion":false,"slashCopy":{},"bindings":{"access":"agent.access","model":"agent.model"}}}),
        )
    }
    pub(crate) fn settings_snapshot(&mut self) -> Result<Value, String> {
        self.sync()?;
        let project = self.published().map(|p| {
            let name = p
                .snapshot
                .root()
                .and_then(|r| r.get("name"))
                .and_then(|n| match n.kind() {
                    pm_model::NodeKind::String(s) => Some(s.to_string_lossy()),
                    _ => None,
                })
                .unwrap_or_default();
            json!({"id":p.project,"name":name})
        });
        let state = self.kernel.state();
        let mods =
            pm_services::surface_backing::projects::mods(&mod_records(&state)).unwrap_or_default();
        let extensions = state
            .extensions
            .values()
            .filter(|e| !archived(e))
            .map(|e| {
                let can_archive = mods.iter().any(|m| {
                    m.id == e.id
                        && m.actions
                            .contains(&pm_services::surface_backing::projects::ModAction::Archive)
                });
                let host = e.source.get("hostRecord");
                let text = |key: &str| {
                    host.and_then(|h| h.get(key))
                        .and_then(pm_model::Value::as_str)
                        .map(str::to_owned)
                };
                let includes = host
                    .and_then(|h| h.get("contributes"))
                    .and_then(pm_model::Value::as_array)
                    .map(|kinds| {
                        kinds
                            .iter()
                            .filter_map(pm_model::Value::as_str)
                            .map(str::to_owned)
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default();
                let source = match (e.installed, text("trust").as_deref()) {
                    (false, _) => "Built in",
                    (true, Some("local")) => "Custom",
                    _ => "Installed",
                };
                // D11 Stop turns it off for this run, not in the saved preference.
                let stopped = host
                    .and_then(|h| h.get("stopped"))
                    .and_then(pm_model::Value::as_bool)
                    == Some(true);
                // Its folder can't be reached (an unmounted volume): kept, said so.
                let about = if host
                    .and_then(|h| h.get("unavailable"))
                    .and_then(pm_model::Value::as_bool)
                    == Some(true)
                {
                    "Unavailable: its folder can't be reached.".to_owned()
                } else {
                    text("about").unwrap_or_default()
                };
                json!({"id":e.id,"name":text("name").unwrap_or_else(|| e.id.clone()),"type":"Other","source":source,
                    "enabled":e.enabled && !stopped,"about":about,"includes":includes,
                    "shortcuts":[],"files":[],"variables":[],"canArchive":can_archive})
            })
            .collect::<Vec<_>>();
        let accounts = self.account_rows();
        let speech = crate::speech::status();
        let speech_rows = speech.as_ref().map(crate::speech::rows).unwrap_or_default();
        let mut publication = json!({"revision":self.native.settings_sequence,"multiWindow":true,"rows":accounts,"extensions":extensions,"extensionsState":"ready",
            "speech":speech.as_ref().map(crate::speech::models).unwrap_or_default()});
        if let Some(rows) = publication["rows"].as_array_mut() {
            rows.extend(speech_rows.iter().cloned());
        }
        if let Some(project) = project {
            publication["project"] = project;
            let mut rows = self.settings_rows()?;
            rows.extend(self.account_rows());
            rows.extend(speech_rows);
            publication["rows"] = json!(rows);
            let p = self.published().ok_or("Project closed")?;
            let root = p.snapshot.root().ok_or("Project closed")?;
            let mut notes = pm_model::Map::new();
            for key in ["memory", "memoryEnabled"] {
                if let Some(node) = root.get(key) {
                    notes.insert(key, node.to_value());
                }
            }
            let root = pm_model::Value::Object(notes);
            let entries = pm_model::memory::list(&root, None, 100).iter().map(|note| {
                let text = |key| note.get(key).and_then(pm_model::Value::as_js_string)
                    .map(pm_model::JsString::to_string_lossy).unwrap_or_default();
                json!({"id":text("id"),"kind":text("kind"),"text":text("text"),"why":text("why"),
                    "author":text("author"),"updatedAt":note.get("updatedAt").and_then(pm_model::Value::as_f64),
                    "resolved":note.get("resolved")==Some(&pm_model::Value::Bool(true))})
            }).collect::<Vec<_>>();
            publication["memory"] =
                json!({"enabled":pm_model::memory::enabled(&root),"entries":entries});
        }
        Ok(publication)
    }
    pub(crate) fn extra_query(
        &mut self,
        query: &Query,
        channels: Option<&[String]>,
    ) -> Result<Option<Value>, String> {
        if channels == Some(&["settings".to_owned()][..]) {
            return self.settings_snapshot().map(Some);
        }
        if channels == Some(&["agent".to_owned()][..]) {
            return self.agent_snapshot().map(Some);
        }
        if channels == Some(&["viewer.chrome".to_owned()][..]) {
            return self.viewer_snapshot().map(Some);
        }
        if channels == Some(&["export".to_owned()][..]) {
            return self.export_snapshot().map(Some);
        }
        if self.panel_id == "inspector" && matches!(query, Query::Evaluated) {
            return self.inspector_snapshot().map(Some);
        }
        if matches!(query, Query::Time) {
            return Ok(Some(
                json!({"time":self.time(),"playing":self.native.facts["playing"]}),
            ));
        }
        if self.panel_id == "assets" && matches!(query, Query::Project) {
            let p = self.published().ok_or("closed")?;
            return Ok(Some(
                json!({"stamp":{"project":p.project,"document":p.snapshot.doc_id,"revision":p.revision()},"media":self.native.media_sequence}),
            ));
        }
        Ok(None)
    }
    pub(crate) fn start_timeline(&mut self, intent: Value) -> Result<(), String> {
        if self.native.timeline_job.is_some() || self.native.timeline_effect_pending != 0 {
            return Err("Timeline command pending".into());
        }
        let p = self.published().ok_or("closed")?;
        self.native.ensure_timeline()?;
        let mut client = self.native.timeline.take().ok_or("Timeline closed")?;
        let time = self.time();
        self.native.timeline_job = Some(Box::pin(async move {
            if intent["type"] == "easingLibrary" {
                let result = crate::timeline_easing::dispatch(&client.kernel, &intent)
                    .await
                    .map(|()| Vec::new());
                return (client, result);
            }
            let context = pm_panel_timeline::v4::Context {
                time,
                expected_document: p.snapshot.doc_id,
                expected_revision: p.revision(),
                millis: intent["millis"].as_u64().unwrap_or_else(|| {
                    static CLOCK: std::sync::OnceLock<std::time::Instant> =
                        std::sync::OnceLock::new();
                    CLOCK
                        .get_or_init(std::time::Instant::now)
                        .elapsed()
                        .as_millis() as u64
                }),
                in_field: false,
                keys_admitted: true,
                property_facts: &PropertyFacts::default(),
            };
            let result = pm_panel_timeline::v4::dispatch(&mut client, &intent, &context)
                .await
                .map_err(|e| e.to_string());
            (client, result)
        }));
        Ok(())
    }
    pub(crate) fn poll_timeline(&mut self) -> Option<Result<Value, String>> {
        if self.native.timeline_effect_pending != 0 {
            return None;
        }
        if let Some(result) = self.native.timeline_effect_result.take() {
            return Some(result.map(|()| Value::Null));
        }
        let future = self.native.timeline_job.as_mut()?;
        let mut cx = Context::from_waker(Waker::noop());
        let Poll::Ready((client, result)) = future.as_mut().poll(&mut cx) else {
            return None;
        };
        self.native.timeline = Some(client);
        self.native.timeline_job = None;
        let effects = match result {
            Ok(effects) => effects,
            Err(error) => return Some(Err(error)),
        };
        let mut actions = Vec::new();
        for effect in effects {
            use pm_panel_timeline::HostEffect;
            let action = match effect {
                HostEffect::Seek(time) => json!({"kind":"seek","time":time}),
                HostEffect::TogglePlayback => json!({"kind":"command","name":"play"}),
                HostEffect::FrameComposition => json!({"kind":"command","name":"viewer.fit"}),
                HostEffect::OpenComposition(id) => json!({"kind":"openComposition","id":id}),
                other => return Some(Err(format!("Timeline host effect unavailable: {other:?}"))),
            };
            actions.push(("timeline.effect".into(), action));
        }
        if actions.is_empty() {
            return Some(Ok(Value::Null));
        }
        self.native.timeline_effect_pending = actions.len();
        self.native.timeline_effect_result = Some(Ok(()));
        self.native.platform.extend(actions);
        // The HostAck stays pending until every native platform request replies.
        None
    }
}
fn property_label(path: &str) -> String {
    pm_panel_timeline::property_label(path)
}

/// A timeline row's name, or the header time, open for typing.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum TimelineEditor {
    Name(String),
    Time,
}
pub(crate) enum NativeCommand {
    Value(Value),
    Kernel(Request),
    State(state::StateTransaction),
    Timeline(Value),
}
impl Backend {
    /// Name and time editing live here, next to the fields they open. The
    /// committed value becomes an ordinary kernel edit or seek.
    fn timeline_intent(&mut self, args: &Value) -> Result<NativeCommand, String> {
        use pm_panel_timeline::{RowId, v4};
        let done = |v: Value| Ok(NativeCommand::Value(v));
        match args["type"].as_str() {
            Some("rename") => {
                let RowId::Layer(id) =
                    v4::parse_row(args["id"].as_str().ok_or("row")?).map_err(|e| e.to_string())?
                else {
                    return Err("Only layers can be renamed".into());
                };
                self.native.timeline_editor = Some(TimelineEditor::Name(id));
                done(Value::Null)
            }
            Some("timeEdit") => {
                self.native.timeline_editor = Some(TimelineEditor::Time);
                done(Value::Null)
            }
            Some("editDone") => {
                let editor = self.native.timeline_editor.take();
                // Empty closes the field, as Escape does; nothing has a blank name.
                let value = args["value"].as_str().unwrap_or("");
                if value.trim().is_empty() {
                    return done(Value::Null);
                }
                match editor {
                    Some(TimelineEditor::Name(id)) => {
                        let id = v4::row_id(&RowId::Layer(id));
                        Ok(NativeCommand::Timeline(
                            json!({"type":"setName","id":id,"name":value}),
                        ))
                    }
                    Some(TimelineEditor::Time) => {
                        self.native.ensure_timeline()?;
                        let fps = self
                            .native
                            .timeline
                            .as_ref()
                            .map_or(30., |c| c.model.index.fps);
                        match v4::parse_timecode(value, fps) {
                            Some(time) => {
                                Ok(NativeCommand::Timeline(json!({"type":"seek","time":time})))
                            }
                            None => Err("Enter a timecode like 00:00:02:15".into()),
                        }
                    }
                    None => done(Value::Null),
                }
            }
            _ => Ok(NativeCommand::Timeline(args.clone())),
        }
    }
    /// Rows for a native menu the Timeline header opens, and the intent each
    /// row sends back. Graph options come from the model's own menu.
    fn timeline_menu(&mut self, args: &Value) -> Result<Value, String> {
        use pm_panel_timeline::v4;
        self.native.ensure_timeline()?;
        let client = self.native.timeline.as_ref().ok_or("Timeline closed")?;
        if args["type"] == "graphOptions" {
            let context = pm_panel_timeline::Context::Graph { series: 0 };
            let rows: Vec<Value> = client
                .model
                .menu(&context)
                .into_iter()
                .filter(|item| item.enabled)
                .map(|item| {
                    json!({
                        "id": item.id,
                        "label": pm_panel_timeline::menu_label(item.id),
                        "checked": item.checked,
                        "intent": {"type":"graphMenu","action":item.id},
                    })
                })
                .collect();
            return Ok(json!(rows));
        }
        let row = args["id"].as_str().ok_or("row")?;
        let pm_panel_timeline::RowId::Layer(id) = v4::parse_row(row).map_err(|e| e.to_string())?
        else {
            return Err("Only layers have parents".into());
        };
        let current = client.model.index.layer(&id).and_then(|l| l.parent.clone());
        let mut rows = vec![json!({
            "id": "none", "label": "None", "checked": current.is_none(),
            "intent": {"type":"parent","ids":[id],"parent":null},
        })];
        rows.extend(
            client
                .model
                .index
                .layers
                .iter()
                .filter(|l| l.id != id)
                .map(|l| {
                    json!({
                        "id": l.id,
                        "label": l.name.to_string_lossy(),
                        "checked": current.as_deref() == Some(l.id.as_str()),
                        "intent": {"type":"parent","ids":[id],"parent":l.id},
                    })
                }),
        );
        Ok(json!(rows))
    }
    pub(crate) fn export_snapshot(&mut self) -> Result<Value, String> {
        self.sync()?;
        let p = self.published().ok_or("closed")?;
        let root = p.snapshot.root().ok_or("closed")?;
        let comp = pm_kernel::model::comp_node(root, p.snapshot.open_comp().as_deref())
            .ok_or("composition")?;
        let n = |key: &str, default: f64| {
            comp.get(key)
                .and_then(|n| match n.kind() {
                    pm_model::NodeKind::Number(v) => Some(*v),
                    _ => None,
                })
                .unwrap_or(default)
        };
        let width = n("w", 1920.) as u32;
        let height = n("h", 1080.) as u32;
        let fps = n("fps", 30.);
        let name = root
            .get("name")
            .and_then(|n| match n.kind() {
                pm_model::NodeKind::String(s) => Some(s.to_string_lossy()),
                _ => None,
            })
            .unwrap_or_default();
        if self.native.export.is_null() {
            let fields = self.fields(&json!([
                "format",
                "scale",
                "width",
                "height",
                "fps",
                "range",
                "quality",
                "mblur",
                "audio",
                "alpha",
                "captionsBurn",
                "captionsSidecar",
                "profile",
                "codec",
                "depth",
                "dynamicRange",
                "useProxies",
                "preset",
                "presetName"
            ]))?;
            let fresh =
                serde_json::to_value(pm_promedia::export_panel::Options::new(width, height, fps))
                    .map_err(|e| e.to_string())?;
            // As main's exporter: the project's export defaults open the sheet
            // (every project has them, normalized); the sheet's other fields keep
            // the last export's.
            let base = self
                .kernel
                .state()
                .settings
                .get("exportOpts")
                .map(wire)
                .filter(|v| crate::delivery::options(v).is_ok())
                .unwrap_or_else(|| fresh.clone());
            let defaults = self.export_defaults(root, fps);
            let mut options = base.clone();
            for key in [
                "format", "scale", "fps", "range", "quality", "mblur", "audio", "alpha",
            ] {
                options[key] = defaults[key].clone();
            }
            // main's planner exports the whole composition when "work" has no
            // work area to use; say so in the sheet.
            let work = comp
                .get("work")
                .and_then(|w| crate::settings::work_span(&wire(&w.to_value())));
            if options["range"] == "work" && work.is_none() {
                options["range"] = json!("all");
            }
            // A format this app can't export (v1 `rec`, `web`) keeps the last one.
            if crate::delivery::options(&options).is_err() {
                options["format"] = base["format"].clone();
            }
            if crate::delivery::options(&options).is_err() {
                options = fresh;
            }
            self.native.export = json!({"version":1,"sequence":1,"project":null,"options":options,"bindings":fields,"presets":[],"capabilities":{},"run":{"phase":"idle"}});
        }
        let options: pm_promedia::export_panel::Options =
            crate::delivery::options(&self.native.export["options"])?;
        // Settings saved before a field existed show its default in the sheet.
        self.native.export["options"] =
            serde_json::to_value(&options).map_err(|e| e.to_string())?;
        let plan = crate::delivery::plan(&p.snapshot, &options, self.time())?;
        self.native.export["project"] = json!({"id":p.project,"name":name,"width":width,"height":height,"hasAudio":plan.has_audio,"captionLayers":plan.captions,"hasWorkArea":comp.get("work").is_some(),"hdr":pm_render::composite::project_color(root).is_ok_and(|c|c.mode==pm_protocol::color::ColorMode::Hdr),"summary":plan.summary});
        self.native.export["presets"] = json!(
            self.kernel
                .state()
                .settings
                .get("renderPresets")
                .map(wire)
                .and_then(|v| v.as_array().cloned())
                .unwrap_or_default()
                .into_iter()
                .filter(|p| p["archived"] != true
                    && p["id"].is_string()
                    && p["name"].is_string()
                    && crate::delivery::options(&p["options"]).is_ok())
                .collect::<Vec<_>>()
        );
        #[cfg(target_os = "macos")]
        {
            let key = format!(
                "{}:{}:{}:{:?}",
                p.snapshot.doc_id,
                p.revision(),
                serde_json::to_string(&options).map_err(|e| e.to_string())?,
                self.native.originals
            );
            if self.native.export_probe_stamp != key {
                self.native.export["capabilities"] = crate::delivery::capabilities(
                    &p.snapshot,
                    &options,
                    self.time(),
                    &self.native.originals,
                );
                self.native.export_probe_stamp = key;
            }
        }
        #[cfg(not(target_os = "macos"))]
        for format in ["prores", "mp4", "webm", "png", "still"] {
            self.native.export["capabilities"][format] =
                json!({"available":false,"reason":"Native encoder probes require macOS."});
        }
        self.native.export["capabilities"]["json"] =
            crate::delivery::portable_capability(&p.snapshot, &self.native.originals);
        Ok(self.native.export.clone())
    }
    pub(crate) fn command_reply(&mut self, command: &str) -> Result<Value, String> {
        match command {
            "export.begin" | "export.retry" | "export.without" => {
                self.native.export["run"] = json!({"phase":"choosingDestination"});
                self.native
                    .platform
                    .push((command.into(), self.native.export["options"].clone()));
                self.native.export["sequence"] =
                    json!(self.native.export["sequence"].as_u64().unwrap_or(0) + 1);
                self.export_snapshot()
            }
            c if c.starts_with("export.") => {
                self.native.export["sequence"] =
                    json!(self.native.export["sequence"].as_u64().unwrap_or(0) + 1);
                self.export_snapshot()
            }
            "profile.set" => Ok(json!({"stateRevision":self.kernel.state().revision})),
            "panel.settings" => self.settings_snapshot(),
            "panel.inspector" | "panel.binding" => self.inspector_snapshot(),
            "viewer.chrome.drive" => self.viewer_snapshot(),
            "panel.project" => {
                let mut reply = self
                    .native
                    .project_reply
                    .take()
                    .ok_or("Missing project acknowledgement")?;
                reply["view"] = self.project_view(None)?;
                Ok(reply)
            }
            _ => self.projection(),
        }
    }
    pub(crate) fn native_command(
        &mut self,
        command: &str,
        args: &Value,
    ) -> Result<NativeCommand, String> {
        self.sync()?;
        let time = self.time();
        if matches!(
            command,
            "export.begin"
                | "export.retry"
                | "export.without"
                | "export.plan"
                | "export.preset.save"
                | "export.reset"
                | "export.closeFailure"
        ) && matches!(
            self.native.export["run"]["phase"].as_str(),
            Some("choosingDestination" | "preparing" | "rendering" | "cancelling")
        ) {
            return Err("Export is in progress".into());
        }
        let value = match command {
            "panel.inspector" => {
                if let Some(request) = self.native.properties.intent(
                    self.model.as_mut().ok_or("No open project")?,
                    self.caller,
                    args,
                    time,
                )? {
                    return Ok(NativeCommand::Kernel(request));
                }
                self.inspector_snapshot()?
            }
            "panel.binding" => {
                let handle = args["handle"].as_str().ok_or("binding handle")?;
                if self.native.fields.contains_key(handle) {
                    return Err(
                        "Local controls must dispatch their authored change listener".into(),
                    );
                }
                self.validate_binding_owner(handle)?;
                if self.panel_id == "layer-effects" {
                    return self
                        .parameter_write(handle, model(&args["value"])?, time)
                        .map(NativeCommand::Kernel);
                }
                let request = self.native.properties.write(
                    self.model.as_mut().ok_or("No open project")?,
                    self.caller,
                    handle,
                    model(&args["value"])?,
                    time,
                )?;
                return Ok(NativeCommand::Kernel(request));
            }
            "panel.timeline" => {
                return self.timeline_intent(args);
            }
            "panel.timeline.menu" => self.timeline_menu(args)?,
            "panel.timeline.dispose" => {
                return Ok(NativeCommand::Timeline(json!({"type":"dispose"})));
            }
            "viewer.chrome.drive" => {
                let p = self.published().ok_or("closed")?;
                let vm = pm_panel_viewerchrome::ViewModel::read(&p, self.kernel.state().revision)
                    .map_err(|e| e.to_string())?;
                let viewport = self.viewer_viewport(&vm)?;
                let plan = self
                    .native
                    .viewer
                    .drive(&vm, &model(args)?, viewport, self.caller)
                    .map_err(|e| e.to_string())?;
                match plan.effect {
                    pm_panel_viewerchrome::Effect::State(tx) => {
                        return Ok(NativeCommand::State(tx));
                    }
                    pm_panel_viewerchrome::Effect::Document(request) => {
                        return Ok(NativeCommand::Kernel(request));
                    }
                    pm_panel_viewerchrome::Effect::Host(action) => {
                        use pm_panel_viewerchrome::{HostAction, TransportAction};
                        let action = match action {
                            HostAction::Transport(action) => match action {
                                TransportAction::Seek(time) => json!({"kind":"seek","time":time}),
                                TransportAction::Step(frames) => {
                                    json!({"kind":"command","name":"stepFrames","args":[frames]})
                                }
                                TransportAction::Toggle => json!({"kind":"command","name":"play"}),
                                TransportAction::Start => {
                                    json!({"kind":"command","name":"gotoStart"})
                                }
                                TransportAction::End => json!({"kind":"command","name":"gotoEnd"}),
                                TransportAction::WorkIn => {
                                    json!({"kind":"command","name":"gotoWorkIn"})
                                }
                                TransportAction::WorkOut => {
                                    json!({"kind":"command","name":"gotoWorkOut"})
                                }
                                TransportAction::Pause => json!({"kind":"pause"}),
                            },
                            HostAction::Command { id, args } => {
                                json!({"kind":"command","name":id,"args":args.iter().map(wire).collect::<Vec<_>>()})
                            }
                            other => json!({"kind":"unavailable","action":format!("{other:?}")}),
                        };
                        self.native.platform.push(("viewer.action".into(), action));
                        Value::Null
                    }
                }
            }
            "panel.project" => return self.project_command(args),
            "panel.settings" => return self.settings_action(args),
            "panel.agent.read" => self.agent_snapshot()?,
            "panel.agent.dispatch" => {
                let agent = self.native.agent.clone().ok_or("Agent panel is not open")?;
                let link = self.native.agent_link.clone();
                let effect = {
                    let mut state = agent.shared();
                    let thread = state.panel.active_id().to_owned();
                    let session = state.panel.session(&thread).map_err(|e| format!("{e:?}"))?;
                    let (provider, live) = (session.settings.provider, session.run.is_some());
                    let sequence = link.admitted_sequence(args, &thread, &session.draft.text());
                    let action = &args["action"];
                    if action["type"] == "retryConnection" {
                        if let Some(accounts) = link.accounts() {
                            accounts.refresh(provider);
                        }
                        drop(state);
                        link.touch();
                        return Ok(NativeCommand::Value(self.agent_snapshot()?));
                    }
                    if action["type"] == "attach" {
                        // The window shows its file picker; chosen files come back
                        // through `Builtins::agent_attach`.
                        drop(state);
                        self.native
                            .platform
                            .push(("agent.attach".into(), Value::Null));
                        link.touch();
                        return Ok(NativeCommand::Value(self.agent_snapshot()?));
                    }
                    if action["type"] == "openMedia" {
                        // Only a file or address one of this thread's replies names.
                        let path = action["path"].as_str().unwrap_or_default();
                        let home = reply_home();
                        let media = state
                            .panel
                            .session(&thread)
                            .map_err(|e| format!("{e:?}"))?
                            .conversation
                            .values()
                            .flat_map(prose_rows)
                            .flat_map(|row| reply_media(row, &home))
                            .find(|m| m.path == path)
                            .ok_or("This file can't be opened from here.")?;
                        drop(state);
                        self.native.platform.push((
                            "agent.media".into(),
                            json!({"path":media.path,"remote":media.remote,"reveal":action["reveal"] == true}),
                        ));
                        return Ok(NativeCommand::Value(self.agent_snapshot()?));
                    }
                    if action["type"] == "openVisual" {
                        // A saved reply is checked again before it's shown (main's
                        // readVisualReply); the window opens its own viewer.
                        let id = action["id"].as_str().unwrap_or_default();
                        let reply = state
                            .panel
                            .visual_reply(&thread, id)
                            .ok()
                            .and_then(|v| pm_agents::visual::read(&v))
                            .ok_or("This visualization can't be shown.")?;
                        drop(state);
                        self.native.platform.push((
                            "agent.visual".into(),
                            json!({"id":id,"title":reply.title,"html":reply.html,"height":reply.height}),
                        ));
                        return Ok(NativeCommand::Value(self.agent_snapshot()?));
                    }
                    crate::agent_host::admit(action, link.available(provider), live)?;
                    if crate::agent_host::sends(action) {
                        crate::agent_host::admit_send(&state, &thread)?;
                    }
                    let run = new_id("run");
                    let request = new_id("request");
                    let new_thread = new_id("thread");
                    let catalogs = |provider| link.catalog(provider);
                    let effect = pm_panel_agentpanel::v4::dispatch_at(
                        &mut state.panel,
                        sequence,
                        args,
                        pm_panel_agentpanel::v4::Context {
                            run: &run,
                            request: &request,
                            new_thread: &new_thread,
                            now: crate::agent_host::now_ms(),
                            popover_open: true,
                            catalogs: &catalogs,
                        },
                    )
                    .map_err(|e| format!("{e:?}"))?;
                    if action["type"] == "draft"
                        && let Some(text) = action["text"].as_str()
                    {
                        link.typed(&thread, text);
                    }
                    let now = crate::agent_host::now_ms();
                    // Typing reaches the archive at the next checkpoint, not per key.
                    if effect.is_none() && matches!(action["type"].as_str(), Some("draft" | "key"))
                    {
                        state.panel.defer_save(now);
                    } else {
                        state
                            .panel
                            .save(
                                &mut KernelStore::new(self.kernel.clone(), self.application),
                                now,
                            )
                            .map_err(|e| format!("{e:?}"))?;
                    }
                    effect
                };
                crate::agent_host::ensure_worker(&agent, &link, &self.kernel, self.application);
                if let Some(effect) = effect {
                    use pm_panel_agentpanel::model::{EffectKind, ShellAction};
                    let operation = match effect.kind {
                        EffectKind::OpenSettings => Some("agent.settings"),
                        EffectKind::Shell(ShellAction::Open) => Some("agent.open"),
                        EffectKind::Shell(ShellAction::Close) => Some("agent.close"),
                        EffectKind::Send { .. } | EffectKind::Stop | EffectKind::Answer { .. } => {
                            crate::agent_host::run_effect(
                                &agent,
                                &link,
                                &self.kernel,
                                self.application,
                                &effect,
                            )?;
                            None
                        }
                        _ => {
                            return Err(
                                "Agent provider or native action service is unavailable".into()
                            );
                        }
                    };
                    if let Some(operation) = operation {
                        self.native.platform.push((operation.into(), Value::Null));
                    }
                }
                link.touch();
                self.agent_snapshot()?
            }
            "profile.get" => {
                let key = args["key"].as_str().ok_or("profile key")?;
                if !profile_key(key) {
                    return Err("Unprivileged profile key".into());
                }
                let state = self.kernel.state();
                let mut result = json!({"stateRevision":state.revision});
                if let Some(value) = state.settings.get(key) {
                    result["value"] = wire(value);
                }
                result
            }
            "profile.set" => {
                let key = args["key"].as_str().ok_or("profile key")?;
                if !profile_key(key) {
                    return Err("Unprivileged profile key".into());
                }
                crate::settings::validate_profile(key, &args["value"])?;
                return Ok(NativeCommand::State(state::StateTransaction {
                    expected_revision: Some(self.kernel.state().revision),
                    ops: vec![state::StateOp::Setting {
                        key: key.into(),
                        value: Some(model(&args["value"])?),
                    }],
                }));
            }
            "export.preset.save" => {
                let state = self.kernel.state();
                let presets = crate::delivery::save_preset(
                    state.settings.get("renderPresets").map(wire),
                    args["name"].as_str().ok_or("Preset name")?,
                    args["options"].clone(),
                )?;
                return Ok(NativeCommand::State(state::StateTransaction {
                    expected_revision: Some(state.revision),
                    ops: vec![state::StateOp::Setting {
                        key: "renderPresets".into(),
                        value: Some(model(&presets)?),
                    }],
                }));
            }
            "export.plan" => {
                let options: pm_promedia::export_panel::Options =
                    crate::delivery::options(&args["options"])?;
                options.validate()?;
                self.native.export["options"] = args["options"].clone();
                self.command_reply(command)?
            }
            "export.reset" | "export.closeFailure" => {
                self.native.export["run"] = json!({"phase":"idle"});
                self.command_reply(command)?
            }
            // The sheet's window shows (reveal) or ends (dismiss) on the host.
            "export.reveal" | "export.dismiss" => {
                self.native.platform.push((command.into(), args.clone()));
                self.export_snapshot()?
            }
            "export.begin" | "export.retry" | "export.without" => {
                self.native.export_probe_stamp.clear();
                let options = if command != "export.begin" {
                    self.native.export["options"].clone()
                } else {
                    args["options"].clone()
                };
                let typed: pm_promedia::export_panel::Options = crate::delivery::options(&options)?;
                typed.validate()?;
                self.native.export["options"] = options.clone();
                return Ok(NativeCommand::State(state::StateTransaction {
                    expected_revision: Some(self.kernel.state().revision),
                    ops: vec![state::StateOp::Setting {
                        key: "exportOpts".into(),
                        value: Some(model(&options)?),
                    }],
                }));
            }
            "export.cancel" => {
                self.native.export["run"] = json!({"phase":"cancelling"});
                self.native
                    .platform
                    .push(("export.cancel".into(), Value::Null));
                self.command_reply(command)?
            }
            "effect.reveal" => {
                self.native
                    .platform
                    .push(("activatePanel".into(), json!("layer-effects")));
                Value::Null
            }
            _ => return Err(format!("Unregistered built-in command {command}")),
        };
        Ok(NativeCommand::Value(value))
    }
}
/// A move of media alone is a plain kernel edit; other Project edits restore
/// the reviewed whole-project result under its document guard.
pub(crate) fn project_edit_request(
    edit: pm_panel_media::Edit,
    prepared: &pm_panel_media::PreparedEdit,
    caller: Caller,
) -> Request {
    match edit {
        pm_panel_media::Edit::Move {
            assets,
            folders,
            target,
        } if folders.is_empty() => Request::new(
            caller,
            pm_kernel::Call::Transact {
                label: Some(prepared.label.clone()),
                expected_revision: Some(prepared.stamp.revision),
                body: pm_kernel::WriteBody::Commands(vec![pm_kernel::Command::Media(
                    pm_kernel::MediaCommand::SetAssetFolder {
                        asset_ids: assets,
                        folder: target,
                    },
                )]),
            },
        )
        .with_document(prepared.stamp.document),
        _ => prepared.request(caller),
    }
}
fn profile_key(key: &str) -> bool {
    matches!(
        key,
        "themeMode" | "previewMemoryMiB" | "autoDownloadCloudMedia" | "restoreWindows"
    )
}

/// A property's timeline readout, the same reading as its Properties field
/// (two places, trailing zeros trimmed): 960 and 1372.84, never 960.00.
fn readout(value: f64) -> String {
    let text = format!("{value:.2}");
    let text = text.trim_end_matches('0').trim_end_matches('.');
    if text == "-0" {
        "0".into()
    } else {
        text.into()
    }
}

/// `workspace.chrome.timeline.showLayerNumbers` (v1–3 and v4
/// `timeline.chrome`): only an explicit false hides the timeline's layer
/// numbers. v1's shipped workspaces all write true; v1.2.0 showed them when
/// the field was missing.
pub(crate) fn layer_numbers(p: &Published) -> bool {
    p.workspace
        .get("chrome")
        .and_then(|c| c.get("timeline"))
        .and_then(|t| t.get("showLayerNumbers"))
        .is_none_or(|v| *v != pm_model::Value::Bool(false))
}

/// The home folder `~/` paths in replies resolve against.
fn reply_home() -> std::path::PathBuf {
    std::env::var_os("HOME").map(Into::into).unwrap_or_default()
}
/// The media an agent's prose names: an assistant message, or a text step of
/// a run's trace (one row each in the panel); none for anything else.
fn reply_media(
    row: &Value,
    home: &std::path::Path,
) -> Vec<pm_services::surfaces::agent_reply::ReplyMedia> {
    let prose = (row["role"] == "assistant" && row["error"] != true) || row["kind"] == "text";
    if !prose {
        return Vec::new();
    }
    pm_services::surfaces::agent_reply::reply_media(row["text"].as_str().unwrap_or_default(), home)
}
/// Every row of a stored message that can carry agent prose.
fn prose_rows(message: &Value) -> Vec<&Value> {
    match message["steps"].as_array() {
        Some(steps) if message["role"] == "trace" => steps.iter().collect(),
        _ => vec![message],
    }
}

/// The Mods records (S105) of the extensions the kernel knows: built-ins and
/// what this Mac installed or made, with their health as the host saw it.
pub(crate) fn mod_records(
    state: &pm_kernel::state::StateSnapshot,
) -> pm_services::surface_backing::Records {
    let entries = state
        .extensions
        .values()
        .filter(|e| !archived(e))
        .map(|e| {
            let host = e
                .source
                .get("hostRecord")
                .map(|h| serde_json::from_str::<Value>(&pm_model::json::to_canonical_string(h)))
                .and_then(Result::ok)
                .unwrap_or(Value::Null);
            let stopped = host["stopped"] == true;
            json!({"id":e.id,"scope":if e.installed { "user" } else { "builtin" },
                "enabled":e.enabled && !stopped,"health":host["health"],"manifest":{}})
        })
        .collect();
    pm_services::surface_backing::Records {
        revision: state.revision,
        entries,
    }
}
/// Deleted from Settings: the folder is in the archive, its values are kept.
fn archived(entry: &pm_kernel::state::ExtensionEntry) -> bool {
    entry
        .source
        .get("hostRecord")
        .and_then(|r| r.get("archived"))
        .and_then(pm_model::Value::as_bool)
        == Some(true)
}

#[cfg(test)]
mod store_tests {
    use super::*;
    use pm_agents::persistence::Store;
    #[test]
    fn readouts_read_like_the_properties_field() {
        assert_eq!(readout(960.), "960");
        assert_eq!(readout(1372.8427), "1372.84");
        assert_eq!(readout(0.5), "0.5");
        assert_eq!(readout(-0.001), "0");
    }
    #[test]
    fn application_agent_archive_is_disjoint_and_preserves_unknown_fields() {
        let actor = KernelActor::spawn(KernelConfig::default());
        let mut application = KernelStore {
            kernel: actor.handle(),
            application: true,
        };
        let mut project = KernelStore {
            kernel: actor.handle(),
            application: false,
        };
        let key = "agentThreads.builtin-application";
        project.set(key, &json!({"realProject":"keep"})).unwrap();
        application
            .set("future", &json!({"authored":"日本🙂"}))
            .unwrap();
        application
            .set(key, &json!({"application":"separate"}))
            .unwrap();
        assert_eq!(
            project.get(key).unwrap(),
            Some(json!({"realProject":"keep"}))
        );
        assert_eq!(
            application.get(key).unwrap(),
            Some(json!({"application":"separate"}))
        );
        assert_eq!(
            application.get("future").unwrap(),
            Some(json!({"authored":"日本🙂"}))
        );
        project
            .set(APPLICATION_AGENT_STORE, &json!(["unknown future shape"]))
            .unwrap();
        assert!(application.set(key, &json!({})).is_err());
        assert_eq!(
            project.get(APPLICATION_AGENT_STORE).unwrap(),
            Some(json!(["unknown future shape"]))
        );
    }
}
