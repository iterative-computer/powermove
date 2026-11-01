//! Worker-only headless driver. The kernel actor is the sole document writer;
//! MCP owns the public JSON vocabulary. See docs/native/cli.md.
pub mod backend;
pub mod migration;
pub mod options;
pub mod project;

pub use pm_mcp::{Error, Result};
use serde_json::{Value, json};

pub fn error(code: &str, message: impl Into<String>) -> Error {
    Error {
        code: code.into(),
        message: message.into(),
        backing: None,
    }
}
pub fn exit_code(e: &Error) -> u8 {
    match e.code.as_str() {
        "invalid_arguments" => 2,
        "not_available_in_this_build" => 3,
        "revision_conflict" => 4,
        "io_error" => 5,
        _ => 1,
    }
}
pub fn to_json(value: &pm_model::Value) -> Value {
    serde_json::from_str(&pm_model::json::to_canonical_string(value)).expect("model JSON")
}
pub fn execute(options: options::Options) -> Result<(Value, u8)> {
    execute_with_backend_factory(options, backend::backend)
}
/// Native hosts can inject a font-aware engine; tests inject failing/delayed
/// implementations without importing AppKit/Metal into the command driver.
pub fn execute_with_backend_factory(
    options: options::Options,
    factory: impl FnOnce(&str) -> Result<Box<dyn backend::Backend>>,
) -> Result<(Value, u8)> {
    if options.command == "help" {
        return Ok((pm_mcp::success(json!({"usage": options::USAGE})), 0));
    }
    if options.command == "migrate" {
        return migration::dry_run(&options).map(|v| (pm_mcp::success(v), 0));
    }
    let mut project = project::Project::open(&options.project, options.command == "apply")?;
    if let Some(expected) = options.expected_revision {
        project.check_revision(expected)?;
    }
    if let Some(expected) = &options.expected_source_hash {
        project.check_source_hash(expected)?;
    }
    let args = &options.arguments;
    let result = match options.command.as_str() {
        "open" | "info" => {
            pm_mcp::catalog::validate("get_project_state", args)?;
            project.info(args)?
        }
        "timeline" => {
            pm_mcp::catalog::validate("get_timeline", args)?;
            project.timeline(args)?
        }
        "apply" => {
            pm_mcp::catalog::validate("apply_commands", args)?;
            project.apply(args, options.output.as_deref().expect("validated output"))?
        }
        "validate" => {
            let result = project.validate()?;
            let status = if result["ok"] == true { 0 } else { 1 };
            return Ok((pm_mcp::success(result), status));
        }
        "render-frames" | "export" => {
            let mut backend = factory(&options.backend)?;
            if options.command == "render-frames" {
                pm_mcp::catalog::validate("render_frames", args)?;
                return backend::render_frames(&project, args, backend.as_mut()).map(|v| (v, 0));
            }
            pm_mcp::catalog::validate("export_video", args)?;
            backend::export(
                &project,
                args,
                options.output.as_deref().expect("validated output"),
                backend.as_mut(),
            )?
        }
        _ => return Err(Error::invalid("Unknown command")),
    };
    Ok((pm_mcp::success(result), 0))
}
