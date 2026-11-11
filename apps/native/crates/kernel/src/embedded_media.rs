//! Media a migrated library copy reads from the project file the original app
//! saved it to (`embeddedMedia` in its metadata). The original's `savedHash`
//! fingerprints the project JSON, not the file, so the file is matched per
//! asset: an item is used only for an asset id the copy holds whose media
//! identity (`storageKey` or id, `fingerprint`, kind, proxy) equals the file's
//! own record of that asset, and whose bytes the file's checksum proves (or,
//! in a format without one, the asset's sampled fingerprint). Bytes the
//! library's Media Store holds win, so a save never copies them again.
use crate::{
    History,
    library::LibraryProject,
    project_io::{self, MediaEntry, MediaSource, OpenOptions, ProjectFile},
};
use pm_model::{Node, NodeKind, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeSet, HashMap},
    io::{Read, Seek, SeekFrom},
    path::Path,
};

/// What makes two asset records name the same bytes: the original app read
/// an asset's media by its storage key, and a proxy replaces them.
#[derive(Debug, PartialEq, Eq)]
struct Identity {
    key: String,
    fingerprint: Option<String>,
    kind: Option<String>,
    proxy: Option<String>,
}
fn identity(id: &str, asset: &Value) -> Option<Identity> {
    let text = |k| asset.get(k).and_then(Value::as_str).map(str::to_owned);
    asset.as_object()?;
    Some(Identity {
        key: text("storageKey").unwrap_or_else(|| id.to_owned()),
        fingerprint: text("fingerprint"),
        kind: text("kind"),
        proxy: asset
            .get("playbackProxy")
            .filter(|p| p.is_truthy())
            .map(|_| {
                asset
                    .get("playbackProxyVersion")
                    .map(pm_model::json::to_canonical_string)
                    .unwrap_or_default()
            }),
    })
}

type Identities = HashMap<String, Vec<Identity>>;
fn history_identities(history: &History) -> Identities {
    let mut out = Identities::new();
    crate::library_media::history_assets(history, |id, asset| {
        if let Some(i) = identity(id, asset) {
            out.entry(id.to_owned()).or_default().push(i);
        }
    });
    out
}

/// The asset records `id` may have: its current record, else the ones only
/// its undo history holds.
fn candidates(
    current: Option<Identity>,
    history: &mut dyn FnMut() -> Vec<Identity>,
) -> Vec<Identity> {
    match current {
        Some(i) => vec![i],
        None => history(),
    }
}

/// The media of the file at `path` this library copy may use, as an untitled
/// file's table, or `None` when it lends none (or is unreadable, or fails its
/// checksums).
/// **Blocking**: reads and hashes the file.
pub(crate) fn for_copy(
    path: &Path,
    lib: &LibraryProject,
    store: Option<&Path>,
) -> Option<ProjectFile> {
    // Every PMV4 media checksum is verified in the open's single hashing pass.
    let opened = project_io::open_project(path, OpenOptions { verify_media: true }).ok()?;
    let file_root = opened.document.root();
    let mut file_history = None;
    let mut lib_history = None;
    let lib_assets = lib.project.get("assets");
    let mut file = opened.file;
    let ids: Vec<String> = file.media.iter().map(|e| e.id.to_string()).collect();
    for id in ids {
        let entry = file.media.get(&id).unwrap();
        let theirs = candidates(
            file_root
                .and_then(|r| r.get("assets"))
                .and_then(|a| a.get(&id))
                .filter(|a| matches!(a.kind(), NodeKind::Object(_)))
                .map(Node::to_value)
                .and_then(|a| identity(&id, &a)),
            &mut || {
                file_history
                    .get_or_insert_with(|| {
                        opened
                            .history
                            .as_ref()
                            .map(history_identities)
                            .unwrap_or_default()
                    })
                    .remove(&id)
                    .unwrap_or_default()
            },
        );
        let ours = candidates(
            lib_assets
                .and_then(|a| a.get(&id))
                .and_then(|a| identity(&id, a)),
            &mut || {
                lib_history
                    .get_or_insert_with(|| {
                        let comp = lib.project.get("compId").and_then(Value::as_str);
                        lib.history
                            .as_ref()
                            .and_then(|h| History::import(h, comp).ok())
                            .map(|h| history_identities(&h))
                            .unwrap_or_default()
                    })
                    .remove(&id)
                    .unwrap_or_default()
            },
        );
        let same = ours.iter().find(|o| theirs.contains(o));
        let keep = same.is_some_and(|asset| {
            proven(entry, asset)
                && !store.is_some_and(|store| {
                    crate::library_media::holds(
                        &id,
                        Some(&asset.key),
                        asset.fingerprint.as_deref(),
                        store,
                    )
                })
        });
        if !keep {
            file.media.remove(&id);
        }
    }
    // Only the media: the copy is a library project, not this file (its
    // name, version and envelope stay the original's).
    (!file.media.is_empty()).then(|| ProjectFile {
        media: file.media,
        ..ProjectFile::untitled()
    })
}

/// The asset ids the library project `project` in `profile` reads from its
/// original's file (`embeddedMedia`), as an open would.
pub fn lent(profile: &Path, project: &str) -> BTreeSet<String> {
    let Some(p) = crate::library::LibraryStore::open(profile)
        .load(project)
        .ok()
        .flatten()
    else {
        return BTreeSet::new();
    };
    let meta = p.session.as_ref();
    let Some(path) = meta
        .filter(|m| m.get("file").is_none())
        .and_then(|m| m.get("embeddedMedia"))
        .and_then(|m| m.get("path"))
        .and_then(Value::as_str)
    else {
        return BTreeSet::new();
    };
    let store = profile.join(crate::library_media::DIR);
    for_copy(Path::new(path), &p, Some(&store))
        .map(|f| f.media.iter().map(|e| e.id.to_string()).collect())
        .unwrap_or_default()
}

/// Whether the item's bytes are proven: by the file's checksum (verified at
/// open and again on every read), or without one by the asset's fingerprint.
fn proven(entry: &MediaEntry, asset: &Identity) -> bool {
    let MediaSource::Embedded { sha256, .. } = &entry.source else {
        return false;
    };
    sha256.is_some()
        || asset
            .fingerprint
            .as_deref()
            .is_some_and(|f| sampled_fingerprint(entry).as_deref() == Some(f))
}

/// The original app's `v2` media fingerprint of an embedded item: 64 KiB
/// start/middle/end samples behind `powermove-media-v2\n<size>\n`.
fn sampled_fingerprint(entry: &MediaEntry) -> Option<String> {
    let MediaSource::Embedded {
        file,
        offset,
        length,
        ..
    } = &entry.source
    else {
        return None;
    };
    let size = *length;
    let sample = size.min(64 * 1024);
    let mut input = std::fs::File::open(&file.path).ok()?;
    let mut hash = Sha256::new();
    hash.update(format!("powermove-media-v2\n{size}\n").as_bytes());
    let mut buffer = vec![0; sample as usize];
    for at in BTreeSet::from([0, (size - sample) / 2, size - sample]) {
        input.seek(SeekFrom::Start(offset + at)).ok()?;
        input.read_exact(&mut buffer).ok()?;
        hash.update(&buffer);
    }
    file.check(false).ok()?;
    Some(format!("v2:{size}:{:x}", hash.finalize()))
}
