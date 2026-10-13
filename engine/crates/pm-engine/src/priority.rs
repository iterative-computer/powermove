//! Priority classes inside an engine (contract 12, section 4 principle 7).
//!
//! Interactive frames come first, then agent snapshots, then agent exports,
//! user jobs, generation, and background work. [`Scheduler`] keeps one FIFO
//! queue per class and always hands out the oldest item of the highest
//! class that may run. Work never fails for being busy: it waits in its
//! class and reports how long it waited. While the interactive engine plays,
//! generation and background work pause entirely; everything else in the
//! job engine runs sliced (contract 13, [`crate::slicing`]).

use std::collections::VecDeque;

use pm_protocol::common::HostMicros;
use pm_protocol::error::EngineProcess;
use pm_protocol::priority::PriorityClass;

use crate::clock;

/// Classes from highest to lowest.
pub const CLASSES_BY_PRIORITY: [PriorityClass; 6] = [
    PriorityClass::Interactive,
    PriorityClass::AgentSnapshot,
    PriorityClass::AgentExport,
    PriorityClass::UserJob,
    PriorityClass::Generation,
    PriorityClass::Background,
];

const fn slot(class: PriorityClass) -> usize {
    match class {
        PriorityClass::Interactive => 0,
        PriorityClass::AgentSnapshot => 1,
        PriorityClass::AgentExport => 2,
        PriorityClass::UserJob => 3,
        PriorityClass::Generation => 4,
        PriorityClass::Background => 5,
    }
}

/// Work submitted to the wrong engine process.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("{class:?} work runs in the {expected:?} engine, not the {got:?} engine")]
pub struct WrongProcess {
    pub class: PriorityClass,
    pub expected: EngineProcess,
    pub got: EngineProcess,
}

/// An item handed out by the scheduler.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Scheduled<T> {
    pub class: PriorityClass,
    pub item: T,
    /// Time spent queued (`queued_ms` in agent results, in µs here).
    pub queued_us: u64,
    /// Whether the item must run in bounded slices (playback is on and the
    /// class is sliced during playback).
    pub sliced: bool,
}

#[derive(Debug, Clone)]
struct Entry<T> {
    item: T,
    since: HostMicros,
}

/// One engine's run queue.
#[derive(Debug, Clone)]
pub struct Scheduler<T> {
    process: EngineProcess,
    queues: [VecDeque<Entry<T>>; 6],
    playing: bool,
}

impl<T> Scheduler<T> {
    pub fn new(process: EngineProcess) -> Self {
        Self {
            process,
            queues: Default::default(),
            playing: false,
        }
    }

    /// Whether the interactive engine is playing (from deadline signals in
    /// the job engine, from the transport in the interactive engine).
    pub fn set_playing(&mut self, playing: bool) {
        self.playing = playing;
    }

    pub fn is_playing(&self) -> bool {
        self.playing
    }

    /// Queues `item`. It always waits for its turn; only work for the other
    /// engine process is refused.
    pub fn push(
        &mut self,
        class: PriorityClass,
        item: T,
        now: HostMicros,
    ) -> Result<(), WrongProcess> {
        if class.process() != self.process {
            return Err(WrongProcess {
                class,
                expected: class.process(),
                got: self.process,
            });
        }
        self.queues[slot(class)].push_back(Entry { item, since: now });
        Ok(())
    }

    /// The class that would run next, if any.
    pub fn peek_class(&self) -> Option<PriorityClass> {
        CLASSES_BY_PRIORITY
            .into_iter()
            .find(|&class| self.runnable(class) && !self.queues[slot(class)].is_empty())
    }

    fn runnable(&self, class: PriorityClass) -> bool {
        !(self.playing && class.pauses_during_playback())
    }

    /// Hands out the oldest item of the highest runnable class.
    pub fn pop(&mut self, now: HostMicros) -> Option<Scheduled<T>> {
        let class = self.peek_class()?;
        let entry = self.queues[slot(class)].pop_front()?;
        Some(Scheduled {
            class,
            item: entry.item,
            queued_us: clock::since(now, entry.since),
            sliced: self.playing && class.sliced_during_playback(),
        })
    }

    /// Puts an item that yielded mid-way (a sliced job between slices) back
    /// at the front of its class, keeping its original queue time.
    pub fn requeue_front(&mut self, scheduled: Scheduled<T>, now: HostMicros) {
        let since = HostMicros::new(now.get().saturating_sub(scheduled.queued_us))
            .unwrap_or(HostMicros::ZERO);
        self.queues[slot(scheduled.class)].push_front(Entry {
            item: scheduled.item,
            since,
        });
    }

    /// Whether `class` must yield to queued work of a higher class
    /// (checked between slices or frames; wgpu has no mid-buffer preemption).
    pub fn should_yield(&self, class: PriorityClass) -> bool {
        self.peek_class().is_some_and(|next| next > class) || !self.runnable(class)
    }

    /// Removes queued items matching `predicate` (cancelled jobs, closed
    /// sessions). Returns them in class then queue order.
    pub fn remove_where(&mut self, mut predicate: impl FnMut(&T) -> bool) -> Vec<T> {
        let mut removed = Vec::new();
        for queue in &mut self.queues {
            let mut kept = VecDeque::with_capacity(queue.len());
            for entry in queue.drain(..) {
                if predicate(&entry.item) {
                    removed.push(entry.item);
                } else {
                    kept.push_back(entry);
                }
            }
            *queue = kept;
        }
        removed
    }

    pub fn len(&self) -> usize {
        self.queues.iter().map(VecDeque::len).sum()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Items waiting in `class`.
    pub fn queued(&self, class: PriorityClass) -> usize {
        self.queues[slot(class)].len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(us: u64) -> HostMicros {
        HostMicros::new(us).unwrap()
    }

    #[test]
    fn classes_are_listed_in_preemption_order() {
        for pair in CLASSES_BY_PRIORITY.windows(2) {
            assert!(pair[0] > pair[1]);
        }
    }

    #[test]
    fn highest_class_first_then_fifo() {
        let mut queue = Scheduler::new(EngineProcess::Job);
        queue.push(PriorityClass::UserJob, "user-1", at(0)).unwrap();
        queue.push(PriorityClass::Background, "bg", at(1)).unwrap();
        queue
            .push(PriorityClass::AgentSnapshot, "snap-1", at(2))
            .unwrap();
        queue
            .push(PriorityClass::AgentExport, "agent", at(3))
            .unwrap();
        queue
            .push(PriorityClass::AgentSnapshot, "snap-2", at(4))
            .unwrap();
        let order: Vec<_> = std::iter::from_fn(|| queue.pop(at(10)).map(|s| s.item)).collect();
        assert_eq!(order, ["snap-1", "snap-2", "agent", "user-1", "bg"]);
    }

    #[test]
    fn work_for_the_other_process_is_refused() {
        let mut job = Scheduler::new(EngineProcess::Job);
        assert!(job.push(PriorityClass::Interactive, 1, at(0)).is_err());
        let mut interactive = Scheduler::new(EngineProcess::Interactive);
        assert!(
            interactive
                .push(PriorityClass::AgentSnapshot, 1, at(0))
                .is_err()
        );
        interactive
            .push(PriorityClass::Interactive, 1, at(0))
            .unwrap();
        let scheduled = interactive.pop(at(0)).unwrap();
        assert!(!scheduled.sliced);
    }

    #[test]
    fn playback_pauses_generation_and_slices_the_rest() {
        let mut queue = Scheduler::new(EngineProcess::Job);
        queue
            .push(PriorityClass::Generation, "proxy", at(0))
            .unwrap();
        queue
            .push(PriorityClass::AgentSnapshot, "snap", at(100))
            .unwrap();
        queue.set_playing(true);
        let snap = queue.pop(at(1_100)).unwrap();
        assert_eq!(snap.item, "snap");
        assert!(snap.sliced);
        assert_eq!(snap.queued_us, 1_000, "queued, never failed");
        assert!(queue.pop(at(2_000)).is_none(), "generation pauses");
        assert!(queue.should_yield(PriorityClass::Generation));
        queue.set_playing(false);
        let proxy = queue.pop(at(3_000)).unwrap();
        assert_eq!(proxy.item, "proxy");
        assert!(!proxy.sliced);
        assert_eq!(proxy.queued_us, 3_000);
    }

    #[test]
    fn lower_classes_yield_between_slices_and_keep_their_queue_time() {
        let mut queue = Scheduler::new(EngineProcess::Job);
        queue.push(PriorityClass::UserJob, "export", at(0)).unwrap();
        let export = queue.pop(at(50)).unwrap();
        assert!(!queue.should_yield(PriorityClass::UserJob));
        queue
            .push(PriorityClass::AgentSnapshot, "snap", at(60))
            .unwrap();
        assert!(queue.should_yield(PriorityClass::UserJob));
        queue.requeue_front(export, at(70));
        assert_eq!(queue.pop(at(80)).unwrap().item, "snap");
        let again = queue.pop(at(90)).unwrap();
        assert_eq!(again.item, "export");
        assert_eq!(again.queued_us, 70);
        assert!(queue.is_empty());
    }

    #[test]
    fn remove_where_drops_cancelled_work() {
        let mut queue = Scheduler::new(EngineProcess::Job);
        for (i, class) in [
            PriorityClass::UserJob,
            PriorityClass::AgentExport,
            PriorityClass::UserJob,
        ]
        .into_iter()
        .enumerate()
        {
            queue.push(class, i, at(0)).unwrap();
        }
        assert_eq!(queue.remove_where(|&i| i != 1), vec![0, 2]);
        assert_eq!(queue.len(), 1);
        assert_eq!(queue.queued(PriorityClass::AgentExport), 1);
    }
}
