//! v1.2.0 property-reveal.ts:7–107. Clock and schema facts are injected.
use crate::{Channel, ModelError, TimelineModel};
use std::collections::{HashMap, HashSet};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Disclosure {
    pub collapsed: bool,
    pub group_collapsed: bool,
    /// None = animated channels; Some(["*"]) = all.
    pub paths: Option<Vec<String>>,
}
#[derive(Clone, Debug, Default)]
pub struct PropertyFacts {
    /// Schema-derived facts absent from the canonical document.
    pub modified: HashSet<(String, String)>,
    pub material: HashSet<(String, String)>,
    pub paint: HashSet<(String, String)>,
    pub missing_effect: HashSet<(String, String)>,
    pub effect_channels: HashSet<(String, String)>,
}
#[derive(Clone, Debug)]
struct Last {
    key: char,
    shift: bool,
    millis: u64,
    ids: Vec<String>,
    before: Vec<Disclosure>,
}
#[derive(Clone, Debug, Default)]
pub struct RevealState {
    pub layers: HashMap<String, Disclosure>,
    last: Option<Last>,
}
impl RevealState {
    pub fn visible(&self, layer: &str, channel: &Channel) -> bool {
        self.layers
            .get(layer)
            .and_then(|d| d.paths.as_ref())
            .map_or(!channel.keys.is_empty(), |paths| {
                paths.iter().any(|p| p == "*" || p == &channel.path)
            })
    }
}
impl TimelineModel {
    pub fn reveal_property(
        &mut self,
        key: char,
        shift: bool,
        millis: u64,
        facts: &PropertyFacts,
    ) -> Result<(), ModelError> {
        if !"psrtaumfel*".contains(key) {
            return Err(ModelError::UnknownCommand(key.to_string()));
        }
        let layers: Vec<_> = self
            .index
            .layers
            .iter()
            .filter(|l| self.selection.layers().is_empty() || self.selection.contains(&l.id))
            .collect();
        if layers.is_empty() {
            return Ok(());
        }
        let ids: Vec<_> = layers.iter().map(|l| l.id.clone()).collect();
        let before: Vec<_> = layers
            .iter()
            .map(|l| {
                self.reveal
                    .layers
                    .get(&l.id)
                    .cloned()
                    .unwrap_or(Disclosure {
                        collapsed: l.collapsed,
                        group_collapsed: l.group_collapsed,
                        paths: None,
                    })
            })
            .collect();
        if key == '*' || (key == 'm' && self.selection.layers().is_empty()) {
            let close = before.iter().all(|d| !d.collapsed);
            for (l, mut d) in layers.iter().zip(before) {
                d.paths = Some(vec!["*".into()]);
                d.collapsed = close;
                if key == 'm' && l.kind == "group" {
                    d.group_collapsed = close;
                }
                self.reveal.layers.insert(l.id.clone(), d);
            }
            self.reveal.last = None;
        } else {
            let double = self.reveal.last.as_ref().is_some_and(|last| {
                last.key == key
                    && last.shift == shift
                    && millis >= last.millis
                    && millis - last.millis < 300
                    && last.ids == ids
            });
            let base = if double {
                self.reveal.last.as_ref().unwrap().before.clone()
            } else {
                before.clone()
            };
            let targets: Vec<Vec<String>> = layers
                .iter()
                .map(|l| {
                    l.channels
                        .iter()
                        .filter(|c| {
                            let p = c.path.as_str();
                            let pair = (l.id.clone(), c.path.clone());
                            match (key, double) {
                                ('p', false) => p.starts_with("position."),
                                ('p', true) => facts.paint.contains(&pair),
                                ('s', false) => p.starts_with("scale."),
                                ('s', true) => {
                                    self.selection.channel() == Some(p)
                                        || (self.selection.channel() == Some("scale")
                                            && p.starts_with("scale."))
                                        || c.keys
                                            .iter()
                                            .any(|k| self.selection.keys().contains(&k.id))
                                }
                                ('r', false) => {
                                    p == "rotation"
                                        || p.starts_with("rotation.")
                                        || p == "orientation"
                                        || p.starts_with("orientation.")
                                }
                                ('r', true) => p == "c.timeRemap" || p == "c.sourceTime",
                                ('a', false) => p.starts_with("anchor."),
                                ('a', true) => facts.material.contains(&pair),
                                ('t', false) => p == "opacity",
                                ('t', true) => p.starts_with("m.") && p.ends_with(".opacity"),
                                ('u', _) => {
                                    !c.keys.is_empty()
                                        || c.expression.is_some()
                                        || (double && facts.modified.contains(&pair))
                                }
                                ('m', true) => p.starts_with("m."),
                                ('m', false) => {
                                    p.starts_with("m.")
                                        && [".shape", ".x", ".y", ".w", ".h", ".rotation"]
                                            .iter()
                                            .any(|s| p.ends_with(s))
                                }
                                ('f', true) => facts.missing_effect.contains(&pair),
                                ('f', false) => p.starts_with("m.") && p.ends_with(".feather"),
                                ('e', true) => c.expression.is_some(),
                                ('e', false) => facts.effect_channels.contains(&pair),
                                ('l', false) => p == "c.gain",
                                _ => false,
                            }
                        })
                        .map(|c| c.path.clone())
                        .collect()
                })
                .collect();
            let close = !shift
                && targets.iter().zip(&base).all(|(t, d)| {
                    !d.collapsed && same_paths(t, d.paths.as_deref().unwrap_or_default())
                });
            for ((l, mut d), target) in layers.iter().zip(base).zip(targets) {
                let paths = if shift {
                    let current: Vec<_> = if d.collapsed {
                        Vec::new()
                    } else if d.paths.as_ref().is_some_and(|p| p.iter().any(|p| p == "*")) {
                        l.channels.iter().map(|c| c.path.clone()).collect()
                    } else {
                        d.paths.clone().unwrap_or_else(|| {
                            l.channels
                                .iter()
                                .filter(|c| !c.keys.is_empty())
                                .map(|c| c.path.clone())
                                .collect()
                        })
                    };
                    if !target.is_empty() && target.iter().all(|p| current.contains(p)) {
                        current
                            .into_iter()
                            .filter(|p| !target.contains(p))
                            .collect()
                    } else {
                        let mut next = current;
                        for p in target {
                            if !next.contains(&p) {
                                next.push(p);
                            }
                        }
                        next
                    }
                } else {
                    target
                };
                d.collapsed = close || (shift && paths.is_empty());
                d.paths = Some(paths);
                self.reveal.layers.insert(l.id.clone(), d);
                let mut parent = l.group.clone();
                let mut seen = HashSet::new();
                while let Some(id) = parent {
                    if !seen.insert(id.clone()) {
                        break;
                    }
                    self.reveal
                        .layers
                        .entry(id.clone())
                        .or_default()
                        .group_collapsed = false;
                    parent = self.index.layer(&id).and_then(|l| l.group.clone());
                }
            }
            self.reveal.last = if double {
                None
            } else {
                Some(Last {
                    key,
                    shift,
                    millis,
                    ids,
                    before,
                })
            };
        }
        self.rebuild_rows();
        Ok(())
    }
}
fn same_paths(a: &[String], b: &[String]) -> bool {
    let a: HashSet<_> = a.iter().collect();
    let b: HashSet<_> = b.iter().collect();
    a == b
}
