//! Read main's `Media Store/Index` records from an immutable checkpoint. Never
//! dereference external source links here: cloud availability belongs to the
//! platform adapter. Owned payloads are bounded, streamed and checksum verified.
use crate::snapshot::open_regular;
use anyhow::{Result, ensure};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{Read, Seek, SeekFrom},
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
};

/// Must perform metadata-only checks. Returning false must never materialize a
/// cloud placeholder. Security-scoped access, when required, stays alive through
/// the returned file's use in the application host.
pub trait SourceAccess {
    fn resident(&mut self, path: &Path) -> Result<bool>;
}

/// Local metadata adapter. It never requests a cloud download. A sandboxed host
/// can wrap this with its security-scoped access capability.
pub struct ResidentSources;
impl SourceAccess for ResidentSources {
    fn resident(&mut self, path: &Path) -> Result<bool> {
        let metadata = match fs::symlink_metadata(path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
            Err(error) => return Err(error.into()),
        };
        #[cfg(target_os = "macos")]
        {
            use std::os::macos::fs::MetadataExt;
            // Darwin SF_DATALESS, as used by first-launch's checkpoint reader.
            if metadata.st_flags() & 0x4000_0000 != 0 {
                return Ok(false);
            }
        }
        ensure!(metadata.is_file(), "source link is not a regular file");
        Ok(true)
    }
}

fn same_file(a: &fs::Metadata, b: &fs::Metadata) -> bool {
    a.dev() == b.dev()
        && a.ino() == b.ino()
        && a.len() == b.len()
        && a.mtime() == b.mtime()
        && a.mtime_nsec() == b.mtime_nsec()
        && a.ctime() == b.ctime()
        && a.ctime_nsec() == b.ctime_nsec()
}

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

    pub fn verify_owned(&self) -> Result<()> {
        if self.owned_path.is_some() {
            self.open_verified(&mut ResidentSources)?;
        }
        Ok(())
    }

    /// Full digest verification even when mtime matches the record (N3). The
    /// caller streams this descriptor, never reopens an unverified path. None
    /// means an offline source link; owned bytes never silently become offline.
    pub fn open_verified(&self, access: &mut impl SourceAccess) -> Result<Option<fs::File>> {
        let path = if let Some(path) = &self.owned_path {
            path
        } else if let Some(path) = &self.source_path {
            if !access.resident(path)? {
                return Ok(None);
            }
            path
        } else {
            anyhow::bail!("media has no payload identity");
        };
        let before = fs::symlink_metadata(path)?;
        ensure!(before.is_file(), "media is not a regular file");
        let mut file = open_regular(path)?;
        ensure!(
            same_file(&before, &file.metadata()?),
            "media replaced before open"
        );
        ensure!(before.len() == self.size, "media length changed");
        let mut hash = Sha256::new();
        let mut total = 0u64;
        let mut buffer = vec![0; 1 << 20];
        loop {
            let n = file.read(&mut buffer)?;
            if n == 0 {
                break;
            }
            total += n as u64;
            ensure!(total <= self.size, "media expanded during verification");
            hash.update(&buffer[..n]);
        }
        ensure!(
            total == self.size
                && format!("{:x}", hash.finalize()) == self.sha256
                && same_file(&before, &file.metadata()?)
                && same_file(&before, &fs::symlink_metadata(path)?),
            "media identity or checksum changed"
        );
        file.seek(SeekFrom::Start(0))?;
        Ok(Some(file))
    }
}

/// Resolve one stable key without scanning or opening unrelated payloads.
/// Metadata-only access never dereferences source links or reads media bytes.
pub fn read_record(root: &Path, key: &str) -> Result<Option<DiskMediaRecord>> {
    let index = root.join("Index");
    if !index.exists() {
        return Ok(None);
    }
    ensure!(
        fs::symlink_metadata(&index)?.is_dir(),
        "unsafe media index directory"
    );
    let name = format!("{:x}.json", Sha256::digest(key.as_bytes()));
    let input = match open_regular(&index.join(name)) {
        Ok(input) => input,
        Err(error)
            if error
                .downcast_ref::<std::io::Error>()
                .is_some_and(|e| e.kind() == std::io::ErrorKind::NotFound) =>
        {
            return Ok(None);
        }
        Err(error) => return Err(error),
    };
    ensure!(
        input.metadata()?.len() <= 1024 * 1024,
        "media metadata too large"
    );
    let record =
        DiskMediaRecord::decode(serde_json::from_reader(input.take(1024 * 1024 + 1))?, root)?;
    ensure!(record.key == key, "media key index mismatch");
    Ok(Some(record))
}

pub fn read_index(root: &Path) -> Result<Vec<DiskMediaRecord>> {
    let index = root.join("Index");
    if !index.exists() {
        return Ok(Vec::new());
    }
    ensure!(
        fs::symlink_metadata(&index)?.is_dir(),
        "unsafe media index directory"
    );
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
            input.metadata()?.len() <= 1024 * 1024,
            "media metadata too large"
        );
        let raw = serde_json::from_reader(input.take(1024 * 1024 + 1))?;
        let record = DiskMediaRecord::decode(raw, root)?;
        ensure!(
            format!("{:x}", Sha256::digest(record.key.as_bytes())) == hash,
            "media key index mismatch"
        );
        record.verify_owned()?;
        records.push(record);
    }
    records.sort_by(|a, b| a.key.cmp(&b.key));
    Ok(records)
}
