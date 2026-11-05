//! Media workers publish only against the exact document and asset inputs they
//! consumed. Tree/table clones are cheap; no project hashing runs on the UI.
use pm_kernel::{Published, project_io::MediaTable};
use pm_model::{Node, NodeKind};
use std::sync::Arc;

#[derive(Clone)]
pub struct Inputs {
    document: u64,
    assets: Option<Node>,
    media: MediaTable,
}
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
    fn same(&self, other: &Self) -> bool {
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
}
pub(crate) struct Resolved {
    pub inputs: Inputs,
    pub catalog: pm_engine_service::MediaCatalog,
}
/// Run only on a worker, carrying the exact inputs into initial adoption too.
pub(crate) fn resolve(published: &Published, cache: &std::path::Path) -> anyhow::Result<Resolved> {
    let inputs = Inputs::published(published);
    let catalog = crate::media::for_session(&published.snapshot, &published.media, published.path.as_deref(), cache)?;
    Ok(Resolved { inputs, catalog })
}
#[derive(Default)]
pub struct Refresh {
    serial: u64,
    visible: Option<Inputs>,
    settled: Option<Inputs>,
    pending: Option<(u64, Inputs)>,
}
pub struct Plan {
    /// Retire old-by-id pixels/readiness before exposing a new asset selection.
    pub clear: bool,
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
        let clear = self.visible.as_ref().is_none_or(|old| !old.same(&inputs));
        self.visible = Some(inputs.clone());
        if clear { self.settled = None; }
        if self.pending.is_some() || self.settled.as_ref().is_some_and(|old| old.same(&inputs)) {
            return Plan { clear, job: None };
        }
        self.serial += 1;
        self.pending = Some((self.serial, inputs));
        Plan { clear, job: Some(self.serial) }
    }
    /// A failed current worker is settled offline until inputs change or an
    /// explicit retry resets us. Stale results replan even without another event.
    pub fn finish(&mut self, serial: u64, latest: Option<Inputs>) -> bool {
        if self.pending.as_ref().is_none_or(|(pending, _)| *pending != serial) {
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

#[cfg(test)]
mod tests {
    use super::*;
    use pm_kernel::project_io::{MediaEntry, MediaSource};
    fn inputs(document: u64, asset: &str) -> Inputs {
        Inputs {
            document,
            assets: Some(Node::parse_bytes(format!(r#"{{"a":{{"originalVersion":"{asset}"}}}}"#).as_bytes(), 1).unwrap()),
            media: MediaTable::default(),
        }
    }
    #[test]
    fn rapid_relink_undo_and_reopen_never_publish_a_stale_catalog() {
        let a = inputs(1, "a");
        let b = inputs(1, "b");
        let mut refresh = Refresh::default();
        refresh.adopt(a.clone());
        assert!(!refresh.plan(a.clone()).clear);
        let job = refresh.plan(b.clone());
        assert!(job.clear);
        let serial = job.job.unwrap();
        assert!(refresh.plan(a.clone()).clear, "Undo retires replacement pixels immediately");
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
            assert!(!plan.clear);
            assert!(plan.job.is_none());
        }
        let mut changed = a.clone();
        changed.media.insert(MediaEntry {
            id: "a".into(), media_type: "image/png".into(), revision: "new".into(),
            source: MediaSource::Memory(Arc::from(&b"synthetic"[..])),
        });
        let plan = refresh.plan(changed.clone());
        assert!(plan.clear);
        assert!(refresh.finish(plan.job.unwrap(), Some(changed.clone())));
        assert!(refresh.plan(changed).job.is_none());
    }
}

use gpui::{prelude::*, Context};
use crate::session::Controller;
impl Controller {
    fn replace_media_catalog(&mut self, catalog: pm_engine_service::MediaCatalog) {
        self.media = catalog;
        if let Some(viewer) = &self.viewer
            && let Err(error) = viewer.media(self.media.clone())
        { self.message = format!("Viewer media: {error}"); }
        if let Some(popout) = &self.popout
            && let Some(viewer) = &*popout.viewer.borrow()
            && let Err(error) = viewer.media(self.media.clone())
        { self.message = format!("Popout media: {error}"); }
        self.builtins.media_runtime = Default::default();
    }
    pub(crate) fn refresh_media(&mut self, cx: &mut Context<Self>) {
        let Some(session) = self.session.as_ref() else { return; };
        let Some(published) = self.kernel.published(&session.project) else { return; };
        if published.session.as_ref().map(|s| &s.handle) != Some(session) { return; }
        let plan = self.media_refresh.plan(Inputs::published(&published));
        if plan.clear { self.replace_media_catalog(Default::default()); }
        let Some(job) = plan.job else { return; };
        let cache = self.profile.join("media-cache").join(pm_kernel::new_id("resolved"));
        cx.spawn(async move |this, cx| {
            let result = cx.background_executor().spawn(async move {
                crate::media::for_session(&published.snapshot, &published.media, published.path.as_deref(), &cache)
            }).await;
            let _ = this.update(cx, |v, cx| {
                let latest = v.session.as_ref().and_then(|session| {
                    v.kernel.published(&session.project).filter(|p| p.session.as_ref().map(|s| &s.handle) == Some(session))
                });
                if v.media_refresh.finish(job, latest.as_deref().map(Inputs::published)) {
                    match result {
                        Ok(catalog) => v.replace_media_catalog(catalog),
                        Err(error) => v.message = format!("Media could not be resolved: {error}"),
                    }
                    v.update_builtins(cx);
                    cx.notify();
                }
                v.refresh_media(cx);
            });
        }).detach();
    }
}
