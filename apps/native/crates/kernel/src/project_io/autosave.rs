//! Per-project autosave scheduling, driven by the kernel's worker host. No
//! timers, threads or JS waits: times and readiness are supplied by the actor.
//! Port of main c7f46a056 `legacy/core/autosave-queue.ts`; jobs freeze their
//! revision only when dequeued. A close/export barrier uses `is_flushed`.
use std::collections::{BTreeMap, VecDeque};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Pending,
    Saving,
    Saved,
    Failure,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub revision: u64,
    pub persisted: u64,
    pub due: u64,
    pub phase: Phase,
    pub error: Option<String>,
    first: u64,
    generation: u64,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Job {
    pub project_id: String,
    pub explicit: bool,
    pub revision: u64,
    generation: u64,
    token: u64,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    Saved,
    Cancelled,
    /// A newer recovery checkpoint won publication. Keep pending state until
    /// its acknowledgement arrives; this is not a failed save.
    Superseded,
    Failure(String),
}

#[derive(Default, Debug)]
pub struct AutosaveQueue {
    entries: BTreeMap<String, Entry>,
    waiting: VecDeque<(String, bool)>,
    active: Option<Job>,
    serial: u64,
}
impl AutosaveQueue {
    pub fn state(&self, id: &str) -> Option<&Entry> {
        self.entries.get(id)
    }

    /// Memory-only writes use this same sequence even when edit revision/Undo
    /// do not advance. Call once per acknowledged durable-state mutation.
    pub fn edit(&mut self, id: &str, now: u64) {
        self.serial += 1;
        let entry = self.entries.entry(id.into()).or_insert(Entry {
            revision: 0,
            persisted: 0,
            first: now,
            due: now,
            phase: Phase::Pending,
            error: None,
            generation: self.serial,
        });
        if entry.revision == entry.persisted || entry.phase == Phase::Failure {
            entry.first = now;
        }
        entry.revision += 1;
        entry.due = now
            .saturating_add(750)
            .min(entry.first.saturating_add(5000));
        entry.phase = if self.active.as_ref().is_some_and(|j| j.project_id == id) {
            Phase::Saving
        } else {
            Phase::Pending
        };
        entry.error = None;
    }

    pub fn forget(&mut self, id: &str) {
        self.entries.remove(id);
        self.waiting.retain(|(key, _)| key != id);
    }

    /// Explicit save is serialized behind active work and captures the latest
    /// revision when it begins. A failed automatic write is retried explicitly
    /// or by a later edit, never in an infinite background failure loop.
    pub fn flush(&mut self, id: &str, now: u64) {
        if !self.entries.contains_key(id) {
            self.serial += 1;
            self.entries.insert(
                id.into(),
                Entry {
                    revision: 0,
                    persisted: 0,
                    due: now,
                    first: now,
                    phase: Phase::Pending,
                    error: None,
                    generation: self.serial,
                },
            );
        }
        self.waiting.push_back((id.into(), true));
    }

    pub fn next(&mut self, now: u64, mut ready: impl FnMut(&str) -> bool) -> Option<Job> {
        if self.active.is_some() {
            return None;
        }
        for (id, e) in &mut self.entries {
            if e.revision > e.persisted
                && e.phase != Phase::Failure
                && e.due <= now
                && !self.waiting.iter().any(|(key, _)| key == id)
            {
                if ready(id) {
                    self.waiting.push_back((id.clone(), false));
                } else {
                    e.due = now.saturating_add(100);
                }
            }
        }
        let (id, explicit) = self.waiting.front()?.clone();
        if !ready(&id) {
            return None;
        }
        self.waiting.pop_front();
        let e = self.entries.get_mut(&id)?;
        self.serial += 1;
        let job = Job {
            project_id: id,
            explicit,
            revision: e.revision,
            generation: e.generation,
            token: self.serial,
        };
        e.phase = Phase::Saving;
        self.active = Some(job.clone());
        Some(job)
    }

    /// A stale completion cannot acknowledge a reopened project's edits.
    /// Returns false for the wrong ticket and leaves the active writer intact.
    pub fn complete(&mut self, job: &Job, outcome: Outcome) -> bool {
        if self.active.as_ref() != Some(job) {
            return false;
        }
        self.active = None;
        let Some(e) = self
            .entries
            .get_mut(&job.project_id)
            .filter(|e| e.generation == job.generation)
        else {
            return true;
        };
        e.error = None;
        match outcome {
            Outcome::Saved => {
                e.persisted = job.revision;
                e.phase = if e.revision == e.persisted {
                    Phase::Saved
                } else {
                    Phase::Pending
                };
            }
            Outcome::Cancelled | Outcome::Superseded => {
                e.phase = if e.revision == e.persisted {
                    Phase::Saved
                } else {
                    Phase::Pending
                };
            }
            Outcome::Failure(error) => {
                e.phase = Phase::Failure;
                e.error = Some(error);
            }
        }
        true
    }

    pub fn is_flushed(&self) -> bool {
        self.active.is_none()
            && self.waiting.is_empty()
            && self
                .entries
                .values()
                .all(|e| e.revision == e.persisted && e.phase == Phase::Saved)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn debounce_maximum_and_transaction_readiness_are_per_project() {
        let mut q = AutosaveQueue::default();
        q.edit("p", 0);
        for n in 1..=10 {
            q.edit("p", n * 500);
        }
        assert_eq!(q.state("p").unwrap().due, 5000);
        assert!(q.next(5000, |_| false).is_none());
        assert!(q.next(5099, |_| true).is_none());
        let j = q.next(5100, |_| true).unwrap();
        assert_eq!(j.revision, 11);
        q.edit("other", 5100);
        assert!(q.next(9000, |_| true).is_none());
        q.complete(&j, Outcome::Saved);
        assert_eq!(q.next(9000, |_| true).unwrap().project_id, "other");
    }
    #[test]
    fn flush_captures_latest_and_failure_or_cancel_cannot_clear_dirty_work() {
        let mut q = AutosaveQueue::default();
        q.edit("p", 0);
        let first = q.next(750, |_| true).unwrap();
        q.flush("p", 800);
        q.edit("p", 850);
        q.complete(&first, Outcome::Saved);
        assert_eq!(q.state("p").unwrap().persisted, 1);
        let latest = q.next(900, |_| true).unwrap();
        assert_eq!(latest.revision, 2);
        q.complete(&latest, Outcome::Failure("disk full".into()));
        assert!(!q.is_flushed());
        assert!(q.next(9000, |_| true).is_none());
        q.flush("p", 9000);
        let retry = q.next(9000, |_| true).unwrap();
        q.complete(&retry, Outcome::Cancelled);
        assert!(!q.is_flushed());
        q.flush("p", 9000);
        let retry = q.next(9000, |_| true).unwrap();
        q.complete(&retry, Outcome::Saved);
        assert!(q.is_flushed());
    }
    #[test]
    fn completion_after_forget_and_reopen_does_not_acknowledge_new_generation() {
        let mut q = AutosaveQueue::default();
        q.edit("p", 0);
        let old = q.next(750, |_| true).unwrap();
        q.forget("p");
        q.edit("p", 800);
        q.complete(&old, Outcome::Saved);
        assert_eq!(q.state("p").unwrap().persisted, 0);
        let new = q.next(1550, |_| true).unwrap();
        assert!(!q.complete(&old, Outcome::Saved));
        q.complete(&new, Outcome::Saved);
        assert!(q.is_flushed());
    }
}
