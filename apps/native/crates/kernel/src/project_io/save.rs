//! The PMV4 writer: a port of the old `ProjectSave` and `ProjectFiles.commit`.
//!
//! Sources: `main:apps/desktop/src/main/project-save.ts`, `project-files.ts`,
//! `durable-file.ts`, `src/shared/project-incremental.ts`.
//!
//! # File layout (what the old writer produced; the reader accepts any order)
//!
//! ```text
//! "PMV4\n"
//! [reused media records copied from the previous file]   (compaction only)
//! document record   {"v":..,"proj":..,"history":..,"ws":..}
//! new media records, in media-table order
//! index   {"version":4,"document":{offset,length,sha256},
//!          "media":[{id,revision,type,length,offset,sha256}, ...]}
//! 64-byte footer: "PMV4END\n", f64 LE index offset @8,
//!                 u32 LE index length @16, zeros, SHA-256 of the index @32
//! ```
//!
//! An in-place save of a PMV4 file whose dead bytes are small does what the
//! old app did: it *clones* the file (APFS `clonefile` via `fs::copy`),
//! checks the clone hashes to the version we last saw, and appends only the
//! new document, new media, index and footer. Unchanged media keeps its
//! offsets and costs no I/O. Otherwise (first save, Save As, a JSON/PMV3
//! source, or dead bytes over `max(16 MiB, live / 2)`) it *compacts*: a new
//! file with reused records first. `tests/ts_writer_parity.rs` checks both
//! layouts byte for byte against files written by the TypeScript writer.
//!
//! # Guarantees
//!
//! * **Streaming.** A sibling temp file (`.<name>.<uuid>.tmp`, exclusive,
//!   mode 0600, published with the replaced file's mode) is written in 1 MiB steps; checksums are computed as bytes
//!   pass. Media is copied range to range, never loaded whole.
//! * **Durable.** `F_FULLFSYNC`, then a read-back verification
//!   ([`VerifyLevel`]), then an atomic publication (exchange + verification
//!   of the displaced file; on a volume without an exchange the old file is
//!   moved aside and the new one installed exclusively; a new file is
//!   installed exclusively), then a directory sync.
//! * **A failed save changes nothing.** Disk full, a vanished or corrupt
//!   media source, failed verification, or a conflicting change on disk all
//!   remove the temp file and leave the previous file byte-identical.
//! * **Stale-write protection.** Replacing needs the exact [`FileVersion`]
//!   we last read or wrote, checked before writing, before publishing, and
//!   again on the displaced file after the exchange.
//! * **Dirty only for what was written.** A plan freezes one document
//!   revision; [`DocumentStore::finish_save`](crate::DocumentStore::finish_save)
//!   marks exactly that state saved.
//!
//! # Differences from the old app
//!
//! * The document's keys are in canonical (UTF-16) order; the old app used
//!   insertion order. Object order is otherwise invisible to its readers, but
//!   `assets` is a list to users (`Object.values` in the Project panel), so it
//!   is written in authored order; see [`crate::asset_order`]. Other objects
//!   whose order an extension reads still come back sorted.
//! * Reused index records are written with the key order of new ones; the
//!   old app kept the previous index's key order.
//! * The media order follows the media table (file order, then imports);
//!   the old app followed `fileAssets` insertion order.
//! * The project-file registry journal, macOS Versions snapshot, and the
//!   renderer upload protocol are not ported (the app shell owns file
//!   associations; there is no renderer upload in a native app).

#[path = "file_guard.rs"]
mod file_guard;

use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{self, BufWriter, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use pm_model::container::{
    ContainerFormat, MAX_INDEX_BYTES, MAX_MEDIA_RECORDS, PMV4_END_MAGIC, PMV4_FOOTER_BYTES,
    PMV4_MAGIC, ReadOptions, read_container,
};
use pm_model::json::write::{Sink, push_quoted};
use pm_model::{Document, JsString, Map, Value};
use sha2::{Digest, Sha256};

use super::durable::{self, BackupPolicy, FileVersion, Publish, TempFile, hex, lstat_file};
use super::envelope::{self, DocumentParts};
use super::{MediaEntry, MediaSource, MediaTable};
use crate::error::{ProjectIoError, io_err};
use crate::history::HistorySnapshot;

const CHUNK: usize = 1 << 20;
/// `Math.max(16 * 1024 * 1024, liveBytes / 2)`.
const COMPACT_MIN_DEAD_BYTES: u64 = 16 * 1024 * 1024;

/// Where a save goes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SaveTarget {
    /// Replace the file this project came from (needs a path).
    InPlace,
    /// Save As. `replace` is the user's consent to overwrite an existing
    /// file (the save sheet asked); without it an existing file is an error.
    /// Save As never reuses ranges of the destination, as in the old app.
    SaveAs { path: PathBuf, replace: bool },
}

/// How much of the finished temp file is read back before publishing.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum VerifyLevel {
    /// Trust the write.
    None,
    /// Re-read index, footer and document with the production reader, check
    /// the project shape and the index, and re-hash every media byte written
    /// by *this* save (not ranges kept from a clone).
    #[default]
    Written,
    /// [`VerifyLevel::Written`] plus every reused range.
    Full,
}

/// Room to leave beyond the file's own size when a full disk stops a save.
const RECLAIM_MARGIN: u64 = 64 << 20;

/// Test hooks that simulate failures.
#[doc(hidden)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WriteFault {
    /// Writes fail with "No space left on device" once this many bytes were
    /// written by the save.
    DiskFullAfter(u64),
    /// [`DiskFullAfter`](Self::DiskFullAfter) for the first attempt only.
    DiskFullOnce(u64),
    /// Everything is written and synced, then the save fails before publishing.
    FailBeforePublish,
}

/// Knobs of [`SavePlan::run`].
#[derive(Clone, Debug, Default)]
pub struct SaveOptions {
    /// Project export embeds admitted source links. Ordinary Save/Save As
    /// leaves links external, including sources that are currently offline.
    /// Unavailable links stay in the document and are listed on the receipt;
    /// changed/corrupt admitted sources still fail verification.
    pub portable: bool,
    pub verify: VerifyLevel,
    /// Keep the replaced version in a backup directory first (the old
    /// app's `Backups/<projectId>/<timestamp>.pmv` with retention).
    pub backups: Option<BackupPolicy>,
    /// Override of the 16 MiB compaction floor (tests).
    #[doc(hidden)]
    pub compact_min_dead_bytes: Option<u64>,
    #[doc(hidden)]
    pub fault: Option<WriteFault>,
    #[doc(hidden)]
    pub hooks: durable::PublishHooks,
    /// The profile folder of the instance that saves: a save's record names
    /// who is saving.
    pub profile: Option<PathBuf>,
    /// Asked for a coordinated write around the publication.
    pub coordinator: Option<durable::SharedWriteCoordinator>,
}

/// The `history` member a plan writes.
#[derive(Clone, Debug)]
pub(crate) enum HistoryToWrite {
    /// This session's stack (imported records included), exported at run time.
    Session(HistorySnapshot),
    /// The member exactly as read (it could not be imported and nothing was
    /// recorded since).
    Raw(Value),
}

/// A save frozen at one document revision. `Send + 'static`: build it on the
/// document's thread ([`DocumentStore::prepare_save`](crate::DocumentStore::prepare_save)),
/// [`run`](SavePlan::run) it on a worker, hand the result to
/// [`DocumentStore::finish_save`](crate::DocumentStore::finish_save).
#[derive(Clone)]
pub struct SavePlan {
    pub(crate) container_index: Option<Value>,
    pub(crate) live_media: std::collections::HashSet<String>,
    pub(crate) source_links: std::collections::HashSet<JsString>,
    /// The document revision being written.
    pub revision: u64,
    /// Identity of the document session (guards `finish_save`).
    pub doc_id: u64,
    pub(crate) state_token: u64,
    pub(crate) outside_writes: u64,
    pub(crate) document: Arc<Document>,
    pub(crate) had_comps: bool,
    pub(crate) history: HistoryToWrite,
    pub(crate) workspace: Option<Value>,
    pub(crate) extra: Map,
    pub(crate) media: Vec<MediaEntry>,
    pub(crate) destination: PathBuf,
    /// Resolved at plan time for in-place saves; for Save As at run time
    /// (hashing an existing destination is I/O).
    pub(crate) publish: Option<Publish>,
    /// Save As consent to overwrite an existing destination.
    pub(crate) replace_existing: bool,
    /// In-place saves may reuse the destination's ranges (`reusePrevious`).
    pub(crate) reuse_previous: bool,
    pub options: SaveOptions,
    /// Alive while a plan or its receipt exists: the store refuses a second
    /// concurrent save to the same document.
    pub(crate) ticket: Arc<()>,
}

impl std::fmt::Debug for SavePlan {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SavePlan")
            .field("revision", &self.revision)
            .field("destination", &self.destination)
            .field("media", &self.media.len())
            .field("reuse_previous", &self.reuse_previous)
            .finish()
    }
}

/// How the file was laid out.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SaveLayout {
    /// A new file (first save, Save As, compaction).
    Compacted,
    /// The previous file cloned, then new records appended.
    Appended,
}

/// What a successful save produced.
#[derive(Clone, Debug)]
pub struct SaveReceipt {
    /// Export receipts must not replace the live document's association.
    pub portable: bool,
    /// These sources were already offline when portable export began. Callers
    /// must surface incomplete media closure rather than claiming completeness.
    pub unavailable_links: Vec<JsString>,
    pub path: PathBuf,
    /// The document revision that is now on disk.
    pub revision: u64,
    pub doc_id: u64,
    pub(crate) state_token: u64,
    pub(crate) outside_writes: u64,
    pub size: u64,
    pub layout: SaveLayout,
    /// SHA-256 of the whole file.
    pub file_hash: String,
    /// SHA-256 of the document record.
    pub document_sha256: String,
    /// The published file (guard for the next save).
    pub version: Arc<FileVersion>,
    /// Every record of the new file, as ranges of it.
    pub media: MediaTable,
    /// Bytes of media kept without being written again (append layout) or
    /// copied from the previous file's ranges (compaction).
    pub reused_media_bytes: u64,
    /// The replaced version's backup, when a [`BackupPolicy`] was given.
    pub backup: Option<PathBuf>,
    /// Another version found at the destination in the exchange (it arrived
    /// after the last check, or cannot be read), kept beside the project
    /// under this name. The person must be told.
    pub displaced: Option<PathBuf>,
    /// Versions an earlier save had replaced and kept, found changed since
    /// (something wrote into them after they were replaced): kept beside
    /// the project under these names. The person must be told.
    pub changed: Vec<PathBuf>,
    /// Keeps the store's "save in progress" guard alive until adopted.
    #[allow(dead_code)]
    pub(crate) ticket: Arc<()>,
}

/// One index entry.
#[derive(Clone, Debug)]
struct Record {
    id: JsString,
    revision: JsString,
    media_type: JsString,
    offset: u64,
    length: u64,
    sha256: String,
}

struct Out<'a> {
    w: BufWriter<&'a File>,
    offset: u64,
    written: u64,
    whole: Sha256,
    limit: Option<u64>,
}

impl Out<'_> {
    fn write_all(&mut self, bytes: &[u8]) -> io::Result<()> {
        if let Some(limit) = self.limit
            && self.written + bytes.len() as u64 > limit
        {
            return Err(io::Error::new(
                io::ErrorKind::StorageFull,
                "No space left on device",
            ));
        }
        if self
            .offset
            .checked_add(bytes.len() as u64)
            .is_none_or(|end| end > (1u64 << 53))
        {
            return Err(io::Error::other(
                "The project exceeds the filesystem numeric range.",
            ));
        }
        self.w.write_all(bytes)?;
        self.whole.update(bytes);
        self.offset += bytes.len() as u64;
        self.written += bytes.len() as u64;
        Ok(())
    }
}

struct DocSink<'a, 'b> {
    out: &'a mut Out<'b>,
    hasher: Sha256,
    len: u64,
    err: Option<io::Error>,
}

impl Sink for DocSink<'_, '_> {
    fn push_str(&mut self, s: &str) {
        if self.err.is_some() {
            return;
        }
        self.hasher.update(s.as_bytes());
        self.len += s.len() as u64;
        if let Err(e) = self.out.write_all(s.as_bytes()) {
            self.err = Some(e);
        }
    }
}

#[cfg(unix)]
fn read_at(file: &File, offset: u64, buf: &mut [u8]) -> io::Result<()> {
    use std::os::unix::fs::FileExt;
    let mut done = 0;
    while done < buf.len() {
        let n = durable::read_retry(|| file.read_at(&mut buf[done..], offset + done as u64))?;
        if n == 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "The project container is truncated.",
            ));
        }
        done += n;
    }
    Ok(())
}

#[cfg(not(unix))]
fn read_at(file: &File, offset: u64, buf: &mut [u8]) -> io::Result<()> {
    use std::io::Read;
    let mut f = file;
    f.seek(SeekFrom::Start(offset))?;
    f.read_exact(buf)
}

/// `readIncrementalIndex` (footer, checksum, `validateProjectIndex`), media
/// records only.
fn read_index(file: &File, size: u64) -> Result<Vec<Record>, ProjectIoError> {
    let invalid = |m: &'static str| {
        ProjectIoError::Container(pm_model::container::ContainerError::Invalid(m))
    };
    if size < 5 + PMV4_FOOTER_BYTES {
        return Err(invalid("The project commit is truncated."));
    }
    let mut footer = [0u8; PMV4_FOOTER_BYTES as usize];
    read_at(file, size - PMV4_FOOTER_BYTES, &mut footer).map_err(|e| ProjectIoError::Io {
        path: "previous project".into(),
        source: e,
    })?;
    if footer[..8] != PMV4_END_MAGIC[..] {
        return Err(invalid("The project commit is truncated or invalid."));
    }
    let offset = f64::from_le_bytes(footer[8..16].try_into().expect("8 bytes"));
    let length = u32::from_le_bytes(footer[16..20].try_into().expect("4 bytes")) as u64;
    let integral = offset.fract() == 0.0 && (5.0..9_007_199_254_740_992.0).contains(&offset);
    if !integral
        || !(2..=MAX_INDEX_BYTES).contains(&length)
        || offset as u64 + length != size - PMV4_FOOTER_BYTES
    {
        return Err(invalid("The project index is invalid."));
    }
    let index_offset = offset as u64;
    let mut bytes = vec![0u8; length as usize];
    read_at(file, index_offset, &mut bytes).map_err(|e| ProjectIoError::Io {
        path: "previous project".into(),
        source: e,
    })?;
    if Sha256::digest(&bytes)[..] != footer[32..64] {
        return Err(invalid("The project index checksum does not match."));
    }
    let index = pm_model::json::parse_bytes(&bytes)
        .map_err(|_| invalid("The project index is invalid."))?;
    if index.get("version").and_then(Value::as_f64) != Some(4.0) {
        return Err(invalid("The project index is invalid."));
    }
    let media = index
        .get("media")
        .and_then(Value::as_array)
        .ok_or_else(|| invalid("The project index is invalid."))?;
    let int = |v: Option<&Value>| {
        v.and_then(Value::as_f64)
            .filter(|n| n.fract() == 0.0 && *n >= 0.0 && *n < 9_007_199_254_740_992.0)
            .map(|n| n as u64)
    };
    let mut out = Vec::with_capacity(media.len());
    let archived = index.get("nativeArchivedMedia").and_then(Value::as_array);
    for item in media.iter().chain(archived.into_iter().flatten()) {
        let s = |k: &str| item.get(k).and_then(Value::as_js_string).cloned();
        let (Some(id), Some(revision), Some(media_type), Some(offset), Some(length), Some(sha256)) = (
            s("id"),
            s("revision"),
            s("type"),
            int(item.get("offset")),
            int(item.get("length")),
            item.get("sha256")
                .and_then(Value::as_str)
                .map(str::to_owned),
        ) else {
            return Err(invalid("The project media index is invalid."));
        };
        if offset < 5 || offset > index_offset || length > index_offset - offset {
            return Err(invalid("The project media index is invalid."));
        }
        out.push(Record {
            id,
            revision,
            media_type,
            offset,
            length,
            sha256,
        });
    }
    Ok(out)
}

fn quoted(s: &JsString) -> String {
    let mut out = String::new();
    push_quoted(s, &mut out);
    out
}

/// `JSON.stringify(index)` with the old writer's key order.
fn index_text(doc: (u64, u64, &str), records: &[Record]) -> String {
    let mut s = format!(
        "{{\"version\":4,\"document\":{{\"offset\":{},\"length\":{},\"sha256\":\"{}\"}},\"media\":[",
        doc.0, doc.1, doc.2
    );
    for (i, r) in records.iter().enumerate() {
        if i > 0 {
            s.push(',');
        }
        s.push_str(&format!(
            "{{\"id\":{},\"revision\":{},\"type\":{},\"length\":{},\"offset\":{},\"sha256\":\"{}\"}}",
            quoted(&r.id),
            quoted(&r.revision),
            quoted(&r.media_type),
            r.length,
            r.offset,
            r.sha256
        ));
    }
    s.push_str("]}");
    s
}

/// Retain future metadata while replacing only the ranges/digests owned by
/// this writer. Preserve the existing byte layout when there are no extras.
fn retain_index_metadata(text: String, original: Option<&Value>) -> String {
    fn extras(target: &mut Value, source: &Value, known: &[&str]) -> bool {
        let (Some(target), Some(source)) = (target.as_object_mut(), source.as_object()) else {
            return false;
        };
        let mut changed = false;
        for (key, value) in source {
            if !key.as_str().is_some_and(|k| known.contains(&k)) {
                target.insert(key.clone(), value.clone());
                changed = true;
            }
        }
        changed
    }
    let Some(original) = original else {
        return text;
    };
    let mut next = pm_model::json::parse(&text).expect("generated index");
    let mut changed = extras(
        &mut next,
        original,
        &["version", "document", "media", "nativeArchivedMedia"],
    );
    if let (Some(doc), Some(old)) = (
        next.as_object_mut().unwrap().get_mut("document"),
        original.get("document"),
    ) {
        changed |= extras(doc, old, &["offset", "length", "sha256"]);
    }
    let old: Vec<_> = ["media", "nativeArchivedMedia"]
        .into_iter()
        .filter_map(|key| original.get(key).and_then(Value::as_array))
        .flatten()
        .collect();
    for field in ["media", "nativeArchivedMedia"] {
        if let Some(Value::Array(media)) = next.as_object_mut().unwrap().get_mut(field) {
            for record in media {
                if let Some(prior) = old.iter().find(|m| m.get("id") == record.get("id")) {
                    changed |= extras(
                        record,
                        prior,
                        &["id", "type", "revision", "offset", "length", "sha256"],
                    );
                }
            }
        }
    }
    if changed {
        pm_model::json::to_canonical_string(&next)
    } else {
        text
    }
}

fn index_with_archives(
    doc: (u64, u64, &str),
    records: &[Record],
    live: &std::collections::HashSet<String>,
) -> String {
    let (active, archived): (Vec<_>, Vec<_>) = records
        .iter()
        .cloned()
        .partition(|r| r.id.as_str().is_some_and(|id| live.contains(id)));
    let text = index_text(doc, &active);
    if archived.is_empty() {
        return text;
    }
    let mut next = pm_model::json::parse(&text).expect("generated index");
    let archived = pm_model::json::parse(&index_text(doc, &archived)).expect("generated index");
    next.as_object_mut().unwrap().insert(
        "nativeArchivedMedia",
        archived.get("media").unwrap().clone(),
    );
    pm_model::json::to_canonical_string(&next)
}

fn sha_of_range(file: &File, offset: u64, length: u64, buf: &mut [u8]) -> io::Result<String> {
    let mut hasher = Sha256::new();
    let mut done = 0u64;
    while done < length {
        let n = (buf.len() as u64).min(length - done) as usize;
        read_at(file, offset + done, &mut buf[..n])?;
        hasher.update(&buf[..n]);
        done += n as u64;
    }
    Ok(hex(&hasher.finalize()))
}

impl SavePlan {
    pub fn destination(&self) -> &Path {
        &self.destination
    }

    fn validate(&self) -> Result<(), ProjectIoError> {
        if self.media.len() > MAX_MEDIA_RECORDS {
            return Err(ProjectIoError::Invalid("Invalid project media".into()));
        }
        let mut seen = std::collections::HashSet::new();
        for entry in &self.media {
            let id_len = entry.id.utf16_len();
            if id_len == 0
                || id_len > 1000
                || entry.revision.utf16_len() == 0
                || entry.revision.utf16_len() > 200
                || entry.media_type.utf16_len() > 1000
                || !seen.insert(entry.id.clone())
            {
                return Err(ProjectIoError::Invalid("Invalid project media".into()));
            }
        }
        // `finish()`'s check, on the form that will be written.
        let root = self.document.root().ok_or(ProjectIoError::NotAProject)?;
        let comp = envelope::stored_comp_fields(root);
        let finite = |k: &str| matches!(comp.get(k).map(pm_model::Node::kind), Some(pm_model::NodeKind::Number(n)) if n.is_finite());
        let layers = matches!(
            comp.get("layers").map(pm_model::Node::kind),
            Some(pm_model::NodeKind::Array(_))
        );
        if !(layers && finite("w") && finite("h")) {
            return Err(ProjectIoError::NotAProject);
        }
        Ok(())
    }

    /// Writes, verifies and publishes the project. **Blocking**: run it off
    /// the UI thread. On `Err` the destination is exactly as it was (see
    /// [`ProjectIoError::Finalize`] and [`ProjectIoError::Conflict`] for the
    /// two cases that say otherwise).
    pub fn run(&self) -> Result<SaveReceipt, ProjectIoError> {
        let fault = self.options.fault;
        let error = match self.attempt(fault) {
            Err(e) if e.is_disk_full() => e,
            outcome => return outcome,
        };
        // Deleting archived generations is the last resort, not a budget: an
        // old version must not cost the user the work they are saving now.
        let needed = fs::metadata(&self.destination).map_or(0, |m| m.len()) + RECLAIM_MARGIN;
        match &self.options.backups {
            Some(policy) if durable::reclaim_archived(policy, needed) > 0 => {
                self.attempt(fault.filter(|f| !matches!(f, WriteFault::DiskFullOnce(_))))
            }
            _ => Err(error),
        }
    }

    fn attempt(&self, fault: Option<WriteFault>) -> Result<SaveReceipt, ProjectIoError> {
        self.validate()?;
        let dest = &self.destination;
        // A save of the project (its lock held), not an export to a path
        // another writer may be staging in.
        if !self.options.portable {
            durable::remove_abandoned_temps(dest);
        }
        let publish = match &self.publish {
            Some(p) => p.clone(),
            None => match FileVersion::read(dest)? {
                Some(v) if self.replace_existing => Publish::Replace(v),
                Some(_) => {
                    return Err(ProjectIoError::DestinationExists(
                        dest.display().to_string(),
                    ));
                }
                None => Publish::CreateNew,
            },
        };
        // The version we may replace must still be on disk. A touched file
        // whose bytes are proven identical is accepted; continue with its
        // current identity (the old app re-read `fileVersion` here).
        let publish = match publish {
            Publish::Replace(v) => {
                durable::check_version(dest, Some(&v), false)?;
                let stat = lstat_file(dest)?.ok_or(ProjectIoError::ChangedOnDisk)?;
                Publish::Replace(FileVersion { stat, ..v })
            }
            Publish::CreateNew => {
                durable::check_version(dest, None, false)?;
                Publish::CreateNew
            }
        };
        let previous = match &publish {
            Publish::Replace(v) => Some(v),
            Publish::CreateNew => None,
        };

        // Which records of the previous file can be kept (`ProjectSave.create`).
        let mut source: Option<File> = None;
        let mut index: Option<Vec<Record>> = None;
        if let (Some(prev), true) = (previous, self.reuse_previous) {
            let file = File::open(dest).map_err(|e| io_err(dest, e))?;
            let stat = durable::stat_of(&file.metadata().map_err(|e| io_err(dest, e))?);
            if !prev.stat.same_file(&stat) {
                return Err(ProjectIoError::ChangedOnDisk);
            }
            let mut magic = [0u8; 5];
            if prev.stat.size >= 5 && read_at(&file, 0, &mut magic).is_ok() && magic == *PMV4_MAGIC
            {
                index = Some(read_index(&file, prev.stat.size)?);
            }
            source = Some(file);
        }
        let available: HashMap<&JsString, &Record> =
            index.iter().flatten().map(|r| (&r.id, r)).collect();
        let mut reused: Vec<Record> = Vec::new();
        let mut required: Vec<&MediaEntry> = Vec::new();
        let mut unavailable_links = Vec::new();
        for entry in &self.media {
            if self.options.portable
                && self.source_links.contains(&entry.id)
                && let MediaSource::File { path, .. } = &entry.source
                && matches!(fs::symlink_metadata(path), Err(e) if e.kind() == io::ErrorKind::NotFound)
            {
                unavailable_links.push(entry.id.clone());
                continue;
            }
            match available.get(&entry.id) {
                Some(saved)
                    if saved.revision == entry.revision && saved.length == entry.length() =>
                {
                    reused.push(Record {
                        media_type: entry.media_type.clone(),
                        ..(*saved).clone()
                    });
                }
                _ => required.push(entry),
            }
        }
        let live: u64 = reused.iter().map(|r| r.length).sum();
        let dead = previous.map_or(0, |p| p.stat.size).saturating_sub(live);
        let floor = self
            .options
            .compact_min_dead_bytes
            .unwrap_or(COMPACT_MIN_DEAD_BYTES);
        let compact = index.is_none() || dead > floor.max(live / 2);

        let mut buf = vec![0u8; CHUNK];
        let (temp, file, out_offset, whole, kept_mode, layout) = if !compact {
            let prev = previous.expect("an index implies a previous file");
            let temp = TempFile::clone_of(dest, dest)?;
            #[cfg(unix)]
            let kept_mode = {
                use std::os::unix::fs::PermissionsExt;
                let mode = prev.stat.mode & 0o777;
                fs::set_permissions(&temp.path, fs::Permissions::from_mode(mode | 0o600))
                    .map_err(|e| io_err(&temp.path, e))?;
                Some(mode)
            };
            #[cfg(not(unix))]
            let kept_mode: Option<u32> = None;
            let file = fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(&temp.path)
                .map_err(|e| io_err(&temp.path, e))?;
            // Verify the staged prefix before trusting reused media.
            let digest = durable::hash_reader(&file, &temp.path)?;
            if hex(&digest.clone().finalize()) != prev.hash {
                return Err(ProjectIoError::ChangedOnDisk);
            }
            let end = (&file)
                .seek(SeekFrom::End(0))
                .map_err(|e| io_err(&temp.path, e))?;
            if end != prev.stat.size {
                return Err(ProjectIoError::ChangedOnDisk);
            }
            (temp, file, end, digest, kept_mode, SaveLayout::Appended)
        } else {
            let (temp, file) = TempFile::create(dest)?;
            // A rewrite keeps the file's mode: a shared 0644 project stays shared.
            #[cfg(unix)]
            let kept_mode = previous.map(|p| p.stat.mode & 0o777);
            #[cfg(not(unix))]
            let kept_mode: Option<u32> = None;
            (
                temp,
                file,
                0,
                Sha256::new(),
                kept_mode,
                SaveLayout::Compacted,
            )
        };
        let tmp_err = |e: io::Error| io_err(&temp.path, e);
        let limit = match fault {
            Some(WriteFault::DiskFullAfter(n) | WriteFault::DiskFullOnce(n)) => Some(n),
            _ => None,
        };
        let mut out = Out {
            w: BufWriter::with_capacity(CHUNK, &file),
            offset: out_offset,
            written: 0,
            whole,
            limit,
        };
        // Records whose bytes this save wrote (verified when `Written`).
        let mut written_records: Vec<usize> = Vec::new();
        if layout == SaveLayout::Compacted {
            out.write_all(PMV4_MAGIC).map_err(tmp_err)?;
            if let Some(src) = &source {
                for (i, record) in reused.iter_mut().enumerate() {
                    let new_offset = out.offset;
                    let mut hasher = Sha256::new();
                    let mut done = 0u64;
                    while done < record.length {
                        let n = (CHUNK as u64).min(record.length - done) as usize;
                        read_at(src, record.offset + done, &mut buf[..n])
                            .map_err(|e| io_err(dest, e))?;
                        hasher.update(&buf[..n]);
                        out.write_all(&buf[..n]).map_err(tmp_err)?;
                        done += n as u64;
                    }
                    if hex(&hasher.finalize()) != record.sha256 {
                        return Err(ProjectIoError::Media {
                            id: record.id.to_string_lossy(),
                            reason: "The saved media checksum does not match.".into(),
                        });
                    }
                    record.offset = new_offset;
                    written_records.push(i);
                }
                let stat = durable::stat_of(&src.metadata().map_err(|e| io_err(dest, e))?);
                if previous.is_some_and(|p| !p.stat.same_file(&stat)) {
                    return Err(ProjectIoError::ChangedOnDisk);
                }
            }
        }
        drop(source);
        durable::check_version(dest, previous, false)?;

        // The document record.
        let history = match &self.history {
            HistoryToWrite::Session(snapshot) => snapshot.to_saved(),
            HistoryToWrite::Raw(value) => value.clone(),
        };
        // Freeze the requirement alongside this exact document/history revision.
        // Old shipped readers ignore it; the transition refusal is specified in
        // docs/native/file-guard.md. Never advertise a new container version that
        // our own production reader cannot reopen.
        let extra = file_guard::save_extra(self.document.root(), &history, &self.extra);
        let doc_offset = out.offset;
        let mut sink = DocSink {
            out: &mut out,
            hasher: Sha256::new(),
            len: 0,
            err: None,
        };
        envelope::write_document(
            &DocumentParts {
                root: self.document.root(),
                had_comps: self.had_comps,
                history: &history,
                workspace: self.workspace.as_ref(),
                extra: &extra,
            },
            &mut sink,
        );
        if let Some(e) = sink.err.take() {
            return Err(tmp_err(e));
        }
        let doc_len = sink.len;
        let doc_sha = hex(&sink.hasher.finalize());
        drop(history);
        if !(2..=u32::MAX as u64).contains(&doc_len) {
            return Err(ProjectIoError::Invalid(
                "Invalid project document size".into(),
            ));
        }

        // New media, in table order.
        let reused_count = reused.len();
        let mut records = reused;
        for entry in required {
            let offset = out.offset;
            let (length, sha256) = write_media(entry, &mut out, &mut buf, &temp.path)?;
            written_records.push(records.len());
            records.push(Record {
                id: entry.id.clone(),
                revision: entry.revision.clone(),
                media_type: entry.media_type.clone(),
                offset,
                length,
                sha256,
            });
        }

        // Index and footer.
        let index_text = retain_index_metadata(
            index_with_archives((doc_offset, doc_len, &doc_sha), &records, &self.live_media),
            self.container_index.as_ref(),
        );
        let index_bytes = index_text.as_bytes();
        if index_bytes.len() as u64 > MAX_INDEX_BYTES {
            return Err(ProjectIoError::Invalid("Invalid project index".into()));
        }
        let index_offset = out.offset;
        out.write_all(index_bytes).map_err(tmp_err)?;
        let mut footer = [0u8; PMV4_FOOTER_BYTES as usize];
        footer[..8].copy_from_slice(PMV4_END_MAGIC);
        footer[8..16].copy_from_slice(&(index_offset as f64).to_le_bytes());
        footer[16..20].copy_from_slice(&(index_bytes.len() as u32).to_le_bytes());
        footer[32..64].copy_from_slice(&Sha256::digest(index_bytes));
        out.write_all(&footer).map_err(tmp_err)?;
        out.w.flush().map_err(tmp_err)?;
        let size = out.offset;
        let file_hash = hex(&out.whole.clone().finalize());
        drop(out);
        #[cfg(unix)]
        if let Some(mode) = kept_mode {
            use std::os::unix::fs::PermissionsExt;
            file.set_permissions(fs::Permissions::from_mode(mode))
                .map_err(tmp_err)?;
        }
        #[cfg(not(unix))]
        let _ = kept_mode;
        durable::sync_file(&file).map_err(tmp_err)?;

        self.verify(
            &file,
            &temp.path,
            &records,
            &written_records,
            &doc_sha,
            &mut buf,
        )?;
        drop(file);
        if fault == Some(WriteFault::FailBeforePublish) {
            return Err(ProjectIoError::Invalid(
                "Injected failure before publishing".into(),
            ));
        }

        // Keep the version being replaced, then publish.
        let backup = match (&self.options.backups, previous) {
            (Some(policy), Some(_)) => Some(durable::backup(policy, dest)?),
            _ => None,
        };
        let host = durable::Host {
            hooks: &self.options.hooks,
            coordinator: self.options.coordinator.as_ref(),
            profile: self.options.profile.as_deref(),
            backups: self.options.backups.as_ref(),
            backup: backup.as_deref(),
        };
        let published = durable::publish(temp, dest, &publish, &file_hash, &host);
        if published.is_err()
            && let (Some(path), Some(prev)) = (&backup, previous)
        {
            {
                let still_old = FileVersion::read(dest)
                    .ok()
                    .flatten()
                    .is_some_and(|c| c.hash == prev.hash);
                if still_old {
                    let _ = fs::remove_file(path);
                } else {
                    // An uncertain publication may still need its old version.
                    let mut recovery = path.clone().into_os_string();
                    recovery.push(".recovery.pmv");
                    let _ = fs::rename(path, recovery);
                }
            }
        }
        let published = published?;
        let backup = published.backup.or(backup);

        let stat = lstat_file(dest)?.ok_or(ProjectIoError::ChangedOnDisk)?;
        // Held from here on, so the session can follow this file if it is
        // renamed or moved (only if the path still names what was published).
        let held = File::open(dest)
            .ok()
            .filter(|f| {
                f.metadata()
                    .is_ok_and(|m| durable::stat_of(&m).same_identity(&stat))
            })
            .map(durable::HeldFile::new);
        let version = Arc::new(FileVersion {
            path: dest.clone(),
            stat,
            hash: file_hash.clone(),
            held,
        });
        let reused_media_bytes: u64 = records[..reused_count].iter().map(|r| r.length).sum();
        let mut table = MediaTable::default();
        for r in &records {
            table.insert(MediaEntry {
                id: r.id.clone(),
                media_type: r.media_type.clone(),
                revision: r.revision.clone(),
                source: MediaSource::Embedded {
                    file: version.clone(),
                    offset: r.offset,
                    length: r.length,
                    sha256: Some(r.sha256.clone()),
                },
            });
        }
        Ok(SaveReceipt {
            portable: self.options.portable,
            unavailable_links,
            path: dest.clone(),
            revision: self.revision,
            doc_id: self.doc_id,
            state_token: self.state_token,
            outside_writes: self.outside_writes,
            size,
            layout,
            file_hash,
            document_sha256: doc_sha,
            version,
            media: table,
            reused_media_bytes,
            backup,
            displaced: published.displaced,
            changed: published.changed,
            ticket: self.ticket.clone(),
        })
    }

    fn verify(
        &self,
        file: &File,
        path: &Path,
        records: &[Record],
        written: &[usize],
        document_sha256: &str,
        buf: &mut [u8],
    ) -> Result<(), ProjectIoError> {
        if self.options.verify == VerifyLevel::None {
            return Ok(());
        }
        let bad = |what: &str| {
            ProjectIoError::Invalid(format!(
                "The saved project failed verification ({what}). Nothing was replaced."
            ))
        };
        let container = read_container(
            file,
            ReadOptions {
                verify_media: false,
            },
        )?;
        if container.format != ContainerFormat::Pmv4
            || container.media.len() + container.archived_media.len() != records.len()
        {
            return Err(bad("record count"));
        }
        for want in records {
            let got = container
                .media
                .iter()
                .chain(&container.archived_media)
                .find(|r| r.id == want.id)
                .ok_or_else(|| bad("missing media index"))?;
            if got.id != want.id
                || got.offset != want.offset
                || got.length != want.length
                || got.revision.as_ref() != Some(&want.revision)
                || got.sha256.as_deref() != Some(want.sha256.as_str())
            {
                return Err(bad("media index"));
            }
        }
        super::validate_stored_project(pm_model::canonical::project_of(&container.document))
            .map_err(|_| bad("project"))?;
        let _ = document_sha256;
        for (i, r) in records.iter().enumerate() {
            if self.options.verify == VerifyLevel::Full || written.contains(&i) {
                let sha =
                    sha_of_range(file, r.offset, r.length, buf).map_err(|e| io_err(path, e))?;
                if sha != r.sha256 {
                    return Err(bad("media checksum"));
                }
            }
        }
        Ok(())
    }
}

/// Streams one media item into `out`, verifying any known checksum.
fn write_media(
    entry: &MediaEntry,
    out: &mut Out<'_>,
    buf: &mut [u8],
    temp: &Path,
) -> Result<(u64, String), ProjectIoError> {
    let media_err = |reason: String| ProjectIoError::Media {
        id: entry.id.to_string_lossy(),
        reason,
    };
    let tmp_err = |e: io::Error| io_err(temp, e);
    let mut hasher = Sha256::new();
    let (length, expected) = match &entry.source {
        MediaSource::Memory(bytes) => {
            hasher.update(bytes);
            out.write_all(bytes).map_err(tmp_err)?;
            (bytes.len() as u64, None)
        }
        MediaSource::Embedded {
            file: version,
            offset,
            length,
            sha256,
        } => {
            let changed = || {
                media_err(format!(
                    "its project file {} changed since it was opened",
                    version.path.display()
                ))
            };
            if !version.is_unchanged()? {
                // The path no longer names the file (renamed, moved or
                // deleted while open). The descriptor held since the version
                // was read is still that file: copy from it while its bytes
                // are proven the version's, before and after.
                let Some(src) = version.held_unchanged()? else {
                    return Err(changed());
                };
                copy_range(
                    src,
                    *offset,
                    *length,
                    out,
                    &mut hasher,
                    buf,
                    &media_err,
                    temp,
                )?;
                if version.held_unchanged()?.is_none() {
                    return Err(changed());
                }
                let digest = hex(&hasher.finalize());
                if sha256.as_deref().is_some_and(|want| want != digest) {
                    return Err(media_err(
                        "The project media checksum does not match.".into(),
                    ));
                }
                return Ok((*length, digest));
            }
            #[cfg(test)]
            source_hooks::hit(&source_hooks::BEFORE_OPEN);
            let src = File::open(&version.path).map_err(|e| io_err(&version.path, e))?;
            // The handle copied from must be the file as verified, and stay it
            // until the copy ends: an item without a checksum (PMV3) has
            // nothing else to prove its bytes.
            let held = |src: &File| -> Result<bool, ProjectIoError> {
                let meta = src.metadata().map_err(|e| io_err(&version.path, e))?;
                Ok(durable::stat_of(&meta).same_file(&version.stat))
            };
            if !held(&src)? {
                return Err(changed());
            }
            copy_range(
                &src,
                *offset,
                *length,
                out,
                &mut hasher,
                buf,
                &media_err,
                temp,
            )?;
            #[cfg(test)]
            source_hooks::hit(&source_hooks::AFTER_COPY);
            if !held(&src)? || !version.is_unchanged()? {
                return Err(changed());
            }
            (*length, sha256.as_deref())
        }
        MediaSource::File {
            path,
            length,
            sha256,
        } => {
            let src =
                File::open(path).map_err(|e| media_err(format!("{} ({e})", path.display())))?;
            let actual = src.metadata().map_err(|e| io_err(path, e))?.len();
            if actual != *length {
                return Err(media_err(format!(
                    "{} is {actual} bytes, expected {length}",
                    path.display()
                )));
            }
            copy_range(&src, 0, *length, out, &mut hasher, buf, &media_err, temp)?;
            (*length, sha256.as_deref())
        }
    };
    let digest = hex(&hasher.finalize());
    if expected.is_some_and(|want| want != digest) {
        return Err(media_err(
            "The project media checksum does not match.".into(),
        ));
    }
    Ok((length, digest))
}

#[allow(clippy::too_many_arguments)]
fn copy_range(
    src: &File,
    offset: u64,
    length: u64,
    out: &mut Out<'_>,
    hasher: &mut Sha256,
    buf: &mut [u8],
    media_err: &dyn Fn(String) -> ProjectIoError,
    temp: &Path,
) -> Result<(), ProjectIoError> {
    let mut done = 0u64;
    while done < length {
        let n = (buf.len() as u64).min(length - done) as usize;
        read_at(src, offset + done, &mut buf[..n])
            .map_err(|e| media_err(format!("could not read it ({e})")))?;
        hasher.update(&buf[..n]);
        out.write_all(&buf[..n]).map_err(|e| io_err(temp, e))?;
        done += n as u64;
    }
    Ok(())
}

/// Test-only points in an embedded source copy, where a test changes the
/// source file as another app would.
#[cfg(test)]
mod source_hooks {
    use std::cell::RefCell;
    pub(super) type Hook = RefCell<Option<Box<dyn FnOnce()>>>;
    thread_local! {
        pub(super) static BEFORE_OPEN: Hook = const { RefCell::new(None) };
        pub(super) static AFTER_COPY: Hook = const { RefCell::new(None) };
    }
    pub(super) fn hit(hook: &'static std::thread::LocalKey<Hook>) {
        if let Some(f) = hook.with(|h| h.borrow_mut().take()) {
            f();
        }
    }
}

#[cfg(test)]
mod embedded_source_tests {
    use super::*;

    /// A fresh folder for one test, removed when dropped.
    struct Dir(PathBuf);
    impl Dir {
        fn new(name: &str) -> Dir {
            let dir =
                std::env::temp_dir().join(format!("pm-save-source-{}-{name}", std::process::id()));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).unwrap();
            Dir(dir)
        }
        fn path(&self) -> &Path {
            &self.0
        }
    }
    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    /// A PMV3 file (items have no checksum) whose one item is `data`.
    fn pmv3(path: &Path, data: &[u8]) {
        let header = format!(
            r#"{{"document":{{"proj":{{"id":"p","name":"P","w":10,"h":10,"fps":30,"dur":1,"layers":[],"assets":{{"a":{{"id":"a","kind":"image"}}}}}}}},"media":[{{"id":"a","type":"application/octet-stream","offset":0,"length":{}}}]}}"#,
            data.len()
        );
        let mut bytes = b"PMV3\n".to_vec();
        bytes.extend((header.len() as u32).to_le_bytes());
        bytes.extend(header.as_bytes());
        bytes.extend(data);
        fs::write(path, bytes).unwrap();
    }

    /// A hook point and what another app does there.
    type Armed = (
        &'static std::thread::LocalKey<source_hooks::Hook>,
        Box<dyn FnOnce()>,
    );

    /// Copies item `a` of `path` as a save would, with `hook` armed.
    fn copy(path: &Path, hook: Option<Armed>) -> Result<Vec<u8>, ProjectIoError> {
        let opened =
            super::super::open_project(path, super::super::OpenOptions::default()).unwrap();
        let entry = opened.file.media.get("a").unwrap().clone();
        assert!(matches!(
            &entry.source,
            MediaSource::Embedded { sha256: None, .. }
        ));
        if let Some((point, f)) = hook {
            point.with(|h| *h.borrow_mut() = Some(f));
        }
        let dir = path.parent().unwrap();
        let temp = dir.join("out.tmp");
        let file = File::options()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(&temp)
            .unwrap();
        let mut out = Out {
            w: BufWriter::new(&file),
            offset: 0,
            written: 0,
            whole: Sha256::new(),
            limit: None,
        };
        let mut buf = vec![0; 1 << 16];
        let result = write_media(&entry, &mut out, &mut buf, &temp);
        drop(out);
        result.map(|_| fs::read(&temp).unwrap())
    }

    #[test]
    fn an_unchanged_source_copies_its_item() {
        let dir = Dir::new("unchanged");
        let path = dir.path().join("v1.pmv");
        pmv3(&path, b"original bytes");
        assert_eq!(copy(&path, None).unwrap(), b"original bytes");
    }

    /// Another app replaces the file between the version check and the open.
    #[test]
    fn a_source_replaced_before_it_opens_is_not_copied() {
        let dir = Dir::new("replaced");
        let path = dir.path().join("v1.pmv");
        pmv3(&path, b"original bytes");
        let other = dir.path().join("other.pmv");
        pmv3(&other, b"replaced bytes");
        let (from, to) = (other.clone(), path.clone());
        let result = copy(
            &path,
            Some((
                &source_hooks::BEFORE_OPEN,
                Box::new(move || fs::rename(from, to).unwrap()),
            )),
        );
        assert!(
            matches!(result, Err(ProjectIoError::Media { .. })),
            "{result:?}"
        );
    }

    /// Another app rewrites the item in place while it is being copied.
    #[test]
    fn a_source_changed_during_the_copy_is_not_kept() {
        let dir = Dir::new("changed");
        let path = dir.path().join("v1.pmv");
        pmv3(&path, b"original bytes");
        let target = path.clone();
        let result = copy(
            &path,
            Some((
                &source_hooks::AFTER_COPY,
                Box::new(move || {
                    let mut f = File::options().write(true).open(&target).unwrap();
                    f.seek(SeekFrom::End(-5)).unwrap();
                    f.write_all(b"XXXXX").unwrap();
                }),
            )),
        );
        assert!(
            matches!(result, Err(ProjectIoError::Media { .. })),
            "{result:?}"
        );
    }
}
