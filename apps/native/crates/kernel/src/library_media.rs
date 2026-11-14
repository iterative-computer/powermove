//! The library's content-addressed media store (`<profile>/Media Store`), the
//! layout main's `media-store/store.ts` defines and the first launch fills from
//! the original app's IndexedDB: `keys/<sha256(key)>.json` maps a media key to
//! `objects/<hash[..2]>/<hash>`. Library projects reference these bytes by
//! `storageKey` (or `media:<fingerprint>`), as they referenced IndexedDB.
use crate::project_io::{MediaEntry, MediaSource, MediaTable};
use pm_model::{Node, NodeKind, Op, Value, patch::PathPart};
use sha2::{Digest, Sha256};
use std::{
    collections::HashSet,
    path::{Path, PathBuf},
};

pub const DIR: &str = "Media Store";

/// Adds the stored bytes of every asset the table lacks, whether the document
/// holds it now or only its undo history does (Save As carries both). An entry
/// the project file already embeds, or one missing or the wrong size in the
/// store, is left alone. Bytes the first launch classified as a proxy whose
/// original is gone stay marked as such ([`MediaTable::insert_proxy_only`]).
pub(crate) fn hydrate(root: &Node, history: &crate::History, media: &mut MediaTable, store: &Path) {
    let mut wanted: Vec<(String, Option<String>, Option<String>)> = Vec::new();
    if let Some(NodeKind::Object(assets)) = root.get("assets").map(Node::kind) {
        for (id, asset) in assets.iter() {
            if let Some(id) = id.as_str() {
                wanted.push((
                    id.to_owned(),
                    text(asset.get("storageKey")).map(str::to_owned),
                    text(asset.get("fingerprint")).map(str::to_owned),
                ));
            }
        }
    }
    history_assets(history, |id, asset| {
        wanted.push((
            id.to_owned(),
            asset
                .get("storageKey")
                .and_then(Value::as_str)
                .map(str::to_owned),
            asset
                .get("fingerprint")
                .and_then(Value::as_str)
                .map(str::to_owned),
        ))
    });
    let mut proxy_only = None;
    for (id, storage_key, fingerprint) in wanted {
        if media.get(&id).is_some() {
            continue;
        }
        let Some((entry, key)) = stored(&id, storage_key, fingerprint, store) else {
            continue;
        };
        if proxy_only
            .get_or_insert_with(|| proxy_only_keys(store))
            .contains(&key)
        {
            media.insert_proxy_only(entry);
        } else {
            media.insert(entry);
        }
    }
}

/// Asset records an undo step would put back: `assets.<id>` leaves, and any
/// inserted or patched object that names its media key, with their asset id.
pub(crate) fn history_assets(history: &crate::History, mut f: impl FnMut(&str, &Value)) {
    fn walk(v: &Value, id: Option<&str>, f: &mut impl FnMut(&str, &Value)) {
        match v {
            Value::Object(m) => {
                if let Some(id) = m.get("id").and_then(Value::as_str).or(id)
                    && (m.get("storageKey").and_then(Value::as_str).is_some()
                        || m.get("fingerprint").and_then(Value::as_str).is_some())
                {
                    f(id, v);
                }
                for (_, v) in m.iter() {
                    walk(v, None, f);
                }
            }
            Value::Array(a) => a.iter().for_each(|v| walk(v, None, f)),
            _ => {}
        }
    }
    for record in history.records() {
        for op in record.forward.iter().chain(record.inverse.iter()) {
            match op {
                Op::Leaf(patch) => {
                    let id = match patch.path.as_slice() {
                        [.., PathPart::Key(parent), PathPart::Key(id)] if *parent == "assets" => {
                            id.as_str()
                        }
                        _ => None,
                    };
                    if let Some(value) = &patch.value {
                        walk(value, id, &mut f);
                    }
                }
                Op::Insert { value, .. } => walk(&value, None, &mut f),
                _ => {}
            }
        }
    }
}

/// Media keys every completed first-launch pass classified as proxy-only
/// (`migration.json`). An unreadable record counts nothing as an original:
/// it marks every key, since proxies must never pass for originals.
fn proxy_only_keys(store: &Path) -> ProxyOnly {
    let Ok(bytes) = std::fs::read(store.join("migration.json")) else {
        return ProxyOnly::None;
    };
    let Some(state) = std::str::from_utf8(&bytes)
        .ok()
        .and_then(|t| pm_model::json::parse(t).ok())
    else {
        return ProxyOnly::All;
    };
    let mut keys = HashSet::new();
    let passes = state.get("passes").and_then(Value::as_object);
    for (_, pass) in passes.into_iter().flat_map(|p| p.iter()) {
        for entry in pass
            .get("closure")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let proxy = entry.get("status").and_then(Value::as_str) == Some("proxy-only")
                || entry.get("exportWarning").and_then(Value::as_bool) == Some(true);
            if !proxy {
                continue;
            }
            for key in [
                entry.get("reference").and_then(|r| r.get("key")),
                entry.get("stored").and_then(|r| r.get("key")),
            ]
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            {
                keys.insert(key.to_owned());
            }
        }
    }
    ProxyOnly::Keys(keys)
}

enum ProxyOnly {
    None,
    All,
    Keys(HashSet<String>),
}
impl ProxyOnly {
    fn contains(&self, key: &str) -> bool {
        match self {
            ProxyOnly::None => false,
            ProxyOnly::All => true,
            ProxyOnly::Keys(keys) => keys.contains(key),
        }
    }
}

/// Whether `entry` is bytes of the library's store, which a library save
/// need not copy anywhere: the store keeps them.
pub(crate) fn is_stored(entry: &MediaEntry, store: &Path) -> bool {
    let MediaSource::File { path, length, .. } = &entry.source else {
        return false;
    };
    if path.starts_with(store.join("objects")) {
        return true;
    }
    use std::os::unix::fs::MetadataExt;
    let keys = store.join("keys");
    let Ok(stamp) = std::fs::metadata(&keys) else {
        return false;
    };
    let identity = (stamp.ino(), stamp.mtime(), stamp.mtime_nsec());
    thread_local! {
        // Atomic key publication changes the directory stamp. Cache the index,
        // not source availability, to avoid rereading every key for every asset.
        static LINKS: std::cell::RefCell<Option<(PathBuf, (u64, i64, i64), HashSet<(PathBuf, u64)>)>> = const { std::cell::RefCell::new(None) };
    }
    let linked = LINKS.with(|cache| {
        let mut cache = cache.borrow_mut();
        if cache
            .as_ref()
            .is_none_or(|(p, s, _)| p != &keys || *s != identity)
        {
            let mut paths = HashSet::new();
            for file in std::fs::read_dir(&keys)
                .ok()
                .into_iter()
                .flatten()
                .flatten()
            {
                let Ok(bytes) = std::fs::read(file.path()) else {
                    continue;
                };
                let Some(record) = std::str::from_utf8(&bytes)
                    .ok()
                    .and_then(|s| pm_model::json::parse(s).ok())
                else {
                    continue;
                };
                if let Some(path) = record.get("path").and_then(Value::as_str)
                    && Path::new(path).is_absolute()
                    && let Some(size) = record.get("size").and_then(Value::as_f64)
                {
                    paths.insert((PathBuf::from(path), size as u64));
                }
            }
            *cache = Some((keys, identity, paths));
        }
        cache.as_ref().unwrap().2.contains(&(path.clone(), *length))
    });
    linked && std::fs::symlink_metadata(path).is_ok_and(|m| m.is_file() && m.len() == *length)
}

fn text(node: Option<&Node>) -> Option<&str> {
    match node?.kind() {
        NodeKind::String(s) => s.as_str(),
        _ => None,
    }
}

/// Whether the store holds bytes for the asset (as [`hydrate`] would find them).
pub(crate) fn holds(
    id: &str,
    storage_key: Option<&str>,
    fingerprint: Option<&str>,
    store: &Path,
) -> bool {
    stored(
        id,
        storage_key.map(str::to_owned),
        fingerprint.map(str::to_owned),
        store,
    )
    .is_some()
}

/// The asset's stored bytes and the media key that found them.
fn stored(
    id: &str,
    storage_key: Option<String>,
    fingerprint: Option<String>,
    store: &Path,
) -> Option<(MediaEntry, String)> {
    let keys = [
        storage_key,
        fingerprint.map(|f| format!("media:{f}")),
        Some(id.to_owned()),
    ];
    keys.into_iter().flatten().find_map(|key| {
        let record = std::fs::read(
            store
                .join("keys")
                .join(format!("{:x}.json", Sha256::digest(key.as_bytes()))),
        )
        .ok()?;
        let record = pm_model::json::parse(std::str::from_utf8(&record).ok()?).ok()?;
        (record.get("key").and_then(Value::as_str) == Some(key.as_str())).then_some(())?;
        let hash = record.get("hash").and_then(Value::as_str)?;
        let size = record.get("size").and_then(Value::as_f64)?;
        if hash.len() != 64 || !hash.bytes().all(|b| b.is_ascii_hexdigit()) || size < 0. {
            return None;
        }
        let valid = |p: &Path| {
            std::fs::symlink_metadata(p)
                .ok()
                .is_some_and(|m| m.is_file() && m.len() == size as u64)
        };
        let object = store.join("objects").join(&hash[..2]).join(hash);
        let path: PathBuf = record
            .get("path")
            .and_then(Value::as_str)
            .map(PathBuf::from)
            .filter(|p| p.is_absolute() && valid(p))
            .unwrap_or(object);
        valid(&path).then_some(())?;
        let entry = MediaEntry {
            id: id.into(),
            media_type: record
                .get("type")
                .and_then(Value::as_str)
                .unwrap_or("application/octet-stream")
                .into(),
            revision: record
                .get("revision")
                .and_then(Value::as_str)
                .unwrap_or(hash)
                .into(),
            source: MediaSource::File {
                path,
                length: size as u64,
                sha256: None,
            },
        };
        Some((entry, key))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stored_bytes_resolve_by_storage_key_and_a_bad_size_stays_offline() {
        let store = std::env::temp_dir().join(crate::new_id("library-media"));
        let hash = "ab".repeat(32);
        std::fs::create_dir_all(store.join("objects/ab")).unwrap();
        std::fs::create_dir_all(store.join("keys")).unwrap();
        std::fs::write(store.join("objects/ab").join(&hash), b"synthetic").unwrap();
        let key = |key: &str, size: u64| {
            let name = format!("{:x}.json", Sha256::digest(key.as_bytes()));
            let record = format!(
                r#"{{"key":"{key}","hash":"{hash}","size":{size},"type":"image/png","revision":"r1","at":1,"source":"indexeddb"}}"#
            );
            std::fs::write(store.join("keys").join(name), record).unwrap();
        };
        key("store:a", 9);
        key("media:v2:9:short", 4);
        let root = Node::parse_bytes(
            br#"{"assets":{"a":{"storageKey":"store:a"},"b":{"fingerprint":"v2:9:short"},"c":{"name":"none"}}}"#,
            1,
        )
        .unwrap();
        let mut media = MediaTable::default();
        hydrate(&root, &crate::History::new(), &mut media, &store);
        let a = media.get("a").expect("stored by key");
        assert!(is_stored(a, &store));
        assert_eq!(a.length(), 9);
        assert!(
            media.get("b").is_none(),
            "a size mismatch is not these bytes"
        );
        assert!(media.get("c").is_none());
        std::fs::remove_dir_all(store).unwrap();
    }
}

#[cfg(test)]
mod linked_tests {
    use super::*;
    #[test]
    fn linked_is_stored_and_invalid_link_falls_back_to_object() {
        let store = std::env::temp_dir().join(crate::new_id("linked-media"));
        std::fs::create_dir_all(store.join("keys")).unwrap();
        let path = store.join("external.mp4");
        std::fs::write(&path, b"abc").unwrap();
        let hash = "ab".repeat(32);
        let record = format!(
            r#"{{"key":"key","hash":"{hash}","size":3,"path":{:?}}}"#,
            path.to_str().unwrap()
        );
        std::fs::write(
            store
                .join("keys")
                .join(format!("{:x}.json", Sha256::digest(b"key"))),
            record,
        )
        .unwrap();
        let (entry, _) = stored("a", Some("key".into()), None, &store).unwrap();
        assert!(is_stored(&entry, &store));
        std::fs::remove_file(path).unwrap();
        assert!(!is_stored(&entry, &store));
        assert!(stored("a", Some("key".into()), None, &store).is_none());
        std::fs::create_dir_all(store.join("objects/ab")).unwrap();
        std::fs::write(store.join("objects/ab").join(hash), b"abc").unwrap();
        assert!(stored("a", Some("key".into()), None, &store).is_some());
        std::fs::remove_dir_all(store).unwrap();
    }
}
