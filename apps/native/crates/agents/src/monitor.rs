//! Actor-owned ordinary-thread run monitor. Watches bind to immutable run IDs,
//! never to a thread's mutable latest-run pointer. No timers or UI are required.
use crate::{Event, Provider, RunResult, trace};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Queued,
    Running,
    Waiting,
    Completed,
    Failed,
    Cancelled,
    Interrupted,
}
impl Status {
    pub fn terminal(self) -> bool {
        matches!(
            self,
            Self::Completed | Self::Failed | Self::Cancelled | Self::Interrupted
        )
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Run {
    pub thread_id: String,
    pub run_id: String,
    pub project_id: String,
    pub status: Status,
    pub provider_instance_id: Provider,
    pub model: Option<String>,
    pub reasoning_effort: Option<String>,
    pub progress: String,
    pub summary: Option<String>,
    pub started_at: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ended_at: Option<u64>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Watch {
    id: u64,
}
#[derive(Default)]
pub struct Monitor {
    records: BTreeMap<String, Run>,
    order: Vec<String>,
    starting: BTreeMap<String, u64>,
    // Unique leases, rather than a parent/target set: releasing a duplicate
    // watch must not remove the cycle barrier held by another owner.
    watches: BTreeMap<u64, (String, String)>,
    next: u64,
}
impl Monitor {
    pub fn reserve(
        &mut self,
        project: &str,
        thread: &str,
        run: &str,
        now: u64,
    ) -> Result<&Run, String> {
        if ![project, thread, run]
            .iter()
            .all(|s| crate::valid_id(s, 200))
        {
            return Err("Invalid project, thread or run id".into());
        }
        if let Some(existing) = self.records.get(run) {
            if existing.project_id != project || existing.thread_id != thread {
                return Err("This run belongs to another thread.".into());
            }
        } else {
            if self.records.len() >= 512 {
                return Err("Archive terminal run records before starting more runs.".into());
            }
            self.records.insert(
                run.into(),
                Run {
                    project_id: project.into(),
                    thread_id: thread.into(),
                    run_id: run.into(),
                    status: Status::Queued,
                    provider_instance_id: Provider::Chatgpt,
                    model: None,
                    reasoning_effort: None,
                    progress: String::new(),
                    summary: None,
                    started_at: now,
                    ended_at: None,
                },
            );
            self.order.push(run.into());
            self.starting.insert(run.into(), now.saturating_add(30_000));
        }
        Ok(&self.records[run])
    }
    pub fn begin(&mut self, mut run: Run, now: u64) -> Result<&Run, String> {
        let existing = self.reserve(&run.project_id, &run.thread_id, &run.run_id, now)?;
        if existing.status.terminal() {
            return Err("This agent request has already finished. Start a new run.".into());
        }
        // Caller-supplied snapshots cannot forge progress or a terminal result.
        run.status = Status::Running;
        run.started_at = now;
        run.progress = existing.progress.clone();
        run.summary = existing.summary.clone();
        run.ended_at = None;
        let id = run.run_id.clone();
        self.starting.remove(&id);
        self.records.insert(id.clone(), run);
        Ok(&self.records[&id])
    }
    pub fn get(
        &self,
        project: &str,
        thread: &str,
        run: Option<&str>,
    ) -> Result<Option<&Run>, String> {
        if let Some(run) = run {
            let record = self.records.get(run);
            if record.is_some_and(|r| r.project_id != project || r.thread_id != thread) {
                return Err("This run belongs to another thread.".into());
            }
            Ok(record)
        } else {
            Ok(self
                .order
                .iter()
                .rev()
                .filter_map(|id| self.records.get(id))
                .find(|r| r.project_id == project && r.thread_id == thread))
        }
    }
    pub fn progress(&mut self, run: &str, text: &str) {
        if let Some(record) = self.records.get_mut(run).filter(|r| !r.status.terminal()) {
            record.progress =
                trace::truncate(&text.split_whitespace().collect::<Vec<_>>().join(" "), 320);
            record.status = if text.starts_with("Waiting for subagents")
                || text.starts_with("Waiting for watched threads")
            {
                Status::Waiting
            } else {
                Status::Running
            };
        }
    }
    pub fn event(&mut self, event: &Event, now: u64) {
        match event {
            Event::Progress { run, text } => self.progress(run, text),
            Event::Trace { run, value } if value["kind"] == "tool-start" => {
                self.progress(run, value["label"].as_str().unwrap_or_default())
            }
            Event::Trace { run, value }
                if value["kind"] == "question" && value["blocking"] == true =>
            {
                self.progress(run, "Waiting for your answer…");
                if let Some(r) = self.records.get_mut(run).filter(|r| !r.status.terminal()) {
                    r.status = Status::Waiting;
                }
            }
            Event::Completed { run, result } => self.finish(run, result, now),
            _ => {}
        }
    }
    pub fn finish(&mut self, run: &str, result: &RunResult, now: u64) {
        if let Some(r) = self.records.get_mut(run).filter(|r| !r.status.terminal()) {
            let (status, summary) = match result {
                RunResult::Success { text, .. } => (Status::Completed, text),
                RunResult::Failure {
                    error, cancelled, ..
                } => (
                    if *cancelled {
                        Status::Cancelled
                    } else {
                        Status::Failed
                    },
                    error,
                ),
            };
            r.status = status;
            r.summary = Some(trace::truncate(summary, 200_000));
            r.ended_at = Some(now);
            self.starting.remove(run);
        }
    }
    /// Admit watches within an authorized project; scope validation precedes
    /// mutation. Shutdown releases watches without cancelling unrelated work.
    pub fn watch(&mut self, project: &str, parent: &str, target: &str) -> Result<Watch, String> {
        if [parent, target].iter().any(|id| {
            self.records
                .get(*id)
                .is_none_or(|r| r.project_id != project)
        }) {
            return Err("This run belongs to another project or is unavailable.".into());
        }
        let mut seen = BTreeSet::new();
        let mut pending = vec![target];
        while let Some(id) = pending.pop() {
            if id == parent {
                return Err("Watching this run would create a cycle.".into());
            }
            if seen.insert(id) {
                pending.extend(
                    self.watches
                        .values()
                        .filter(|(p, _)| p == id)
                        .map(|(_, t)| t.as_str()),
                );
            }
        }
        if self.watches.values().filter(|(p, _)| p == parent).count() >= 32 {
            return Err("A run can watch up to 32 thread runs.".into());
        }
        let id = self.next.checked_add(1).ok_or("Watch identity exhausted")?;
        self.next = id;
        self.watches.insert(id, (parent.into(), target.into()));
        Ok(Watch { id })
    }
    pub fn release(&mut self, watch: &Watch) {
        self.watches.remove(&watch.id);
    }
    /// Validate a lease before including its result in a scoped continuation.
    pub fn owns_watch(&self, project: &str, parent: &str, watch: &Watch) -> bool {
        self.watches.get(&watch.id).is_some_and(|(owner, target)| {
            owner == parent
                && self
                    .records
                    .get(target)
                    .is_some_and(|r| r.project_id == project)
        })
    }
    pub fn is_watching(&self, watch: &Watch) -> bool {
        self.watches.contains_key(&watch.id)
    }
    /// Returns a terminal snapshot without acknowledging or releasing it.
    /// The orchestrator persists delivery before releasing the lease.
    pub fn result(&self, watch: &Watch) -> Option<&Run> {
        let (_, target) = self.watches.get(&watch.id)?;
        self.records.get(target).filter(|r| r.status.terminal())
    }
    pub fn tick(&mut self, now: u64) {
        let expired = self
            .starting
            .iter()
            .filter(|(_, deadline)| now >= **deadline)
            .map(|(id, _)| id.clone())
            .collect::<Vec<_>>();
        for id in expired {
            self.finish(
                &id,
                &RunResult::failure("The thread did not start its agent run.", false),
                now,
            );
        }
    }
    /// Retire only after the host has durably archived this exact snapshot.
    /// The host retains the immutable run-ID index and serves archived reads.
    pub fn archive_terminal(
        &mut self,
        run: &str,
        archive: impl FnOnce(&Run) -> Result<(), String>,
    ) -> Result<(), String> {
        let record = self.records.get(run).ok_or("Unknown run")?;
        if !record.status.terminal() || self.watches.values().any(|(p, t)| p == run || t == run) {
            return Err("The run is active or still watched.".into());
        }
        archive(record)?;
        self.records.remove(run);
        self.order.retain(|id| id != run);
        Ok(())
    }
    pub fn shutdown(&mut self, now: u64) {
        for id in self.order.clone() {
            self.finish(
                &id,
                &RunResult::failure("Powermove closed before the run finished.", true),
                now,
            );
        }
        self.watches.clear();
    }
    // N4: no automatic pruning. Explicit retirement requires a durable archive.
}
