//! The headless document host interface (contract 15, section 22).
//!
//! On the hidden MCP host (`powermove mcp`, Phase 3c on macOS) host services
//! run in the CLI process and each open project gets its own headless
//! document host worker: pm-wasm, the kernel, history, the agent harness, and
//! the extension registry, persisted to the profile store. The document model
//! stays TypeScript. This contract is the host ↔ worker interface: instances,
//! persistence, the extension runtime, and agent tool routing.

use serde::{Deserialize, Serialize};
#[cfg(feature = "ts")]
use ts_rs::TS;

use crate::common::{AgentRunId, DocumentId, ExtensionId, ProjectId, Revision, Seq};
use crate::host::CapabilityHandle;
use crate::validate::{Validate, Violation, ensure};
use crate::version::ContractId;

/// One document host worker for one project.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct InstanceId(pub u32);

/// Open a project in a new instance (host → worker pool).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct OpenProject {
    pub project: ProjectId,
    /// The project's profile-store entry.
    pub store: CapabilityHandle,
    /// The agent session pinning this project, if any. On the hidden host
    /// each pinned project has its own document host.
    pub pinned_by: Option<AgentRunId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ProjectOpened {
    pub instance: InstanceId,
    pub document: DocumentId,
    pub revision: Revision,
}

/// Why a worker persists.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum PersistReason {
    Autosave,
    ToolCall,
    Close,
    HostShutdown,
}

/// Persist the instance's document and history to the profile store.
/// History is per instance, so concurrent runs on two projects never touch
/// each other's rollback (appendix A, item 16).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct Persist {
    pub instance: InstanceId,
    pub reason: PersistReason,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct Persisted {
    pub instance: InstanceId,
    pub revision: Revision,
    pub bytes: crate::common::ByteSize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct CloseProject {
    pub instance: InstanceId,
}

/// Where an extension comes from. Local and agent-published extensions
/// register in the instance's kernel; Store extensions run in their
/// contained sandbox.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum ExtensionSource {
    Local,
    AgentPublished,
    Store,
}

/// Register (or hot-reload) an extension in an instance.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct RegisterExtension {
    pub instance: InstanceId,
    pub extension: ExtensionId,
    pub version: String,
    pub source: ExtensionSource,
    pub bundle: CapabilityHandle,
}

/// The registration finished and its definition snapshots reached the
/// engine at `definitions_generation` before this reply was sent, so a
/// `render_frames` call right after `publish_extensions` sees them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ExtensionRegistered {
    pub instance: InstanceId,
    pub extension: ExtensionId,
    pub definitions_generation: Seq,
}

/// Where a tool call runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum ToolRoute {
    /// The project's document host instance (pm-wasm, kernel, history, harness).
    DocumentHost,
    /// Needs a real DOM: a Chromium tab started on demand on the pinned project.
    Chromium,
    /// Host services in the CLI process: the project packer, media tools over
    /// the `ffmpeg` executable, the Store client, extension forking and
    /// inspection, and orchestration (the same split as the desktop's
    /// `main/agent-tools/bridge.ts`).
    HostServices,
    /// The job engine (exports and render status).
    JobEngine,
}

/// DOM tools (section 22): the hidden host starts Chromium on demand for
/// them and fails with a named error when Chromium is missing.
pub const DOM_TOOLS: &[&str] = &[
    "capture_panel",
    "computer_use_panel",
    "interact_panel",
    "get_panel_state",
    "open_panel",
    "get_panel_layout",
    "set_panel_layout",
];

/// Tools served by host services. Mirrors what the desktop bridge runs in
/// main today: `save_project` (the packer), `AGENT_MEDIA_TOOL_NAMES`,
/// `POWERMOVE_STORE_TOOL_NAMES`, creative-workspace inspection, extension
/// forking, and `ORCHESTRATION_TOOL_NAMES`.
pub const HOST_TOOLS: &[&str] = &[
    "save_project",
    "probe_media",
    "sample_media_frames",
    "media_contact_sheet",
    "media_waveform",
    "transcribe_media",
    "store_search",
    "store_extension",
    "store_source",
    "store_library",
    "store_install",
    "store_update",
    "store_uninstall",
    "store_publish_prepare",
    "store_publish",
    "inspect_creative_workspace",
    "inspect_creative_extension",
    "fork_builtin_extension",
    "stage_fork_rebase",
    "orchestrator_capabilities",
    "delegate_task",
    "task_status",
    "task_cancel",
    "task_send",
    "thread_launch",
    "thread_list",
    "thread_read",
    "thread_send",
    "thread_watch",
    "thread_unwatch",
    "thread_cancel",
];

/// Tools served by the job engine through host services.
pub const JOB_TOOLS: &[&str] = &["export_video", "get_render_status"];

/// Routes an agent tool by name. Every other tool (document, 3D, media
/// import and replace, captions, `render_frames`, `check_project`) runs in
/// the document host, which calls host services for bytes and the job
/// engine for frames.
pub fn route_tool(name: &str) -> ToolRoute {
    if DOM_TOOLS.contains(&name) {
        ToolRoute::Chromium
    } else if JOB_TOOLS.contains(&name) {
        ToolRoute::JobEngine
    } else if HOST_TOOLS.contains(&name) {
        ToolRoute::HostServices
    } else {
        ToolRoute::DocumentHost
    }
}

/// External MCP limits (section 20) that the hidden host keeps.
pub mod limits {
    pub const AUTH_TIMEOUT_MS: u32 = 10_000;
    pub const MAX_LINE_BYTES: usize = 2 * 1024 * 1024;
    pub const MAX_QUEUED_REQUESTS: usize = 32;
    pub const MAX_RESPONSE_BYTES: usize = 64 * 1024 * 1024;
}

/// An agent tool call routed to an instance. Arguments and results are the
/// tools' existing JSON schemas, unchanged.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ToolCall {
    pub instance: InstanceId,
    pub run: AgentRunId,
    pub call_id: String,
    pub tool: String,
    pub arguments: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ToolResult {
    pub instance: InstanceId,
    pub call_id: String,
    /// The document revision after the call (agents' snapshots read their own writes).
    pub revision: Revision,
    pub content: serde_json::Value,
    pub is_error: bool,
}

impl Validate for ToolCall {
    fn validate(&self) -> Result<(), Violation> {
        const C: ContractId = ContractId::DocumentHost;
        ensure(
            !self.tool.is_empty()
                && self.tool.len() <= 64
                && self
                    .tool
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_'),
            C,
            || format!("tool name {:?} must be snake_case", self.tool),
        )?;
        ensure(
            !self.call_id.is_empty() && self.call_id.len() <= 128,
            C,
            || "call id must be 1 to 128 bytes".into(),
        )?;
        let route = route_tool(&self.tool);
        ensure(route == ToolRoute::DocumentHost, C, || {
            format!("{} routes to {route:?}, not a document host", self.tool)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn routing() {
        assert_eq!(route_tool("capture_panel"), ToolRoute::Chromium);
        assert_eq!(route_tool("set_panel_layout"), ToolRoute::Chromium);
        assert_eq!(route_tool("export_video"), ToolRoute::JobEngine);
        assert_eq!(route_tool("save_project"), ToolRoute::HostServices);
        assert_eq!(route_tool("apply_commands"), ToolRoute::DocumentHost);
        assert_eq!(route_tool("edit_3d"), ToolRoute::DocumentHost);
        assert_eq!(route_tool("render_frames"), ToolRoute::DocumentHost);
        assert_eq!(
            route_tool("import_media_to_timeline"),
            ToolRoute::DocumentHost
        );
        assert_eq!(route_tool("import_media"), ToolRoute::DocumentHost);
        assert_eq!(route_tool("check_project"), ToolRoute::DocumentHost);
        assert_eq!(route_tool("probe_media"), ToolRoute::HostServices);
        assert_eq!(route_tool("store_install"), ToolRoute::HostServices);
        assert_eq!(route_tool("delegate_task"), ToolRoute::HostServices);
        assert_eq!(route_tool("thread_cancel"), ToolRoute::HostServices);
    }

    #[test]
    fn tool_calls_validate() {
        let call = ToolCall {
            instance: InstanceId(1),
            run: "run-1".into(),
            call_id: "c1".into(),
            tool: "apply_commands".into(),
            arguments: serde_json::json!({ "commands": [] }),
        };
        assert!(call.validate().is_ok());
        assert!(
            ToolCall {
                tool: "open_panel".into(),
                ..call.clone()
            }
            .validate()
            .is_err()
        );
        assert!(
            ToolCall {
                tool: "export_video".into(),
                ..call.clone()
            }
            .validate()
            .is_err(),
            "exports go to the job engine"
        );
        assert!(
            ToolCall {
                tool: "Bad-Name".into(),
                ..call
            }
            .validate()
            .is_err()
        );
    }
}
