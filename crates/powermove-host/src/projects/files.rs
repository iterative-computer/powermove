//! Host-only overwrite grants, backups, and project reading (port of
//! `main/project-files.ts`). Renderer data can never invent an overwrite path.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::SystemTime;

use anyhow::{Context, Result, bail};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::durable::{Stamp, atomic_write, changed_file, check_version, copy_snapshot, file_version, hash_range, publish_file, read_at, read_exactly, sync_directory, temporary_sibling, write_hashed, FileVersion};
use super::format::{
    ContainerMedia, IncrementalMedia, SaveMedia, is_incremental_project, is_project_container, project_container_index, read_incremental_index, sha256_hex, validate_project_document,
};
use super::save::{ProjectSave, range_sha256};
use crate::ids::{FILE_CHUNK_BYTES, is_project_id};

const KEPT_BACKUPS: usize = 5;
const CHANGED_WHILE_OPENING: &str = "The project changed while opening. Try opening it again.";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Association {
    path: String,
    hash: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    generation: Option<u64>,
}

struct Reader {
    file: File,
    size: u64,
    stamp: Stamp,
}

/// Media ranges as `project:open` reports them (`ProjectMediaRange`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum MediaRange {
    Incremental(IncrementalMedia),
    Container(ContainerMedia),
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Opened {
    pub path: String,
    pub project_id: String,
    pub token: String,
    pub size: u64,
    pub document: Value,
    pub media: Vec<MediaRange>,
}

pub struct ProjectFiles {
    registry: PathBuf,
    backup_directory: PathBuf,
    /// Saves, commits and opens run one at a time (the TS `serial` queue).
    serial: Mutex<()>,
    associations: Mutex<HashMap<String, Association>>,
    saving: Mutex<HashSet<PathBuf>>,
    readers: Mutex<HashMap<String, Arc<Mutex<Reader>>>>,
}

fn path_text(path: &Path) -> Result<String> {
    path.to_str().map(str::to_owned).context("The project path is not valid Unicode.")
}

impl ProjectFiles {
    pub fn load(registry: PathBuf, backup_directory: PathBuf) -> Arc<Self> {
        let files = Arc::new(Self {
            registry,
            backup_directory,
            serial: Mutex::new(()),
            associations: Mutex::default(),
            saving: Mutex::default(),
            readers: Mutex::default(),
        });
        files.restore();
        files
    }

    fn restore(&self) {
        match fs::read(&self.registry) {
            Ok(bytes) => match serde_json::from_slice::<BTreeMap<String, Value>>(&bytes) {
                Ok(entries) => {
                    let mut associations = self.associations.lock();
                    for (id, value) in entries {
                        if let Ok(association) = serde_json::from_value::<Association>(value)
                            && is_project_id(&id)
                            && Path::new(&association.path).is_absolute()
                        {
                            associations.insert(id, association);
                        }
                    }
                }
                Err(error) => tracing::warn!("Project file associations unavailable; Save As will be used. {error}"),
            },
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => tracing::warn!("Project file associations unavailable; Save As will be used. {error}"),
        }
        // A file commit can outlive a failed registry write. Reconcile only an
        // exact matching committed file; never grant an overwrite from stale intent.
        let directory = self.registry.parent().unwrap_or(Path::new("."));
        let prefix = format!("{}.pending-", self.registry.file_name().unwrap_or_default().to_string_lossy());
        let Ok(entries) = fs::read_dir(directory) else { return };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if !name.starts_with(&prefix) || !name.ends_with(".json") {
                continue;
            }
            if let Err(error) = self.reconcile(&entry.path()) {
                tracing::warn!("Project save metadata needs recovery. {error:#}");
            }
        }
    }

    fn reconcile(&self, journal: &Path) -> Result<()> {
        #[derive(Deserialize)]
        struct Journal {
            id: String,
            path: String,
            hash: String,
            generation: u64,
        }
        let value: Journal = serde_json::from_slice(&fs::read(journal)?)?;
        let known = self.associations.lock().get(&value.id).and_then(|association| association.generation).unwrap_or(0);
        if is_project_id(&value.id)
            && Path::new(&value.path).is_absolute()
            && value.generation > known
            && file_version(Path::new(&value.path))?.is_some_and(|version| version.hash == value.hash)
        {
            self.associations.lock().insert(value.id, Association { path: value.path, hash: value.hash, generation: Some(value.generation) });
            self.persist()?;
        }
        fs::remove_file(journal)?;
        Ok(())
    }

    fn persist(&self) -> Result<()> {
        if let Some(parent) = self.registry.parent() {
            fs::create_dir_all(parent)?;
        }
        let snapshot: BTreeMap<String, Association> = self.associations.lock().iter().map(|(id, association)| (id.clone(), association.clone())).collect();
        atomic_write(&self.registry, &mut serde_json::to_vec(&snapshot)?.as_slice())
    }

    pub fn destination(&self, id: &str) -> Option<String> {
        self.associations.lock().get(id).map(|association| association.path.clone())
    }

    /// Backups of `id`, newest first.
    pub fn backups(&self, id: &str) -> Result<Vec<PathBuf>> {
        if !is_project_id(id) {
            bail!("Invalid project id");
        }
        let directory = self.backup_directory.join(id);
        let entries = match fs::read_dir(&directory) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(error.into()),
        };
        let mut names: Vec<String> = entries
            .flatten()
            .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_file()))
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .filter(|name| is_backup_name(name))
            .collect();
        names.sort_unstable_by(|a, b| b.cmp(a));
        Ok(names.into_iter().map(|name| directory.join(name)).collect())
    }

    fn backup(&self, id: &str, destination: &Path) -> Result<PathBuf> {
        let directory = self.backup_directory.join(id);
        fs::create_dir_all(&directory)?;
        let timestamp = backup_timestamp(SystemTime::now());
        for collision in 0.. {
            let name = if collision == 0 { format!("{timestamp}.pmv") } else { format!("{timestamp}.pmv-{collision}") };
            let file = directory.join(name);
            match copy_snapshot(destination, &file) {
                Ok(()) => {
                    if let Err(error) = sync_directory(&directory) {
                        let _ = fs::remove_file(&file);
                        return Err(error.into());
                    }
                    return Ok(file);
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error.into()),
            }
        }
        unreachable!()
    }

    fn prune(&self, id: &str) {
        let result = self.backups(id).and_then(|backups| {
            for path in backups.iter().skip(KEPT_BACKUPS) {
                fs::remove_file(path)?;
            }
            Ok(())
        });
        if let Err(error) = result {
            tracing::warn!("Could not prune project backups for {id}. {error:#}");
        }
    }

    fn commit(&self, id: &str, destination: &str, temporary: &Path, hash: &str, previous: Option<&FileVersion>) -> Result<String> {
        let target = Path::new(destination);
        check_version(target, previous)?;
        let journal = PathBuf::from(format!("{}.pending-{}.json", self.registry.display(), uuid::Uuid::new_v4()));
        let generation = self.associations.lock().get(id).and_then(|association| association.generation).unwrap_or(0) + 1;
        let mut backup = None;
        let result = (|| {
            if previous.is_some() {
                backup = Some(self.backup(id, target)?);
            }
            check_version(target, previous)?;
            if let Some(parent) = self.registry.parent() {
                fs::create_dir_all(parent)?;
            }
            let intent = serde_json::json!({ "id": id, "path": destination, "hash": hash, "generation": generation });
            atomic_write(&journal, &mut serde_json::to_vec(&intent)?.as_slice())?;
            publish_file(temporary, target, previous)?;
            self.associations.lock().insert(id.to_owned(), Association { path: destination.to_owned(), hash: hash.to_owned(), generation: Some(generation) });
            // The user's file is durably committed. A failed private registry write
            // must not report that their file failed; retain the journal for restart.
            match self.persist().and_then(|()| Ok(fs::remove_file(&journal)?)) {
                Ok(()) => {}
                Err(error) => tracing::warn!("Project saved; its file association will be repaired on restart. {error:#}"),
            }
            self.prune(id);
            Ok(destination.to_owned())
        })();
        if result.is_err()
            && let Some(backup) = backup
        {
            let current = file_version(target).ok().flatten();
            if previous.is_some_and(|previous| current.is_some_and(|current| current.hash == previous.hash)) {
                let _ = fs::remove_file(&backup);
            } else {
                // A failed/uncertain publication may still need its old version.
                // Keep it separately so it cannot evict successful backup generations.
                let mut recovery = backup.clone().into_os_string();
                recovery.push(".recovery.pmv");
                let _ = fs::rename(&backup, recovery);
            }
        }
        result
    }

    /// Resolve where `id` saves and refuse a file changed outside Powermove.
    fn prepare(&self, id: &str, selected: Option<&str>) -> Result<(String, Option<FileVersion>)> {
        if !is_project_id(id) {
            bail!("Invalid project id");
        }
        let known = self.associations.lock().get(id).cloned();
        let Some(destination) = selected.map(str::to_owned).or_else(|| known.as_ref().map(|known| known.path.clone())) else {
            bail!("Choose a destination with Save As.");
        };
        if self.saving.lock().contains(Path::new(&destination)) {
            bail!("A save to this project file is already in progress.");
        }
        let previous = file_version(Path::new(&destination))?;
        if selected.is_none()
            && let Some(known) = &known
            && previous.as_ref().map(|previous| &previous.hash) != Some(&known.hash)
        {
            return Err(changed_file());
        }
        Ok((destination, previous))
    }

    pub fn begin_incremental(self: &Arc<Self>, id: &str, media: Vec<SaveMedia>, document_bytes: u64, selected: Option<&str>) -> Result<ProjectSave> {
        let _serial = self.serial.lock();
        let (destination, previous) = self.prepare(id, selected)?;
        let target = PathBuf::from(&destination);
        self.saving.lock().insert(target.clone());
        let files = self.clone();
        let (commit_id, commit_previous) = (id.to_owned(), previous.clone());
        let commit_destination = destination.clone();
        let commit = Box::new(move |temporary: &Path, hash: String| {
            let _serial = files.serial.lock();
            files.commit(&commit_id, &commit_destination, temporary, &hash, commit_previous.as_ref())
        });
        let files = self.clone();
        let release = Box::new(move || {
            files.saving.lock().remove(&target);
        });
        ProjectSave::create(Path::new(&destination), previous.as_ref(), media, document_bytes, commit, release, selected.is_none())
    }

    pub fn save(&self, id: &str, data: &mut dyn Read, selected: Option<&str>) -> Result<String> {
        let _serial = self.serial.lock();
        let (destination, previous) = self.prepare(id, selected)?;
        let temporary = temporary_sibling(Path::new(&destination));
        let result = (|| {
            let mut file = fs::OpenOptions::new().write(true).create_new(true).open(&temporary)?;
            let hash = write_hashed(&mut file, data)?;
            file.sync_all()?;
            drop(file);
            self.commit(id, &destination, &temporary, &hash, previous.as_ref())
        })();
        let _ = fs::remove_file(&temporary);
        result
    }

    pub fn open(&self, path: &Path) -> Result<Opened> {
        let _serial = self.serial.lock();
        let file = File::open(path)?;
        let metadata = file.metadata()?;
        if !metadata.is_file() {
            bail!("This is not a readable project file.");
        }
        let stamp = Stamp::of(&metadata);
        let size = stamp.size;
        let prefix = read_exactly(&file, 0, size.min(9))?;
        let (document, media) = if is_incremental_project(&prefix) {
            let index = read_incremental_index(size, |offset, length| read_exactly(&file, offset, length))?;
            let bytes = read_exactly(&file, index.document.offset, index.document.length)?;
            if sha256_hex(&bytes) != index.document.sha256 {
                bail!("The project document checksum does not match.");
            }
            let document: Value = serde_json::from_slice(&bytes)?;
            for item in &index.media {
                if range_sha256(&file, item.offset, item.length)? != item.sha256 {
                    bail!("The project media checksum does not match.");
                }
            }
            (document, index.media.into_iter().map(MediaRange::Incremental).collect())
        } else if is_project_container(&prefix) {
            if prefix.len() < 9 {
                bail!("The project container is truncated.");
            }
            let header_length = u32::from_le_bytes(prefix[5..9].try_into().unwrap()) as u64;
            if header_length < 2 || header_length > size - 9 {
                bail!("The project container header is invalid.");
            }
            let header: Value = serde_json::from_slice(&read_exactly(&file, 9, header_length)?)?;
            let (document, media) = project_container_index(&header, 9 + header_length, size)?;
            (document, media.into_iter().map(MediaRange::Container).collect())
        } else {
            // Legacy JSON stores its media inside the document itself.
            (serde_json::from_slice(&read_exactly(&file, 0, size)?)?, Vec::new())
        };
        validate_project_document(&document)?;
        let hash = hex::encode(sha2::Digest::finalize(hash_range(&file, 0, None)?));
        if Stamp::of(&file.metadata()?) != stamp {
            bail!(CHANGED_WHILE_OPENING);
        }
        let text = path_text(path)?;
        let project_id = uuid::Uuid::new_v4().to_string();
        let token = uuid::Uuid::new_v4().to_string();
        self.associations.lock().insert(project_id.clone(), Association { path: text.clone(), hash, generation: None });
        self.persist()?;
        self.readers.lock().insert(token.clone(), Arc::new(Mutex::new(Reader { file, size, stamp })));
        Ok(Opened { path: text, project_id, token, size, document, media })
    }

    pub fn read(&self, token: &str, offset: u64, length: u64) -> Result<Vec<u8>> {
        let reader = self.readers.lock().get(token).cloned();
        let reader = reader.as_ref().and_then(|reader| reader.try_lock());
        let Some(reader) = reader.filter(|reader| offset <= reader.size && (1..=FILE_CHUNK_BYTES as u64).contains(&length) && length <= reader.size - offset) else {
            bail!("Invalid project read range");
        };
        let mut data = vec![0; length as usize];
        let read = read_at(&reader.file, &mut data, offset)?;
        if read == 0 {
            bail!("The project container is truncated.");
        }
        data.truncate(read);
        Ok(data)
    }

    pub fn close(&self, token: &str, verify: bool) -> Result<()> {
        let Some(reader) = self.readers.lock().remove(token) else { return Ok(()) };
        let reader = reader.lock();
        if verify && Stamp::of(&reader.file.metadata()?) != reader.stamp {
            bail!(CHANGED_WHILE_OPENING);
        }
        Ok(())
    }
}

/// `YYYY-MM-DDTHH-MM-SS-mmmZ`: an ISO timestamp with `:` and `.` replaced.
fn backup_timestamp(time: SystemTime) -> String {
    let since = time.duration_since(SystemTime::UNIX_EPOCH).unwrap_or_default();
    let seconds = since.as_secs() as i64;
    let (days, rest) = (seconds.div_euclid(86_400), seconds.rem_euclid(86_400));
    // Howard Hinnant's civil_from_days.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}-{:02}-{:02}-{:03}Z",
        rest / 3600,
        rest / 60 % 60,
        rest % 60,
        since.subsec_millis()
    )
}

/// `/^\d{4}-\d{2}-\d{2}T\d{2}-\d{2}-\d{2}-\d{3}Z\.pmv(?:-\d+)?$/`
fn is_backup_name(name: &str) -> bool {
    const TEMPLATE: &[u8] = b"0000-00-00T00-00-00-000Z.pmv";
    let bytes = name.as_bytes();
    if bytes.len() < TEMPLATE.len() {
        return false;
    }
    let (stamp, suffix) = bytes.split_at(TEMPLATE.len());
    let stamp_ok = stamp.iter().zip(TEMPLATE).all(|(byte, pattern)| if *pattern == b'0' { byte.is_ascii_digit() } else { byte == pattern });
    stamp_ok && (suffix.is_empty() || (suffix.len() > 1 && suffix[0] == b'-' && suffix[1..].iter().all(u8::is_ascii_digit)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_backup_names() {
        let time = SystemTime::UNIX_EPOCH + std::time::Duration::from_millis(1_760_352_896_789);
        let name = format!("{}.pmv", backup_timestamp(time));
        assert_eq!(name, "2025-10-13T10-54-56-789Z.pmv");
        assert!(is_backup_name(&name));
        assert!(is_backup_name(&format!("{name}-2")));
        assert!(!is_backup_name(&format!("{name}-")));
        assert!(!is_backup_name(&format!("{name}.recovery.pmv")));
    }
}
