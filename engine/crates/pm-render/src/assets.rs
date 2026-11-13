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
    /// The host is still looking `asset`'s file up (just imported or
    /// relinked): a frame draws nothing for it, never the Media Offline card.
    fn resolving(&self, _asset: &str) -> bool {
        false
    }
    /// The original file of `asset` when [`Self::path`] gives a converted
    /// copy of it (a proxy or optimized media): Interpret Footage describes
    /// the original, and is carried to the copy by what the copy's maker did
    /// (`crate::footage::for_copy`).
    fn original(&self, _asset: &str) -> Option<PathBuf> {
        None
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
    /// The original of an asset whose path is a converted copy.
    pub originals: std::collections::BTreeMap<String, PathBuf>,
}

impl AssetResolver for MapResolver {
    fn path(&self, asset: &str, _record: Option<&Node>) -> Option<PathBuf> {
        self.paths.get(asset).cloned()
    }
    fn original(&self, asset: &str) -> Option<PathBuf> {
        self.originals.get(asset).cloned()
    }
}
