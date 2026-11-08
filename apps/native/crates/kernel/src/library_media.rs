//! The library's content-addressed media store (`<profile>/Media Store`), the
//! layout main's `media-store/store.ts` defines and the first launch fills from
//! the original app's IndexedDB: `keys/<sha256(key)>.json` maps a media key to
//! `objects/<hash[..2]>/<hash>`. Library projects reference these bytes by
//! `storageKey` (or `media:<fingerprint>`), as they referenced IndexedDB.
use crate::project_io::{MediaEntry, MediaSource, MediaTable};
use pm_model::{Node, NodeKind, Value};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

pub const DIR: &str = "Media Store";

/// Adds the stored bytes of every asset the table lacks. An entry the
/// project file already embeds, or one that is missing or the wrong size in
/// the store, is left alone.
pub(crate) fn hydrate(root: &Node, media: &mut MediaTable, store: &Path) {
    let Some(NodeKind::Object(assets)) = root.get("assets").map(Node::kind) else {
        return;
    };
    for (id, asset) in assets.iter() {
        let Some(id) = id.as_str() else { continue };
        if media.get(id).is_some() {
            continue;
        }
        if let Some(entry) = stored(id, asset, store) {
            media.insert(entry);
        }
    }
}

/// Whether `entry` is bytes of the library's store, which a library save
/// need not copy anywhere: the store keeps them.
pub(crate) fn is_stored(entry: &MediaEntry, store: &Path) -> bool {
    matches!(&entry.source, MediaSource::File { path, .. } if path.starts_with(store.join("objects")))
}

fn text(node: Option<&Node>) -> Option<&str> {
    match node?.kind() {
        NodeKind::String(s) => s.as_str(),
        _ => None,
    }
}

fn stored(id: &str, asset: &Node, store: &Path) -> Option<MediaEntry> {
    let keys = [
        text(asset.get("storageKey")).map(str::to_owned),
        text(asset.get("fingerprint")).map(|f| format!("media:{f}")),
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
        let path: PathBuf = store.join("objects").join(&hash[..2]).join(hash);
        (std::fs::metadata(&path).ok()?.len() == size as u64).then_some(())?;
        Some(MediaEntry {
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
        })
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
        hydrate(&root, &mut media, &store);
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
