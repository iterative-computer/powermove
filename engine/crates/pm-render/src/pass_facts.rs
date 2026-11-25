//! Composition pass facts kept across property edits. These values read
//! group, matteSource, solo and the layer addresses resolved by group
//! reuse. Scalar equality also handles copy-on-write layer lists.

use crate::js;
use pm_eval::{JsValue, node};
use pm_model::{JsString, Node};
use std::sync::Arc;

pub(crate) struct PassFacts {
    pub(crate) owners: pm_eval::groups::LayerOwners,
    pub(crate) matte_sources: MatteSources,
    pub(crate) solo: bool,
}

impl PassFacts {
    pub(crate) fn of(all: &[&Node]) -> Self {
        Self {
            owners: pm_eval::groups::LayerOwners::of(all),
            matte_sources: MatteSources::of(all),
            solo: all.iter().any(|l| js::flag(l, "solo")),
        }
    }
}

// A group cache also depends on stack order, stable ids and parent
// resolution, including a missing parent that could become available.
const FIELDS: [&str; 6] = ["group", "matteSource", "solo", "id", "parent", "type"];
const MAX_ENTRIES: usize = 128;

struct Entry {
    comp: (JsString, bool),
    revision: u64,
    members: Vec<[Option<Node>; FIELDS.len()]>,
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
        let at = self.0.iter().position(|e| e.comp == key);
        if let Some(at) = at {
            let mut entry = self.0.remove(at);
            let valid = entry.revision <= revision
                && entry.members.len() == all.len()
                && all.iter().zip(&entry.members).all(|(&layer, old)| {
                    FIELDS.iter().zip(old).all(|(key, previous)| {
                        match (node::member(Some(layer), key), previous) {
                            (None, None) => true,
                            (Some(n), Some(old)) => match n.kind() {
                                pm_model::NodeKind::Object(_) | pm_model::NodeKind::Array(_) => {
                                    false
                                }
                                _ => n.kind() == old.kind(),
                            },
                            _ => false,
                        }
                    })
                });
            if valid {
                entry.revision = revision;
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
            members: all
                .iter()
                .map(|l| FIELDS.map(|k| node::member(Some(l), k).cloned()))
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
