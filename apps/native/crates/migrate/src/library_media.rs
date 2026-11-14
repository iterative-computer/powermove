//! Read main's `Media Store` records (`Index/`, or the new engine's
//! content-addressed `keys/` + `objects/`) from an immutable checkpoint. Never
//! dereference external source links here: cloud availability belongs to the
//! platform adapter. Owned payloads are bounded, streamed and checksum verified.
use crate::snapshot::open_regular;
use anyhow::{Result, ensure};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File},
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
};

#[derive(Clone, Debug)]
pub struct DiskMediaRecord {
    pub key: String,
    pub revision: String,
    pub mime: String,
    pub size: u64,
    pub sha256: String,
    pub owned_path: Option<PathBuf>,
    pub source_path: Option<PathBuf>,
    /// Complete record, including future fields. Keep with the checkpoint.
    pub raw: Value,
}
impl DiskMediaRecord {
    pub fn decode(raw: Value, root: &Path) -> Result<Self> {
        let text = |k| {
            raw.get(k)
                .and_then(Value::as_str)
                .map(str::to_owned)
                .ok_or_else(|| anyhow::anyhow!("invalid media index record"))
        };
        let key = text("key")?;
        ensure!(
            !key.is_empty()
                && key.len() <= 256
                && key
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b":._-".contains(&b)),
            "invalid media key"
        );
        let sha256 = text("sha256")?;
        ensure!(
            sha256.len() == 64
                && sha256
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)),
            "invalid media digest"
        );
        let size = raw
            .get("size")
            .and_then(Value::as_u64)
            .filter(|s| *s < (1u64 << 53))
            .ok_or_else(|| anyhow::anyhow!("invalid media size"))?;
        ensure!(
            raw.get("mtime")
                .and_then(Value::as_f64)
                .is_some_and(f64::is_finite),
            "invalid media timestamp"
        );
        let (owned_path, source_path) = if raw.get("sourcePath").is_some() {
            let source = text("sourcePath")?;
            ensure!(
                Path::new(&source).is_absolute()
                    && !source.contains('\0')
                    && source.len() <= 16_384
                    && raw.get("file").is_none(),
                "invalid source link"
            );
            (None, Some(source.into()))
        } else {
            let file = text("file")?;
            let (hash, extension) = file
                .split_once('.')
                .ok_or_else(|| anyhow::anyhow!("invalid owned media name"))?;
            ensure!(
                hash.len() == 64
                    && hash
                        .bytes()
                        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
                    && [
                        "png", "jpg", "webp", "gif", "mp4", "mov", "webm", "mp3", "wav", "ogg",
                        "flac", "bin"
                    ]
                    .contains(&extension),
                "invalid owned media name"
            );
            (Some(root.join(file)), None)
        };
        Ok(Self {
            key,
            revision: text("revision")?,
            mime: text("type")?,
            size,
            sha256,
            owned_path,
            source_path,
            raw,
        })
    }

    /// Open the exact bytes, refusing links and cloud placeholders before reading.
    /// Full verification deliberately ignores mtime as an identity shortcut.
    pub fn open_verified(&self) -> Result<File> {
        use std::os::unix::fs::MetadataExt;
        let path = self
            .owned_path
            .as_ref()
            .or(self.source_path.as_ref())
            .ok_or_else(|| anyhow::anyhow!("media has no payload"))?;
        let mut file = open_regular(path)?;
        let before = file.metadata()?;
        ensure!(before.len() == self.size, "media length changed");
        let mut hash = Sha256::new();
        let mut total = 0u64;
        let mut buffer = [0; 65536];
        loop {
            let n = file.read(&mut buffer)?;
            if n == 0 {
                break;
            }
            total += n as u64;
            ensure!(total <= self.size, "media length changed");
            hash.update(&buffer[..n]);
        }
        let after = file.metadata()?;
        let current = fs::symlink_metadata(path)?;
        let identity = |m: &fs::Metadata| {
            (
                m.dev(),
                m.ino(),
                m.len(),
                m.mtime(),
                m.mtime_nsec(),
                m.ctime(),
                m.ctime_nsec(),
            )
        };
        ensure!(
            current.is_file()
                && identity(&before) == identity(&after)
                && identity(&after) == identity(&current),
            "media identity changed"
        );
        ensure!(
            total == self.size && format!("{:x}", hash.finalize()) == self.sha256,
            "media checksum changed"
        );
        file.seek(SeekFrom::Start(0))?;
        Ok(file)
    }

    pub fn verify_owned(&self) -> Result<()> {
        if self.owned_path.is_some() {
            self.open_verified()?;
        }
        Ok(())
    }

    /// Preserve a reason for unavailable source links. Owned corruption is fatal.
    pub fn source_status(&self) -> Result<SourceStatus> {
        if self.source_path.is_none() {
            self.verify_owned()?;
            return Ok(SourceStatus::Owned);
        }
        match self.open_verified() {
            Ok(_) => Ok(SourceStatus::Verified),
            Err(e) => {
                if let Some(io) = e.downcast_ref::<std::io::Error>() {
                    return match io.kind() {
                        std::io::ErrorKind::NotFound => Ok(SourceStatus::Missing),
                        std::io::ErrorKind::PermissionDenied => Ok(SourceStatus::Denied),
                        _ => Err(e),
                    };
                }
                // The pre-open dataless check is a policy refusal, not offline success.
                if e.to_string().starts_with("source_not_local:") {
                    return Ok(SourceStatus::NotLocal);
                }
                Ok(SourceStatus::Changed)
            }
        }
    }
}

/// Metadata-only check before identifying a source link. Returns false for a
/// missing or cloud-only file; reading one would start a download.
pub fn is_resident(path: &Path) -> Result<bool> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error.into()),
    };
    #[cfg(target_os = "macos")]
    {
        use std::os::macos::fs::MetadataExt;
        // Darwin SF_DATALESS, as checked by `open_regular`.
        if metadata.st_flags() & 0x4000_0000 != 0 {
            return Ok(false);
        }
    }
    // A link that couldn't be resolved to a real file (dangling, or pointing
    // at a folder) is media that isn't here: the project keeps it offline.
    if metadata.file_type().is_symlink() {
        return Ok(false);
    }
    ensure!(metadata.is_file(), "source is not a regular file");
    Ok(true)
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SourceStatus {
    Owned,
    Verified,
    Missing,
    Denied,
    NotLocal,
    Changed,
}

/// Main's media records in `root` (`<profile>/Media Store`): the shipped
/// app's `Index/` and the content-addressed `keys/` + `objects/` store that
/// development builds with the new engine wrote. An `Index/` record wins
/// over a `keys/` record of the same key.
pub fn read_index(root: &Path) -> Result<Vec<DiskMediaRecord>> {
    match fs::symlink_metadata(root) {
        Ok(m) => ensure!(m.is_dir(), "unsafe media root"),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e.into()),
    }
    let mut records = read_main_index(root, true)?;
    let indexed: std::collections::BTreeSet<String> =
        records.iter().map(|r| r.key.clone()).collect();
    records.extend(
        read_store_keys(root)?
            .into_iter()
            .filter(|r| !indexed.contains(&r.key)),
    );
    records.sort_by(|a, b| a.key.cmp(&b.key));
    Ok(records)
}

fn read_main_index(root: &Path, verify: bool) -> Result<Vec<DiskMediaRecord>> {
    let index = root.join("Index");
    match fs::symlink_metadata(&index) {
        Ok(m) => ensure!(m.is_dir(), "unsafe media index directory"),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e.into()),
    }
    let mut records = Vec::new();
    for entry in fs::read_dir(&index)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        let Some(hash) = name.strip_suffix(".json") else {
            continue;
        };
        if hash.len() != 64
            || !hash
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            continue;
        }
        let input = open_regular(&entry.path())?;
        ensure!(
            input.metadata()?.len() <= 256 * 1024,
            "media index record too large"
        );
        let mut bytes = Vec::new();
        input.take(256 * 1024 + 1).read_to_end(&mut bytes)?;
        ensure!(bytes.len() <= 256 * 1024, "media index record too large");
        let raw = serde_json::from_slice(&bytes)?;
        let record = DiskMediaRecord::decode(raw, root)?;
        ensure!(
            format!("{:x}", Sha256::digest(record.key.as_bytes())) == hash,
            "media key index mismatch"
        );
        if verify {
            record.verify_owned()?;
        }
        records.push(record);
    }
    Ok(records)
}

fn hex64(text: &str) -> bool {
    text.len() == 64
        && text
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// The content-addressed store's records: `keys/<sha256(key)>.json` naming
/// `objects/<hash[..2]>/<hash>` by its BLAKE3 hash (the native library's own
/// layout). Each object is read once to check its name and take the SHA-256
/// the rest of the migration verifies. A record whose object is missing, the
/// wrong size or other bytes is not media this store can vouch for: it is
/// left out, and its assets stay offline as they would in the original app.
fn read_store_keys(root: &Path) -> Result<Vec<DiskMediaRecord>> {
    let keys = root.join("keys");
    match fs::symlink_metadata(&keys) {
        Ok(m) => ensure!(m.is_dir(), "unsafe media key directory"),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e.into()),
    }
    let mut records = Vec::new();
    for entry in fs::read_dir(&keys)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        let Some(name_hash) = name.strip_suffix(".json").filter(|h| hex64(h)) else {
            continue;
        };
        if let Some(record) = store_record(root, &entry.path(), name_hash) {
            records.push(record);
        }
    }
    Ok(records)
}

fn store_record(root: &Path, path: &Path, name_hash: &str) -> Option<DiskMediaRecord> {
    let input = open_regular(path).ok()?;
    let mut bytes = Vec::new();
    input.take(256 * 1024 + 1).read_to_end(&mut bytes).ok()?;
    if bytes.len() > 256 * 1024 {
        return None;
    }
    let raw: Value = serde_json::from_slice(&bytes).ok()?;
    let text = |k: &str| raw.get(k).and_then(Value::as_str);
    let key = text("key")?.to_owned();
    let valid_key = !key.is_empty()
        && key.len() <= 256
        && key
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b":._-".contains(&b));
    if !valid_key || format!("{:x}", Sha256::digest(key.as_bytes())) != name_hash {
        return None;
    }
    let hash = text("hash").filter(|h| hex64(h))?.to_owned();
    let size = raw
        .get("size")
        .and_then(Value::as_u64)
        .filter(|s| *s < (1u64 << 53))?;
    let owned = root.join("objects").join(&hash[..2]).join(&hash);
    let mut object = open_regular(&owned).ok()?;
    if object.metadata().ok()?.len() != size {
        return None;
    }
    let (mut blake, mut sha) = (blake3::Hasher::new(), Sha256::new());
    let mut buffer = vec![0; 1024 * 1024];
    let mut total = 0u64;
    loop {
        let n = object.read(&mut buffer).ok()?;
        if n == 0 {
            break;
        }
        total += n as u64;
        blake.update(&buffer[..n]);
        sha.update(&buffer[..n]);
    }
    if total != size || blake.finalize().to_hex().as_str() != hash {
        return None;
    }
    Some(DiskMediaRecord {
        revision: text("revision")
            .filter(|r| !r.is_empty())
            .unwrap_or(&hash)
            .to_owned(),
        mime: text("type")
            .unwrap_or("application/octet-stream")
            .to_owned(),
        size,
        sha256: format!("{:x}", sha.finalize()),
        owned_path: Some(owned),
        source_path: None,
        key,
        raw,
    })
}

/// Read metadata only; owned paths refer to the live, read-only source profile.
pub fn read_link_index(metadata_root: &Path, source_root: &Path) -> Result<Vec<DiskMediaRecord>> {
    let mut records = read_main_index(metadata_root, false)?;
    let owned = fs::read_dir(source_root)
        .ok()
        .into_iter()
        .flatten()
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .collect::<Vec<_>>();
    for record in &mut records {
        if let Some(path) = &record.owned_path {
            record.owned_path = path.file_name().map(|name| source_root.join(name));
        }
        if record.owned_path.is_none() {
            record.owned_path = owned
                .iter()
                .find(|p| {
                    p.file_name()
                        .and_then(|n| n.to_str())
                        .is_some_and(|n| n.starts_with(&format!("{}.", record.sha256)))
                })
                .cloned();
        }
    }
    let keys = metadata_root.join("keys");
    if keys.is_dir() {
        for entry in fs::read_dir(keys)? {
            let path = entry?.path();
            if path.extension().is_none_or(|x| x != "json") {
                continue;
            }
            let raw: Value = serde_json::from_reader(open_regular(&path)?)?;
            let Some(key) = raw.get("key").and_then(Value::as_str) else {
                continue;
            };
            if records.iter().any(|r| r.key == key) {
                continue;
            }
            let Some(hash) = raw.get("hash").and_then(Value::as_str).filter(|h| hex64(h)) else {
                continue;
            };
            let Some(size) = raw.get("size").and_then(Value::as_u64) else {
                continue;
            };
            records.push(DiskMediaRecord {
                key: key.to_owned(),
                size,
                sha256: String::new(),
                revision: raw
                    .get("revision")
                    .and_then(Value::as_str)
                    .unwrap_or(hash)
                    .into(),
                mime: raw
                    .get("type")
                    .and_then(Value::as_str)
                    .unwrap_or("application/octet-stream")
                    .into(),
                owned_path: Some(source_root.join("objects").join(&hash[..2]).join(hash)),
                source_path: None,
                raw,
            });
        }
    }
    Ok(records)
}
