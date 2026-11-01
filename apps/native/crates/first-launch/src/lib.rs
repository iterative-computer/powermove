//! Native beta first-launch migration. Call on a background worker before
//! attaching the shell's library. No source profile is ever opened for writing.
//! Checkpoints contain private user state; keep them out of Git/telemetry.
mod checkpoint;
mod closure;
mod disk;
pub mod secrets;
pub use checkpoint::Checkpoint;

use anyhow::{Context, Result, ensure};
use fs2::FileExt;
use pm_kernel::{
    History, Kernel, KernelConfig, LibraryStore,
    locks::{OpenDecision, older_electron_running},
    persistence::{DiskFormat, LibraryWriter},
    session::{ShellCall, ShellReply},
    state::{ExtensionEntry, StateOp, StateTransaction},
};
use pm_model::json::{Value as KernelValue, parse, to_canonical_string};
use pm_native_migrate::{
    indexeddb, local_storage,
    migration::{self, ClosureInput, Pass},
    snapshot::Snapshot,
};
use secrets::{SecretAccess, SecretStep};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeSet,
    fs::{self, File, OpenOptions},
    os::unix::fs::{OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
};

pub const BETA_BUNDLE_ID: &str = "com.zellzoi.powermove.native-beta";
pub fn beta_profile(application_support: &Path) -> PathBuf {
    application_support.join(BETA_BUNDLE_ID)
}
#[derive(Clone, Debug)]
pub struct Config {
    pub electron_profile: PathBuf,
    pub native_profile: PathBuf,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "kebab-case")]
pub enum Step {
    Checkpoint,
    Bootstrap,
    Library,
    Media,
    LocalStorage,
    Extensions,
    Secrets,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct State {
    pub version: u32,
    pub bundle_id: String,
    pub source: PathBuf,
    pub checkpoint_hash: Option<String>,
    pub availability_hash: Option<String>,
    pub completed: BTreeSet<Step>,
    pub imported_projects: BTreeSet<String>,
    pub imported_keys: BTreeSet<String>,
    pub imported_files: BTreeSet<PathBuf>,
    pub history_refusals: BTreeSet<String>,
    pub missing_projects: BTreeSet<String>,
    pub secrets: SecretStep,
    pub imported_secrets: BTreeSet<String>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    WaitingForElectron,
    Started(Step),
    /// Only counts; private names/paths/secret material never go to UI events.
    Progress {
        step: Step,
        done: usize,
        total: usize,
    },
    Published(Step),
    SecretPromptRequired,
    Complete,
}
#[derive(Clone, Debug)]
pub enum Outcome {
    WaitingForElectron,
    NeedsSecretConsent {
        interrupted: bool,
    },
    Complete {
        projects: usize,
        history_refusals: usize,
        missing_projects: usize,
        blocked_owners: usize,
        proxy_only: usize,
        already_offline: usize,
    },
}
/// Completed bootstrap launches capture a fresh media delta, without rewriting
/// imported settings/library state. Interrupted bootstrap resumes its checkpoint.
/// The hook runs AFTER durable publication and may return an error to pause.
pub fn run(config: &Config, hook: impl FnMut(Event) -> Result<()>) -> Result<Outcome> {
    let (_, native) = validate(config)?;
    if native.join("migration.json").is_file() {
        let state: State = disk::json(&native.join("migration.json"))?;
        if state.completed.contains(&Step::Secrets) {
            return delta(config, hook);
        }
    }
    run_inner(config, None, hook)
}
/// The host must display `secrets::PROMPT` and receive a fresh user approval
/// before each invocation of this opt-in entry point. A crash after access began
/// leaves AccessStarted, and ordinary `run` never retries that Keychain access.
pub fn resume_with_secret_consent(
    config: &Config,
    access: &mut dyn SecretAccess,
    hook: impl FnMut(Event) -> Result<()>,
) -> Result<Outcome> {
    run_inner(config, Some(access), hook)
}
fn validate(config: &Config) -> Result<(PathBuf, PathBuf)> {
    let source = config.electron_profile.canonicalize()?;
    let native = disk::resolved(&config.native_profile)?;
    ensure!(
        config
            .native_profile
            .file_name()
            .is_some_and(|s| s == BETA_BUNDLE_ID),
        "native beta profile must use its bundle id directory"
    );
    ensure!(
        !native.starts_with(&source) && !source.starts_with(&native),
        "native profile overlaps Electron profile"
    );
    ensure!(
        !fs::symlink_metadata(&config.native_profile).is_ok_and(|m| m.file_type().is_symlink()),
        "native profile cannot be a symlink"
    );
    Ok((source, native))
}
fn validate_native_paths(native: &Path) -> Result<()> {
    // Reject linked ancestors before any writer runs. Existing beta files may
    // be edited/deleted, but native paths must never point back into Electron.
    for path in [
        "store",
        "Media Store",
        "first-launch-checkpoint",
        "first-launch-deltas",
        "extensions-shared",
        "extensions-overlay",
    ] {
        let p = native.join(path);
        ensure!(
            !fs::symlink_metadata(&p).is_ok_and(|m| m.file_type().is_symlink()),
            "symlink in native migration paths"
        );
    }
    disk::walk(&native.join("Media Store"))?;
    Ok(())
}
fn publish(root: &Path, state: &State) -> Result<()> {
    disk::atomic(&root.join("migration.json"), state)
}
fn finish(
    root: &Path,
    state: &mut State,
    step: Step,
    hook: &mut impl FnMut(Event) -> Result<()>,
) -> Result<()> {
    state.completed.insert(step);
    publish(root, state)?;
    hook(Event::Published(step))
}
fn run_inner(
    config: &Config,
    mut access: Option<&mut dyn SecretAccess>,
    mut hook: impl FnMut(Event) -> Result<()>,
) -> Result<Outcome> {
    let (source, native) = validate(config)?;
    // Waiting performs no writes, not even to the native destination. Never
    // remove, follow, or acquire Electron's SingletonLock.
    if older_electron_running(&source) {
        hook(Event::WaitingForElectron)?;
        return Ok(Outcome::WaitingForElectron);
    }
    if native.exists() && !native.join("migration.json").exists() {
        // A kill may occur after creating the process lock or temporary state
        // file, before the first state rename. Those two private artifacts are
        // safe to resume; any other preexisting destination remains refused.
        for entry in fs::read_dir(&native)? {
            let entry = entry?;
            ensure!(
                ["first-launch.lock", "migration.first-launch-part"]
                    .iter()
                    .any(|n| entry.file_name() == *n),
                "destination is not a fresh native beta profile"
            );
            ensure!(
                entry.file_type()?.is_file(),
                "invalid bootstrap recovery artifact"
            );
        }
    }
    disk::private_dir(&native)?;
    disk::space(&native, 0)?;
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(native.join("first-launch.lock"))?;
    lock.try_lock_exclusive()
        .context("first launch already running")?;
    let _lock = lock;
    validate_native_paths(&native)?;
    let mut state: State = if native.join("migration.json").exists() {
        disk::json(&native.join("migration.json"))?
    } else {
        let state = State {
            version: 1,
            bundle_id: BETA_BUNDLE_ID.into(),
            source: source.clone(),
            checkpoint_hash: None,
            availability_hash: None,
            completed: BTreeSet::new(),
            imported_projects: BTreeSet::new(),
            imported_keys: BTreeSet::new(),
            imported_files: BTreeSet::new(),
            history_refusals: BTreeSet::new(),
            missing_projects: BTreeSet::new(),
            secrets: SecretStep::NotStarted,
            imported_secrets: BTreeSet::new(),
        };
        publish(&native, &state)?;
        state
    };
    ensure!(
        state.version == 1 && state.bundle_id == BETA_BUNDLE_ID && state.source == source,
        "profile migration identity mismatch"
    );
    let root = native.join("first-launch-checkpoint");
    if !root.exists() {
        hook(Event::Started(Step::Checkpoint))?;
        Checkpoint::capture(&source, &root)?;
    }
    let cp = Checkpoint::load(&root)?;
    ensure!(cp.source == source, "checkpoint belongs to another source");
    let hash = disk::hash(&root.join("checkpoint.json"))?.0;
    if let Some(expected) = &state.checkpoint_hash {
        ensure!(expected == &hash, "checkpoint manifest changed");
    } else {
        state.checkpoint_hash = Some(hash);
    }
    if !state.completed.contains(&Step::Checkpoint) {
        finish(&native, &mut state, Step::Checkpoint, &mut hook)?;
    }
    let batches = read_batches(&cp, &root)?;
    let closure_path = root.join("availability.json");
    let closure: ClosureInput = if closure_path.exists() {
        disk::json(&closure_path)?
    } else {
        let c = closure::inventory(&cp, &root, &batches)?;
        disk::atomic(&closure_path, &c)?;
        c
    };
    let availability_hash = disk::hash(&closure_path)?.0;
    if let Some(expected) = &state.availability_hash {
        ensure!(
            expected == &availability_hash,
            "availability checkpoint changed"
        );
    } else {
        state.availability_hash = Some(availability_hash);
        publish(&native, &state)?;
    }
    // Keep the kernel's library lock throughout transfer, including media.
    // Passing Electron here would reject a safe native write if it starts later;
    // source immutability is already established by the sealed checkpoint.
    let writer = LibraryWriter::open(&native, DiskFormat::V1_2, None)
        .map_err(|e| anyhow::anyhow!("native library unavailable: {e:?}"))?;
    if !state.completed.contains(&Step::Bootstrap) {
        hook(Event::Started(Step::Bootstrap))?;
        bootstrap(&cp, &root, &native, &writer, &mut state, &mut hook)?;
        finish(&native, &mut state, Step::Bootstrap, &mut hook)?;
    }
    if !state.completed.contains(&Step::Library) {
        hook(Event::Started(Step::Library))?;
        library(&cp, &root, &native, &writer, &mut state, &mut hook)?;
        finish(&native, &mut state, Step::Library, &mut hook)?;
    }
    hook(Event::Started(Step::Media))?;
    let pass = media(&native, &batches, &closure, &mut hook)?;
    if !state.completed.contains(&Step::Media) {
        finish(&native, &mut state, Step::Media, &mut hook)?;
    }
    if !state.completed.contains(&Step::LocalStorage) {
        hook(Event::Started(Step::LocalStorage))?;
        let entries = if let Some(path) = &cp.local_storage {
            local_storage::read(&Snapshot::open(&root.join(path))?)?
        } else {
            Vec::new()
        };
        // Preserve every origin/key and exact DOM value, including arbitrary
        // trusted-extension state; no browser evaluator flag activates native code.
        disk::atomic(&native.join("local-storage.json"), &entries)?;
        finish(&native, &mut state, Step::LocalStorage, &mut hook)?;
    }
    writer.flush()?;
    drop(writer);
    if !state.completed.contains(&Step::Extensions) {
        hook(Event::Started(Step::Extensions))?;
        extensions(&native)?;
        finish(&native, &mut state, Step::Extensions, &mut hook)?;
    }
    if state.secrets != SecretStep::Complete {
        let payloads = secrets::inventory(&root.join("profile"))?;
        if payloads.is_empty() {
            state.secrets = SecretStep::Complete;
            finish(&native, &mut state, Step::Secrets, &mut hook)?;
        } else if let Some(access) = access.as_mut() {
            ensure!(
                state.secrets == SecretStep::PromptRequired
                    || state.secrets == SecretStep::AccessStarted,
                "run first launch and show the secret prompt before granting consent"
            );
            state.secrets = SecretStep::AccessStarted;
            publish(&native, &state)?;
            let password = access.legacy_password()?;
            for payload in payloads {
                let id = format!(
                    "{}:{}",
                    payload.id,
                    blake3::hash(&payload.ciphertext).to_hex()
                );
                if state.imported_secrets.contains(&id) {
                    continue;
                }
                let plain = secrets::decrypt_mac(&password, &payload.ciphertext)?;
                access.put_native(&id, &plain)?;
                state.imported_secrets.insert(id);
                publish(&native, &state)?;
            }
            state.secrets = SecretStep::Complete;
            finish(&native, &mut state, Step::Secrets, &mut hook)?;
        } else {
            let interrupted = state.secrets == SecretStep::AccessStarted;
            if !interrupted && state.secrets != SecretStep::PromptRequired {
                state.secrets = SecretStep::PromptRequired;
                publish(&native, &state)?;
            }
            hook(Event::SecretPromptRequired)?;
            return Ok(Outcome::NeedsSecretConsent { interrupted });
        }
    }
    hook(Event::Complete)?;
    Ok(summary(&state, &pass))
}
fn read_batches(cp: &Checkpoint, root: &Path) -> Result<Vec<(Snapshot, Vec<indexeddb::Record>)>> {
    cp.indexeddb
        .iter()
        .map(|path| {
            let s = Snapshot::open(&root.join(path))?;
            let records = indexeddb::read(&s)?.records;
            Ok((s, records))
        })
        .collect()
}
fn media(
    native: &Path,
    batches: &[(Snapshot, Vec<indexeddb::Record>)],
    closure: &ClosureInput,
    hook: &mut impl FnMut(Event) -> Result<()>,
) -> Result<Pass> {
    let mut needed = 0u64;
    for (_, records) in batches {
        for r in records {
            needed = needed.saturating_add(r.payload().size);
        }
    }
    for r in &closure.references {
        if let Some(p) = &r.source_path
            && p.is_file()
        {
            needed = needed.saturating_add(fs::metadata(p)?.len());
        }
    }
    disk::space(native, needed)?;
    let mut count = 0;
    migration::migrate_many(
        &batches
            .iter()
            .map(|(s, r)| (s, r.as_slice()))
            .collect::<Vec<_>>(),
        closure,
        &native.join("Media Store"),
        |_| {
            disk::space(native, 0)?;
            count += 1;
            hook(Event::Progress {
                step: Step::Media,
                done: count,
                total: 0,
            })
        },
    )
}
fn summary(state: &State, pass: &Pass) -> Outcome {
    Outcome::Complete {
        projects: state.imported_projects.len(),
        history_refusals: state.history_refusals.len(),
        missing_projects: state.missing_projects.len(),
        blocked_owners: pass.blocked_owners.len(),
        proxy_only: pass
            .closure
            .iter()
            .filter(|e| e.status == migration::Status::ProxyOnly)
            .count(),
        already_offline: pass
            .closure
            .iter()
            .filter(|e| e.status == migration::Status::AlreadyOffline)
            .count(),
    }
}
fn bootstrap(
    cp: &Checkpoint,
    root: &Path,
    native: &Path,
    writer: &LibraryWriter,
    state: &mut State,
    hook: &mut impl FnMut(Event) -> Result<()>,
) -> Result<()> {
    let store = LibraryStore::open(&root.join("profile"));
    for key in cp.logical_keys.keys() {
        let prefix = key.split('.').next().unwrap();
        if [
            "projects",
            "projectTrash",
            "autosave",
            "project",
            "projectJournal",
            "projectState",
            "projectMeta",
            "projectHistory",
            "projectWorkspace",
        ]
        .contains(&prefix)
        {
            continue;
        }
        if state.imported_keys.contains(key) {
            continue;
        }
        let mut value = store.read(key)?.context("checkpoint key disappeared")?;
        if [
            "openWindows",
            "windowTabs",
            "lastActiveProject",
            "openTabs",
            "restoreWindows",
        ]
        .contains(&key.as_str())
        {
            let mut logical = as_json(&value)?;
            remap_project_refs(&mut logical, &cp.projects, true);
            value = to_kernel(&logical)?;
        } else if key.starts_with("agentThreads.") {
            let mut logical = as_json(&value)?;
            remap_project_refs(&mut logical, &cp.projects, false);
            value = to_kernel(&logical)?;
        }
        // Settings and storage values stay exact. Only agent-family OWNER keys
        // change to follow their copied project; attachment IDs stay stable.
        let target = if let Some((family, id)) = key.split_once('.') {
            if ["agentThreads", "agentModel"].contains(&family) {
                format!(
                    "{family}.{}",
                    cp.projects.get(id).map(String::as_str).unwrap_or(id)
                )
            } else {
                key.clone()
            }
        } else {
            key.clone()
        };
        if writer.store().read(&target)?.is_none() {
            disk::space(native, to_canonical_string(&value).len() as u64)?;
            writer.set(&target, value)?;
            writer.flush()?;
        }
        state.imported_keys.insert(key.clone());
        publish(native, state)?;
        hook(Event::Progress {
            step: Step::Bootstrap,
            done: state.imported_keys.len(),
            total: cp.logical_keys.len(),
        })?;
    }
    for (relative, entry) in &cp.files {
        if relative.starts_with("store") || state.imported_files.contains(relative) {
            continue;
        }
        let target = native.join(checkpoint::file_target(relative)?);
        disk::private_dir(target.parent().unwrap())?;
        ensure!(
            target.parent().unwrap().canonicalize()?.starts_with(native),
            "native file destination escaped profile"
        );
        if !target.exists() {
            let temp = target.with_extension("first-launch-copy");
            if temp.exists() {
                fs::remove_file(&temp)?;
            }
            let copied = disk::copy(&root.join("profile").join(relative), &temp)?;
            ensure!(copied.hash == entry.hash, "checkpoint copy differs");
            fs::rename(temp, &target)?;
            File::open(target.parent().unwrap())?.sync_all()?;
        }
        state.imported_files.insert(relative.clone());
        publish(native, state)?;
    }
    disk::private_dir(&native.join("extensions-overlay"))?;
    let shared = native.join("extensions-shared");
    if shared.is_dir() {
        fn seal(p: &Path) -> Result<()> {
            for e in fs::read_dir(p)? {
                let p = e?.path();
                let m = fs::symlink_metadata(&p)?;
                ensure!(
                    !m.file_type().is_symlink(),
                    "symlink in extension checkpoint"
                );
                if m.is_dir() {
                    seal(&p)?;
                } else {
                    let exec = m.permissions().mode() & 0o100;
                    fs::set_permissions(&p, fs::Permissions::from_mode(0o400 | exec))?;
                }
            }
            fs::set_permissions(p, fs::Permissions::from_mode(0o500))?;
            Ok(())
        }
        seal(&shared)?;
    }
    Ok(())
}

fn remap_project_refs(
    value: &mut serde_json::Value,
    ids: &std::collections::BTreeMap<String, String>,
    all_strings: bool,
) {
    match value {
        serde_json::Value::String(s) if all_strings => {
            if let Some(id) = ids.get(s) {
                *s = id.clone();
            }
        }
        serde_json::Value::Array(values) => {
            for v in values {
                remap_project_refs(v, ids, all_strings);
            }
        }
        serde_json::Value::Object(map) => {
            for (key, v) in map {
                if key == "projectId" || key == "project" || all_strings {
                    remap_project_refs(v, ids, true);
                } else {
                    remap_project_refs(v, ids, false);
                }
            }
        }
        _ => {}
    }
}
fn library(
    cp: &Checkpoint,
    root: &Path,
    native: &Path,
    writer: &LibraryWriter,
    state: &mut State,
    hook: &mut impl FnMut(Event) -> Result<()>,
) -> Result<()> {
    let store = LibraryStore::open(&root.join("profile"));
    if !root.join("profile/store").is_dir() {
        return Ok(());
    }
    let trash: BTreeSet<_> = store.trash()?.into_iter().map(|r| r.id).collect();
    for (source, id) in &cp.projects {
        if state.imported_projects.contains(source) {
            continue;
        }
        let Some(p) = store.load(source)? else {
            state.missing_projects.insert(source.clone());
            publish(native, state)?;
            continue;
        };
        let mut project = as_json(&p.project)?;
        let name = format!(
            "{} — Native",
            project
                .get("name")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("Project")
        );
        project["id"] = id.clone().into();
        project["name"] = name.into();
        let history = p.history.clone().unwrap_or_else(|| {
            parse(r#"{"version":1,"paths":"composition","index":-1,"entries":[]}"#).unwrap()
        });
        if History::import(
            &history,
            p.project.get("compId").and_then(KernelValue::as_str),
        )
        .is_err()
        {
            state.history_refusals.insert(source.clone());
        }
        let mut meta = p
            .session
            .as_ref()
            .map(as_json)
            .transpose()?
            .unwrap_or(serde_json::json!({}));
        // Library copies must never reacquire/save an external association. The
        // source association survives in the immutable 3-way checkpoint.
        if let Some(m) = meta.as_object_mut() {
            m.remove("file");
        }
        if writer.store().load(id)?.is_none() {
            let needed = serde_json::to_vec(&project)?.len() as u64
                + to_canonical_string(&history).len() as u64
                + serde_json::to_vec(&meta)?.len() as u64
                + p.workspace
                    .as_ref()
                    .map_or(0, |v| to_canonical_string(v).len() as u64);
            disk::space(native, needed.saturating_mul(3))?;
            writer.save_project(
                id,
                to_kernel(&project)?,
                history,
                p.workspace.unwrap_or_else(|| parse("{}").unwrap()),
                to_kernel(&meta)?,
            )?;
        }
        if trash.contains(source) {
            writer.trash(id)?;
        }
        state.imported_projects.insert(source.clone());
        publish(native, state)?;
        hook(Event::Progress {
            step: Step::Library,
            done: state.imported_projects.len(),
            total: cp.projects.len(),
        })?;
    }
    Ok(())
}
fn as_json(v: &KernelValue) -> Result<serde_json::Value> {
    Ok(serde_json::from_str(&to_canonical_string(v))?)
}
fn to_kernel(v: &serde_json::Value) -> Result<KernelValue> {
    parse(&serde_json::to_string(v)?).map_err(|_| anyhow::anyhow!("invalid migration JSON"))
}
fn extensions(native: &Path) -> Result<()> {
    let mut kernel = Kernel::new(KernelConfig::default());
    let result = kernel.shell(ShellCall::AttachLibrary {
        profile: native.into(),
        format: DiskFormat::V1_2,
        electron_profile: None,
    })?;
    ensure!(
        matches!(result, ShellReply::Library(OpenDecision::Writable)),
        "native library not writable"
    );
    let shared = native.join("extensions-shared");
    let mut ops = Vec::new();
    if shared.is_dir() {
        for e in fs::read_dir(&shared)? {
            let e = e?;
            let path = e.path().join("manifest.json");
            if !path.is_file() {
                continue;
            }
            let manifest: serde_json::Value = disk::json(&path)?;
            let id = e
                .file_name()
                .into_string()
                .map_err(|_| anyhow::anyhow!("invalid extension filename"))?;
            if id.starts_with('.') {
                continue;
            }
            let enabled = kernel.state().extensions.get(&id).is_none_or(|e| e.enabled);
            ops.push(StateOp::Extension(ExtensionEntry{id,installed:true,enabled,version:manifest.get("version").and_then(serde_json::Value::as_str).unwrap_or("").into(),source:to_kernel(&serde_json::json!({"shared":e.path(),"readOnly":true,"overlay":native.join("extensions-overlay")}))?}));
        }
    }
    if !ops.is_empty() {
        kernel.state_transaction(StateTransaction {
            expected_revision: None,
            ops,
        })?;
    }
    Ok(())
}

/// Fresh media-only first-launch delta, after bootstrap is complete. Settings,
/// libraries and beta edits keep their checkpoint identities. Every invocation
/// snapshots Electron again; identical captures are no-ops in pm-native-migrate.
/// The original checkpoint remains the future 3-way base.
pub fn delta(config: &Config, mut hook: impl FnMut(Event) -> Result<()>) -> Result<Outcome> {
    let (source, native) = validate(config)?;
    if older_electron_running(&source) {
        hook(Event::WaitingForElectron)?;
        return Ok(Outcome::WaitingForElectron);
    }
    let state: State = disk::json(&native.join("migration.json"))?;
    ensure!(
        state.source == source && state.bundle_id == BETA_BUNDLE_ID && state.version == 1,
        "profile migration identity mismatch"
    );
    ensure!(
        state.completed.contains(&Step::Media),
        "bootstrap must finish before a delta"
    );
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .open(native.join("first-launch.lock"))?;
    lock.try_lock_exclusive()?;
    validate_native_paths(&native)?;
    let _writer = LibraryWriter::open(&native, DiskFormat::V1_2, None)
        .map_err(|e| anyhow::anyhow!("native library unavailable: {e:?}"))?;
    let base = native.join("first-launch-checkpoint");
    ensure!(
        state.checkpoint_hash.as_ref() == Some(&disk::hash(&base.join("checkpoint.json"))?.0),
        "checkpoint manifest changed"
    );
    Checkpoint::load(&base)?;
    ensure!(
        state.availability_hash.as_ref() == Some(&disk::hash(&base.join("availability.json"))?.0),
        "availability checkpoint changed"
    );
    let deltas = native.join("first-launch-deltas");
    disk::private_dir(&deltas)?;
    // Retain all availability checkpoints, including interrupted passes. Reuse
    // the unfinished capture on resume; callers finish it before starting fresh.
    let pending = deltas.join("pending");
    let cp = if pending.exists() {
        Checkpoint::load(&pending)?
    } else {
        Checkpoint::capture(&source, &pending)?
    };
    let batches = read_batches(&cp, &pending)?;
    let availability = pending.join("availability.json");
    let closure = if availability.exists() {
        disk::json(&availability)?
    } else {
        let c = closure::inventory(&cp, &pending, &batches)?;
        disk::atomic(&availability, &c)?;
        c
    };
    let pass = media(&native, &batches, &closure, &mut hook)?;
    let identity = disk::hash(&pending.join("checkpoint.json"))?.0;
    let destination = deltas.join(format!(
        "{}-{}-{}",
        &identity[..16],
        &pass.snapshot[..16],
        &pass.base_hash[..16]
    ));
    if destination.exists() {
        fs::remove_dir_all(&pending)?;
    } else {
        fs::rename(&pending, destination)?;
        File::open(&deltas)?.sync_all()?;
    }
    hook(Event::Complete)?;
    Ok(summary(&state, &pass))
}
