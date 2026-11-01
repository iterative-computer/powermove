//! Opening and saving `.pmv` projects.
//!
//! # Open ([`open_project`])
//!
//! Blocking and I/O heavy: call it **off the UI thread** (GPUI callers pass
//! it to their background executor; [`spawn_open`] is a plain-thread
//! fallback). It reads JSON, PMV3 and PMV4 through
//! [`pm_model::container::read_container`], then, like `ProjectFiles.open`
//! in the old app:
//!
//! * hashes the whole file in one pass (`hashProjectFile`), verifying every
//!   PMV4 media checksum on the way; the hash is the file's
//!   [`FileVersion`], the guard every later in-place save checks;
//! * refuses a file that changed while it was being read;
//! * validates the stored project (`assertProjectDocument`: an object with a
//!   `layers` array and finite `w` and `h`);
//! * splits the saved envelope (`{ v, proj, history, ws, ... }`) from the
//!   project, keeping `history`, `ws` and every member it does not know
//!   verbatim ([`Envelope`]);
//! * canonicalizes the project ([`pm_model::canonical::canonical_project`]:
//!   every composition in `comps/<id>`, `compId` names the open one);
//! * builds the [`MediaTable`]: every embedded record as a *range of the file
//!   it came from* (never loaded into memory), plus legacy base64 `media` of
//!   old JSON files decoded into memory;
//! * imports the saved undo history when it is replayable
//!   ([`History::from_saved`]).
//!
//! # Save ([`SavePlan::run`])
//!
//! Always writes PMV4, the format and layout of the old `ProjectSave`; see
//! the [`save`] module for the algorithm and guarantees.

mod base64;
mod durable;
mod envelope;
pub mod save;

use std::collections::HashSet;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use pm_model::canonical::{canonical_project, open_comp_id};
use pm_model::container::{ContainerFormat, MediaRecord, ReadOptions, read_container};
use pm_model::{JsString, Map, Value};
use sha2::{Digest, Sha256};

pub use durable::{
    BackupPolicy, FileStat, FileVersion, Publish, check_version, hash_file, list_backups,
    temporary_sibling,
};
pub use save::{SaveOptions, SavePlan, SaveReceipt, SaveTarget, VerifyLevel, WriteFault};

use crate::error::{ProjectIoError, io_err};
use crate::history::History;

/// The `v` member the old app wrote (`PM.version`). Every save writes it,
/// whatever the opened file had, as the old app did.
pub const ENVELOPE_VERSION: &str = "1.0.0";

/// Where the bytes of one embedded media item come from when saving.
#[derive(Clone, Debug)]
pub enum MediaSource {
    /// A range of a project file we opened or saved. Copied into the new
    /// file and checksum verified; never held in memory. `file` is the
    /// version the range belongs to and is re-checked before reading, so a
    /// replaced source is an error, never silent corruption.
    Embedded {
        file: Arc<FileVersion>,
        offset: u64,
        length: u64,
        /// Known for PMV4 sources; PMV3 records carry none.
        sha256: Option<String>,
    },
    /// A file on disk (an import or a linked clip). `length` is what the
    /// catalog recorded; a different size at save time is an error.
    File {
        path: PathBuf,
        length: u64,
        sha256: Option<String>,
    },
    /// Bytes in memory (small generated assets, legacy base64 media).
    Memory(Arc<[u8]>),
}

/// One item of the project's embedded media.
#[derive(Clone, Debug)]
pub struct MediaEntry {
    /// The asset id (`assets[id]` in the project).
    pub id: JsString,
    /// MIME type recorded in the file (`type`).
    pub media_type: JsString,
    /// Storage revision. Different bytes must have a different revision:
    /// an in-place save reuses the previous file's range for an id only when
    /// revision and length both match (`ProjectSave.create`).
    pub revision: JsString,
    pub source: MediaSource,
}

impl MediaEntry {
    pub fn length(&self) -> u64 {
        match &self.source {
            MediaSource::Embedded { length, .. } | MediaSource::File { length, .. } => *length,
            MediaSource::Memory(bytes) => bytes.len() as u64,
        }
    }

    /// A new revision string for bytes without one (`crypto.randomUUID()`).
    pub fn fresh_revision() -> JsString {
        JsString::from(durable::random_uuid().as_str())
    }
}

/// Embedded media by asset id, in file order.
#[derive(Clone, Debug, Default)]
pub struct MediaTable {
    entries: Vec<MediaEntry>,
}

impl MediaTable {
    pub fn iter(&self) -> impl Iterator<Item = &MediaEntry> {
        self.entries.iter()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn get(&self, id: &str) -> Option<&MediaEntry> {
        self.entries.iter().find(|e| e.id == *id)
    }

    /// Adds or replaces an entry by id; an existing id keeps its position.
    pub fn insert(&mut self, entry: MediaEntry) {
        match self.entries.iter_mut().find(|e| e.id == entry.id) {
            Some(slot) => *slot = entry,
            None => self.entries.push(entry),
        }
    }

    pub fn remove(&mut self, id: &str) -> Option<MediaEntry> {
        let pos = self.entries.iter().position(|e| e.id == *id)?;
        Some(self.entries.remove(pos))
    }
}

/// The saved document minus the project.
#[derive(Clone, Debug, PartialEq)]
pub struct Envelope {
    /// `v` as read (informational: saves write [`ENVELOPE_VERSION`]).
    pub version: Option<Value>,
    /// `history` exactly as read. Written back unchanged when it could not be
    /// imported and this session recorded no undo step.
    pub history: Option<Value>,
    /// `ws` (the workspace snapshot), kept verbatim until the shell lane
    /// replaces it with [`Envelope::workspace`].
    pub workspace: Option<Value>,
    /// Every other member, kept verbatim (a newer app's fields survive).
    pub extra: Map,
    /// `false` for a bare project file (no `proj` wrapper). It is wrapped on
    /// its first save, as the old app did.
    pub wrapped: bool,
    /// Whether the stored project had a `comps` member, so a project that
    /// had none does not gain an empty one.
    pub had_comps: bool,
}

impl Default for Envelope {
    fn default() -> Envelope {
        Envelope {
            version: Some(Value::from(ENVELOPE_VERSION)),
            history: None,
            workspace: None,
            extra: Map::new(),
            wrapped: true,
            had_comps: true,
        }
    }
}

/// Everything about the file a project came from that saving needs.
#[derive(Clone, Debug)]
pub struct ProjectFile {
    /// `None` for an untitled project.
    pub path: Option<PathBuf>,
    /// The format it was read as.
    pub format: ContainerFormat,
    /// The file as we last read or wrote it (`None` when untitled).
    pub version: Option<Arc<FileVersion>>,
    pub envelope: Envelope,
    pub media: MediaTable,
}

impl ProjectFile {
    /// A project that has never been saved.
    pub fn untitled() -> ProjectFile {
        ProjectFile {
            path: None,
            format: ContainerFormat::Pmv4,
            version: None,
            envelope: Envelope::default(),
            media: MediaTable::default(),
        }
    }
}

/// A project read from disk, ready for [`DocumentStore::open`](crate::DocumentStore::open).
#[derive(Debug)]
pub struct OpenedProject {
    pub file: ProjectFile,
    /// The canonical project (the open composition in `comps/<compId>`) as a
    /// revisioned tree at revision 1, built on the opening thread.
    pub document: pm_model::Document,
    /// The open composition (`None` for a legacy root-layers project).
    pub open_comp: Option<String>,
    /// The saved undo history, when replayable.
    pub history: Option<History>,
}

/// How to open.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OpenOptions {
    /// Verify every PMV4 media checksum while hashing the file. On by
    /// default, as in the old app.
    pub verify_media: bool,
}

impl Default for OpenOptions {
    fn default() -> Self {
        OpenOptions { verify_media: true }
    }
}

/// `hashProjectFile`: the SHA-256 of the whole file, verifying the given
/// (already validated, non-overlapping) media ranges in the same pass.
fn hash_project_file(
    file: &File,
    path: &Path,
    size: u64,
    media: &[&MediaRecord],
) -> Result<String, ProjectIoError> {
    let bad = || {
        ProjectIoError::Container(pm_model::container::ContainerError::Invalid(
            "The project media checksum does not match.",
        ))
    };
    let empty = durable::hex(&Sha256::digest([]));
    for record in media {
        if record.length == 0 && record.sha256.as_deref().is_some_and(|s| s != empty) {
            return Err(bad());
        }
    }
    let mut ranges: Vec<&MediaRecord> = media
        .iter()
        .copied()
        .filter(|r| r.length > 0 && r.sha256.is_some())
        .collect();
    ranges.sort_by_key(|r| r.offset);
    let mut whole = Sha256::new();
    let mut index = 0;
    let mut media_hash = Sha256::new();
    let mut buf = vec![0u8; (1usize << 20).min(size.max(1) as usize)];
    let mut position = 0u64;
    #[cfg(unix)]
    use std::os::unix::fs::FileExt;
    while position < size {
        let want = (buf.len() as u64).min(size - position) as usize;
        #[cfg(unix)]
        let n = durable::read_retry(|| file.read_at(&mut buf[..want], position))
            .map_err(|e| io_err(path, e))?;
        #[cfg(not(unix))]
        let n = {
            use std::io::{Read, Seek, SeekFrom};
            let mut f = file;
            f.seek(SeekFrom::Start(position)).map_err(|e| io_err(path, e))?;
            f.read(&mut buf[..want]).map_err(|e| io_err(path, e))?
        };
        if n == 0 {
            return Err(ProjectIoError::Container(
                pm_model::container::ContainerError::Invalid("The project container is truncated."),
            ));
        }
        let end = position + n as u64;
        whole.update(&buf[..n]);
        while index < ranges.len() {
            let record = ranges[index];
            if record.offset >= end {
                break;
            }
            let start = position.max(record.offset);
            let stop = end.min(record.offset + record.length);
            if stop > start {
                media_hash.update(&buf[(start - position) as usize..(stop - position) as usize]);
            }
            if record.offset + record.length > end {
                break;
            }
            let digest = durable::hex(&std::mem::take(&mut media_hash).finalize());
            if Some(digest.as_str()) != record.sha256.as_deref() {
                return Err(bad());
            }
            index += 1;
        }
        position = end;
    }
    if index != ranges.len() {
        return Err(ProjectIoError::Container(
            pm_model::container::ContainerError::Invalid("The project container is truncated."),
        ));
    }
    Ok(durable::hex(&whole.finalize()))
}

/// Opens a project file. **Blocking**: run it off the UI thread.
pub fn open_project(path: &Path, options: OpenOptions) -> Result<OpenedProject, ProjectIoError> {
    let file = File::open(path).map_err(|e| io_err(path, e))?;
    let meta = file.metadata().map_err(|e| io_err(path, e))?;
    if !meta.is_file() {
        return Err(ProjectIoError::Invalid(
            "This is not a readable project file.".into(),
        ));
    }
    let before = durable::stat_of(&meta);
    // Index and document checksums are always verified by the reader; media
    // checksums are verified below, in the single hashing pass.
    let container = read_container(&file, ReadOptions { verify_media: false })?;
    let verify: Vec<&MediaRecord> = if options.verify_media {
        container.media.iter().collect()
    } else {
        Vec::new()
    };
    let hash = hash_project_file(&file, path, before.size, &verify)?;
    let after = durable::stat_of(&file.metadata().map_err(|e| io_err(path, e))?);
    if before.size != after.size
        || before.mtime_ns != after.mtime_ns
        || before.ctime_ns != after.ctime_ns
    {
        return Err(ProjectIoError::Invalid(
            "The project changed while opening. Try opening it again.".into(),
        ));
    }
    let version = Arc::new(FileVersion {
        path: path.to_owned(),
        stat: after,
        hash,
    });
    from_container(container, Some(path.to_owned()), Some(version))
}

/// Splits, validates and canonicalizes a decoded container (shared by
/// [`open_project`] and tests that build files in memory).
pub(crate) fn from_container(
    container: pm_model::container::Container,
    path: Option<PathBuf>,
    version: Option<Arc<FileVersion>>,
) -> Result<OpenedProject, ProjectIoError> {
    let format = container.format;
    // `document.proj || document`.
    let (mut fields, mut project, wrapped) = match container.document {
        Value::Object(mut map) if map.get("proj").is_some_and(Value::is_truthy) => {
            let proj = map.remove("proj").unwrap_or(Value::Null);
            (map, proj, true)
        }
        other => (Map::new(), other, false),
    };
    validate_stored_project(&project)?;
    let had_comps = project.get("comps").is_some();

    let mut media = MediaTable::default();
    if let Some(version) = &version {
        for record in &container.media {
            media.insert(MediaEntry {
                id: record.id.clone(),
                media_type: record.media_type.clone(),
                revision: record
                    .revision
                    .clone()
                    .unwrap_or_else(MediaEntry::fresh_revision),
                source: MediaSource::Embedded {
                    file: version.clone(),
                    offset: record.offset,
                    length: record.length,
                    sha256: record.sha256.clone(),
                },
            });
        }
    }

    // Legacy JSON: `media: { <id>: { type, data: "<base64>" } }` beside the
    // project (`restoreProjectFileMedia`). Only media of assets the project
    // or its history can reach is restored; it becomes a proper PMV4 record
    // on the next save, and the member is not written back.
    if container.media.is_empty() {
        let legacy = if wrapped {
            fields.remove("media")
        } else {
            project.as_object_mut().and_then(|p| p.remove("media"))
        };
        if let Some(legacy) = legacy {
            let reachable = reachable_in_stored(&project, fields.get("history"));
            if let Value::Object(legacy) = legacy {
                for (id, source) in legacy.iter() {
                    let Some(id_str) = id.as_str() else { continue };
                    if !reachable.contains(id_str) {
                        continue;
                    }
                    let ty = source.get("type").and_then(Value::as_js_string);
                    let data = source.get("data").and_then(Value::as_str);
                    let bytes = data.and_then(base64::decode);
                    let (Some(ty), Some(bytes)) = (ty, bytes) else {
                        return Err(ProjectIoError::Media {
                            id: id.to_string_lossy(),
                            reason: "Invalid saved media".into(),
                        });
                    };
                    media.insert(MediaEntry {
                        id: id.clone(),
                        media_type: ty.clone(),
                        revision: MediaEntry::fresh_revision(),
                        source: MediaSource::Memory(bytes.into()),
                    });
                }
            }
        }
    }

    let version_member = fields.remove("v");
    let raw_history = fields.remove("history");
    let workspace = fields.remove("ws");
    let canonical = canonical_project(project);
    let open_comp = open_comp_id(&canonical).map(str::to_owned);
    let history = raw_history
        .as_ref()
        .and_then(|h| History::from_saved(h, open_comp.as_deref()));
    Ok(OpenedProject {
        file: ProjectFile {
            path,
            format,
            version,
            envelope: Envelope {
                version: version_member,
                history: raw_history,
                workspace,
                extra: fields,
                wrapped,
                had_comps,
            },
            media,
        },
        document: pm_model::Document::new(Some(canonical), 1),
        open_comp,
        history,
    })
}

/// Asset ids reachable from a *stored* project and its saved history
/// (`fileAssets` in `legacy/core/project-file.ts`).
fn reachable_in_stored(project: &Value, history: Option<&Value>) -> HashSet<String> {
    let mut ids = HashSet::new();
    let mut add_keys = |obj: Option<&Value>, ids: &mut HashSet<String>| {
        if let Some(Value::Object(map)) = obj {
            ids.extend(map.keys().filter_map(|k| k.as_str().map(str::to_owned)));
        }
    };
    if let Some(entries) = history
        .and_then(|h| h.get("entries"))
        .and_then(Value::as_array)
    {
        for entry in entries {
            for side in ["forward", "backward"] {
                let Some(patches) = entry.get(side).and_then(Value::as_array) else {
                    continue;
                };
                for patch in patches {
                    let Ok(patch) = pm_model::Patch::from_value(patch) else {
                        continue;
                    };
                    collect_patch_assets(&patch, &mut ids, &mut add_keys);
                }
            }
        }
    }
    add_keys(project.get("assets"), &mut ids);
    ids
}

pub(crate) fn collect_patch_assets(
    patch: &pm_model::Patch,
    ids: &mut HashSet<String>,
    add_keys: &mut impl FnMut(Option<&Value>, &mut HashSet<String>),
) {
    use pm_model::PathPart;
    if !patch.exists {
        return;
    }
    let key = |i: usize| match patch.path.get(i) {
        Some(PathPart::Key(k)) => k.as_str(),
        _ => None,
    };
    let value = patch.value.as_ref();
    match patch.path.len() {
        0 => add_keys(value.and_then(|v| v.get("assets")), ids),
        1 if key(0) == Some("assets") => add_keys(value, ids),
        2 if key(0) == Some("assets") => {
            if value.and_then(|v| v.get("id")).is_some_and(Value::is_truthy) {
                if let Some(id) = key(1) {
                    ids.insert(id.to_owned());
                }
            }
        }
        _ => {}
    }
}

/// `assertProjectDocument` on the stored project: an object with a `layers`
/// array and finite `w` and `h`.
pub(crate) fn validate_stored_project(project: &Value) -> Result<(), ProjectIoError> {
    let finite = |k: &str| project.get(k).and_then(Value::as_f64).is_some_and(f64::is_finite);
    let ok = project.as_object().is_some()
        && project.get("layers").is_some_and(|l| l.as_array().is_some())
        && finite("w")
        && finite("h");
    if ok { Ok(()) } else { Err(ProjectIoError::NotAProject) }
}

/// Runs [`open_project`] on a new thread. GPUI callers usually use their
/// own background executor instead.
pub fn spawn_open(
    path: PathBuf,
    options: OpenOptions,
) -> std::thread::JoinHandle<Result<OpenedProject, ProjectIoError>> {
    std::thread::Builder::new()
        .name("pm-open-project".into())
        .spawn(move || open_project(&path, options))
        .expect("spawn the open thread")
}
