use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::{BTreeMap, BTreeSet};

pub type Id = u64;
pub type Props = Map<String, Value>;
pub const PRIMITIVES: &[&str] = &[
    "div",
    "text",
    "img",
    "input",
    "textarea",
    "button",
    "scroll",
    "virtual-list",
    "anchored",
    "path",
    "canvas",
    "frame",
    "markdown",
    "code",
    "Glass",
    "material",
    "webview",
];
const MAX_ID: u64 = 9_007_199_254_740_991;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Listener {
    #[serde(default)]
    pub capture: bool,
    #[serde(default)]
    pub bubble: bool,
    #[serde(default)]
    pub prevent_default: bool,
    /// Empty means all keys/events; otherwise cancel only these exact key names.
    #[serde(default)]
    pub keys: Vec<String>,
}
pub type Listeners = BTreeMap<String, Listener>;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "camelCase", deny_unknown_fields)]
pub enum Op {
    Create {
        id: Id,
        r#type: String,
    },
    InsertBefore {
        parent: Id,
        id: Id,
        before: Option<Id>,
    },
    Remove {
        id: Id,
    },
    SetProps {
        id: Id,
        patch: Props,
    },
    SetText {
        id: Id,
        text: String,
    },
    SetListeners {
        id: Id,
        flags: Listeners,
    },
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Commit {
    pub base_revision: u64,
    pub ops: Vec<Op>,
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct InputState {
    pub observed: String,
    pub displayed: String,
    pub selection: (usize, usize),
    pub composing: bool,
    #[serde(default)]
    pub composition: Option<(usize, usize)>,
    pub writes: u64,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Node {
    pub id: Id,
    pub r#type: String,
    pub parent: Option<Id>,
    pub children: Vec<Id>,
    pub props: Props,
    pub text: String,
    pub listeners: Listeners,
    pub input: Option<InputState>,
}
impl Node {
    fn new(id: Id, kind: String) -> Self {
        let input = matches!(kind.as_str(), "input" | "textarea").then(InputState::default);
        Self {
            id,
            r#type: kind,
            parent: None,
            children: vec![],
            props: Props::new(),
            text: String::new(),
            listeners: Listeners::new(),
            input,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Diagnostic {
    pub id: Id,
    pub prop: String,
    pub code: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rejected {
    pub op: Option<usize>,
    pub reason: String,
}
impl std::fmt::Display for Rejected {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}: {}", self.op, self.reason)
    }
}
impl std::error::Error for Rejected {}
#[derive(Clone, Debug)]
pub struct Limits {
    pub nodes: usize,
    pub ops: usize,
    pub depth: usize,
    pub bytes: usize,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            nodes: 100_001,
            ops: 600_000,
            depth: 256,
            bytes: 16 * 1024 * 1024,
        }
    }
}
#[derive(Clone, Debug)]
pub struct Surface {
    revision: u64,
    nodes: BTreeMap<Id, Node>,
    // Acknowledged IDs are never recycled: queued events cannot alias a new element (N7).
    high_id: Id,
    /// The revision that last changed each node's props, text, listeners,
    /// children or place. An event planned earlier may still land on an
    /// element whose subtree has not changed since.
    changed: BTreeMap<Id, u64>,
    pub limits: Limits,
    pub allow_webview: bool,
    styles: crate::style::StyleSheet,
}
impl Default for Surface {
    fn default() -> Self {
        Self {
            revision: 0,
            nodes: BTreeMap::from([(0, Node::new(0, "root".into()))]),
            high_id: 0,
            changed: BTreeMap::new(),
            limits: Limits::default(),
            allow_webview: false,
            styles: crate::style::StyleSheet::default(),
        }
    }
}
impl Surface {
    pub fn revision(&self) -> u64 {
        self.revision
    }
    pub fn nodes(&self) -> &BTreeMap<Id, Node> {
        &self.nodes
    }
    pub fn node(&self, id: Id) -> Option<&Node> {
        self.nodes.get(&id)
    }
    pub fn apply_json(&mut self, wire: &str) -> Result<Vec<Diagnostic>, Rejected> {
        if wire.len() > self.limits.bytes {
            return Err(reject(None, "commit byte limit"));
        }
        let commit = serde_json::from_str(wire)
            .map_err(|e| reject(None, &format!("invalid commit: {e}")))?;
        self.apply(commit)
    }
    /// Worker-side validation. Publish only the accepted immutable tree to the UI thread.
    pub fn apply(&mut self, commit: Commit) -> Result<Vec<Diagnostic>, Rejected> {
        if commit.base_revision != self.revision {
            return Err(reject(None, "stale revision"));
        }
        if commit.ops.len() > self.limits.ops {
            return Err(reject(None, "commit operation limit"));
        }
        if serde_json::to_vec(&commit).map_or(true, |b| b.len() > self.limits.bytes) {
            return Err(reject(None, "commit byte limit"));
        }
        let mut next = self.clone();
        let mut diagnostics = vec![];
        let revision = self
            .revision
            .checked_add(1)
            .ok_or_else(|| reject(None, "revision exhausted"))?;
        for (index, op) in commit.ops.into_iter().enumerate() {
            let touched = next.touched(&op);
            next.apply_op(op, &mut diagnostics)
                .map_err(|reason| reject(Some(index), &reason))?;
            for id in touched {
                next.changed.insert(id, revision);
            }
        }
        next.validate().map_err(|reason| reject(None, &reason))?;
        let nodes = &next.nodes;
        next.changed.retain(|id, _| nodes.contains_key(id));
        next.revision = revision;
        *self = next;
        Ok(diagnostics)
    }
    /// Nodes whose own state an op changes: the node, and any parent whose
    /// children it adds to or takes from.
    fn touched(&self, op: &Op) -> Vec<Id> {
        let parent = |id: &Id| self.nodes.get(id).and_then(|n| n.parent);
        match op {
            Op::Create { id, .. } => vec![*id],
            Op::InsertBefore { parent: to, id, .. } => [Some(*id), Some(*to), parent(id)]
                .into_iter()
                .flatten()
                .collect(),
            Op::Remove { id } => parent(id).into_iter().collect(),
            Op::SetProps { id, .. } | Op::SetText { id, .. } | Op::SetListeners { id, .. } => {
                vec![*id]
            }
        }
    }
    /// True when `id` exists and nothing in its subtree changed after `revision`.
    pub fn unchanged_since(&self, id: Id, revision: u64) -> bool {
        let mut pending = vec![id];
        while let Some(id) = pending.pop() {
            let Some(node) = self.nodes.get(&id) else {
                return false;
            };
            if self.changed.get(&id).is_some_and(|at| *at > revision) {
                return false;
            }
            pending.extend(&node.children);
        }
        true
    }
    fn apply_op(&mut self, op: Op, diagnostics: &mut Vec<Diagnostic>) -> Result<(), String> {
        match op {
            Op::Create { id, r#type } => {
                if id <= self.high_id || id > MAX_ID {
                    return Err("id must be a fresh increasing safe integer".into());
                }
                if !PRIMITIVES.contains(&r#type.as_str()) {
                    return Err("unknown primitive".into());
                }
                if r#type == "webview" && !self.allow_webview {
                    return Err("webview requires host opt-in".into());
                }
                if self.nodes.len() >= self.limits.nodes {
                    return Err("node limit".into());
                }
                self.high_id = id;
                self.nodes.insert(id, Node::new(id, r#type));
            }
            Op::InsertBefore { parent, id, before } => {
                if id == 0 || id == parent {
                    return Err("invalid root/self insertion".into());
                }
                self.require(id)?;
                self.require(parent)?;
                if matches!(
                    self.nodes[&parent].r#type.as_str(),
                    "input" | "textarea" | "img" | "path" | "canvas" | "frame" | "webview"
                ) {
                    return Err("leaf cannot contain children".into());
                }
                if before == Some(id) && self.nodes[&id].parent == Some(parent) {
                    return Ok(());
                }
                if let Some(before) = before
                    && self.require(before)?.parent != Some(parent)
                {
                    return Err("before is not a child of parent".into());
                }
                let mut ancestor = Some(parent);
                while let Some(a) = ancestor {
                    if a == id {
                        return Err("cycle".into());
                    }
                    ancestor = self.nodes[&a].parent;
                }
                self.detach(id);
                let children = &mut self.nodes.get_mut(&parent).unwrap().children;
                let at = before.map_or(children.len(), |b| {
                    children.iter().position(|c| *c == b).unwrap()
                });
                children.insert(at, id);
                self.nodes.get_mut(&id).unwrap().parent = Some(parent);
            }
            Op::Remove { id } => {
                if id == 0 {
                    return Err("cannot remove root".into());
                }
                self.require(id)?;
                self.detach(id);
                let mut pending = vec![id];
                while let Some(id) = pending.pop() {
                    pending.extend(self.nodes.remove(&id).unwrap().children);
                }
            }
            Op::SetText { id, text } => {
                let node = self.node_mut(id)?;
                if !node.children.is_empty() {
                    return Err("setText cannot erase children; remove explicitly".into());
                }
                node.text = text;
            }
            Op::SetListeners { id, flags } => {
                if flags.keys().any(|k| k.is_empty() || k.len() > 64) {
                    return Err("invalid event name".into());
                }
                self.node_mut(id)?.listeners = flags;
            }
            Op::SetProps { id, patch } => {
                if let Some(rules) = patch.get("styleRules") {
                    if id != 0 {
                        return Err("styleRules must be installed on root".into());
                    }
                    let rules = if rules.is_null() {
                        vec![]
                    } else {
                        serde_json::from_value(rules.clone())
                            .map_err(|e| format!("invalid styleRules: {e}"))?
                    };
                    self.styles.replace(rules)?;
                }
                let node = self.node_mut(id)?;
                for (key, value) in patch {
                    if !known_prop(&key) {
                        diagnostics.push(Diagnostic {
                            id,
                            prop: key.clone(),
                            code: "unknownProp".into(),
                        });
                    }
                    crate::schema::validate(&node.r#type, &key, &value)?;
                    validate_prop(&key, &value)?;
                    if key == "value"
                        && !value.is_null()
                        && let Some(input) = &mut node.input
                    {
                        let desired = match &value {
                            Value::String(text) => text.clone(),
                            Value::Number(number)
                                if matches!(
                                    node.props.get("control").and_then(Value::as_str),
                                    Some("number" | "slider" | "angle")
                                ) =>
                            {
                                number.to_string()
                            }
                            _ => {
                                return Err(
                                    "input value must be string (or numeric control number)".into(),
                                );
                            }
                        };
                        if desired != input.observed {
                            input.displayed = desired.clone();
                            input.observed = desired.clone();
                            // Native selection uses UTF-16 offsets, like Cocoa/JS strings.
                            let len = desired.encode_utf16().count();
                            input.selection =
                                (input.selection.0.min(len), input.selection.1.min(len));
                            input.composing = false;
                            input.composition = None;
                            input.writes += 1;
                        }
                    }
                    if value.is_null() {
                        node.props.remove(&key);
                    } else {
                        node.props.insert(key, value);
                    }
                }
            }
        }
        Ok(())
    }
    fn require(&self, id: Id) -> Result<&Node, String> {
        self.nodes
            .get(&id)
            .ok_or_else(|| format!("missing element {id}"))
    }
    fn node_mut(&mut self, id: Id) -> Result<&mut Node, String> {
        self.nodes
            .get_mut(&id)
            .ok_or_else(|| format!("missing element {id}"))
    }
    fn detach(&mut self, id: Id) {
        if let Some(parent) = self.nodes[&id].parent {
            self.nodes
                .get_mut(&parent)
                .unwrap()
                .children
                .retain(|c| *c != id);
        }
        self.nodes.get_mut(&id).unwrap().parent = None;
    }
    fn validate(&self) -> Result<(), String> {
        let mut seen = BTreeSet::new();
        let mut stack = vec![(0, 0)];
        while let Some((id, depth)) = stack.pop() {
            if depth > self.limits.depth {
                return Err("depth limit".into());
            }
            if !seen.insert(id) {
                return Err("cycle/duplicate child".into());
            }
            for child in &self.nodes[&id].children {
                if self.nodes[child].parent != Some(id) {
                    return Err("inconsistent parent".into());
                }
                stack.push((*child, depth + 1));
            }
        }
        for node in self.nodes.values() {
            if node.r#type == "virtual-list"
                && let Some((_, end)) = node.props.get("range").and_then(crate::schema::range)
                && let Some(count) = node.props.get("count").and_then(Value::as_u64)
                && end > count
            {
                return Err("virtual range exceeds count".into());
            }
        }
        if seen.len() != self.nodes.len() {
            return Err("unattached element".into());
        }
        Ok(())
    }
    pub fn computed_style(&self, id: Id) -> Option<Props> {
        self.node(id).map(|node| self.styles.resolve(node))
    }
    pub fn observe_input(
        &mut self,
        id: Id,
        value: String,
        selection: (usize, usize),
        composing: bool,
    ) -> Result<(), String> {
        self.observe_input_composition(id, value, selection, composing, None)
    }
    pub fn observe_input_composition(
        &mut self,
        id: Id,
        value: String,
        selection: (usize, usize),
        composing: bool,
        composition: Option<(usize, usize)>,
    ) -> Result<(), String> {
        let input = self.node_mut(id)?.input.as_mut().ok_or("not an input")?;
        let len = value.encode_utf16().count();
        if selection.0 > selection.1 || selection.1 > len {
            return Err("invalid UTF-16 selection".into());
        }
        if composition.is_some_and(|(a, b)| !composing || a > b || b > len) {
            return Err("invalid UTF-16 composition".into());
        }
        input.composition = composition;
        input.observed = value.clone();
        input.displayed = value;
        input.selection = selection;
        input.composing = composing;
        Ok(())
    }
}
fn reject(op: Option<usize>, reason: &str) -> Rejected {
    Rejected {
        op,
        reason: reason.into(),
    }
}
fn known_prop(key: &str) -> bool {
    matches!(
        key,
        "id" | "class"
            | "role"
            | "attr"
            | "state"
            | "style"
            | "value"
            | "checked"
            | "disabled"
            | "label"
            | "labelledBy"
            | "tabIndex"
            | "focusScope"
            | "binding"
            | "src"
            | "alt"
            | "title"
            | "min"
            | "max"
            | "step"
            | "precision"
            | "unit"
            | "mixed"
            | "control"
            | "options"
            | "selected"
            | "open"
            | "anchor"
            | "placement"
            | "count"
            | "range"
            | "itemSize"
            | "d"
            | "surface"
            | "language"
            | "material"
            | "preventDefault"
            | "focusRequest"
            | "styleRules"
            | "speed"
            | "metadata"
            | "resource"
            | "time"
            | "source"
            | "points"
    ) || key.starts_with("aria-")
        || key.starts_with("data-")
}
fn validate_prop(key: &str, value: &Value) -> Result<(), String> {
    if value.is_null() {
        return Ok(());
    }
    let valid = match key {
        "id" | "class" | "role" | "label" | "labelledBy" | "src" | "alt" | "title" | "control"
        | "unit" | "language" | "material" | "d" | "focusRequest" => value.is_string(),
        "checked" | "disabled" | "mixed" | "open" | "focusScope" => value.is_boolean(),
        "attr" | "state" | "style" | "preventDefault" => value.is_object(),
        "tabIndex" | "min" | "max" | "step" | "precision" | "count" | "itemSize" => {
            value.is_number()
        }
        "binding" => value.is_string(),
        _ => true,
    };
    if !valid {
        return Err(format!("invalid {key}"));
    }
    if key == "style" {
        crate::style::validate_style(value.as_object().unwrap())?;
    }
    Ok(())
}
