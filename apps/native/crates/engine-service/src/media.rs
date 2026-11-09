//! The source paths the viewer and its audio thread resolve assets through.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

/// An asset id to its resolved file: `None` for an asset the host has not
/// answered for yet, `Some(None)` for one it reported offline.
#[derive(Clone, Default)]
pub struct MediaMap(pub Arc<BTreeMap<String, Option<PathBuf>>>);

impl MediaMap {
    /// The file an asset resolved to, when it did.
    pub fn path(&self, asset: &str) -> Option<PathBuf> {
        self.0.get(asset).cloned().flatten()
    }

    /// The host's answer for an asset: `None` before it answered, `Some(None)`
    /// when the file is offline.
    pub fn answer(&self, asset: &str) -> Option<Option<PathBuf>> {
        self.0.get(asset).cloned()
    }
}

impl pm_render::AssetResolver for MediaMap {
    fn path(&self, asset: &str, _record: Option<&pm_model::Node>) -> Option<PathBuf> {
        self.0.get(asset).cloned().flatten()
    }
}

impl pm_audio::AssetResolver for MediaMap {
    fn resolve(&self, asset: &str) -> Option<PathBuf> {
        self.0.get(asset).cloned().flatten()
    }
}
