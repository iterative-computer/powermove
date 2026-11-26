//! Composition pass facts kept across property edits. These values read
//! group, matteSource, solo and the layer addresses resolved by group
//! reuse. Scalar equality also handles copy-on-write layer lists.

use crate::js;
use pm_eval::{JsValue, node};
use pm_model::{JsString, Node};
use std::{collections::HashMap, sync::Arc};

pub(crate) struct PassFacts {
    pub(crate) owners: pm_eval::groups::LayerOwners,
    pub(crate) matte_sources: MatteSources,
    pub(crate) solo: bool,
    pub(crate) has_3d: bool,
    group_spans: Option<HashMap<JsString, Span>>,
}

impl PassFacts {
    pub(crate) fn of(all: &[&Node]) -> Self {
        Self {
            owners: pm_eval::groups::LayerOwners::of(all),
            group_spans: group_spans(all),
            matte_sources: MatteSources::of(all),
            solo: all.iter().any(|l| js::flag(l, "solo")),
            has_3d: all
                .iter()
                .any(|l| js::flag(l, "threeD") || pm_eval::parenting::Role3d::of(l).is_some()),
        }
    }

    pub(crate) fn has_group_span(&self, layer: &Node) -> bool {
        node::string(layer.get("id")).is_some_and(|id| {
            self.group_spans
                .as_ref()
                .is_some_and(|spans| spans.contains_key(id))
        })
    }

    pub(crate) fn group_outside(&self, layer: &Node, time: f64) -> bool {
        let Some(id) = node::string(layer.get("id")) else {
            return false;
        };
        self.group_spans
            .as_ref()
            .and_then(|spans| spans.get(id))
            .is_some_and(|s| time < s.from - 1e-6 || time > s.end + 2e-6)
    }
}

#[derive(Clone, Copy, PartialEq)]
struct Span {
    from: f64,
    end: f64,
}

impl Span {
    const EMPTY: Self = Self {
        from: f64::INFINITY,
        end: f64::NEG_INFINITY,
    };
    const ANY: Self = Self {
        from: f64::NEG_INFINITY,
        end: f64::INFINITY,
    };

    fn union(self, other: Self) -> Self {
        Self {
            from: self.from.min(other.from),
            end: self.end.max(other.end),
        }
    }
}

fn can_draw_empty(layer: &Node) -> bool {
    let has_stages = ["fx", "masks"].into_iter().any(|key| {
        layer.get(key).is_some_and(|stages| {
            !matches!(stages.kind(), pm_model::NodeKind::Null)
                && node::array(Some(stages)).is_none_or(|stages| !stages.is_empty())
        })
    });
    let non_normal_blend = layer.get("blend").is_some_and(|blend| {
        !matches!(blend.kind(), pm_model::NodeKind::Null)
            && !node::is_str(Some(blend), "normal")
    });
    has_stages
        || non_normal_blend
        || ["transitionIn", "transitionOut", "matteSource"]
            .into_iter()
            .any(|k| node::truthy(layer.get(k)))
}

fn paired(layer: &Node) -> bool {
    ["transitionIn", "transitionOut"]
        .into_iter()
        .any(|k| layer.get(k).and_then(|t| t.get("paired")).is_some())
}

fn span(layer: &Node) -> Span {
    if node::is_str(layer.get("type"), "group") {
        return if can_draw_empty(layer) {
            Span::ANY
        } else {
            Span::EMPTY
        };
    }
    match (
        node::number(layer.get("from")),
        node::number(layer.get("dur")),
    ) {
        (Some(from), Some(dur)) if from.is_finite() && dur.is_finite() && !paired(layer) => {
            if dur > 0.0 {
                Span {
                    from,
                    end: from + dur,
                }
            } else {
                Span::EMPTY
            }
        }
        _ => Span::ANY,
    }
}

/// A group's saved timing does not constrain its children. A cheap bound
/// over their timing can reject an empty pass, including nested groups.
fn group_spans(layers: &[&Node]) -> Option<HashMap<JsString, Span>> {
    let mut groups = HashMap::new();
    let mut spans = HashMap::new();
    let mut ids = std::collections::HashSet::new();
    for layer in layers {
        let id = node::string(layer.get("id"))?;
        if !ids.insert(id) {
            return None;
        }
        if node::is_str(layer.get("type"), "group") {
            groups.insert(id, node::string(layer.get("group")));
            spans.insert(id.clone(), Span::EMPTY);
        }
    }
    for layer in layers {
        let extent = span(layer);
        if extent == Span::EMPTY {
            continue;
        }
        let mut owner = if node::is_str(layer.get("type"), "group") {
            node::string(layer.get("id"))
        } else {
            node::string(layer.get("group"))
        };
        while let Some(id) = owner {
            let Some(current) = spans.get_mut(id) else {
                break;
            };
            let widened = current.union(extent);
            // The ancestors have already received any unchanged extent.
            // This also terminates a malformed group cycle.
            if *current == widened {
                break;
            }
            *current = widened;
            owner = groups[id];
        }
    }
    Some(spans)
}

// A group cache also depends on stack order, stable ids and parent
// resolution, including a missing parent that could become available.
const FIELDS: [&str; 7] = [
    "group",
    "matteSource",
    "solo",
    "id",
    "parent",
    "type",
    "threeD",
];

fn inputs(layer: &Node) -> impl Iterator<Item = Option<&Node>> {
    FIELDS
        .into_iter()
        .map(move |key| layer.get(key))
        .chain([layer.get("d").and_then(|d| d.get("definition"))])
}

fn same_input(a: Option<&Node>, b: &Option<Node>) -> bool {
    match (a.map(Node::kind), b.as_ref().map(Node::kind)) {
        (Some(pm_model::NodeKind::Object(a)), Some(pm_model::NodeKind::Object(b))) => {
            Arc::ptr_eq(a, b)
        }
        (Some(pm_model::NodeKind::Array(a)), Some(pm_model::NodeKind::Array(b))) => {
            Arc::ptr_eq(a, b)
        }
        (a, b) => a == b,
    }
}
const MAX_ENTRIES: usize = 128;

struct Entry {
    comp: (JsString, bool),
    revision: u64,
    layers: Option<Node>,
    members: Vec<([Option<Node>; FIELDS.len() + 1], Span)>,
    facts: Arc<PassFacts>,
}

#[derive(Default)]
pub(crate) struct FactsCache(Vec<Entry>);

impl FactsCache {
    pub(crate) fn clear(&mut self) {
        self.0.clear();
    }

    pub(crate) fn get(
        &mut self,
        comp: &pm_eval::Comp<'_>,
        all: &[&Node],
        revision: u64,
    ) -> Arc<PassFacts> {
        let key = (comp.id().clone(), comp.at_root());
        let layers = comp.layers_node();
        let at = self.0.iter().position(|e| e.comp == key);
        if let Some(at) = at {
            let mut entry = self.0.remove(at);
            let same_layers = matches!(
                (layers.map(Node::kind), entry.layers.as_ref().map(Node::kind)),
                (Some(pm_model::NodeKind::Array(a)), Some(pm_model::NodeKind::Array(b)))
                    if Arc::ptr_eq(a, b)
            );
            let valid = entry.revision <= revision
                && (same_layers
                    || (entry.members.len() == all.len()
                        && all.iter().zip(&entry.members).all(
                            |(&layer, (old, previous_span))| {
                                span(layer) == *previous_span
                                    && inputs(layer)
                                        .zip(old)
                                        .all(|(current, previous)| same_input(current, previous))
                            },
                        )));
            if valid {
                entry.revision = revision;
                entry.layers = layers.cloned();
                let facts = entry.facts.clone();
                self.0.push(entry);
                return facts;
            }
        }
        let facts = Arc::new(PassFacts::of(all));
        if self.0.len() == MAX_ENTRIES {
            self.0.remove(0);
        }
        self.0.push(Entry {
            comp: key,
            revision,
            layers: layers.cloned(),
            members: all
                .iter()
                .map(|l| {
                    let mut inputs = inputs(l);
                    (
                        std::array::from_fn(|_| inputs.next().flatten().cloned()),
                        span(l),
                    )
                })
                .collect(),
            facts: facts.clone(),
        });
        facts
    }
}

pub(crate) struct MatteSources {
    undefined: bool,
    strings: std::collections::BTreeSet<JsString>,
    /// Numbers, booleans, `null`, and objects: rare, compared one by one.
    others: Vec<JsValue>,
}

impl MatteSources {
    pub(crate) fn of(layers: &[&Node]) -> MatteSources {
        let mut set = MatteSources {
            undefined: false,
            strings: std::collections::BTreeSet::new(),
            others: Vec::new(),
        };
        for l in layers {
            match node::get(l, b"matteSource").to_js() {
                JsValue::Undefined => set.undefined = true,
                JsValue::String(s) => {
                    set.strings.insert(s);
                }
                other => set.others.push(other),
            }
        }
        set
    }

    pub(crate) fn contains(&self, id: &JsValue) -> bool {
        match id {
            JsValue::Undefined => self.undefined,
            JsValue::String(s) => self.strings.contains(s),
            _ => self.others.iter().any(|m| same_value_zero(m, id)),
        }
    }
}

fn same_value_zero(a: &JsValue, b: &JsValue) -> bool {
    match (a, b) {
        (JsValue::Number(x), JsValue::Number(y)) => x == y || (x.is_nan() && y.is_nan()),
        _ => pm_eval::js_sem::strict_eq(a, b),
    }
}
