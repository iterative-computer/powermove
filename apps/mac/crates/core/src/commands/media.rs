//! Media sources and project assets (`assets`, asset folders).
//!
//! Ports media replacement (`legacy/assistant/media-management.ts`,
//! `manage_media`/`replace_media`), the library's asset operations
//! (`legacy/core/library.ts`, `asset-folders.ts`). Importing files (decode,
//! probe, store bytes) is the media lane's; the core only records the
//! asset entry it produces.

use pm_model::Value;

use super::Cx;
use crate::error::CommandError;

/// Media commands.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum MediaCommand {
    /// Point a media layer (`image`, `video`, `audio`) at another asset,
    /// keeping its timing, trim and transform. Label "Replace Media".
    Replace { layer_id: String, asset_id: String },
    /// Record an imported asset entry (`assets[id]`: name, kind, size,
    /// duration, ...). Label "Import".
    AddAsset { asset_id: String, asset: Value },
    /// Remove an asset no layer uses (`This media is used by …`). Label
    /// "Remove Media".
    RemoveAsset { asset_id: String },
    /// Rename an asset. Label "Rename Media".
    RenameAsset { asset_id: String, name: String },
    /// Move assets into a library folder (`None`: the root). Label "Move
    /// to Folder".
    SetAssetFolder {
        asset_ids: Vec<String>,
        folder: Option<String>,
    },
}

impl MediaCommand {
    pub fn label(&self) -> String {
        match self {
            MediaCommand::Replace { .. } => "Replace Media",
            MediaCommand::AddAsset { .. } => "Import",
            MediaCommand::RemoveAsset { .. } => "Remove Media",
            MediaCommand::RenameAsset { .. } => "Rename Media",
            MediaCommand::SetAssetFolder { .. } => "Move to Folder",
        }
        .to_owned()
    }

    pub(super) fn plan(&self, cx: &mut Cx<'_>) -> Result<(), CommandError> {
        let _ = cx;
        Err(CommandError::NotImplemented(match self {
            MediaCommand::Replace { .. } => "replace_media",
            MediaCommand::AddAsset { .. } => "add_asset",
            MediaCommand::RemoveAsset { .. } => "remove_asset",
            MediaCommand::RenameAsset { .. } => "rename_asset",
            MediaCommand::SetAssetFolder { .. } => "set_asset_folder",
        }))
    }
}
