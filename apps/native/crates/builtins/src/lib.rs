//! Application v4 embedding. JavaScript owns presentation; the kernel remains
//! the only document writer. GPUI consumes immutable accepted surfaces.
#![cfg_attr(test, recursion_limit = "512")]
mod agent_host;
#[cfg(test)]
mod agent_tests;
mod data;
pub mod delivery;
pub mod media_import;
mod models;
mod properties;
mod settings;
mod timeline_easing;
pub mod timeline_scene;
pub mod virtualization;
use data::Backend;
use pm_kernel::KernelHandle;
use pm_runtime_v8::*;
use pm_surface::{
    Surface, events::EventDetail, host_api::*, host_data::HostDataService, interaction::Interaction,
};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};

pub const LIVE_PANELS: &[&str] = &[
    "timeline",
    "inspector",
    "layer-effects",
    "viewer",
    "assets",
    "fxbrowser",
    "agent",
    "settings",
    "export",
];
const BUNDLE: &str = include_str!(concat!(env!("OUT_DIR"), "/builtins.js"));

#[derive(Clone, Debug)]
pub struct Publication {
    pub panel: String,
    pub generation: u64,
    pub surface: Arc<Surface>,
    pub timeline: Option<Arc<timeline_scene::Scene>>,
}
#[derive(Clone, Debug)]
pub struct PlatformRequest {
    pub id: u64,
    pub panel: String,
    pub generation: u64,
    pub operation: String,
    pub input: Value,
}
struct Entry {
    generation: u64,
    backend: Arc<Mutex<Backend>>,
    router: HostRouter<HostDataService<data::Data>>,
    surface: Surface,
    interaction: Interaction,
}
/// Completed byte receipts stay native; the JSX panel receives metadata only.
#[derive(Clone)]
pub struct RetainedAsset {
    pub id: String,
    pub metadata: pm_model::Value,
    pub original: pm_kernel::media_archive::Original,
    pub previous: Option<pm_kernel::media_archive::Original>,
}

/// One host/pool per project window. A realm and native state exist separately
/// for every extension; there is no editor-isolate shortcut for v4 built-ins.
pub struct Builtins {
    pool: RuntimePool,
    broker: Arc<Broker>,
    realms: BTreeMap<String, Realm>,
}
struct Broker {
    project: String,
    application: bool,
    kernel: KernelHandle,
    entries: Mutex<BTreeMap<String, Entry>>,
    publications: Mutex<BTreeMap<String, Publication>>,
    platform: Mutex<Vec<PlatformRequest>>,
    errors: Mutex<Vec<String>>,
    facts: Mutex<Value>,
    platform_replies: Mutex<BTreeMap<u64, Value>>,
    timeline_requests: Mutex<BTreeMap<u64, (String, u64)>>,
    platform_ids: std::sync::atomic::AtomicU64,
    wake: Arc<dyn Fn() + Send + Sync>,
    agent: Arc<agent_host::AgentLink>,
}
impl Builtins {
    pub fn new(
        kernel: KernelHandle,
        project: &str,
        wake: impl Fn() + Send + Sync + 'static,
    ) -> Result<Self> {
        Self::create(kernel, project, false, wake)
    }
    /// Application-owned Agent/Settings without inventing an open document.
    pub fn application(
        kernel: KernelHandle,
        wake: impl Fn() + Send + Sync + 'static,
    ) -> Result<Self> {
        Self::create(kernel, "builtin-application", true, wake)
    }
    fn create(
        kernel: KernelHandle,
        project: &str,
        application: bool,
        wake: impl Fn() + Send + Sync + 'static,
    ) -> Result<Self> {
        let broker = Arc::new(Broker {
            project: project.into(),
            application,
            kernel,
            entries: Default::default(),
            publications: Default::default(),
            platform: Default::default(),
            errors: Default::default(),
            facts: Mutex::new(json!({})),
            platform_replies: Default::default(),
            timeline_requests: Default::default(),
            platform_ids: std::sync::atomic::AtomicU64::new(1),
            wake: Arc::new(wake),
            agent: Default::default(),
        });
        let pool = RuntimePool::new(
            format!("window:{project}"),
            Config {
                limits: Limits::default(),
                watchdog: WatchdogPolicy::default(),
                kernel: broker.clone(),
                host: broker.clone(),
                watchdog_hooks: broker.clone(),
                engine: Some(broker.clone()),
                cache: CodeCache::new(4 * 1024 * 1024),
                admission: Admission::new(640 * 1024 * 1024),
            },
        )?;
        Ok(Self {
            pool,
            broker,
            realms: BTreeMap::new(),
        })
    }
    /// Ticket is awaited off the UI handler. First publication is delivered only
    /// after Rust accepts the complete atomic surface commit.
    pub fn mount(&mut self, panel: &str) -> Result<Ticket<Evaluation>> {
        if !LIVE_PANELS.contains(&panel)
            || self.realms.contains_key(panel)
            || (self.broker.application && !matches!(panel, "settings" | "agent"))
        {
            return Err(Error::Invalid("unregistered/already mounted built-in"));
        }
        let realm = self.pool.realm(RealmKind::V4Extension(panel.into()))?;
        if matches!(panel, "agent" | "settings") {
            self.broker.agent.add_realm(realm.clone());
        }
        let ticket = realm.eval(format!("{BUNDLE}\n__pmBuiltin.mount({})", json!(panel)))?;
        self.realms.insert(panel.into(), realm);
        Ok(ticket)
    }
    /// What agent runs may use. Without this the panel reports no connection.
    pub fn set_agent_environment(
        &self,
        environment: pm_agents::launch::Environment,
        open: std::sync::Arc<pm_agents::account::Opener>,
    ) {
        self.broker.agent.set_environment(environment, open);
    }
    pub fn publish(&self) -> Vec<(String, Result<Ticket<Evaluation>>)> {
        self.realms
            .iter()
            .map(|(id, realm)| (id.clone(), realm.eval("__pmBuiltin.publish()")))
            .collect()
    }
    pub fn take_publications(&self) -> Vec<Publication> {
        std::mem::take(&mut *self.broker.publications.lock().unwrap())
            .into_values()
            .collect()
    }
    pub fn update_facts(&self, facts: Value) {
        *self.broker.facts.lock().unwrap() = facts.clone();
        for entry in self.broker.entries.lock().unwrap().values_mut() {
            entry.backend.lock().unwrap().native.facts = facts.clone();
        }
    }
    /// Worker-only native observations. Never expose filesystem paths to JSX.
    pub fn update_media(
        &self,
        originals: BTreeMap<String, std::path::PathBuf>,
        observed: pm_panel_media::readiness::ObservedRuntime,
    ) {
        for entry in self.broker.entries.lock().unwrap().values_mut() {
            let mut backend = entry.backend.lock().unwrap();
            backend.native.originals = originals.clone();
            backend.native.media_runtime = observed.clone();
        }
    }
    /// Admit worker-owned originals, then await the actor's durable outcome.
    /// A metadata-only StoredAsset is never sufficient for live admission.
    pub fn admit_media(
        &self,
        owner: &pm_panel_media::Stamp,
        items: Vec<RetainedAsset>,
        folder: Option<String>,
        replace: Option<String>,
        place: Vec<pm_kernel::Command>,
        caller: pm_kernel::Caller,
    ) -> std::result::Result<
        pm_kernel::Pending<std::result::Result<pm_kernel::Ack, pm_kernel::KernelFailure>>,
        String,
    > {
        if self.broker.application || owner.project != self.broker.project {
            return Err("Media completion belongs to a different project".into());
        }
        let published = self
            .broker
            .kernel
            .published(&owner.project)
            .ok_or("Project closed")?;
        let catalog = pm_panel_media::Catalog::published(&published)
            .map_err(|_| "The project's media could not be read")?;
        let mut stored = Vec::new();
        let mut bindings = Vec::new();
        for item in items {
            let id = replace.as_ref().unwrap_or(&item.id).clone();
            stored.push(pm_panel_media::StoredAsset {
                id: item.id,
                metadata: item.metadata,
                original_version: item.original.version().into(),
            });
            let expected = published
                .snapshot
                .root()
                .and_then(|r| r.get("assets"))
                .and_then(|a| a.get(&id))
                .map(pm_model::Node::to_value);
            bindings.push(pm_kernel::MediaBinding {
                asset: id,
                expected,
                original: item.original,
                previous: item.previous,
            });
        }
        let mut edit = catalog
            .prepare_import(owner, stored, folder, replace)
            .map_err(|error| match error {
                pm_panel_media::Error::MissingItem => "That folder is no longer in the project",
                _ => "The import no longer matches the project",
            })?;
        if !place.is_empty() {
            edit.label = match place.len() {
                1 => "Import file".to_owned(),
                n => format!("Import {n} files"),
            };
        }
        Ok(self.broker.kernel.call(
            &owner.project,
            edit.request_with_media(caller, bindings, place),
        ))
    }

    pub fn platform_reply(&self, id: u64, result: std::result::Result<Value, String>) {
        if id == 0 {
            return;
        }
        self.broker.platform_replies.lock().unwrap().insert(
            id,
            match result {
                Ok(value) => json!({"accepted":true,"value":value}),
                Err(error) => json!({"accepted":false,"error":error}),
            },
        );
    }
    pub fn observe_export(&self, run: Value) -> Result<Ticket<Evaluation>> {
        self.realms
            .get("export")
            .ok_or(Error::Closed)?
            .eval(format!(
                "runtime.engine('export.observe',{});__pmBuiltin.publish()",
                run
            ))
    }
    pub fn show(&self, panel: &str) -> Result<Ticket<Evaluation>> {
        self.realms
            .get(panel)
            .ok_or(Error::Closed)?
            .eval("__pmBuiltin.show()")
    }
    /// A shell-admitted mask reveal uses the same model and persistence as M in the timeline.
    pub fn reveal_masks(&self) -> Result<Ticket<Evaluation>> {
        static CLOCK: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
        let millis = CLOCK
            .get_or_init(std::time::Instant::now)
            .elapsed()
            .as_millis() as u64;
        self.realms
            .get("timeline")
            .ok_or(Error::Closed)?
            .eval(format!(
                "__pmBuiltin.command('panel.timeline',{})",
                json!({"type":"keyDown","millis":millis,"event":{"key":"m","repeat":false}})
            ))
    }
    pub fn timeline_key(
        &self,
        layer: &str,
        channel: &str,
        key: &str,
        additive: bool,
    ) -> Result<Ticket<Evaluation>> {
        self.timeline_action(json!({"type":"selectKeys","layer":layer,"path":channel,"ids":[key],"additive":additive}))
    }
    /// Enqueue on the existing v4 worker. The ticket covers actor receipts and
    /// platform acknowledgements; the main thread only polls it. Drivers can
    /// include decimal document/revision stamps from the inspected frame.
    pub fn timeline_action(&self, intent: Value) -> Result<Ticket<Evaluation>> {
        self.realms
            .get("timeline")
            .ok_or(Error::Closed)?
            .eval(format!("__pmBuiltin.command('panel.timeline',{})", intent))
    }

    /// Inspect exact library source and CAS revision on the panel worker.
    pub fn timeline_easing_library(&self) -> Result<Ticket<Evaluation>> {
        self.realms
            .get("timeline")
            .ok_or(Error::Closed)?
            .eval("JSON.stringify(runtime.engine('timeline.easing.library',{}))")
    }
    pub fn write(&self, panel: &str, handle: &str, value: Value) -> Result<Ticket<Evaluation>> {
        self.realms.get(panel).ok_or(Error::Closed)?.eval(format!(
            "__pmBuiltin.write({},{})",
            json!(handle),
            value
        ))
    }
    pub fn take_platform_requests(&self) -> Vec<PlatformRequest> {
        std::mem::take(&mut *self.broker.platform.lock().unwrap())
    }
    pub fn status(&self) -> Vec<Value> {
        self.broker.entries.lock().unwrap().iter().map(|(id,e)|json!({"panel":id,"generation":e.generation,"surfaceRevision":e.surface.revision(),"subscriptions":e.router.subscription_count(),"kernelRevision":e.backend.lock().unwrap().service_revision})).collect()
    }
    pub fn take_errors(&self) -> Vec<String> {
        std::mem::take(&mut *self.broker.errors.lock().unwrap())
    }
    /// Validate the exact tree revision before enqueueing. An old click must not
    /// activate a newly selected layer after a publication or project switch.
    pub fn event(
        &self,
        panel: &str,
        generation: u64,
        revision: u64,
        target: u64,
        event: &str,
        detail: EventDetail,
    ) -> Result<Ticket<Evaluation>> {
        let mut entries = self.broker.entries.lock().unwrap();
        let entry = entries.get_mut(panel).ok_or(Error::Closed)?;
        let event = entry
            .interaction
            .drive(
                &mut entry.surface,
                generation,
                entry.generation,
                revision,
                target,
                event,
                detail,
            )
            .map_err(Error::Host)?;
        let wire = serde_json::to_string(&event).map_err(|e| Error::Host(e.to_string()))?;
        self.realms
            .get(panel)
            .ok_or(Error::Closed)?
            .eval(format!("__pmBuiltin.event({wire})"))
    }
    /// Caller drops the owning window without waiting on JS. Runtime disposal
    /// revokes its writer on the isolate owner, including after a failed mount.
    pub fn close_panel(&self, id: &str) {
        if let Some(realm) = self.realms.get(id) {
            realm.close();
        }
    }
    pub fn close(&self) {
        self.broker.agent.close();
        for realm in self.realms.values() {
            realm.close();
        }
    }
}
impl Drop for Builtins {
    fn drop(&mut self) {
        self.close();
    }
}
impl Broker {
    fn panel<'a>(&self, generation: &'a Generation) -> Result<&'a str> {
        let id = generation
            .realm
            .rsplit_once("V4Extension(\"")
            .and_then(|(_, s)| s.strip_suffix("\")"))
            .ok_or(Error::Invalid("not a v4 built-in realm"))?;
        if !LIVE_PANELS.contains(&id) {
            return Err(Error::Invalid("unknown panel"));
        }
        Ok(id)
    }
    fn report(&self, message: String) {
        let mut errors = self.errors.lock().unwrap();
        if errors.len() < 64 {
            errors.push(message);
        }
        drop(errors);
        (self.wake)();
    }
}
impl EngineProxy for Broker {
    fn call(
        &self,
        generation: &Generation,
        operation: &str,
        input: &Value,
        budget: HostBudget,
    ) -> Result<Value> {
        budget.check()?;
        let panel = self.panel(generation)?;
        let mut entries = self.entries.lock().unwrap();
        if operation == "identity" {
            if entries.contains_key(panel) {
                return Err(Error::Invalid("duplicate panel identity"));
            }
            let identity = Identity {
                realm: generation.realm.clone(),
                generation: generation.number,
                project: self.project.clone(),
            };
            let backend = Arc::new(Mutex::new(
                Backend::new(
                    self.kernel.clone(),
                    identity.clone(),
                    panel,
                    self.application,
                )
                .map_err(Error::Host)?,
            ));
            backend.lock().unwrap().native.agent_link = self.agent.clone();
            let facts = self.facts.lock().unwrap().clone();
            if !facts.is_null() && facts.as_object().is_some_and(|m| !m.is_empty()) {
                backend.lock().unwrap().native.facts = facts;
            }
            let router = HostRouter::new(
                identity.clone(),
                HostDataService::new(identity.clone(), data::Data(backend.clone()))
                    .map_err(Error::Host)?,
            )
            .map_err(Error::Host)?;
            entries.insert(
                panel.into(),
                Entry {
                    generation: generation.number,
                    backend,
                    router,
                    surface: Surface::default(),
                    interaction: Interaction::default(),
                },
            );
            return Ok(serde_json::to_value(identity).unwrap());
        }
        let entry = entries
            .get_mut(panel)
            .filter(|e| e.generation == generation.number)
            .ok_or(Error::StaleGeneration)?;
        match operation {
            "tokens.row" => Ok(json!(
                pm_design::Tokens::new(pm_design::Scheme::Dark, Default::default())
                    .metrics
                    .row
            )),
            "host.enqueue" => {
                entry
                    .router
                    .enqueue(
                        serde_json::from_value(input.clone())
                            .map_err(|e| Error::Host(e.to_string()))?,
                    )
                    .map_err(Error::Host)?;
                Ok(Value::Null)
            }
            "host.poll" => {
                {
                    let mut backend = entry.backend.lock().unwrap();
                    // Drain effect receipts on the panel worker. Main only
                    // deposits replies; it never locks a worker-owned backend.
                    {
                        let mut pending = self.timeline_requests.lock().unwrap();
                        let mut replies = self.platform_replies.lock().unwrap();
                        let ids = pending
                            .iter()
                            .filter(|(_, owner)| owner.0 == panel && owner.1 == generation.number)
                            .map(|(id, _)| *id)
                            .collect::<Vec<_>>();
                        for id in ids {
                            if let Some(reply) = replies.remove(&id) {
                                pending.remove(&id);
                                backend.native.timeline_effect_pending =
                                    backend.native.timeline_effect_pending.saturating_sub(1);
                                if reply["accepted"] != true {
                                    backend.native.timeline_effect_result =
                                        Some(Err(reply["error"]
                                            .as_str()
                                            .unwrap_or("Native timeline effect rejected")
                                            .into()));
                                }
                            }
                        }
                    }
                    backend.tick().map_err(Error::Host)?;
                    let requests = std::mem::take(&mut backend.native.platform);
                    if !requests.is_empty() {
                        let mut queue = self.platform.lock().unwrap();
                        for (operation, input) in requests {
                            let id = if operation == "timeline.effect" {
                                let id = self
                                    .platform_ids
                                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                                self.timeline_requests
                                    .lock()
                                    .unwrap()
                                    .insert(id, (panel.into(), generation.number));
                                id
                            } else {
                                0
                            };
                            queue.push(PlatformRequest {
                                id,
                                panel: panel.into(),
                                generation: generation.number,
                                operation,
                                input,
                            });
                        }
                        (self.wake)();
                    }
                }
                serde_json::to_value(
                    entry
                        .router
                        .poll(input.as_u64().ok_or(Error::Invalid("request id"))?)
                        .map_err(Error::Host)?,
                )
                .map_err(|e| Error::Host(e.to_string()))
            }
            "host.publications" => {
                entry.backend.lock().unwrap().tick().map_err(Error::Host)?;
                serde_json::to_value(entry.router.publications().map_err(Error::Host)?)
                    .map_err(|e| Error::Host(e.to_string()))
            }
            "host.retire" => {
                entry.router.retire().map_err(Error::Host)?;
                Ok(Value::Null)
            }
            "surface.commit" => {
                let commit = serde_json::from_value(input.clone())
                    .map_err(|e| Error::Host(e.to_string()))?;
                match entry.surface.apply(commit) {
                    Ok(_) => {
                        entry.interaction.reconcile(&entry.surface);
                        self.publications.lock().unwrap().insert(
                            panel.into(),
                            Publication {
                                panel: panel.into(),
                                generation: generation.number,
                                surface: Arc::new(entry.surface.clone()),
                                timeline: entry
                                    .backend
                                    .lock()
                                    .unwrap()
                                    .native
                                    .timeline_scene
                                    .clone(),
                            },
                        );
                        (self.wake)();
                        Ok(json!({"accepted":true,"revision":entry.surface.revision()}))
                    }
                    Err(error) => Ok(
                        json!({"accepted":false,"revision":entry.surface.revision(),"error":error.to_string()}),
                    ),
                }
            }
            "export.observe" => {
                let mut backend = entry.backend.lock().unwrap();
                backend.native.export["run"] = input.clone();
                backend.native.export["sequence"] =
                    json!(backend.native.export["sequence"].as_u64().unwrap_or(0) + 1);
                Ok(Value::Null)
            }
            "fields.register" => entry
                .backend
                .lock()
                .unwrap()
                .fields(input)
                .map_err(Error::Host),
            "inspector.snapshot" => entry
                .backend
                .lock()
                .unwrap()
                .inspector_snapshot()
                .map_err(Error::Host),
            "timeline.easing.library" => Ok(timeline_easing::inspect(&self.kernel)),
            "timeline.frame" => entry
                .backend
                .lock()
                .unwrap()
                .timeline_frame(input)
                .map_err(Error::Host),
            "agent.snapshot" => entry
                .backend
                .lock()
                .unwrap()
                .agent_snapshot()
                .map_err(Error::Host),
            "settings.snapshot" => entry
                .backend
                .lock()
                .unwrap()
                .settings_snapshot()
                .map_err(Error::Host),
            "project.layout" => Ok(
                json!({"columns":1,"rowHeight":pm_design::Tokens::new(pm_design::Scheme::Dark,Default::default()).metrics.row}),
            ),
            "platform.enqueue" => {
                let mut requests = self.platform.lock().unwrap();
                if requests.len() >= 128 {
                    return Err(Error::Quota("platform queue"));
                }
                let id = self
                    .platform_ids
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                let (operation, platform_input) = inspector_platform(panel, input)?;
                requests.push(PlatformRequest {
                    id,
                    panel: panel.into(),
                    generation: generation.number,
                    operation: operation.into(),
                    input: platform_input,
                });
                (self.wake)();
                Ok(json!(id))
            }
            "platform.poll" => Ok(self
                .platform_replies
                .lock()
                .unwrap()
                .remove(&input.as_u64().ok_or(Error::Invalid("platform request"))?)
                .unwrap_or(Value::Null)),
            "effects.bindings" => {
                entry.backend.lock().unwrap().local_fields();
                Ok(json!({"search":"effects.search", "scope":"effects.scope"}))
            }
            "effects.registry" => Ok(entry.backend.lock().unwrap().registry()),
            "effects.target" => entry.backend.lock().unwrap().target().map_err(Error::Host),
            "report" => {
                self.report(format!("{panel}: {input}"));
                Ok(Value::Null)
            }
            "platform.activatePanel" | "platform.focus" | "platform.announce" => {
                let mut requests = self.platform.lock().unwrap();
                if requests.len() >= 128 {
                    return Err(Error::Quota("platform queue"));
                }
                requests.push(PlatformRequest {
                    id: 0,
                    panel: panel.into(),
                    generation: generation.number,
                    operation: operation.into(),
                    input: input.clone(),
                });
                (self.wake)();
                Ok(Value::Null)
            }
            // Menus require an asynchronous reply from AppKit. Refuse them until
            // installed; a cancelled menu must never be reported as a choice.
            "platform.effectMenu" | "platform.contextMenu" => {
                Err(Error::Host("native panel menu adapter unavailable".into()))
            }
            _ => Err(Error::Invalid("unknown built-in host operation")),
        }
    }
}
/// Normalize owned panel requests to the existing asynchronous AppKit routes.
fn inspector_platform<'a>(panel: &str, request: &'a Value) -> Result<(&'a str, Value)> {
    let operation = request["operation"]
        .as_str()
        .ok_or(Error::Invalid("platform operation"))?;
    let input = &request["input"];
    if panel == "inspector" && operation == "inspector" {
        match input["type"].as_str() {
            Some("add-effect-menu") => return Ok(("effectMenu", input.clone())),
            Some("animator-menu") => return Ok(("contextMenu", input["items"].clone())),
            Some("row-menu") => {
                let items = input["items"]
                    .as_array()
                    .ok_or(Error::Invalid("row menu"))?;
                let mut options = Vec::new();
                for item in items {
                    if item["separatorBefore"] == true {
                        options.push(Value::Null);
                    }
                    options.push(json!({"id":item["action"],"label":item["label"],
                        "disabled":item["enabled"] != true,"checked":item["checked"]}));
                }
                return Ok(("contextMenu", json!(options)));
            }
            _ => {}
        }
    }
    Ok((operation, input.clone()))
}

impl KernelHooks for Broker {
    fn load_storage(&self, _: &str) -> Result<Value> {
        Ok(json!({}))
    }
    fn commit_activation(
        &self,
        previous: Option<&Generation>,
        next: &Generation,
        ops: &[Value],
        storage: &Value,
    ) -> Result<()> {
        if previous.is_some_and(|old| old != next) || !ops.is_empty() || storage != &json!({}) {
            return Err(Error::Invalid(
                "built-in activation cannot publish untyped edits",
            ));
        }
        Ok(())
    }
    fn commit_retained(&self, _: &Generation, _: &[Value]) -> Result<()> {
        Err(Error::StaleGeneration)
    }
    fn revoke_generation(&self, generation: &Generation) {
        if let Ok(panel) = self.panel(generation) {
            let mut entries = self.entries.lock().unwrap();
            if let Some(entry) = entries
                .get_mut(panel)
                .filter(|e| e.generation == generation.number)
            {
                let _ = entry.router.retire();
            }
        }
    }
    fn seal_open_gestures(&self, _: &Generation) {}
}
impl HostProxy for Broker {
    fn fetch(
        &self,
        _: &Generation,
        _: FetchRequest,
        _: Cancellation,
    ) -> Result<Box<dyn pm_runtime_v8::HostRequest>> {
        Err(Error::Host(
            "built-in network access requires service permission".into(),
        ))
    }
    fn console(&self, _: &Generation, level: &str, message: &str) {
        self.report(format!("{level}: {message}"));
    }
}
impl WatchdogHooks for Broker {
    fn notice(&self, _: &Generation, _: u64) {
        self.report("Built-in panel is taking longer than expected".into());
    }
    fn prompt(&self, _: &Generation, _: u64) -> WatchdogDecision {
        self.report("Built-in panel is still running; waiting for its owner".into());
        WatchdogDecision::Wait
    }
}

#[cfg(test)]
mod tests;
