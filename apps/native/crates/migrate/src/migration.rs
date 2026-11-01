//! Additive media migration. The caller supplies an availability checkpoint for
//! the COMPLETE reference closure (including history/trash/takes/agent owners).
//! The importer never deletes source data, objects, mappings, or previous passes.
//! A project with a newly missing payload is blocked individually. An incomplete
//! inventory, unknown original identity or a mapping conflict prevents cutover.
use crate::{
    Result,
    indexeddb::{Record, v2_fingerprint},
    snapshot::{Snapshot, atomic_json, check_space, copy_verified_length, hash_file, open_regular},
};
use anyhow::{Context, ensure};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File, OpenOptions},
    io::{Read, Seek, SeekFrom},
    os::unix::fs::{OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct KeyRecord {
    pub key: String,
    pub hash: String,
    pub size: u64,
    #[serde(rename = "type")]
    pub mime: String,
    pub revision: String,
    pub at: f64,
    pub source: String,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "kebab-case")]
pub enum OwnerKind {
    Current,
    Trash,
    Take,
    Undo,
    Redo,
    Agent,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum Availability {
    Original,
    ProxyOnly,
    Offline,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Reference {
    /// Local manifest identifier only. Never print it in telemetry.
    pub owner: String,
    pub owner_kind: OwnerKind,
    pub key: String,
    #[serde(default)]
    pub legacy_id: Option<String>,
    #[serde(default)]
    pub revision: Option<String>,
    #[serde(default)]
    pub fingerprint: Option<String>,
    #[serde(default)]
    pub original_blake3: Option<String>,
    #[serde(default)]
    pub source_path: Option<PathBuf>,
    #[serde(default)]
    pub playback_proxy: bool,
    #[serde(default)]
    pub playback_proxy_version: u64,
    #[serde(default)]
    pub poster: bool,
    pub before: Availability,
}
impl Reference {
    pub fn is_proxy(&self) -> bool {
        self.playback_proxy || self.playback_proxy_version > 0
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClosureInput {
    /// Must name ALL six domains, even if a domain has zero references.
    pub inventoried: BTreeSet<OwnerKind>,
    pub references: Vec<Reference>,
}
impl ClosureInput {
    pub fn complete(&self) -> bool {
        [
            OwnerKind::Current,
            OwnerKind::Trash,
            OwnerKind::Take,
            OwnerKind::Undo,
            OwnerKind::Redo,
            OwnerKind::Agent,
        ]
        .into_iter()
        .all(|k| self.inventoried.contains(&k))
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum Status {
    VerifiedOriginal,
    ProxyOnly,
    AlreadyOffline,
    Unclassified,
    NewlyMissing,
    RegeneratePoster,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClosureEntry {
    pub reference: Reference,
    pub status: Status,
    pub original: Option<KeyRecord>,
    pub stored: Option<KeyRecord>,
    pub export_warning: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Imported {
    /// Full source metadata stays in the private checkpoint, including unknown
    /// fields and external File metadata; host key JSON stays schema-compatible.
    pub source_record: Record,
    pub stored: Option<KeyRecord>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum Phase {
    Copying,
    CheckingClosure,
    Complete,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Pass {
    pub phase: Phase,
    pub snapshot: String,
    pub base_hash: String,
    pub records: BTreeMap<String, Imported>,
    pub closure: Vec<ClosureEntry>,
    pub blocked_owners: BTreeSet<String>,
    pub mapping_conflicts: BTreeSet<String>,
    pub cutover_ready: bool,
}
#[derive(Default, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct State {
    version: u32,
    passes: BTreeMap<String, Pass>,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Checkpoint {
    ObjectPublished,
    RecordPublished,
    ClosurePublished,
}

/// Returns a private manifest, never a list suitable for telemetry. `hook` is
/// for progress and crash-injection tests; errors leave the last atomic step.
pub fn migrate(
    snapshot: &Snapshot,
    records: &[Record],
    closure: &ClosureInput,
    root: &Path,
    mut hook: impl FnMut(Checkpoint) -> Result<()>,
) -> Result<Pass> {
    ensure!(
        snapshot.complete(),
        "sampled snapshots cannot authorize a media migration"
    );
    ensure!(
        records.iter().all(|r| r.payload_index < r.external.len()),
        "invalid media payload index"
    );
    snapshot.ensure_destination(root)?;
    fs::create_dir_all(root)?;
    fs::set_permissions(root, fs::Permissions::from_mode(0o700))?;
    check_space(root, 0)?;
    // Serialization is per destination; source is never locked or opened by a
    // database engine. TR/native must also hold the app's library lock.
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(root.join("migration.lock"))?;
    lock.try_lock_exclusive()
        .context("migration already running")?;
    let _lock = lock;
    for dir in ["objects", "keys", "tmp"] {
        fs::create_dir_all(root.join(dir))?;
    }
    let snapshot_id = snapshot.identity()?;
    let base_hash = blake3::hash(&serde_json::to_vec(&(records, closure))?)
        .to_hex()
        .to_string();
    let id = format!("{snapshot_id}:{base_hash}");
    let state_file = root.join("migration.json");
    let mut state: State = if state_file.exists() {
        serde_json::from_reader(open_regular(&state_file)?)?
    } else {
        State {
            version: 1,
            ..State::default()
        }
    };
    ensure!(state.version == 1, "unsupported migration checkpoint");
    let mut pass = state.passes.get(&id).cloned().unwrap_or(Pass {
        phase: Phase::Copying,
        snapshot: snapshot_id,
        base_hash,
        records: BTreeMap::new(),
        closure: Vec::new(),
        blocked_owners: BTreeSet::new(),
        mapping_conflicts: BTreeSet::new(),
        cutover_ready: false,
    });
    if pass.phase == Phase::Complete {
        verify_pass(root, &pass)?;
        return Ok(pass); // No timestamp/state rewrite, even on a subsequent launch.
    }
    save(&mut state, &id, &pass, &state_file)?;
    let mut seen = BTreeSet::new();
    for record in records {
        let rid = record_id(record);
        ensure!(seen.insert(rid.clone()), "duplicate input record");
        if let Some(done) = pass.records.get(&rid) {
            if let Some(key) = &done.stored {
                verify_key(root, key)?;
            }
            continue;
        }
        let payload = record.payload();
        let path = record.payload_path(snapshot)?;
        let stored = if let Some(path) = path {
            let key = import_file(
                root,
                &path,
                payload.size,
                ImportInfo {
                    key: &record.key,
                    mime: &payload.mime,
                    revision: record.revision(),
                    at: record.value.get("at").and_then(Value::as_f64),
                    expected: None,
                },
            )?;
            hook(Checkpoint::ObjectPublished)?;
            Some(key)
        } else if payload.size == 0 {
            Some(import_empty(
                root,
                &record.key,
                &payload.mime,
                record.revision(),
            )?)
        } else {
            None
        };
        // Checkpoint the manifest BEFORE publishing the public key. On conflict
        // keep the existing key and both objects. Resolution belongs to cutover.
        pass.records.insert(
            rid,
            Imported {
                source_record: record.clone(),
                stored: stored.clone(),
            },
        );
        save(&mut state, &id, &pass, &state_file)?;
        if let Some(key) = stored {
            publish_key(root, &key, &mut pass.mapping_conflicts)?;
        }
        save(&mut state, &id, &pass, &state_file)?;
        hook(Checkpoint::RecordPublished)?;
    }
    // Resume a crash between the record checkpoint and its key publication.
    for r in pass.records.values() {
        if let Some(k) = &r.stored {
            publish_key(root, k, &mut pass.mapping_conflicts)?;
        }
    }
    pass.phase = Phase::CheckingClosure;
    save(&mut state, &id, &pass, &state_file)?;
    pass.closure.clear();
    pass.blocked_owners.clear();
    for reference in &closure.references {
        let matches: Vec<&Imported> = pass
            .records
            .values()
            .filter(|r| {
                r.source_record.key == reference.key
                    || reference.legacy_id.as_deref() == Some(&r.source_record.key)
            })
            .collect();
        let selected = select_record(&matches, reference)?;
        let stored = selected.and_then(|r| r.stored.clone());
        // The captured record remains a rendition when project metadata says so,
        // even if a sampled legacy fingerprint happens to collide.
        let mut original = None;
        if !reference.poster {
            if let Some(path) = &reference.source_path
                && path.is_file()
                && identity_matches(path, reference)?
            {
                let size = fs::metadata(path)?.len();
                original = Some(import_file(
                    root,
                    path,
                    size,
                    ImportInfo {
                        key: &format!("blake3-source:{}", sha256(&serde_json::to_vec(reference)?)),
                        mime: "application/octet-stream",
                        revision: None,
                        at: None,
                        expected: reference.original_blake3.as_deref(),
                    },
                )?);
                if let Some(k) = original.as_mut() {
                    ensure!(
                        identity_matches(&object_path(root, &k.hash)?, reference)?,
                        "original changed during migration"
                    );
                    k.source = "import".into();
                }
            }
            if original.is_none()
                && !reference.is_proxy()
                && let Some(key) = &stored
                && identity_matches(&object_path(root, &key.hash)?, reference)?
            {
                original = Some(key.clone());
            }
        }
        let status = if reference.poster {
            if stored.is_some() {
                Status::VerifiedOriginal
            } else {
                Status::RegeneratePoster
            }
        } else if original.is_some() {
            Status::VerifiedOriginal
        } else if stored.is_some() && reference.is_proxy() {
            Status::ProxyOnly
        } else if stored.is_some() {
            // A sampled mismatch can mean a transformed TIFF/HEIC/animation.
            // Preserve it and warn; never call those bytes a verified original.
            if reference.fingerprint.is_some() {
                Status::ProxyOnly
            } else {
                Status::Unclassified
            }
        } else if reference.before == Availability::Offline {
            Status::AlreadyOffline
        } else {
            Status::NewlyMissing
        };
        // Availability can improve. Losing an available original (even when a
        // proxy remains), or losing an available proxy, blocks that owner.
        let lost = match reference.before {
            Availability::Original => original.is_none() && !reference.poster,
            Availability::ProxyOnly => original.is_none() && stored.is_none() && !reference.poster,
            Availability::Offline => false,
        };
        if lost || status == Status::NewlyMissing || status == Status::Unclassified {
            pass.blocked_owners.insert(reference.owner.clone());
        }
        let export_warning = matches!(status, Status::ProxyOnly | Status::Unclassified);
        pass.closure.push(ClosureEntry {
            reference: reference.clone(),
            status,
            original,
            stored,
            export_warning,
        });
        save(&mut state, &id, &pass, &state_file)?;
    }
    // Missing *unreferenced* blobs also remain visible in the checkpoint. A
    // complete source snapshot cannot silently omit available orphaned records.
    pass.cutover_ready =
        closure.complete() && pass.blocked_owners.is_empty() && pass.mapping_conflicts.is_empty();
    verify_pass(root, &pass)?;
    hook(Checkpoint::ClosurePublished)?;
    pass.phase = Phase::Complete;
    save(&mut state, &id, &pass, &state_file)?;
    Ok(pass)
}
fn save(state: &mut State, id: &str, pass: &Pass, path: &Path) -> Result<()> {
    state.passes.insert(id.to_owned(), pass.clone());
    atomic_json(path, state)
}
fn record_id(r: &Record) -> String {
    format!("{}:{}:{}", r.database, r.store, sha256(r.key.as_bytes()))
}
fn sha256(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}
fn select_record<'a>(records: &[&'a Imported], r: &Reference) -> Result<Option<&'a Imported>> {
    let exact: Vec<_> = records
        .iter()
        .copied()
        .filter(|i| i.source_record.key == r.key)
        .collect();
    let candidates = if exact.is_empty() {
        records.to_vec()
    } else {
        exact
    };
    let candidates: Vec<_> = candidates
        .into_iter()
        .filter(|i| {
            r.revision
                .as_ref()
                .is_none_or(|rev| i.source_record.revision() == Some(rev))
        })
        .collect();
    ensure!(
        candidates.len() <= 1,
        "ambiguous media reference across databases/stores"
    );
    Ok(candidates.first().copied())
}
fn valid_hash(hash: &str) -> bool {
    hash.len() == 64
        && hash
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn object_path(root: &Path, hash: &str) -> Result<PathBuf> {
    ensure!(valid_hash(hash), "invalid object hash");
    Ok(root.join("objects").join(&hash[..2]).join(hash))
}
fn verify_key(root: &Path, key: &KeyRecord) -> Result<()> {
    let (hash, size) = hash_file(&object_path(root, &key.hash)?)?;
    ensure!(
        hash == key.hash && size == key.size,
        "published media verification failed"
    );
    Ok(())
}
fn verify_pass(root: &Path, pass: &Pass) -> Result<()> {
    let mut seen = BTreeSet::new();
    for k in pass
        .records
        .values()
        .filter_map(|r| r.stored.as_ref())
        .chain(
            pass.closure
                .iter()
                .flat_map(|e| [e.original.as_ref(), e.stored.as_ref()])
                .flatten(),
        )
    {
        if seen.insert(&k.hash) {
            verify_key(root, k)?;
        }
    }
    for item in pass.records.values() {
        if let Some(key) = &item.stored
            && !pass.mapping_conflicts.contains(&key.key)
        {
            let file = root
                .join("keys")
                .join(format!("{}.json", sha256(key.key.as_bytes())));
            let actual: KeyRecord = serde_json::from_reader(open_regular(&file)?)?;
            ensure!(
                same_mapping(&actual, key),
                "published key mapping changed or missing"
            );
        }
    }
    Ok(())
}
fn same_mapping(a: &KeyRecord, b: &KeyRecord) -> bool {
    a.key == b.key
        && a.hash == b.hash
        && a.size == b.size
        && a.mime == b.mime
        && a.revision == b.revision
}
fn publish_key(root: &Path, key: &KeyRecord, conflicts: &mut BTreeSet<String>) -> Result<()> {
    ensure!(
        !key.key.is_empty()
            && key.key.len() <= 300
            && key
                .key
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b":._-".contains(&b)),
        "legacy key is outside host store schema; checkpoint retained"
    );
    ensure!(
        key.mime.len() <= 200
            && !key.revision.is_empty()
            && key.revision.len() <= 200
            && key.at.is_finite()
            && key.size < (1u64 << 53),
        "media metadata is outside host store schema"
    );
    let target = root
        .join("keys")
        .join(format!("{}.json", sha256(key.key.as_bytes())));
    if target.exists() {
        let old: KeyRecord = serde_json::from_reader(open_regular(&target)?)?;
        verify_key(root, &old)?;
        if !same_mapping(&old, key) {
            conflicts.insert(key.key.clone());
        }
    } else {
        atomic_json(&target, key)?;
    }
    Ok(())
}
fn import_empty(root: &Path, key: &str, mime: &str, revision: Option<&str>) -> Result<KeyRecord> {
    let part = root.join("tmp/empty.part");
    let f = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(&part)?;
    f.sync_all()?;
    drop(f);
    let result = import_file(
        root,
        &part,
        0,
        ImportInfo {
            key,
            mime,
            revision,
            at: Some(0.0),
            expected: None,
        },
    );
    fs::remove_file(&part)?;
    result
}
struct ImportInfo<'a> {
    key: &'a str,
    mime: &'a str,
    revision: Option<&'a str>,
    at: Option<f64>,
    expected: Option<&'a str>,
}
fn import_file(root: &Path, source: &Path, size: u64, info: ImportInfo<'_>) -> Result<KeyRecord> {
    let ImportInfo {
        key,
        mime,
        revision,
        at,
        expected,
    } = info;
    check_space(root, size)?;
    let (hash, read_size) = hash_file(source)?;
    ensure!(read_size == size, "source length changed");
    if let Some(expected) = expected {
        ensure!(hash == expected, "source BLAKE3 differs from reference");
    }
    let target = object_path(root, &hash)?;
    if !target.exists() {
        let temp = root.join("tmp").join(format!("{hash}.part"));
        // Only our migration lock holder can own this deterministic staging file.
        if temp.exists() {
            fs::remove_file(&temp)?;
        }
        copy_verified_length(source, &temp, size)?;
        let (stored_hash, stored_size) = hash_file(&temp)?;
        ensure!(
            stored_hash == hash && stored_size == size,
            "copied object BLAKE3 mismatch"
        );
        fs::create_dir_all(target.parent().unwrap())?;
        fs::set_permissions(&temp, fs::Permissions::from_mode(0o400))?;
        fs::rename(&temp, &target)?;
        File::open(target.parent().unwrap())?.sync_all()?;
    }
    let record = KeyRecord {
        key: key.into(),
        hash: hash.clone(),
        size,
        mime: mime.into(),
        revision: revision.filter(|s| !s.is_empty()).unwrap_or(&hash).into(),
        at: at.unwrap_or(0.0),
        source: "indexeddb".into(),
    };
    verify_key(root, &record)?;
    Ok(record)
}
fn identity_matches(path: &Path, r: &Reference) -> Result<bool> {
    if let Some(hash) = &r.original_blake3 {
        ensure!(valid_hash(hash), "invalid expected BLAKE3");
        return Ok(hash_file(path)?.0 == *hash);
    }
    if let Some(fp) = &r.fingerprint {
        v2_fingerprint(fp)?;
        return Ok(sampled_fingerprint(path)? == *fp);
    }
    Ok(false)
}
/// Exact shipped `v2` rule: deduplicated 64 KiB start/middle/end samples,
/// prefixed with `powermove-media-v2\n<size>\n`. Full BLAKE3 is separate.
pub fn sampled_fingerprint(path: &Path) -> Result<String> {
    let mut file = open_regular(path)?;
    let size = file.metadata()?.len();
    let sample = size.min(64 * 1024);
    let offsets = [0, (size - sample) / 2, size - sample];
    let mut seen = BTreeSet::new();
    let mut hash = Sha256::new();
    hash.update(format!("powermove-media-v2\n{size}\n").as_bytes());
    let mut buffer = vec![0; sample as usize];
    for offset in offsets {
        if seen.insert(offset) {
            file.seek(SeekFrom::Start(offset))?;
            file.read_exact(&mut buffer)?;
            hash.update(&buffer);
        }
    }
    ensure!(
        file.metadata()?.len() == size,
        "source changed during sampled fingerprint"
    );
    Ok(format!("v2:{size}:{}", hex::encode(hash.finalize())))
}

/// Finds complete asset-shaped objects in JSON project/history dictionaries,
/// including nested patch `value`s and take snapshots. It is a discovery aid,
/// NOT an assertion of closure: references represented only by packed paths,
/// embedded PMV media or agent artifact files must also be supplied by the host.
/// Unknown fields remain in the original input; this function never rewrites it.
pub fn discover_assets(value: &Value) -> Vec<Value> {
    fn walk(v: &Value, out: &mut Vec<Value>) {
        match v {
            Value::Object(o) => {
                if o.get("id").and_then(Value::as_str).is_some()
                    && (o.contains_key("storageKey")
                        || o.contains_key("fingerprint")
                        || o.contains_key("playbackProxy"))
                {
                    out.push(v.clone());
                }
                for v in o.values() {
                    walk(v, out);
                }
            }
            Value::Array(a) => {
                for v in a {
                    walk(v, out);
                }
            }
            _ => {}
        }
    }
    let mut out = Vec::new();
    walk(value, &mut out);
    out
}

/// Logical localStorage export is private, atomic JSON. Exact DOM strings and
/// unknown origin keys are retained alongside convenience parsed JSON values.
pub fn export_local_storage(snapshot: &Snapshot, path: &Path) -> Result<usize> {
    let entries = crate::local_storage::read(snapshot)?;
    atomic_json(path, &entries)?;
    Ok(entries.len())
}
