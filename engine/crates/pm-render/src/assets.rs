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
    /// True only for a validated viewer proxy supplied by this resolver.
    /// Such media uses its generated display aspect in original coordinates.
    fn is_proxy(&self, _asset: &str) -> bool { false }
    /// A named availability state when a viewer representation is pending.
    fn unavailable_reason(&self, _asset: &str, _record: Option<&Node>) -> Option<String> { None }

    /// A viewer asset explicitly pinned to the exact original pixels. This
    /// also disables partial-resolution decode of that original.
    fn force_full_original(&self, _asset: &str) -> bool {
        false
    }

    /// A measured, source-space decode scale that the viewer may use while
    /// playing. `None` retains the complete source picture.
    fn preview_decode_scale(&self, _asset: &str) -> Option<f64> {
        None
    }

    /// A separately measured intermediate tier when the preferred reduced
    /// decode is too small for a transformed layer's output footprint.
    fn preview_decode_fallback_scale(&self, _asset: &str) -> Option<f64> {
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
}

impl AssetResolver for MapResolver {
    fn path(&self, asset: &str, _record: Option<&Node>) -> Option<PathBuf> {
        self.paths.get(asset).cloned()
    }
}
