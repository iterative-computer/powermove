//! Read main's `Media Store/Index` records from an immutable checkpoint. Never
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

pub fn read_index(root: &Path) -> Result<Vec<DiskMediaRecord>> {
    match fs::symlink_metadata(root) {
        Ok(m) => ensure!(m.is_dir(), "unsafe media root"),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e.into()),
    }
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
        record.verify_owned()?;
        records.push(record);
    }
    records.sort_by(|a, b| a.key.cmp(&b.key));
    Ok(records)
}
