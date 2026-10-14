//! Ordinary thread runs as the orchestrator sees them (port of
//! `agent-thread-monitor.ts`). Watches bind to a run, not a mutable "latest
//! turn" pointer, and a watch that would make two runs wait on each other is
//! refused.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Weak};
use std::time::Duration;

use anyhow::{Result, bail};
use parking_lot::Mutex;
use serde::Serialize;
use serde_json::Value;
use tokio::sync::watch;

use super::orchestrator::now_ms;
use super::orchestrator_providers::provider_id;
use super::request::{RunRequest, RunResult};
use super::trace;

const STARTING_TIMEOUT: Duration = Duration::from_secs(30);
const RECORDS: usize = 512;

/// `AgentThreadRun`.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ThreadRun {
    pub thread_id: String,
    pub run_id: String,
    pub project_id: String,
    pub status: &'static str,
    pub provider_instance_id: &'static str,
    pub model: Option<String>,
    pub reasoning_effort: Option<String>,
    pub progress: String,
    pub summary: Option<String>,
    pub started_at: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ended_at: Option<u64>,
}

impl ThreadRun {
    pub fn terminal(&self) -> bool {
        matches!(self.status, "completed" | "failed" | "cancelled" | "interrupted")
    }
}

struct Record {
    state: ThreadRun,
    done: watch::Sender<bool>,
}

#[derive(Default)]
struct Inner {
    records: HashMap<String, Record>,
    /// Run ids in reservation order, for "latest run of a thread".
    order: Vec<String>,
    /// watcher run → watched run → watch count.
    edges: HashMap<String, HashMap<String, usize>>,
}

#[derive(Default)]
pub struct ThreadMonitor {
    inner: Mutex<Inner>,
    this: Mutex<Weak<ThreadMonitor>>,
}

impl ThreadMonitor {
    pub fn new() -> Arc<Self> {
        let monitor = Arc::new(Self::default());
        *monitor.this.lock() = Arc::downgrade(&monitor);
        monitor
    }

    /// A root run starts (or claims the reservation a watcher made for it).
    pub fn begin(&self, request: &RunRequest) -> Result<()> {
        let thread = request.thread_id.as_deref().unwrap_or("legacy");
        self.reserve(&request.project_id, thread, &request.id)?;
        let mut inner = self.inner.lock();
        let record = inner.records.get_mut(&request.id).expect("reserved above");
        if record.state.terminal() {
            bail!("This agent request has already finished. Start a new run.");
        }
        record.state.status = "running";
        record.state.provider_instance_id = provider_id(request.provider);
        record.state.model = request.model.clone();
        record.state.reasoning_effort = request.reasoning_effort.clone();
        record.state.started_at = now_ms();
        prune(&mut inner);
        Ok(())
    }

    pub fn reserve(&self, project_id: &str, thread_id: &str, run_id: &str) -> Result<()> {
        let mut inner = self.inner.lock();
        if let Some(existing) = inner.records.get(run_id) {
            if existing.state.project_id != project_id || existing.state.thread_id != thread_id {
                bail!("This run belongs to another thread.");
            }
            return Ok(());
        }
        let state = ThreadRun {
            thread_id: thread_id.into(),
            run_id: run_id.into(),
            project_id: project_id.into(),
            status: "queued",
            provider_instance_id: "chatgpt",
            model: None,
            reasoning_effort: None,
            progress: String::new(),
            summary: None,
            started_at: now_ms(),
            ended_at: None,
        };
        inner.records.insert(run_id.into(), Record { state, done: watch::channel(false).0 });
        inner.order.push(run_id.into());
        drop(inner);
        // The renderer can announce a run before its request reaches the host.
        let monitor = self.this.lock().clone();
        let run_id = run_id.to_owned();
        if tokio::runtime::Handle::try_current().is_ok() {
            tokio::spawn(async move {
                tokio::time::sleep(STARTING_TIMEOUT).await;
                if let Some(monitor) = monitor.upgrade()
                    && monitor.get_run(&run_id).is_some_and(|state| state.status == "queued")
                {
                    monitor.finish_with(&run_id, false, "The thread did not start its agent run.", false);
                }
            });
        }
        Ok(())
    }

    fn get_run(&self, run_id: &str) -> Option<ThreadRun> {
        self.inner.lock().records.get(run_id).map(|record| record.state.clone())
    }

    /// A run of `thread_id`: `run_id`, or the thread's latest known run.
    pub fn get(&self, project_id: &str, thread_id: &str, run_id: Option<&str>) -> Result<Option<ThreadRun>> {
        let inner = self.inner.lock();
        if let Some(run_id) = run_id {
            let state = inner.records.get(run_id).map(|record| record.state.clone());
            if state.as_ref().is_some_and(|state| state.project_id != project_id || state.thread_id != thread_id) {
                bail!("This run belongs to another thread.");
            }
            return Ok(state);
        }
        Ok(inner.order.iter().rev().filter_map(|id| inner.records.get(id)).find(|record| record.state.project_id == project_id && record.state.thread_id == thread_id).map(|record| record.state.clone()))
    }

    /// Resolves once the run is terminal.
    pub fn done(&self, run_id: &str) -> Option<watch::Receiver<bool>> {
        self.inner.lock().records.get(run_id).map(|record| record.done.subscribe())
    }

    pub fn progress(&self, run_id: &str, text: &str) {
        let mut inner = self.inner.lock();
        let Some(record) = inner.records.get_mut(run_id).filter(|record| !record.state.terminal()) else { return };
        record.state.progress = trace::truncate(&text.split_whitespace().collect::<Vec<_>>().join(" "), trace::PROGRESS_CHARS);
        record.state.status = if text.starts_with("Waiting for subagents") || text.starts_with("Waiting for watched threads") { "waiting" } else { "running" };
    }

    pub fn trace(&self, run_id: &str, step: &Value) {
        match step["kind"].as_str() {
            Some("tool-start") => self.progress(run_id, step["label"].as_str().unwrap_or_default()),
            Some("question") if step["blocking"] == true => {
                self.progress(run_id, "Waiting for your answer…");
                if let Some(record) = self.inner.lock().records.get_mut(run_id) {
                    record.state.status = "waiting";
                }
            }
            _ => {}
        }
    }

    pub fn finish(&self, run_id: &str, result: &RunResult) {
        match result {
            RunResult::Done { text, .. } => self.finish_with(run_id, true, text, false),
            RunResult::Failed { error, cancelled } => self.finish_with(run_id, false, error, *cancelled),
        }
    }

    pub fn finish_with(&self, run_id: &str, ok: bool, text: &str, cancelled: bool) {
        let mut inner = self.inner.lock();
        let Some(record) = inner.records.get_mut(run_id).filter(|record| !record.state.terminal()) else { return };
        record.state.status = if ok { "completed" } else if cancelled { "cancelled" } else { "failed" };
        record.state.summary = Some(trace::truncate(text, 200_000));
        record.state.ended_at = Some(now_ms());
        record.done.send_replace(true);
    }

    /// Record that `parent` waits on `target`; refused when it would form a cycle.
    pub fn watch(&self, parent: &str, target: &str) -> Result<()> {
        let mut inner = self.inner.lock();
        let mut seen = HashSet::new();
        let mut stack = vec![target.to_owned()];
        while let Some(id) = stack.pop() {
            if id == parent {
                bail!("Watching this run would create a cycle.");
            }
            if seen.insert(id.clone()) {
                stack.extend(inner.edges.get(&id).into_iter().flat_map(|next| next.keys().cloned()));
            }
        }
        *inner.edges.entry(parent.into()).or_default().entry(target.into()).or_default() += 1;
        Ok(())
    }

    /// Release one `watch(parent, target)`.
    pub fn unwatch(&self, parent: &str, target: &str) {
        let mut inner = self.inner.lock();
        let Some(targets) = inner.edges.get_mut(parent) else { return };
        match targets.get_mut(target) {
            Some(count) if *count > 1 => *count -= 1,
            Some(_) => drop(targets.remove(target)),
            None => {}
        }
        if targets.is_empty() {
            inner.edges.remove(parent);
        }
    }

    pub fn forget(&self, project_id: &str) {
        let mut inner = self.inner.lock();
        inner.records.retain(|_, record| record.state.project_id != project_id);
        let Inner { records, order, .. } = &mut *inner;
        order.retain(|id| records.contains_key(id));
    }

    pub fn shutdown(&self) {
        let ids: Vec<String> = self.inner.lock().records.keys().cloned().collect();
        for id in ids {
            self.finish_with(&id, false, "Powermove closed before the run finished.", true);
        }
        self.inner.lock().edges.clear();
    }
}

fn prune(inner: &mut Inner) {
    let mut index = 0;
    while inner.records.len() > RECORDS && index < inner.order.len() {
        let id = inner.order[index].clone();
        let watched = inner.edges.values().any(|edges| edges.contains_key(&id));
        if inner.records.get(&id).is_some_and(|record| record.state.terminal()) && !watched {
            inner.records.remove(&id);
            inner.order.remove(index);
        } else {
            index += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn watches_refuse_cycles_and_release_by_count() {
        let monitor = ThreadMonitor::new();
        monitor.watch("a", "b").unwrap();
        monitor.watch("b", "c").unwrap();
        assert!(monitor.watch("c", "a").unwrap_err().to_string().contains("cycle"));
        assert!(monitor.watch("a", "a").is_err());
        monitor.watch("a", "b").unwrap();
        monitor.unwatch("a", "b");
        assert!(monitor.watch("c", "a").is_err(), "one watch a→b remains");
        monitor.unwatch("a", "b");
        monitor.watch("c", "a").unwrap();
    }

    #[test]
    fn runs_belong_to_their_thread_and_finish_once() {
        let monitor = ThreadMonitor::new();
        monitor.reserve("p", "t1", "run-1").unwrap();
        assert!(monitor.reserve("p", "t2", "run-1").is_err());
        assert!(monitor.get("p", "t2", Some("run-1")).is_err());
        assert_eq!(monitor.get("p", "t1", None).unwrap().unwrap().run_id, "run-1");
        let done = monitor.done("run-1").unwrap();
        monitor.finish_with("run-1", true, "ok", false);
        monitor.finish_with("run-1", false, "late", false);
        let state = monitor.get("p", "t1", Some("run-1")).unwrap().unwrap();
        assert_eq!((state.status, state.summary.as_deref()), ("completed", Some("ok")));
        assert!(*done.borrow());
    }
}
