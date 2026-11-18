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
        let ids = |assets: Option<&Node>| -> BTreeSet<String> {
            assets
                .and_then(|a| match a.kind() {
                    NodeKind::Object(map) => Some(map.keys().map(|k| k.to_string()).collect()),
                    _ => None,
                })
                .unwrap_or_default()
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
/// Run only on a worker.
fn resolve(
    published: &Published,
    cache: &std::path::Path,
) -> anyhow::Result<crate::media::Resolution> {
    crate::media::for_session(
        &published.snapshot,
        &published.media,
        published.path.as_deref(),
        cache,
    )
}
#[derive(Default)]
pub struct Refresh {
    serial: u64,
    visible: Option<Inputs>,
    settled: Option<Inputs>,
    resolved: bool,
    observation_epoch: u64,
    pending: Option<(u64, Inputs)>,
    retry_pending: bool,
}
pub struct Plan {
    /// Source inputs that changed; `None` means a different document.
    /// Same-document paths keep showing until the replacement resolves.
    pub retire: Option<BTreeSet<String>>,
    pub job: Option<u64>,
}
impl Refresh {
    pub fn reset(&mut self) {
        self.serial += 1;
        self.visible = None;
        self.settled = None;
        self.resolved = false;
        self.pending = None;
        self.retry_pending = false;
    }
    /// A file became available without changing the document's inputs.
    /// Keep the display paths and coalesce retries behind any current read.
    pub(crate) fn retry(&mut self) {
        self.settled = None;
        self.resolved = false;
        self.retry_pending = self.pending.is_some();
    }
    /// Settled at `inputs` (what a finished job leaves).
    #[cfg(test)]
    pub fn adopt(&mut self, inputs: Inputs) {
        self.reset();
        self.visible = Some(inputs.clone());
        self.settled = Some(inputs);
        self.catalog_adopted();
    }
    pub(crate) fn catalog_ready(&self, inputs: &Inputs) -> bool {
        self.resolved && self.settled.as_ref().is_some_and(|old| old.same(inputs))
    }
    pub(crate) fn observation_epoch(&self) -> u64 {
        self.observation_epoch
    }
    fn catalog_adopted(&mut self) {
        self.resolved = self.settled.is_some();
        if self.resolved {
            // A cloud restore can change readable bytes at the same path.
            // Only observations made after this adoption can be reused.
            self.observation_epoch += 1;
        }
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
            self.resolved = false;
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
        self.resolved = false;
        if std::mem::take(&mut self.retry_pending) {
            return false;
        }
        if latest.as_ref().is_none_or(|latest| !inputs.same(latest)) {
            return false;
        }
        self.settled = Some(inputs);
        true
    }
}

/// Only a document switch clears displayed source paths immediately. A
/// same-document replacement is published atomically by adopt_media.
fn retire_catalog(
    catalog: &mut pm_engine_service::MediaCatalog,
    ids: Option<BTreeSet<String>>,
) -> bool {
    if ids.is_some() {
        return false;
    }
    *catalog = Default::default();
    true
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
    fn retire_media(&mut self, ids: Option<BTreeSet<String>>, cx: &mut Context<Self>) {
        if !retire_catalog(&mut self.media_base, ids) {
            return;
        }
        self.decorate_media(cx);
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
        self.retire_media(plan.retire, cx);
        let Some(job) = plan.job else {
            return;
        };
        // Until the job lands, a newly imported clip with no known file
        // draws nothing rather than Media Offline: the
        // card is wrong for it, and drawing it held the viewer seconds after
        // an import.
        let resolving: BTreeSet<String> = published
            .media
            .iter()
            .map(|item| item.id.to_string())
            .filter(|id| !self.media_base.originals.contains_key(id))
            .collect();
        if resolving != self.media_base.resolving {
            self.media_base.resolving = resolving;
            self.decorate_media(cx);
            self.publish_media_catalog();
        }
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
                        Ok(resolution) => v.adopt_media(resolution, cx),
                        Err(error) => {
                            v.message = format!("Media could not be resolved: {error}");
                            // Nothing is being looked up any more: what has
                            // no file is offline.
                            v.media_base.resolving.clear();
                            v.decorate_media(cx);
                            v.publish_media_catalog();
                        }
                    }
                    v.update_builtins(cx);
                    cx.notify();
                }
                v.refresh_media(cx);
            });
        })
        .detach();
    }
    pub(crate) fn adopt_media(
        &mut self,
        resolution: crate::media::Resolution,
        cx: &mut Context<Self>,
    ) {
        if !resolution.failed.is_empty() {
            self.message = crate::media::failure_note(resolution.failed.len());
        }
        if !resolution.cloud.is_empty() {
            self.offer_cloud_media(resolution.cloud, cx);
        }
        self.media_base = resolution.catalog;
        self.media_resolved = true;
        let before = std::mem::take(&mut self.media);
        self.decorate_media(cx);
        self.media_refresh.catalog_adopted();
        self.identify_media();
        if self.media != before {
            self.publish_media_catalog();
        }
        let imported = self
            .pro_media
            .take_imported(|id| self.media_base.originals.contains_key(id));
        if !imported.is_empty() {
            self.proxies_on_import(&imported);
        }
    }
    /// The catalog the viewer reads: the resolver's, with the open
    /// project's proxies, optimized media and Always Play Original.
    pub(crate) fn decorate_media(&mut self, cx: &mut Context<Self>) {
        let mut media = self.media_base.clone();
        if let Some(project) = self.session.as_ref().map(|s| s.project.clone()) {
            self.pro_media
                .decorate(&project, &mut media, &self.project_assets());
        }
        self.media = media;
        self.publish_offline_media(cx);
    }
    /// The assets the viewer cannot read in its media mode, for the timeline's
    /// Media Offline clips (the same catalog the engine draws the card from).
    pub(crate) fn offline_media(&self) -> BTreeSet<String> {
        if !self.media_resolved {
            return BTreeSet::new();
        }
        let readable = self.media.viewer_paths(self.media_mode);
        self.project_assets()
            .into_iter()
            .filter(|id| !readable.contains_key(id))
            .collect()
    }
    pub(crate) fn publish_offline_media(&self, cx: &mut Context<Self>) {
        if let Some(timeline) = self.builtins.timeline.clone() {
            let offline = self.offline_media();
            let sources = self.media.originals.clone();
            timeline.update(cx, |t, cx| {
                t.set_offline_media(offline, cx);
                t.set_audio_sources(sources, cx);
            });
        }
    }
    /// The pro media worker moved: renditions change the catalog; every
    /// change redraws the Project panel's status and the tasks window.
    pub(crate) fn pro_media_changed(&mut self, renditions: bool, cx: &mut Context<Self>) {
        if let (Some(session), Some(p)) = (&self.session, &self.published) {
            self.pro_media
                .prioritize(&session.project, &p.snapshot, self.time());
        }
        if renditions {
            let before = self.media.clone();
            self.decorate_media(cx);
            if self.media != before {
                self.publish_media_catalog();
            }
        }
        self.update_builtins(cx);
        crate::session::sync_render_storage(&self.pro_media, &self.engine);
        if let Some(tasks) = &self.tasks_window {
            tasks.update(cx, |_, cx| cx.notify());
        }
        cx.notify();
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
    fn background_source_refresh_keeps_visible_paths_until_adoption() {
        let mut catalog = pm_engine_service::MediaCatalog::default();
        catalog
            .originals
            .insert("a".into(), "/synthetic/original.png".into());
        catalog
            .proxies
            .insert("a".into(), "/synthetic/proxy.png".into());
        let before = catalog.clone();
        assert!(!retire_catalog(
            &mut catalog,
            Some(BTreeSet::from(["a".into()]))
        ));
        assert!(
            catalog == before,
            "background refresh removed displayed pixels"
        );
        assert!(retire_catalog(&mut catalog, None));
        assert!(
            catalog == Default::default(),
            "document switches forget old sources"
        );
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
    fn trimming_a_precomp_does_not_restart_media_resolution() {
        use pm_kernel::{DocumentStore, Origin, Snapshot, TimingCommand};
        let project = pm_model::json::parse(
            r#"{
            "id":"P", "compId":"C", "compName":"Main", "w":64, "h":64, "fps":30, "dur":10,
            "layers":[{"id":"pre", "type":"precomp", "name":"Nested", "on":true,
                "from":0, "dur":10, "d":{"comp":"N", "w":64, "h":64}}],
            "comps":{"N":{"id":"N", "name":"Nested", "w":64, "h":64,
                "fps":30, "dur":10, "layers":[]}},
            "assets":{"a":{"id":"a", "name":"Fixture", "kind":"image", "originalVersion":"v1"}}
        }"#,
        )
        .unwrap();
        let mut store = DocumentStore::new_untitled(project);
        let media = MediaTable::default();
        let make_inputs = |snapshot: &Snapshot| Inputs {
            document: snapshot.doc_id,
            assets: snapshot.root().and_then(|root| root.get("assets")).cloned(),
            media: media.clone(),
        };
        let original = make_inputs(&store.snapshot());
        let mut refresh = Refresh::default();
        refresh.adopt(original.clone());
        let gesture = store.begin_gesture("Trim", Origin::Interface).unwrap();
        for time in [0.2, 0.8, 0.4, 0.] {
            store
                .update_gesture_from_start(
                    gesture,
                    &[TimingCommand::TrimIn {
                        layer_ids: vec!["pre".into()],
                        time,
                    }
                    .into()],
                )
                .unwrap();
            let next = make_inputs(&store.snapshot());
            assert!(original.same(&next), "a trim changes no media inputs");
            let plan = refresh.plan(next);
            assert_eq!(plan.retire, Some(BTreeSet::new()));
            assert!(
                plan.job.is_none(),
                "a trim must not schedule media resolution"
            );
        }
        store.commit_gesture(gesture).unwrap();
        assert!(refresh.plan(make_inputs(&store.snapshot())).job.is_none());
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
    fn probe_inputs(document: u64, asset: &str) -> crate::builtins::MediaProbeInputs {
        (
            inputs(document, asset),
            Default::default(),
            Default::default(),
            0,
        )
    }
    fn started(plan: crate::builtins::MediaProbePlan) -> u64 {
        match plan {
            crate::builtins::MediaProbePlan::Start(serial) => serial,
            other => panic!("expected a new probe, got {other:?}"),
        }
    }
    #[test]
    fn probe_import_bursts_run_one_read_then_only_the_latest() {
        use crate::builtins::{MediaProbePlan, MediaProbeRefresh};
        let mut probe = MediaProbeRefresh::default();
        let first = probe_inputs(1, "first");
        let serial = started(probe.plan(first.clone()));
        assert_eq!(
            probe.plan(first.clone()),
            MediaProbePlan::Waiting,
            "queued work is not an observation that can be rebound"
        );
        for revision in 0..50 {
            assert_eq!(
                probe.plan(probe_inputs(1, &format!("import-{revision}"))),
                MediaProbePlan::Waiting,
                "an import must not add another worker"
            );
        }
        let latest = probe_inputs(1, "last");
        assert!(
            !probe.finish(serial, Some(&latest)),
            "stale observations must not publish"
        );
        let serial = started(probe.plan(latest.clone()));
        assert_eq!(probe.plan(latest.clone()), MediaProbePlan::Waiting);
        assert!(probe.finish(serial, Some(&latest)));
        assert_eq!(probe.plan(latest), MediaProbePlan::Reuse);
    }
    #[test]
    fn probe_path_changes_wait_and_undo_reuses_only_settled_inputs() {
        use crate::builtins::{MediaProbePlan, MediaProbeRefresh};
        let mut probe = MediaProbeRefresh::default();
        let mut original = probe_inputs(1, "a");
        original
            .1
            .insert("a".into(), "/synthetic/original.png".into());
        let serial = started(probe.plan(original.clone()));
        assert!(probe.finish(serial, Some(&original)));
        let mut changed = original.clone();
        changed.2.insert("a".into(), "/synthetic/proxy.png".into());
        let serial = started(probe.plan(changed.clone()));
        assert_eq!(probe.plan(changed.clone()), MediaProbePlan::Waiting);
        // Undo can reuse what was actually observed, even while the other
        // read finishes. That other read must never rebind its own result.
        assert_eq!(probe.plan(original.clone()), MediaProbePlan::Reuse);
        assert!(!probe.finish(serial, Some(&original)));
        assert_eq!(probe.plan(original), MediaProbePlan::Reuse);
    }
    #[test]
    fn a_probe_from_a_retired_host_cannot_finish_the_new_hosts_read() {
        use crate::builtins::{MediaProbePlan, MediaProbeRefresh};
        let mut old = MediaProbeRefresh::default();
        let stale = started(old.plan(probe_inputs(1, "old")));
        let mut next = MediaProbeRefresh::default();
        let current = probe_inputs(2, "new");
        let serial = started(next.plan(current.clone()));
        assert_ne!(serial, stale);
        assert!(!next.finish(stale, Some(&current)));
        assert_eq!(next.plan(current.clone()), MediaProbePlan::Waiting);
        assert!(next.finish(serial, Some(&current)));
        assert_eq!(next.plan(current), MediaProbePlan::Reuse);
    }
    #[test]
    fn a_restored_file_at_the_same_path_requires_a_fresh_observation() {
        use crate::builtins::{MediaProbePlan, MediaProbeRefresh};
        let mut probe = MediaProbeRefresh::default();
        let original = probe_inputs(1, "a");
        let serial = started(probe.plan(original.clone()));
        assert!(probe.finish(serial, Some(&original)));
        let mut restored = original.clone();
        restored.3 += 1;
        let serial = started(probe.plan(restored.clone()));
        let mut newer = restored.clone();
        newer.3 += 1;
        assert_eq!(probe.plan(newer.clone()), MediaProbePlan::Waiting);
        assert!(!probe.finish(serial, Some(&newer)));
        let serial = started(probe.plan(newer.clone()));
        assert!(probe.finish(serial, Some(&newer)));
        assert_eq!(probe.plan(newer), MediaProbePlan::Reuse);
    }

    #[test]
    fn probes_wait_for_an_adopted_catalog_and_never_probe_failed_resolution() {
        let original = inputs(1, "original");
        let changed = inputs(1, "changed");
        let failed = inputs(1, "failed");
        let mut refresh = Refresh::default();
        refresh.adopt(original.clone());
        let epoch = refresh.observation_epoch();
        assert!(refresh.catalog_ready(&original));
        let job = refresh.plan(changed.clone()).job.unwrap();
        assert!(
            !refresh.catalog_ready(&changed),
            "held display paths are not new readiness evidence"
        );
        assert!(refresh.finish(job, Some(changed.clone())));
        assert!(
            !refresh.catalog_ready(&changed),
            "completion is not catalog adoption"
        );
        refresh.catalog_adopted();
        assert!(refresh.observation_epoch() > epoch);
        assert!(refresh.catalog_ready(&changed));
        assert!(!refresh.catalog_ready(&inputs(2, "changed")));
        let job = refresh.plan(failed.clone()).job.unwrap();
        assert!(refresh.finish(job, Some(failed.clone())));
        // An error never calls catalog_adopted. It stays settled without
        // launching probes against the retained display-only paths.
        assert!(!refresh.catalog_ready(&failed));
        assert!(refresh.plan(failed.clone()).job.is_none());
        assert!(!refresh.catalog_ready(&failed));
        assert!(!refresh.catalog_ready(&inputs(2, "failed")));
    }
    #[test]
    fn cloud_retries_keep_display_paths_and_coalesce_behind_the_current_read() {
        let current = inputs(1, "download");
        let mut refresh = Refresh::default();
        refresh.adopt(current.clone());
        refresh.retry();
        let plan = refresh.plan(current.clone());
        assert_eq!(plan.retire, Some(BTreeSet::new()));
        let job = plan.job.unwrap();
        for _ in 0..50 {
            refresh.retry();
            let plan = refresh.plan(current.clone());
            assert_eq!(plan.retire, Some(BTreeSet::new()));
            assert!(
                plan.job.is_none(),
                "downloads must not add parallel resolvers"
            );
        }
        assert!(
            !refresh.finish(job, Some(current.clone())),
            "the read predates a downloaded file"
        );
        let job = refresh.plan(current.clone()).job.unwrap();
        assert!(refresh.finish(job, Some(current.clone())));
        refresh.catalog_adopted();
        assert!(refresh.catalog_ready(&current));
        assert!(refresh.plan(current.clone()).job.is_none());

        // A cloud retry finishing after a document switch cannot publish.
        refresh.retry();
        let job = refresh.plan(current).job.unwrap();
        let other = inputs(2, "other");
        assert_eq!(refresh.plan(other.clone()).retire, None);
        assert!(!refresh.finish(job, Some(other.clone())));
        assert!(refresh.plan(other).job.is_some());
    }
    #[test]
    fn retry_and_reset_require_observations_after_the_new_catalog_adoption() {
        let current = inputs(1, "same-path");
        let mut refresh = Refresh::default();
        refresh.adopt(current.clone());
        let first = refresh.observation_epoch();
        refresh.retry();
        let job = refresh.plan(current.clone()).job.unwrap();
        assert!(refresh.finish(job, Some(current.clone())));
        refresh.catalog_adopted();
        let restored = refresh.observation_epoch();
        assert!(restored > first, "same paths can hold newly restored bytes");
        refresh.reset();
        let job = refresh.plan(current.clone()).job.unwrap();
        assert!(refresh.finish(job, Some(current)));
        refresh.catalog_adopted();
        assert!(
            refresh.observation_epoch() > restored,
            "a reset cannot reuse an old observation identity"
        );
    }
}
