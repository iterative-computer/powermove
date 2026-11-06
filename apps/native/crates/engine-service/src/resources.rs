//! P2 resource snapshot seam. Pins belong to the viewer/export request, not
//! mutable global registries. Unsupported requirements fail visibly.
use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ResourceSnapshot {
    pub generation: u64,
    pub extension_generations: std::collections::BTreeMap<String, u64>,
    pub project_fonts: Vec<String>,
    /// Effect, transition and layer definitions extensions registered (the
    /// registry's winners). Viewers and exports render them natively.
    #[serde(default)]
    pub definitions: Vec<pm_protocol::definitions::DefinitionSnapshot>,
}
impl ResourceSnapshot {
    pub fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.project_fonts.is_empty(),
            "project font activation requires the P2 snapshot host"
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
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unavailable_resources_never_silently_render_with_builtins() {
        let mut s = ResourceSnapshot::default();
        assert!(s.validate().is_ok());
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
