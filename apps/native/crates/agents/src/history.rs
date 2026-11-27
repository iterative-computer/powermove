//! Lossless native event receipts in the existing thread archive. Call only on
//! the kernel actor. A callback may be acknowledged after publish returns Ok.
use crate::{
    Event, error,
    persistence::{Store, Threads},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::io;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Receipt {
    /// Stable worker/run identity; sequence numbers may span wire and event
    /// entries, so they must increase but need not be contiguous.
    pub source: String,
    pub sequence: u64,
    pub event: Event,
}
impl Receipt {
    fn validate(&self) -> io::Result<()> {
        if !crate::valid_id(&self.source, 200) {
            return Err(error("Invalid event source"));
        }
        let run = match &self.event {
            Event::Trace { run, .. }
            | Event::Progress { run, .. }
            | Event::Session { run, .. }
            | Event::Completed { run, .. } => Some(run),
            _ => None,
        };
        if run.is_some_and(|run| run != &self.source) {
            return Err(error("Event belongs to another run"));
        }
        Ok(())
    }
}
pub fn publish(
    threads: &mut Threads,
    store: &mut impl Store,
    thread: &str,
    receipts: &[Receipt],
) -> io::Result<()> {
    for receipt in receipts {
        receipt.validate()?;
    }
    threads.update(store, |archive| {
        let record = archive["threads"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|t| t["id"] == thread)
            .ok_or_else(|| error("Unknown thread"))?;
        append(record, receipts)
    })
}
/// Append to a private candidate, so a host can publish its conversation
/// projection and exact event receipt in the same durable archive write.
/// Errors leave the supplied record unchanged.
pub fn append(destination: &mut Value, receipts: &[Receipt]) -> io::Result<()> {
    if !destination.is_object() {
        return Err(error("Unknown thread record shape"));
    }
    for receipt in receipts {
        receipt.validate()?;
    }
    let mut candidate = destination.clone();
    let record = &mut candidate;
    if record.get("nativeRunEvents").is_none() {
        record["nativeRunEvents"] = json!([]);
    }
    let history = record["nativeRunEvents"]
        .as_array_mut()
        .ok_or_else(|| error("Unknown native event history shape"))?;
    let mut index = std::collections::BTreeMap::new();
    let mut latest = std::collections::BTreeMap::<String, u64>::new();
    for (position, previous) in history.iter().enumerate() {
        let old: Receipt = serde_json::from_value(previous.clone())
            .map_err(|_| error("Unknown native event receipt"))?;
        old.validate()?;
        if index
            .insert((old.source.clone(), old.sequence), position)
            .is_some()
        {
            return Err(error("Duplicate native event identity"));
        }
        latest
            .entry(old.source)
            .and_modify(|n| *n = (*n).max(old.sequence))
            .or_insert(old.sequence);
    }
    for receipt in receipts {
        let value = serde_json::to_value(receipt)?;
        let key = (receipt.source.clone(), receipt.sequence);
        if let Some(position) = index.get(&key) {
            if history[*position] != value {
                return Err(error("Event receipt identity conflict"));
            }
            continue;
        }
        if latest
            .get(&receipt.source)
            .is_some_and(|n| receipt.sequence <= *n)
        {
            return Err(error("Out-of-order event receipt"));
        }
        latest.insert(receipt.source.clone(), receipt.sequence);
        index.insert(key, history.len());
        history.push(value);
    }
    *destination = candidate;
    Ok(())
}

/// Read the exact archived JSON, retaining future fields and complete compaction
/// traces. Provider context excerpts are a separate, bounded conversation view.
pub fn read(threads: &Threads, thread: &str) -> io::Result<Vec<Value>> {
    let record = threads
        .thread(thread)
        .ok_or_else(|| error("Unknown thread"))?;
    match record.get("nativeRunEvents") {
        None => Ok(vec![]),
        Some(Value::Array(events)) => Ok(events.clone()),
        Some(_) => Err(error("Unknown native event history shape")),
    }
}
