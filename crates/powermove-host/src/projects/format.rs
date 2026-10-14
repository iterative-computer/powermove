//! The `.pmv` container formats (port of `shared/project-incremental.ts` and
//! the read side of `shared/project-container.ts`).
//!
//! PMV4 keeps immutable media at stable offsets and publishes a complete tail
//! index. Writers build a sibling copy and atomically replace the file; live
//! files are never appended in place. PMV3 and legacy JSON stay readable.

use std::collections::HashSet;

use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

pub const INCREMENTAL_MAGIC: &[u8] = b"PMV4\n";
const END_MAGIC: &[u8] = b"PMV4END\n";
pub const PROJECT_FOOTER_BYTES: u64 = 64;
const MAX_INDEX_BYTES: u64 = 64 * 1024 * 1024;
pub const CONTAINER_MAGIC: &[u8] = b"PMV3\n";
pub const MAX_MEDIA: usize = 100_000;
/// `Number.MAX_SAFE_INTEGER`
pub const MAX_SAFE_INTEGER: u64 = (1 << 53) - 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectRecord {
    pub offset: u64,
    pub length: u64,
    pub sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IncrementalMedia {
    pub id: String,
    pub revision: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub length: u64,
    pub offset: u64,
    pub sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ProjectIndex {
    pub version: u32,
    pub document: ProjectRecord,
    pub media: Vec<IncrementalMedia>,
}

/// What the renderer declares it will save (`SaveMedia`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SaveMedia {
    pub id: String,
    pub revision: String,
    pub kind: String,
    pub length: u64,
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

/// `Number.isSafeInteger(value) && value >= 0`
pub fn safe_uint(value: &Value) -> Option<u64> {
    if let Some(number) = value.as_u64() {
        return (number <= MAX_SAFE_INTEGER).then_some(number);
    }
    let number = value.as_f64()?;
    (number >= 0.0 && number.fract() == 0.0 && number <= MAX_SAFE_INTEGER as f64).then_some(number as u64)
}

/// A JavaScript string's `length` (UTF-16 code units).
pub fn js_len(text: &str) -> usize {
    text.encode_utf16().count()
}

/// JavaScript truthiness.
pub fn truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(flag) => *flag,
        Value::Number(number) => number.as_f64().is_some_and(|n| n != 0.0 && !n.is_nan()),
        Value::String(text) => !text.is_empty(),
        Value::Array(_) | Value::Object(_) => true,
    }
}

/// `document.proj || document` must look like a Powermove project.
pub fn validate_project_document(document: &Value) -> Result<()> {
    let project = match document.get("proj") {
        Some(proj) if truthy(proj) => proj,
        _ => document,
    };
    let valid = truthy(project)
        && project.get("layers").is_some_and(Value::is_array)
        && project.get("w").is_some_and(Value::is_number)
        && project.get("h").is_some_and(Value::is_number);
    if !valid {
        bail!("This is not a Powermove project.");
    }
    Ok(())
}

pub fn validate_save_media(media: &Value) -> Result<Vec<SaveMedia>> {
    let invalid = || anyhow::anyhow!("Invalid project media");
    let items = media.as_array().filter(|items| items.len() <= MAX_MEDIA).ok_or_else(invalid)?;
    let mut ids = HashSet::new();
    let mut result = Vec::with_capacity(items.len());
    for item in items {
        let text = |key: &str| item.get(key).and_then(Value::as_str);
        let (Some(id), Some(revision), Some(kind), Some(length)) =
            (text("id"), text("revision"), text("type"), item.get("length").and_then(safe_uint))
        else {
            return Err(invalid());
        };
        if id.is_empty() || js_len(id) > 1000 || ids.contains(id) || revision.is_empty() || js_len(revision) > 200 || js_len(kind) > 1000 {
            return Err(invalid());
        }
        ids.insert(id.to_owned());
        result.push(SaveMedia { id: id.into(), revision: revision.into(), kind: kind.into(), length });
    }
    Ok(result)
}

pub fn is_incremental_project(bytes: &[u8]) -> bool {
    bytes.starts_with(INCREMENTAL_MAGIC)
}

pub fn is_project_container(bytes: &[u8]) -> bool {
    bytes.starts_with(CONTAINER_MAGIC)
}

fn is_hash(text: &str) -> bool {
    text.len() == 64 && text.bytes().all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

pub fn project_footer(offset: u64, length: u64, digest: &str) -> Result<[u8; PROJECT_FOOTER_BYTES as usize]> {
    if !(5..=MAX_SAFE_INTEGER).contains(&offset) || !(2..=MAX_INDEX_BYTES).contains(&length) || !is_hash(digest) {
        bail!("Invalid project index");
    }
    let mut footer = [0u8; PROJECT_FOOTER_BYTES as usize];
    footer[..8].copy_from_slice(END_MAGIC);
    footer[8..16].copy_from_slice(&(offset as f64).to_le_bytes());
    footer[16..20].copy_from_slice(&(length as u32).to_le_bytes());
    footer[32..].copy_from_slice(&hex::decode(digest)?);
    Ok(footer)
}

pub fn read_project_footer(footer: &[u8], size: u64) -> Result<ProjectRecord> {
    if footer.len() != PROJECT_FOOTER_BYTES as usize || !footer.starts_with(END_MAGIC) {
        bail!("The project commit is truncated or invalid.");
    }
    let offset = f64::from_le_bytes(footer[8..16].try_into().unwrap());
    let length = u32::from_le_bytes(footer[16..20].try_into().unwrap()) as u64;
    let safe = offset.fract() == 0.0 && (5.0..=MAX_SAFE_INTEGER as f64).contains(&offset);
    if size > MAX_SAFE_INTEGER
        || !safe
        || !(2..=MAX_INDEX_BYTES).contains(&length)
        || size < PROJECT_FOOTER_BYTES
        || offset as u64 + length != size - PROJECT_FOOTER_BYTES
    {
        bail!("The project index is invalid.");
    }
    Ok(ProjectRecord { offset: offset as u64, length, sha256: hex::encode(&footer[32..]) })
}

fn record_range(value: &Value, index_offset: u64) -> Option<ProjectRecord> {
    let offset = value.get("offset").and_then(safe_uint)?;
    let length = value.get("length").and_then(safe_uint)?;
    let sha256 = value.get("sha256").and_then(Value::as_str)?;
    (offset >= 5 && offset <= index_offset && length <= index_offset - offset && is_hash(sha256))
        .then(|| ProjectRecord { offset, length, sha256: sha256.into() })
}

pub fn validate_project_index(value: &Value, index_offset: u64) -> Result<ProjectIndex> {
    let invalid = || anyhow::anyhow!("The project index is invalid.");
    if value.get("version").and_then(safe_uint) != Some(4) {
        return Err(invalid());
    }
    let document = value.get("document").and_then(|document| record_range(document, index_offset)).filter(|document| document.length >= 2).ok_or_else(invalid)?;
    let items = value.get("media").and_then(Value::as_array).filter(|items| items.len() <= MAX_MEDIA).ok_or_else(invalid)?;
    let mut ids = HashSet::new();
    let mut media = Vec::with_capacity(items.len());
    for item in items {
        let text = |key: &str| item.get(key).and_then(Value::as_str);
        let (Some(range), Some(id), Some(revision), Some(kind)) = (record_range(item, index_offset), text("id"), text("revision"), text("type")) else {
            bail!("The project media index is invalid.");
        };
        if id.is_empty() || js_len(id) > 1000 || ids.contains(id) || revision.is_empty() || js_len(revision) > 200 || js_len(kind) > 1000 {
            bail!("The project media index is invalid.");
        }
        ids.insert(id.to_owned());
        media.push(IncrementalMedia { id: id.into(), revision: revision.into(), kind: kind.into(), length: range.length, offset: range.offset, sha256: range.sha256 });
    }
    let mut ranges: Vec<(u64, u64)> = std::iter::once((document.offset, document.length))
        .chain(media.iter().map(|item| (item.offset, item.length)))
        .filter(|(_, length)| *length > 0)
        .collect();
    ranges.sort_unstable();
    if ranges.windows(2).any(|pair| pair[1].0 < pair[0].0 + pair[0].1) {
        bail!("The project records overlap.");
    }
    Ok(ProjectIndex { version: 4, document, media })
}

/// Read and verify a PMV4 tail index through `read(offset, length)`.
pub fn read_incremental_index(size: u64, mut read: impl FnMut(u64, u64) -> Result<Vec<u8>>) -> Result<ProjectIndex> {
    if size < INCREMENTAL_MAGIC.len() as u64 + PROJECT_FOOTER_BYTES {
        bail!("The project commit is truncated.");
    }
    let tail = read_project_footer(&read(size - PROJECT_FOOTER_BYTES, PROJECT_FOOTER_BYTES)?, size)?;
    let bytes = read(tail.offset, tail.length)?;
    if sha256_hex(&bytes) != tail.sha256 {
        bail!("The project index checksum does not match.");
    }
    validate_project_index(&serde_json::from_slice(&bytes)?, tail.offset)
}

/// A PMV3 media range, absolute in the file (`ProjectMediaRange`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ContainerMedia {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub offset: u64,
    pub length: u64,
}

/// Validate every PMV3 range before any imported media can be persisted.
pub fn project_container_index(header: &Value, body_start: u64, file_size: u64) -> Result<(Value, Vec<ContainerMedia>)> {
    let document = header.get("document").filter(|document| truthy(document));
    let (Some(document), Some(items)) = (document, header.get("media").and_then(Value::as_array)) else {
        bail!("The project container header is invalid.");
    };
    if body_start > file_size || file_size > MAX_SAFE_INTEGER {
        bail!("The project container header is invalid.");
    }
    let body = file_size - body_start;
    let mut ids = HashSet::new();
    let mut media = Vec::with_capacity(items.len());
    for item in items {
        let id = item.get("id").and_then(Value::as_str);
        let kind = item.get("type").and_then(Value::as_str);
        let offset = item.get("offset").and_then(safe_uint);
        let length = item.get("length").and_then(safe_uint);
        let (Some(id), Some(kind), Some(offset), Some(length)) = (id, kind, offset, length) else {
            bail!("The project container media index is invalid.");
        };
        if ids.contains(id) || offset > body || length > body - offset {
            bail!("The project container media index is invalid.");
        }
        ids.insert(id.to_owned());
        media.push(ContainerMedia { id: id.into(), kind: kind.into(), offset: body_start + offset, length });
    }
    Ok((document.clone(), media))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn footer_round_trips_and_checks_its_position() {
        let digest = sha256_hex(b"index");
        let footer = project_footer(1234, 99, &digest).unwrap();
        assert_eq!(&footer[..8], b"PMV4END\n");
        // The Mac app stores the offset as a little-endian float64.
        assert_eq!(f64::from_le_bytes(footer[8..16].try_into().unwrap()), 1234.0);
        let size = 1234 + 99 + PROJECT_FOOTER_BYTES;
        assert_eq!(read_project_footer(&footer, size).unwrap(), ProjectRecord { offset: 1234, length: 99, sha256: digest });
        assert!(read_project_footer(&footer, size + 1).is_err());
        let mut torn = footer;
        torn[0] = b'X';
        assert!(read_project_footer(&torn, size).is_err());
    }

    #[test]
    fn index_round_trips_and_rejects_bad_checksums() {
        let mut file = INCREMENTAL_MAGIC.to_vec();
        let document = br#"{"layers":[],"w":1,"h":1}"#;
        file.extend_from_slice(document);
        let index = ProjectIndex { version: 4, document: ProjectRecord { offset: 5, length: document.len() as u64, sha256: sha256_hex(document) }, media: vec![] };
        let bytes = serde_json::to_vec(&index).unwrap();
        let footer = project_footer(file.len() as u64, bytes.len() as u64, &sha256_hex(&bytes)).unwrap();
        file.extend_from_slice(&bytes);
        file.extend_from_slice(&footer);

        let read = |file: &Vec<u8>| {
            let file = file.clone();
            read_incremental_index(file.len() as u64, move |offset, length| Ok(file[offset as usize..(offset + length) as usize].to_vec()))
        };
        assert_eq!(read(&file).unwrap(), index);

        let mut corrupted = file.clone();
        let at = corrupted.len() - PROJECT_FOOTER_BYTES as usize - 2;
        corrupted[at] ^= 1;
        assert_eq!(read(&corrupted).unwrap_err().to_string(), "The project index checksum does not match.");
    }

    #[test]
    fn rejects_overlapping_records() {
        let hash = sha256_hex(b"");
        let index = json!({ "version": 4, "document": { "offset": 5, "length": 10, "sha256": hash },
            "media": [{ "id": "a", "revision": "1", "type": "image/png", "offset": 10, "length": 4, "sha256": hash }] });
        assert_eq!(validate_project_index(&index, 100).unwrap_err().to_string(), "The project records overlap.");
    }

    #[test]
    fn validates_container_ranges() {
        let header = json!({ "document": { "w": 1 }, "media": [{ "id": "a", "type": "x", "offset": 0, "length": 4 }] });
        let (_, media) = project_container_index(&header, 20, 24).unwrap();
        assert_eq!(media[0].offset, 20);
        assert!(project_container_index(&header, 20, 23).is_err());
    }
}
