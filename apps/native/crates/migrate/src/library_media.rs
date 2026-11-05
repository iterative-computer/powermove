//! Read main's `Media Store/Index` records from an immutable checkpoint. Never
//! dereference external source links here: cloud availability belongs to the
//! platform adapter. Owned payloads are bounded, streamed and checksum verified.
use crate::snapshot::open_regular;
use anyhow::{Result, ensure};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::Read,
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

    pub fn verify_owned(&self) -> Result<()> {
        if let Some(path) = &self.owned_path {
            let mut file = open_regular(path)?;
            let before = file.metadata()?;
            ensure!(before.len() == self.size, "owned media length changed");
            let mut hash = Sha256::new();
            let mut total = 0u64;
            let mut buffer = vec![0; 1 << 20];
            loop {
                let n = file.read(&mut buffer)?;
                if n == 0 {
                    break;
                }
                total += n as u64;
                hash.update(&buffer[..n]);
            }
            ensure!(
                total == self.size && format!("{:x}", hash.finalize()) == self.sha256,
                "owned media checksum changed"
            );
        }
        Ok(())
    }
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
        let raw = serde_json::from_reader(open_regular(&entry.path())?)?;
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
