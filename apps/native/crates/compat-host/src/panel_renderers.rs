//! A realm has one admitted DOM and one input stream. Its visible panel views
//! are independent WebKit renderers, seeded from the current document.
use crate::{CanvasFrame, HostEvent, SyncHandler};
use serde_json::{Value, json};
use std::{
    cell::{Cell, RefCell},
    collections::BTreeMap,
    rc::Rc,
};

struct Pane {
    applied: u64,
    origin: Option<(f64, f64)>,
}

pub(crate) struct PanelFeedback {
    canonical: Rc<dyn SyncHandler>,
    primary: Cell<u64>,
    panes: RefCell<BTreeMap<String, Pane>>,
    sent: Cell<u64>,
    event: Cell<u64>,
    inputs: RefCell<BTreeMap<Option<String>, std::collections::VecDeque<(u64, u64)>>>,
    session: RefCell<Value>,
}

impl PanelFeedback {
    pub(crate) fn new(canonical: Rc<dyn SyncHandler>) -> Self {
        Self {
            canonical,
            primary: Cell::new(0),
            panes: RefCell::default(),
            sent: Cell::new(0),
            event: Cell::new(0),
            inputs: RefCell::default(),
            session: RefCell::default(),
        }
    }

    pub(crate) fn opening(&self, panel: &str) {
        self.panes.borrow_mut().insert(
            panel.to_owned(),
            Pane {
                applied: self.sent.get(),
                origin: None,
            },
        );
    }

    pub(crate) fn new_document(&self, panel: &str) {
        self.inputs.borrow_mut().remove(&Some(panel.to_owned()));
    }

    pub(crate) fn place(&self, panel: &str, origin: (f64, f64)) {
        if let Some(pane) = self.panes.borrow_mut().get_mut(panel) {
            pane.origin = Some(origin);
        }
    }

    pub(crate) fn retired(&self, panel: &str) {
        self.panes.borrow_mut().remove(panel);
        let _ = self.flush();
    }

    pub(crate) fn local_input(&self, owner: Option<&str>, canonical: Option<u64>) -> Option<u64> {
        let canonical = canonical?;
        let streams = self.inputs.borrow();
        streams
            .get(&owner.map(str::to_owned))?
            .iter()
            .rev()
            .find(|(global, _)| *global <= canonical)
            .map(|(_, local)| *local)
    }

    pub(crate) fn received(&self, owner: Option<&str>, mut message: Value) -> crate::Result<()> {
        match message["kind"].as_str() {
            Some("applied") => {
                let sequence = message["seq"]
                    .as_u64()
                    .ok_or(crate::Error::InvalidRequest)?;
                if let Some(owner) = owner {
                    let mut panes = self.panes.borrow_mut();
                    let Some(pane) = panes.get_mut(owner) else {
                        return Ok(());
                    };
                    pane.applied = pane.applied.max(sequence);
                } else {
                    self.primary.set(self.primary.get().max(sequence));
                    self.session.replace(message["session"].clone());
                }
                self.flush()?;
            }
            Some("event") => {
                if let Some(id) = owner {
                    let Some((x, y)) = self.panes.borrow().get(id).and_then(|pane| pane.origin)
                    else {
                        return Ok(());
                    };
                    for (key, offset) in [("clientX", x), ("clientY", y)] {
                        if let Some(value) = message["data"][key].as_f64() {
                            message["data"][key] = json!(value + offset);
                        }
                    }
                }
                // WebKit document-local serials can overlap. The native relay
                // orders input as it reaches the one realm.
                let seq = self.event.get() + 1;
                let local = message["seq"]
                    .as_u64()
                    .ok_or(crate::Error::InvalidRequest)?;
                message["seq"] = json!(seq);
                self.canonical.remote_dom(message)?;
                self.event.set(seq);
                let mut inputs = self.inputs.borrow_mut();
                let stream = inputs.entry(owner.map(str::to_owned)).or_default();
                if stream.len() >= 256 {
                    stream.pop_front();
                }
                stream.push_back((seq, local));
            }
            _ if owner.is_none() => self.canonical.remote_dom(message)?,
            _ => {}
        }
        Ok(())
    }

    fn flush(&self) -> crate::Result<()> {
        let lowest = self
            .panes
            .borrow()
            .values()
            .map(|pane| pane.applied)
            .fold(self.primary.get(), u64::min);
        if lowest > self.sent.get() {
            self.canonical.remote_dom(json!({
                "kind":"applied", "session": &*self.session.borrow(), "seq": lowest
            }))?;
            self.sent.set(lowest);
        }
        Ok(())
    }
}

pub(crate) struct PanelPageEvents {
    pub(crate) owner: String,
    pub(crate) feedback: Rc<PanelFeedback>,
}

impl SyncHandler for PanelPageEvents {
    fn event(&self, event: HostEvent) {
        match event {
            HostEvent::Ready { .. } | HostEvent::GeometryChanged => {
                self.feedback.canonical.event(HostEvent::GeometryChanged);
            }
            HostEvent::DockScroll { .. }
            | HostEvent::DockReveal { .. }
            | HostEvent::DockContent { .. } => self.feedback.canonical.event(event),
            HostEvent::ProcessTerminated | HostEvent::Failure(_) => {
                self.feedback.canonical.event(HostEvent::GeometryChanged);
            }
            _ => {}
        }
    }

    fn remote_dom(&self, payload: Value) -> crate::Result<()> {
        let applied = payload["kind"] == "applied";
        self.feedback.received(Some(&self.owner), payload)?;
        if applied {
            self.feedback.canonical.event(HostEvent::GeometryChanged);
        }
        Ok(())
    }
}

#[derive(Default)]
pub(crate) struct CanvasCache {
    frames: BTreeMap<u64, CanvasFrame>,
    bytes: usize,
}

impl CanvasCache {
    pub(crate) fn update(&mut self, frame: &CanvasFrame) {
        if let Some(old) = self.frames.remove(&frame.node) {
            self.bytes -= old.png.len();
        }
        if frame.png.len() > crate::MAX_PENDING_BYTES {
            return;
        }
        while self.bytes + frame.png.len() > crate::MAX_PENDING_BYTES {
            let Some((_, removed)) = self.frames.pop_first() else {
                break;
            };
            self.bytes -= removed.png.len();
        }
        self.bytes += frame.png.len();
        self.frames.insert(frame.node, frame.clone());
    }

    pub(crate) fn frames(&self) -> impl Iterator<Item = &CanvasFrame> {
        self.frames.values()
    }
}

/// Maximum detached panel renderers kept warm per editor window. Each keeps a
/// WKWebView; two until a release build meets PM-604's memory bar with more.
pub const WARM_PANEL_LIMIT: usize = 2;
const WARM_PANEL_TTL: std::time::Duration = std::time::Duration::from_secs(30);

/// Recently detached views keep their renderer and are never drawn or counted
/// in the live feedback group. Old entries drop their WKWebView.
pub(crate) struct WarmPanels<T> {
    entries: std::collections::VecDeque<(String, std::time::Instant, T)>,
}

impl<T> Default for WarmPanels<T> {
    fn default() -> Self {
        Self {
            entries: Default::default(),
        }
    }
}

impl<T> WarmPanels<T> {
    pub(crate) fn retain(&mut self, id: String, pane: T, now: std::time::Instant) {
        self.expire(now);
        self.entries.push_back((id, now + WARM_PANEL_TTL, pane));
        if self.entries.len() > WARM_PANEL_LIMIT {
            self.entries.pop_front();
        }
    }

    pub(crate) fn reveal(&mut self, id: &str, now: std::time::Instant) -> Option<T> {
        self.expire(now);
        let index = self.entries.iter().position(|(owner, _, _)| owner == id)?;
        self.entries.remove(index).map(|(_, _, pane)| pane)
    }

    pub(crate) fn expire(&mut self, now: std::time::Instant) {
        while self
            .entries
            .front()
            .is_some_and(|(_, expires, _)| *expires <= now)
        {
            self.entries.pop_front();
        }
    }

    pub(crate) fn ids_and_deadlines(&self) -> impl Iterator<Item = (&str, std::time::Instant)> {
        self.entries
            .iter()
            .map(|(id, expires, _)| (id.as_str(), *expires))
    }

    pub(crate) fn remove(&mut self, id: &str) {
        if let Some(index) = self.entries.iter().position(|(owner, _, _)| owner == id) {
            self.entries.remove(index);
        }
    }

    pub(crate) fn next_expiry(&self) -> Option<std::time::Instant> {
        self.entries.front().map(|(_, expires, _)| *expires)
    }

    pub(crate) fn panes(&mut self) -> impl Iterator<Item = (&str, &mut T)> {
        self.entries
            .iter_mut()
            .map(|(id, _, pane)| (id.as_str(), pane))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    #[test]
    fn dock_panel_cache_keeps_two_detached_views_and_expires_them() {
        assert_eq!(WARM_PANEL_LIMIT, 2, "PM-604: no more warm WKWebViews yet");
        let now = Instant::now();
        let mut cache = WarmPanels::default();
        let earliest = Rc::new(());
        let expired = Rc::downgrade(&earliest);
        cache.retain("pexels".into(), earliest, now);
        let voiceover = Rc::new(());
        cache.retain("voiceover".into(), voiceover.clone(), now);
        let notes = Rc::new(());
        let torn_down = Rc::downgrade(&notes);
        cache.retain("notes".into(), notes, now);
        assert!(expired.upgrade().is_none());

        let recalled = cache
            .reveal("voiceover", now + Duration::from_secs(2))
            .unwrap();
        assert!(
            Rc::ptr_eq(&recalled, &voiceover),
            "re-use the live renderer"
        );
        cache.expire(now + Duration::from_secs(29));
        assert!(torn_down.upgrade().is_some());
        cache.expire(now + Duration::from_secs(30));
        assert!(torn_down.upgrade().is_none());
        assert!(cache.next_expiry().is_none());
    }
}
