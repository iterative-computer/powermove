//! S102. Tag: core/generated-bindings.ts and GeneratedPanel.svelte.
//! This module routes source edits; it never evaluates scripts or simulates a preview.
use super::{Session, Ticket, invalid, stale, ticket, unavailable};
use crate::Result;
use serde_json::{Value, json};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Binding {
    Source { target: String, path: String },
    Local(String),
    Parameter(String),
    Button,
    Readout(String),
    Unsupported(String),
}
pub fn binding(control: &Value) -> Result<Binding> {
    let kind = control["type"]
        .as_str()
        .ok_or_else(|| invalid("missing generated control type"))?;
    if kind == "button" {
        return Ok(Binding::Button);
    }
    if kind == "readout" {
        return Ok(Binding::Readout(
            control["source"]
                .as_str()
                .unwrap_or("selection.summary")
                .into(),
        ));
    }
    if ![
        "slider", "text", "color", "fill", "toggle", "select", "curve",
    ]
    .contains(&kind)
    {
        return Ok(Binding::Unsupported(kind.into()));
    }
    let target = control["target"]
        .as_str()
        .filter(|v| !v.is_empty())
        .or_else(|| control["binding"]["target"].as_str());
    let path = control["path"]
        .as_str()
        .filter(|v| !v.is_empty())
        .or_else(|| control["binding"]["path"].as_str());
    if let (Some(target), Some(path)) = (target, path) {
        return Ok(Binding::Source {
            target: target.into(),
            path: path.into(),
        });
    }
    if let Some(key) = control["stateKey"].as_str().filter(|s| !s.is_empty()) {
        return Ok(Binding::Local(key.into()));
    }
    Ok(Binding::Parameter(
        control["param"]
            .as_str()
            .filter(|s| !s.is_empty())
            .or_else(|| control["label"].as_str())
            .ok_or_else(|| invalid("missing parameter name"))?
            .into(),
    ))
}
pub fn strip_manifest_commands(commands: &Value) -> Result<Vec<Value>> {
    let commands = commands
        .as_array()
        .ok_or_else(|| invalid("commands must be an array"))?;
    commands
        .iter()
        .map(|command| {
            let mut clean = command
                .as_object()
                .cloned()
                .ok_or_else(|| invalid("command must be an object"))?;
            for key in ["overrideLock", "preserveHandEdits", "markIntent"] {
                clean.remove(key);
            }
            Ok(Value::Object(clean))
        })
        .collect()
}

pub trait GeneratedSource {
    /// Resolve `$selection` and names against this immutable published session
    /// on every call, not at panel mount. Must return a stable layer id.
    fn layer_id(&self, session: &Session, target: &str) -> Result<String>;
    /// Composition aliases/work area and fill normalization use the shared kernel
    /// edit adapter. Missing support is Unavailable, never a scene parameter fallback.
    fn composition_command(&self, path: &str, value: Value) -> Result<Value>;
}
pub fn edit_command(
    source: &impl GeneratedSource,
    session: &Session,
    binding: &Binding,
    value: Value,
    time: f64,
) -> Result<Value> {
    if !time.is_finite() {
        return Err(invalid("invalid playhead"));
    }
    match binding {
        Binding::Parameter(name) => {
            Ok(json!({"type":"set_scene_parameter", "name": name, "value": value}))
        }
        Binding::Source { target, path }
            if ["$composition", "composition"].contains(&target.as_str()) =>
        {
            source.composition_command(path, value)
        }
        Binding::Source { target, path } => {
            let target = source.layer_id(session, target)?;
            if let Some(key) = path.strip_prefix("content.") {
                return Ok(json!({"type":"set_content","target":target,"patch":{key:value}}));
            }
            if let Some(key) = path.strip_prefix("layer.") {
                return Ok(json!({"type":"set_layer","target":target,"patch":{key:value}}));
            }
            if let Some(channel) = path
                .strip_prefix("properties.")
                .or_else(|| path.strip_prefix("transform."))
            {
                return Ok(
                    json!({"type":"set_property","target":target,"path":channel,"value":value,"time":time,"mode":"auto","preserveHandEdits":false,"markIntent":"human"}),
                );
            }
            Err(unavailable(
                "unsupported source binding; retained for compatibility host",
            ))
        }
        _ => Err(invalid("control is not a source edit")),
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum ButtonEffect {
    Tool {
        action: Value,
        state: Value,
        apply: bool,
    },
    Undo,
    Reset,
    Commands(Vec<Value>),
    Command(String),
    AgentPrompt(Value),
    None,
}
pub fn button_effect(control: &Value, state: &Value) -> Result<ButtonEffect> {
    let action = &control["action"];
    match action["type"].as_str() {
        Some("transform" | "easing" | "script") => {
            let apply = match action["mode"].as_str() {
                Some("apply") => true,
                Some("preview") => false,
                _ => return Err(invalid("invalid tool mode")),
            };
            Ok(ButtonEffect::Tool {
                action: action.clone(),
                state: state.clone(),
                apply,
            })
        }
        Some("history") if action["command"] == "undo" => Ok(ButtonEffect::Undo),
        Some("reset") => Ok(ButtonEffect::Reset),
        Some(_) => Err(unavailable("unrecognized generated action retained")),
        None => {
            if let Some(commands) = control.get("commands") {
                return strip_manifest_commands(commands).map(ButtonEffect::Commands);
            }
            if let Some(cmd) = control["cmd"].as_str().filter(|s| !s.is_empty()) {
                return Ok(ButtonEffect::Command(cmd.into()));
            }
            if let Some(prompt) = control.get("prompt").filter(|v| !v.is_null()) {
                return Ok(ButtonEffect::AgentPrompt(prompt.clone()));
            }
            Ok(ButtonEffect::None)
        }
    }
}
#[derive(Clone, Debug, PartialEq)]
pub struct ToolRequest {
    pub ticket: Ticket,
    pub session: Session,
    pub revision: u64,
    pub effect: ButtonEffect,
}
pub trait GeneratedHost {
    /// Host must enforce target/keyframe/script capabilities, revision, hand-edit
    /// protection and one Undo step for apply. Script execution stays off-main.
    fn run(&mut self, request: &ToolRequest) -> Result<Value>;
    fn save_local(&mut self, panel_id: &str, state: &Value) -> Result<()>;
}
pub struct GeneratedPanel {
    pub definition: Value,
    pub state: Value,
    defaults: Value,
    pub preview: Option<Value>,
    active: Option<ToolRequest>,
    dirty: bool,
}
impl GeneratedPanel {
    pub fn new(definition: Value) -> Result<Self> {
        super::id(&definition)?;
        if !definition["controls"].is_array() {
            return Err(invalid("missing generated controls"));
        }
        let state = definition
            .get("state")
            .cloned()
            .unwrap_or_else(|| json!({}));
        if !state.is_object() {
            return Err(invalid("invalid local state"));
        }
        Ok(Self {
            definition,
            defaults: state.clone(),
            state,
            preview: None,
            active: None,
            dirty: false,
        })
    }
    pub fn set_local(&mut self, key: &str, value: Value) -> Result<()> {
        self.state
            .as_object_mut()
            .ok_or_else(|| invalid("invalid state"))?
            .insert(key.into(), value);
        self.dirty = true;
        self.preview = None;
        // A result for an older tool state must never be adopted/applied.
        self.active = None;
        Ok(())
    }
    pub fn reset(&mut self) {
        self.state = self.defaults.clone();
        self.dirty = true;
        self.preview = None;
        self.active = None;
    }
    /// Called after the tagged 140 ms debounce AND on unmount/switch/shutdown.
    /// Dirty state is retained on worker failure (N4).
    pub fn flush_local(&mut self, host: &mut impl GeneratedHost) -> Result<()> {
        if self.dirty {
            host.save_local(super::id(&self.definition)?, &self.state)?;
            self.dirty = false;
        }
        Ok(())
    }
    pub fn begin(
        &mut self,
        session: Session,
        revision: u64,
        control: &Value,
    ) -> Result<ToolRequest> {
        if self.active.is_some() {
            return Err(unavailable("generated action already running"));
        }
        let request = ToolRequest {
            ticket: ticket(),
            session,
            revision,
            effect: button_effect(control, &self.state)?,
        };
        self.active = Some(request.clone());
        self.preview = None;
        Ok(request)
    }
    pub fn settle(
        &mut self,
        request: &ToolRequest,
        current: &Session,
        revision: u64,
        result: Result<Value>,
    ) -> Result<()> {
        if self.active.as_ref() != Some(request) {
            return Err(stale());
        }
        self.active = None;
        if &request.session != current || request.revision != revision {
            return Err(stale());
        }
        // Worker failure is propagated, not reformatted into a successful preview.
        self.preview = Some(result?);
        Ok(())
    }
}
