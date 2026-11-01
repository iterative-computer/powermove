//! Media sources and project assets (`assets`, asset folders).
//!
//! Ports media replacement (`legacy/assistant/media-management.ts`,
//! `manage_media`/`replace_media`), the library's asset operations
//! (`legacy/core/library.ts`, `asset-folders.ts`). Importing files (decode,
//! probe, store bytes) is the media lane's; the core only records the
//! asset entry it produces.

use pm_model::{Map, Value};

use super::{Cx, values};
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
    /// Remove media (`MediaImport.removeAsset`): the layers showing it go
    /// with it, as before (children parented to them are unparented). Label
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
        match self {
            MediaCommand::Replace { layer_id, asset_id } => {
                let (loc, layer) = cx.layer_mut_checked(layer_id)?;
                if !matches!(
                    values::text(&layer, "type"),
                    Some("image" | "video" | "audio")
                ) {
                    return Err(CommandError::Invalid(
                        "Choose an image, video or audio layer".into(),
                    ));
                }
                if cx
                    .lookup(&["assets".into(), asset_id.as_str().into()])
                    .is_none()
                {
                    return Err(CommandError::Invalid("Media not found".into()));
                }
                let mut path = loc.path();
                path.extend(["d".into(), "asset".into()]);
                cx.set(path, values::s(asset_id))
            }
            MediaCommand::AddAsset { asset_id, asset } => {
                if asset.as_object().is_none() {
                    return Err(CommandError::Invalid(
                        "An asset entry must be an object".into(),
                    ));
                }
                if cx.lookup(&["assets".into()]).is_none() {
                    cx.set(vec!["assets".into()], Value::Object(Map::new()))?;
                }
                let mut entry = asset.clone();
                values::obj(&mut entry).insert("id", values::s(asset_id));
                cx.set(vec!["assets".into(), asset_id.as_str().into()], entry)
            }
            MediaCommand::RemoveAsset { asset_id } => remove_asset(cx, asset_id),
            MediaCommand::RenameAsset { asset_id, name } => {
                let path = vec!["assets".into(), asset_id.as_str().into(), "name".into()];
                if cx.lookup(&path[..2]).is_none() {
                    return Err(CommandError::Invalid("Media not found".into()));
                }
                let t = values::js_trim(name);
                if t.is_empty() {
                    return Ok(());
                }
                cx.set(path, values::s(t))
            }
            MediaCommand::SetAssetFolder { .. } => {
                Err(CommandError::NotImplemented("set_asset_folder"))
            }
        }
    }
}

/// `layerAssetId(layer)`: the asset a media layer shows.
fn layer_asset(layer: &Value) -> Option<&str> {
    layer.get("d")?.get("asset")?.as_str()
}

/// `PM.MediaImport.removeAsset(project, id)`: layers showing the asset go
/// with it (children parented to them are unparented), then the entry.
fn remove_asset(cx: &mut Cx<'_>, asset_id: &str) -> Result<(), CommandError> {
    if cx.lookup(&["assets".into(), asset_id.into()]).is_none() {
        return Err(CommandError::Invalid("Media not found".into()));
    }
    let comps: Vec<String> = match cx.root().get("comps").map(pm_model::Node::kind) {
        Some(pm_model::NodeKind::Object(m)) => m
            .keys()
            .filter_map(|k| k.as_str().map(str::to_owned))
            .collect(),
        _ => Vec::new(),
    };
    for comp in comps {
        let layers = cx.layers(Some(&comp));
        let doomed: Vec<String> = layers
            .iter()
            .filter(|l| layer_asset(l) == Some(asset_id))
            .filter_map(|l| values::text(l, "id").map(str::to_owned))
            .collect();
        if doomed.is_empty() {
            continue;
        }
        let next: Vec<Value> = layers
            .into_iter()
            .filter(|l| !values::text(l, "id").is_some_and(|i| doomed.iter().any(|d| d == i)))
            .map(|mut l| {
                if values::text(&l, "parent").is_some_and(|p| doomed.iter().any(|d| d == p)) {
                    values::obj(&mut l).insert("parent", Value::Null);
                }
                l
            })
            .collect();
        super::layers::write_layers(cx, Some(&comp), next)?;
    }
    cx.remove(vec!["assets".into(), asset_id.into()])
}
