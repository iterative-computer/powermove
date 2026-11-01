//! Immutable generations: a failed publication cannot destroy a previous value.
use anyhow::{Context, Result, bail, ensure};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
};

pub const MAX_SETTING_BYTES: usize = 32 * 1024 * 1024;

pub(crate) fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

pub(crate) fn component(value: &str) -> Result<()> {
    ensure!(
        !value.is_empty()
            && value.len() <= 128
            && value
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_'),
        "invalid identifier"
    );
    Ok(())
}

/// A private, explicitly supplied service directory. Never discovers HOME/userData.
/// Keep this capability on one worker; the advisory lock fences other instances.
pub struct ServiceStorage {
    root: PathBuf,
    _lock: File,
}

impl ServiceStorage {
    /// Creates a fresh directory or reopens one previously created by this service.
    /// The caller must provide a disposable test path or an authorized native directory.
    pub fn open(root: impl AsRef<Path>) -> Result<Self> {
        let root = root.as_ref();
        ensure!(root.is_absolute(), "service root must be absolute");
        // Reject symlink ancestors, including a preexisting root, before creating anything.
        for path in root.ancestors() {
            if let Ok(meta) = fs::symlink_metadata(path) {
                ensure!(!meta.file_type().is_symlink(), "symlink service root");
            }
        }
        if root.exists() {
            ensure!(root.is_dir(), "service root is not a directory");
            reject_symlink(&root.join("pm-services-root"))?;
            ensure!(
                fs::read(root.join("pm-services-root"))? == b"pm-services-v1\n",
                "unrecognized service root"
            );
        } else {
            fs::create_dir(root).context("create explicit service directory")?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(root, fs::Permissions::from_mode(0o700))?;
            }
            let mut marker = private_file(&root.join("pm-services-root"))?;
            marker.write_all(b"pm-services-v1\n")?;
            marker.sync_all()?;
            File::open(root)?.sync_all()?;
            File::open(root.parent().context("missing root parent")?)?.sync_all()?;
        }
        let lock_path = root.join("writer.lock");
        reject_symlink(&lock_path)?;
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(lock_path)?;
        lock.try_lock_exclusive()
            .context("service directory already has a writer")?;
        Ok(Self {
            root: root.to_path_buf(),
            _lock: lock,
        })
    }

    pub(crate) fn directory(&self, namespace: &str, key: &str) -> Result<PathBuf> {
        component(namespace)?;
        let parent = self.root.join(namespace);
        reject_symlink(&parent)?;
        fs::create_dir_all(&parent)?;
        let path = parent.join(digest(key.as_bytes()));
        reject_symlink(&path)?;
        fs::create_dir_all(&path)?;
        File::open(&parent)?.sync_all()?;
        File::open(&self.root)?.sync_all()?;
        Ok(path)
    }

    /// JSON settings retain unknown keys/types and every old value, including deletions.
    pub fn setting(&self, key: &str) -> Result<Option<serde_json::Value>> {
        validate_key(key)?;
        let dir = self.directory("settings", key)?;
        let Some((_, path)) = generations(&dir)?.pop() else {
            return Ok(None);
        };
        reject_symlink(&path)?;
        let bytes = read_capped(&path, MAX_SETTING_BYTES + 4096)?;
        let record: Setting = serde_json::from_slice(&bytes)
            .context("settings need recovery; refusing an empty fallback")?;
        ensure!(
            record.key == key && record.format == 1,
            "setting identity/format mismatch"
        );
        Ok(if record.deleted {
            None
        } else {
            Some(record.value)
        })
    }

    pub fn set(&mut self, key: &str, value: serde_json::Value) -> Result<u64> {
        self.write_setting(key, Some(value))
    }

    pub fn archive_setting(&mut self, key: &str) -> Result<u64> {
        self.write_setting(key, None)
    }

    fn write_setting(&mut self, key: &str, value: Option<serde_json::Value>) -> Result<u64> {
        validate_key(key)?;
        if let Some(value) = &value {
            ensure!(
                serde_json::to_vec(value)?.len() <= MAX_SETTING_BYTES,
                "setting exceeds 32 MiB"
            );
        }
        // Refuse to cover up corrupt latest state. Recovery is explicit.
        self.setting(key)?;
        let dir = self.directory("settings", key)?;
        let bytes = serde_json::to_vec(&Setting {
            format: 1,
            key: key.into(),
            deleted: value.is_none(),
            value: value.unwrap_or(serde_json::Value::Null),
        })?;
        self.publish(&dir, bytes.as_slice())
            .map(|(generation, _)| generation)
    }

    /// Stream an immutable backup. Caller supplies the kernel's coherent snapshot reader.
    /// No retention deletion: all generations remain available for recovery.
    pub fn backup(&mut self, project: &str, snapshot: impl Read) -> Result<Backup> {
        component(project)?;
        let dir = self.directory("backups", project)?;
        let (generation, path) = self.publish(&dir, snapshot)?;
        let (sha256, bytes) = hash_file(&path)?;
        Ok(Backup {
            project: project.into(),
            generation,
            path,
            sha256,
            bytes,
        })
    }

    pub fn backups(&self, project: &str) -> Result<Vec<Backup>> {
        component(project)?;
        generations(&self.directory("backups", project)?)?
            .into_iter()
            .rev()
            .map(|(generation, path)| {
                let (sha256, bytes) = hash_file(&path)?;
                Ok(Backup {
                    project: project.into(),
                    generation,
                    path,
                    sha256,
                    bytes,
                })
            })
            .collect()
    }

    pub(crate) fn publish(
        &mut self,
        directory: &Path,
        mut bytes: impl Read,
    ) -> Result<(u64, PathBuf)> {
        let mut generation = generations(directory)?
            .last()
            .map_or(Some(1), |(n, _)| n.checked_add(1))
            .context("generation exhausted")?;
        // Failed/incomplete staging files are archived too, never reused or deleted.
        let (staging, mut file) = loop {
            let path = directory.join(format!("{generation:020}.pending"));
            match private_file(&path) {
                Ok(file) => break (path, file),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                    generation = generation.checked_add(1).context("generation exhausted")?
                }
                Err(error) => return Err(error.into()),
            }
        };
        let mut hasher = Sha256::new();
        let mut size = 0u64;
        let mut buffer = [0; 64 * 1024];
        loop {
            let read = bytes.read(&mut buffer)?;
            if read == 0 {
                break;
            }
            file.write_all(&buffer[..read])?;
            hasher.update(&buffer[..read]);
            size = size
                .checked_add(read as u64)
                .context("payload size overflow")?;
        }
        file.sync_all()?;
        let checksum = format!("{:x} {size}", hasher.finalize());
        let mut receipt = private_file(&directory.join(format!("{generation:020}.sha256")))?;
        receipt.write_all(checksum.as_bytes())?;
        receipt.sync_all()?;
        // A published generation must never outlive its integrity receipt.
        File::open(directory)?.sync_all()?;
        let destination = directory.join(format!("{generation:020}.data"));
        // Hard-link is an atomic no-clobber publication. Keep the staging name as archive.
        fs::hard_link(&staging, &destination)?;
        File::open(directory)?.sync_all()?;
        File::open(directory.parent().context("missing parent")?)?.sync_all()?;
        Ok((generation, destination))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Backup {
    pub project: String,
    pub generation: u64,
    pub path: PathBuf,
    pub sha256: String,
    pub bytes: u64,
}

#[derive(Serialize, Deserialize)]
struct Setting {
    format: u32,
    key: String,
    deleted: bool,
    value: serde_json::Value,
}

fn validate_key(key: &str) -> Result<()> {
    ensure!(
        !key.is_empty() && key.len() <= 512 && !key.chars().any(char::is_control),
        "invalid setting key"
    );
    Ok(())
}

pub(crate) fn generations(directory: &Path) -> Result<Vec<(u64, PathBuf)>> {
    let mut entries = Vec::new();
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if let Some(number) = name.strip_suffix(".data").filter(|n| n.len() == 20) {
            let number: u64 = number.parse().context("invalid generation")?;
            ensure!(entry.file_type()?.is_file(), "non-file generation");
            entries.push((number, entry.path()));
        }
    }
    entries.sort_by_key(|(number, _)| *number);
    Ok(entries)
}

pub(crate) fn reject_symlink(path: &Path) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(meta) => ensure!(!meta.file_type().is_symlink(), "symlink service entry"),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => (),
        Err(error) => return Err(error.into()),
    }
    Ok(())
}

fn private_file(path: &Path) -> std::io::Result<File> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path)
}

pub(crate) fn read_capped(path: &Path, max: usize) -> Result<Vec<u8>> {
    reject_symlink(path)?;
    let mut bytes = Vec::new();
    File::open(path)?
        .take(max as u64 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > max {
        bail!("service payload too large")
    }
    if path
        .extension()
        .is_some_and(|extension| extension == "data")
    {
        verify_integrity(path, &digest(&bytes), bytes.len() as u64)?;
    }
    Ok(bytes)
}

pub(crate) fn hash_file(path: &Path) -> Result<(String, u64)> {
    reject_symlink(path)?;
    let mut file = File::open(path)?;
    let mut hash = Sha256::new();
    let mut buffer = [0; 64 * 1024];
    let mut size = 0;
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hash.update(&buffer[..read]);
        size += read as u64;
    }
    let hash = format!("{:x}", hash.finalize());
    verify_integrity(path, &hash, size)?;
    Ok((hash, size))
}

fn verify_integrity(path: &Path, hash: &str, size: u64) -> Result<()> {
    let receipt = read_capped(&path.with_extension("sha256"), 128)?;
    ensure!(
        receipt == format!("{hash} {size}").as_bytes(),
        "service generation integrity failure"
    );
    Ok(())
}
