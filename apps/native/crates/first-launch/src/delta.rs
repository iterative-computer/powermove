//! Later launches. Every capture is merged three ways against the newest merged
//! checkpoint: what the original app changed comes in, native edits stay, and an
//! edit on both sides keeps both (the original's bytes stay in its checkpoint and
//! the conflict is listed in the Migration Report). Checkpoints are never rewritten.
use crate::{Checkpoint, store_preparation};
use crate::{
    Config, Event, Outcome, State, Step, as_json, checkpoint, closure, disk, extensions,
    imported_key, media, place_file, project_key, publish, read_batches, require_supported_media,
    seal_shared, summary, to_kernel, validate, validate_native_paths,
};
use anyhow::{Context, Result, ensure};
use fs2::FileExt;
use pm_kernel::{
    History, LibraryStore,
    library::LibraryProject,
    locks::older_electron_running,
    persistence::{DiskFormat, LibraryWriter},
    profile::{LocalStorageEntry, ProfileStorage},
    state::{StateOp, is_setting},
};
use pm_model::json::{Value as KernelValue, parse, to_canonical_string};
use pm_native_migrate::{local_storage, migration::ClosureInput, snapshot::Snapshot};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File, OpenOptions},
    os::unix::fs::{OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProjectReceipt {
    pub native: String,
    /// Hash of the source project and history at the last import.
    pub source: String,
    /// Hash of the native copy as that import wrote it.
    pub copy: String,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "kebab-case")]
pub enum ConflictKind {
    Setting,
    File,
    LocalStorage,
    Project,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub struct Conflict {
    pub kind: ConflictKind,
    /// Store key, profile-relative path, `origin key`, or project name.
    pub id: String,
}

pub(crate) struct ProjectCopy {
    project: KernelValue,
    history: KernelValue,
    workspace: KernelValue,
    meta: KernelValue,
    pub refused: bool,
    source: String,
    copy: String,
}
fn fingerprint(project: &KernelValue, history: Option<&KernelValue>) -> String {
    let mut hasher = blake3::Hasher::new();
    hasher.update(to_canonical_string(project).as_bytes());
    hasher.update(b"\0");
    if let Some(history) = history {
        hasher.update(to_canonical_string(history).as_bytes());
    }
    hasher.finalize().to_hex().to_string()
}
/// The native form of a source project. Its authored name stays exactly as it was.
pub(crate) fn project_copy(p: &LibraryProject, id: &str) -> Result<ProjectCopy> {
    let mut project = as_json(&p.project)?;
    project["id"] = id.into();
    let history = p.history.clone().unwrap_or_else(|| {
        parse(r#"{"version":1,"paths":"composition","index":-1,"entries":[]}"#).unwrap()
    });
    let refused = History::import(
        &history,
        p.project.get("compId").and_then(KernelValue::as_str),
    )
    .is_err();
    let mut meta = p
        .session
        .as_ref()
        .map(as_json)
        .transpose()?
        .unwrap_or(serde_json::json!({}));
    // A read-only media association is not an in-place save destination.
    // The kernel can resolve it when this named copy is explicitly opened.
    if let Some(m) = meta.as_object_mut()
        && let Some(file) = m.remove("file")
    {
        m.insert("embeddedMedia".into(), file);
    }
    let project = to_kernel(&project)?;
    let workspace = p
        .workspace
        .as_ref()
        .map(|v| to_kernel(&pm_native_migrate::workspace::migrate(&as_json(v)?)))
        .transpose()?
        .unwrap_or_else(|| parse("{}").unwrap());
    Ok(ProjectCopy {
        source: fingerprint(&p.project, p.history.as_ref()),
        copy: fingerprint(&project, Some(&history)),
        project,
        history,
        workspace,
        meta: to_kernel(&meta)?,
        refused,
    })
}
impl ProjectCopy {
    pub(crate) fn bytes(&self) -> u64 {
        (to_canonical_string(&self.project).len()
            + to_canonical_string(&self.history).len()
            + to_canonical_string(&self.meta).len()
            + to_canonical_string(&self.workspace).len()) as u64
    }
    pub(crate) fn save(&self, writer: &LibraryWriter, id: &str) -> Result<()> {
        writer.save_project(
            id,
            self.project.clone(),
            self.history.clone(),
            self.workspace.clone(),
            self.meta.clone(),
        )?;
        Ok(())
    }
    pub(crate) fn receipt(&self, id: &str) -> ProjectReceipt {
        ProjectReceipt {
            native: id.to_owned(),
            source: self.source.clone(),
            copy: self.copy.clone(),
        }
    }
}

#[derive(Clone, Debug)]
pub enum DeltaPrepared {
    WaitingForElectron,
    /// Nothing changed in the original app since the last merge.
    Unchanged,
    /// Library state is merged; `delta_media` finishes the capture.
    MediaPending,
}
fn lease(native: &Path) -> Result<File> {
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(native.join("first-launch.lock"))?;
    lock.try_lock_exclusive()
        .context("first launch already running")?;
    Ok(lock)
}
fn base_root(native: &Path, state: &State) -> PathBuf {
    state.delta_base.as_ref().map_or_else(
        || native.join("first-launch-checkpoint"),
        |name| native.join("first-launch-deltas").join(name),
    )
}
fn load_state(native: &Path, source: &Path) -> Result<State> {
    let state: State = disk::json(&native.join("migration.json"))?;
    ensure!(
        state.source == source && state.bundle_id == crate::BETA_BUNDLE_ID && state.version == 1,
        "profile migration identity mismatch"
    );
    ensure!(
        state.completed.contains(&Step::Media),
        "bootstrap must finish before a delta"
    );
    Ok(state)
}

/// Capture the original app's profile and merge everything but media. Run with
/// the library detached: this holds the kernel's library writer. Reuses a
/// pending capture, so an interrupted merge resumes and every action is idempotent.
pub fn delta_store(
    config: &Config,
    mut hook: impl FnMut(Event) -> Result<()>,
) -> Result<DeltaPrepared> {
    let (source, native) = validate(config)?;
    if older_electron_running(&source) {
        hook(Event::WaitingForElectron)?;
        return Ok(DeltaPrepared::WaitingForElectron);
    }
    let mut state = load_state(&native, &source)?;
    let _lock = lease(&native)?;
    validate_native_paths(&native)?;
    let writer = LibraryWriter::open(&native, DiskFormat::V1_2, None)
        .map_err(|e| anyhow::anyhow!("native library unavailable: {e:?}"))?;
    let original = native.join("first-launch-checkpoint");
    ensure!(
        state.checkpoint_hash.as_ref() == Some(&disk::hash(&original.join("checkpoint.json"))?.0),
        "checkpoint manifest changed"
    );
    ensure!(
        state.availability_hash.as_ref()
            == Some(&disk::hash(&original.join("availability.json"))?.0),
        "availability checkpoint changed"
    );
    Checkpoint::load(&original)?;
    let base_dir = base_root(&native, &state);
    let base = Checkpoint::load(&base_dir)?;
    let deltas = native.join("first-launch-deltas");
    disk::private_dir(&deltas)?;
    // Retain all availability checkpoints, including interrupted passes. Reuse
    // the unfinished capture on resume; callers finish it before starting fresh.
    let pending = deltas.join("pending");
    let next = if pending.exists() {
        Checkpoint::load(&pending)?
    } else {
        hook(Event::Started(Step::Checkpoint))?;
        let stamp = checkpoint::source_stamp(&source)?;
        let next = Checkpoint::capture(&source, &pending)?;
        disk::bytes(&pending.join("source-stamp"), stamp.as_bytes())?;
        next
    };
    require_supported_media(&next, &pending)?;
    let mut merge = Merge {
        base: &base,
        base_root: &base_dir,
        next: &next,
        next_root: &pending,
        native: &native,
        writer: &writer,
        state: &mut state,
    };
    hook(Event::Started(Step::Bootstrap))?;
    let settings = merge.keys()?;
    hook(Event::Started(Step::Library))?;
    merge.projects(&mut hook)?;
    let extensions_changed = merge.files()?;
    hook(Event::Started(Step::LocalStorage))?;
    merge.local_storage()?;
    publish(&native, &state)?;
    writer.flush()?;
    drop(writer);
    if extensions_changed || !settings.is_empty() {
        hook(Event::Started(Step::Extensions))?;
        extensions(&native, settings)?;
    }
    Ok(DeltaPrepared::MediaPending)
}

/// Bring in a pending capture's media and promote it to the next base. No
/// library writer is taken: media objects are immutable and the attached kernel
/// holds the library lock for this process, so the library can be open meanwhile.
pub fn delta_media(config: &Config, mut hook: impl FnMut(Event) -> Result<()>) -> Result<Outcome> {
    let (source, native) = validate(config)?;
    let mut state = load_state(&native, &source)?;
    let _lock = lease(&native)?;
    let deltas = native.join("first-launch-deltas");
    let pending = deltas.join("pending");
    let next = Checkpoint::load(&pending)?;
    let batches = read_batches(&next, &pending)?;
    let availability = pending.join("availability.json");
    let closure: ClosureInput = if availability.exists() {
        disk::json(&availability)?
    } else {
        let c = closure::inventory(&next, &pending, &batches)?;
        disk::atomic(&availability, &c)?;
        c
    };
    hook(Event::Started(Step::Media))?;
    let pass = media(&native, &pending, &batches, &closure, &mut hook)?;
    let identity = disk::hash(&pending.join("checkpoint.json"))?.0;
    let name = format!(
        "{}-{}-{}",
        &identity[..16],
        &pass.snapshot[..16],
        &pass.base_hash[..16]
    );
    let destination = deltas.join(&name);
    let base = Checkpoint::load(&base_root(&native, &state))?;
    let moved = next.logical_keys != base.logical_keys
        || next.files != base.files
        || next.chromium_payloads != base.chromium_payloads;
    let stamp = fs::read_to_string(pending.join("source-stamp")).ok();
    if destination.exists() {
        disk::archive(&pending)?;
    } else {
        fs::rename(&pending, destination)?;
        File::open(&deltas)?.sync_all()?;
    }
    if moved {
        state.delta_base = Some(name);
    }
    if moved || state.source_stamp.is_none() {
        state.source_stamp = stamp.or(state.source_stamp.take());
        publish(&native, &state)?;
    }
    hook(Event::Complete)?;
    Ok(summary(&state, &pass))
}

/// One complete delta with the library detached.
pub fn delta(config: &Config, mut hook: impl FnMut(Event) -> Result<()>) -> Result<Outcome> {
    match delta_store(config, &mut hook)? {
        DeltaPrepared::WaitingForElectron => Ok(Outcome::WaitingForElectron),
        _ => {
            let native = disk::resolved(&config.native_profile)?;
            let _writer = LibraryWriter::open(&native, DiskFormat::V1_2, None)
                .map_err(|e| anyhow::anyhow!("native library unavailable: {e:?}"))?;
            delta_media(config, hook)
        }
    }
}

/// App launch path: an unchanged source costs a metadata walk, nothing else.
/// A pending capture from an interrupted launch still gets finished.
pub fn delta_if_changed(
    config: &Config,
    mut hook: impl FnMut(Event) -> Result<()>,
) -> Result<DeltaPrepared> {
    let (source, native) = validate(config)?;
    let state: State = disk::json(&native.join("migration.json"))?;
    if !native.join("first-launch-deltas/pending").exists()
        && state.source_stamp.as_deref() == Some(&checkpoint::source_stamp(&source)?)
    {
        return Ok(DeltaPrepared::Unchanged);
    }
    delta_store(config, &mut hook)
}

struct Merge<'a> {
    base: &'a Checkpoint,
    base_root: &'a Path,
    next: &'a Checkpoint,
    next_root: &'a Path,
    native: &'a Path,
    writer: &'a LibraryWriter,
    state: &'a mut State,
}
impl Merge<'_> {
    fn base_store(&self) -> LibraryStore {
        LibraryStore::open(&self.base_root.join("profile"))
    }
    fn next_store(&self) -> LibraryStore {
        LibraryStore::open(&self.next_root.join("profile"))
    }
    /// Returns the merged result of every changed setting. The kernel's durable
    /// record outranks the store at attach, so these go through it afterwards,
    /// including after a resume that finds the store already merged.
    fn keys(&mut self) -> Result<Vec<StateOp>> {
        let mut settings = Vec::new();
        let keys: BTreeSet<_> = self
            .base
            .logical_keys
            .keys()
            .chain(self.next.logical_keys.keys())
            .cloned()
            .collect();
        let (base_store, next_store) = (self.base_store(), self.next_store());
        for key in keys {
            if project_key(&key)
                || self.base.logical_keys.get(&key) == self.next.logical_keys.get(&key)
            {
                continue;
            }
            let read = |store: &LibraryStore| -> Result<Option<(String, KernelValue)>> {
                store
                    .read(&key)?
                    .map(|v| imported_key(&key, v, &self.next.projects))
                    .transpose()
            };
            let before = read(&base_store)?;
            let after = read(&next_store)?;
            let target = after
                .as_ref()
                .or(before.as_ref())
                .context("changed key has no value on either side")?
                .0
                .clone();
            let current = self.writer.store().read(&target)?;
            let map = |value: Option<&KernelValue>| -> Result<store_preparation::Store> {
                value.map_or(Ok(Default::default()), |v| {
                    Ok([(target.clone(), as_json(v)?)].into())
                })
            };
            let plan = store_preparation::merge(
                &map(before.as_ref().map(|(_, v)| v))?,
                &map(current.as_ref())?,
                &map(after.as_ref().map(|(_, v)| v))?,
            );
            for conflict in plan.conflicts {
                self.state.conflicts.insert(Conflict {
                    kind: ConflictKind::Setting,
                    id: conflict.path.join(" ▸ "),
                });
            }
            let merged = plan.values.get(&target);
            if is_setting(&target) {
                settings.push(StateOp::Setting {
                    key: target.clone(),
                    value: merged.map(to_kernel).transpose()?,
                });
            }
            let same = match (merged, &current) {
                (Some(m), Some(c)) => *m == as_json(c)?,
                (None, None) => true,
                _ => false,
            };
            if same {
                continue;
            }
            match merged {
                Some(value) => {
                    disk::space(self.native, value.to_string().len() as u64)?;
                    self.writer.set(&target, to_kernel(value)?)?;
                }
                None => self.writer.delete(&target)?,
            }
            self.writer.flush()?;
        }
        Ok(settings)
    }
    fn stored_fingerprint(&self, id: &str) -> Result<Option<String>> {
        Ok(self
            .writer
            .store()
            .load(id)?
            .map(|p| fingerprint(&p.project, p.history.as_ref())))
    }
    fn projects(&mut self, hook: &mut impl FnMut(Event) -> Result<()>) -> Result<()> {
        let store = self.next_store();
        if !self.next_root.join("profile/store").is_dir() {
            return Ok(());
        }
        let trash: BTreeSet<_> = store.trash()?.into_iter().map(|r| r.id).collect();
        let base_trash: BTreeSet<_> = self
            .base_store()
            .trash()
            .map(|rows| rows.into_iter().map(|r| r.id).collect())
            .unwrap_or_default();
        let total = self.next.projects.len();
        for (done, (source, id)) in self.next.projects.iter().enumerate() {
            let Some(p) = store.load(source)? else {
                self.state.missing_projects.insert(source.clone());
                continue;
            };
            self.state.missing_projects.remove(source);
            let copy = project_copy(&p, id)?;
            let name = p
                .project
                .get("name")
                .and_then(KernelValue::as_str)
                .unwrap_or("Untitled")
                .to_owned();
            match self.state.receipts.get(source).cloned() {
                None => {
                    if self.writer.store().load(id)?.is_none() {
                        disk::space(self.native, copy.bytes().saturating_mul(3))?;
                        copy.save(self.writer, id)?;
                    }
                    if trash.contains(source) {
                        self.writer.trash(id)?;
                    }
                    self.state.receipts.insert(source.clone(), copy.receipt(id));
                }
                Some(receipt) if receipt.source == copy.source => {
                    if trash.contains(source) != base_trash.contains(source)
                        && self.stored_fingerprint(id)?.as_ref() == Some(&receipt.copy)
                    {
                        if trash.contains(source) {
                            self.writer.trash(id)?;
                        } else {
                            self.writer.restore(id)?;
                        }
                    }
                    continue;
                }
                Some(receipt) => {
                    let current = self.stored_fingerprint(id)?;
                    if current.as_ref() == Some(&copy.copy) {
                        // Interrupted after the write, before the receipt.
                    } else if current.as_ref() == Some(&receipt.copy) {
                        disk::space(self.native, copy.bytes().saturating_mul(3))?;
                        copy.save(self.writer, id)?;
                    } else {
                        // Library work is additive: both versions stay in the library.
                        let key = blake3::hash(format!("{source}\0{}", copy.source).as_bytes())
                            .to_hex()
                            .to_string();
                        let keep = format!("native-{}", &key[..32]);
                        let incoming = project_copy(&p, &keep)?;
                        if self.writer.store().load(&keep)?.is_none() {
                            disk::space(self.native, incoming.bytes().saturating_mul(3))?;
                            incoming.save(self.writer, &keep)?;
                        }
                        self.state.conflicts.insert(Conflict {
                            kind: ConflictKind::Project,
                            id: name.clone(),
                        });
                        self.state.receipts.insert(
                            source.clone(),
                            ProjectReceipt {
                                source: copy.source.clone(),
                                ..receipt
                            },
                        );
                        continue;
                    }
                    self.state.receipts.insert(source.clone(), copy.receipt(id));
                }
            }
            self.state.history_refusals.remove(source);
            if copy.refused {
                self.state.history_refusals.insert(source.clone());
            }
            publish(self.native, self.state)?;
            hook(Event::Progress {
                step: Step::Library,
                done: done + 1,
                total,
            })?;
        }
        Ok(())
    }
    /// Three way by relative path, extension trees included. Returns whether an
    /// extension changed, so its registration can be refreshed.
    fn files(&mut self) -> Result<bool> {
        let paths: BTreeSet<&PathBuf> = self
            .base
            .files
            .keys()
            .chain(self.next.files.keys())
            .collect();
        let shared = self.native.join("extensions-shared");
        let mut extensions_changed = false;
        for relative in paths {
            let (before, after) = (self.base.files.get(relative), self.next.files.get(relative));
            if relative.starts_with("store") || before.map(|e| &e.hash) == after.map(|e| &e.hash) {
                continue;
            }
            let target = self.native.join(checkpoint::file_target(relative)?);
            let current = target
                .is_file()
                .then(|| disk::hash(&target).map(|h| h.0))
                .transpose()?;
            if current.as_ref() == after.map(|e| &e.hash) {
                continue;
            }
            if current.as_ref() != before.map(|e| &e.hash) {
                self.state.conflicts.insert(Conflict {
                    kind: ConflictKind::File,
                    id: relative.display().to_string(),
                });
                continue;
            }
            if target.starts_with(&shared) {
                extensions_changed = true;
                unseal(&shared, &target)?;
            }
            match after {
                Some(entry) => place_file(
                    &self.next_root.join("profile").join(relative),
                    &target,
                    entry,
                    self.native,
                )?,
                None => fs::remove_file(&target)?,
            }
        }
        seal_shared(&shared)?;
        Ok(extensions_changed)
    }
    fn local_storage(&mut self) -> Result<()> {
        fn entries(root: &Path, cp: &Checkpoint) -> Result<BTreeMap<(String, String), String>> {
            let Some(path) = &cp.local_storage else {
                return Ok(BTreeMap::new());
            };
            Ok(local_storage::read(&Snapshot::open(&root.join(path))?)?
                .into_iter()
                .map(|e| ((e.origin, e.key), e.value))
                .collect())
        }
        let before = entries(self.base_root, self.base)?;
        let after = entries(self.next_root, self.next)?;
        if before == after {
            return Ok(());
        }
        let storage = ProfileStorage::new(self.writer);
        let mut current: BTreeMap<_, _> = storage
            .local_storage()?
            .into_iter()
            .map(|e| ((e.origin, e.key), e.value))
            .collect();
        for key in before.keys().chain(after.keys()).collect::<BTreeSet<_>>() {
            let (b, a) = (before.get(key), after.get(key));
            if b == a || current.get(key) == a {
                continue;
            }
            if current.get(key) != b {
                self.state.conflicts.insert(Conflict {
                    kind: ConflictKind::LocalStorage,
                    id: format!("{} {}", key.0, key.1),
                });
                continue;
            }
            match a {
                Some(value) => current.insert(key.clone(), value.clone()),
                None => current.remove(key),
            };
        }
        storage.replace_local_storage(
            &current
                .into_iter()
                .map(|((origin, key), value)| LocalStorageEntry { origin, key, value })
                .collect::<Vec<_>>(),
        )?;
        Ok(())
    }
}
/// Extension copies stay read-only; only a merge opens the path it writes.
fn unseal(shared: &Path, target: &Path) -> Result<()> {
    let open = |dir: &Path| fs::set_permissions(dir, fs::Permissions::from_mode(0o700));
    open(shared)?;
    let mut dir = shared.to_path_buf();
    for part in target.strip_prefix(shared)?.parent().into_iter().flatten() {
        dir.push(part);
        if dir.is_dir() {
            open(&dir)?;
        }
    }
    Ok(())
}
