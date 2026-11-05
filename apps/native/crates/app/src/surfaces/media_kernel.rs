//! Worker admission seam for completed S038 batches and S006 replacements.
//! Uses existing app dependencies only. Bytes are archived by the media worker
//! before this call; a failed/stale kernel commit never removes that archive.
use super::media::{Owner, Target};
use pm_kernel::{Call, Caller, KernelHandle, Pending, Published, Request};
use pm_model::{Map, Value};
use std::collections::BTreeSet;

#[derive(Clone, Debug)]
pub struct ArchivedAsset {
    pub id: String,
    pub metadata: Value,
    /// Durable original-byte receipt, not a proxy path or an unprobed filename.
    pub original_version: String,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AdmissionError {
    Stale,
    InvalidAsset,
    MissingAsset,
    ArchiveRequired,
}
pub fn owner(window: String, published: &Published) -> Owner {
    Owner {
        window,
        project: published.project.clone(),
        document: published.snapshot.doc_id,
        revision: published.revision(),
    }
}
/// Build on a worker, then enqueue Restore: one history entry, atomic lifetime
/// and revision guard, no second writer and no changes to layer timing/trim.
/// Completed assets only; cancelled/failed jobs must not be supplied here.
pub fn admission(
    target: &Target,
    published: &Published,
    assets: Vec<ArchivedAsset>,
    replace: bool,
) -> Result<Request, AdmissionError> {
    if target.owner.project != published.project
        || target.owner.document != published.snapshot.doc_id
        || target.owner.revision != published.revision()
    {
        return Err(AdmissionError::Stale);
    }
    if assets.is_empty() || (replace && (assets.len() != 1 || assets[0].id != target.id)) {
        return Err(AdmissionError::InvalidAsset);
    }
    let mut root = published
        .snapshot
        .document
        .to_value()
        .ok_or(AdmissionError::InvalidAsset)?;
    let object = root.as_object_mut().ok_or(AdmissionError::InvalidAsset)?;
    if object.get("assets").is_none() {
        object.insert("assets", Value::Object(Map::new()));
    }
    let map = object
        .get_mut("assets")
        .and_then(Value::as_object_mut)
        .ok_or(AdmissionError::InvalidAsset)?;
    let mut ids = BTreeSet::new();
    for asset in assets {
        if asset.id.is_empty() || !ids.insert(asset.id.clone()) {
            return Err(AdmissionError::InvalidAsset);
        }
        if asset.original_version.is_empty() {
            return Err(AdmissionError::ArchiveRequired);
        }
        let metadata = asset
            .metadata
            .as_object()
            .ok_or(AdmissionError::InvalidAsset)?;
        if !["name", "kind"]
            .iter()
            .all(|key| metadata.get(key).and_then(Value::as_str).is_some())
        {
            return Err(AdmissionError::InvalidAsset);
        }
        let mut entry = if replace {
            map.get(asset.id.as_str())
                .cloned()
                .ok_or(AdmissionError::MissingAsset)?
        } else {
            if map.get(asset.id.as_str()).is_some() {
                return Err(AdmissionError::InvalidAsset);
            }
            Value::Object(Map::new())
        };
        let fields = entry.as_object_mut().ok_or(AdmissionError::InvalidAsset)?;
        for (key, value) in metadata.iter() {
            if !replace || key.as_str() != Some("folder") {
                fields.insert(key.clone(), value.clone());
            }
        }
        fields.insert("id", Value::from(asset.id.as_str()));
        fields.insert(
            "originalVersion",
            Value::from(asset.original_version.as_str()),
        );
        map.insert(asset.id.as_str(), entry);
    }
    Ok(Request::new(
        Caller::native(),
        Call::Restore {
            project: root,
            label: if replace {
                "Replace Media"
            } else {
                "Import Media"
            }
            .into(),
            expected_revision: target.owner.revision,
            expected_document: target.owner.document,
        },
    ))
}
pub fn submit(
    kernel: &KernelHandle,
    target: &Target,
    request: Request,
) -> Pending<Result<pm_kernel::Ack, pm_kernel::KernelFailure>> {
    kernel.call(&target.owner.project, request)
}
#[cfg(test)]
#[path = "media_kernel_tests.rs"]
mod tests;
