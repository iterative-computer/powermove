//! P2 resource snapshot seam. Pins belong to the viewer/export request, not
//! mutable global registries. Unsupported requirements fail visibly.
use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ResourceSnapshot {
    pub generation: u64,
    pub extension_generations: std::collections::BTreeMap<String, u64>,
    pub project_fonts: Vec<String>,
    /// Effects, transitions and layers contributed by extensions, frozen with
    /// this generation so a frame or export job never sees a later registration.
    #[serde(default)]
    pub definitions: Vec<pm_protocol::definitions::DefinitionSnapshot>,
    /// Extensions are still loading: an effect `definitions` lacks may yet be
    /// registered. A frame that would bypass it is not drawn, cached or
    /// exported until a settled snapshot arrives.
    #[serde(default)]
    pub pending: bool,
    /// While `pending`: the extensions still loading, as (id, name), so a
    /// frame that waits for one says which.
    #[serde(default)]
    pub loading: Vec<(String, String)>,
    /// Installed extensions that provide nothing now, with why: they are
    /// named beside the effects they would provide. Names; pins nothing.
    #[serde(default)]
    pub unavailable: Vec<Unavailable>,
}
/// An installed extension that provides nothing now (turned off, stopped,
/// failed to start), named beside an effect it would provide. `why` reads
/// after its name: "is turned off".
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Unavailable {
    pub id: String,
    pub name: String,
    pub why: String,
}
/// Whether an effect's id is its extension's: `<extension>` or
/// `<extension>.<name>`.
pub fn provides(extension: &str, effect: &str) -> bool {
    effect
        .strip_prefix(extension)
        .is_some_and(|rest| rest.is_empty() || rest.starts_with('.'))
}
/// The extension an effect's id names (`<extension>.<name>`), when it names one.
pub fn named_extension(effect: &str) -> Option<&str> {
    effect
        .split_once('.')
        .map(|(id, _)| id)
        .filter(|id| !id.is_empty())
}
impl ResourceSnapshot {
    /// The installed extension that would provide `effect`, if it is one that
    /// provides nothing now.
    pub fn unavailable_owner(&self, effect: &str) -> Option<&Unavailable> {
        self.unavailable.iter().find(|u| provides(&u.id, effect))
    }
    pub fn validate(&self) -> anyhow::Result<()> {
        // Definitions arrive whole in `definitions`; a pinned extension
        // generation names content nothing here can load.
        anyhow::ensure!(
            self.extension_generations.is_empty() && self.project_fonts.is_empty(),
            "extension/font resource activation requires the P2 snapshot host"
        );
        let mut seen = std::collections::BTreeSet::new();
        for d in &self.definitions {
            anyhow::ensure!(
                seen.insert((d.kind, d.id.clone())),
                "extension definition {} is listed twice",
                d.id.as_str()
            );
        }
        Ok(())
    }
    /// The same pins, whichever viewer holds them: switching between viewers
    /// on one engine must not reinstall definitions or drop cached frames.
    /// Which extensions are still loading names a wait; it pins nothing.
    pub fn same_pins(a: &std::sync::Arc<Self>, b: &std::sync::Arc<Self>) -> bool {
        std::sync::Arc::ptr_eq(a, b)
            || (a.pending == b.pending
                && a.definitions == b.definitions
                && a.extension_generations == b.extension_generations
                && a.project_fonts == b.project_fonts)
    }
}
/// One window's resources as they change, for every viewer of that window
/// (the editor's, a popped-out one). A viewer attached here draws the latest
/// snapshot and every one published after it: none keeps the snapshot it
/// happened to start with, so a viewer opened while extensions load does not
/// wait for them forever, and none draws an effect's old code after a reload.
#[derive(Clone, Default)]
pub struct ResourceFeed(std::sync::Arc<std::sync::Mutex<Feed>>);
#[derive(Default)]
struct Feed {
    current: std::sync::Arc<ResourceSnapshot>,
    viewers: Vec<crate::Viewer>,
}
impl ResourceFeed {
    /// A feed whose viewers draw `snapshot` until another is published.
    pub fn new(snapshot: std::sync::Arc<ResourceSnapshot>) -> Self {
        Self(std::sync::Arc::new(std::sync::Mutex::new(Feed {
            current: snapshot,
            viewers: vec![],
        })))
    }
    /// A viewer placed by `make` with this feed's latest snapshot, attached
    /// before any publication can pass it by (`EngineService::viewer_fed`).
    pub(crate) fn place(
        &self,
        make: impl FnOnce(std::sync::Arc<ResourceSnapshot>) -> anyhow::Result<crate::Viewer>,
    ) -> anyhow::Result<crate::Viewer> {
        let mut feed = self.0.lock().unwrap();
        feed.viewers.retain(crate::Viewer::live);
        let viewer = make(feed.current.clone())?;
        feed.viewers.push(viewer.clone());
        Ok(viewer)
    }
    /// The viewer draws this feed's resources from now on (one already
    /// drawing; a new one is placed with [`crate::EngineService::viewer_fed`]).
    pub fn attach(&self, viewer: &crate::Viewer) -> anyhow::Result<()> {
        let mut feed = self.0.lock().unwrap();
        feed.viewers.retain(crate::Viewer::live);
        viewer.resources(feed.current.clone())?;
        feed.viewers.push(viewer.clone());
        Ok(())
    }
    /// Every attached viewer draws `snapshot` from now on; the first refusal
    /// is returned once all have been told.
    pub fn publish(&self, snapshot: std::sync::Arc<ResourceSnapshot>) -> anyhow::Result<()> {
        snapshot.validate()?;
        let mut feed = self.0.lock().unwrap();
        feed.current = snapshot;
        feed.viewers.retain(crate::Viewer::live);
        let mut refused = Ok(());
        for viewer in &feed.viewers {
            let told = viewer.resources(feed.current.clone());
            refused = refused.and(told);
        }
        refused
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unavailable_resources_never_silently_render_with_builtins() {
        let mut s = ResourceSnapshot::default();
        assert!(s.validate().is_ok());
        s.extension_generations.insert("fixture".into(), 7);
        assert!(s.validate().is_err());
        s.extension_generations.clear();
        s.project_fonts.push("fixture-font".into());
        assert!(s.validate().is_err());
        s.project_fonts.clear();
        let invert = pm_render::all_builtin_definitions()
            .definitions
            .into_iter()
            .find(|d| d.id.as_str() == "invert")
            .unwrap();
        s.definitions = vec![invert.clone(), invert];
        assert!(s.validate().is_err(), "one winner per kind and id");
    }
}
