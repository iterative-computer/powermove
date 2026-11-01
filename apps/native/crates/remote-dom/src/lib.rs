//! Worker-owned, platform-neutral remote DOM admission and nonblocking routing.
//! The macOS host supplies queue-backed sinks; this crate never enters JS or WebKit.
pub mod protocol;
use protocol::*;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::sync::Arc;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    Invalid(&'static str),
    StaleSession,
    Sequence,
    TooLarge,
    Backpressure,
    PageFault(String),
}
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for Error {}

/// Implement as a bounded queue try_send, never as evaluateJavaScript + wait.
/// Failure MUST leave the queue unchanged. Accepted messages remain ordered.
pub trait TrySink<T> {
    fn try_send(&mut self, message: T) -> Result<(), Error>;
}

#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub bytes: usize,
    pub operations: usize,
    pub nodes: usize,
    pub in_flight: u64,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            bytes: 1024 * 1024,
            operations: 16_384,
            nodes: 100_000,
            in_flight: 8,
        }
    }
}

#[derive(Clone, Debug)]
struct Node {
    kind: Option<NodeKind>,
    parent: Option<NodeId>,
    listeners: BTreeSet<String>,
}

/// One router per panel/generation. Retire it together with its page and realm.
/// Detached nodes retain identity until generation retirement.
#[derive(Clone, Debug)]
pub struct Router {
    session: Session,
    limits: Limits,
    nodes: Arc<BTreeMap<NodeId, Node>>,
    views: BTreeMap<u64, Arc<BTreeMap<NodeId, Node>>>,
    sent: u64,
    applied: u64,
    event: u64,
    faulted: bool,
    measures: BTreeSet<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Inspection {
    pub session: Session,
    pub sent: u64,
    pub applied: u64,
    pub last_event: u64,
    pub nodes: usize,
    pub faulted: bool,
}

impl Router {
    pub fn new(session: Session, limits: Limits) -> Result<Self, Error> {
        if session.panel.is_empty() || session.panel.len() > 256 || session.generation == 0 {
            return Err(Error::Invalid("session"));
        }
        let mut nodes = BTreeMap::new();
        for (id, kind, parent) in [
            (DOCUMENT, None, Some(WINDOW)),
            (HTML, Some(NodeKind::Element), Some(DOCUMENT)),
            (HEAD, Some(NodeKind::Element), Some(HTML)),
            (BODY, Some(NodeKind::Element), Some(HTML)),
            (WINDOW, None, None),
        ] {
            nodes.insert(
                id,
                Node {
                    kind,
                    parent,
                    listeners: BTreeSet::new(),
                },
            );
        }
        let nodes = Arc::new(nodes);
        let views = BTreeMap::from([(0, Arc::clone(&nodes))]);
        Ok(Self {
            session,
            limits,
            nodes,
            views,
            sent: 0,
            applied: 0,
            event: 0,
            faulted: false,
            measures: BTreeSet::new(),
        })
    }

    pub fn inspect(&self) -> Inspection {
        Inspection {
            session: self.session.clone(),
            sent: self.sent,
            applied: self.applied,
            last_event: self.event,
            nodes: self.nodes.len(),
            faulted: self.faulted,
        }
    }

    pub fn decode_batch(&self, bytes: &[u8]) -> Result<Batch, Error> {
        if bytes.len() > self.limits.bytes {
            return Err(Error::TooLarge);
        }
        serde_json::from_slice(bytes).map_err(|_| Error::Invalid("batch JSON"))
    }

    pub fn decode_page(&self, bytes: &[u8]) -> Result<PageMessage, Error> {
        if bytes.len() > self.limits.bytes {
            return Err(Error::TooLarge);
        }
        serde_json::from_slice(bytes).map_err(|_| Error::Invalid("page JSON"))
    }

    fn session(&self, session: &Session) -> Result<(), Error> {
        if session != &self.session {
            return Err(Error::StaleSession);
        }
        if self.faulted {
            return Err(Error::Invalid("faulted page requires new generation"));
        }
        Ok(())
    }

    /// Validate the whole batch against staged state; neither admission failure nor
    /// a full transport queue advances sequence/tree state. Retry the same batch.
    pub fn route_batch(
        &mut self,
        batch: Batch,
        page: &mut impl TrySink<Batch>,
    ) -> Result<(), Error> {
        self.session(&batch.session)?;
        if batch.version != VERSION {
            return Err(Error::Invalid("version"));
        }
        if batch.seq != self.sent + 1 || batch.seq > MAX_ID {
            return Err(Error::Sequence);
        }
        if batch
            .caused_by
            .is_some_and(|event| event == 0 || event > self.event)
        {
            return Err(Error::Sequence);
        }
        if self.sent - self.applied >= self.limits.in_flight {
            return Err(Error::Backpressure);
        }
        if batch.ops.len() > self.limits.operations
            || serde_json::to_vec(&batch)
                .map_err(|_| Error::Invalid("non-JSON batch"))?
                .len()
                > self.limits.bytes
        {
            return Err(Error::TooLarge);
        }
        let mut staged = self.clone();
        for op in &batch.ops {
            staged.validate(op)?;
        }
        if staged.nodes.len() > self.limits.nodes {
            return Err(Error::TooLarge);
        }
        staged.sent = batch.seq;
        staged.views.insert(batch.seq, Arc::clone(&staged.nodes));
        page.try_send(batch)?;
        *self = staged;
        Ok(())
    }

    fn node(&self, id: NodeId) -> Result<&Node, Error> {
        self.nodes.get(&id).ok_or(Error::Invalid("unknown node"))
    }
    fn element(&self, id: NodeId) -> Result<(), Error> {
        if self.node(id)?.kind != Some(NodeKind::Element) {
            return Err(Error::Invalid("expected element"));
        }
        Ok(())
    }
    fn validate(&mut self, op: &Op) -> Result<(), Error> {
        match op {
            Op::Create {
                id,
                kind,
                name,
                namespace,
            } => {
                if *id <= WINDOW || *id > MAX_ID || self.nodes.contains_key(id) {
                    return Err(Error::Invalid("node id"));
                }
                if *kind == NodeKind::Element
                    && (!valid_name(name)
                        || ["script", "iframe", "object", "embed", "base"]
                            .contains(&name.to_ascii_lowercase().as_str())
                        || namespace.as_ref().is_some_and(|s| s.len() > 2048))
                {
                    return Err(Error::Invalid("element name"));
                }
                Arc::make_mut(&mut self.nodes).insert(
                    *id,
                    Node {
                        kind: Some(*kind),
                        parent: None,
                        listeners: BTreeSet::new(),
                    },
                );
            }
            Op::Insert { parent, id, before } => {
                if *id <= WINDOW {
                    return Err(Error::Invalid("reserved root"));
                }
                self.node(*id)?;
                if !matches!(
                    self.node(*parent)?.kind,
                    Some(NodeKind::Element | NodeKind::Fragment)
                ) {
                    return Err(Error::Invalid("parent"));
                }
                if let Some(before) = before {
                    if self.node(*before)?.parent != Some(*parent) {
                        return Err(Error::Invalid("anchor"));
                    }
                    if before == id {
                        return Ok(());
                    }
                }
                let mut ancestor = Some(*parent);
                while let Some(p) = ancestor {
                    if p == *id {
                        return Err(Error::Invalid("cycle"));
                    }
                    ancestor = self.node(p)?.parent;
                }
                Arc::make_mut(&mut self.nodes).get_mut(id).unwrap().parent = Some(*parent);
            }
            Op::Remove { parent, id } => {
                if *id <= WINDOW || self.node(*id)?.parent != Some(*parent) {
                    return Err(Error::Invalid("remove parent"));
                }
                Arc::make_mut(&mut self.nodes).get_mut(id).unwrap().parent = None;
            }
            Op::Attribute {
                id, name, value, ..
            } => {
                self.element(*id)?;
                if !valid_attribute(name, value.as_deref()) {
                    return Err(Error::Invalid("attribute"));
                }
            }
            Op::Text { id, .. } => {
                if !matches!(
                    self.node(*id)?.kind,
                    Some(NodeKind::Text | NodeKind::Comment)
                ) {
                    return Err(Error::Invalid("text target"));
                }
            }
            Op::Property { id, name, value } => {
                self.element(*id)?;
                if !valid_property(name, value) {
                    return Err(Error::Invalid("property"));
                }
            }
            Op::Listen {
                id, event, enabled, ..
            } => {
                self.node(*id)?;
                if event.is_empty() || event.len() > 128 {
                    return Err(Error::Invalid("event"));
                }
                let node = Arc::make_mut(&mut self.nodes).get_mut(id).unwrap();
                if *enabled {
                    node.listeners.insert(event.clone());
                } else {
                    node.listeners.remove(event);
                }
            }
            Op::Focus { id, .. } | Op::Blur { id } | Op::ObserveResize { id, .. } => {
                self.element(*id)?
            }
            Op::Selection {
                id,
                start,
                end,
                direction,
            } => {
                self.element(*id)?;
                if start > end || !["forward", "backward", "none"].contains(&direction.as_str()) {
                    return Err(Error::Invalid("selection"));
                }
            }
            Op::Scroll { id, x, y, behavior } => {
                self.element(*id)?;
                if !x.is_finite()
                    || !y.is_finite()
                    || !["auto", "instant", "smooth"].contains(&behavior.as_str())
                {
                    return Err(Error::Invalid("scroll"));
                }
            }
            Op::Measure {
                request,
                ids,
                styles,
            } => {
                if *request == 0 || *request > MAX_ID || !self.measures.insert(*request) {
                    return Err(Error::Invalid("measure id"));
                }
                if ids.len() > self.limits.nodes || styles.len() > 128 {
                    return Err(Error::TooLarge);
                }
                for id in ids {
                    self.element(*id)?;
                }
            }
        }
        Ok(())
    }

    /// Page feedback goes straight to the realm queue. No UI thread enters JS.
    /// Duplicate acknowledgements are harmless; stale/replayed events are refused.
    pub fn route_page(
        &mut self,
        message: PageMessage,
        realm: &mut impl TrySink<PageMessage>,
    ) -> Result<(), Error> {
        self.session(message.session())?;
        if serde_json::to_vec(&message)
            .map_err(|_| Error::Invalid("non-JSON page message"))?
            .len()
            > self.limits.bytes
        {
            return Err(Error::TooLarge);
        }
        let mut staged = self.clone();
        match &message {
            PageMessage::Applied { seq, .. } => {
                if *seq < self.applied || *seq > self.sent {
                    return Err(Error::Sequence);
                }
                staged.applied = *seq;
                staged.views.retain(|version, _| version >= seq);
            }
            PageMessage::Event {
                seq,
                applied,
                target,
                event,
                ..
            } => {
                if *seq != self.event + 1
                    || *seq > MAX_ID
                    || *applied < self.applied
                    || *applied > self.sent
                {
                    return Err(Error::Sequence);
                }
                let view = self.views.get(applied).ok_or(Error::Sequence)?;
                let mut ancestor = Some(*target);
                let mut subscribed = false;
                let mut attached = false;
                while let Some(id) = ancestor {
                    let node = view.get(&id).ok_or(Error::Invalid("unknown node"))?;
                    subscribed |= node.listeners.contains(event);
                    attached |= id == WINDOW;
                    ancestor = node.parent;
                }
                if !subscribed || !attached {
                    return Err(Error::Invalid("unsubscribed event"));
                }
                staged.event = *seq;
            }
            PageMessage::Layout {
                applied,
                request,
                entries,
                ..
            } => {
                if *applied < self.applied || *applied > self.sent {
                    return Err(Error::Sequence);
                }
                if let Some(request) = request
                    && !staged.measures.remove(request)
                {
                    return Err(Error::Invalid("unknown measure"));
                }
                let mut seen = BTreeSet::new();
                for entry in entries {
                    if self
                        .views
                        .get(applied)
                        .and_then(|nodes| nodes.get(&entry.id))
                        .is_none_or(|node| node.kind != Some(NodeKind::Element))
                    {
                        return Err(Error::Invalid("layout target"));
                    }
                    if !seen.insert(entry.id)
                        || ![
                            entry.rect.x,
                            entry.rect.y,
                            entry.rect.width,
                            entry.rect.height,
                            entry.scroll_left,
                            entry.scroll_top,
                            entry.scroll_width,
                            entry.scroll_height,
                            entry.client_width,
                            entry.client_height,
                        ]
                        .iter()
                        .all(|n| n.is_finite())
                        || entry.rect.width < 0.0
                        || entry.rect.height < 0.0
                    {
                        return Err(Error::Invalid("layout"));
                    }
                }
            }
            PageMessage::Fault { seq, message, .. } => {
                if *seq == 0 || *seq > self.sent {
                    return Err(Error::Sequence);
                }
                realm.try_send(PageMessage::Fault {
                    session: self.session.clone(),
                    seq: *seq,
                    message: message.clone(),
                })?;
                self.faulted = true;
                return Err(Error::PageFault(message.clone()));
            }
        }
        realm.try_send(message)?;
        *self = staged;
        Ok(())
    }
}

fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 256
        && !name
            .chars()
            .any(|c| c.is_whitespace() || c.is_control() || "<>/\"'=\0".contains(c))
}
fn valid_property(name: &str, value: &serde_json::Value) -> bool {
    match name {
        "value" => value.is_string(),
        "checked" | "indeterminate" | "selected" | "disabled" | "multiple" | "readOnly"
        | "muted" | "open" => value.is_boolean(),
        "selectedIndex" | "tabIndex" => value
            .as_i64()
            .is_some_and(|n| (-1..=i32::MAX as i64).contains(&n)),
        _ => false,
    }
}

fn valid_attribute(name: &str, value: Option<&str>) -> bool {
    let lower = name.to_ascii_lowercase();
    if !valid_name(name) || lower.starts_with("on") || lower == "srcdoc" {
        return false;
    }
    if ["href", "src", "xlink:href", "action", "formaction"].contains(&lower.as_str()) {
        let value: String = value
            .unwrap_or_default()
            .chars()
            .filter(|c| !c.is_ascii_whitespace() && !c.is_control())
            .collect::<String>()
            .to_ascii_lowercase();
        if value.starts_with("javascript:")
            || value.starts_with("vbscript:")
            || value.starts_with("data:text/html")
        {
            return false;
        }
    }
    true
}
