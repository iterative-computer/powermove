//! Where an asset's bytes are (section 21, "Where bytes live").
//!
//! The renderer never decides where media lives: the host (the media store
//! in main, a job snapshot, a test) answers with a file path per asset.

use std::path::PathBuf;

use pm_model::Node;

/// Maps document asset ids to files.
pub trait AssetResolver: Send + Sync {
    /// The original media file of `asset` (its document record is passed
    /// when the document has one), or `None` when the media is offline.
    fn path(&self, asset: &str, record: Option<&Node>) -> Option<PathBuf>;

    /// Exact Chromium Canvas2D PNG for a Blender image map. Job and viewer
    /// hosts obtain this from the document window through `compatFrame`;
    /// standalone callers may provide a captured Chromium conversion.
    fn blender_png(
        &self,
        _asset: &str,
        _source: &std::path::Path,
    ) -> Result<Option<std::sync::Arc<Vec<u8>>>, String> {
        Ok(None)
    }
}

/// Reads the record's `sourcePath` (linked media).
#[derive(Debug, Default, Clone, Copy)]
pub struct SourcePaths;

impl AssetResolver for SourcePaths {
    fn path(&self, _asset: &str, record: Option<&Node>) -> Option<PathBuf> {
        let path = record?.get("sourcePath")?;
        match path.kind() {
            pm_model::NodeKind::String(s) => Some(PathBuf::from(s.to_string_lossy())),
            _ => None,
        }
    }
}

/// A fixed map from asset id to path, with an optional fallback resolver.
#[derive(Default)]
pub struct MapResolver {
    pub paths: std::collections::BTreeMap<String, PathBuf>,
}

impl AssetResolver for MapResolver {
    fn path(&self, asset: &str, _record: Option<&Node>) -> Option<PathBuf> {
        self.paths.get(asset).cloned()
    }
}
