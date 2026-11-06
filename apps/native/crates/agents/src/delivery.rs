//! Ordered worker-to-actor boundary. Native inspection is a separate typed
//! channel from the frozen provider callbacks; no observation is discarded.
//! The caller owns one queue on its actor and drains it without blocking.
use crate::{Effects, Event};
use serde_json::Value;
use std::collections::VecDeque;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Surface {
    Codex,
    Claude,
    CodexExec,
}
#[derive(Debug, PartialEq)]
pub enum Delivery {
    Wire(Value),
    Callback(Event),
    Inspection(Event),
    CloseStdin,
}
#[derive(Debug, PartialEq)]
pub struct Entry {
    pub sequence: u64,
    pub delivery: Delivery,
}
pub struct Queue {
    capacity: usize,
    next: u64,
    entries: VecDeque<Entry>,
}
impl Queue {
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity,
            next: 0,
            entries: VecDeque::new(),
        }
    }
    /// Admission is atomic: on saturation the complete effects are returned for
    /// retry, with no wire write, callback or sequence number consumed.
    pub fn push(&mut self, surface: Surface, effects: Effects) -> Result<(), Effects> {
        let count = effects
            .outgoing
            .len()
            .saturating_add(effects.events.len())
            .saturating_add(usize::from(effects.close_stdin));
        if count > self.capacity.saturating_sub(self.entries.len())
            || u64::try_from(count)
                .ok()
                .and_then(|n| self.next.checked_add(n))
                .is_none()
        {
            return Err(effects);
        }
        for delivery in ordered(surface, effects) {
            self.entries.push_back(Entry {
                sequence: self.next,
                delivery,
            });
            self.next += 1;
        }
        Ok(())
    }
    pub fn pop(&mut self) -> Option<Entry> {
        self.entries.pop_front()
    }
}
/// This is the worker's dispatch order, not a promise that a pipe write has
/// reached the provider. The interrupt RPC acknowledgement supplies that fence.
pub(crate) fn ordered(surface: Surface, effects: Effects) -> impl Iterator<Item = Delivery> {
    effects
        .outgoing
        .into_iter()
        .map(Delivery::Wire)
        .chain(effects.events.into_iter().map(move |e| event(surface, e)))
        .chain(effects.close_stdin.then_some(Delivery::CloseStdin))
}

/// Classify the public callback contract while retaining every native event.
pub(crate) fn event(surface: Surface, e: Event) -> Delivery {
    let callback = match &e {
        Event::Trace { .. }
        | Event::Progress { .. }
        | Event::Completed { .. }
        | Event::Warning(_) => true,
        Event::Session { .. } => surface == Surface::Claude,
        Event::Ready | Event::Notification(_) | Event::Stopped(_) => false,
    };
    if callback {
        Delivery::Callback(e)
    } else {
        Delivery::Inspection(e)
    }
}
