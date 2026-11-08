//! Agent extension authoring, as main's `change-history.ts`, `fork.ts`,
//! `rebase.ts` and `validate-staged-extensions.ts`. An agent edits a private
//! stage; publication checks a snapshot of it that the agent cannot write,
//! promotes it atomically through a journal, and keeps the previous version for
//! Undo. Nothing in a stage runs until it is published.
mod merge;
mod scan;
pub use merge::merge_text;
pub use scan::{scan_extension, scan_text};

use crate::{Error, Result};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File},
    io::{Read, Write},
    os::unix::fs::OpenOptionsExt,
    path::{Path, PathBuf},
    sync::Mutex,
};

const MAX_FILES: usize = 4_000;
const MAX_BYTES: u64 = 32 * 1024 * 1024;
const RECORD: &str = "change-set.json";
const SNAPSHOT: &str = ".snapshot-";
const PENDING: &str = ".pending-";
pub const FORK_BASE: &str = ".forked-from";

/// What the application supplies. Everything here runs on an MCP worker.
pub trait ExtensionHost: Send + Sync {
    /// Folders the app loads extensions from, later ones winning. The last is
    /// the writable user folder that publication promotes into.
    fn roots(&self) -> Vec<PathBuf>;
    /// App-owned change history; agent processes never get this path.
    fn history(&self) -> PathBuf;
    /// The shipped built-in extension sources, read-only.
    fn builtins(&self) -> Option<PathBuf>;
    /// The frozen manifest parser (apiVersion 1–4), normalized.
    fn manifest(&self, raw: &Value) -> std::result::Result<Value, String>;
    /// Compile with the app's own toolchain and activate in a throwaway runtime.
    fn check(&self, dir: &Path, manifest: &Value) -> std::result::Result<(), String>;
    /// Published: every open window reloads these extensions now.
    fn reload(&self, ids: &[String]);
    /// The authoring pack written into each agent workspace (`api/`).
    fn api_pack(&self) -> Vec<(String, String)>;
}

pub fn extension_id(s: &str) -> bool {
    (2..=64).contains(&s.len())
        && (s.as_bytes()[0].is_ascii_lowercase() || s.as_bytes()[0].is_ascii_digit())
        && s.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}
fn invalid(message: impl Into<String>) -> Error {
    Error::invalid(message)
}

/// One live folder owns its publication, recovery and baseline reads.
static LIVE: Mutex<()> = Mutex::new(());
fn locked<T>(task: impl FnOnce() -> Result<T>) -> Result<T> {
    let _guard = LIVE.lock().unwrap_or_else(|e| e.into_inner());
    task()
}

// ---- trees -------------------------------------------------------------------

struct Budget {
    files: usize,
    bytes: u64,
}
impl Budget {
    fn take(&mut self, bytes: u64, what: &str) -> Result<()> {
        self.files += 1;
        self.bytes += bytes;
        if self.files > MAX_FILES || self.bytes > MAX_BYTES {
            return Err(invalid(format!("{what} exceeds the safe copy limits.")));
        }
        Ok(())
    }
}
fn sorted(dir: &Path) -> Result<Vec<fs::DirEntry>> {
    let mut entries = fs::read_dir(dir)?.collect::<std::io::Result<Vec<_>>>()?;
    entries.sort_by_key(|e| e.file_name());
    Ok(entries)
}
fn name(entry: &fs::DirEntry) -> Result<String> {
    entry
        .file_name()
        .into_string()
        .map_err(|_| invalid("Extension paths must be UTF-8."))
}
/// Reads a regular file without following a link swapped in since the listing.
fn read_regular(path: &Path, relative: &str) -> Result<Vec<u8>> {
    let mut file = fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
        .map_err(|e| {
            if e.raw_os_error() == Some(libc::ELOOP) {
                invalid(format!(
                    "Extension contains an unsupported symbolic link: {relative}"
                ))
            } else {
                e.into()
            }
        })?;
    let meta = file.metadata()?;
    if !meta.is_file() {
        return Err(invalid(format!(
            "Extension contains an unsupported file: {relative}"
        )));
    }
    let mut data = Vec::with_capacity(meta.len() as usize);
    (&mut file).take(meta.len() + 1).read_to_end(&mut data)?;
    if data.len() as u64 > meta.len() {
        return Err(invalid(format!(
            "Extension staging changed while it was being read: {relative}"
        )));
    }
    Ok(data)
}
/// Files and folders of a tree, links and special files refused.
#[derive(Default, Clone)]
pub struct Tree {
    pub directories: Vec<String>,
    pub files: BTreeMap<String, Vec<u8>>,
}
fn read_tree(root: &Path, skip_top: Option<&str>, budget: &mut Budget) -> Result<Tree> {
    fn walk(
        dir: &Path,
        prefix: &str,
        skip: Option<&str>,
        tree: &mut Tree,
        budget: &mut Budget,
    ) -> Result<()> {
        for entry in sorted(dir)? {
            let name = name(&entry)?;
            if prefix.is_empty() && Some(name.as_str()) == skip {
                continue;
            }
            let relative = if prefix.is_empty() {
                name
            } else {
                format!("{prefix}/{name}")
            };
            let meta = fs::symlink_metadata(entry.path())?;
            if meta.file_type().is_symlink() {
                return Err(invalid(format!(
                    "Extension contains an unsupported symbolic link: {relative}"
                )));
            }
            if meta.is_dir() {
                tree.directories.push(relative.clone());
                walk(&entry.path(), &relative, None, tree, budget)?;
            } else if meta.is_file() {
                let data = read_regular(&entry.path(), &relative)?;
                budget.take(data.len() as u64, "Extension source")?;
                tree.files.insert(relative, data);
            } else {
                return Err(invalid(format!(
                    "Extension contains an unsupported file: {relative}"
                )));
            }
        }
        Ok(())
    }
    let meta = fs::symlink_metadata(root)?;
    if meta.file_type().is_symlink() || !meta.is_dir() {
        return Err(invalid("Extension folder is not a regular folder."));
    }
    let mut tree = Tree::default();
    walk(root, "", skip_top, &mut tree, budget)?;
    Ok(tree)
}
fn write_file(path: &Path, data: &[u8]) -> Result<()> {
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)?;
    file.write_all(data)?;
    Ok(())
}
fn write_tree(dest: &Path, tree: &Tree) -> Result<()> {
    fs::create_dir_all(dest)?;
    for dir in &tree.directories {
        fs::create_dir_all(dest.join(dir))?;
    }
    for (relative, data) in &tree.files {
        let path = dest.join(relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        write_file(&path, data)?;
    }
    Ok(())
}
fn copy_tree(source: &Path, dest: &Path) -> Result<()> {
    let tree = read_tree(source, None, &mut Budget { files: 0, bytes: 0 })?;
    write_tree(dest, &tree)
}
fn tree_hash(root: &Path) -> Result<String> {
    let tree = read_tree(root, None, &mut Budget { files: 0, bytes: 0 })?;
    let mut entries: Vec<(&str, Option<&[u8]>)> = tree
        .directories
        .iter()
        .map(|d| (d.as_str(), None))
        .chain(
            tree.files
                .iter()
                .map(|(p, d)| (p.as_str(), Some(d.as_slice()))),
        )
        .collect();
    entries.sort_by(|a, b| a.0.cmp(b.0));
    let mut hash = Sha256::new();
    for (path, data) in entries {
        match data {
            None => hash.update(format!("d\0{path}\0")),
            Some(data) => {
                hash.update(format!("f\0{path}\0"));
                hash.update(data);
            }
        }
    }
    Ok(format!("{:x}", hash.finalize()))
}
/// Extension folders by id across `roots`, later roots winning.
fn folders(roots: &[PathBuf]) -> BTreeMap<String, PathBuf> {
    let mut out = BTreeMap::new();
    for root in roots {
        for entry in fs::read_dir(root).into_iter().flatten().flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            let is_dir = fs::symlink_metadata(entry.path()).is_ok_and(|m| m.is_dir());
            if is_dir && extension_id(&name) {
                out.insert(name, entry.path());
            }
        }
    }
    out
}
fn hashes(roots: &[PathBuf]) -> Result<BTreeMap<String, String>> {
    folders(roots)
        .into_iter()
        .map(|(id, dir)| Ok((id, tree_hash(&dir)?)))
        .collect()
}
fn root_hash(map: &BTreeMap<String, String>) -> String {
    let mut hash = Sha256::new();
    for (id, value) in map {
        hash.update(format!("{id}\0{value}\0"));
    }
    format!("{:x}", hash.finalize())
}
fn fresh(prefix: &str) -> String {
    pm_kernel::new_id(prefix)
}
fn now_iso() -> String {
    let ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64);
    let (days, rem) = (ms.div_euclid(86_400_000), ms.rem_euclid(86_400_000));
    // Civil date from days since 1970-01-01 (Howard Hinnant's algorithm).
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
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}.{:03}Z",
        rem / 3_600_000,
        rem / 60_000 % 60,
        rem / 1000 % 60,
        rem % 1000
    )
}

// ---- stage -------------------------------------------------------------------

/// A run's private copy of the user's extensions and the hashes it started from.
#[derive(Clone, Debug)]
pub struct Stage {
    pub roots: Vec<PathBuf>,
    pub staging: PathBuf,
    pub history: PathBuf,
    pub project: String,
    pub run: String,
    pub baseline: BTreeMap<String, String>,
}
impl Stage {
    fn live(&self) -> &Path {
        self.roots.last().expect("a writable extension folder")
    }
}
pub fn history_for(host: &dyn ExtensionHost, project: &str) -> PathBuf {
    let safe: String = project
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .take(120)
        .collect();
    host.history().join(safe)
}
pub fn prepare(
    host: &dyn ExtensionHost,
    staging: &Path,
    project: &str,
    run: &str,
) -> Result<Stage> {
    let roots = host.roots();
    let live = roots
        .last()
        .ok_or_else(|| invalid("No user extension folder."))?;
    let history = history_for(host, project);
    locked(|| {
        recover_locked(&history, live)?;
        fs::create_dir_all(live)?;
        if staging.exists() {
            fs::remove_dir_all(staging)?;
        }
        fs::create_dir_all(staging)?;
        for (id, dir) in folders(&roots) {
            copy_tree(&dir, &staging.join(id))?;
        }
        Ok(Stage {
            baseline: hashes(&[staging.to_owned()])?,
            roots: roots.clone(),
            staging: staging.to_owned(),
            history: history.clone(),
            project: project.into(),
            run: run.into(),
        })
    })
}

/// The published record of one change set, kept with its `before` trees.
pub fn record_json(id: &str, project: &str, before: &str, after: &str, changes: &[Value]) -> Value {
    json!({"version":1,"id":id,"projectId":project,"createdAt":now_iso(),
        "beforeRootHash":before,"afterRootHash":after,"changes":changes})
}

/// Validate, then atomically promote the declared changes. `Ok(None)` means
/// nothing changed. Errors carry main's agent-facing wording.
pub fn publish(
    host: &dyn ExtensionHost,
    stage: &Stage,
    declared: &[Value],
) -> Result<Option<Value>> {
    let ids: Vec<&str> = declared.iter().filter_map(|c| c["id"].as_str()).collect();
    fs::create_dir_all(&stage.history)?;
    let snapshot = stage
        .history
        .join(fresh(&format!("{SNAPSHOT}{}-", stage.run)));
    let result = (|| {
        let copy = snapshot.join("stage");
        fs::create_dir_all(&copy)?;
        // Only what publishing reads; unchanged folders publish as their baseline.
        let mut unchanged = BTreeSet::new();
        for (id, dir) in folders(std::slice::from_ref(&stage.staging)) {
            if stage.baseline.contains_key(&id)
                && !ids.contains(&id.as_str())
                && tree_hash(&dir).ok().as_ref() == stage.baseline.get(&id)
            {
                unchanged.insert(id);
                continue;
            }
            copy_tree(&dir, &copy.join(&id))?;
        }
        validate(host, &copy, declared)?;
        locked(|| promote(host, stage, &copy, &unchanged, declared))
    })();
    let _ = fs::remove_dir_all(&snapshot);
    result
}
fn validate(host: &dyn ExtensionHost, copy: &Path, declared: &[Value]) -> Result<()> {
    for change in declared {
        let id = change["id"].as_str().unwrap_or_default();
        if !extension_id(id) {
            return Err(invalid("Invalid staged extension id."));
        }
        if change["action"] == "removed" {
            continue;
        }
        let dir = copy.join(id);
        let raw: Value = fs::read(dir.join("manifest.json"))
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .ok_or_else(|| {
                invalid(format!("{id}: provide a valid manifest.json in the current staging directory. Nothing was published."))
            })?;
        let manifest = host
            .manifest(&raw)
            .map_err(|e| invalid(format!("{id}: {e}. Nothing was published.")))?;
        if manifest["id"] != id {
            return Err(invalid(format!(
                "{id}: the manifest id must match its folder. Nothing was published."
            )));
        }
        let blocked = scan_extension(id, &dir)?;
        if !blocked.is_empty() {
            return Err(invalid(blocked.join("\n")));
        }
        host.check(&dir, &manifest).map_err(|e| {
            invalid(format!(
                "{id} failed compilation: {e}. Repair its staged source. Nothing was published."
            ))
        })?;
    }
    Ok(())
}
fn promote(
    host: &dyn ExtensionHost,
    stage: &Stage,
    copy: &Path,
    unchanged: &BTreeSet<String>,
    declared: &[Value],
) -> Result<Option<Value>> {
    let mut staged = hashes(&[copy.to_owned()])?;
    for id in unchanged {
        staged.insert(id.clone(), stage.baseline[id].clone());
    }
    let action = |id: &str| match (stage.baseline.get(id), staged.get(id)) {
        (None, _) => "created",
        (_, None) => "removed",
        _ => "updated",
    };
    let actual: Vec<String> = stage
        .baseline
        .keys()
        .chain(staged.keys())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .filter(|id| stage.baseline.get(*id) != staged.get(*id))
        .cloned()
        .collect();
    let expected: Vec<Value> = actual
        .iter()
        .map(|id| json!({"id":id,"action":action(id)}))
        .collect();
    let report = |message: String| {
        invalid(format!(
            "{message} Nothing was applied. Actual staged extension changes: {}. Inspect these changes and return each once in extensions with the indicated action, or revert unintended staged edits before completing.",
            Value::Array(expected.clone())
        ))
    };
    let mut declared_ids: Vec<&str> = declared.iter().filter_map(|c| c["id"].as_str()).collect();
    declared_ids.sort_unstable();
    if declared_ids.windows(2).any(|w| w[0] == w[1]) {
        return Err(report(
            "The agent reported the same extension change more than once.".into(),
        ));
    }
    if actual
        .iter()
        .map(String::as_str)
        .ne(declared_ids.iter().copied())
    {
        return Err(report(
            "The agent change report did not match its staged files.".into(),
        ));
    }
    if actual.is_empty() {
        return Ok(None);
    }
    let live = stage.live();
    let current = hashes(&stage.roots)?;
    let conflicts: Vec<&str> = actual
        .iter()
        .filter(|id| current.get(*id) != stage.baseline.get(*id))
        .map(String::as_str)
        .collect();
    if !conflicts.is_empty() {
        return Err(Error::conflict(format!(
            "Newer changes exist in these extensions: {}. No extension files were overwritten. Completed project edits were kept. Review the retained staged edits against the current versions before publishing; repeating the request cannot resolve this conflict.",
            conflicts.join(", ")
        )));
    }
    for id in &actual {
        let reported = declared.iter().find(|c| c["id"] == id.as_str());
        let said = reported
            .and_then(|c| c["action"].as_str())
            .unwrap_or("missing");
        if said != action(id) {
            return Err(report(format!(
                "The agent reported {id} as {said}, but it was {}.",
                action(id)
            )));
        }
        if action(id) == "removed" && !live.join(id).exists() {
            return Err(invalid(format!(
                "{id} comes from the shipped app's extension folder and cannot be removed here. Disable it in Settings › Extensions instead. Nothing was applied."
            )));
        }
    }
    let pending = stage
        .history
        .join(fresh(&format!("{PENDING}{}-", stage.run)));
    let (before, next) = (pending.join("before"), pending.join("next"));
    fs::create_dir_all(&before)?;
    fs::create_dir_all(&next)?;
    for id in &actual {
        if staged.contains_key(id) {
            copy_tree(&copy.join(id), &next.join(id))?;
        }
    }
    let mut after = current.clone();
    for id in &actual {
        match staged.get(id) {
            Some(hash) => after.insert(id.clone(), hash.clone()),
            None => after.remove(id),
        };
    }
    let changes: Vec<Value> = declared
        .iter()
        .map(|c| json!({"id":c["id"],"action":c["action"],"summary":c["summary"]}))
        .collect();
    let record = record_json(
        &stage.run,
        &stage.project,
        &root_hash(&current),
        &root_hash(&after),
        &changes,
    );
    write_file(&pending.join(RECORD), format!("{:#}\n", record).as_bytes())?;
    let swap = || -> Result<()> {
        for id in &actual {
            let target = live.join(id);
            if target.exists() {
                fs::rename(&target, before.join(id))?;
            }
            if next.join(id).exists() {
                fs::rename(next.join(id), &target)?;
            }
        }
        let final_dir = stage.history.join(&stage.run);
        if final_dir.exists() {
            fs::remove_dir_all(&final_dir)?;
        }
        fs::rename(&pending, final_dir)?;
        Ok(())
    };
    if let Err(error) = swap() {
        // A failed rollback keeps the journal so the next start retries it.
        if rollback(&pending, live, &record).is_ok() {
            let _ = fs::remove_dir_all(&pending);
        }
        return Err(error);
    }
    host.reload(&actual);
    Ok(Some(record))
}
/// Undo a journal: restore each backup; remove what was moved in without one.
fn rollback(pending: &Path, live: &Path, record: &Value) -> Result<()> {
    fs::create_dir_all(live)?;
    for change in record["changes"].as_array().into_iter().flatten() {
        let id = change["id"].as_str().unwrap_or_default();
        if !extension_id(id) {
            return Err(invalid("Invalid pending extension id."));
        }
        let target = live.join(id);
        let previous = pending.join("before").join(id);
        if previous.exists() {
            // The backup stays until the whole journal goes, so a crash during
            // recovery never leaves a half-restored copy as the only one.
            let restored = pending.join("restore").join(id);
            if restored.exists() {
                fs::remove_dir_all(&restored)?;
            }
            copy_tree(&previous, &restored)?;
            if target.exists() {
                fs::remove_dir_all(&target)?;
            }
            fs::rename(&restored, &target)?;
        } else if change["action"] != "removed"
            && !pending.join("next").join(id).exists()
            && target.exists()
        {
            fs::remove_dir_all(&target)?;
        }
    }
    Ok(())
}
fn recover_locked(history: &Path, live: &Path) -> Result<()> {
    fs::create_dir_all(history)?;
    for entry in sorted(history)? {
        let name = name(&entry)?;
        if !name.starts_with(PENDING) || !entry.path().is_dir() {
            continue;
        }
        if let Some(record) = fs::read(entry.path().join(RECORD))
            .ok()
            .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
        {
            rollback(&entry.path(), live, &record)?;
        }
        fs::remove_dir_all(entry.path())?;
    }
    Ok(())
}
/// Boot-time repair of every project's interrupted promotion, before the
/// extension host scans live folders. Leftover snapshots go too.
pub fn recover_all(host: &dyn ExtensionHost) -> Result<()> {
    let Some(live) = host.roots().last().cloned() else {
        return Ok(());
    };
    locked(|| {
        for entry in fs::read_dir(host.history()).into_iter().flatten().flatten() {
            if entry.path().is_dir() {
                recover_locked(&entry.path(), &live)?;
                for leftover in fs::read_dir(entry.path())?.flatten() {
                    if leftover.file_name().to_string_lossy().starts_with(SNAPSHOT) {
                        fs::remove_dir_all(leftover.path())?;
                    }
                }
            }
        }
        Ok(())
    })
}

/// Change sets of a project, newest first.
pub fn change_sets(history: &Path) -> Vec<Value> {
    let mut out: Vec<Value> = fs::read_dir(history)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| !e.file_name().to_string_lossy().starts_with('.'))
        .filter_map(|e| serde_json::from_slice(&fs::read(e.path().join(RECORD)).ok()?).ok())
        .filter(|r: &Value| r["restored"] != true)
        .collect();
    out.sort_by(|a, b| b["createdAt"].as_str().cmp(&a["createdAt"].as_str()));
    out
}
/// Put back what a change set replaced. Only the newest publication can be
/// undone, so a later edit is never silently discarded.
pub fn restore(host: &dyn ExtensionHost, project: &str, change_set: &str) -> Result<Value> {
    if change_set.is_empty()
        || change_set.len() > 160
        || !change_set
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    {
        return Err(invalid("Invalid extension change-set id."));
    }
    let roots = host.roots();
    let live = roots
        .last()
        .ok_or_else(|| invalid("No user extension folder."))?
        .clone();
    let history = history_for(host, project);
    let record = locked(|| {
        recover_locked(&history, &live)?;
        let directory = history.join(change_set);
        let mut record: Value = serde_json::from_slice(&fs::read(directory.join(RECORD))?)
            .map_err(|_| invalid("Invalid extension change set."))?;
        if record["restored"] == true {
            return Err(invalid("This extension change was already undone."));
        }
        if root_hash(&hashes(&roots)?) != record["afterRootHash"].as_str().unwrap_or_default() {
            return Err(invalid(
                "Newer extension changes exist. Restore newer change sets first.",
            ));
        }
        let (before, redo) = (directory.join("before"), directory.join("redo"));
        if redo.exists() {
            fs::remove_dir_all(&redo)?;
        }
        fs::create_dir_all(&redo)?;
        let mut touched = vec![];
        let result = (|| -> Result<()> {
            for change in record["changes"].as_array().into_iter().flatten() {
                let id = change["id"]
                    .as_str()
                    .filter(|id| extension_id(id))
                    .ok_or_else(|| invalid("Invalid extension id."))?;
                let target = live.join(id);
                if target.exists() {
                    fs::rename(&target, redo.join(id))?;
                }
                touched.push(id.to_owned());
                if before.join(id).exists() {
                    copy_tree(&before.join(id), &target)?;
                }
            }
            Ok(())
        })();
        if let Err(error) = result {
            for id in touched.iter().rev() {
                let _ = fs::remove_dir_all(live.join(id));
                let _ = fs::rename(redo.join(id), live.join(id));
            }
            return Err(error);
        }
        record["restored"] = json!(true);
        write_file(
            &directory.join(RECORD),
            format!("{:#}\n", record).as_bytes(),
        )?;
        Ok(record)
    })?;
    let ids: Vec<String> = record["changes"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|c| c["id"].as_str().map(str::to_owned))
        .collect();
    host.reload(&ids);
    Ok(record)
}

// ---- forks -------------------------------------------------------------------

fn manifest_object(data: Option<&Vec<u8>>, label: &str) -> Result<serde_json::Map<String, Value>> {
    let data = data.ok_or_else(|| invalid(format!("{label} is missing manifest.json.")))?;
    match serde_json::from_slice::<Value>(data) {
        Ok(Value::Object(map)) => Ok(map),
        Ok(_) => Err(invalid(format!("Invalid {label} manifest."))),
        Err(_) => Err(invalid(format!("Invalid {label} manifest JSON."))),
    }
}
fn parsed(
    host: &dyn ExtensionHost,
    raw: &serde_json::Map<String, Value>,
    id: &str,
    label: &str,
) -> Result<Value> {
    let manifest = host
        .manifest(&Value::Object(raw.clone()))
        .map_err(|e| invalid(format!("Invalid {label} manifest: {e}")))?;
    if manifest["id"] != id {
        return Err(invalid(format!(
            "{label} manifest id {} does not match {id}.",
            manifest["id"].as_str().unwrap_or_default()
        )));
    }
    Ok(manifest)
}
fn pretty(value: &Value) -> Vec<u8> {
    format!("{}\n", serde_json::to_string_pretty(value).unwrap()).into_bytes()
}

/// Copy a shipped built-in into the stage as a user extension that replaces
/// it, with a pristine copy in `.forked-from` as the future merge base.
pub fn fork(
    host: &dyn ExtensionHost,
    id: &str,
    fork_id: Option<&str>,
    staging: &Path,
) -> Result<Value> {
    let fork_id = fork_id.map_or_else(|| format!("{id}-fork"), str::to_owned);
    if !extension_id(id) || !extension_id(&fork_id) {
        return Err(invalid("Invalid extension id."));
    }
    if fork_id == id {
        return Err(invalid(
            "Fork id must differ from the built-in extension id.",
        ));
    }
    let builtins = host
        .builtins()
        .ok_or_else(|| Error::unavailable(crate::catalog::backing("fork_builtin_extension")))?;
    let source = fs::canonicalize(&builtins)?.join(id);
    if !source.is_dir() {
        return Err(invalid(format!("Unknown built-in extension: {id}")));
    }
    let tree = read_tree(&source, None, &mut Budget { files: 0, bytes: 0 })?;
    let raw = manifest_object(tree.files.get("manifest.json"), "built-in")?;
    let manifest = parsed(host, &raw, id, "built-in")?;
    let forked_from = format!("{id}@{}", manifest["version"].as_str().unwrap_or_default());
    let mut rewritten = raw.clone();
    rewritten.insert("id".into(), json!(fork_id));
    rewritten.insert(
        "name".into(),
        json!(format!("{} (fork)", raw["name"].as_str().unwrap_or(id))),
    );
    let mut replaces: Vec<Value> = raw
        .get("replaces")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    if !replaces.contains(&json!(id)) {
        replaces.push(json!(id));
    }
    rewritten.insert("replaces".into(), json!(replaces));
    rewritten.insert("forkedFrom".into(), json!(forked_from));
    rewritten.insert(
        "author".into(),
        raw.get("author").cloned().unwrap_or(json!("user")),
    );
    parsed(host, &rewritten, &fork_id, "fork")?;
    fs::create_dir_all(staging)?;
    let dest = fs::canonicalize(staging)?.join(&fork_id);
    fs::create_dir(&dest).map_err(|e| {
        if e.kind() == std::io::ErrorKind::AlreadyExists {
            invalid("Extension fork target already exists.")
        } else {
            e.into()
        }
    })?;
    let mut work = tree.clone();
    work.files.remove("manifest.json");
    write_tree(&dest, &work)?;
    write_tree(&dest.join(FORK_BASE), &tree)?;
    // The manifest goes last: an incomplete copy never looks like an extension.
    write_file(
        &dest.join("manifest.json"),
        &pretty(&Value::Object(rewritten)),
    )?;
    File::open(&dest)?.sync_all()?;
    Ok(
        json!({"dir":dest,"forkId":fork_id,"files":tree.files.keys().collect::<Vec<_>>(),
        "forkedFrom":forked_from,
        "reminder":format!("List {fork_id} in the result's extensions array with action created.")}),
    )
}

fn changed(a: &BTreeMap<String, Vec<u8>>, b: &BTreeMap<String, Vec<u8>>) -> Vec<String> {
    a.keys()
        .chain(b.keys())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .filter(|k| a.get(*k) != b.get(*k))
        .cloned()
        .collect()
}

/// Three-way rebase of a user fork onto the shipped built-in, in the stage.
/// The live fork is never touched; `conflicts` are left for the agent.
pub fn rebase(host: &dyn ExtensionHost, fork_id: &str, staging: &Path) -> Result<Value> {
    if !extension_id(fork_id) {
        return Err(invalid("Invalid extension id."));
    }
    let builtins = host
        .builtins()
        .ok_or_else(|| Error::unavailable(crate::catalog::backing("stage_fork_rebase")))?;
    let roots = host.roots();
    let folders = folders(&roots);
    let Some(user_dir) = folders.get(fork_id) else {
        if builtins.join(fork_id).exists() {
            return Err(invalid(format!(
                "Only user extensions can be rebased: {fork_id}"
            )));
        }
        return Err(invalid(format!("Unknown extension: {fork_id}")));
    };
    let base_dir = user_dir.join(FORK_BASE);
    if !base_dir.is_dir() {
        return Err(invalid(format!(
            "Fork {fork_id} is missing its {FORK_BASE} base."
        )));
    }
    let mut budget = Budget { files: 0, bytes: 0 };
    let user = read_tree(user_dir, Some(FORK_BASE), &mut budget)?;
    let user_raw = manifest_object(user.files.get("manifest.json"), "user fork")?;
    let user_manifest = parsed(host, &user_raw, fork_id, "user fork")?;
    let forked = user_manifest["forkedFrom"]
        .as_str()
        .ok_or_else(|| invalid(format!("Extension {fork_id} is not a fork.")))?;
    let (origin, base_version) = forked
        .rsplit_once('@')
        .filter(|(id, _)| extension_id(id))
        .ok_or_else(|| invalid(format!("Malformed forkedFrom value: {forked}")))?;
    if !user_manifest["replaces"]
        .as_array()
        .is_some_and(|r| r.iter().any(|v| v == origin))
    {
        return Err(invalid(format!("Fork {fork_id} must replace {origin}.")));
    }
    let shipped = builtins.join(origin);
    if !shipped.is_dir() {
        return Err(invalid(format!(
            "Fork {fork_id} references an unknown built-in: {origin}"
        )));
    }
    let base = read_tree(&base_dir, None, &mut budget)?;
    let current = read_tree(&shipped, None, &mut budget)?;
    let base_raw = manifest_object(base.files.get("manifest.json"), "fork base")?;
    let current_raw = manifest_object(current.files.get("manifest.json"), "shipped built-in")?;
    let base_manifest = parsed(host, &base_raw, origin, "fork base")?;
    if base_manifest["version"] != base_version {
        return Err(invalid(format!(
            "Fork {fork_id} records {origin}@{base_version}, but its base is version {}.",
            base_manifest["version"].as_str().unwrap_or_default()
        )));
    }
    let current_version = parsed(host, &current_raw, origin, "shipped built-in")?["version"]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    if current_version == base_version {
        return Err(invalid(format!(
            "Fork {fork_id} is already based on the shipped {origin}@{base_version}."
        )));
    }
    // Manifest: keep the fork's identity; take upstream changes the fork left alone.
    let identity = ["id", "name", "author", "version", "forkedFrom"];
    let with_origin = |mut m: serde_json::Map<String, Value>| {
        let mut r: Vec<Value> = m
            .get("replaces")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        if !r.contains(&json!(origin)) {
            r.push(json!(origin));
        }
        m.insert("replaces".into(), json!(r));
        m
    };
    let (mb, mc) = (with_origin(base_raw), with_origin(current_raw));
    let mut manifest = user_raw.clone();
    let mut manifest_conflict = false;
    let keys: BTreeSet<&String> = mb.keys().chain(user_raw.keys()).chain(mc.keys()).collect();
    for key in keys {
        let (u, b, c) = (user_raw.get(key), mb.get(key), mc.get(key));
        if identity.contains(&key.as_str()) || u == c || b == c {
            continue;
        }
        if u != b {
            manifest_conflict = true;
            continue;
        }
        match c {
            Some(c) => manifest.insert(key.clone(), c.clone()),
            None => manifest.remove(key),
        };
    }
    let mut files = user.files.clone();
    let (mut conflicts, mut merged) = (vec![], vec![]);
    if manifest_conflict {
        conflicts.push("manifest.json".to_owned());
    } else {
        merged.push("manifest.json".to_owned());
    }
    let upstream = changed(&base.files, &current.files);
    for relative in upstream.iter().filter(|r| *r != "manifest.json") {
        let (b, u, c) = (
            base.files.get(relative),
            user.files.get(relative),
            current.files.get(relative),
        );
        if u == c {
            continue;
        }
        if u == b {
            match c {
                Some(c) => files.insert(relative.clone(), c.clone()),
                None => files.remove(relative),
            };
            merged.push(relative.clone());
        } else if let (Some(u), Some(c)) = (u, c)
            && let Some(text) = merge_text(b.map(Vec::as_slice), u, c)
        {
            files.insert(relative.clone(), text);
            merged.push(relative.clone());
        } else {
            conflicts.push(relative.clone());
        }
    }
    // A new file colliding with a folder is structural: hand the whole tree back.
    let structural = files.keys().any(|path| {
        let parts: Vec<&str> = path.split('/').collect();
        (1..parts.len()).any(|i| files.contains_key(&parts[..i].join("/")))
    });
    let mut tree = Tree {
        directories: user.directories.clone(),
        files,
    };
    if structural {
        tree = user.clone();
        manifest = user_raw.clone();
        conflicts = upstream.clone();
        merged.clear();
    }
    conflicts.sort();
    merged.sort();
    let staging = {
        fs::create_dir_all(staging)?;
        fs::canonicalize(staging)?
    };
    for root in roots.iter().chain([&builtins]) {
        let root = fs::canonicalize(root).unwrap_or_else(|_| root.clone());
        if staging.starts_with(&root) || root.starts_with(&staging) {
            return Err(invalid(
                "Extension rebase staging must not overlap an extension source root.",
            ));
        }
    }
    let working = staging.join(fork_id);
    let rebase_dir = staging.join(".rebase").join(fork_id);
    let (base_out, ours_out) = (rebase_dir.join("base"), rebase_dir.join("ours"));
    for dir in [&working, &rebase_dir] {
        if dir.exists() {
            fs::remove_dir_all(dir)?;
        }
    }
    tree.files.remove("manifest.json");
    write_tree(&working, &tree)?;
    write_tree(&base_out, &base)?;
    write_tree(&ours_out, &current)?;
    write_tree(&working.join(FORK_BASE), &current)?;
    manifest.insert(
        "forkedFrom".into(),
        json!(format!("{origin}@{current_version}")),
    );
    write_file(
        &working.join("manifest.json"),
        &pretty(&Value::Object(manifest)),
    )?;
    Ok(
        json!({"forkId":fork_id,"workingDir":working,"baseDir":base_out,"oursDir":ours_out,
        "changedByUser":changed(&base.files, &user.files),"changedUpstream":upstream,
        "conflicts":conflicts,"mergedFiles":merged,"forkedFrom":origin,
        "base":base_version,"current":current_version}),
    )
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
