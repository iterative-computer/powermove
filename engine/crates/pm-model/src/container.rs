//! Read-only project file readers: plain JSON, PMV3, and PMV4
//! (`apps/desktop/docs/project-files.md`, `src/shared/project-container.ts`,
//! `src/shared/project-incremental.ts`).
//!
//! Formats:
//!
//! * **JSON**: the document as UTF-8 text.
//! * **PMV3**: `PMV3\n`, a little-endian `u32` header length, a JSON header
//!   `{ document, media: [{ id, type, offset, length }] }`, then the media
//!   bytes; media offsets are relative to the end of the header.
//! * **PMV4**: `PMV4\n`, records at stable offsets (the document JSON and the
//!   media), a JSON index `{ version: 4, document: { offset, length, sha256 },
//!   media: [{ id, revision, type, offset, length, sha256 }] }`, and a 64-byte
//!   footer: `PMV4END\n`, the index offset as a little-endian `f64` at byte 8,
//!   the index length as a little-endian `u32` at byte 16, and the index's
//!   SHA-256 at bytes 32 to 63.
//!
//! Detection and validation follow `decodeProjectContainer` exactly, including
//! its error messages: the PMV4 magic first, then the PMV3 magic, otherwise
//! JSON. Text is decoded like `TextDecoder` (a leading byte order mark is
//! dropped and invalid UTF-8 becomes U+FFFD) and parsed like `JSON.parse`. The
//! PMV4 index and document checksums are always verified; media checksums are
//! verified when [`ReadOptions::verify_media`] is set (as
//! `decodeProjectContainer` does) or later with [`Container::verify_media`].
//!
//! A saved document is `{ proj, history, ws, v }` or, in older files, the bare
//! project; [`crate::canonical::project_of`] picks the project.

use core::fmt;

use sha2::{Digest, Sha256};

use crate::json::{self, JsString, ParseError, Value};

/// `PMV3\n`.
pub const PMV3_MAGIC: &[u8; 5] = b"PMV3\n";
/// `PMV4\n`.
pub const PMV4_MAGIC: &[u8; 5] = b"PMV4\n";
/// `PMV4END\n`, the start of the PMV4 footer.
pub const PMV4_END_MAGIC: &[u8; 8] = b"PMV4END\n";
/// Size of the PMV4 footer.
pub const PMV4_FOOTER_BYTES: u64 = 64;
/// Largest PMV4 index.
pub const MAX_INDEX_BYTES: u64 = 64 * 1024 * 1024;
/// Most media records in a PMV4 index.
pub const MAX_MEDIA_RECORDS: usize = 100_000;
/// `Number.MAX_SAFE_INTEGER`.
const MAX_SAFE_INTEGER: f64 = 9_007_199_254_740_991.0;

/// Which format a file used.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ContainerFormat {
    Json,
    Pmv3,
    Pmv4,
}

/// One embedded media item. Offsets are absolute file offsets.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MediaRecord {
    pub id: JsString,
    /// The MIME type the file recorded (`type`).
    pub media_type: JsString,
    pub offset: u64,
    pub length: u64,
    /// PMV4 storage revision.
    pub revision: Option<JsString>,
    /// PMV4 SHA-256 as lowercase hex.
    pub sha256: Option<String>,
}

/// A decoded project file.
#[derive(Clone, Debug, PartialEq)]
pub struct Container {
    pub format: ContainerFormat,
    /// The saved document (`{ proj, history, ... }` or a bare project).
    pub document: Value,
    pub media: Vec<MediaRecord>,
}

/// Why a file could not be read.
#[derive(Clone, Debug, PartialEq)]
pub enum ContainerError {
    /// A structural problem, with the message `decodeProjectContainer` throws.
    Invalid(&'static str),
    /// JSON that `JSON.parse` rejects.
    Json(ParseError),
    /// Reading the source failed.
    Io(String),
}

impl fmt::Display for ContainerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ContainerError::Invalid(message) => f.write_str(message),
            ContainerError::Json(e) => write!(f, "invalid JSON: {e}"),
            ContainerError::Io(e) => write!(f, "read failed: {e}"),
        }
    }
}

impl std::error::Error for ContainerError {}

impl From<ParseError> for ContainerError {
    fn from(e: ParseError) -> ContainerError {
        ContainerError::Json(e)
    }
}

const TRUNCATED: ContainerError = ContainerError::Invalid("The project container is truncated.");
const BAD_HEADER: ContainerError =
    ContainerError::Invalid("The project container header is invalid.");
const BAD_MEDIA_INDEX: ContainerError =
    ContainerError::Invalid("The project container media index is invalid.");
const BAD_COMMIT: ContainerError =
    ContainerError::Invalid("The project commit is truncated or invalid.");
const BAD_INDEX: ContainerError = ContainerError::Invalid("The project index is invalid.");
const BAD_INDEX_CHECKSUM: ContainerError =
    ContainerError::Invalid("The project index checksum does not match.");
const BAD_PMV4_MEDIA: ContainerError =
    ContainerError::Invalid("The project media index is invalid.");
const OVERLAP: ContainerError = ContainerError::Invalid("The project records overlap.");
const BAD_CONTENT_CHECKSUM: ContainerError =
    ContainerError::Invalid("The project content checksum does not match.");

/// Random-access bytes: a slice in memory or a file on disk.
pub trait RandomAccess {
    /// Total length in bytes.
    fn size(&self) -> Result<u64, ContainerError>;
    /// Fills `buf` from `offset`; fails if the source is shorter.
    fn read_at(&self, offset: u64, buf: &mut [u8]) -> Result<(), ContainerError>;
}

impl RandomAccess for [u8] {
    fn size(&self) -> Result<u64, ContainerError> {
        Ok(self.len() as u64)
    }

    fn read_at(&self, offset: u64, buf: &mut [u8]) -> Result<(), ContainerError> {
        let start = usize::try_from(offset).map_err(|_| TRUNCATED)?;
        let end = start.checked_add(buf.len()).ok_or(TRUNCATED)?;
        buf.copy_from_slice(self.get(start..end).ok_or(TRUNCATED)?);
        Ok(())
    }
}

impl RandomAccess for Vec<u8> {
    fn size(&self) -> Result<u64, ContainerError> {
        self.as_slice().size()
    }

    fn read_at(&self, offset: u64, buf: &mut [u8]) -> Result<(), ContainerError> {
        self.as_slice().read_at(offset, buf)
    }
}

#[cfg(unix)]
impl RandomAccess for std::fs::File {
    fn size(&self) -> Result<u64, ContainerError> {
        self.metadata()
            .map(|m| m.len())
            .map_err(|e| ContainerError::Io(e.to_string()))
    }

    fn read_at(&self, offset: u64, buf: &mut [u8]) -> Result<(), ContainerError> {
        use std::os::unix::fs::FileExt;
        self.read_exact_at(buf, offset)
            .map_err(|e| ContainerError::Io(e.to_string()))
    }
}

#[cfg(windows)]
impl RandomAccess for std::fs::File {
    fn size(&self) -> Result<u64, ContainerError> {
        self.metadata()
            .map(|m| m.len())
            .map_err(|e| ContainerError::Io(e.to_string()))
    }

    fn read_at(&self, mut offset: u64, mut buf: &mut [u8]) -> Result<(), ContainerError> {
        use std::os::windows::fs::FileExt;
        while !buf.is_empty() {
            let n = self
                .seek_read(buf, offset)
                .map_err(|e| ContainerError::Io(e.to_string()))?;
            if n == 0 {
                return Err(TRUNCATED);
            }
            buf = &mut buf[n..];
            offset += n as u64;
        }
        Ok(())
    }
}

/// How much to verify while reading.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ReadOptions {
    /// Verify every PMV4 media checksum before returning (what
    /// `decodeProjectContainer` does). Off, media is verified on demand.
    pub verify_media: bool,
}

impl Default for ReadOptions {
    fn default() -> ReadOptions {
        ReadOptions { verify_media: true }
    }
}

fn read_vec<R: RandomAccess + ?Sized>(
    source: &R,
    offset: u64,
    length: u64,
) -> Result<Vec<u8>, ContainerError> {
    let length = usize::try_from(length).map_err(|_| TRUNCATED)?;
    let mut buf = vec![0u8; length];
    source.read_at(offset, &mut buf)?;
    Ok(buf)
}

fn sha256_of_range<R: RandomAccess + ?Sized>(
    source: &R,
    offset: u64,
    length: u64,
) -> Result<String, ContainerError> {
    const CHUNK: u64 = 1 << 20;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; CHUNK.min(length) as usize];
    let mut done = 0;
    while done < length {
        let n = CHUNK.min(length - done) as usize;
        source.read_at(offset + done, &mut buf[..n])?;
        hasher.update(&buf[..n]);
        done += n as u64;
    }
    Ok(crate::hash::hex(&hasher.finalize().into()))
}

/// Reads a project file of any format from memory.
pub fn read_container_bytes(
    bytes: &[u8],
    options: ReadOptions,
) -> Result<Container, ContainerError> {
    read_container(bytes, options)
}

/// Reads a project file of any format.
pub fn read_container<R: RandomAccess + ?Sized>(
    source: &R,
    options: ReadOptions,
) -> Result<Container, ContainerError> {
    let size = source.size()?;
    let prefix = read_vec(source, 0, size.min(9))?;
    if prefix.len() >= 5 && prefix[..5] == *PMV4_MAGIC {
        return read_pmv4(source, size, options);
    }
    if prefix.len() >= 5 && prefix[..5] == *PMV3_MAGIC {
        return read_pmv3(source, size);
    }
    let text = read_vec(source, 0, size)?;
    Ok(Container {
        format: ContainerFormat::Json,
        document: json::parse_file_text(&text)?,
        media: Vec::new(),
    })
}

/// `Number.isSafeInteger` for a JSON value.
fn safe_integer(value: Option<&Value>) -> Option<f64> {
    match value {
        Some(Value::Number(n)) if n.fract() == 0.0 && n.abs() <= MAX_SAFE_INTEGER => Some(*n),
        _ => None,
    }
}

fn read_pmv3<R: RandomAccess + ?Sized>(source: &R, size: u64) -> Result<Container, ContainerError> {
    if size < 9 {
        return Err(TRUNCATED);
    }
    let mut length = [0u8; 4];
    source.read_at(5, &mut length)?;
    let header_length = u32::from_le_bytes(length) as u64;
    let body_start = 9 + header_length;
    if header_length < 2 || body_start > size {
        return Err(BAD_HEADER);
    }
    let header = json::parse_file_text(&read_vec(source, 9, header_length)?)?;
    let (document, media) = pmv3_index(header, body_start, size)?;
    Ok(Container {
        format: ContainerFormat::Pmv3,
        document,
        media,
    })
}

/// `projectContainerIndex(header, bodyStart, fileSize)`.
fn pmv3_index(
    header: Value,
    body_start: u64,
    file_size: u64,
) -> Result<(Value, Vec<MediaRecord>), ContainerError> {
    let Value::Object(mut header) = header else {
        return Err(BAD_HEADER);
    };
    let document_ok = header.get("document").is_some_and(Value::is_truthy);
    let Some(Value::Array(items)) = header.remove("media") else {
        return Err(BAD_HEADER);
    };
    if !document_ok {
        return Err(BAD_HEADER);
    }
    let available = (file_size - body_start) as f64;
    let mut seen = std::collections::BTreeSet::new();
    let mut media = Vec::with_capacity(items.len());
    for item in &items {
        let (Some(Value::String(id)), Some(Value::String(media_type))) =
            (item.get("id"), item.get("type"))
        else {
            return Err(BAD_MEDIA_INDEX);
        };
        let (Some(offset), Some(length)) = (
            safe_integer(item.get("offset")),
            safe_integer(item.get("length")),
        ) else {
            return Err(BAD_MEDIA_INDEX);
        };
        if seen.contains(id)
            || offset < 0.0
            || length < 0.0
            || offset > available
            || length > available - offset
        {
            return Err(BAD_MEDIA_INDEX);
        }
        seen.insert(id.clone());
        media.push(MediaRecord {
            id: id.clone(),
            media_type: media_type.clone(),
            offset: body_start + offset as u64,
            length: length as u64,
            revision: None,
            sha256: None,
        });
    }
    let document = header.remove("document").unwrap_or(Value::Null);
    Ok((document, media))
}

fn is_hash(value: &Value) -> Option<String> {
    let s = value.as_str()?;
    (s.len() == 64 && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')))
        .then(|| s.to_owned())
}

/// `validRange` from `validateProjectIndex`.
fn valid_range(record: Option<&Value>, index_offset: u64) -> Option<(u64, u64, String)> {
    let record = record?;
    if !record.is_truthy() {
        return None;
    }
    let offset = safe_integer(record.get("offset"))?;
    let length = safe_integer(record.get("length"))?;
    let limit = index_offset as f64;
    if offset < 5.0 || length < 0.0 || offset > limit || length > limit - offset {
        return None;
    }
    let sha256 = is_hash(record.get("sha256")?)?;
    Some((offset as u64, length as u64, sha256))
}

fn read_pmv4<R: RandomAccess + ?Sized>(
    source: &R,
    size: u64,
    options: ReadOptions,
) -> Result<Container, ContainerError> {
    // readProjectFooter
    if size < PMV4_FOOTER_BYTES {
        return Err(BAD_COMMIT);
    }
    let footer = read_vec(source, size - PMV4_FOOTER_BYTES, PMV4_FOOTER_BYTES)?;
    if footer[..8] != *PMV4_END_MAGIC {
        return Err(BAD_COMMIT);
    }
    let offset = f64::from_le_bytes(footer[8..16].try_into().expect("8 bytes"));
    let length = u32::from_le_bytes(footer[16..20].try_into().expect("4 bytes")) as u64;
    let offset_ok = offset.fract() == 0.0 && offset.abs() <= MAX_SAFE_INTEGER && offset >= 5.0;
    if !offset_ok
        || size as f64 > MAX_SAFE_INTEGER
        || !(2..=MAX_INDEX_BYTES).contains(&length)
        || offset as u64 + length != size - PMV4_FOOTER_BYTES
    {
        return Err(BAD_INDEX);
    }
    let index_offset = offset as u64;
    let index_digest = crate::hash::hex(footer[32..64].try_into().expect("32 bytes"));
    let raw = read_vec(source, index_offset, length)?;
    if crate::hash::sha256_hex(&raw) != index_digest {
        return Err(BAD_INDEX_CHECKSUM);
    }
    let index = json::parse_file_text(&raw)?;

    // validateProjectIndex
    if index.get("version") != Some(&Value::Number(4.0)) {
        return Err(BAD_INDEX);
    }
    let document = valid_range(index.get("document"), index_offset)
        .filter(|(_, length, _)| *length >= 2)
        .ok_or(BAD_INDEX)?;
    let Some(items) = index.get("media").and_then(Value::as_array) else {
        return Err(BAD_INDEX);
    };
    if items.len() > MAX_MEDIA_RECORDS {
        return Err(BAD_INDEX);
    }
    let mut seen = std::collections::BTreeSet::new();
    let mut media = Vec::with_capacity(items.len());
    for item in items {
        let (offset, length, sha256) =
            valid_range(Some(item), index_offset).ok_or(BAD_PMV4_MEDIA)?;
        let (
            Some(Value::String(id)),
            Some(Value::String(revision)),
            Some(Value::String(media_type)),
        ) = (item.get("id"), item.get("revision"), item.get("type"))
        else {
            return Err(BAD_PMV4_MEDIA);
        };
        let id_len = id.utf16_len();
        let revision_len = revision.utf16_len();
        if id_len == 0
            || id_len > 1000
            || seen.contains(id)
            || revision_len == 0
            || revision_len > 200
            || media_type.utf16_len() > 1000
        {
            return Err(BAD_PMV4_MEDIA);
        }
        seen.insert(id.clone());
        media.push(MediaRecord {
            id: id.clone(),
            media_type: media_type.clone(),
            offset,
            length,
            revision: Some(revision.clone()),
            sha256: Some(sha256),
        });
    }
    let mut ranges: Vec<(u64, u64)> = std::iter::once((document.0, document.1))
        .chain(media.iter().map(|m| (m.offset, m.length)))
        .filter(|(_, length)| *length > 0)
        .collect();
    ranges.sort_by_key(|(offset, _)| *offset);
    if ranges.windows(2).any(|w| w[1].0 < w[0].0 + w[0].1) {
        return Err(OVERLAP);
    }

    let bytes = read_vec(source, document.0, document.1)?;
    if crate::hash::sha256_hex(&bytes) != document.2 {
        return Err(BAD_CONTENT_CHECKSUM);
    }
    let document = json::parse_file_text(&bytes)?;
    let container = Container {
        format: ContainerFormat::Pmv4,
        document,
        media,
    };
    if options.verify_media {
        for record in &container.media {
            container.verify_media(source, record)?;
        }
    }
    Ok(container)
}

impl Container {
    /// The media bytes of `record` from an in-memory file.
    pub fn media_bytes<'a>(&self, file: &'a [u8], record: &MediaRecord) -> Option<&'a [u8]> {
        let start = usize::try_from(record.offset).ok()?;
        let end = start.checked_add(usize::try_from(record.length).ok()?)?;
        file.get(start..end)
    }

    /// Checks a PMV4 media checksum (PMV3 records carry none and always pass).
    pub fn verify_media<R: RandomAccess + ?Sized>(
        &self,
        source: &R,
        record: &MediaRecord,
    ) -> Result<(), ContainerError> {
        match &record.sha256 {
            Some(expected)
                if sha256_of_range(source, record.offset, record.length)? != *expected =>
            {
                Err(BAD_CONTENT_CHECKSUM)
            }
            _ => Ok(()),
        }
    }

    /// The project inside the document (`document.proj || document`).
    pub fn project(&self) -> &Value {
        crate::canonical::project_of(&self.document)
    }
}
