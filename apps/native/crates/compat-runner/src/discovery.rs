//! Immediate extension children of the user's roots, without following links.
use crate::manifest;
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

#[derive(Debug, Clone, PartialEq)]
pub struct Discovered {
    pub id: String,
    pub directory: PathBuf,
    /// `Err` holds the frozen parser's reason; the record shows it in Settings.
    pub manifest: Result<Value, String>,
}

fn scan_candidate(directory: &Path, name: &str) -> Option<Discovered> {
    let meta = fs::symlink_metadata(directory).ok()?;
    if meta.file_type().is_symlink() || !meta.is_dir() {
        return None;
    }
    let path = directory.join("manifest.json");
    let meta = fs::symlink_metadata(&path).ok()?;
    if meta.file_type().is_symlink() || !meta.is_file() {
        return None;
    }
    let parsed = fs::read(&path)
        .map_err(|e| e.to_string())
        .and_then(|bytes| serde_json::from_slice(&bytes).map_err(|e| e.to_string()))
        .and_then(|raw: Value| manifest::parse(&raw))
        .and_then(|manifest| {
            if manifest["id"] == name {
                Ok(manifest)
            } else {
                Err(format!("manifest id does not match folder \"{name}\""))
            }
        });
    Some(Discovered {
        id: name.to_owned(),
        directory: directory.to_owned(),
        manifest: parsed,
    })
}

/// Later roots replace earlier ones. A missing root is an empty one.
pub fn scan(roots: &[PathBuf]) -> Vec<Discovered> {
    let mut found = BTreeMap::new();
    for root in roots {
        let Ok(entries) = fs::read_dir(root) else {
            continue;
        };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if manifest::extension_id(&name)
                && let Some(candidate) = scan_candidate(&entry.path(), &name)
            {
                found.insert(name, candidate);
            }
        }
    }
    found.into_values().collect()
}

/// The record the realm's loader plans from.
pub fn record(found: &Discovered, enabled: bool, graph: Result<Value, String>) -> Value {
    let mut record = json!({"id":found.id,"scope":"user","enabled":enabled,"trust":"local"});
    match (&found.manifest, graph) {
        (Err(error), _) => {
            record["manifest"] = Value::Null;
            record["health"] = json!({"state":"manifest-error","error":error});
        }
        (Ok(manifest), Err(error)) => {
            record["manifest"] = manifest.clone();
            record["health"] = json!({"state":"build-error","error":error});
        }
        (Ok(manifest), Ok(graph)) => {
            record["manifest"] = manifest.clone();
            record["graph"] = graph;
            record["health"] = json!({"state":"ok"});
        }
    }
    record
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn later_roots_win_and_broken_manifests_stay_visible_with_their_reason() {
        let root = tempfile::tempdir().unwrap();
        let (first, second) = (root.path().join("a"), root.path().join("b"));
        for (base, id, version) in [
            (&first, "kept", "1.0.0"),
            (&first, "shadowed", "1.0.0"),
            (&second, "shadowed", "2.0.0"),
        ] {
            fs::create_dir_all(base.join(id)).unwrap();
            fs::write(
                base.join(id).join("manifest.json"),
                json!({"id":id,"name":id,"version":version,"apiVersion":1}).to_string(),
            )
            .unwrap();
        }
        fs::create_dir_all(second.join("broken")).unwrap();
        fs::write(second.join("broken/manifest.json"), "{").unwrap();
        fs::create_dir_all(second.join("Not-An-Id")).unwrap();
        let found = scan(&[first, second, root.path().join("missing")]);
        assert_eq!(
            found.iter().map(|f| f.id.as_str()).collect::<Vec<_>>(),
            ["broken", "kept", "shadowed"]
        );
        assert!(found[0].manifest.is_err());
        assert_eq!(found[2].manifest.as_ref().unwrap()["version"], "2.0.0");
        let record = record(&found[0], true, Ok(json!({})));
        assert_eq!(record["health"]["state"], "manifest-error");
    }
}
