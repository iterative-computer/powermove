//! Corpus-only selection/binding. Never resolve an uncaptured host path.
use serde_json::Value;
use std::{collections::BTreeSet, path::PathBuf};

pub fn project_assets(v: &Value) -> Value {
    fn scan(v: &Value, ids: &mut BTreeSet<String>) {
        match v {
            Value::Object(m) => {
                for (k, x) in m {
                    if matches!(k.as_str(), "assets" | "history") {
                        continue;
                    }
                    if matches!(k.as_str(), "asset" | "assetId") {
                        if let Some(id) = x.as_str() {
                            ids.insert(id.into());
                        }
                    } else if k == "maps" {
                        // 3D material maps name assets by texture slot, not assetId.
                        if let Some(maps) = x.as_object() {
                            ids.extend(maps.values().filter_map(Value::as_str).map(str::to_owned));
                        }
                    }
                    scan(x, ids);
                }
            }
            Value::Array(a) => {
                for x in a {
                    scan(x, ids);
                }
            }
            _ => {}
        }
    }
    let mut ids = BTreeSet::new();
    scan(v, &mut ids);
    Value::Object(
        v["assets"]
            .as_object()
            .map(|all| {
                ids.into_iter()
                    .filter_map(|id| all.get(&id).map(|a| (id, a.clone())))
                    .collect()
            })
            .unwrap_or_default(),
    )
}

pub fn proxy(a: &Value) -> bool {
    a["playbackProxy"] == true || a["playbackProxyVersion"].as_f64().is_some_and(|n| n > 0.)
}

fn keys(id: &str, a: &Value) -> Vec<String> {
    let mut keys = Vec::new();
    for key in [
        a["storageKey"].as_str().map(str::to_owned),
        a["fingerprint"].as_str().map(|s| format!("media:{s}")),
        Some(id.to_owned()),
    ]
    .into_iter()
    .flatten()
    {
        if !key.is_empty() && !keys.contains(&key) {
            keys.push(key);
        }
    }
    keys
}

#[derive(Debug, PartialEq, Eq)]
pub struct Binding {
    pub path: PathBuf,
    pub proxy: bool,
    pub expected_hash: Option<String>,
}

impl Binding {
    pub fn verify(
        &self,
        root: &std::path::Path,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        if !self.path.canonicalize()?.starts_with(root.canonicalize()?) {
            return Err("media escaped snapshot".into());
        }
        if let Some(hash) = &self.expected_hash {
            let mut input = std::fs::File::open(&self.path)?;
            let mut hasher = blake3::Hasher::new();
            std::io::copy(&mut input, &mut hasher)?;
            if hasher.finalize().to_hex().as_str() != hash {
                return Err("media_content_hash_mismatch".into());
            }
        }
        Ok(())
    }
}

/// Captured candidates and their byte identities. Paths are validated by the caller.
/// Both capture and binding use key precedence; a stale bare-ID copy cannot make
/// a content-key original ambiguous. Missing/partial checksums never prove equality.
pub fn binding(id: &str, asset: &Value, map: &Value) -> Result<Option<Binding>, &'static str> {
    let records = map["records"].as_array().ok_or("invalid_media_manifest")?;
    let mut chosen = Vec::new();
    for key in keys(id, asset) {
        chosen = records
            .iter()
            .filter(|r| {
                r["key"] == key
                    && asset["revision"]
                        .as_str()
                        .is_none_or(|rev| r["revision"] == rev)
            })
            .collect();
        if !chosen.is_empty() {
            break;
        }
    }
    if !chosen.is_empty() {
        let first = chosen[0];
        if chosen.len() > 1 {
            let hash = first["sha256"].as_str().ok_or("ambiguous_media")?;
            if hash.len() != 64 || !hash.bytes().all(|b| b.is_ascii_hexdigit()) {
                return Err("ambiguous_media");
            }
            if chosen.iter().any(|r| {
                r["sha256"].as_str() != Some(hash) || proxy(&r["value"]) != proxy(&first["value"])
            }) {
                return Err("ambiguous_media");
            }
        }
        let path = first["path"].as_str().ok_or("invalid_media_manifest")?;
        return Ok(Some(Binding {
            path: path.into(),
            proxy: proxy(asset) || proxy(&first["value"]),
            expected_hash: asset["contentHash"]
                .as_str()
                .or_else(|| first["blake3"].as_str())
                .map(str::to_owned),
        }));
    }
    for source in [
        asset["link"]["path"].as_str(),
        asset["sourcePath"].as_str(),
        asset["path"].as_str(),
    ]
    .into_iter()
    .flatten()
    {
        if let Some(path) = map["linked"][source].as_str() {
            return Ok(Some(Binding {
                path: path.into(),
                proxy: proxy(asset),
                expected_hash: asset["link"]["hash"].as_str().map(str::to_owned),
            }));
        }
    }
    Ok(None)
}
