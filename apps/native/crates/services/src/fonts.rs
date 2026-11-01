//! v1.2.0 font cache: cache only a nonempty enumeration at a stable registry version.
use crate::error::Result;

pub trait FontHook {
    fn version(&mut self) -> Result<Option<u64>>;
    fn families(&mut self) -> Result<Option<Vec<String>>>;
}

pub struct FontCatalog<H> {
    pub hook: H,
    cached: Option<(u64, Vec<String>)>,
}
impl<H: FontHook> FontCatalog<H> {
    pub fn new(hook: H) -> Self {
        Self { hook, cached: None }
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
