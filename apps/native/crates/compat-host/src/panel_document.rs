//! The current admitted DOM, ready to seed a new renderer without restarting
//! the extension realm or retaining an ever-growing history of its batches.
use pm_remote_dom::protocol::{Batch, NodeId, Op, Session};
use std::collections::BTreeMap;

#[derive(Default)]
struct Node {
    create: Option<Op>,
    attributes: BTreeMap<(Option<String>, String), Op>,
    properties: BTreeMap<String, Op>,
    text: Option<Op>,
    listeners: BTreeMap<String, Op>,
    observe: Option<Op>,
    scroll: Option<Op>,
    children: Vec<NodeId>,
    parent: Option<NodeId>,
}

#[derive(Default)]
pub(crate) struct PanelDocument {
    nodes: BTreeMap<NodeId, Node>,
    session: Option<Session>,
    version: u32,
    sequence: u64,
}

impl PanelDocument {
    pub(crate) fn sequence(&self) -> u64 {
        self.sequence
    }

    /// The realm's fallback root placement is replaced by the native slot
    /// before paint. Its hide/show notifications do not change the bitmap.
    pub(crate) fn changes_paint(&self, batch: &Batch) -> bool {
        self.session.as_ref() != Some(&batch.session)
            || batch.ops.iter().any(|op| match op {
                Op::Listen { .. } | Op::ObserveResize { .. } | Op::Measure { .. } => false,
                Op::Attribute { id, name, namespace: None, value: Some(style) }
                    if name == "style" && fallback_style(style) => {
                        !self.nodes.get(id).is_some_and(|node| {
                            node.attributes.contains_key(&(None, "data-pm-contribution".into()))
                                && matches!(node.attributes.get(&(None, "style".into())),
                                    Some(Op::Attribute { value: Some(prior), .. }) if fallback_style(prior))
                        })
                    }
                _ => true,
            })
    }

    pub(crate) fn apply(&mut self, batch: &Batch) {
        if self.session.as_ref() != Some(&batch.session) || batch.seq == 1 {
            *self = Self::default();
            self.session = Some(batch.session.clone());
        }
        self.version = batch.version;
        self.sequence = batch.seq;
        for op in &batch.ops {
            match op {
                Op::Create { id, .. } => {
                    self.nodes.entry(*id).or_default().create = Some(op.clone())
                }
                Op::Insert { parent, id, before } => {
                    self.remove(*id);
                    let siblings = &mut self.nodes.entry(*parent).or_default().children;
                    let index = before
                        .and_then(|before| siblings.iter().position(|id| *id == before))
                        .unwrap_or(siblings.len());
                    siblings.insert(index, *id);
                    self.nodes.entry(*id).or_default().parent = Some(*parent);
                }
                Op::Remove { id, .. } => self.remove(*id),
                Op::Attribute {
                    id,
                    name,
                    namespace,
                    value,
                } => {
                    let attributes = &mut self.nodes.entry(*id).or_default().attributes;
                    let key = (namespace.clone(), name.clone());
                    if value.is_some() {
                        attributes.insert(key, op.clone());
                    } else {
                        attributes.remove(&key);
                    }
                }
                Op::Property { id, name, .. } => {
                    self.nodes
                        .entry(*id)
                        .or_default()
                        .properties
                        .insert(name.clone(), op.clone());
                }
                Op::Text { id, .. } => self.nodes.entry(*id).or_default().text = Some(op.clone()),
                Op::Listen {
                    id, event, enabled, ..
                } => {
                    let listeners = &mut self.nodes.entry(*id).or_default().listeners;
                    if *enabled {
                        listeners.insert(event.clone(), op.clone());
                    } else {
                        listeners.remove(event);
                    }
                }
                Op::ObserveResize { id, enabled } => {
                    self.nodes.entry(*id).or_default().observe = enabled.then(|| op.clone());
                }
                Op::Scroll { id, .. } => {
                    self.nodes.entry(*id).or_default().scroll = Some(op.clone())
                }
                // Input, selection and measurement are live requests. A newly
                // visible renderer restores the DOM, not yesterday's action.
                Op::Focus { .. }
                | Op::Blur { .. }
                | Op::Selection { .. }
                | Op::Range { .. }
                | Op::Measure { .. } => {}
            }
        }
    }

    fn remove(&mut self, id: NodeId) {
        if let Some(parent) = self.nodes.entry(id).or_default().parent.take() {
            self.nodes
                .entry(parent)
                .or_default()
                .children
                .retain(|child| *child != id);
        }
    }

    pub(crate) fn snapshot(&self) -> Option<Batch> {
        let mut ops = Vec::new();
        // Create all nodes first. Insert order is each parent's DOM order.
        ops.extend(self.nodes.values().filter_map(|node| node.create.clone()));
        for (&id, node) in &self.nodes {
            ops.extend(node.attributes.values().cloned());
            ops.extend(node.text.iter().cloned());
            ops.extend(node.children.iter().map(|child| Op::Insert {
                parent: id,
                id: *child,
                before: None,
            }));
        }
        // WebKit can reset a property when its parent is attached, or when its
        // owning select gets its options. Apply these after the tree exists.
        for node in self.nodes.values() {
            ops.extend(node.properties.values().cloned());
            ops.extend(node.listeners.values().cloned());
            ops.extend(node.observe.iter().cloned());
            ops.extend(node.scroll.iter().cloned());
        }
        Some(Batch {
            version: self.version,
            session: self.session.clone()?,
            seq: self.sequence,
            caused_by: None,
            ops,
        })
    }
}

fn fallback_style(style: &str) -> bool {
    if style == "display:none" {
        return true;
    }
    let Some(bounds) = style
        .strip_prefix("position:fixed;")
        .and_then(|style| style.strip_suffix(";overflow:auto;box-sizing:border-box"))
    else {
        return false;
    };
    let mut fields = bounds.split(';');
    ["left:", "top:", "width:", "height:"]
        .into_iter()
        .all(|field| {
            fields
                .next()
                .and_then(|value| value.strip_prefix(field))
                .and_then(|value| value.strip_suffix("px"))
                .and_then(|value| value.parse::<f64>().ok())
                .is_some_and(f64::is_finite)
        })
        && fields.next().is_none()
}

#[cfg(test)]
mod tests {
    use super::*;
    use pm_remote_dom::protocol::{BODY, NodeKind, VERSION};

    #[test]
    fn native_geometry_updates_do_not_invalidate_the_painted_panel() {
        let mut document = PanelDocument::default();
        let attribute = |name: &str, value: &str| Op::Attribute {
            id: 6,
            name: name.into(),
            namespace: None,
            value: Some(value.into()),
        };
        let session = Session {
            panel: "compat".into(),
            generation: 1,
        };
        let initial = Batch {
            version: VERSION,
            session: session.clone(),
            seq: 1,
            caused_by: None,
            ops: vec![
                attribute("data-pm-contribution", "pexels"),
                attribute("style", "display:none"),
            ],
        };
        document.apply(&initial);
        let changed = |name, value| Batch {
            seq: 2,
            ops: vec![attribute(name, value)],
            ..initial.clone()
        };
        assert!(!document.changes_paint(&changed("style", "position:fixed;left:4px;top:2px;width:240px;height:42px;overflow:auto;box-sizing:border-box")));
        assert!(document.changes_paint(&changed("style", "position:fixed;left:4px;top:2px;width:240px;height:42px;overflow:auto;box-sizing:border-box;color:blue")));
        assert!(document.changes_paint(&changed("aria-label", "new title")));
        document.apply(&changed("style", "background:blue"));
        assert!(document.changes_paint(&changed("style", "display:none")));
    }

    #[test]
    fn dock_panel_snapshot_seeds_the_current_tree_and_drops_old_work() {
        let session = Session {
            panel: "compat".into(),
            generation: 1,
        };
        let mut document = PanelDocument::default();
        let batch = |seq, ops| Batch {
            version: VERSION,
            session: session.clone(),
            seq,
            caused_by: None,
            ops,
        };
        let create = |id| Op::Create {
            id,
            kind: NodeKind::Element,
            name: "section".into(),
            namespace: None,
        };
        document.apply(&batch(
            1,
            vec![
                create(6),
                create(7),
                Op::Insert {
                    parent: BODY,
                    id: 6,
                    before: None,
                },
                Op::Insert {
                    parent: BODY,
                    id: 7,
                    before: Some(6),
                },
                Op::Text {
                    id: 6,
                    value: "old label".into(),
                },
            ],
        ));
        document.apply(&batch(
            2,
            vec![
                Op::Remove {
                    parent: BODY,
                    id: 7,
                },
                Op::Text {
                    id: 6,
                    value: "current label".into(),
                },
                Op::Measure {
                    request: 1,
                    ids: vec![6],
                    styles: vec![],
                },
            ],
        ));
        let snapshot = document.snapshot().unwrap();
        assert_eq!(snapshot.seq, 2);
        assert_eq!(
            snapshot.ops,
            [
                create(6),
                create(7),
                Op::Insert {
                    parent: BODY,
                    id: 6,
                    before: None
                },
                Op::Text {
                    id: 6,
                    value: "current label".into()
                },
            ]
        );
    }
}
