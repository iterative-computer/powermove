//! Automatic 3-way rebase for user built-in forks across software updates.
//!
//! When Powermove ships an updated built-in panel, any user fork (`replaces: [...]`
//! with a `.forked-from/` merge base) is automatically 3-way merged against the
//! newly shipped built-in on startup:
//! - Clean merges update the fork and `.forked-from/` atomically with zero user
//!   or agent intervention.
//! - True overlapping-line conflicts leave the live fork untouched so it keeps
//!   running seamlessly on the backward-compatible v4 API instead of breaking.
use pm_mcp::extensions::{FORK_BASE, extension_id, merge_text};
use serde_json::{Map, Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
    sync::Mutex,
};

const MAX_FILES: usize = 4_000;
const MAX_BYTES: u64 = 32 * 1024 * 1024;

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Report {
    pub rebased: Vec<(String, String)>,
    pub conflicts: Vec<(String, Vec<String>)>,
}

static RAN_FOR: Mutex<BTreeSet<PathBuf>> = Mutex::new(BTreeSet::new());

/// Runs automatic fork rebase once per `(profile, shipped_builtins_dir)` per
/// process on a background thread, then triggers an extension rescan if any
/// fork was seamlessly updated.
pub fn ensure_rebased_async(profile: PathBuf) {
    std::thread::Builder::new()
        .name("pm-fork-rebase".into())
        .spawn(move || {
            let Ok(builtins) = pm_builtins::builtin_sources(&profile.join("builtin-extensions"))
            else {
                return;
            };
            {
                let mut ran = RAN_FOR.lock().unwrap_or_else(|e| e.into_inner());
                if !ran.insert(builtins.clone()) {
                    return;
                }
            }
            let roots = crate::extensions::roots(&profile);
            let report = auto_rebase_all(&roots, &builtins);
            for (fork_id, origin) in &report.rebased {
                eprintln!(
                    "extensions: automatically rebased fork {fork_id} onto updated {origin}"
                );
            }
            for (fork_id, files) in &report.conflicts {
                eprintln!(
                    "extensions: kept existing fork {fork_id} running (manual rebase optional for: {})",
                    files.join(", ")
                );
            }
            if !report.rebased.is_empty() {
                pm_compat_runner::host::rescan_all();
            }
        })
        .ok();
}

/// Scans `roots` for user forks whose `.forked-from/` differs from the shipped
/// `builtins` directory and atomically 3-way merges any clean updates.
pub fn auto_rebase_all(roots: &[PathBuf], builtins: &Path) -> Report {
    let mut report = Report::default();
    let mut seen = BTreeSet::new();
    // Iterate in reverse so the winning (writable) root takes precedence.
    for root in roots.iter().rev() {
        let Ok(entries) = fs::read_dir(root) else {
            continue;
        };
        let mut names: Vec<String> = entries
            .flatten()
            .filter_map(|e| e.file_name().into_string().ok())
            .filter(|name| extension_id(name))
            .collect();
        names.sort();
        for fork_id in names {
            if !seen.insert(fork_id.clone()) {
                continue;
            }
            let user_dir = root.join(&fork_id);
            if !user_dir.join(FORK_BASE).join("manifest.json").is_file() {
                continue;
            }
            match auto_rebase_one(&user_dir, &fork_id, builtins) {
                Ok(Some(RebaseOutcome::Rebased(origin))) => {
                    report.rebased.push((fork_id, origin));
                }
                Ok(Some(RebaseOutcome::Conflict(files))) => {
                    report.conflicts.push((fork_id, files));
                }
                Ok(None) | Err(_) => {}
            }
        }
    }
    report
}

enum RebaseOutcome {
    Rebased(String),
    Conflict(Vec<String>),
}

#[derive(Clone, Default)]
struct Tree {
    files: BTreeMap<String, Vec<u8>>,
}

fn read_tree(root: &Path, skip: Option<&str>) -> Result<Tree, String> {
    let mut out = Tree::default();
    let mut bytes = 0u64;
    fn walk(
        base: &Path,
        dir: &Path,
        skip: Option<&str>,
        out: &mut Tree,
        bytes: &mut u64,
    ) -> Result<(), String> {
        let entries = fs::read_dir(dir).map_err(|e| e.to_string())?;
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name_str = name.to_string_lossy();
            if skip.is_some_and(|s| s == name_str.as_ref()) {
                continue;
            }
            let path = entry.path();
            let meta = fs::symlink_metadata(&path).map_err(|e| e.to_string())?;
            if meta.file_type().is_symlink() {
                return Err("symlink not supported in fork".into());
            }
            if meta.is_dir() {
                walk(base, &path, None, out, bytes)?;
            } else if meta.is_file() {
                if out.files.len() >= MAX_FILES {
                    return Err("too many files".into());
                }
                let data = fs::read(&path).map_err(|e| e.to_string())?;
                *bytes += data.len() as u64;
                if *bytes > MAX_BYTES {
                    return Err("fork too large".into());
                }
                let rel = path
                    .strip_prefix(base)
                    .map_err(|e| e.to_string())?
                    .to_string_lossy()
                    .replace('\\', "/");
                out.files.insert(rel, data);
            }
        }
        Ok(())
    }
    walk(root, root, skip, &mut out, &mut bytes)?;
    Ok(out)
}

fn write_tree(dest: &Path, tree: &Tree) -> Result<(), String> {
    fs::create_dir_all(dest).map_err(|e| e.to_string())?;
    for (rel, data) in &tree.files {
        let path = dest.join(rel);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        fs::write(&path, data).map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn manifest_map(data: Option<&Vec<u8>>) -> Result<Map<String, Value>, String> {
    let bytes = data.ok_or("missing manifest.json")?;
    match serde_json::from_slice::<Value>(bytes) {
        Ok(Value::Object(map)) => Ok(map),
        _ => Err("invalid manifest.json".into()),
    }
}

fn changed_files(a: &BTreeMap<String, Vec<u8>>, b: &BTreeMap<String, Vec<u8>>) -> Vec<String> {
    a.keys()
        .chain(b.keys())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .filter(|k| a.get(*k) != b.get(*k))
        .cloned()
        .collect()
}

fn auto_rebase_one(
    user_dir: &Path,
    fork_id: &str,
    builtins: &Path,
) -> Result<Option<RebaseOutcome>, String> {
    let base_dir = user_dir.join(FORK_BASE);
    let user = read_tree(user_dir, Some(FORK_BASE))?;
    let user_raw = manifest_map(user.files.get("manifest.json"))?;
    let user_manifest = pm_compat_runner::manifest::parse(&Value::Object(user_raw.clone()))?;
    let Some(forked) = user_manifest["forkedFrom"].as_str() else {
        return Ok(None);
    };
    let Some((origin, _base_version)) = forked.rsplit_once('@').filter(|(id, _)| extension_id(id))
    else {
        return Ok(None);
    };
    if !user_manifest["replaces"]
        .as_array()
        .is_some_and(|r| r.iter().any(|v| v == origin))
    {
        return Ok(None);
    }
    let shipped = builtins.join(origin);
    if !shipped.join("manifest.json").is_file() {
        return Ok(None);
    }
    let base = read_tree(&base_dir, None)?;
    let current = read_tree(&shipped, None)?;
    // Fast path: shipped built-in is identical to `.forked-from` (no app update to this panel).
    if base.files == current.files {
        return Ok(None);
    }
    let base_raw = manifest_map(base.files.get("manifest.json"))?;
    let current_raw = manifest_map(current.files.get("manifest.json"))?;
    let current_version = current_raw
        .get("version")
        .and_then(Value::as_str)
        .unwrap_or("1.0.0")
        .to_owned();

    // 3-way merge manifest.json while preserving fork identity fields.
    let identity = ["id", "name", "author", "version", "forkedFrom"];
    let with_origin = |mut m: Map<String, Value>| {
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
    let mut conflicts = Vec::new();
    let keys: BTreeSet<&String> = mb.keys().chain(user_raw.keys()).chain(mc.keys()).collect();
    for key in keys {
        let (u, b, c) = (user_raw.get(key), mb.get(key), mc.get(key));
        if identity.contains(&key.as_str()) || u == c || b == c {
            continue;
        }
        if u != b {
            conflicts.push("manifest.json".to_owned());
            break;
        }
        match c {
            Some(c) => {
                manifest.insert(key.clone(), c.clone());
            }
            None => {
                manifest.remove(key);
            }
        }
    }

    // 3-way merge every file changed upstream since `.forked-from`.
    let mut files = user.files.clone();
    let upstream = changed_files(&base.files, &current.files);
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
                Some(c) => {
                    files.insert(relative.clone(), c.clone());
                }
                None => {
                    files.remove(relative);
                }
            }
        } else if let (Some(u), Some(c)) = (u, c)
            && let Some(merged) = merge_text(b.map(Vec::as_slice), u, c)
        {
            files.insert(relative.clone(), merged);
        } else {
            conflicts.push(relative.clone());
        }
    }

    // Guard against file/directory structural collisions.
    let structural = files.keys().any(|path| {
        let parts: Vec<&str> = path.split('/').collect();
        (1..parts.len()).any(|i| files.contains_key(&parts[..i].join("/")))
    });
    if structural {
        conflicts = upstream;
    }

    if !conflicts.is_empty() {
        conflicts.sort();
        conflicts.dedup();
        return Ok(Some(RebaseOutcome::Conflict(conflicts)));
    }

    // Validate the merged manifest before touching disk.
    manifest.insert(
        "forkedFrom".into(),
        json!(format!("{origin}@{current_version}")),
    );
    let _ = pm_compat_runner::manifest::parse(&Value::Object(manifest.clone()))?;

    // Atomic swap: stage -> backup old -> promote staged -> delete backup.
    let parent = user_dir.parent().ok_or("fork has no parent dir")?;
    let pid = std::process::id();
    let staged = parent.join(format!(".rebase-stage-{fork_id}-{pid}"));
    let backup = parent.join(format!(".rebase-backup-{fork_id}-{pid}"));
    let _ = fs::remove_dir_all(&staged);
    let _ = fs::remove_dir_all(&backup);

    let mut out_tree = Tree { files };
    out_tree.files.remove("manifest.json");
    if let Err(err) = (|| -> Result<(), String> {
        write_tree(&staged, &out_tree)?;
        write_tree(&staged.join(FORK_BASE), &current)?;
        let manifest_bytes = format!(
            "{}\n",
            serde_json::to_string_pretty(&Value::Object(manifest)).map_err(|e| e.to_string())?
        );
        fs::write(staged.join("manifest.json"), manifest_bytes).map_err(|e| e.to_string())?;
        Ok(())
    })() {
        let _ = fs::remove_dir_all(&staged);
        return Err(err);
    }

    fs::rename(user_dir, &backup).map_err(|e| {
        let _ = fs::remove_dir_all(&staged);
        e.to_string()
    })?;
    if let Err(err) = fs::rename(&staged, user_dir) {
        // Restore original fork immediately if promotion fails.
        let _ = fs::rename(&backup, user_dir);
        let _ = fs::remove_dir_all(&staged);
        return Err(err.to_string());
    }
    let _ = fs::remove_dir_all(&backup);
    Ok(Some(RebaseOutcome::Rebased(format!(
        "{origin}@{current_version}"
    ))))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(root: &Path, rel: &str, text: &str) {
        let path = root.join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }

    fn builtin_manifest(id: &str, version: &str) -> String {
        json!({
            "id": id,
            "name": "Timeline",
            "version": version,
            "apiVersion": 4,
            "entry": "index.tsx"
        })
        .to_string()
    }

    fn fork_manifest(fork_id: &str, origin: &str, base_version: &str) -> String {
        json!({
            "id": fork_id,
            "name": "My Timeline",
            "version": "1.0.0",
            "apiVersion": 4,
            "entry": "index.tsx",
            "author": "user",
            "replaces": [origin],
            "forkedFrom": format!("{origin}@{base_version}")
        })
        .to_string()
    }

    #[test]
    fn clean_three_way_auto_rebase_preserves_user_edits_and_updates_base() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("extensions");
        let builtins = tmp.path().join("builtins");
        let fork = root.join("timeline-custom");
        let shipped = builtins.join("timeline");

        let base_code = "line1\nline2\nline3\nline4\nline5\n";
        let user_code = "line1 // user custom header\nline2\nline3\nline4\nline5\n";
        let new_shipped = "line1\nline2\nline3\nline4\nline5 // app update fix\n";

        // Base snapshot in .forked-from
        write(&fork.join(FORK_BASE), "manifest.json", &builtin_manifest("timeline", "1.0.0"));
        write(&fork.join(FORK_BASE), "index.tsx", base_code);
        // User fork
        write(&fork, "manifest.json", &fork_manifest("timeline-custom", "timeline", "1.0.0"));
        write(&fork, "index.tsx", user_code);
        // Updated shipped built-in (e.g., after software update)
        write(&shipped, "manifest.json", &builtin_manifest("timeline", "1.1.0"));
        write(&shipped, "index.tsx", new_shipped);
        write(&shipped, "helper.ts", "export const v = 2;\n");

        let report = auto_rebase_all(&[root.clone()], &builtins);
        assert_eq!(
            report.rebased,
            vec![("timeline-custom".into(), "timeline@1.1.0".into())]
        );
        assert!(report.conflicts.is_empty());

        // Both user customization AND app update fix are merged!
        let merged = fs::read_to_string(fork.join("index.tsx")).unwrap();
        assert!(merged.contains("user custom header"));
        assert!(merged.contains("app update fix"));
        assert_eq!(
            fs::read_to_string(fork.join("helper.ts")).unwrap(),
            "export const v = 2;\n"
        );
        // Fork identity kept, forkedFrom bumped, .forked-from updated.
        let m: Value = serde_json::from_str(&fs::read_to_string(fork.join("manifest.json")).unwrap()).unwrap();
        assert_eq!(m["id"], "timeline-custom");
        assert_eq!(m["name"], "My Timeline");
        assert_eq!(m["forkedFrom"], "timeline@1.1.0");
        assert_eq!(
            fs::read_to_string(fork.join(FORK_BASE).join("index.tsx")).unwrap(),
            new_shipped
        );

        // Second run is a clean O(1) no-op.
        let second = auto_rebase_all(&[root], &builtins);
        assert!(second.rebased.is_empty());
        assert!(second.conflicts.is_empty());
    }

    #[test]
    fn overlapping_conflict_keeps_existing_fork_intact() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("extensions");
        let builtins = tmp.path().join("builtins");
        let fork = root.join("timeline-custom");
        let shipped = builtins.join("timeline");

        write(&fork.join(FORK_BASE), "manifest.json", &builtin_manifest("timeline", "1.0.0"));
        write(&fork.join(FORK_BASE), "index.tsx", "const a = 1;\n");
        write(&fork, "manifest.json", &fork_manifest("timeline-custom", "timeline", "1.0.0"));
        write(&fork, "index.tsx", "const a = 42;\n");
        write(&shipped, "manifest.json", &builtin_manifest("timeline", "1.1.0"));
        write(&shipped, "index.tsx", "const a = 99;\n");

        let report = auto_rebase_all(&[root], &builtins);
        assert!(report.rebased.is_empty());
        assert_eq!(
            report.conflicts,
            vec![("timeline-custom".into(), vec!["index.tsx".into()])]
        );
        // User's live fork is completely untouched and still runnable!
        assert_eq!(
            fs::read_to_string(fork.join("index.tsx")).unwrap(),
            "const a = 42;\n"
        );
    }
}
