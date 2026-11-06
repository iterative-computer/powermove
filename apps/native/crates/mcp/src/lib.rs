//! Agent-native access without GPUI, Electron, Node or a JS runtime.
//!
//! `Automation` owns a single `pm_kernel::Kernel` on a worker. Embedders that
//! already own a kernel actor use `ActorAutomation` (async, no second writer).
//! Missing actor operations are explicit integration requirements; see mcp.md.
pub mod automation;
pub mod catalog;
pub mod commands;
pub mod creative;
pub mod effect;
pub mod export_jobs;
pub mod extensions;
pub mod inspection;
pub mod media;
pub mod media_tools;
pub mod memory;
pub mod memory_admission;
pub mod registration;
pub mod registration_cli;
pub mod scene3d;
pub mod services;
pub mod session;
pub mod store;
pub mod strips;
pub mod transport;
mod validation;
pub mod video;
pub mod views;

pub use automation::{
    ActorAutomation, Automation, AutomationEvent, CaptureRequest, EngineHook, SurfaceHook,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
pub use session::McpSession;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Error {
    pub code: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub backing: Option<String>,
}
pub type Result<T> = std::result::Result<T, Error>;
impl Error {
    pub fn invalid(message: impl Into<String>) -> Self {
        Self {
            code: "invalid_arguments".into(),
            message: message.into(),
            backing: None,
        }
    }
    pub fn unavailable(backing: impl Into<String>) -> Self {
        let backing = backing.into();
        Self {
            code: "not_available_in_this_build".into(),
            message: format!("Not available in this build: {backing}"),
            backing: Some(backing),
        }
    }
    pub fn kernel(e: impl std::fmt::Debug) -> Self {
        Self {
            code: "kernel_error".into(),
            message: format!("{e:?}"),
            backing: Some("pm-kernel".into()),
        }
    }
    pub fn conflict(message: impl Into<String>) -> Self {
        Self {
            code: "revision_conflict".into(),
            message: message.into(),
            backing: Some("pm-kernel".into()),
        }
    }
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}
impl std::error::Error for Error {}
impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Self {
            code: "io_error".into(),
            message: e.to_string(),
            backing: None,
        }
    }
}
pub fn success(value: Value) -> Value {
    success_model(&model(&value))
}
/// MCP text contains JSON, whose escapes preserve every JavaScript UTF-16 unit.
pub fn success_model(value: &pm_model::Value) -> Value {
    json!({"content":[{"type":"text","text":pm_model::json::to_canonical_string(value)}],"isError":false})
}
pub fn failure(error: Error) -> Value {
    // The frozen stdio host sends Error.message verbatim. Structured diagnostics
    // stay on dispatch/Result; wrapping them in text changes agent-visible errors.
    json!({"content":[{"type":"text","text":error.message}],"isError":true})
}
/// `Object.values(assets)`: authored order, as the Project panel lists them.
pub(crate) fn assets_in_order(project: &Value) -> Vec<&Value> {
    use pm_kernel::asset_order::{FIELD, arrange};
    let Some(assets) = project["assets"].as_object() else {
        return Vec::new();
    };
    let keys = assets.keys().map(|id| id.as_str().into()).collect();
    arrange(keys, &|id| {
        project[FIELD].get(id.as_str()?).and_then(Value::as_f64)
    })
    .iter()
    .filter_map(|id| assets.get(id.as_str()?))
    .collect()
}

pub(crate) fn model(value: &Value) -> pm_model::Value {
    pm_model::json::parse(&value.to_string()).expect("valid JSON")
}
pub(crate) fn json_value(value: &pm_model::Value) -> Value {
    serde_json::from_str(&pm_model::json::to_canonical_string(value)).expect("valid model JSON")
}
