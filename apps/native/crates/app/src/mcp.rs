//! App-owned MCP connections. Every tool runs through `McpSession` on an MCP
//! worker, against the app's actor. This layer adds the app's session fence.
use pm_kernel::{session::*, *};
pub use pm_mcp::app_host::{Host, Operation};
use pm_mcp::{Automation, Error, McpSession, Result, transport::ToolConnection};
use serde_json::Value;
use std::{
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
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
    /// Install the production engine hosts. Call on a worker before serving.
    pub fn connect_engine(&self, activity: Arc<AtomicBool>) -> anyhow::Result<()> {
        let jobs = crate::mcp_services::EngineJobs::new(
            self.host.kernel.clone(),
            std::env::current_exe()?,
            activity,
            self.host.profile.join("cache/mcp"),
        );
        let mut automation = self.automation.lock().unwrap();
        automation.export_child = jobs.clone();
        automation.engine = Some(jobs);
        Ok(())
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
    fn admit(&self, name: &str) -> Result<Option<Operation>> {
        let operation = if name == "export_video" {
            None
        } else {
            Some(self.host.try_operation().ok_or_else(|| {
                Error::conflict("A project operation is in progress. Inspect and retry.")
            })?)
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

pub fn headless(profile: PathBuf, exports: PathBuf, launch: Option<PathBuf>) -> anyhow::Result<()> {
    std::thread::spawn(move || -> anyhow::Result<()> {
        let actor = KernelActor::spawn(KernelConfig {
            journal_root: Some(profile.join("journals")),
            ..Default::default()
        });
        let (host, _) = Host::new(actor.handle(), profile.clone());
        attach(&host.kernel, &profile)?;
        if let Some(path) = launch {
            host.open(super::session::stable_id(&path), SessionSource::File(path))?;
        }
        let tools = Tools::new(host, exports);
        tools.connect_engine(Arc::default())?;
        let shared = tools.clone();
        let server =
            pm_mcp::transport::LoopbackServer::start_with(&profile, move || shared.connect())?;
        let result = pm_mcp::transport::serve_stdio(
            std::io::stdin().lock(),
            std::io::stdout().lock(),
            tools.connect(),
        );
        drop(server);
        actor.shutdown();
        result.map_err(Into::into)
    })
    .join()
    .map_err(|_| anyhow::anyhow!("MCP worker panicked"))?
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
