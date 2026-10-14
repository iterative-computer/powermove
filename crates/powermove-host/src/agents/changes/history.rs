//! Guarded promotion of a run's extension stage into the live folder, with
//! an undo record (port of `codex/change-history.ts`).
//!
//! `<userData>/Agent Change History/<project>/<changeSetId>/` keeps
//! `change-set.json` and `before/<id>` (the prior live folders). A publish is
//! journaled as `.pending-*` first, so a crash mid-swap is rolled back by
//! [`recover`] before anything reads the live folder again. Everything here
//! blocks; callers run it on a blocking thread.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context as _, Result, bail};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

use super::super::workspace::{ValidationError, copy_tree, extension_hashes, folder_hash};
use crate::ids::is_extension_id;

const RECORD_FILE: &str = "change-set.json";
pub const SNAPSHOT_PREFIX: &str = ".snapshot-";
const PENDING_PREFIX: &str = ".pending-";

/// One publish, restore or recovery at a time, across every project: they
/// all compare and swap folders in the same live directory.
static TRANSACTIONS: Mutex<()> = Mutex::new(());

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Action {
    Created,
    Updated,
    Removed,
}

/// `AgentExtensionChange`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Change {
    pub id: String,
    pub action: Action,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
}

impl Change {
    pub fn new(id: &str, action: Action) -> Self {
        Self { id: id.to_owned(), action, summary: None }
    }
}

/// `parseAgentExtensionChanges`: `None` unless the report's `extensions` is an array.
pub fn parse_changes(value: &Value) -> Option<Vec<Change>> {
    let items = value.as_array()?;
    Some(
        items
            .iter()
            .filter_map(|item| {
                let id = item["id"].as_str().filter(|id| is_extension_id(id))?;
                let action = serde_json::from_value(item["action"].clone()).ok()?;
                Some(Change { id: id.to_owned(), action, summary: item["summary"].as_str().map(str::to_owned) })
            })
            .take(32)
            .collect(),
    )
}

/// `ExtensionChangeSetRecord`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Record {
    pub version: u32,
    pub id: String,
    pub project_id: String,
    pub created_at: String,
    pub before_root_hash: String,
    pub after_root_hash: String,
    pub changes: Vec<Change>,
}

/// A run's private copy of the live extensions (`ExtensionStage`).
#[derive(Debug, Clone)]
pub struct Stage {
    pub live: PathBuf,
    pub staging: PathBuf,
    /// `<userData>/Agent Change History/<project>`.
    pub history: PathBuf,
    pub project_id: String,
    pub run_id: String,
    pub baseline: BTreeMap<String, String>,
}

impl Stage {
    /// Copy `live` into `staging` and record its baseline (`prepareExtensionStage`).
    pub fn prepare(live: &Path, staging: &Path, history: &Path, project_id: &str, run_id: &str) -> Result<Self> {
        recover(history, live)?;
        std::fs::create_dir_all(live)?;
        remove_if_present(staging)?;
        copy_tree(live, staging, &mut 0)?;
        Ok(Self { live: live.into(), staging: staging.into(), history: history.into(), project_id: project_id.into(), run_id: run_id.into(), baseline: extension_hashes(staging)? })
    }
}

/// A copy of the changed stage folders in the app-owned history folder,
/// which agent processes cannot write: a process that outlived its command
/// can still rewrite the stage, but not what was checked and ships
/// (`withStageSnapshot`). Removed on drop.
pub struct Snapshot {
    directory: PathBuf,
    /// Changed and reported folders only.
    pub staging: PathBuf,
    /// Throwaway compiler output.
    pub compiled: PathBuf,
    /// Folders that matched their baseline and were not copied.
    unchanged: Vec<String>,
}

impl Drop for Snapshot {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.directory);
    }
}

impl Snapshot {
    /// Copy every stage folder that differs from its baseline, plus `reported`.
    pub fn take(stage: &Stage, reported: &[Change]) -> Result<Self> {
        let directory = stage.history.join(format!("{SNAPSHOT_PREFIX}{}-{}", stage.run_id, uuid::Uuid::new_v4()));
        let mut snapshot = Self { staging: directory.join("stage"), compiled: directory.join("compiled"), directory, unchanged: Vec::new() };
        std::fs::create_dir_all(&snapshot.staging)?;
        let mut count = 0;
        for entry in std::fs::read_dir(&stage.staging).into_iter().flatten() {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().into_owned();
            if !entry.file_type()?.is_dir() || !is_extension_id(&name) {
                continue;
            }
            let baseline = stage.baseline.get(&name);
            if baseline.is_some() && !reported.iter().any(|change| change.id == name) && baseline == Some(&folder_hash(&entry.path())?) {
                snapshot.unchanged.push(name);
                continue;
            }
            copy_tree(&entry.path(), &snapshot.staging.join(&name), &mut count)?;
        }
        Ok(snapshot)
    }
}

fn root_hash(hashes: &BTreeMap<String, String>) -> String {
    let mut hasher = Sha256::new();
    for (id, hash) in hashes {
        hasher.update(format!("{id}\0{hash}\0"));
    }
    hex::encode(hasher.finalize())
}

fn action_name(action: Action) -> &'static str {
    match action {
        Action::Created => "created",
        Action::Updated => "updated",
        Action::Removed => "removed",
    }
}

fn action(before: Option<&String>, after: Option<&String>) -> Action {
    match (before, after) {
        (None, _) => Action::Created,
        (_, None) => Action::Removed,
        _ => Action::Updated,
    }
}

/// Promote the snapshot's changes into the live folder (`publishExtensionChanges`).
/// `None` when nothing changed. A `ValidationError` means the agent's report
/// does not match its files; nothing was applied.
pub fn publish(stage: &Stage, snapshot: &Snapshot, declared: &[Change]) -> Result<Option<Record>> {
    let _transaction = TRANSACTIONS.lock();
    if root_hash(&extension_hashes(&stage.live)?) != root_hash(&stage.baseline) {
        bail!("Extensions changed while the agent was working. Nothing was overwritten; retry the request.");
    }
    let mut staged = extension_hashes(&snapshot.staging)?;
    for id in &snapshot.unchanged {
        staged.insert(id.clone(), stage.baseline[id].clone());
    }
    let actual: Vec<String> = stage.baseline.keys().chain(staged.keys()).filter(|id| stage.baseline.get(*id) != staged.get(*id)).cloned().collect::<std::collections::BTreeSet<_>>().into_iter().collect();
    // `JSON.stringify(expectedChanges)`, keys in the TypeScript order.
    let expected = actual.iter().map(|id| format!(r#"{{"id":"{id}","action":"{}"}}"#, action_name(action(stage.baseline.get(id), staged.get(id))))).collect::<Vec<_>>().join(",");
    let report_error = |message: String| -> anyhow::Error {
        ValidationError(format!(
            "{message} Nothing was applied. Actual staged extension changes: [{expected}]. Inspect these changes and return each once in extensions with the indicated action, or revert unintended staged edits before completing."
        ))
        .into()
    };
    let mut declared_ids: Vec<&str> = declared.iter().map(|change| change.id.as_str()).collect();
    declared_ids.sort_unstable();
    if declared_ids.windows(2).any(|pair| pair[0] == pair[1]) {
        return Err(report_error("The agent reported the same extension change more than once.".into()));
    }
    if declared_ids != actual.iter().map(String::as_str).collect::<Vec<_>>() {
        return Err(report_error("The agent change report did not match its staged files.".into()));
    }
    if actual.is_empty() {
        return Ok(None);
    }
    for id in &actual {
        let expected = action(stage.baseline.get(id), staged.get(id));
        let reported = declared.iter().find(|change| &change.id == id).map(|change| change.action);
        if reported != Some(expected) {
            return Err(report_error(format!("The agent reported {id} as {}, but it was {}.", reported.map_or("missing", action_name), action_name(expected))));
        }
    }

    let pending = stage.history.join(format!("{PENDING_PREFIX}{}-{}", stage.run_id, uuid::Uuid::new_v4()));
    let (before, next) = (pending.join("before"), pending.join("next"));
    let finished = stage.history.join(&stage.run_id);
    remove_if_present(&finished)?;
    std::fs::create_dir_all(&before)?;
    std::fs::create_dir_all(&next)?;
    let mut count = 0;
    for id in actual.iter().filter(|id| staged.contains_key(*id)) {
        copy_tree(&snapshot.staging.join(id), &next.join(id), &mut count)?;
    }
    let mut after = stage.baseline.clone();
    for id in &actual {
        match staged.get(id) {
            Some(hash) => after.insert(id.clone(), hash.clone()),
            None => after.remove(id),
        };
    }
    let record = Record {
        version: 1,
        id: stage.run_id.clone(),
        project_id: stage.project_id.clone(),
        created_at: iso_now(),
        before_root_hash: root_hash(&stage.baseline),
        after_root_hash: root_hash(&after),
        changes: declared.to_vec(),
    };
    std::fs::write(pending.join(RECORD_FILE), format!("{}\n", serde_json::to_string_pretty(&record)?))?;

    let swapped = (|| -> Result<()> {
        for id in &actual {
            let live = stage.live.join(id);
            if live.exists() {
                rename(&live, &before.join(id))?;
            }
            let replacement = next.join(id);
            if replacement.exists() {
                rename(&replacement, &live)?;
            }
        }
        rename(&pending, &finished)
    })();
    if let Err(error) = swapped {
        // Keep the journal and backups if rollback itself fails so boot can retry.
        if rollback(&pending, &stage.live, &record).is_ok() {
            let _ = std::fs::remove_dir_all(&pending);
        }
        return Err(error);
    }
    Ok(Some(record))
}

/// Put back the folders a change set replaced (`restoreExtensionChangeSet`).
pub fn restore(live: &Path, history: &Path, change_set_id: &str) -> Result<Record> {
    if change_set_id.is_empty() || change_set_id.len() > 160 || !change_set_id.bytes().all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-') {
        bail!("Invalid extension change-set id.");
    }
    recover(history, live)?;
    let _transaction = TRANSACTIONS.lock();
    let directory = history.join(change_set_id);
    let record: Record = serde_json::from_slice(&std::fs::read(directory.join(RECORD_FILE)).context("Unknown extension change set.")?)?;
    if root_hash(&extension_hashes(live)?) != record.after_root_hash {
        bail!("Newer extension changes exist. Restore newer change sets first.");
    }
    let (before, redo) = (directory.join("before"), directory.join("redo"));
    remove_if_present(&redo)?;
    std::fs::create_dir_all(&redo)?;
    let mut touched: Vec<&str> = Vec::new();
    let restored = (|| -> Result<()> {
        for change in &record.changes {
            if !is_extension_id(&change.id) {
                bail!("Invalid extension id in the change set.");
            }
            let target = live.join(&change.id);
            if target.exists() {
                rename(&target, &redo.join(&change.id))?;
            }
            touched.push(&change.id);
            let snapshot = before.join(&change.id);
            if snapshot.exists() {
                copy_tree(&snapshot, &target, &mut 0)?;
            }
        }
        Ok(())
    })();
    if let Err(error) = restored {
        for id in touched.into_iter().rev() {
            let target = live.join(id);
            let _ = std::fs::remove_dir_all(&target);
            if redo.join(id).exists() {
                let _ = rename(&redo.join(id), &target);
            }
        }
        return Err(error);
    }
    Ok(record)
}

/// Roll back every interrupted publish journaled under `history`
/// (`recoverInterruptedExtensionTransactions`).
pub fn recover(history: &Path, live: &Path) -> Result<()> {
    let _transaction = TRANSACTIONS.lock();
    std::fs::create_dir_all(history)?;
    for entry in std::fs::read_dir(history)? {
        let entry = entry?;
        if !entry.file_type()?.is_dir() || !entry.file_name().to_string_lossy().starts_with(PENDING_PREFIX) {
            continue;
        }
        let pending = entry.path();
        if let Ok(bytes) = std::fs::read(pending.join(RECORD_FILE)) {
            rollback(&pending, live, &serde_json::from_slice(&bytes)?)?;
        }
        std::fs::remove_dir_all(&pending)?;
    }
    Ok(())
}

/// Boot-time repair across every project before the registry scans the live
/// folder (`recoverAllInterruptedExtensionTransactions`). Also drops snapshot
/// leftovers, which only a crash leaves behind.
pub fn recover_all(user_data: &Path, live: &Path) -> Result<()> {
    let root = user_data.join("Agent Change History");
    let Ok(entries) = std::fs::read_dir(&root) else { return Ok(()) };
    for entry in entries {
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            continue;
        }
        recover(&entry.path(), live)?;
        for leftover in std::fs::read_dir(entry.path())? {
            let leftover = leftover?;
            if leftover.file_name().to_string_lossy().starts_with(SNAPSHOT_PREFIX) {
                let _ = std::fs::remove_dir_all(leftover.path());
            }
        }
    }
    Ok(())
}

fn rollback(pending: &Path, live: &Path, record: &Record) -> Result<()> {
    std::fs::create_dir_all(live)?;
    for change in &record.changes {
        if !is_extension_id(&change.id) {
            bail!("Invalid pending extension id.");
        }
        let target = live.join(&change.id);
        let previous = pending.join("before").join(&change.id);
        if previous.exists() {
            // Keep the backup until the whole journal is removed: a crash
            // during recovery must never leave a partial tree as the only copy.
            let restored = pending.join("restore").join(&change.id);
            remove_if_present(&restored)?;
            copy_tree(&previous, &restored, &mut 0)?;
            remove_if_present(&target)?;
            rename(&restored, &target)?;
        } else if change.action == Action::Created && !pending.join("next").join(&change.id).exists() {
            // Only a creation whose prepared replacement was moved can be ours.
            remove_if_present(&target)?;
        }
        // Updated/removed entries without a backup were not touched.
    }
    Ok(())
}

fn remove_if_present(directory: &Path) -> Result<()> {
    match std::fs::remove_dir_all(directory) {
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => Err(error.into()),
        _ => Ok(()),
    }
}

/// Windows refuses a folder rename while a scanner or the watcher briefly
/// holds a handle inside it; retry a few times before failing.
fn rename(from: &Path, to: &Path) -> Result<()> {
    for attempt in 0.. {
        match std::fs::rename(from, to) {
            Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied && attempt < 10 => std::thread::sleep(Duration::from_millis(50)),
            result => return result.with_context(|| format!("move {} to {}", from.display(), to.display())),
        }
    }
    unreachable!()
}

/// `new Date().toISOString()`.
fn iso_now() -> String {
    let elapsed = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
    let (days, seconds) = ((elapsed.as_secs() / 86_400) as i64, elapsed.as_secs() % 86_400);
    // Howard Hinnant's civil_from_days.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}.{:03}Z", seconds / 3_600, seconds / 60 % 60, seconds % 60, elapsed.subsec_millis())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::extensions::compiler::tests::write;

    fn manifest(id: &str) -> String {
        format!(r#"{{ "id": "{id}", "name": "{id}", "version": "1.0.0", "apiVersion": 3 }}"#)
    }

    fn stage(root: &Path) -> Stage {
        Stage::prepare(&root.join("live"), &root.join("stage"), &root.join("history"), "p1", "run-1").unwrap()
    }

    #[test]
    fn publishes_restores_and_rejects_mismatched_reports() {
        let root = tempfile::tempdir().unwrap();
        write(&root.path().join("live").join("keep"), &[("manifest.json", &manifest("keep")), ("index.ts", "1")]);
        write(&root.path().join("live").join("old"), &[("manifest.json", &manifest("old"))]);
        let stage = stage(root.path());
        write(&stage.staging.join("keep"), &[("index.ts", "2")]);
        write(&stage.staging.join("fresh"), &[("manifest.json", &manifest("fresh"))]);
        std::fs::remove_dir_all(stage.staging.join("old")).unwrap();

        let wrong = [Change::new("fresh", Action::Created), Change::new("keep", Action::Created), Change::new("old", Action::Removed)];
        let snapshot = Snapshot::take(&stage, &wrong).unwrap();
        let error = publish(&stage, &snapshot, &wrong).unwrap_err();
        assert!(error.downcast_ref::<ValidationError>().unwrap().0.starts_with("The agent reported keep as created, but it was updated."));
        let error = publish(&stage, &snapshot, &wrong[..1]).unwrap_err();
        assert!(error.to_string().contains(r#"Actual staged extension changes: [{"id":"fresh","action":"created"},{"id":"keep","action":"updated"},{"id":"old","action":"removed"}]"#), "{error}");

        let right = [Change::new("fresh", Action::Created), Change { summary: Some("Two".into()), ..Change::new("keep", Action::Updated) }, Change::new("old", Action::Removed)];
        let record = publish(&stage, &snapshot, &right).unwrap().unwrap();
        drop(snapshot);
        let live = root.path().join("live");
        assert_eq!(std::fs::read_to_string(live.join("keep").join("index.ts")).unwrap(), "2");
        assert!(live.join("fresh").join("manifest.json").is_file());
        assert!(!live.join("old").exists());
        let history = root.path().join("history");
        assert!(history.join("run-1").join("before").join("old").join("manifest.json").is_file());
        assert_eq!(std::fs::read_dir(&history).unwrap().count(), 1, "the snapshot and journal are gone");

        // A second publish from the same (now stale) baseline is refused.
        let snapshot = Snapshot::take(&stage, &right).unwrap();
        assert!(publish(&stage, &snapshot, &right).unwrap_err().to_string().starts_with("Extensions changed while the agent was working."));

        let restored = restore(&live, &history, &record.id).unwrap();
        assert_eq!(restored.changes, right);
        assert_eq!(std::fs::read_to_string(live.join("keep").join("index.ts")).unwrap(), "1");
        assert!(live.join("old").join("manifest.json").is_file());
        assert!(!live.join("fresh").exists());
        assert_eq!(restore(&live, &history, &record.id).unwrap_err().to_string(), "Newer extension changes exist. Restore newer change sets first.");
        assert!(restore(&live, &history, "../x").is_err());
    }

    #[test]
    fn recovers_an_interrupted_publish() {
        let root = tempfile::tempdir().unwrap();
        let (live, user_data) = (root.path().join("live"), root.path().join("data"));
        let history = user_data.join("Agent Change History").join("p1");
        write(&live.join("keep"), &[("manifest.json", &manifest("keep")), ("index.ts", "old")]);
        // The crash came after `keep` moved to before/ and its replacement
        // moved in, and after `fresh` moved in.
        let pending = history.join(".pending-run-1-x");
        write(&pending.join("before").join("keep"), &[("manifest.json", &manifest("keep")), ("index.ts", "old")]);
        write(&live.join("keep"), &[("index.ts", "new")]);
        write(&live.join("fresh"), &[("manifest.json", &manifest("fresh"))]);
        std::fs::create_dir_all(pending.join("next")).unwrap();
        let record = Record {
            version: 1,
            id: "run-1".into(),
            project_id: "p1".into(),
            created_at: iso_now(),
            before_root_hash: String::new(),
            after_root_hash: String::new(),
            changes: vec![Change::new("fresh", Action::Created), Change::new("keep", Action::Updated)],
        };
        std::fs::write(pending.join(RECORD_FILE), serde_json::to_string(&record).unwrap()).unwrap();
        std::fs::create_dir_all(history.join(format!("{SNAPSHOT_PREFIX}run-2"))).unwrap();

        recover_all(&user_data, &live).unwrap();
        assert_eq!(std::fs::read_to_string(live.join("keep").join("index.ts")).unwrap(), "old");
        assert!(!live.join("fresh").exists());
        assert_eq!(std::fs::read_dir(&history).unwrap().count(), 0);
        assert!(record.created_at.ends_with('Z') && record.created_at.len() == 24, "{}", record.created_at);
    }
}
