//! Owner-pinned main catalog with the two owner-selected Studio 3D contracts.
//! Tagged data stays available as compatibility evidence.
use crate::{Error, Result};
use serde_json::Value;
use std::sync::OnceLock;

pub fn catalog() -> &'static Value {
    static VALUE: OnceLock<Value> = OnceLock::new();
    VALUE.get_or_init(|| {
        let mut value: Value =
            serde_json::from_str(include_str!("../fixtures/catalog-main-c7f46a056.json"))
                .expect("frozen catalog");
        let studio: Value =
            serde_json::from_str(include_str!("../fixtures/scene3d-source-catalog.json"))
                .expect("3D source catalog");
        for replacement in studio["tools"].as_array().unwrap() {
            let tool = value["tools"]
                .as_array_mut()
                .unwrap()
                .iter_mut()
                .find(|t| t["name"] == replacement["name"])
                .expect("existing 3D tool");
            *tool = replacement.clone();
        }
        value
    })
}
pub fn legacy_catalog() -> &'static Value {
    static LEGACY: OnceLock<Value> = OnceLock::new();
    LEGACY.get_or_init(|| {
        serde_json::from_str(include_str!("../fixtures/catalog-v1.2.0.json"))
            .expect("legacy catalog")
    })
}
pub fn tools() -> &'static Vec<Value> {
    catalog()["tools"].as_array().unwrap()
}
pub fn spec(name: &str) -> Option<&'static Value> {
    tools().iter().find(|t| t["name"] == name)
}
pub fn permits(subset: &str, name: &str) -> bool {
    catalog()[subset]
        .as_array()
        .unwrap()
        .iter()
        .any(|n| n == name)
}
/// Every tool name, then the subset a planning (editor) run may call.
pub fn names() -> Vec<String> {
    tools()
        .iter()
        .filter_map(|tool| tool["name"].as_str().map(str::to_owned))
        .collect()
}
/// What a Composition-only run may call: inspection, plus edits that go through
/// its own session, so the provider's shell and files stay out of reach.
pub fn composition_names() -> Vec<String> {
    const EDITS: &[&str] = &[
        "start_session",
        "finish_session",
        "rollback_changes",
        "project_memory",
        "select_layers",
        "apply_commands",
        "edit_layer_strips",
        "edit_video",
        "edit_3d",
    ];
    names()
        .into_iter()
        .filter(|name| permits("inspection", name) || EDITS.contains(&name.as_str()))
        .collect()
}
pub fn validate(name: &str, args: &Value) -> Result<()> {
    let schema =
        &spec(name).ok_or_else(|| Error::invalid(format!("Unknown tool: {name}")))?["inputSchema"];
    validate_value(schema, args, "arguments")
}
fn validate_value(s: &Value, v: &Value, path: &str) -> Result<()> {
    let bad = || Error::invalid(format!("Invalid {path}"));
    if let Some(variants) = s["oneOf"].as_array()
        && variants
            .iter()
            .filter(|s| validate_value(s, v, path).is_ok())
            .count()
            != 1
    {
        return Err(bad());
    }
    let matches = |t: &str| match t {
        "object" => v.is_object(),
        "array" => v.is_array(),
        "string" => v.is_string(),
        "boolean" => v.is_boolean(),
        "null" => v.is_null(),
        "number" => v.is_number(),
        "integer" => v.as_f64().is_some_and(|n| n.fract() == 0.0),
        _ => false,
    };
    if let Some(t) = s["type"].as_str() {
        if !matches(t) {
            return Err(bad());
        }
    } else if let Some(ts) = s["type"].as_array()
        && !ts.iter().any(|t| t.as_str().is_some_and(matches))
    {
        return Err(bad());
    }
    if let Some(values) = s["enum"].as_array()
        && !values.contains(v)
    {
        return Err(bad());
    }
    if let Some(o) = v.as_object() {
        if let Some(required) = s["required"].as_array() {
            for k in required {
                if !o.contains_key(k.as_str().unwrap()) {
                    return Err(Error::invalid(format!(
                        "Missing {path}.{}",
                        k.as_str().unwrap()
                    )));
                }
            }
        }
        for (k, value) in o {
            if let Some(child) = s["properties"].get(k) {
                validate_value(child, value, &format!("{path}.{k}"))?;
            } else if s["additionalProperties"] == false {
                return Err(Error::invalid(format!("Unknown {path}.{k}")));
            }
        }
    }
    if let Some(a) = v.as_array() {
        if s["minItems"].as_u64().is_some_and(|n| a.len() < n as usize)
            || s["maxItems"].as_u64().is_some_and(|n| a.len() > n as usize)
        {
            return Err(bad());
        }
        if s["uniqueItems"] == true && a.iter().enumerate().any(|(i, v)| a[..i].contains(v)) {
            return Err(bad());
        }
        for (i, value) in a.iter().enumerate() {
            if s.get("items").is_some() {
                validate_value(&s["items"], value, &format!("{path}[{i}]"))?;
            }
        }
    }
    if let Some(t) = v.as_str() {
        let len = t.encode_utf16().count(); // JSON Schema as consumed by the shipped JS host.
        if s["minLength"].as_u64().is_some_and(|n| len < n as usize)
            || s["maxLength"].as_u64().is_some_and(|n| len > n as usize)
        {
            return Err(bad());
        }
        if let Some(p) = s["pattern"].as_str()
            && !regex::Regex::new(p).map_err(|_| bad())?.is_match(t)
        {
            return Err(bad());
        }
    }
    if let Some(n) = v.as_f64()
        && (s["minimum"].as_f64().is_some_and(|b| n < b)
            || s["maximum"].as_f64().is_some_and(|b| n > b)
            || s["exclusiveMinimum"].as_f64().is_some_and(|b| n <= b))
    {
        return Err(bad());
    }
    Ok(())
}

/// Tools with concrete bodies/adapters. HOST_BACKED tools need an injected
/// native worker capabilities; without them they still fail explicitly.
pub const IMPLEMENTED: &[&str] = &[
    "project_memory",
    "list_projects",
    "create_project",
    "open_project",
    "start_session",
    "finish_session",
    "manage_project",
    "save_project",
    "get_project_state",
    "get_timeline",
    "select_layers",
    "apply_commands",
    "edit_layer_strips",
    "rollback_changes",
    "get_panel_layout",
    "inspect_creative_workspace",
    "inspect_creative_extension",
    "get_3d_scene",
    "edit_3d",
    "check_project",
    "list_media",
    "manage_media",
    "edit_video",
    "render_frames",
    "probe_media",
    "sample_media_frames",
    "media_contact_sheet",
    "media_waveform",
    "export_video",
    "export_captions",
    "import_media",
    "replace_media",
    "import_media_to_timeline",
    "thread_list",
    "thread_read",
    "validate_effect",
    "fork_builtin_extension",
    "stage_fork_rebase",
    "publish_extensions",
    "store_search",
    "store_extension",
    "store_source",
    "open_panel",
    "get_panel_state",
    "interact_panel",
    "capture_panel",
    "computer_use_panel",
    "set_panel_layout",
    "get_workspace_state",
];
/// Adapter availability is distinct from a working standalone backend.
pub const HOST_BACKED: &[&str] = &[
    "thread_list",
    "thread_read",
    "import_media",
    "replace_media",
    "import_media_to_timeline",
    "fork_builtin_extension",
    "stage_fork_rebase",
    "publish_extensions",
    "render_frames",
    "probe_media",
    "sample_media_frames",
    "media_contact_sheet",
    "media_waveform",
    "export_video",
    "store_search",
    "store_extension",
    "store_source",
    "open_panel",
    "get_panel_state",
    "interact_panel",
    "capture_panel",
    "computer_use_panel",
    "set_panel_layout",
    "get_workspace_state",
];
pub fn backing(name: &str) -> &'static str {
    match name {
        "start_session" | "finish_session" | "rollback_changes" => {
            "kernel history marks, guarded restore and squash; isolated workspace"
        }
        "get_project_state" | "get_timeline" | "select_layers" | "apply_commands" => {
            "pm-kernel document/commands/selection"
        }
        "edit_layer_strips" => "pm-kernel timing/layers and atomic id-keyed operations",
        "list_projects" | "create_project" | "open_project" | "manage_project" | "save_project" => {
            "pm-kernel library/shell/PMV writer"
        }
        "render_frames"
        | "export_video"
        | "probe_media"
        | "sample_media_frames"
        | "media_contact_sheet"
        | "media_waveform"
        | "transcribe_media" => "engine-service/media jobs",
        "validate_effect" => "native frozen kernel/glsl.ts registration validator port",
        "project_memory" => {
            "kernel non-revision durable memory metadata admission and application actor binding"
        }
        "get_panel_layout" => "pm-kernel saved workspace and winning panel registrations",
        "set_panel_layout" => {
            "workspace editing state and external workspace Undo/Redo continuation host"
        }
        "get_workspace_state" => {
            "macOS shell viewport, renderer diagnostics, projects-screen state and full extension-health records"
        }
        "capture_panel" => {
            "macOS registered panel renderer, encoded pixels and stable capture target validation"
        }
        "computer_use_panel" => {
            "macOS panel input/capture host, observed pixel bounds and stable target validation"
        }
        "open_panel" | "get_panel_state" | "interact_panel" => {
            "macOS shell/remote-DOM panel host, fresh semantic refs and stable element IDs"
        }
        "store_search" | "store_extension" | "store_source" => {
            "native Store same-origin bounded HTTP transport (search/detail/release/tree/file)"
        }
        n if n.starts_with("store_") => {
            "Store install/publish host: verified artifacts, durable StoreKernel, permission/trust, provenance, local-tree hashes and native publish confirmation"
        }
        "get_3d_scene" | "check_project" => {
            "pm-kernel evaluated composition/3D/source-timing queries and optional live media registry"
        }
        "edit_video" => "pm-kernel guarded id-keyed layer edits and source-aware timing adapter",
        "export_captions" => "native caption text exporter",
        "edit_3d" => "pm-scene3d and guarded kernel Studio document commands",
        "generate_captions" => "kernel caption commands and transcription runtime",
        "list_media" => {
            "pm-kernel asset/folder/reference snapshot and optional live media registry"
        }
        "manage_media" => "pm-kernel guarded asset/folder edits and native media Undo",
        "replace_media" | "import_media" | "import_media_to_timeline" => {
            "durable media import/version service: verified bytes, fingerprints, retained versions and native Undo"
        }
        "fork_builtin_extension" | "stage_fork_rebase" | "publish_extensions" => {
            "the app's extension host: shipped built-in sources, native compiler, isolated activation and reload"
        }
        "inspect_creative_workspace" | "inspect_creative_extension" => {
            "creative-app inventory service"
        }
        _ => "agent runtime, user MCP servers and scripting sandbox",
    }
}
