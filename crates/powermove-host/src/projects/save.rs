//! One incremental project save (port of `main/project-save.ts`).
//!
//! A bounded upload writes directly into a destination-side transaction file.
//! Reused ranges come only from the host's verified previous container.

use std::collections::{HashMap, VecDeque};
use std::fs::{self, File, OpenOptions};
use std::path::{Path, PathBuf};

use anyhow::{Result, bail};
use serde_json::Value;
use sha2::{Digest, Sha256};

use super::durable::{FileVersion, check_version, copy_snapshot, hash_range, read_exactly, temporary_sibling, write_all_at};
use super::format::{INCREMENTAL_MAGIC, IncrementalMedia, MAX_SAFE_INTEGER, ProjectIndex, ProjectRecord, SaveMedia, is_incremental_project, project_footer, read_incremental_index, sha256_hex, validate_project_document};
use crate::ids::FILE_CHUNK_BYTES;

pub type Commit = Box<dyn FnOnce(&Path, String) -> Result<String> + Send>;
pub type Release = Box<dyn FnOnce() + Send>;

struct Incoming {
    id: Option<String>,
    length: u64,
    received: u64,
    offset: u64,
    digest: Sha256,
    source: Option<SaveMedia>,
}

pub struct ProjectSave {
    pub required: Vec<String>,
    records: Vec<IncrementalMedia>,
    incoming: VecDeque<Incoming>,
    document: Option<ProjectRecord>,
    offset: u64,
    digest: Sha256,
    file: Option<File>,
    ended: bool,
    failed: bool,
    temporary: PathBuf,
    commit: Option<Commit>,
    release: Option<Release>,
}

impl ProjectSave {
    /// `reuse_previous` is false for Save As: a chosen file's old media is
    /// never trusted, only the project's own known file.
    pub fn create(
        destination: &Path,
        previous: Option<&FileVersion>,
        media: Vec<SaveMedia>,
        document_bytes: u64,
        commit: Commit,
        release: Release,
        reuse_previous: bool,
    ) -> Result<Self> {
        if !(2..=0xffff_ffff).contains(&document_bytes) {
            release();
            bail!("Invalid project document size");
        }
        let mut save = Self {
            required: Vec::new(),
            records: Vec::new(),
            incoming: VecDeque::new(),
            document: None,
            offset: 0,
            digest: Sha256::new(),
            file: None,
            ended: false,
            failed: false,
            temporary: temporary_sibling(destination),
            commit: Some(commit),
            release: Some(release),
        };
        // On error `save` drops, which removes the transaction file and releases.
        save.prepare(destination, previous, media, document_bytes, reuse_previous)?;
        Ok(save)
    }

    fn prepare(&mut self, destination: &Path, previous: Option<&FileVersion>, media: Vec<SaveMedia>, document_bytes: u64, reuse_previous: bool) -> Result<()> {
        let mut source = None;
        let mut index = None;
        if let Some(previous) = previous.filter(|_| reuse_previous) {
            let file = File::open(destination)?;
            let size = previous.info.size;
            if is_incremental_project(&read_exactly(&file, 0, size.min(5))?) {
                index = Some(read_incremental_index(size, |offset, length| read_exactly(&file, offset, length))?);
            }
            source = Some(file);
        }
        let mut available: HashMap<String, IncrementalMedia> = index.as_ref().map(|index| index.media.iter().map(|item| (item.id.clone(), item.clone())).collect()).unwrap_or_default();
        for item in &media {
            match available.remove(&item.id) {
                Some(saved) if saved.revision == item.revision && saved.length == item.length => {
                    self.records.push(IncrementalMedia { kind: item.kind.clone(), ..saved });
                }
                _ => self.required.push(item.id.clone()),
            }
        }
        let live: u64 = self.records.iter().map(|item| item.length).sum();
        let dead = previous.map_or(0, |previous| previous.info.size as i128) - live as i128;
        // Compact only when meaningful dead data accumulates. No unbounded log
        // growth; Undo-reachable media remains in the incoming manifest.
        let compact = index.is_none() || dead > (16 * 1024 * 1024) as i128 && dead * 2 > live as i128;
        match previous.filter(|_| !compact) {
            Some(previous) => {
                copy_snapshot(destination, &self.temporary)?;
                self.file = Some(OpenOptions::new().read(true).write(true).open(&self.temporary)?);
                self.offset = previous.info.size;
                self.digest = previous.digest.clone();
            }
            None => {
                self.file = Some(OpenOptions::new().read(true).write(true).create_new(true).open(&self.temporary)?);
                self.append(INCREMENTAL_MAGIC)?;
                let mut records = std::mem::take(&mut self.records);
                for item in &mut records {
                    let source = source.as_ref().expect("reused media comes from the previous file");
                    let new_offset = self.offset;
                    let mut digest = Sha256::new();
                    let mut copied = 0;
                    while copied < item.length {
                        let bytes = read_exactly(source, item.offset + copied, (FILE_CHUNK_BYTES as u64).min(item.length - copied))?;
                        digest.update(&bytes);
                        self.append(&bytes)?;
                        copied += bytes.len() as u64;
                    }
                    if hex::encode(digest.finalize()) != item.sha256 {
                        bail!("The saved media checksum does not match.");
                    }
                    item.offset = new_offset;
                }
                self.records = records;
            }
        }
        check_version(destination, previous)?;
        let required: std::collections::HashSet<&str> = self.required.iter().map(String::as_str).collect();
        self.incoming.push_back(Incoming { id: None, length: document_bytes, received: 0, offset: self.offset, digest: Sha256::new(), source: None });
        for item in media.into_iter().filter(|item| required.contains(item.id.as_str())) {
            self.incoming.push_back(Incoming { id: Some(item.id.clone()), length: item.length, received: 0, offset: 0, digest: Sha256::new(), source: Some(item) });
        }
        Ok(())
    }

    fn append(&mut self, bytes: &[u8]) -> Result<()> {
        if self.offset + bytes.len() as u64 > MAX_SAFE_INTEGER {
            bail!("The project exceeds the filesystem numeric range.");
        }
        write_all_at(self.file.as_ref().expect("transaction file is open"), bytes, self.offset)?;
        self.digest.update(bytes);
        self.offset += bytes.len() as u64;
        Ok(())
    }

    fn finish_records(&mut self) {
        while self.incoming.front().is_some_and(|item| item.received == item.length) {
            let item = self.incoming.pop_front().unwrap();
            let record = ProjectRecord { offset: if item.length > 0 { item.offset } else { self.offset }, length: item.length, sha256: hex::encode(item.digest.finalize()) };
            match item.source {
                Some(source) => self.records.push(IncrementalMedia { id: source.id, revision: source.revision, kind: source.kind, length: record.length, offset: record.offset, sha256: record.sha256 }),
                None => self.document = Some(record),
            }
            if let Some(next) = self.incoming.front_mut() {
                next.offset = self.offset;
            }
        }
    }

    /// Append the next chunk of the document (`id == None`) or a required asset.
    pub fn write(&mut self, id: Option<&str>, data: &[u8]) -> Result<()> {
        if self.ended || self.failed {
            bail!("A project write is already in progress or has ended.");
        }
        self.finish_records();
        let valid = self.incoming.front().is_some_and(|target| {
            target.id.as_deref() == id && !data.is_empty() && data.len() <= FILE_CHUNK_BYTES && data.len() as u64 <= target.length - target.received
        });
        if !valid {
            bail!("Invalid project save chunk");
        }
        if let Err(error) = self.append(data) {
            self.failed = true;
            return Err(error);
        }
        let target = self.incoming.front_mut().unwrap();
        target.digest.update(data);
        target.received += data.len() as u64;
        self.finish_records();
        Ok(())
    }

    /// Seal the index and footer, then publish through the commit callback.
    pub fn finish(&mut self) -> Result<String> {
        if self.ended || self.failed {
            bail!("The project save is incomplete.");
        }
        self.finish_records();
        let Some(document) = self.document.clone().filter(|_| self.incoming.is_empty()) else {
            bail!("The project save is incomplete.");
        };
        let result = self.seal(&document);
        self.dispose();
        result
    }

    fn seal(&mut self, document: &ProjectRecord) -> Result<String> {
        let bytes = read_exactly(self.file.as_ref().unwrap(), document.offset, document.length)?;
        validate_project_document(&serde_json::from_slice::<Value>(&bytes)?)?;
        let index = ProjectIndex { version: 4, document: document.clone(), media: std::mem::take(&mut self.records) };
        let bytes = serde_json::to_vec(&index)?;
        let footer = project_footer(self.offset, bytes.len() as u64, &sha256_hex(&bytes))?;
        self.append(&bytes)?;
        self.append(&footer)?;
        let file = self.file.take().unwrap();
        file.sync_all()?;
        drop(file);
        let commit = self.commit.take().expect("a save commits once");
        commit(&self.temporary, hex::encode(self.digest.clone().finalize()))
    }

    pub fn dispose(&mut self) {
        if self.ended {
            return;
        }
        self.ended = true;
        self.file = None;
        let _ = fs::remove_file(&self.temporary);
        if let Some(release) = self.release.take() {
            release();
        }
    }
}

impl Drop for ProjectSave {
    fn drop(&mut self) {
        self.dispose();
    }
}

/// Hash a range of an open file (used to verify media on open).
pub fn range_sha256(file: &File, offset: u64, length: u64) -> Result<String> {
    Ok(hex::encode(hash_range(file, offset, Some(length))?.finalize()))
}
