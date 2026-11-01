//! Rust-owned control gestures. The sink enqueues kernel actor calls; it must not wait.
use crate::{Id, Surface};
use serde_json::Value;
use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq)]
pub enum Edit {
    Begin { binding: String },
    Write { binding: String, value: Value },
    Commit { binding: String },
    Cancel { binding: String },
}
pub trait BindingSink {
    fn enqueue(&mut self, edit: Edit) -> Result<(), String>;
}
#[derive(Clone, Debug, Default)]
pub struct Bindings {
    live: BTreeMap<Id, String>,
}
impl Bindings {
    pub fn begin(
        &mut self,
        surface: &Surface,
        id: Id,
        sink: &mut impl BindingSink,
    ) -> Result<(), String> {
        if self.live.contains_key(&id) {
            return Err("gesture already active".into());
        }
        let node = surface.node(id).ok_or("missing binding target")?;
        if node.props.get("disabled").and_then(Value::as_bool) == Some(true) {
            return Err("disabled binding".into());
        }
        let binding = node
            .props
            .get("binding")
            .and_then(Value::as_str)
            .ok_or("missing binding")?
            .to_string();
        sink.enqueue(Edit::Begin {
            binding: binding.clone(),
        })?;
        self.live.insert(id, binding);
        Ok(())
    }
    pub fn write(&self, id: Id, value: Value, sink: &mut impl BindingSink) -> Result<(), String> {
        sink.enqueue(Edit::Write {
            binding: self.live.get(&id).ok_or("no active gesture")?.clone(),
            value,
        })
    }
    pub fn finish(
        &mut self,
        id: Id,
        cancel: bool,
        sink: &mut impl BindingSink,
    ) -> Result<(), String> {
        let binding = self.live.get(&id).ok_or("no active gesture")?.clone();
        sink.enqueue(if cancel {
            Edit::Cancel { binding }
        } else {
            Edit::Commit { binding }
        })?;
        self.live.remove(&id);
        Ok(())
    }
    pub fn once(
        &mut self,
        surface: &Surface,
        id: Id,
        value: Value,
        sink: &mut impl BindingSink,
    ) -> Result<(), String> {
        self.begin(surface, id, sink)?;
        if let Err(error) = self.write(id, value, sink) {
            let _ = self.finish(id, true, sink);
            return Err(error);
        }
        self.finish(id, false, sink)
    }
    /// Destroy/rebind cancels the original binding, never a newly installed descriptor.
    pub fn reconcile(
        &mut self,
        surface: &Surface,
        sink: &mut impl BindingSink,
    ) -> Result<(), String> {
        let stale: Vec<_> = self
            .live
            .iter()
            .filter(|(id, binding)| {
                surface.node(**id).is_none_or(|n| {
                    n.props.get("binding").and_then(Value::as_str) != Some(binding)
                        || n.props.get("disabled").and_then(Value::as_bool) == Some(true)
                })
            })
            .map(|(id, _)| *id)
            .collect();
        for id in stale {
            self.finish(id, true, sink)?;
        }
        Ok(())
    }
}
/// Shipped scrub semantics: clamps scrubs/arrows, typed finite commits are separate.
#[derive(Clone, Copy, Debug)]
pub struct Scrub {
    pub start: f64,
    pub delta: (f64, f64),
    pub step: f64,
    pub speed: f64,
    pub shift: bool,
    pub alt: bool,
    pub bounds: (f64, f64),
}
pub fn scrub(sample: Scrub) -> Result<f64, String> {
    let Scrub {
        start,
        delta: (dx, dy),
        step,
        speed,
        shift,
        alt,
        bounds,
    } = sample;
    if ![start, dx, dy, step, speed, bounds.0, bounds.1]
        .iter()
        .all(|v| v.is_finite())
        || bounds.0 > bounds.1
    {
        return Err("invalid numeric binding".into());
    }
    let multiplier = if shift {
        10.
    } else if alt {
        0.1
    } else {
        1.
    };
    let delta = if dx.abs() >= dy.abs() { dx } else { -dy };
    let value = start + delta * step * speed * multiplier;
    if !value.is_finite() {
        return Err("non-finite scrub".into());
    }
    Ok((value.clamp(bounds.0, bounds.1) * 10_000.).round() / 10_000.)
}
