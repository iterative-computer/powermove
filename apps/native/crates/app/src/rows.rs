//! Layer rows for the temporary panels. Listing every layer is O(layers), so
//! large compositions build their rows on a worker; the main thread only
//! swaps in the finished `Arc`. Panels render only visible rows.
use pm_kernel::{LayerInfo, document::Snapshot};
use std::sync::Arc;

/// Compositions up to this size list their rows inline (a few ms at most),
/// so small projects keep deterministic, same-turn rows.
pub const INLINE_LAYERS: usize = 2_000;

/// (document session, revision): revisions only grow within one session.
pub type Key = (u64, u64);

pub fn key(snapshot: &Snapshot) -> Key {
    (snapshot.doc_id, snapshot.revision)
}

pub fn layer_count(snapshot: &Snapshot) -> usize {
    snapshot
        .root()
        .and_then(|root| {
            let open = pm_kernel::model::open_comp_id(root);
            pm_kernel::model::comp_node(root, open.as_deref())
        })
        .map_or(0, |comp| pm_kernel::model::layers_of(comp).count())
}

pub fn list(snapshot: &Snapshot) -> Vec<LayerInfo> {
    snapshot
        .root()
        .map(|root| pm_kernel::model::list_layers(root, None))
        .unwrap_or_default()
}

#[derive(Default)]
pub struct LayerRows {
    pub rows: Arc<Vec<LayerInfo>>,
    /// The publication `rows` describe.
    pub key: Option<Key>,
    /// A worker computation in flight (at most one).
    pending: Option<Key>,
}

pub enum Plan {
    /// Rows already describe this publication, or a worker is busy; when it
    /// finishes the caller re-plans against the newest publication.
    Current,
    Inline,
    Worker,
}

impl LayerRows {
    pub fn clear(&mut self) {
        *self = Self::default();
    }
    pub fn plan(&mut self, latest: Key, count: usize) -> Plan {
        if self.key == Some(latest) || self.pending.is_some() {
            return Plan::Current;
        }
        if count <= INLINE_LAYERS {
            return Plan::Inline;
        }
        self.pending = Some(latest);
        Plan::Worker
    }
    /// Accepts rows unless they belong to another document session or are
    /// older than the rows already shown. Returns whether they were applied.
    pub fn finish(&mut self, key: Key, rows: Vec<LayerInfo>, latest: Option<Key>) -> bool {
        if self.pending == Some(key) {
            self.pending = None;
        }
        self.accept(key, rows, latest)
    }
    pub fn accept(&mut self, key: Key, rows: Vec<LayerInfo>, latest: Option<Key>) -> bool {
        let same_session = latest.is_some_and(|l| l.0 == key.0);
        let newer = self.key.is_none_or(|k| k.0 != key.0 || k.1 <= key.1);
        if same_session && newer {
            self.rows = Arc::new(rows);
            self.key = Some(key);
            true
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn row(id: &str) -> LayerInfo {
        LayerInfo {
            id: id.into(),
            name: id.into(),
            kind: "shape".into(),
            parent: None,
            group: None,
            in_time: 0.,
            out_time: 1.,
            locked: false,
            effectively_locked: false,
            visible: true,
            solo: false,
            shy: false,
            index: 0,
        }
    }
    #[test]
    fn small_compositions_list_inline_and_large_ones_use_one_worker() {
        let mut rows = LayerRows::default();
        assert!(matches!(rows.plan((1, 1), 10), Plan::Inline));
        assert!(rows.accept((1, 1), vec![row("a")], Some((1, 1))));
        assert!(matches!(rows.plan((1, 1), 10), Plan::Current));
        assert!(matches!(rows.plan((1, 2), 60_001), Plan::Worker));
        // A second publication while the worker runs waits for re-planning.
        assert!(matches!(rows.plan((1, 3), 60_001), Plan::Current));
        assert!(rows.finish((1, 2), vec![row("b")], Some((1, 3))));
        assert!(matches!(rows.plan((1, 3), 60_001), Plan::Worker));
        assert!(rows.finish((1, 3), vec![row("c")], Some((1, 3))));
        assert_eq!(rows.rows[0].id, "c");
        assert!(matches!(rows.plan((1, 3), 60_001), Plan::Current));
    }
    #[test]
    fn stale_or_foreign_rows_never_replace_newer_rows() {
        let mut rows = LayerRows::default();
        assert!(rows.accept((1, 5), vec![row("new")], Some((1, 5))));
        assert!(!rows.accept((1, 4), vec![row("old")], Some((1, 5))));
        // Rows computed for a closed document session are dropped.
        assert!(!rows.finish((1, 6), vec![row("closed")], Some((2, 1))));
        assert_eq!(rows.rows[0].id, "new");
        // A new session may start at a lower revision.
        assert!(rows.accept((2, 1), vec![row("other")], Some((2, 1))));
        assert_eq!(rows.rows[0].id, "other");
    }
}
