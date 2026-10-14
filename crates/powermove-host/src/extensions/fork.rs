//! Copy a shipped extension into the user folder (port of `fork.ts`).
//!
//! The pristine `.forked-from` tree is kept as the merge base for a later
//! three-way "update fork" when the shipped built-in changes (`rebase.rs`).

use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, bail};
use serde_json::{Value, json};

use super::manifest::{is_extension_id, parse_manifest};

pub struct Forked {
    pub fork_id: String,
    pub dir: PathBuf,
    pub files: Vec<String>,
    pub forked_from: String,
}

/// Fork built-in `id` from `resources_dir` into `target_dir` as `fork_id`
/// (default `<id>-fork`). Blocking.
pub fn fork_builtin(resources_dir: &Path, id: &str, target_dir: &Path, fork_id: Option<&str>) -> Result<Forked> {
    let id = validate_id(id, "built-in extension")?;
    let fork_id = fork_id.map_or_else(|| format!("{id}-fork"), str::to_owned);
    validate_id(&fork_id, "fork")?;
    if fork_id == id {
        bail!("Fork id must differ from the built-in extension id.");
    }
    if !resources_dir.is_absolute() || !target_dir.is_absolute() {
        bail!("Extension fork roots must be absolute paths.");
    }

    let resources_root = dunce::canonicalize(resources_dir)?;
    let source = resources_root.join(id);
    require_immediate_directory(&resources_root, &source, "Built-in extension")?;

    let manifest_path = source.join("manifest.json");
    let metadata = std::fs::symlink_metadata(&manifest_path)?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        bail!("Built-in extension {id} manifest is not a regular file.");
    }
    let raw: Value = serde_json::from_str(&std::fs::read_to_string(&manifest_path)?)
        .ok()
        .with_context(|| format!("Built-in extension {id} has invalid manifest JSON."))?;
    let manifest = parse_manifest(&raw).map_err(|error| anyhow::anyhow!("Built-in extension {id} has an invalid manifest: {error}"))?;
    if manifest.id != id {
        bail!("Built-in extension directory {id} does not match manifest id {}.", manifest.id);
    }

    std::fs::create_dir_all(target_dir)?;
    let target_root = dunce::canonicalize(target_dir)?;
    let destination = target_root.join(&fork_id);
    match std::fs::symlink_metadata(&destination) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
        Ok(_) => bail!("Extension fork target already exists."),
    }

    let forked_from = format!("{id}@{}", manifest.version);
    let mut replaces = manifest.replaces.clone().unwrap_or_default();
    if !replaces.iter().any(|item| item == id) {
        replaces.push(id.to_owned());
    }
    let mut rewritten = raw.as_object().cloned().unwrap_or_default();
    rewritten.insert("id".into(), json!(fork_id));
    rewritten.insert("name".into(), json!(format!("{} (fork)", manifest.name)));
    rewritten.insert("replaces".into(), json!(replaces));
    rewritten.insert("forkedFrom".into(), json!(forked_from));
    rewritten.insert("author".into(), json!(manifest.author.as_deref().unwrap_or("user")));
    let rewritten = Value::Object(rewritten);
    if let Err(error) = parse_manifest(&rewritten) {
        bail!("Fork manifest would be invalid: {error}");
    }

    let temporary = target_root.join(format!(".{fork_id}.tmp-{}", hex::encode(rand::random::<[u8; 8]>())));
    std::fs::create_dir(&temporary)?;
    let written = (|| -> Result<Vec<String>> {
        let files = copy_source_tree(&source, &temporary, "")?;
        let pristine = temporary.join(".forked-from");
        std::fs::create_dir(&pristine)?;
        copy_source_tree(&source, &pristine, "")?;
        std::fs::write(temporary.join("manifest.json"), format!("{}\n", serde_json::to_string_pretty(&rewritten)?))?;
        std::fs::rename(&temporary, &destination)?;
        Ok(files)
    })();
    match written {
        Ok(files) => Ok(Forked { fork_id, dir: destination, files, forked_from }),
        Err(error) => {
            let _ = std::fs::remove_dir_all(&temporary);
            Err(error)
        }
    }
}

fn copy_source_tree(source: &Path, destination: &Path, prefix: &str) -> Result<Vec<String>> {
    let mut copied = Vec::new();
    let mut entries: Vec<std::fs::DirEntry> = std::fs::read_dir(source)?.collect::<std::io::Result<_>>()?;
    entries.sort_by_key(|entry| entry.file_name().to_string_lossy().to_lowercase());
    for entry in entries {
        let name = entry.file_name().to_string_lossy().into_owned();
        let kind = entry.file_type()?;
        if name == ".DS_Store" || (name.starts_with('.') && kind.is_dir()) {
            continue;
        }
        let relative = if prefix.is_empty() { name.clone() } else { format!("{prefix}/{name}") };
        if (kind.is_dir() && name == "__tests__") || (kind.is_file() && name.ends_with(".test.ts")) {
            continue;
        }
        let source_path = entry.path();
        let destination_path = destination.join(&name);
        let metadata = std::fs::symlink_metadata(&source_path)?;
        if metadata.file_type().is_symlink() {
            bail!("Built-in extension contains an unsupported symbolic link: {relative}");
        }
        if metadata.is_dir() {
            std::fs::create_dir(&destination_path)?;
            copied.extend(copy_source_tree(&source_path, &destination_path, &relative)?);
        } else if metadata.is_file() {
            // COPYFILE_EXCL: never overwrite.
            std::fs::OpenOptions::new().write(true).create_new(true).open(&destination_path)?;
            std::fs::copy(&source_path, &destination_path)?;
            copied.push(relative);
        } else {
            bail!("Built-in extension contains an unsupported file: {relative}");
        }
    }
    Ok(copied)
}

fn validate_id<'a>(value: &'a str, label: &str) -> Result<&'a str> {
    if !is_extension_id(value) {
        bail!("Invalid {label} id.");
    }
    Ok(value)
}

fn require_immediate_directory(root: &Path, candidate: &Path, label: &str) -> Result<()> {
    let metadata = match std::fs::symlink_metadata(candidate) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => bail!("{label} does not exist."),
        Err(error) => return Err(error.into()),
    };
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        bail!("{label} is not a regular directory.");
    }
    if dunce::canonicalize(candidate)?.parent() != Some(root) {
        bail!("{label} path escapes its root.");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn forks_a_builtin_with_its_pristine_base() {
        let root = tempfile::tempdir().unwrap();
        let builtins = root.path().join("builtin");
        let source = builtins.join("tools");
        std::fs::create_dir_all(source.join("__tests__")).unwrap();
        std::fs::write(source.join("manifest.json"), r#"{ "id": "tools", "name": "Tools", "version": "1.0.0", "apiVersion": 3, "extra": 1 }"#).unwrap();
        std::fs::write(source.join("index.ts"), "export default () => {}").unwrap();
        std::fs::write(source.join("index.test.ts"), "test").unwrap();
        let users = root.path().join("user");

        let forked = fork_builtin(&builtins, "tools", &users, None).unwrap();
        assert_eq!(forked.fork_id, "tools-fork");
        assert_eq!(forked.files, vec!["index.ts".to_owned(), "manifest.json".to_owned()]);
        let manifest: Value = serde_json::from_str(&std::fs::read_to_string(forked.dir.join("manifest.json")).unwrap()).unwrap();
        assert_eq!(manifest["name"], "Tools (fork)");
        assert_eq!(manifest["replaces"], json!(["tools"]));
        assert_eq!(manifest["forkedFrom"], "tools@1.0.0");
        assert_eq!(manifest["author"], "user");
        assert_eq!(manifest["extra"], 1);
        assert!(forked.dir.join(".forked-from").join("index.ts").is_file());
        assert!(!forked.dir.join("__tests__").exists());

        let again = fork_builtin(&builtins, "tools", &users, None).err().unwrap();
        assert_eq!(again.to_string(), "Extension fork target already exists.");
        assert_eq!(fork_builtin(&builtins, "missing", &users, None).err().unwrap().to_string(), "Built-in extension does not exist.");
    }
}
