//! Versioned JSON messages. IDs are exact JavaScript integers; no executable payload.
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub type NodeId = u64;
pub const VERSION: u32 = 1;
pub const MAX_ID: u64 = (1 << 53) - 1;
pub const DOCUMENT: NodeId = 1;
pub const HTML: NodeId = 2;
pub const HEAD: NodeId = 3;
pub const BODY: NodeId = 4;
pub const WINDOW: NodeId = 5;

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Session {
    pub panel: String,
    pub generation: u32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Batch {
    pub version: u32,
    pub session: Session,
    pub seq: u64,
    pub caused_by: Option<u64>,
    pub ops: Vec<Op>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeKind {
    Element,
    Text,
    Comment,
    Fragment,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventPolicy {
    Observe,
    Block,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum Op {
    Create {
        id: NodeId,
        kind: NodeKind,
        name: String,
        namespace: Option<String>,
    },
    Insert {
        parent: NodeId,
        id: NodeId,
        before: Option<NodeId>,
    },
    Remove {
        parent: NodeId,
        id: NodeId,
    },
    Attribute {
        id: NodeId,
        name: String,
        namespace: Option<String>,
        value: Option<String>,
    },
    Text {
        id: NodeId,
        value: String,
    },
    Property {
        id: NodeId,
        name: String,
        value: serde_json::Value,
    },
    Listen {
        id: NodeId,
        event: String,
        enabled: bool,
        policy: EventPolicy,
    },
    Focus {
        id: NodeId,
        prevent_scroll: bool,
    },
    Blur {
        id: NodeId,
    },
    Selection {
        id: NodeId,
        start: u32,
        end: u32,
        direction: String,
    },
    Range {
        anchor: Option<NodeId>,
        anchor_offset: u32,
        focus: Option<NodeId>,
        focus_offset: u32,
    },
    Scroll {
        id: NodeId,
        x: f64,
        y: f64,
        behavior: String,
    },
    Measure {
        request: u64,
        ids: Vec<NodeId>,
        styles: Vec<String>,
    },
    ObserveResize {
        id: NodeId,
        enabled: bool,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LayoutEntry {
    pub id: NodeId,
    pub rect: Rect,
    pub scroll_left: f64,
    pub scroll_top: f64,
    pub scroll_width: f64,
    pub scroll_height: f64,
    pub client_width: f64,
    pub client_height: f64,
    pub styles: BTreeMap<String, String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum PageMessage {
    Applied {
        session: Session,
        seq: u64,
    },
    Event {
        session: Session,
        seq: u64,
        applied: u64,
        target: NodeId,
        event: String,
        bubbles: bool,
        cancelable: bool,
        default_prevented: bool,
        data: BTreeMap<String, serde_json::Value>,
    },
    Layout {
        session: Session,
        applied: u64,
        request: Option<u64>,
        entries: Vec<LayoutEntry>,
    },
    Fault {
        session: Session,
        seq: u64,
        message: String,
    },
}

impl PageMessage {
    pub fn session(&self) -> &Session {
        match self {
            Self::Applied { session, .. }
            | Self::Event { session, .. }
            | Self::Layout { session, .. }
            | Self::Fault { session, .. } => session,
        }
    }
}
