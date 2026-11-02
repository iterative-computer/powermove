//! Kernel-owned facts and contribution metadata. No DOM, JS closures or runtime.
use crate::{
    library::{LibraryError, LibraryStore, STATIC_KEYS},
    tx::Writer,
};
use pm_model::json::{Value, parse};
use std::{collections::BTreeMap, sync::Arc};

#[derive(Clone, Debug, PartialEq)]
pub struct UiState {
    pub tool: Value,
    pub timeline: Value,
    pub viewer: Value,
    pub transport: Value,
    pub session: Value,
}
impl Default for UiState {
    fn default() -> Self {
        Self {
    tool: parse(r#"{"tool":"select","toolShape":"rect"}"#).unwrap(),
    timeline: parse(r#"{"graph":false,"pps":100,"scrollY":0,"scrollT":0,"keySelectionActive":false}"#).unwrap(),
    viewer: parse(r#"{"pan":[0,0],"zoom":1,"fit":true,"shown":0,"showControls":true,"activePath":null,"textSelection":null,"canvasTextEditing":null,"preview":null}"#).unwrap(),
    transport: parse(r#"{"time":0,"playing":false}"#).unwrap(), session: Value::Object(Default::default()),
}
    }
}
#[derive(Clone, Debug, PartialEq)]
pub struct Registration {
    pub token: String,
    pub owner: Writer,
    pub id: String,
    pub definition: Value,
}
pub const REGISTRIES: &[&str] = &[
    "panels",
    "inspectorSections",
    "commands",
    "keybindings",
    "effects",
    "transitions",
    "layerTypes",
    "themes",
    "status",
    "services",
    "menus",
    "palette",
];
#[derive(Clone, Debug, PartialEq)]
pub struct ExtensionEntry {
    pub id: String,
    pub installed: bool,
    pub enabled: bool,
    pub version: String,
    pub source: Value,
}
#[derive(Clone, Debug, Default, PartialEq)]
pub struct StateSnapshot {
    pub revision: u64,
    /// Exact v1.2 keys, including workspaces (array), workspace/defaultWorkspace
    /// (ids), workspaceTrash (array). Unknown fields inside values are opaque.
    pub settings: Arc<BTreeMap<String, Value>>,
    pub ui: Arc<BTreeMap<String, UiState>>,
    /// Ordered stacks; last provider wins. Disposal reveals previous providers.
    pub registries: Arc<BTreeMap<String, Vec<Registration>>>,
    pub extensions: Arc<BTreeMap<String, ExtensionEntry>>,
}
impl StateSnapshot {
    pub fn load(store: &LibraryStore) -> Result<Self, LibraryError> {
        let mut s = Self::default();
        for key in STATIC_KEYS {
            if is_setting(key)
                && let Some(v) = store.read(key)?
            {
                Arc::make_mut(&mut s.settings).insert(key.into(), v);
            }
        }
        // Per-provider model preferences and opaque per-extension storage use
        // their existing dynamic library keys. Never flatten or discard them.
        for k in store.keys()? {
            if let crate::library::StoreKey::Dynamic { prefix, id } = k
                && matches!(
                    prefix.as_str(),
                    "agentModel" | "agentThreads" | "agentAttachment" | "ext"
                )
            {
                let key = format!("{prefix}.{id}");
                if let Some(v) = store.read(&key)? {
                    Arc::make_mut(&mut s.settings).insert(key, v);
                }
            }
        }
        if let Some(m) = s.settings.get("extensions").and_then(Value::as_object) {
            for (id, enabled) in m {
                Arc::make_mut(&mut s.extensions).insert(
                    id.to_string_lossy(),
                    ExtensionEntry {
                        id: id.to_string_lossy(),
                        installed: false,
                        enabled: enabled.as_bool().unwrap_or(true),
                        version: String::new(),
                        source: Value::Null,
                    },
                );
            }
        }
        Ok(s)
    }
    pub fn provider(&self, registry: &str, id: &str) -> Option<&Registration> {
        self.registries
            .get(registry)?
            .iter()
            .rev()
            .find(|r| r.id == id)
    }
    pub fn stage(&self, tx: StateTransaction) -> Result<(Self, Vec<StateChange>), String> {
        if tx.expected_revision.is_some_and(|r| r != self.revision) {
            return Err("Kernel state changed".into());
        }
        let mut next = self.clone();
        for op in tx.ops {
            match op {
                StateOp::Setting { key, value } => {
                    if !is_setting(&key) {
                        return Err(format!("Not a settings key: {key}"));
                    }
                    if (key == "workspaces" || key == "workspaceTrash")
                        && value.as_ref().is_some_and(|v| v.as_array().is_none())
                    {
                        return Err("Workspaces must be an array".into());
                    }
                    match value {
                        Some(v) => {
                            Arc::make_mut(&mut next.settings).insert(key, v);
                        }
                        None => {
                            Arc::make_mut(&mut next.settings).remove(&key);
                        }
                    }
                }
                StateOp::Ui { project, next: ui } => {
                    if ![
                        &ui.tool,
                        &ui.timeline,
                        &ui.viewer,
                        &ui.transport,
                        &ui.session,
                    ]
                    .iter()
                    .all(|v| v.as_object().is_some())
                    {
                        return Err("UI facts must be objects".into());
                    }
                    Arc::make_mut(&mut next.ui).insert(project, ui);
                }
                StateOp::Register { registry, entry } => {
                    if !REGISTRIES.contains(&registry.as_str())
                        || entry.id.is_empty()
                        || entry.token.is_empty()
                    {
                        return Err("Invalid registration".into());
                    }
                    if next
                        .registries
                        .values()
                        .flatten()
                        .any(|r| r.token == entry.token)
                    {
                        return Err("Registration token already exists".into());
                    }
                    Arc::make_mut(&mut next.registries)
                        .entry(registry)
                        .or_default()
                        .push(entry);
                }
                StateOp::Dispose { token } => {
                    for entries in Arc::make_mut(&mut next.registries).values_mut() {
                        entries.retain(|r| r.token != token);
                    }
                }
                StateOp::DisposeOwner { owner } => {
                    for entries in Arc::make_mut(&mut next.registries).values_mut() {
                        entries.retain(|r| r.owner != owner);
                    }
                }
                StateOp::Extension(entry) => {
                    if entry.id.is_empty() {
                        return Err("Extension id is required".into());
                    }
                    let mut enabled = next
                        .settings
                        .get("extensions")
                        .and_then(Value::as_object)
                        .cloned()
                        .unwrap_or_default();
                    enabled.insert(entry.id.clone(), Value::Bool(entry.enabled));
                    Arc::make_mut(&mut next.settings)
                        .insert("extensions".into(), Value::Object(enabled));
                    Arc::make_mut(&mut next.extensions).insert(entry.id.clone(), entry);
                }
            }
        }
        if next.settings != self.settings
            && let Some(enabled) = next.settings.get("extensions").and_then(Value::as_object)
        {
            for (id, v) in enabled {
                if let Some(e) = Arc::make_mut(&mut next.extensions).get_mut(&id.to_string_lossy())
                    && let Some(b) = v.as_bool()
                {
                    e.enabled = b;
                }
            }
        }
        let mut changes = Vec::new();
        for key in self
            .settings
            .keys()
            .chain(next.settings.keys())
            .collect::<std::collections::BTreeSet<_>>()
        {
            if self.settings.get(key) != next.settings.get(key) {
                changes.push(StateChange::Setting {
                    key: key.clone(),
                    value: next.settings.get(key).cloned(),
                });
            }
        }
        for (id, ui) in next.ui.iter() {
            if self.ui.get(id) != Some(ui) {
                changes.push(StateChange::Ui {
                    project: id.clone(),
                });
            }
        }
        if self.registries != next.registries {
            changes.push(StateChange::Registries);
        }
        if self.extensions != next.extensions {
            changes.push(StateChange::Extensions);
        }
        if !changes.is_empty() {
            next.revision += 1;
        }
        Ok((next, changes))
    }
}
pub fn is_setting(key: &str) -> bool {
    (STATIC_KEYS.contains(&key)
        && !["projects", "projectTrash", "autosave", "takes"].contains(&key))
        || matches!(crate::library::parse_key(key), Some(crate::library::StoreKey::Dynamic { prefix, .. }) if matches!(prefix.as_str(), "agentModel" | "agentThreads" | "agentAttachment" | "ext"))
}
#[derive(Clone, Debug, Default)]
pub struct StateTransaction {
    pub expected_revision: Option<u64>,
    pub ops: Vec<StateOp>,
}
#[derive(Clone, Debug)]
pub enum StateOp {
    Setting {
        key: String,
        value: Option<Value>,
    },
    Ui {
        project: String,
        next: UiState,
    },
    Register {
        registry: String,
        entry: Registration,
    },
    Dispose {
        token: String,
    },
    DisposeOwner {
        owner: Writer,
    },
    Extension(ExtensionEntry),
}
#[derive(Clone, Debug, PartialEq)]
pub enum StateChange {
    Setting { key: String, value: Option<Value> },
    Ui { project: String },
    Registries,
    Extensions,
}
#[derive(Clone, Debug)]
pub struct StateAck {
    pub revision: u64,
    pub changes: Vec<StateChange>,
}

impl StateSnapshot {
    pub(crate) fn durable_value(&self) -> Value {
        use pm_model::json::Map;
        let settings: Map = self
            .settings
            .iter()
            .map(|(k, v)| (k.clone().into(), v.clone()))
            .collect();
        let extensions = self
            .extensions
            .values()
            .map(|e| {
                let mut m = Map::new();
                m.insert("id", Value::from(e.id.clone()));
                m.insert("installed", Value::Bool(e.installed));
                m.insert("enabled", Value::Bool(e.enabled));
                m.insert("version", Value::from(e.version.clone()));
                m.insert("source", e.source.clone());
                Value::Object(m)
            })
            .collect();
        let mut root = Map::new();
        root.insert("settings", Value::Object(settings));
        root.insert("extensions", Value::Array(extensions));
        Value::Object(root)
    }
    pub(crate) fn restore_durable(&mut self, v: &Value) -> Result<(), String> {
        let settings = v
            .get("settings")
            .and_then(Value::as_object)
            .ok_or("Invalid kernel settings record")?;
        if settings
            .keys()
            .any(|key| !is_setting(&key.to_string_lossy()))
        {
            return Err("Invalid kernel settings key".into());
        }
        self.settings = Arc::new(
            settings
                .iter()
                .map(|(k, v)| (k.to_string_lossy(), v.clone()))
                .collect(),
        );
        for e in v
            .get("extensions")
            .and_then(Value::as_array)
            .ok_or("Invalid extension registry record")?
        {
            let id = e
                .get("id")
                .and_then(Value::as_str)
                .ok_or("Invalid extension id")?
                .to_owned();
            Arc::make_mut(&mut self.extensions).insert(
                id.clone(),
                ExtensionEntry {
                    id,
                    installed: e
                        .get("installed")
                        .and_then(Value::as_bool)
                        .ok_or("Invalid install state")?,
                    enabled: e
                        .get("enabled")
                        .and_then(Value::as_bool)
                        .ok_or("Invalid enable state")?,
                    version: e
                        .get("version")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .into(),
                    source: e.get("source").cloned().unwrap_or(Value::Null),
                },
            );
        }
        Ok(())
    }
}

impl UiState {
    pub fn from_session(session: Value) -> Self {
        let mut ui = Self {
            session,
            ..Default::default()
        };
        if let Some(timeline) = ui
            .session
            .get("timeline")
            .filter(|v| v.as_object().is_some())
        {
            ui.timeline = timeline.clone();
        }
        if let Some(tool) = ui.session.get("tool").filter(|v| v.as_object().is_some()) {
            ui.tool = tool.clone();
        }
        if let Some(viewer) = ui.session.get("view").filter(|v| v.as_object().is_some()) {
            ui.viewer = viewer.clone();
        }
        if let Some(time) = ui.session.get("time").cloned()
            && let Value::Object(m) = &mut ui.transport
        {
            m.insert("time", time);
        }
        ui
    }
    pub fn to_session(&self) -> Value {
        let mut m = self.session.as_object().cloned().unwrap_or_default();
        m.insert("timeline", self.timeline.clone());
        m.insert("tool", self.tool.clone());
        m.insert("view", self.viewer.clone());
        if let Some(time) = self.transport.get("time") {
            m.insert("time", time.clone());
        }
        Value::Object(m)
    }
}
