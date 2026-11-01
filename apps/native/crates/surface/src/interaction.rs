//! Native event planning never calls JS; declared defaults are decided before enqueue.
use crate::{Id, Surface};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Delivery {
    pub id: Id,
    pub capture: bool,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EventPlan {
    pub revision: u64,
    pub target: Id,
    pub prevent_default: bool,
    pub deliveries: Vec<Delivery>,
}
#[derive(Clone, Debug, Default)]
pub struct Interaction {
    captures: BTreeMap<u64, Id>,
    pub focused: Option<Id>,
    scopes: Vec<(Id, Option<Id>)>,
}
impl Interaction {
    pub fn capture(&mut self, surface: &Surface, pointer: u64, id: Id) -> Result<(), String> {
        if surface.node(id).is_none() {
            return Err("missing capture target".into());
        }
        self.captures.insert(pointer, id);
        Ok(())
    }
    pub fn release(&mut self, pointer: u64) {
        self.captures.remove(&pointer);
    }
    pub fn captured(&self, pointer: u64) -> Option<Id> {
        self.captures.get(&pointer).copied()
    }
    /// Call after an accepted commit; rejected commits do not alter focus/capture.
    pub fn reconcile(&mut self, surface: &Surface) {
        self.captures.retain(|_, id| surface.node(*id).is_some());
        self.scopes.retain(|(id, _)| surface.node(*id).is_some());
        if self.focused.is_some_and(|id| surface.node(id).is_none()) {
            self.focused = None;
        }
    }
    pub fn plan(
        &self,
        surface: &Surface,
        hit: Id,
        pointer: Option<u64>,
        event: &str,
        key: Option<&str>,
    ) -> Result<EventPlan, String> {
        let target = pointer.and_then(|p| self.captured(p)).unwrap_or(hit);
        let path = path(surface, target)?;
        let mut deliveries = vec![];
        let mut prevent_default = false;
        for id in path.iter().rev() {
            if let Some(flags) = surface.node(*id).unwrap().listeners.get(event) {
                let key_matches =
                    flags.keys.is_empty() || key.is_some_and(|k| flags.keys.iter().any(|v| v == k));
                prevent_default |= flags.prevent_default && key_matches;
                if flags.capture {
                    deliveries.push(Delivery {
                        id: *id,
                        capture: true,
                    });
                }
            }
        }
        for id in &path {
            if surface
                .node(*id)
                .unwrap()
                .listeners
                .get(event)
                .is_some_and(|f| f.bubble)
            {
                deliveries.push(Delivery {
                    id: *id,
                    capture: false,
                });
            }
        }
        Ok(EventPlan {
            revision: surface.revision(),
            target,
            prevent_default,
            deliveries,
        })
    }
    pub fn push_scope(&mut self, surface: &Surface, id: Id) -> Result<(), String> {
        if !surface
            .node(id)
            .and_then(|n| n.props.get("focusScope"))
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
        {
            return Err("not a focus scope".into());
        }
        self.scopes.push((id, self.focused));
        self.focused = None;
        Ok(())
    }
    pub fn pop_scope(&mut self, surface: &Surface) {
        if let Some((_, restore)) = self.scopes.pop() {
            self.focused = restore.filter(|id| surface.node(*id).is_some());
        }
    }
    pub fn focus(&mut self, surface: &Surface, id: Id) -> Result<(), String> {
        let node = surface.node(id).ok_or("missing focus target")?;
        if node.props.get("disabled").and_then(|v| v.as_bool()) == Some(true) {
            return Err("disabled focus target".into());
        }
        if let Some((scope, _)) = self.scopes.last()
            && !path(surface, id)?.contains(scope)
        {
            return Err("focus outside active scope".into());
        }
        self.focused = Some(id);
        Ok(())
    }
    pub fn tab(&mut self, surface: &Surface, backwards: bool) -> Result<Option<Id>, String> {
        let root = self.scopes.last().map_or(0, |s| s.0);
        let mut order = vec![];
        let mut pending = vec![root];
        while let Some(id) = pending.pop() {
            let node = surface.node(id).ok_or("missing focus scope")?;
            let disabled = node.props.get("disabled").and_then(|v| v.as_bool()) == Some(true);
            let tab = node.props.get("tabIndex").and_then(|v| v.as_i64());
            if !disabled
                && (tab.is_some_and(|n| n >= 0)
                    || (tab.is_none()
                        && matches!(node.r#type.as_str(), "input" | "textarea" | "button")))
            {
                order.push(id);
            }
            pending.extend(node.children.iter().rev());
        }
        if order.is_empty() {
            self.focused = None;
            return Ok(None);
        }
        let current = self
            .focused
            .and_then(|id| order.iter().position(|n| *n == id));
        let next = match (current, backwards) {
            (Some(i), true) => (i + order.len() - 1) % order.len(),
            (Some(i), false) => (i + 1) % order.len(),
            (None, true) => order.len() - 1,
            (None, false) => 0,
        };
        self.focused = Some(order[next]);
        Ok(self.focused)
    }
}
fn path(surface: &Surface, id: Id) -> Result<Vec<Id>, String> {
    let mut path = vec![];
    let mut cursor = Some(id);
    while let Some(id) = cursor {
        let node = surface.node(id).ok_or("missing event target")?;
        path.push(id);
        cursor = node.parent;
    }
    Ok(path)
}
/// Coordinates are logical points, not device pixels; begins at exactly 3 pt.
#[derive(Clone, Debug)]
pub struct Drag {
    start: (f64, f64),
    active: bool,
}
impl Drag {
    pub fn new(x: f64, y: f64) -> Self {
        Self {
            start: (x, y),
            active: false,
        }
    }
    pub fn update(&mut self, x: f64, y: f64) -> Option<(f64, f64)> {
        if !x.is_finite() || !y.is_finite() {
            return None;
        }
        let delta = (x - self.start.0, y - self.start.1);
        self.active |= delta.0.hypot(delta.1) >= 3.;
        self.active.then_some(delta)
    }
}
