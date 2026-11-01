//! P2 resource snapshot seam. Pins belong to the viewer/export request, not
//! mutable global registries. Unsupported requirements fail visibly.
use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ResourceSnapshot {
    pub generation: u64,
    pub extension_generations: std::collections::BTreeMap<String, u64>,
    pub project_fonts: Vec<String>,
}
impl ResourceSnapshot {
    pub fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.extension_generations.is_empty() && self.project_fonts.is_empty(),
            "extension/font resource activation requires the P2 snapshot host"
        );
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
        s.extension_generations.insert("fixture".into(), 7);
        assert!(s.validate().is_err());
        s.extension_generations.clear();
        s.project_fonts.push("fixture-font".into());
        assert!(s.validate().is_err());
    }
}
