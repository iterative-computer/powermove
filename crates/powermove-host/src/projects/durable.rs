//! Crash-safe file publication (port of `main/durable-file.ts` for Windows).
//!
//! There is no swap/clone on Windows: snapshots are plain copies, a first
//! save publishes with create-new semantics (hard link, then drop the
//! temporary), and an overwrite is a rename after re-checking the version.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use anyhow::{Context, Result, anyhow, bail};
use sha2::{Digest, Sha256};

use crate::ids::FILE_CHUNK_BYTES;

pub const CHANGED_FILE: &str = "The project file was moved, deleted, or changed outside Powermove. Use Save As to avoid overwriting other work.";

pub fn changed_file() -> anyhow::Error {
    anyhow!(CHANGED_FILE)
}

/// `.<name>.<uuid>.tmp` next to `destination`.
pub fn temporary_sibling(destination: &Path) -> PathBuf {
    let name = destination.file_name().map(|name| name.to_string_lossy()).unwrap_or_default();
    destination.with_file_name(format!(".{name}.{}.tmp", uuid::Uuid::new_v4()))
}

pub fn sync_directory(directory: &Path) -> io::Result<()> {
    // Windows cannot flush a directory handle; NTFS journals the rename itself.
    #[cfg(windows)]
    let _ = directory;
    #[cfg(not(windows))]
    File::open(directory)?.sync_all()?;
    Ok(())
}

/// Copy `source` to a `destination` that must not exist yet.
pub fn copy_snapshot(source: &Path, destination: &Path) -> io::Result<()> {
    let mut input = File::open(source)?;
    let mut output = OpenOptions::new().write(true).create_new(true).open(destination)?;
    let copied = io::copy(&mut input, &mut output).and_then(|_| output.sync_all());
    if copied.is_err() {
        drop(output);
        let _ = fs::remove_file(destination);
    }
    copied
}

pub fn read_at(file: &File, buffer: &mut [u8], offset: u64) -> io::Result<usize> {
    #[cfg(windows)]
    return std::os::windows::fs::FileExt::seek_read(file, buffer, offset);
    #[cfg(unix)]
    return std::os::unix::fs::FileExt::read_at(file, buffer, offset);
}

pub fn write_all_at(file: &File, mut data: &[u8], mut offset: u64) -> Result<()> {
    while !data.is_empty() {
        #[cfg(windows)]
        let written = std::os::windows::fs::FileExt::seek_write(file, data, offset)?;
        #[cfg(unix)]
        let written = std::os::unix::fs::FileExt::write_at(file, data, offset)?;
        if written == 0 {
            bail!("Could not write project data");
        }
        data = &data[written..];
        offset += written as u64;
    }
    Ok(())
}

pub fn read_exactly(file: &File, position: u64, length: u64) -> Result<Vec<u8>> {
    let mut data = vec![0; usize::try_from(length)?];
    let mut offset = 0;
    while offset < data.len() {
        let read = read_at(file, &mut data[offset..], position + offset as u64)?;
        if read == 0 {
            bail!("The project container is truncated.");
        }
        offset += read;
    }
    Ok(data)
}

/// Hash `length` bytes from `offset` (to the end of the file when `None`).
pub fn hash_range(file: &File, offset: u64, length: Option<u64>) -> Result<Sha256> {
    let mut hash = Sha256::new();
    let mut buffer = vec![0; FILE_CHUNK_BYTES];
    let mut position = offset;
    loop {
        let want = match length {
            Some(length) => (offset + length - position).min(buffer.len() as u64) as usize,
            None => buffer.len(),
        };
        if want == 0 {
            break;
        }
        let read = read_at(file, &mut buffer[..want], position)?;
        if read == 0 {
            break;
        }
        hash.update(&buffer[..read]);
        position += read as u64;
    }
    Ok(hash)
}

/// Copy everything `reader` yields into `file` and return its SHA-256.
pub fn write_hashed(file: &mut File, reader: &mut dyn Read) -> Result<String> {
    let mut hash = Sha256::new();
    let mut buffer = vec![0; FILE_CHUNK_BYTES];
    loop {
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hash.update(&buffer[..read]);
        io::Write::write_all(file, &buffer[..read])?;
    }
    Ok(hex::encode(hash.finalize()))
}

/// What identifies one version of a file without reading it. Windows has no
/// stable inode in std, so size and timestamps stand in for `sameFile`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stamp {
    pub size: u64,
    modified: Option<SystemTime>,
    created: Option<SystemTime>,
    is_file: bool,
}

impl Stamp {
    pub fn of(metadata: &fs::Metadata) -> Self {
        Self { size: metadata.len(), modified: metadata.modified().ok(), created: metadata.created().ok(), is_file: metadata.is_file() }
    }
}

#[derive(Clone)]
pub struct FileVersion {
    pub info: Stamp,
    pub hash: String,
    /// The whole file's running digest, so an append can continue it.
    pub digest: Sha256,
}

fn lstat(path: &Path) -> Result<Option<Stamp>> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => Ok(Some(Stamp::of(&metadata))),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

pub fn file_version(destination: &Path) -> Result<Option<FileVersion>> {
    let Some(info) = lstat(destination)? else { return Ok(None) };
    // Never write through or replace a symlink supplied in a destination.
    if !info.is_file {
        bail!("The destination cannot be safely backed up. Choose another file with Save As.");
    }
    let file = File::open(destination)?;
    if info != Stamp::of(&file.metadata()?) {
        return Err(changed_file());
    }
    let digest = hash_range(&file, 0, None)?;
    if info != Stamp::of(&file.metadata()?) || lstat(destination)?.as_ref() != Some(&info) {
        return Err(changed_file());
    }
    Ok(Some(FileVersion { info, hash: hex::encode(digest.clone().finalize()), digest }))
}

pub fn check_version(destination: &Path, expected: Option<&FileVersion>) -> Result<()> {
    let current = lstat(destination)?;
    let same = match expected {
        Some(expected) => current.as_ref() == Some(&expected.info),
        None => current.is_none(),
    };
    if !same {
        return Err(changed_file());
    }
    Ok(())
}

/// Publish only a flushed sibling. A new file never replaces one that
/// appeared after the check; an existing one is replaced only while it is
/// still the version the save started from.
pub fn publish_file(temporary: &Path, destination: &Path, expected: Option<&FileVersion>) -> Result<()> {
    check_version(destination, expected)?;
    if expected.is_none() {
        match fs::hard_link(temporary, destination) {
            Ok(()) => {
                let _ = fs::remove_file(temporary);
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => return Err(changed_file()),
            // Volumes without hard links (FAT, some network shares).
            Err(_) => {
                check_version(destination, None)?;
                fs::rename(temporary, destination)?;
            }
        }
    } else {
        check_version(destination, expected)?;
        fs::rename(temporary, destination)?;
    }
    sync_directory(destination.parent().unwrap_or(Path::new(".")))?;
    Ok(())
}

/// Replace `path` only after every byte has reached disk; a failed write
/// preserves the original.
pub fn atomic_write(path: &Path, reader: &mut dyn Read) -> Result<()> {
    let temporary = temporary_sibling(path);
    let result = (|| {
        let mut file = OpenOptions::new().write(true).create_new(true).open(&temporary)?;
        write_hashed(&mut file, reader)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&temporary, path)?;
        sync_directory(path.parent().context("no parent directory")?)?;
        anyhow::Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}
