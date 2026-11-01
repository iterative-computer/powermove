//! File-only fork service. The immutable shipped tree is a host capability;
//! nothing is compiled, registered or executed while copying into run staging.
use pm_mcp::{
    Error, Result,
    extensions::{ExtensionSources, ForkedBuiltin},
};
use serde_json::{Value, json};
use std::{
    collections::BTreeSet,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
};
use url::Url;

pub struct BuiltinSources {
    root: PathBuf,
}
impl BuiltinSources {
    pub fn new(root: PathBuf) -> Result<Self> {
        if !root.is_absolute() {
            return Err(Error::invalid(
                "Extension fork roots must be absolute paths.",
            ));
        }
        Ok(Self { root })
    }
}
fn invalid(message: impl Into<String>) -> Error {
    Error::invalid(message)
}
fn valid_id(s: &str) -> bool {
    (2..=64).contains(&s.len())
        && (s.as_bytes()[0].is_ascii_lowercase() || s.as_bytes()[0].is_ascii_digit())
        && s.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}
fn version(s: &str) -> bool {
    let parts = s.split('.').collect::<Vec<_>>();
    parts.len() == 3
        && parts
            .iter()
            .all(|s| (1..=6).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_digit()))
}
fn utf16(s: &str) -> usize {
    s.encode_utf16().count()
}
fn text<'a>(raw: &'a Value, key: &str, max: usize, required: bool) -> Result<Option<&'a str>> {
    let value = &raw[key];
    if value.is_null() {
        return if required {
            Err(invalid(format!("invalid \"{key}\"")))
        } else {
            Ok(None)
        };
    }
    value
        .as_str()
        .filter(|s| !s.is_empty() && utf16(s) <= max)
        .map(Some)
        .ok_or_else(|| invalid(format!("invalid \"{key}\"")))
}
fn list(raw: &Value, key: &str) -> Result<()> {
    if let Some(value) = raw.get(key)
        && value.as_array().is_none_or(|a| {
            a.len() > 32 || a.iter().any(|v| v.as_str().is_none_or(|s| !valid_id(s)))
        })
    {
        return Err(invalid(format!("invalid \"{key}\"")));
    }
    Ok(())
}
fn link(s: &str) -> bool {
    if utf16(s) > 200 {
        return false;
    }
    let Ok(url) = Url::parse(s) else { return false };
    if url.scheme() != "https" || url.origin().ascii_serialization() != s {
        return false;
    }
    let host = url.host_str().unwrap_or("");
    let labels = host.split('.').collect::<Vec<_>>();
    labels.len() > 1
        && labels.iter().all(|label| {
            (1..=63).contains(&label.len())
                && label.as_bytes()[0].is_ascii_alphanumeric()
                && label.as_bytes()[label.len() - 1].is_ascii_alphanumeric()
                && label
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        })
        && labels.last().is_some_and(|s| {
            !s.bytes().all(|b| b.is_ascii_digit()) && !["localhost", "local"].contains(s)
        })
}
/// Admission from packages/registry/src/manifest.ts at v1.2.0. Unknown raw
/// fields survive fork rewriting; normalized fields are used only for checks.
pub fn validate_manifest(raw: &Value) -> Result<()> {
    if !raw.is_object() {
        return Err(invalid("manifest must be an object"));
    }
    let manifest_id = text(raw, "id", 64, true)?
        .filter(|s| valid_id(s))
        .ok_or_else(|| invalid("invalid \"id\" (a-z, 0-9, -; 2–64 chars)"))?;
    text(raw, "name", 80, true)?;
    if !text(raw, "version", 20, true)?.is_some_and(version) {
        return Err(invalid("invalid \"version\" (x.y.z)"));
    }
    let api = raw["apiVersion"]
        .as_f64()
        .filter(|n| *n >= 1. && n.fract() == 0.)
        .ok_or_else(|| invalid("invalid \"apiVersion\""))?;
    if api > 3. {
        return Err(invalid(format!(
            "apiVersion {api} is newer than this app (3)"
        )));
    }
    text(raw, "description", 400, false)?;
    if let Some(entry) = text(raw, "entry", 200, false)?
        && (entry.contains(['\\', '\0'])
            || entry.starts_with('/')
            || entry.contains("..")
            || ![".ts", ".js", ".mjs"]
                .iter()
                .any(|ext| entry.ends_with(ext)))
    {
        return Err(invalid("invalid \"entry\" path"));
    }
    if let Some(contributes) = raw.get("contributes") {
        let kinds = [
            "panels",
            "inspector",
            "media",
            "commands",
            "keybindings",
            "effects",
            "transitions",
            "layers",
            "themes",
            "palette",
            "menus",
            "status",
            "hooks",
        ];
        if contributes.as_array().is_none_or(|a| {
            a.iter()
                .any(|v| v.as_str().is_none_or(|s| !kinds.contains(&s)))
        }) {
            return Err(invalid("invalid \"contributes\""));
        }
    }
    for key in ["replaces", "dependsOn", "features", "integrates"] {
        list(raw, key)?;
    }
    for key in ["replaces", "dependsOn"] {
        if raw[key]
            .as_array()
            .is_some_and(|a| a.iter().any(|v| v == manifest_id))
        {
            return Err(invalid("extension cannot reference itself"));
        }
    }
    text(raw, "forkedFrom", 160, false)?;
    if let Some(vars) = raw.get("vars") {
        if api < 2. {
            return Err(invalid("\"vars\" requires apiVersion 2"));
        }
        let vars = vars
            .as_array()
            .filter(|a| a.len() <= 32)
            .ok_or_else(|| invalid("invalid \"vars\""))?;
        let mut seen = BTreeSet::new();
        for var in vars {
            let key = var["key"].as_str().unwrap_or("");
            if !var.is_object()
                || !(2..=64).contains(&key.len())
                || !key.as_bytes().first().is_some_and(u8::is_ascii_uppercase)
                || !key
                    .bytes()
                    .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_')
                || var["label"]
                    .as_str()
                    .is_none_or(|s| !(1..=80).contains(&utf16(s)))
                || var
                    .get("hint")
                    .is_some_and(|v| v.as_str().is_none_or(|s| utf16(s) > 200))
                || ["secret", "required"]
                    .iter()
                    .any(|key| var.get(key).is_some_and(|v| !v.is_boolean()))
            {
                return Err(invalid("invalid \"vars\""));
            }
            if !seen.insert(key) {
                return Err(invalid(format!("duplicate var key \"{key}\"")));
            }
        }
    }
    for key in ["permissions", "links"] {
        if let Some(values) = raw.get(key) {
            if api < 3. {
                return Err(invalid(format!("\"{key}\" requires apiVersion 3")));
            }
            let cap = if key == "links" { 5 } else { 8 };
            let values = values
                .as_array()
                .filter(|a| a.len() <= cap)
                .ok_or_else(|| invalid(format!("invalid \"{key}\"")))?;
            let mut seen = BTreeSet::new();
            for value in values {
                let s = value
                    .as_str()
                    .ok_or_else(|| invalid(format!("invalid \"{key}\"")))?;
                if !seen.insert(s)
                    || if key == "links" {
                        !link(s)
                    } else {
                        ![
                            "network",
                            "clipboard",
                            "assets",
                            "project:write",
                            "full-access",
                        ]
                        .contains(&s)
                    }
                {
                    return Err(invalid(format!("invalid \"{key}\"")));
                }
            }
        }
    }
    if raw.get("author").is_some_and(|v| {
        v.as_str()
            .is_none_or(|s| !["powermove", "user", "agent"].contains(&s))
    }) {
        return Err(invalid("invalid \"author\""));
    }
    Ok(())
}
fn regular_dir(path: &Path) -> Result<()> {
    let m = fs::symlink_metadata(path)?;
    if !m.is_dir() || m.is_symlink() {
        return Err(invalid("Extension directory is not a regular directory."));
    }
    Ok(())
}
fn write(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut f = OpenOptions::new().write(true).create_new(true).open(path)?;
    f.write_all(bytes)?;
    f.sync_all()?;
    Ok(())
}
fn copy_tree(
    source: &Path,
    dest: &Path,
    prefix: &str,
    files: &mut Vec<String>,
    bytes: &mut u64,
) -> Result<()> {
    if prefix.split('/').count() > 64 {
        return Err(invalid("Extension source tree is too deeply nested"));
    }
    let mut entries = fs::read_dir(source)?.collect::<std::io::Result<Vec<_>>>()?;
    entries.sort_by_key(|e| e.file_name());
    for entry in entries {
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| invalid("Extension source path is not UTF-8"))?;
        let metadata = fs::symlink_metadata(entry.path())?;
        if name == ".DS_Store"
            || (metadata.is_dir() && (name.starts_with('.') || name == "__tests__"))
            || (metadata.is_file() && name.ends_with(".test.ts"))
        {
            continue;
        }
        if metadata.is_symlink() {
            return Err(invalid(
                "Built-in extension contains an unsupported symbolic link",
            ));
        }
        let relative = if prefix.is_empty() {
            name.clone()
        } else {
            format!("{prefix}/{name}")
        };
        if metadata.is_dir() {
            fs::create_dir(dest.join(&name))?;
            copy_tree(&entry.path(), &dest.join(name), &relative, files, bytes)?;
        } else if metadata.is_file() {
            if files.len() >= 400
                || metadata.len() > 8 * 1024 * 1024
                || *bytes + metadata.len() > 8 * 1024 * 1024
            {
                return Err(invalid("Extension source exceeds the file/byte budget"));
            }
            let mut data = Vec::new();
            File::open(entry.path())?
                .take(8 * 1024 * 1024 + 1)
                .read_to_end(&mut data)?;
            *bytes += data.len() as u64;
            if *bytes > 8 * 1024 * 1024 {
                return Err(invalid("Extension source exceeds the byte budget"));
            }
            // The authored manifest is published only after all source/base files
            // are complete. Failed staging remains inert and recoverable (N4).
            if relative != "manifest.json" {
                write(&dest.join(&name), &data)?;
            }
            files.push(relative);
        } else {
            return Err(invalid("Built-in extension contains an unsupported file"));
        }
    }
    File::open(dest)?.sync_all()?;
    Ok(())
}
impl ExtensionSources for BuiltinSources {
    fn fork_builtin(&self, id: &str, fork: Option<&str>, staging: &Path) -> Result<ForkedBuiltin> {
        let fork_id = fork
            .map(str::to_owned)
            .unwrap_or_else(|| format!("{id}-fork"));
        if !valid_id(id) || !valid_id(&fork_id) {
            return Err(invalid("Invalid extension id."));
        }
        if fork_id == id {
            return Err(invalid(
                "Fork id must differ from the built-in extension id.",
            ));
        }
        if !staging.is_absolute() {
            return Err(invalid("Extension fork roots must be absolute paths."));
        }
        let root = fs::canonicalize(&self.root)?;
        let source = root.join(id);
        regular_dir(&source)?;
        let manifest_file = source.join("manifest.json");
        let meta = fs::symlink_metadata(&manifest_file)?;
        if !meta.is_file() || meta.is_symlink() || meta.len() > 8 * 1024 * 1024 {
            return Err(invalid(
                "Built-in extension manifest is not a bounded regular file.",
            ));
        }
        let raw: Value = serde_json::from_slice(&fs::read(&manifest_file)?)
            .map_err(|_| invalid("Built-in extension has invalid manifest JSON."))?;
        validate_manifest(&raw)?;
        if raw["id"] != id {
            return Err(invalid(
                "Built-in extension directory does not match manifest id.",
            ));
        }
        let forked_from = format!("{id}@{}", raw["version"].as_str().unwrap());
        let mut rewritten = raw.clone();
        rewritten["id"] = json!(fork_id);
        rewritten["name"] = json!(format!("{} (fork)", raw["name"].as_str().unwrap()));
        let mut replaces = raw["replaces"].as_array().cloned().unwrap_or_default();
        if !replaces.contains(&json!(id)) {
            replaces.push(json!(id));
        }
        let mut seen = BTreeSet::new();
        replaces.retain(|v| seen.insert(v.as_str().unwrap().to_owned()));
        rewritten["replaces"] = json!(replaces);
        rewritten["forkedFrom"] = json!(forked_from);
        rewritten["author"] = raw.get("author").cloned().unwrap_or(json!("user"));
        validate_manifest(&rewritten)?;
        fs::create_dir_all(staging)?;
        regular_dir(staging)?;
        let staging = fs::canonicalize(staging)?;
        let dest = staging.join(&fork_id);
        // Exclusive directory reservation cannot replace a concurrent author's
        // work. Manifest-last publication is safe because run staging is inert.
        fs::create_dir(&dest).map_err(|e| {
            if e.kind() == std::io::ErrorKind::AlreadyExists {
                invalid("Extension fork target already exists.")
            } else {
                e.into()
            }
        })?;
        let mut files = vec![];
        copy_tree(&source, &dest, "", &mut files, &mut 0)?;
        let pristine = dest.join(".forked-from");
        fs::create_dir(&pristine)?;
        copy_tree(&source, &pristine, "", &mut vec![], &mut 0)?;
        let original = fs::read(&manifest_file)?;
        write(&pristine.join("manifest.json"), &original)?;
        let manifest = format!("{}\n", serde_json::to_string_pretty(&rewritten).unwrap());
        write(&dest.join("manifest.json"), manifest.as_bytes())?;
        File::open(&pristine)?.sync_all()?;
        File::open(&dest)?.sync_all()?;
        File::open(staging)?.sync_all()?;
        Ok(ForkedBuiltin {
            fork_id,
            dir: dest,
            files,
            forked_from,
        })
    }
}
