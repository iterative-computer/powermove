//! Immutable original ownership. The saved manifest grants only bounded reads
//! inside the attached native archive, or already verified embedded media.
use crate::{
    media_archive::Original,
    project_io::{MediaEntry, MediaSource, MediaTable},
};
use pm_model::{Map, Node, Value};
use std::collections::HashSet;

pub const FIELD: &str = "nativeMediaVersions";
#[derive(Clone, Debug)]
pub struct MediaBinding {
    pub asset: String,
    /// The complete asset descriptor observed before the worker started.
    pub expected: Option<Value>,
    pub original: Original,
    /// Retain an existing readable original before replacement. A missing
    /// original may only be absent here when the actor has no read capability.
    pub previous: Option<Original>,
}
#[derive(Clone, Debug)]
pub struct MediaAdmission {
    pub expected_revision: u64,
    pub project: Value,
    pub label: String,
    pub bindings: Vec<MediaBinding>,
    /// Applied in the same transaction, so one Undo also removes the layers
    /// placing what was admitted.
    pub place: Vec<crate::Command>,
}
pub(crate) struct Plan {
    pub manifest: Value,
    pub entries: Vec<MediaEntry>,
}

fn invalid(message: &str) -> String {
    message.into()
}
fn record(original: &Original) -> Value {
    let mut m = Map::new();
    m.insert("sha256", Value::from(original.hash()));
    m.insert("length", Value::Number(original.length() as f64));
    m.insert("mime", Value::from(original.mime()));
    Value::Object(m)
}
fn manifest(root: &Node) -> Result<Value, String> {
    match root.get(FIELD) {
        None => {
            let mut m = Map::new();
            m.insert("version", Value::Number(1.));
            m.insert("originals", Value::Object(Map::new()));
            m.insert("baselines", Value::Object(Map::new()));
            m.insert("owners", Value::Object(Map::new()));
            Ok(Value::Object(m))
        }
        Some(value) => {
            let value = value.to_value();
            if value.get("version").and_then(Value::as_f64) != Some(1.)
                || value.get("originals").and_then(Value::as_object).is_none()
                || value.get("baselines").and_then(Value::as_object).is_none()
            {
                return Err(invalid(
                    "Retained original metadata is invalid or newer than this build",
                ));
            }
            for (id, entry) in value.get("originals").unwrap().as_object().unwrap().iter() {
                validate_record(id.as_str().ok_or("Invalid retained original ID")?, entry)?;
            }
            Ok(value)
        }
    }
}
fn validate_record(id: &str, record: &Value) -> Result<(), String> {
    if id.is_empty()
        || id.contains(['/', '\\', '\0'])
        || record
            .get("sha256")
            .and_then(Value::as_str)
            .is_none_or(|h| !crate::media_archive::valid_hash(h))
        || record
            .get("length")
            .and_then(Value::as_f64)
            .is_none_or(|n| {
                !n.is_finite() || n < 0. || n.fract() != 0. || n > 9_007_199_254_740_991.
            })
        || record.get("mime").and_then(Value::as_str).is_none()
    {
        return Err(invalid("Invalid retained original record"));
    }
    Ok(())
}
fn same_record(entry: &Value, original: &Original) -> bool {
    entry.get("sha256").and_then(Value::as_str) == Some(original.hash())
        && entry.get("length").and_then(Value::as_f64) == Some(original.length() as f64)
        && entry.get("mime").and_then(Value::as_str) == Some(original.mime())
}
/// Names and folder moves do not select a different original.
fn descriptor(value: &Value) -> Value {
    let mut value = value.clone();
    if let Some(m) = value.as_object_mut() {
        for key in ["id", "name", "folder"] {
            m.remove(key);
        }
    }
    value
}
fn same_capability(a: &MediaEntry, b: &MediaEntry) -> bool {
    if a.id != b.id || a.media_type != b.media_type || a.revision != b.revision {
        return false;
    }
    match (&a.source, &b.source) {
        (MediaSource::Memory(a), MediaSource::Memory(b)) => std::sync::Arc::ptr_eq(a, b) || a == b,
        (
            MediaSource::File {
                path: a,
                length: al,
                sha256: ah,
            },
            MediaSource::File {
                path: b,
                length: bl,
                sha256: bh,
            },
        ) => a == b && al == bl && ah == bh,
        (
            MediaSource::Embedded {
                file: a,
                offset: ao,
                length: al,
                sha256: ah,
            },
            MediaSource::Embedded {
                file: b,
                offset: bo,
                length: bl,
                sha256: bh,
            },
        ) => a == b && ao == bo && al == bl && ah == bh,
        _ => false,
    }
}
fn add_record(
    manifest: &mut Value,
    id: &str,
    original: &Original,
    media: &MediaTable,
    entries: &mut Vec<MediaEntry>,
) -> Result<(), String> {
    let originals = manifest
        .as_object_mut()
        .unwrap()
        .get_mut("originals")
        .unwrap()
        .as_object_mut()
        .unwrap();
    if let Some(existing) = originals.get(id) {
        if !same_record(existing, original) {
            return Err(invalid(
                "A retained original ID already belongs to different bytes",
            ));
        }
    } else {
        if media.get(id).is_some() {
            return Err(invalid(
                "A project media ID already owns the retained original ID",
            ));
        }
        originals.insert(id, record(original));
    }
    entries.push(original.entry(id));
    Ok(())
}
pub(crate) fn plan(
    root: &Node,
    next: &Node,
    media: &MediaTable,
    bindings: &[MediaBinding],
) -> Result<Plan, String> {
    if bindings.is_empty() {
        return Err(invalid("Media admission needs completed original receipts"));
    }
    let mut manifest = manifest(root)?;
    for (id, _) in manifest
        .get("originals")
        .unwrap()
        .as_object()
        .unwrap()
        .iter()
    {
        if root
            .get("assets")
            .and_then(object)
            .is_some_and(|a| a.contains_key(id))
            || next
                .get("assets")
                .and_then(object)
                .is_some_and(|a| a.contains_key(id))
        {
            return Err(invalid(
                "A retained original ID conflicts with a project asset",
            ));
        }
    }
    for binding in bindings {
        if root
            .get("assets")
            .and_then(|a| a.get(binding.original.version()))
            .is_some()
            || next
                .get("assets")
                .and_then(|a| a.get(binding.original.version()))
                .is_some()
        {
            return Err(invalid(
                "A retained original ID conflicts with a project asset",
            ));
        }
    }
    let mut entries = Vec::new();
    let mut seen = HashSet::new();
    for binding in bindings {
        if binding.asset.is_empty() || !seen.insert(&binding.asset) {
            return Err(invalid("Media admission has duplicate or empty asset IDs"));
        }
        let before = root
            .get("assets")
            .and_then(|a| a.get(&binding.asset))
            .map(Node::to_value);
        if !match (&before, &binding.expected) {
            (Some(a), Some(b)) => a.identical(b),
            (None, None) => true,
            _ => false,
        } {
            return Err(invalid(
                "The media changed while its original was being retained",
            ));
        }
        let after = next
            .get("assets")
            .and_then(|a| a.get(&binding.asset))
            .ok_or("The imported asset is missing")?;
        if after.get("playbackProxy").and_then(crate::model::bool_of) == Some(true)
            || after
                .get("playbackProxyVersion")
                .and_then(crate::model::number_of)
                .is_some_and(|n| n != 0.)
        {
            return Err(invalid(
                "An original receipt cannot be admitted as a proxy fallback",
            ));
        }
        if after.get("originalVersion").and_then(crate::model::str_of)
            != Some(binding.original.version())
        {
            return Err(invalid(
                "The imported asset does not select its retained original",
            ));
        }
        if let Some(before) = before {
            let prior = match (&binding.previous, media.get(&binding.asset)) {
                (Some(prior), Some(current)) if matches_prior(prior, current) => Some(prior),
                (None, None) => None,
                _ => {
                    return Err(invalid(
                        "The prior original was not retained from the current read capability",
                    ));
                }
            };
            let version = before.get("originalVersion").and_then(Value::as_str);
            if let Some(version) = version {
                let existing = manifest
                    .get("originals")
                    .unwrap()
                    .get(version)
                    .ok_or("The selected original is absent from its ownership manifest")?;
                if let Some(prior) = prior {
                    if !same_record(existing, prior) {
                        return Err(invalid("The selected original changed before replacement"));
                    }
                    entries.push(prior.entry(version));
                }
            } else {
                let mut baseline = Map::new();
                baseline.insert("descriptor", descriptor(&before));
                if let Some(prior) = prior {
                    add_record(&mut manifest, prior.version(), prior, media, &mut entries)?;
                    baseline.insert("original", Value::from(prior.version()));
                } else {
                    baseline.insert("unavailable", Value::Bool(true));
                }
                let baselines = manifest
                    .as_object_mut()
                    .unwrap()
                    .get_mut("baselines")
                    .unwrap()
                    .as_object_mut()
                    .unwrap();
                if let Some(existing) = baselines.get(&binding.asset) {
                    if !existing
                        .get("descriptor")
                        .is_some_and(|d| d.identical(&descriptor(&before)))
                    {
                        return Err(invalid(
                            "A different legacy original already owns this asset baseline",
                        ));
                    }
                    // Preserve its existing immutable identity and unknown data.
                } else {
                    baselines.insert(binding.asset.as_str(), Value::Object(baseline));
                }
            }
        } else if binding.previous.is_some() {
            return Err(invalid("A new asset cannot supply a prior original"));
        }
        add_record(
            &mut manifest,
            binding.original.version(),
            &binding.original,
            media,
            &mut entries,
        )?;
        entries.push(binding.original.entry(&binding.asset));
    }
    let m = manifest.as_object_mut().unwrap();
    if !m.contains_key("owners") {
        m.insert("owners", Value::Object(Map::new()));
    }
    let owners = m
        .get_mut("owners")
        .unwrap()
        .as_object_mut()
        .ok_or("Invalid retained original owner table")?;
    for binding in bindings {
        if !owners.contains_key(&binding.asset) {
            owners.insert(binding.asset.as_str(), Value::Array(vec![]));
        }
        let versions = owners
            .get_mut(&binding.asset)
            .unwrap()
            .as_array_mut()
            .ok_or("Invalid retained original owner record")?;
        let previous = root
            .get("assets")
            .and_then(|a| a.get(&binding.asset))
            .and_then(|a| a.get("originalVersion"))
            .and_then(crate::model::str_of)
            .or_else(|| binding.previous.as_ref().map(Original::version));
        for version in std::iter::once(binding.original.version()).chain(previous) {
            if !versions.iter().any(|v| v.as_str() == Some(version)) {
                versions.push(Value::from(version));
            }
        }
    }
    Ok(Plan { manifest, entries })
}

/// Re-select immutable bytes after Undo/Redo/restore. Unknown or unsubstantiated
/// legacy states become unavailable; they never silently borrow newer bytes.
pub(crate) fn retarget(root: &Node, media: &mut MediaTable) {
    let Some(manifest) = root.get(FIELD).map(Node::to_value) else {
        return;
    };
    if manifest.get("version").and_then(Value::as_f64) != Some(1.) {
        return;
    }
    let Some(originals) = manifest.get("originals").and_then(Value::as_object) else {
        return;
    };
    let baselines = manifest.get("baselines").and_then(Value::as_object);
    let owner_ids = manifest
        .get("owners")
        .and_then(Value::as_object)
        .into_iter()
        .flat_map(|m| m.keys())
        .filter_map(|k| k.as_str().map(str::to_owned));
    let ids: HashSet<_> = baselines
        .into_iter()
        .flat_map(|m| m.keys())
        .filter_map(|k| k.as_str().map(str::to_owned))
        .chain(
            root.get("assets")
                .and_then(object)
                .into_iter()
                .flat_map(|m| m.iter())
                .filter(|(_, a)| a.get("originalVersion").is_some())
                .filter_map(|(id, _)| id.as_str().map(str::to_owned)),
        )
        .chain(owner_ids)
        .collect();
    for id in ids {
        let asset = root.get("assets").and_then(|a| a.get(&id));
        let selected = asset
            .and_then(|a| a.get("originalVersion"))
            .and_then(crate::model::str_of)
            .map(str::to_owned)
            .or_else(|| {
                let asset = asset?.to_value();
                let baseline = baselines?.get(&id)?;
                baseline
                    .get("descriptor")?
                    .identical(&descriptor(&asset))
                    .then(|| {
                        baseline
                            .get("original")
                            .and_then(Value::as_str)
                            .map(str::to_owned)
                    })
                    .flatten()
            });
        let source = selected
            .as_ref()
            .filter(|v| originals.get(v).is_some())
            .and_then(|v| {
                media
                    .get(v)
                    .filter(|entry| matches_record(entry, originals.get(v).unwrap()))
            })
            .cloned();
        if let Some(mut source) = source {
            source.id = id.as_str().into();
            if !media
                .get(&id)
                .is_some_and(|current| same_capability(current, &source))
            {
                media.insert(source);
            }
        } else {
            media.remove(&id);
        }
    }
}

pub(crate) fn matches_record(entry: &MediaEntry, record: &Value) -> bool {
    let hash = record.get("sha256").and_then(Value::as_str);
    entry.length() as f64 == record.get("length").and_then(Value::as_f64).unwrap_or(-1.)
        && entry.media_type.as_str() == record.get("mime").and_then(Value::as_str)
        && match &entry.source {
            MediaSource::File { sha256, .. } | MediaSource::Embedded { sha256, .. } => {
                sha256.as_deref() == hash && hash.is_some()
            }
            MediaSource::Memory(bytes) => {
                use sha2::{Digest, Sha256};
                hash.is_some_and(|expected| format!("{:x}", Sha256::digest(bytes)) == expected)
            }
        }
}

/// An omitted duplicate version is reconstructed only from matching embedded
/// bytes. File paths and saved string IDs alone do not certify a match.
pub(crate) fn hydrate_embedded(root: &Node, media: &mut MediaTable) {
    let Some(manifest) = root.get(FIELD).map(Node::to_value) else {
        return;
    };
    if manifest.get("version").and_then(Value::as_f64) != Some(1.) {
        return;
    }
    let Some(originals) = manifest.get("originals").and_then(Value::as_object) else {
        return;
    };
    for (id, record) in originals.iter() {
        let Some(id) = id.as_str() else { continue };
        if validate_record(id, record).is_err() {
            continue;
        }
        if media.get(id).is_some_and(|entry| {
            matches!(entry.source, MediaSource::Embedded { .. }) && matches_record(entry, record)
        }) {
            continue;
        }
        let found = root
            .get("assets")
            .and_then(object)
            .into_iter()
            .flat_map(|m| m.iter())
            .find_map(|(asset, value)| {
                let asset = asset.as_str()?;
                let selected = value.get("originalVersion").and_then(crate::model::str_of)
                    == Some(id)
                    || manifest
                        .get("baselines")
                        .and_then(|b| b.get(asset))
                        .is_some_and(|b| {
                            b.get("original").and_then(Value::as_str) == Some(id)
                                && b.get("descriptor")
                                    .is_some_and(|d| d.identical(&descriptor(&value.to_value())))
                        });
                if !selected {
                    return None;
                }
                media
                    .get(asset)
                    .filter(|entry| {
                        matches!(entry.source, MediaSource::Embedded { .. })
                            && matches_record(entry, record)
                    })
                    .cloned()
            });
        if let Some(mut entry) = found {
            entry.id = id.into();
            media.insert(entry);
        }
    }
    retarget(root, media);
}

pub(crate) fn hydrate_archive(
    root: &Node,
    media: &mut MediaTable,
    archive: &crate::media_archive::Archive,
) {
    hydrate_embedded(root, media);
    let Some(manifest) = root.get(FIELD).map(Node::to_value) else {
        return;
    };
    if manifest.get("version").and_then(Value::as_f64) != Some(1.) {
        return;
    }
    let Some(originals) = manifest.get("originals").and_then(Value::as_object) else {
        return;
    };
    for (id, record) in originals.iter() {
        let Some(id) = id.as_str() else { continue };
        if validate_record(id, record).is_err()
            || media
                .get(id)
                .is_some_and(|entry| matches_record(entry, record))
        {
            continue;
        }
        if let Some(entry) = archive.reference(
            id,
            record.get("sha256").unwrap().as_str().unwrap(),
            record.get("length").unwrap().as_f64().unwrap() as u64,
            record.get("mime").unwrap().as_str().unwrap(),
        ) {
            media.insert(entry);
        }
    }
    retarget(root, media);
}

pub(crate) fn redundant_active_original(
    root: &Node,
    media: &MediaTable,
    entry: &MediaEntry,
) -> bool {
    let Some(manifest) = root.get(FIELD).map(Node::to_value) else {
        return false;
    };
    if manifest.get("version").and_then(Value::as_f64) != Some(1.) {
        return false;
    }
    let Some(record) = manifest
        .get("originals")
        .and_then(|m| entry.id.as_str().and_then(|id| m.get(id)))
    else {
        return false;
    };
    if !matches_record(entry, record) {
        return false;
    }
    root.get("assets")
        .and_then(object)
        .into_iter()
        .flat_map(|m| m.iter())
        .any(|(asset, value)| {
            (value.get("originalVersion").and_then(crate::model::str_of) == entry.id.as_str()
                || asset
                    .as_str()
                    .and_then(|id| manifest.get("baselines").and_then(|b| b.get(id)))
                    .is_some_and(|baseline| {
                        baseline.get("original").and_then(Value::as_str) == entry.id.as_str()
                            && baseline
                                .get("descriptor")
                                .is_some_and(|d| d.identical(&descriptor(&value.to_value())))
                    }))
                && asset
                    .as_str()
                    .and_then(|id| media.get(id))
                    .is_some_and(|alias| matches_record(alias, record))
        })
}

fn object(node: &Node) -> Option<&pm_model::json::object_map::ObjectMap<Node>> {
    match node.kind() {
        pm_model::NodeKind::Object(m) => Some(m),
        _ => None,
    }
}

fn matches_prior(prior: &Original, current: &MediaEntry) -> bool {
    let Some(observed) = prior.observed() else {
        return false;
    };
    if same_capability(observed, current) {
        return true;
    }
    if observed.id != current.id
        || observed.revision != current.revision
        || observed.media_type != current.media_type
        || current.length() != prior.length()
        || current.media_type.as_str() != Some(prior.mime())
    {
        return false;
    }
    match &current.source {
        MediaSource::File { sha256, .. } | MediaSource::Embedded { sha256, .. } => {
            sha256.as_deref() == Some(prior.hash())
        }
        _ => false,
    }
}
