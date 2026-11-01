//! Ordered, asynchronous resolution. A pending decision has no deadline.
use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, HashMap, VecDeque};
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Binding {
    pub id: String,
    pub chord: String,
    pub command: String,
    pub args: Vec<serde_json::Value>,
    pub priority: i32,
    pub in_fields: bool,
    pub repeat: bool,
    pub loose: bool,
}
pub fn shipped() -> Vec<Binding> {
    serde_json::from_str(include_str!("../data/keymap.json")).expect("frozen W1 table")
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Class {
    NeverDeclines,
    MayDecline,
}
pub fn class(command: &str) -> Class {
    if matches!(
        command,
        "settings"
            | "palette"
            | "toggleSidebar"
            | "toggleInspector"
            | "fieldMenu"
            | "newWindow"
            | "quit"
    ) {
        Class::NeverDeclines
    } else {
        Class::MayDecline
    }
}
#[derive(Clone, Debug, PartialEq)]
pub struct Key {
    pub token: u64,
    pub chord: String,
    pub field: bool,
    pub repeat: bool,
    pub up: bool,
    pub compat: bool,
}
#[derive(Clone, Debug, PartialEq)]
pub enum Effect {
    Dispatch {
        token: u64,
        command: String,
        args: Vec<serde_json::Value>,
        class: Class,
    },
    Replay(u64),
    Consumed(u64),
}
#[derive(Default)]
pub struct Resolver {
    pub bindings: Vec<Binding>,
    queue: VecDeque<Key>,
    pending: Option<(Key, Vec<Binding>, usize)>,
    bypassed: HashMap<u64, Box<Resolver>>,
    bypass_lane: bool,
    waiting_replay: Option<u64>,
}
fn parts(chord: &str) -> (BTreeSet<&str>, &str) {
    // A literal plus is represented by a trailing + (cmd+shift++).
    let split = if chord.ends_with("++") {
        chord.len() - 1
    } else {
        chord.rfind('+').map_or(0, |i| i + 1)
    };
    let key = &chord[split..];
    let mods = chord[..split]
        .split('+')
        .filter(|m| !m.is_empty())
        .collect();
    (mods, key)
}
fn matches(binding: &Binding, key: &Key) -> bool {
    if key.field && !binding.in_fields || key.repeat && !binding.repeat {
        return false;
    }
    if binding.chord == key.chord {
        return true;
    }
    if !binding.loose {
        return false;
    }
    let (bm, bk) = parts(&binding.chord);
    let (km, kk) = parts(&key.chord);
    let semantic = |s: BTreeSet<&str>| {
        s.into_iter()
            .map(|m| {
                if m == "cmd" || m == "ctrl" {
                    "primary"
                } else {
                    m
                }
            })
            .map(str::to_string)
            .collect::<BTreeSet<_>>()
    };
    let (bm, km) = (semantic(bm), semantic(km));
    bk == kk && bm.is_subset(&km)
}
impl Resolver {
    pub fn new() -> Self {
        Self {
            bindings: shipped(),
            ..Default::default()
        }
    }
    pub fn submit(&mut self, key: Key) -> Vec<Effect> {
        if key.compat {
            return vec![Effect::Replay(key.token)];
        }
        if matches!(key.chord.as_str(), "escape" | "cmd+." | "cmd+q")
            && !key.up
            && !self.bypass_lane
        {
            if key.chord == "cmd+q" {
                return vec![Effect::Dispatch {
                    token: key.token,
                    command: "quit".into(),
                    args: vec![],
                    class: Class::NeverDeclines,
                }];
            }
            let mut lane = Self {
                bindings: self.bindings.clone(),
                bypass_lane: true,
                ..Default::default()
            };
            if key.chord == "cmd+." {
                lane.bindings.push(Binding {
                    id: "shell.cancel".into(),
                    chord: "cmd+.".into(),
                    command: "cancel".into(),
                    args: vec![],
                    priority: i32::MAX,
                    in_fields: true,
                    repeat: false,
                    loose: false,
                });
            }
            let token = key.token;
            let effects = lane.submit(key);
            if lane.pending_token().is_some() || lane.waiting_replay.is_some() {
                self.bypassed.insert(token, Box::new(lane));
            }
            return effects;
        }
        self.queue.push_back(key);
        self.drain()
    }
    fn drain(&mut self) -> Vec<Effect> {
        let mut out = vec![];
        while self.pending.is_none() && self.waiting_replay.is_none() {
            let Some(key) = self.queue.pop_front() else {
                break;
            };
            if key.up {
                self.waiting_replay = Some(key.token);
                out.push(Effect::Replay(key.token));
                break;
            }
            let mut bindings: Vec<_> = self
                .bindings
                .iter()
                .enumerate()
                .filter(|(_, b)| matches(b, &key))
                .map(|(i, b)| (i, b.clone()))
                .collect();
            bindings.sort_by_key(|(i, b)| {
                (
                    b.priority,
                    b.chord != key.chord,
                    std::cmp::Reverse(parts(&b.chord).0.len()),
                    *i,
                )
            });
            let candidates = bindings.into_iter().map(|(_, b)| b).collect();
            self.pending = Some((key, candidates, 0));
            out.extend(self.advance());
        }
        out
    }
    fn advance(&mut self) -> Vec<Effect> {
        let Some((key, bindings, index)) = self.pending.as_mut() else {
            return vec![];
        };
        if let Some(binding) = bindings.get(*index) {
            let class = class(&binding.command);
            let effect = Effect::Dispatch {
                token: key.token,
                command: binding.command.clone(),
                args: binding.args.clone(),
                class,
            };
            if class == Class::NeverDeclines {
                self.pending = None;
            }
            vec![effect]
        } else {
            let token = key.token;
            self.pending = None;
            self.waiting_replay = Some(token);
            vec![Effect::Replay(token)]
        }
    }
    pub fn resolve(&mut self, token: u64, handled: bool) -> Vec<Effect> {
        if let Some(mut lane) = self.bypassed.remove(&token) {
            let effects = lane.resolve(token, handled);
            if lane.pending_token().is_some() || lane.waiting_replay.is_some() {
                self.bypassed.insert(token, lane);
            }
            return effects;
        }
        let Some((key, _, index)) = self.pending.as_mut() else {
            return vec![];
        };
        if key.token != token {
            return vec![];
        }
        let mut out = vec![];
        if handled {
            self.pending = None;
            out.push(Effect::Consumed(token));
        } else {
            *index += 1;
            out.extend(self.advance());
        }
        out.extend(self.drain());
        out
    }
    pub fn replayed(&mut self, token: u64) -> Vec<Effect> {
        if let Some(mut lane) = self.bypassed.remove(&token) {
            let effects = lane.replayed(token);
            if lane.pending.is_some() || lane.waiting_replay.is_some() {
                self.bypassed.insert(token, lane);
            }
            return effects;
        }
        if self.waiting_replay == Some(token) {
            self.waiting_replay = None;
            self.drain()
        } else {
            vec![]
        }
    }
    pub fn pending_token(&self) -> Option<u64> {
        self.pending.as_ref().map(|p| p.0.token)
    }
}

/// US physical fallback only for the shifted keys named by W1; ordinary letters
/// keep their layout/IME characters. Native function key codes are layout independent.
pub fn physical_fallback(key: String, code: u16, shift: bool) -> String {
    if let Some(f) = match code {
        122 => Some(1),
        120 => Some(2),
        99 => Some(3),
        118 => Some(4),
        96 => Some(5),
        97 => Some(6),
        98 => Some(7),
        100 => Some(8),
        101 => Some(9),
        109 => Some(10),
        103 => Some(11),
        111 => Some(12),
        _ => None,
    } {
        return format!("f{f}");
    }
    if shift {
        let fallback = match code {
            18 => Some("1"),
            19 => Some("2"),
            20 => Some("3"),
            21 => Some("4"),
            23 => Some("5"),
            22 => Some("6"),
            26 => Some("7"),
            28 => Some("8"),
            25 => Some("9"),
            29 => Some("0"),
            33 => Some("["),
            30 => Some("]"),
            44 => Some("/"),
            _ => None,
        };
        if let Some(k) = fallback {
            return k.into();
        }
    }
    key
}
