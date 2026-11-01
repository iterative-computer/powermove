//! v1.2.0 font cache: cache only a nonempty enumeration at a stable registry version.
use crate::error::Result;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Face {
    /// Registered face name from CoreText, retained verbatim for face selection.
    pub name: String,
    /// CSS/OpenType weight as used by the engine, including fractional weights.
    /// The broker maps CoreText's native weight; never infer it from the name.
    pub weight: f32,
    pub italic: bool,
}

/// Implement all three operations through the same main-thread CoreText broker.
/// Invoke from a services worker; do not introduce a synchronous UI/JS wait.
pub trait FontHook {
    fn version(&mut self) -> Result<Option<u64>>;
    fn families(&mut self) -> Result<Option<Vec<String>>>;
    fn faces(&mut self, family: &str) -> Result<Vec<Face>>;
}

pub struct FontCatalog<H> {
    pub hook: H,
    cached: Option<(u64, Vec<String>)>,
}
impl<H: FontHook> FontCatalog<H> {
    pub fn new(hook: H) -> Self {
        Self { hook, cached: None }
    }
    /// Family submenus and the inspector Weight row consume the same broker facts.
    /// The shell draws family names in their own face (review item 5). Face lists
    /// are read fresh, so font activation cannot leave a stale face cache here.
    pub fn faces(&mut self, family: &str) -> Result<Vec<Face>> {
        self.hook.faces(family)
    }
    pub fn families(&mut self) -> Result<Option<Vec<String>>> {
        let version = self.hook.version()?;
        if let (Some(version), Some((cached, families))) = (version, &self.cached)
            && version == *cached
        {
            return Ok(Some(families.clone()));
        }
        // On an error no stale entry can survive and become a later hit.
        self.cached = None;
        let families = self.hook.families()?;
        if let (Some(version), Some(families)) = (version, &families)
            && !families.is_empty()
            && self.hook.version()? == Some(version)
        {
            self.cached = Some((version, families.clone()));
        }
        Ok(families)
    }
}
