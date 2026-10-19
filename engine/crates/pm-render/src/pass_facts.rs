//! Composition pass facts kept across property edits. These values read
//! only group, matteSource, and solo; a layer drag changes none of them.
//! Node revisions plus member addresses prove the same values and order
//! without rebuilding LayerOwners or allocating a matte set each frame.

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

const FIELDS: [&str; 3] = ["group", "matteSource", "solo"];
const MAX_ENTRIES: usize = 128;

struct Entry {
    layers: usize,
    revision: u64,
    members: Vec<[usize; 3]>,
    facts: Arc<PassFacts>,
}

#[derive(Default)]
pub(crate) struct FactsCache(Vec<Entry>);

fn members(layer: &Node) -> [usize; 3] {
    FIELDS.map(|key| node::member(Some(layer), key).map_or(0, |v| v as *const Node as usize))
}

impl FactsCache {
    pub(crate) fn clear(&mut self) {
        self.0.clear();
    }

    pub(crate) fn get(&mut self, layers: &[Node], all: &[&Node], revision: u64) -> Arc<PassFacts> {
        let address = layers.as_ptr() as usize;
        let at = self.0.iter().position(|e| e.layers == address);
        if let Some(at) = at {
            let mut entry = self.0.remove(at);
            let valid = entry.revision <= revision
                && entry.members.len() == layers.len()
                && layers.iter().zip(&entry.members).all(|(layer, old)| {
                    FIELDS.iter().zip(old).all(|(key, address)| {
                        let value = node::member(Some(layer), key);
                        value.map_or(0, |v| v as *const Node as usize) == *address
                            && value.is_none_or(|v| v.rev() <= entry.revision)
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
            layers: address,
            revision,
            members: layers.iter().map(members).collect(),
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
