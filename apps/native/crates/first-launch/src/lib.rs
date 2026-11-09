//! Native beta first-launch migration. Call on a background worker before
//! attaching the shell's library. No source profile is ever opened for writing.
//! Checkpoints contain private user state; keep them out of Git/telemetry.
mod checkpoint;
mod closure;
mod delta;
mod disk;
pub mod integration;
pub mod recovery;
pub mod report;
pub mod secrets;
pub mod sparkle;
pub mod store_preparation;
pub mod surfaces;
pub use checkpoint::Checkpoint;
pub use delta::{
    Conflict, ConflictKind, DeltaPrepared, delta, delta_if_changed, delta_media, delta_store,
};
pub use disk::LowDisk;

use anyhow::{Context, Result, ensure};
use fs2::FileExt;
use pm_kernel::{
    Kernel, KernelConfig, LibraryStore,
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

/// The native profile folder can't be used: linked, foreign or overlapping.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProfileRefused;
impl std::fmt::Display for ProfileRefused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("the native library folder can't be used")
    }
}
impl std::error::Error for ProfileRefused {}
/// macOS answered no, or never answered. A fresh click is the only retry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeychainDenied;
impl std::fmt::Display for KeychainDenied {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Keychain access wasn't allowed")
    }
}
impl std::error::Error for KeychainDenied {}
/// A preserved input requiring a real adapter, not a success-shaped capability.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MigrationRequirement {
    MainFileMediaAdapter,
}
impl std::fmt::Display for MigrationRequirement {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("main_file_media_adapter_required: preserved main-reference Media Store contains records unsupported by the native importer")
    }
}
impl std::error::Error for MigrationRequirement {}
fn require_supported_media(cp: &Checkpoint, checkpoint_root: &Path) -> Result<()> {
    let paths = cp
        .files
        .keys()
        .filter(|path| {
            path.starts_with("Media Store/Index")
                || (path.starts_with("Media Store") && path.components().count() == 2)
        })
        .collect::<Vec<_>>();
    if paths.is_empty() {
        return Ok(());
    }
    // delta-data supplies the real verified file-index importer. Keep beta2's
    // refusal for malformed/unrecognized records and unindexed owned payloads;
    // merely preserving their bytes is not evidence of media migration.
    let media_root = checkpoint_root.join("profile/Media Store");
    let records = pm_native_migrate::library_media::read_index(&media_root).map_err(|error| {
        anyhow::Error::new(MigrationRequirement::MainFileMediaAdapter).context(error)
    })?;
    let mut admitted = std::collections::BTreeSet::new();
    for record in records {
        use sha2::{Digest, Sha256};
        admitted.insert(PathBuf::from(format!(
            "Media Store/Index/{:x}.json",
            Sha256::digest(record.key.as_bytes())
        )));
        if let Some(owned) = record.owned_path {
            let name = owned.file_name().context("owned media has no filename")?;
            admitted.insert(Path::new("Media Store").join(name));
        }
    }
    if paths.iter().any(|path| !admitted.contains(*path)) {
        return Err(MigrationRequirement::MainFileMediaAdapter.into());
    }
    Ok(())
}
pub const BETA_BUNDLE_ID: &str = "com.zellzoi.powermove.native-beta";
pub fn beta_profile(application_support: &Path) -> PathBuf {
    application_support.join(BETA_BUNDLE_ID)
}
/// The shipped Electron profile folder inside Application Support.
pub const ELECTRON_FOLDER: &str = "Powermove";
/// The one profile layout shared by the app and this coordinator.
pub fn default_paths(application_support: &Path) -> Config {
    Config {
        electron_profile: application_support.join(ELECTRON_FOLDER),
        native_profile: beta_profile(application_support),
    }
}
/// What this launch has to do, from local file metadata only.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Launch {
    /// No original app on this Mac: onboarding, then the editor.
    Fresh,
    /// A source exists and bootstrap isn't finished: the gate runs.
    Migrate,
    /// Bootstrap finished earlier: no gate, later changes come in behind it.
    Later,
}
pub fn detect(config: &Config) -> Launch {
    let state = config.native_profile.join("migration.json");
    if disk::json::<State>(&state).is_ok_and(|s| s.completed.contains(&Step::Secrets)) {
        Launch::Later
    } else if config.electron_profile.is_dir()
        && (state.exists() || !config.native_profile.join("store").exists())
    {
        Launch::Migrate
    } else {
        // Work began here before the original app appeared. It stays as it is.
        Launch::Fresh
    }
}
/// Copied from the source, or written when onboarding finishes.
pub fn onboarding_completed(native: &Path) -> bool {
    native.join("onboarding-v1.json").is_file()
}
#[derive(Clone, Debug, PartialEq, Eq)]
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
    /// Per source project: what was imported, so a later delta can tell a
    /// native edit from an edit in the original app.
    #[serde(default)]
    pub receipts: std::collections::BTreeMap<String, delta::ProjectReceipt>,
    /// Newest merged delta checkpoint, the three-way base of the next one.
    #[serde(default)]
    pub delta_base: Option<String>,
    /// Both versions kept. The original's bytes stay in its delta checkpoint.
    #[serde(default)]
    pub conflicts: BTreeSet<delta::Conflict>,
    /// Source metadata stamp of the newest merged capture.
    #[serde(default)]
    pub source_stamp: Option<String>,
    /// Snapshot id of the media pass that describes the library now.
    #[serde(default)]
    pub latest_pass: Option<String>,
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
            return delta::delta(config, hook);
        }
    }
    run_inner(config, None, false, false, hook)
}
/// The host must display `secrets::PROMPT` and receive a fresh user approval
/// before each invocation of this opt-in entry point. A crash after access began
/// leaves AccessStarted, and ordinary `run` never retries that Keychain access.
pub fn resume_with_secret_consent(
    config: &Config,
    access: &mut dyn SecretAccess,
    hook: impl FnMut(Event) -> Result<()>,
) -> Result<Outcome> {
    run_inner(config, Some(access), false, false, hook)
}
/// N002 Not Now / Continue Without Them. Finishes the isolated beta import,
/// retaining every encrypted input and imported locator. Never accesses Keychain.
/// A later ordinary launch/delta cannot turn this choice into another prompt.
pub fn skip_secret_consent(
    config: &Config,
    hook: impl FnMut(Event) -> Result<()>,
) -> Result<Outcome> {
    run_inner(config, None, true, false, hook)
}
/// Settings ▸ Accounts ▸ Bring Sign-Ins from the Original App…; call only after a
/// fresh explicit consent. Reads current encrypted source (including deletions),
/// preserves the original checkpoint and never reimports settings/library edits.
pub fn retry_secret_consent(
    config: &Config,
    access: &mut dyn SecretAccess,
    hook: impl FnMut(Event) -> Result<()>,
) -> Result<Outcome> {
    run_inner(config, Some(access), false, true, hook)
}
fn validate(config: &Config) -> Result<(PathBuf, PathBuf)> {
    checked_paths(config).context(ProfileRefused)
}
fn checked_paths(config: &Config) -> Result<(PathBuf, PathBuf)> {
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
/// The engine's compiled-shader cache folder in the native profile.
pub const SHADER_CACHE: &str = "shader-cache";
fn fresh_destination(native: &Path) -> Result<()> {
    if native.exists() && !native.join("migration.json").exists() {
        // A kill may occur after creating the process lock or temporary state
        // file, before the first state rename. Those two private artifacts are
        // safe to resume. The engine starts before this gate and keeps its
        // compiled-shader cache here: derived data, never work. Any other
        // preexisting destination remains refused.
        for entry in fs::read_dir(native)? {
            let entry = entry?;
            if entry.file_name() == SHADER_CACHE && entry.file_type()?.is_dir() {
                continue;
            }
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
    Ok(())
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
    skip_secrets: bool,
    retry_current_secrets: bool,
    mut hook: impl FnMut(Event) -> Result<()>,
) -> Result<Outcome> {
    let (source, native) = validate(config)?;
    // Waiting performs no writes, not even to the native destination. Never
    // remove, follow, or acquire Electron's SingletonLock.
    if older_electron_running(&source) {
        hook(Event::WaitingForElectron)?;
        return Ok(Outcome::WaitingForElectron);
    }
    fresh_destination(&native).context(ProfileRefused)?;
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
            receipts: Default::default(),
            delta_base: None,
            conflicts: BTreeSet::new(),
            source_stamp: None,
            latest_pass: None,
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
        // Stamped before capture; capture itself proves nothing moved after.
        state.source_stamp = Some(checkpoint::source_stamp(&source)?);
        publish(&native, &state)?;
        Checkpoint::capture(&source, &root)?;
    }
    let cp = Checkpoint::load(&root)?;
    require_supported_media(&cp, &root)?;
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
    let pass = media(&native, &root, &batches, &closure, &mut hook)?;
    if !state.completed.contains(&Step::Media) {
        state.latest_pass = Some(pass.snapshot.clone());
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
        pm_kernel::profile::ProfileStorage::new(&writer).import_local_storage(
            &entries
                .into_iter()
                .map(|e| pm_kernel::profile::LocalStorageEntry {
                    origin: e.origin,
                    key: e.key,
                    value: e.value,
                })
                .collect::<Vec<_>>(),
        )?;
        finish(&native, &mut state, Step::LocalStorage, &mut hook)?;
    }
    writer.flush()?;
    drop(writer);
    if !state.completed.contains(&Step::Extensions) {
        hook(Event::Started(Step::Extensions))?;
        extensions(&native, Vec::new())?;
        finish(&native, &mut state, Step::Extensions, &mut hook)?;
    }
    if state.secrets != SecretStep::Complete || retry_current_secrets {
        let retry =
            retry_current_secrets || (access.is_some() && state.completed.contains(&Step::Secrets));
        if retry_current_secrets {
            ensure!(
                state.completed.contains(&Step::Secrets),
                "finish first launch before retrying sign-ins from Settings"
            );
        }
        let secret_profile = if retry {
            secrets::capture_retry(&source, &native)?
        } else {
            root.join("profile")
        };
        let payloads = secrets::inventory(&secret_profile)?;
        if payloads.is_empty() {
            state.secrets = SecretStep::Complete;
            finish(&native, &mut state, Step::Secrets, &mut hook)?;
        } else if skip_secrets {
            ensure!(
                matches!(
                    state.secrets,
                    SecretStep::PromptRequired | SecretStep::AccessStarted | SecretStep::Skipped
                ),
                "show the consent choice before declining it"
            );
            state.secrets = SecretStep::Skipped;
            finish(&native, &mut state, Step::Secrets, &mut hook)?;
        } else if let Some(access) = access.as_mut() {
            ensure!(
                state.secrets == SecretStep::PromptRequired
                    || state.secrets == SecretStep::AccessStarted
                    || state.secrets == SecretStep::Skipped
                    || retry_current_secrets,
                "run first launch and show the secret prompt before granting consent"
            );
            state.secrets = SecretStep::AccessStarted;
            publish(&native, &state)?;
            hook(Event::Started(Step::Secrets))?;
            let password = access.legacy_password().context(KeychainDenied)?;
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
                // Persist only the native item's location; plaintext never enters the kernel.
                pm_kernel::profile::ProfileStorage::new(
                    &LibraryWriter::open(&native, pm_kernel::persistence::DiskFormat::V1_2, None)
                        .map_err(|e| anyhow::anyhow!("{e:?}"))?,
                )
                .register_secret(&pm_kernel::profile::SecretLocator {
                    id: payload.id.clone(),
                    version: blake3::hash(&payload.ciphertext).to_hex().to_string(),
                    service: format!("{} migration secrets", BETA_BUNDLE_ID),
                    account: blake3::hash(id.as_bytes()).to_hex().to_string(),
                })?;
                state.imported_secrets.insert(id);
                publish(&native, &state)?;
            }
            state.secrets = SecretStep::Complete;
            finish(&native, &mut state, Step::Secrets, &mut hook)?;
        } else if state.secrets != SecretStep::Skipped {
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
    checkpoint_root: &Path,
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
    let disk_records =
        pm_native_migrate::library_media::read_index(&checkpoint_root.join("profile/Media Store"))?;
    // One record publication per source record, however many objects it has.
    let total = batches.iter().map(|(_, r)| r.len()).sum::<usize>() + disk_records.len();
    migration::migrate_many_with_disk(
        &batches
            .iter()
            .map(|(s, r)| (s, r.as_slice()))
            .collect::<Vec<_>>(),
        &disk_records,
        closure,
        &native.join("Media Store"),
        |checkpoint| {
            disk::space(native, 0)?;
            if checkpoint != migration::Checkpoint::RecordPublished {
                return Ok(());
            }
            count += 1;
            hook(Event::Progress {
                step: Step::Media,
                done: count,
                total,
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
        if project_key(key) || state.imported_keys.contains(key) {
            continue;
        }
        let value = store.read(key)?.context("checkpoint key disappeared")?;
        let (target, value) = imported_key(key, value, &cp.projects)?;
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
        if !target.exists() {
            place_file(&root.join("profile").join(relative), &target, entry, native)?;
        }
        state.imported_files.insert(relative.clone());
        publish(native, state)?;
    }
    disk::private_dir(&native.join("extensions-overlay"))?;
    seal_shared(&native.join("extensions-shared"))
}
fn seal_shared(shared: &Path) -> Result<()> {
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
    if shared.is_dir() {
        seal(shared)?;
    }
    Ok(())
}
/// Verified copy into a native path, published by rename. Never leaves the profile.
fn place_file(from: &Path, target: &Path, entry: &disk::FileEntry, native: &Path) -> Result<()> {
    disk::private_dir(target.parent().unwrap())?;
    ensure!(
        target.parent().unwrap().canonicalize()?.starts_with(native),
        "native file destination escaped profile"
    );
    let temp = target.with_extension("first-launch-copy");
    if temp.exists() {
        disk::archive(&temp)?;
    }
    let copied = disk::copy(from, &temp)?;
    ensure!(copied.hash == entry.hash, "checkpoint copy differs");
    fs::rename(temp, target)?;
    File::open(target.parent().unwrap())?.sync_all()?;
    Ok(())
}
const PROJECT_FAMILIES: [&str; 9] = [
    "projects",
    "projectTrash",
    "autosave",
    "project",
    "projectJournal",
    "projectState",
    "projectMeta",
    "projectHistory",
    "projectWorkspace",
];
fn project_key(key: &str) -> bool {
    PROJECT_FAMILIES.contains(&key.split('.').next().unwrap_or(key))
}
/// A nonproject key as the native library stores it. Settings and storage
/// values stay exact. Only agent-family OWNER keys follow their copied project;
/// attachment IDs stay stable.
fn imported_key(
    key: &str,
    mut value: KernelValue,
    projects: &std::collections::BTreeMap<String, String>,
) -> Result<(String, KernelValue)> {
    if ["workspaces", "workspaceTrash"].contains(&key) {
        let raw = as_json(&value)?;
        if let Some(items) = raw.as_array() {
            value = to_kernel(&serde_json::Value::Array(
                items
                    .iter()
                    .map(pm_native_migrate::workspace::migrate)
                    .collect(),
            ))?;
        }
    }
    if [
        "openWindows",
        "windowTabs",
        "lastActiveProject",
        "openTabs",
        "restoreWindows",
    ]
    .contains(&key)
    {
        let mut logical = as_json(&value)?;
        remap_project_refs(&mut logical, projects, true);
        value = to_kernel(&logical)?;
    } else if key.starts_with("agentThreads.") {
        let mut logical = as_json(&value)?;
        remap_project_refs(&mut logical, projects, false);
        value = to_kernel(&logical)?;
    }
    let target = match key.split_once('.') {
        Some((family, id)) if ["agentThreads", "agentModel"].contains(&family) => format!(
            "{family}.{}",
            projects.get(id).map(String::as_str).unwrap_or(id)
        ),
        _ => key.to_owned(),
    };
    Ok((target, value))
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
        let copy = delta::project_copy(&p, id)?;
        if copy.refused {
            state.history_refusals.insert(source.clone());
        }
        if writer.store().load(id)?.is_none() {
            disk::space(native, copy.bytes().saturating_mul(3))?;
            copy.save(writer, id)?;
        }
        state.receipts.insert(source.clone(), copy.receipt(id));
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
/// Registers shared extension copies and applies `ops`, through the kernel so
/// its durable state record stays the one authority on settings.
fn extensions(native: &Path, mut ops: Vec<StateOp>) -> Result<()> {
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
    // Removed from the original app: unregistered. The overlay and its checkpoint stay.
    let gone = kernel
        .state()
        .extensions
        .values()
        .filter(|e| e.installed && !shared.join(&e.id).join("manifest.json").is_file())
        .map(|e| {
            let mut entry = e.clone();
            entry.installed = false;
            StateOp::Extension(entry)
        })
        .collect::<Vec<_>>();
    ops.extend(gone);
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
            let pm_kernel::profile::ProfileReply::Sources(sources) = kernel
                .profile(pm_kernel::profile::ProfileCall::ExtensionSources { id: id.clone() })?
            else {
                unreachable!()
            };
            ops.push(StateOp::Extension(ExtensionEntry {
                id,
                installed: true,
                enabled,
                version: manifest
                    .get("version")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("")
                    .into(),
                source: to_kernel(&serde_json::json!({"shared":sources.shared,
                    "readOnly":sources.shared_read_only,"overlay":sources.overlay}))?,
            }));
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

/// Result of an explicit Settings retry. No secret count or decrypted value is
/// part of the UI contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecretRetryOutcome {
    WaitingForElectron,
    Complete,
}
/// Bring Sign-Ins from the Original App… while the native library is attached.
/// Run on a worker after fresh consent. `kernel` must own `config.native_profile`.
/// Locators go through its actor; this never creates a competing LibraryWriter.
/// It reads fresh encrypted source and retains every old checkpoint/receipt.
pub fn retry_secret_consent_in_library(
    config: &Config,
    kernel: &pm_kernel::KernelHandle,
    access: &mut dyn SecretAccess,
    mut hook: impl FnMut(Event) -> Result<()>,
) -> Result<SecretRetryOutcome> {
    let (source, native) = validate(config)?;
    if older_electron_running(&source) {
        return Ok(SecretRetryOutcome::WaitingForElectron);
    }
    let lease = OpenOptions::new()
        .read(true)
        .write(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(native.join("first-launch.lock"))?;
    lease
        .try_lock_exclusive()
        .context("first launch already running")?;
    validate_native_paths(&native)?;
    let mut state: State = disk::json(&native.join("migration.json"))?;
    ensure!(
        state.version == 1 && state.bundle_id == BETA_BUNDLE_ID && state.source == source,
        "profile migration identity mismatch"
    );
    ensure!(
        state.completed.contains(&Step::Secrets),
        "finish first launch before retrying sign-ins from Settings"
    );
    let profile = secrets::capture_retry(&source, &native)?;
    let payloads = secrets::inventory(&profile)?;
    let pending: Vec<_> = payloads
        .into_iter()
        .filter(|p| {
            !state.imported_secrets.contains(&format!(
                "{}:{}",
                p.id,
                blake3::hash(&p.ciphertext).to_hex()
            ))
        })
        .collect();
    if !pending.is_empty() {
        state.secrets = SecretStep::AccessStarted;
        publish(&native, &state)?;
        hook(Event::Started(Step::Secrets))?;
        let password = access.legacy_password().context(KeychainDenied)?;
        for payload in pending {
            let version = blake3::hash(&payload.ciphertext).to_hex().to_string();
            let id = format!("{}:{version}", payload.id);
            let plain = secrets::decrypt_mac(&password, &payload.ciphertext)?;
            access.put_native(&id, &plain)?;
            let locator = pm_kernel::profile::SecretLocator {
                id: payload.id,
                version,
                service: format!("{BETA_BUNDLE_ID} migration secrets"),
                account: blake3::hash(id.as_bytes()).to_hex().to_string(),
            };
            kernel
                .profile(pm_kernel::profile::ProfileCall::RegisterSecret(locator))
                .wait()
                .context("native kernel stopped during secret registration")??;
            state.imported_secrets.insert(id);
            publish(&native, &state)?;
        }
    }
    state.secrets = SecretStep::Complete;
    finish(&native, &mut state, Step::Secrets, &mut hook)?;
    Ok(SecretRetryOutcome::Complete)
}
