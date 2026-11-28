//! App-owned MCP connections. Every tool runs through `McpSession` on an MCP
//! worker, against the app's actor. This layer adds the app's session fence.
use pm_kernel::{session::*, *};
pub use pm_mcp::app_host::{Host, Operation};
use pm_mcp::{Automation, Error, McpSession, Result, transport::ToolConnection};
use serde_json::Value;
use std::{
    path::PathBuf,
    sync::{Arc, Mutex, atomic::Ordering},
};

/// The session fence, taken as soon as no other transition holds it. An
/// open the Library already made for this window (a new tab's project)
/// waits its turn behind another tab's open, a recovery question, say,
/// rather than being dropped (PM-454).
pub(crate) async fn take_fence(host: &Host, executor: &gpui::BackgroundExecutor) -> Operation {
    loop {
        if let Some(operation) = host.try_operation() {
            return operation;
        }
        executor.timer(std::time::Duration::from_millis(50)).await;
    }
}

/// The app's own way to the agent's render path, for a still the person
/// selects (the composer's Select part of the screen).
static STILLS: std::sync::OnceLock<Arc<Mutex<Automation>>> = std::sync::OnceLock::new();
static COMPLETION_HOST: std::sync::OnceLock<Arc<Host>> = std::sync::OnceLock::new();
/// A PNG of `project`'s composition at `time`, at most `width` pixels wide,
/// rendered as the agent's render_frames renders. Off the main thread.
pub fn still(project: &str, time: f64, width: u32) -> std::result::Result<Vec<u8>, String> {
    use base64::Engine;
    let automation = STILLS.get().ok_or("Frame capture isn't ready yet.")?;
    let out = automation
        .lock()
        .map_err(|_| "Frame capture is unavailable.".to_owned())?
        .render(project, &serde_json::json!({"times": [time], "width": width}))
        .map_err(|e| e.to_string())?;
    let image = out.images.first().ok_or("The frame couldn't be rendered.")?;
    base64::engine::general_purpose::STANDARD
        .decode(&image.data)
        .map_err(|e| e.to_string())
}

pub(crate) fn aero_launcher() -> pm_agents::aero::Launcher {
    pm_agents::aero::Launcher(Arc::new(|request, scope, token| {
        let host = STILLS.get().ok_or("Powermove's tool host is still starting.")?;
        let project = request.project.as_deref().ok_or("Open a project to use Aerolite.")?;
        pm_mcp::session::aero_tools::Tools::open(
            host.clone(), project, None, scope, request.access, Some(token),
        )
        .map(|tools| Box::new(tools) as Box<dyn pm_agents::aero::Session>)
        .map_err(|e| e.to_string())
    }))
}

/// The tool host shared by every connection to this app.
#[derive(Clone)]
pub struct Tools {
    pub host: Arc<Host>,
    pub automation: Arc<Mutex<Automation>>,
}
impl Tools {
    /// Lets `still` render through this host (once, at startup).
    pub fn keep_for_stills(&self) {
        let _ = STILLS.set(self.automation.clone());
        let _ = COMPLETION_HOST.set(self.host.clone());
    }
    pub fn new(host: Arc<Host>, exports: PathBuf) -> Self {
        Self {
            automation: Arc::new(Mutex::new(Automation::attached(host.clone(), exports))),
            host,
        }
    }
    pub fn connect(&self) -> Connection {
        Connection {
            host: self.host.clone(),
            session: McpSession::new(self.automation.clone()),
        }
    }
}
pub struct Connection {
    host: Arc<Host>,
    session: McpSession,
}
impl Connection {
    /// UI open/close and session-changing MCP requests take the exclusive
    /// fence; inspection, capture, panel, and render tools take a shared fence
    /// so parallel agent calls and subagents never reject each other with
    /// "A project operation is in progress".
    fn admit(&self, name: &str, wait: bool) -> Result<Option<Operation>> {
        const UNFENCED: [&str; 9] = [
            "export_video",
            "apply_commands",
            "edit_layer_strips",
            "edit_video",
            "edit_3d",
            "list_projects",
            "store_search",
            "store_extension",
            "store_source",
        ];
        const EXCLUSIVE: [&str; 4] = [
            "open_project",
            "create_project",
            "manage_project",
            "save_project",
        ];
        let acquire = || {
            if EXCLUSIVE.contains(&name) {
                self.host.try_operation()
            } else {
                self.host.try_shared_operation()
            }
        };
        let operation = if UNFENCED.contains(&name) {
            None
        } else if let Some(op) = acquire() {
            Some(op)
        } else if wait
            && !self.host.closing.load(Ordering::Acquire)
            && !self.host.is_awaiting_recovery()
        {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
            let mut acquired = None;
            while std::time::Instant::now() < deadline {
                if self.host.closing.load(Ordering::Acquire) || self.host.is_awaiting_recovery() {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(25));
                if let Some(op) = acquire() {
                    acquired = Some(op);
                    break;
                }
            }
            Some(acquired.ok_or_else(|| self.host.fenced())?)
        } else {
            Some(acquire().ok_or_else(|| self.host.fenced())?)
        };
        if self.host.closing.load(Ordering::Acquire) {
            return Err(Error::conflict(
                "The app is closing. Finish or cancel the close first.",
            ));
        }
        Ok(operation)
    }
    #[cfg(test)]
    pub fn dispatch(&mut self, name: &str, args: &Value) -> Result<Value> {
        let _operation = self.admit(name, false)?;
        self.session.dispatch(name, args)
    }
}
impl ToolConnection for Connection {
    fn bind_agent_run(&mut self, run: &str) {
        self.session.set_agent_run(run);
    }
    fn list(&mut self) -> Result<Value> {
        Ok(self.session.tools())
    }
    fn invoke(&mut self, name: &str, args: Value) -> Result<Value> {
        if name != "finish_session" {
            self.session.wait_for_pause()?;
        }
        Ok(match self.admit(name, true) {
            Ok(_operation) => self.session.call(name, args),
            Err(error) => pm_mcp::failure(error),
        })
    }
    fn disconnect(&mut self) -> Result<()> {
        self.session.close()
    }
}

pub fn attach(kernel: &KernelHandle, profile: &std::path::Path) -> anyhow::Result<()> {
    match kernel
        .shell(ShellCall::AttachLibrary {
            profile: profile.to_owned(),
            format: pm_kernel::persistence::DiskFormat::V1_2,
            electron_profile: None,
        })
        .wait()
    {
        Some(Ok(ShellReply::Library(pm_kernel::locks::OpenDecision::Writable))) => Ok(()),
        other => anyhow::bail!("Cannot attach isolated native library: {other:?}"),
    }
}

#[cfg(test)]
#[path = "mcp_tests.rs"]
mod tests;

/// Host-owned run completion and result actions, on the panel worker.
pub fn completion(
    operation: &str,
    project: &str,
    run: &str,
    payload: &Value,
) -> std::result::Result<Value, String> {
    completion_on(
        COMPLETION_HOST
            .get()
            .ok_or("Completion host is not ready")?,
        STILLS.get().ok_or("Completion host is not ready")?,
        operation,
        project,
        run,
        payload,
    )
}
fn completion_on(
    host: &Host,
    automation: &Mutex<Automation>,
    operation: &str,
    project: &str,
    run: &str,
    payload: &Value,
) -> std::result::Result<Value, String> {
    // MCP thread/delegation calls already hold this lease. Read-only checks
    // and private receipt bookkeeping must not reacquire it recursively.
    let _operation = if matches!(operation, "apply" | "undo") {
        Some(
            host.try_operation()
                .ok_or("The project is changing. Try this action again.")?,
        )
    } else if operation == "openPanel" {
        Some(
            host.try_shared_operation()
                .ok_or("The project is changing. Try this action again.")?,
        )
    } else {
        None
    };
    automation
        .lock()
        .map_err(|_| "Completion host stopped")?
        .completion_action(operation, project, run, payload)
        .map_err(|e| e.to_string())
}
