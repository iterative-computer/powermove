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
}
impl ResourceSnapshot {
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
