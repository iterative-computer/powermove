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

/// The tool host shared by every connection to this app.
#[derive(Clone)]
pub struct Tools {
    pub host: Arc<Host>,
    pub automation: Arc<Mutex<Automation>>,
}
impl Tools {
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
    /// UI open/close and MCP requests fence session identity without blocking
    /// the main thread. Ordinary UI edits and gestures never take this fence.
    /// A running export reads a frozen snapshot, so it does not hold the fence
    /// through its long wait and the user can still close the project.
    /// Library and Store reads never touch the open project, so they also run
    /// while an open waits on a person (a recovery choice).
    fn admit(&self, name: &str) -> Result<Option<Operation>> {
        const UNFENCED: [&str; 5] = [
            "export_video",
            "list_projects",
            "store_search",
            "store_extension",
            "store_source",
        ];
        let operation = if UNFENCED.contains(&name) {
            None
        } else {
            Some(
                self.host
                    .try_operation()
                    .ok_or_else(|| self.host.fenced())?,
            )
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
        let _operation = self.admit(name)?;
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
        Ok(match self.admit(name) {
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
