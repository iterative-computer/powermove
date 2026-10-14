//! Agent runs: Codex and Claude Code with Powermove's editor tools (port of src/main/codex, claude, agent-tools).
//!
//! `service` registers the channels. Runs go to one of three transports:
//! `codex_exec` (autonomous ChatGPT), `codex_app` (editor ChatGPT, with live
//! steering) and `claude_run` (both Claude modes); autonomous runs share the
//! workspace pipeline in `autonomous`. Editor tools reach the CLIs through
//! the loopback MCP endpoint in `tools`.

mod accounts;
mod app_server;
mod approvals;
mod autonomous;
pub(crate) mod changes;
mod claude_events;
mod claude_run;
mod codex_app;
mod codex_events;
mod codex_exec;
mod compatible;
mod compatible_workspace;
#[cfg(test)]
mod live_tests;
mod orchestrator;
mod orchestrator_providers;
mod orchestrator_threads;
mod process;
mod prompts;
mod request;
mod runtime;
mod service;
mod tools;
mod trace;
mod workspace;

use std::sync::Arc;

use serde_json::{Value, json};
use tokio::sync::mpsc;

use crate::host::Config;
use crate::router::{Client, Router};

/// Where one run's `codex:event` progress and trace rows go.
#[derive(Clone)]
pub struct RunEvents {
    emit: Arc<dyn Fn(&'static str, Value) + Send + Sync>,
}

impl RunEvents {
    /// Events for the window that started run `id`.
    pub fn window(owner: Arc<Client>, id: String) -> Self {
        Self {
            emit: Arc::new(move |kind, value| {
                let field = if kind == "progress" { "text" } else { "step" };
                owner.emit("codex:event", json!({ "id": id, "kind": kind, field: value }));
            }),
        }
    }

    /// Events handled by `emit` (the orchestrator routes child runs' events).
    pub fn new(emit: impl Fn(&'static str, Value) + Send + Sync + 'static) -> Self {
        Self { emit: Arc::new(emit) }
    }

    /// Events collected on a channel (tests).
    pub fn channel() -> (Self, mpsc::UnboundedReceiver<(&'static str, Value)>) {
        let (sender, receiver) = mpsc::unbounded_channel();
        (Self { emit: Arc::new(move |kind, value| drop(sender.send((kind, value)))) }, receiver)
    }

    pub fn trace(&self, step: Value) {
        (self.emit)("trace", step);
    }

    pub fn progress(&self, text: &str) {
        if !text.is_empty() {
            (self.emit)("progress", Value::String(trace::truncate(text, trace::PROGRESS_CHARS)));
        }
    }
}

pub use service::Agents;

/// Register the agent channels. Keep the returned handle to call
/// [`Agents::shutdown`] on quit, which stops every CLI process.
pub async fn register(router: &mut Router, config: &Config) -> anyhow::Result<Arc<Agents>> {
    service::register(router, config).await
}
