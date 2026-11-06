//! Media workers publish only against the exact document and asset inputs they
//! consumed. Tree/table clones are cheap; no project hashing runs on the UI.
use crate::session::Controller;
use gpui::Context;
use pm_kernel::{Published, project_io::MediaTable};
use pm_model::{Node, NodeKind};
use std::{collections::BTreeSet, sync::Arc};

#[derive(Clone)]
pub struct Inputs {
    document: u64,
    assets: Option<Node>,
    media: MediaTable,
}
/// Asset members that select bytes. A rename or folder move changes neither.
const STORAGE_FIELDS: [&str; 4] = ["originalVersion", "sourcePath", "path", "playbackProxy"];
impl Inputs {
    pub fn published(p: &Published) -> Self {
        Self {
            document: p.snapshot.doc_id,
            assets: p.snapshot.root().and_then(|r| r.get("assets")).cloned(),
            media: p.media.clone(),
        }
    }
    pub(crate) fn matches(&self, published: &Published) -> bool {
        self.same(&Self::published(published))
    }
    pub(crate) fn same(&self, other: &Self) -> bool {
        fn same_node(a: Option<&Node>, b: Option<&Node>) -> bool {
            match (a.map(Node::kind), b.map(Node::kind)) {
                (Some(NodeKind::Object(a)), Some(NodeKind::Object(b))) => Arc::ptr_eq(a, b),
                (Some(NodeKind::Array(a)), Some(NodeKind::Array(b))) => Arc::ptr_eq(a, b),
                (a, b) => a == b,
            }
        }
        self.document == other.document
            && self.media.shares_entries_with(&other.media)
            && same_node(self.assets.as_ref(), other.assets.as_ref())
    }
    /// Asset ids whose resolved bytes may differ from `old`. `None`: every id,
    /// because the document itself changed. Saving moves media to new file
    /// ranges without changing any id's content, so it retires nothing.
    fn changed_ids(&self, old: &Self) -> Option<BTreeSet<String>> {
        use crate::media::content_identity;
        use std::collections::HashMap;
        if self.document != old.document {
            return None;
        }
        let identities = |table: &'_ MediaTable| {
            table
                .iter()
                .map(|item| {
                    (
                        item.id.to_string(),
                        (
                            item.media_type.to_string(),
                            content_identity(item).0.map(str::to_owned),
                            content_identity(item).1,
                            content_identity(item).2,
                        ),
                    )
                })
                .collect::<HashMap<_, _>>()
        };
        let (before, after) = (identities(&old.media), identities(&self.media));
        let mut changed: BTreeSet<_> = before
            .iter()
            .filter(|(id, identity)| after.get(*id) != Some(*identity))
            .map(|(id, _)| id.clone())
            .collect();
        changed.extend(after.keys().filter(|id| !before.contains_key(*id)).cloned());
        let storage = |assets: Option<&Node>, id: &str| {
            let asset = assets.and_then(|a| a.get(id));
            STORAGE_FIELDS.map(|field| asset.and_then(|a| a.get(field)).map(Node::to_value))
        };
        let ids = |assets: Option<&Node>| {
            assets
                .and_then(|a| match a.kind() {
                    NodeKind::Object(map) => Some(map.keys().map(|k| k.to_string()).collect()),
                    _ => None,
                })
                .unwrap_or_else(BTreeSet::new)
        };
        let (old_assets, new_assets) = (old.assets.as_ref(), self.assets.as_ref());
        for id in ids(old_assets).union(&ids(new_assets)) {
            if storage(old_assets, id) != storage(new_assets, id) {
                changed.insert(id.clone());
            }
        }
        Some(changed)
    }
}
pub(crate) struct Resolved {
    pub inputs: Inputs,
    pub resolution: crate::media::Resolution,
}
/// Run only on a worker, carrying the exact inputs into initial adoption too.
pub(crate) fn resolve(published: &Published, cache: &std::path::Path) -> anyhow::Result<Resolved> {
    let inputs = Inputs::published(published);
    let resolution = crate::media::for_session(
        &published.snapshot,
        &published.media,
        published.path.as_deref(),
        cache,
    )?;
    Ok(Resolved { inputs, resolution })
}
#[derive(Default)]
pub struct Refresh {
    serial: u64,
    visible: Option<Inputs>,
    settled: Option<Inputs>,
    pending: Option<(u64, Inputs)>,
}
pub struct Plan {
    /// Retire these ids' pixels and readiness before exposing a new selection.
    /// `None` retires everything. Other ids keep showing through the refresh.
    pub retire: Option<BTreeSet<String>>,
    pub job: Option<u64>,
}
impl Refresh {
    pub fn reset(&mut self) {
        self.serial += 1;
        self.visible = None;
        self.settled = None;
        self.pending = None;
    }
    /// Initial open already resolved these exact inputs on its open worker.
    pub fn adopt(&mut self, inputs: Inputs) {
        self.reset();
        self.visible = Some(inputs.clone());
        self.settled = Some(inputs);
    }
    pub fn plan(&mut self, inputs: Inputs) -> Plan {
        let retire = match &self.visible {
            None => None,
            Some(old) if old.same(&inputs) => Some(BTreeSet::new()),
            Some(old) => inputs.changed_ids(old),
        };
        let changed = self.visible.as_ref().is_none_or(|old| !old.same(&inputs));
        self.visible = Some(inputs.clone());
        if changed {
            self.settled = None;
        }
        if self.pending.is_some() || self.settled.as_ref().is_some_and(|old| old.same(&inputs)) {
            return Plan { retire, job: None };
        }
        self.serial += 1;
        self.pending = Some((self.serial, inputs));
        Plan {
            retire,
            job: Some(self.serial),
        }
    }
    /// A failed current worker is settled offline until inputs change or an
    /// explicit retry resets us. Stale results replan even without another event.
    pub fn finish(&mut self, serial: u64, latest: Option<Inputs>) -> bool {
        if self
            .pending
            .as_ref()
            .is_none_or(|(pending, _)| *pending != serial)
        {
            return false;
        }
        let (_, inputs) = self.pending.take().unwrap();
        if latest.as_ref().is_none_or(|latest| !inputs.same(latest)) {
            return false;
        }
        self.settled = Some(inputs);
        true
    }
}

impl Controller {
    fn publish_media_catalog(&mut self) {
        if let Some(viewer) = &self.viewer
            && let Err(error) = viewer.media(self.media.clone())
        {
            self.message = format!("Viewer media: {error}");
        }
        if let Some(popout) = &self.popout
            && let Some(viewer) = &*popout.viewer.borrow()
            && let Err(error) = viewer.media(self.media.clone())
        {
            self.message = format!("Popout media: {error}");
        }
    }
    fn retire_media(&mut self, ids: Option<BTreeSet<String>>) {
        match ids {
            None => self.media = Default::default(),
            Some(ids) if ids.is_empty() => return,
            Some(ids) => {
                for id in &ids {
                    self.media.originals.remove(id);
                    self.media.proxies.remove(id);
                }
            }
        }
        self.publish_media_catalog();
    }
    pub(crate) fn refresh_media(&mut self, cx: &mut Context<Self>) {
        let Some(session) = self.session.as_ref() else {
            return;
        };
        let Some(published) = self.kernel.published(&session.project) else {
            return;
        };
        if published.session.as_ref().map(|s| &s.handle) != Some(session) {
            return;
        }
        let plan = self.media_refresh.plan(Inputs::published(&published));
        self.retire_media(plan.retire);
        let Some(job) = plan.job else {
            return;
        };
        let cache = crate::media::cache_root(&self.profile);
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { resolve(&published, &cache) })
                .await;
            let _ = this.update(cx, |v, cx| {
                let latest = v.session.as_ref().and_then(|session| {
                    v.kernel
                        .published(&session.project)
                        .filter(|p| p.session.as_ref().map(|s| &s.handle) == Some(session))
                });
                if v.media_refresh
                    .finish(job, latest.as_deref().map(Inputs::published))
                {
                    match result {
                        Ok(resolved) => v.adopt_media(resolved.resolution),
                        Err(error) => v.message = format!("Media could not be resolved: {error}"),
                    }
                    v.update_builtins(cx);
                    cx.notify();
                }
                v.refresh_media(cx);
            });
        })
        .detach();
    }
    fn adopt_media(&mut self, resolution: crate::media::Resolution) {
        if !resolution.failed.is_empty() {
            self.message = crate::media::failure_note(resolution.failed.len());
        }
        if resolution.catalog == self.media {
            return;
        }
        self.media = resolution.catalog;
        self.publish_media_catalog();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pm_kernel::project_io::{MediaEntry, MediaSource};
    fn inputs(document: u64, asset: &str) -> Inputs {
        Inputs {
            document,
            assets: Some(
                Node::parse_bytes(
                    format!(r#"{{"a":{{"originalVersion":"{asset}"}}}}"#).as_bytes(),
                    1,
                )
                .unwrap(),
            ),
            media: MediaTable::default(),
        }
    }
    #[test]
    fn rapid_relink_undo_and_reopen_never_publish_a_stale_catalog() {
        let a = inputs(1, "a");
        let b = inputs(1, "b");
        let mut refresh = Refresh::default();
        refresh.adopt(a.clone());
        assert_eq!(refresh.plan(a.clone()).retire, Some(BTreeSet::new()));
        let job = refresh.plan(b.clone());
        assert_eq!(job.retire, Some(BTreeSet::from(["a".to_owned()])));
        let serial = job.job.unwrap();
        assert_eq!(
            refresh.plan(a.clone()).retire,
            Some(BTreeSet::from(["a".to_owned()])),
            "Undo retires replacement pixels immediately"
        );
        assert!(!refresh.finish(serial, Some(a.clone())));
        let serial = refresh.plan(a.clone()).job.unwrap();
        let reopened = inputs(2, "a");
        refresh.adopt(reopened.clone());
        assert!(!refresh.finish(serial, Some(reopened.clone())));
        assert!(refresh.plan(reopened).job.is_none());
    }
    #[test]
    fn same_assets_and_storage_do_not_recopy_on_selection_or_composition_edits() {
        let a = inputs(1, "a");
        let mut refresh = Refresh::default();
        refresh.adopt(a.clone());
        for _ in 0..20 {
            let plan = refresh.plan(a.clone());
            assert_eq!(plan.retire, Some(BTreeSet::new()));
            assert!(plan.job.is_none());
        }
        let mut changed = a.clone();
        changed.media.insert(MediaEntry {
            id: "a".into(),
            media_type: "image/png".into(),
            revision: "new".into(),
            source: MediaSource::Memory(Arc::from(&b"synthetic"[..])),
        });
        let plan = refresh.plan(changed.clone());
        assert_eq!(plan.retire, Some(BTreeSet::from(["a".to_owned()])));
        assert!(refresh.finish(plan.job.unwrap(), Some(changed.clone())));
        assert!(refresh.plan(changed).job.is_none());
    }
    #[test]
    fn a_rename_and_a_save_keep_other_media_on_screen() {
        let entry = |file_range: u64| MediaEntry {
            id: "a".into(),
            media_type: "image/png".into(),
            revision: "r".into(),
            source: MediaSource::File {
                path: format!("/synthetic/{file_range}.png").into(),
                length: 4,
                sha256: Some("0".repeat(64)),
            },
        };
        let mut before = inputs(1, "a");
        before.media.insert(entry(1));
        let mut after = before.clone();
        after.media = MediaTable::default();
        after.media.insert(entry(2));
        after.assets = Some(
            Node::parse_bytes(br#"{"a":{"originalVersion":"a","name":"Renamed"}}"#, 1).unwrap(),
        );
        let mut refresh = Refresh::default();
        refresh.adopt(before);
        let plan = refresh.plan(after);
        assert_eq!(plan.retire, Some(BTreeSet::new()));
        assert!(plan.job.is_some(), "paths are still re-resolved");
    }
}
