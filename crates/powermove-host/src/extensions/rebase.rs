//! Three-way rebase of a user fork onto a newer shipped built-in (port of
//! `rebase.ts`), and the local "update fork" built on it (`update-fork.ts`).

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component, Path, PathBuf};

use anyhow::{Result, anyhow, bail};
use serde::Serialize;
use serde_json::{Map, Value};

use super::manifest::{Manifest, is_extension_id, is_extension_version, parse_manifest};

const MAX_FILES: usize = 4_000;
const MAX_BYTES: usize = 32 * 1024 * 1024;
pub const FORK_BASE_DIRECTORY: &str = ".forked-from";

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ForkRebaseInfo {
    pub fork_id: String,
    /// The built-in id the fork replaces.
    pub forked_from: String,
    pub base: String,
    pub current: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StagedRebase {
    #[serde(flatten)]
    pub info: ForkRebaseInfo,
    pub working_dir: PathBuf,
    pub base_dir: PathBuf,
    pub ours_dir: PathBuf,
    pub changed_by_user: Vec<String>,
    pub changed_upstream: Vec<String>,
    pub conflicts: Vec<String>,
    pub merged_files: Vec<String>,
}

#[derive(Clone, Default)]
struct Tree {
    directories: Vec<String>,
    files: BTreeMap<String, Vec<u8>>,
}

#[derive(Default)]
struct Budget {
    files: usize,
    bytes: usize,
}

struct Fork {
    info: ForkRebaseInfo,
    manifest_raw: Map<String, Value>,
    user: Tree,
    base: Tree,
    current: Tree,
}

/// Validate a user fork and report the built-in version it can be rebased onto.
pub fn read_fork_rebase_info(fork_id: &str, user_dir: &Path, builtin_dir: &Path) -> Result<ForkRebaseInfo> {
    validate_absolute(&[user_dir, builtin_dir])?;
    Ok(inspect_fork(fork_id, user_dir, builtin_dir)?.info)
}

/// Build a three-way rebase workspace without touching the live extension:
/// `<staging>/<forkId>` is the merged tree, `<staging>/.rebase/<forkId>/{base,ours}`
/// the read-only inputs. Blocking.
pub fn stage_fork_rebase(fork_id: &str, staging: &Path, user_dir: &Path, builtin_dir: &Path) -> Result<StagedRebase> {
    validate_absolute(&[staging, user_dir, builtin_dir])?;
    let stage_root = lexical(staging);
    for source in [lexical(user_dir), lexical(builtin_dir)] {
        if source.starts_with(&stage_root) || stage_root.starts_with(&source) {
            bail!("Extension rebase staging must not overlap an extension source root.");
        }
    }
    let fork = inspect_fork(fork_id, user_dir, builtin_dir)?;
    let working_dir = contained_child(&stage_root, &fork.info.fork_id)?;
    let rebase_dir = contained_child(&contained_child(&stage_root, ".rebase")?, &fork.info.fork_id)?;
    let base_dir = contained_child(&rebase_dir, "base")?;
    let ours_dir = contained_child(&rebase_dir, "ours")?;

    let changed_by_user = changed_files(&fork.base.files, &fork.user.files);
    let changed_upstream = changed_files(&fork.base.files, &fork.current.files);
    let merged = merge_fork(&fork)?;

    // Every source has been read and validated before staged state is removed.
    std::fs::create_dir_all(&stage_root)?;
    for directory in [&working_dir, &rebase_dir] {
        remove_if_present(directory)?;
    }
    write_tree(&working_dir, &merged.tree)?;
    write_tree(&base_dir, &fork.base)?;
    write_tree(&ours_dir, &fork.current)?;
    let working_base = contained_child(&working_dir, FORK_BASE_DIRECTORY)?;
    remove_if_present(&working_base)?;
    write_tree(&working_base, &fork.current)?;
    let mut manifest = merged.manifest;
    manifest.insert("forkedFrom".into(), Value::String(format!("{}@{}", fork.info.forked_from, fork.info.current)));
    std::fs::write(working_dir.join("manifest.json"), format!("{}\n", serde_json::to_string_pretty(&Value::Object(manifest))?))?;

    Ok(StagedRebase {
        info: fork.info,
        working_dir,
        base_dir,
        ours_dir,
        changed_by_user,
        changed_upstream,
        conflicts: merged.conflicts,
        merged_files: merged.merged_files,
    })
}

struct Merged {
    tree: Tree,
    manifest: Map<String, Value>,
    conflicts: Vec<String>,
    merged_files: Vec<String>,
}

/// Keep the fork's identity; merge the other manifest fields as JSON values so
/// formatting and bookkeeping never create artificial conflicts.
fn merge_manifest(fork: &Fork) -> Result<(Map<String, Value>, bool)> {
    let mut base = manifest_object(fork.base.files.get("manifest.json"), "fork base")?;
    let mut current = manifest_object(fork.current.files.get("manifest.json"), "shipped built-in")?;
    let user = &fork.manifest_raw;
    let mut manifest = user.clone();
    for input in [&mut base, &mut current] {
        let mut replaces: Vec<Value> = input.get("replaces").and_then(Value::as_array).cloned().unwrap_or_default();
        let origin = Value::String(fork.info.forked_from.clone());
        if !replaces.contains(&origin) {
            replaces.push(origin);
        }
        input.insert("replaces".into(), Value::Array(replaces));
    }
    let identity = ["id", "name", "author", "version", "forkedFrom"];
    let keys: BTreeSet<&String> = base.keys().chain(user.keys()).chain(current.keys()).collect();
    let mut conflict = false;
    for key in keys {
        let (in_base, in_user, in_current) = (base.get(key), user.get(key), current.get(key));
        if identity.contains(&key.as_str()) || in_user == in_current || in_base == in_current {
            continue;
        }
        if in_user != in_base {
            conflict = true;
            continue;
        }
        match in_current {
            Some(value) => manifest.insert(key.clone(), value.clone()),
            None => manifest.remove(key),
        };
    }
    Ok((manifest, conflict))
}

fn merge_fork(fork: &Fork) -> Result<Merged> {
    let mut files = fork.user.files.clone();
    let mut conflicts = Vec::new();
    let mut merged_files = Vec::new();
    let (manifest, conflict) = merge_manifest(fork)?;
    if conflict { &mut conflicts } else { &mut merged_files }.push("manifest.json".to_owned());
    let upstream = changed_files(&fork.base.files, &fork.current.files);
    for relative in &upstream {
        if relative == "manifest.json" {
            continue;
        }
        let (base, user, current) = (fork.base.files.get(relative), fork.user.files.get(relative), fork.current.files.get(relative));
        if user == current {
            continue;
        }
        if user == base {
            match current {
                Some(current) => files.insert(relative.clone(), current.clone()),
                None => files.remove(relative),
            };
            merged_files.push(relative.clone());
        } else if let (Some(user), Some(current)) = (user, current) {
            match merge_text(base.map(Vec::as_slice), user, current) {
                Some(text) => {
                    files.insert(relative.clone(), text);
                    merged_files.push(relative.clone());
                }
                None => conflicts.push(relative.clone()),
            }
        } else {
            conflicts.push(relative.clone());
        }
    }
    // A new file can collide with a customized folder (or the reverse). Leave
    // those structural changes to the agent instead of writing a partial tree.
    let prefixes = |relative: &str| {
        let parts: Vec<String> = relative.split('/').map(str::to_owned).collect();
        (1..parts.len()).map(move |end| parts[..end].join("/"))
    };
    if files.keys().any(|relative| prefixes(relative).any(|prefix| files.contains_key(&prefix))) {
        return Ok(Merged { tree: fork.user.clone(), manifest: fork.manifest_raw.clone(), conflicts: upstream, merged_files: Vec::new() });
    }
    let mut directories: BTreeSet<String> = fork
        .user
        .directories
        .iter()
        .filter(|directory| !prefixes(directory).chain([directory.to_string()]).any(|prefix| files.contains_key(&prefix)))
        .cloned()
        .collect();
    for relative in files.keys() {
        directories.extend(prefixes(relative));
    }
    conflicts.sort();
    merged_files.sort();
    Ok(Merged { tree: Tree { directories: directories.into_iter().collect(), files }, manifest, conflicts, merged_files })
}

/// `mergeText` (cloud/pull.ts): a line-level three-way merge of UTF-8 text
/// without NUL bytes; `None` is a conflict.
pub fn merge_text(base: Option<&[u8]>, ours: &[u8], theirs: &[u8]) -> Option<Vec<u8>> {
    let text = |bytes: &[u8]| if bytes.contains(&0) { None } else { std::str::from_utf8(bytes).ok().map(str::to_owned) };
    let (ours, theirs, base) = (text(ours)?, text(theirs)?, base.map_or(Some(String::new()), text)?);
    diffy::merge(&base, &ours, &theirs).ok().map(String::into_bytes)
}

fn inspect_fork(fork_id: &str, user_dir: &Path, builtin_dir: &Path) -> Result<Fork> {
    if !is_extension_id(fork_id) {
        bail!("Invalid extension id.");
    }
    let user_root = lexical(user_dir);
    let builtin_root = lexical(builtin_dir);
    let user_directory = contained_child(&user_root, fork_id)?;
    match entry_kind(&user_directory)? {
        Kind::Missing if entry_kind(&contained_child(&builtin_root, fork_id)?)? != Kind::Missing => {
            bail!("Only user extensions can be rebased: {fork_id}")
        }
        Kind::Missing => bail!("Unknown extension: {fork_id}"),
        Kind::Other => bail!("Invalid user extension directory: {fork_id}"),
        Kind::Directory => {}
    }
    let base_directory = contained_child(&user_directory, FORK_BASE_DIRECTORY)?;
    if entry_kind(&base_directory)? != Kind::Directory {
        bail!("Fork {fork_id} is missing its {FORK_BASE_DIRECTORY} base.");
    }

    let mut budget = Budget::default();
    let user = read_tree(&user_directory, &mut budget, Some(FORK_BASE_DIRECTORY))?;
    let user_manifest = snapshot_manifest(&user, fork_id, "user fork")?;
    let manifest_raw = manifest_object(user.files.get("manifest.json"), "user fork")?;
    let Some(forked_from) = user_manifest.forked_from.as_deref() else { bail!("Extension {fork_id} is not a fork.") };
    let (origin, version) = parse_forked_from(forked_from)?;
    if !user_manifest.replaces.as_ref().is_some_and(|replaces| replaces.iter().any(|id| id == &origin)) {
        bail!("Fork {fork_id} must replace {origin}.");
    }
    let builtin_directory = contained_child(&builtin_root, &origin)?;
    if entry_kind(&builtin_directory)? != Kind::Directory {
        bail!("Fork {fork_id} references an unknown built-in: {origin}");
    }
    let base = read_tree(&base_directory, &mut budget, None)?;
    let current = read_tree(&builtin_directory, &mut budget, None)?;
    let base_manifest = snapshot_manifest(&base, &origin, "fork base")?;
    if base_manifest.version != version {
        bail!("Fork {fork_id} records {origin}@{version}, but its base is version {}.", base_manifest.version);
    }
    let current_manifest = snapshot_manifest(&current, &origin, "shipped built-in")?;
    if current_manifest.version == version {
        bail!("Fork {fork_id} is already based on the shipped {origin}@{version}.");
    }
    Ok(Fork {
        info: ForkRebaseInfo { fork_id: fork_id.to_owned(), forked_from: origin, base: version, current: current_manifest.version },
        manifest_raw,
        user,
        base,
        current,
    })
}

fn parse_forked_from(value: &str) -> Result<(String, String)> {
    let malformed = || anyhow!("Malformed forkedFrom value: {value}");
    let (id, version) = value.rsplit_once('@').ok_or_else(malformed)?;
    if id.is_empty() || !is_extension_id(id) || !is_extension_version(version) {
        return Err(malformed());
    }
    Ok((id.to_owned(), version.to_owned()))
}

fn snapshot_manifest(tree: &Tree, expected: &str, label: &str) -> Result<Manifest> {
    let raw = manifest_object(tree.files.get("manifest.json"), label)?;
    let manifest = parse_manifest(&Value::Object(raw)).map_err(|error| anyhow!("Invalid {label} manifest: {error}"))?;
    if manifest.id != expected {
        bail!("{label} manifest id {} does not match {expected}.", manifest.id);
    }
    Ok(manifest)
}

fn manifest_object(data: Option<&Vec<u8>>, label: &str) -> Result<Map<String, Value>> {
    let data = data.ok_or_else(|| anyhow!("{label} is missing manifest.json."))?;
    match serde_json::from_slice::<Value>(data) {
        Ok(Value::Object(map)) => Ok(map),
        Ok(_) => bail!("Invalid {label} manifest."),
        Err(_) => bail!("Invalid {label} manifest JSON."),
    }
}

fn read_tree(root: &Path, budget: &mut Budget, excluded_top_level: Option<&str>) -> Result<Tree> {
    let metadata = std::fs::symlink_metadata(root)?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        bail!("Extension rebase requires a regular directory: {}", root.display());
    }
    fn walk(directory: &Path, prefix: &str, tree: &mut Tree, budget: &mut Budget, excluded: Option<&str>) -> Result<()> {
        let mut entries: Vec<std::fs::DirEntry> = std::fs::read_dir(directory)?.collect::<std::io::Result<_>>()?;
        entries.sort_by_key(std::fs::DirEntry::file_name);
        for entry in entries {
            let name = entry.file_name().to_string_lossy().into_owned();
            if prefix.is_empty() && Some(name.as_str()) == excluded {
                continue;
            }
            let relative = if prefix.is_empty() { name } else { format!("{prefix}/{name}") };
            let metadata = std::fs::symlink_metadata(entry.path())?;
            if metadata.file_type().is_symlink() {
                bail!("Extension rebase does not allow symbolic links: {relative}");
            }
            if metadata.is_dir() {
                tree.directories.push(relative.clone());
                walk(&entry.path(), &relative, tree, budget, excluded)?;
                continue;
            }
            if !metadata.is_file() {
                bail!("Extension rebase does not allow special files: {relative}");
            }
            let data = std::fs::read(entry.path())?;
            budget.files += 1;
            budget.bytes += data.len();
            if budget.files > MAX_FILES || budget.bytes > MAX_BYTES {
                bail!("Extension rebase exceeds the safe copy limits.");
            }
            tree.files.insert(relative, data);
        }
        Ok(())
    }
    let mut tree = Tree::default();
    walk(root, "", &mut tree, budget, excluded_top_level)?;
    Ok(tree)
}

fn write_tree(destination: &Path, tree: &Tree) -> Result<()> {
    let join = |relative: &str| relative.split('/').fold(destination.to_path_buf(), |path, part| path.join(part));
    std::fs::create_dir_all(destination)?;
    for relative in &tree.directories {
        std::fs::create_dir_all(join(relative))?;
    }
    for (relative, data) in &tree.files {
        let output = join(relative);
        if let Some(parent) = output.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(output, data)?;
    }
    Ok(())
}

fn changed_files(before: &BTreeMap<String, Vec<u8>>, after: &BTreeMap<String, Vec<u8>>) -> Vec<String> {
    before.keys().chain(after.keys()).collect::<BTreeSet<_>>().into_iter().filter(|relative| before.get(*relative) != after.get(*relative)).cloned().collect()
}

#[derive(PartialEq, Eq)]
enum Kind {
    Missing,
    Directory,
    Other,
}

fn entry_kind(target: &Path) -> Result<Kind> {
    match std::fs::symlink_metadata(target) {
        Ok(metadata) if !metadata.file_type().is_symlink() && metadata.is_dir() => Ok(Kind::Directory),
        Ok(_) => Ok(Kind::Other),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Kind::Missing),
        Err(error) => Err(error.into()),
    }
}

fn remove_if_present(directory: &Path) -> Result<()> {
    match std::fs::remove_dir_all(directory) {
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => Err(error.into()),
        _ => Ok(()),
    }
}

fn validate_absolute(directories: &[&Path]) -> Result<()> {
    if directories.iter().any(|directory| !directory.is_absolute()) {
        bail!("Extension rebase paths must be absolute.");
    }
    Ok(())
}

/// `path.resolve`: normalize `.` and `..` without touching the disk.
fn lexical(path: &Path) -> PathBuf {
    let mut result = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                result.pop();
            }
            other => result.push(other),
        }
    }
    result
}

fn contained_child(root: &Path, child: &str) -> Result<PathBuf> {
    let result = lexical(&root.join(child));
    if result == lexical(root) || !result.starts_with(lexical(root)) {
        bail!("Extension rebase path escapes its root.");
    }
    Ok(result)
}

/* ── update fork (update-fork.ts) ─────────────────────────── */

/// `ForkUpdateResult` without the change-history fields.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum ForkUpdate {
    Updated { version: String },
    NeedsAgent { reason: &'static str, conflicts: Vec<String> },
}

/// Merge `fork_id` onto the shipped built-in in `scratch`, check the result
/// with `validate` (a compile), then swap it in for the live fork folder.
/// No agent or network. Refuses if the live folder changed meanwhile. Blocking.
pub fn update_fork(
    fork_id: &str,
    user_dir: &Path,
    builtin_dir: &Path,
    scratch: &Path,
    signature: impl Fn(&Path) -> Result<String>,
    validate: impl FnOnce(&Path) -> Result<(), String>,
) -> Result<ForkUpdate> {
    std::fs::create_dir_all(scratch)?;
    let temporary = scratch.join(format!("update-{}", hex::encode(rand::random::<[u8; 6]>())));
    std::fs::create_dir(&temporary)?;
    let result = (|| -> Result<ForkUpdate> {
        let live = contained_child(&lexical(user_dir), fork_id)?;
        let before = signature(&live)?;
        let rebase = stage_fork_rebase(fork_id, &temporary.join("merge"), user_dir, builtin_dir)?;
        if !rebase.conflicts.is_empty() {
            return Ok(ForkUpdate::NeedsAgent { reason: "conflict", conflicts: rebase.conflicts });
        }
        if validate(&rebase.working_dir).is_err() {
            return Ok(ForkUpdate::NeedsAgent { reason: "validation", conflicts: Vec::new() });
        }
        if signature(&live)? != before {
            bail!("The extension changed while it was being updated. Try again.");
        }
        let previous = temporary.join("previous");
        std::fs::rename(&live, &previous)?;
        if let Err(error) = std::fs::rename(&rebase.working_dir, &live) {
            std::fs::rename(&previous, &live)?;
            return Err(error.into());
        }
        Ok(ForkUpdate::Updated { version: rebase.info.current })
    })();
    let _ = std::fs::remove_dir_all(&temporary);
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(root: &Path, files: &[(&str, &str)]) {
        for (relative, text) in files {
            let path = root.join(relative);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        }
    }

    fn manifest(id: &str, version: &str, extra: &str) -> String {
        format!(r#"{{ "id": "{id}", "name": "Tools", "version": "{version}", "apiVersion": 3{extra} }}"#)
    }

    #[test]
    fn merges_user_and_upstream_changes_and_reports_conflicts() {
        let root = tempfile::tempdir().unwrap();
        let builtins = root.path().join("builtin");
        let users = root.path().join("user");
        let fork = users.join("tools-fork");
        let base_manifest = manifest("tools", "1.0.0", "");
        write(&fork.join(FORK_BASE_DIRECTORY), &[("manifest.json", &base_manifest), ("a.ts", "one\ntwo\nthree\n"), ("b.ts", "b\n"), ("c.ts", "c\n")]);
        let fork_manifest = manifest("tools-fork", "1.0.0", r#", "replaces": ["tools"], "forkedFrom": "tools@1.0.0""#);
        write(&fork, &[("manifest.json", &fork_manifest), ("a.ts", "ONE\ntwo\nthree\n"), ("b.ts", "mine\n"), ("c.ts", "c\n")]);
        let shipped = manifest("tools", "1.1.0", r#", "description": "new""#);
        write(&builtins.join("tools"), &[("manifest.json", &shipped), ("a.ts", "one\ntwo\nTHREE\n"), ("b.ts", "theirs\n"), ("c.ts", "c2\n")]);

        let info = read_fork_rebase_info("tools-fork", &users, &builtins).unwrap();
        assert_eq!((info.base.as_str(), info.current.as_str()), ("1.0.0", "1.1.0"));

        let staged = stage_fork_rebase("tools-fork", &root.path().join("stage"), &users, &builtins).unwrap();
        assert_eq!(staged.conflicts, vec!["b.ts".to_owned()]);
        assert_eq!(staged.merged_files, vec!["a.ts".to_owned(), "c.ts".to_owned(), "manifest.json".to_owned()]);
        assert_eq!(std::fs::read_to_string(staged.working_dir.join("a.ts")).unwrap(), "ONE\ntwo\nTHREE\n");
        assert_eq!(std::fs::read_to_string(staged.working_dir.join("c.ts")).unwrap(), "c2\n");
        let merged: Value = serde_json::from_str(&std::fs::read_to_string(staged.working_dir.join("manifest.json")).unwrap()).unwrap();
        assert_eq!(merged["forkedFrom"], "tools@1.1.0");
        assert_eq!(merged["description"], "new");
        assert_eq!(merged["id"], "tools-fork");
        assert!(staged.working_dir.join(FORK_BASE_DIRECTORY).join("b.ts").is_file());

        let outcome = update_fork("tools-fork", &users, &builtins, &root.path().join("scratch"), |_| Ok(String::new()), |_| Ok(())).unwrap();
        assert_eq!(outcome, ForkUpdate::NeedsAgent { reason: "conflict", conflicts: vec!["b.ts".into()] });

        std::fs::write(fork.join("b.ts"), "b\n").unwrap();
        let outcome = update_fork("tools-fork", &users, &builtins, &root.path().join("scratch"), |_| Ok(String::new()), |_| Ok(())).unwrap();
        assert_eq!(outcome, ForkUpdate::Updated { version: "1.1.0".into() });
        assert_eq!(std::fs::read_to_string(fork.join("b.ts")).unwrap(), "theirs\n");
        assert_eq!(read_fork_rebase_info("tools-fork", &users, &builtins).unwrap_err().to_string(), "Fork tools-fork is already based on the shipped tools@1.1.0.");
    }
}
