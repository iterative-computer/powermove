//! MCP-owned hold policy. The kernel never owns or polls this queue.
//! Time is supplied by the host so an idle queue schedules no work.
use serde_json::Value;
use std::collections::{HashMap, HashSet, VecDeque};

pub const RELEASE_DELAY_MS: u64 = 250;
pub const TOOL_WAIT_MS: u64 = 4_000;
// Approved shortcuts. Keep independent of the existing shell cancel/grid keys.
pub const STOP: &str = "cmd+alt+.";
pub const PAUSE: &str = "cmd+shift+.";
pub const STEER: &str = "cmd+shift+'";
pub const UNDO_TURN: &str = "cmd+alt+z";

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Path {
    pub layer: String,
    pub property: String,
}
#[derive(Clone, Debug, PartialEq)]
pub struct Write {
    pub path: Path,
    pub value: Value,
}
#[derive(Clone, Debug, PartialEq)]
pub enum Resolution {
    Applied { ticket: u64, writes: Vec<Write> },
    Dropped { ticket: u64, reason: String },
}
#[derive(Clone, Debug, PartialEq)]
pub enum Admission {
    Apply(Vec<Write>),
    Held { ticket: u64 },
    UserOwned { path: Path, value: Value },
}
struct Held {
    ticket: u64,
    writes: Vec<Write>,
}
#[derive(Default)]
pub struct HoldQueue {
    gestures: HashSet<u64>,
    fields: HashSet<Path>,
    owned: HashMap<Path, Value>,
    held: VecDeque<Held>,
    released_at: Option<u64>,
    next_ticket: u64,
}
impl HoldQueue {
    pub fn begin_gesture(&mut self, lease: u64) {
        self.gestures.insert(lease);
        self.released_at = None;
    }
    pub fn end_gesture(&mut self, lease: u64, now: u64) {
        self.gestures.remove(&lease);
        self.release_if_clear(now);
    }
    pub fn focus(&mut self, path: Path) {
        self.fields.insert(path);
        self.released_at = None;
    }
    pub fn blur(&mut self, path: &Path, now: u64) {
        self.fields.remove(path);
        self.release_if_clear(now);
    }
    /// Call only for committed Interface changes, never for agent revisions.
    pub fn user_changed(&mut self, path: Path, value: Value) {
        self.owned.insert(path, value);
    }
    /// An explicit steer can give a path back to this turn.
    pub fn allow_steered(&mut self, path: &Path) {
        self.owned.remove(path);
    }
    pub fn admit(&mut self, writes: Vec<Write>) -> Admission {
        if let Some((write, value)) = writes
            .iter()
            .find_map(|w| self.owned.get(&w.path).map(|v| (w, v)))
        {
            return Admission::UserOwned {
                path: write.path.clone(),
                value: value.clone(),
            };
        }
        if !self.gestures.is_empty()
            || !self.held.is_empty()
            || writes.iter().any(|w| self.fields.contains(&w.path))
        {
            self.next_ticket += 1;
            let ticket = self.next_ticket;
            self.held.push_back(Held { ticket, writes });
            Admission::Held { ticket }
        } else {
            Admission::Apply(writes)
        }
    }
    fn release_if_clear(&mut self, now: u64) {
        if self.gestures.is_empty() && self.fields.is_empty() {
            self.released_at = Some(now);
        }
    }
    /// Only the host's release event calls this, after the 250 ms grace period.
    /// A held transaction is atomic; if one write loses, drop the transaction.
    pub fn resolve(&mut self, now: u64) -> Vec<Resolution> {
        if !self.gestures.is_empty()
            || !self.fields.is_empty()
            || self
                .released_at
                .is_none_or(|t| now.saturating_sub(t) < RELEASE_DELAY_MS)
        {
            return Vec::new();
        }
        self.held
            .drain(..)
            .map(|h| {
                if let Some(w) = h.writes.iter().find(|w| self.owned.contains_key(&w.path)) {
                    Resolution::Dropped {
                        ticket: h.ticket,
                        reason: format!(
                            "The user changed {} · {} while this transaction waited.",
                            w.path.layer, w.path.property
                        ),
                    }
                } else {
                    Resolution::Applied {
                        ticket: h.ticket,
                        writes: h.writes,
                    }
                }
            })
            .collect()
    }
    pub fn target_removed(&mut self, layer: &str) -> Vec<Resolution> {
        let mut dropped = Vec::new();
        self.held.retain(|h| {
            if h.writes.iter().any(|w| w.path.layer == layer) {
                dropped.push(Resolution::Dropped {
                    ticket: h.ticket,
                    reason: format!("target_removed: {layer}"),
                });
                false
            } else {
                true
            }
        });
        dropped
    }
    pub fn pending(&self) -> usize {
        self.held.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn path(name: &str) -> Path {
        Path {
            layer: "L".into(),
            property: name.into(),
        }
    }
    fn write(name: &str) -> Write {
        Write {
            path: path(name),
            value: json!(140),
        }
    }
    #[test]
    fn every_transaction_waits_during_a_gesture_and_lands_in_order() {
        let mut q = HoldQueue::default();
        q.begin_gesture(1);
        assert_eq!(q.admit(vec![write("scale")]), Admission::Held { ticket: 1 });
        assert_eq!(
            q.admit(vec![write("rotation")]),
            Admission::Held { ticket: 2 }
        );
        q.end_gesture(1, 100);
        assert!(q.resolve(349).is_empty());
        let resolved = q.resolve(350);
        assert!(matches!(
            resolved.as_slice(),
            [
                Resolution::Applied { ticket: 1, .. },
                Resolution::Applied { ticket: 2, .. }
            ]
        ));
        assert_eq!(q.pending(), 0);
    }
    #[test]
    fn user_wins_and_owns_the_path_for_the_rest_of_the_turn() {
        let mut q = HoldQueue::default();
        q.focus(path("scale"));
        q.admit(vec![write("scale")]);
        q.user_changed(path("scale"), json!(118));
        q.blur(&path("scale"), 0);
        assert!(matches!(
            q.resolve(250).as_slice(),
            [Resolution::Dropped { ticket: 1, .. }]
        ));
        assert!(
            matches!(q.admit(vec![write("scale")]),Admission::UserOwned{value,..} if value==json!(118))
        );
        q.allow_steered(&path("scale"));
        assert!(matches!(q.admit(vec![write("scale")]), Admission::Apply(_)));
    }
    #[test]
    fn nested_gestures_and_typing_never_release_early() {
        let mut q = HoldQueue::default();
        q.begin_gesture(1);
        q.begin_gesture(2);
        q.focus(path("x"));
        q.admit(vec![write("x")]);
        q.end_gesture(1, 0);
        q.end_gesture(2, 0);
        assert!(q.resolve(500).is_empty());
        q.blur(&path("x"), 500);
        assert_eq!(q.resolve(750).len(), 1);
    }
    #[test]
    fn deletion_drops_only_queued_transactions_for_that_target() {
        let mut q = HoldQueue::default();
        q.begin_gesture(1);
        q.admit(vec![write("x")]);
        assert!(
            matches!(q.target_removed("L").as_slice(),[Resolution::Dropped{ticket:1,reason}] if reason.contains("target_removed"))
        );
        assert_eq!(q.pending(), 0);
    }
    #[test]
    fn idle_queue_has_no_release_work() {
        let mut q = HoldQueue::default();
        for now in [0, 12_000, 15_000, 25_000] {
            assert!(q.resolve(now).is_empty());
        }
        assert_eq!(q.pending(), 0);
    }
}

// Focus is transient host state. A property text editor registers while its
// uncommitted text owns the value; no document revision or UI wake is emitted.
fn fields() -> &'static std::sync::Mutex<HashSet<(String, String)>> {
    static FIELDS: std::sync::OnceLock<std::sync::Mutex<HashSet<(String, String)>>> =
        std::sync::OnceLock::new();
    FIELDS.get_or_init(Default::default)
}
pub fn touch_field(project: &str, field: &str, active: bool) {
    let mut fields = fields().lock().unwrap();
    let key = (project.into(), field.into());
    if active {
        fields.insert(key);
    } else {
        fields.remove(&key);
    }
}
pub fn typing(project: &str) -> bool {
    fields().lock().unwrap().iter().any(|(id, _)| id == project)
}
