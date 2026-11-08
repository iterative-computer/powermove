//! Worker-only resolution of project media to renderer paths. The cache is
//! content-addressed under one profile directory, so opening, saving or editing
//! a project reuses earlier bytes. Never writes a project or a source file.
use anyhow::Result;
use pm_engine_service::MediaCatalog;
use pm_kernel::{
    Snapshot,
    project_io::{MediaEntry, MediaSource, MediaTable},
};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
    sync::Mutex,
};

/// Objects kept beyond those the open project needs, oldest evicted first.
const CACHE_BUDGET: u64 = 8 << 30;
const OBJECTS: &str = "objects";
/// One writer at a time: a concurrent prune must not take another's objects.
static CACHE: Mutex<()> = Mutex::new(());

pub struct Resolution {
    pub catalog: MediaCatalog,
    /// Items left offline; one bad item never hides the rest of the project.
    pub failed: Vec<String>,
}

pub fn cache_root(profile: &Path) -> PathBuf {
    profile.join("media-cache")
}

/// The media cache's content-addressed objects (each named by its bytes).
pub fn objects_dir(profile: &Path) -> PathBuf {
    cache_root(profile).join(OBJECTS)
}

pub fn failure_note(count: usize) -> String {
    match count {
        1 => "1 media item is offline".into(),
        count => format!("{count} media items are offline"),
    }
}

fn extension(media_type: &str) -> &'static str {
    match media_type {
        "video/mp4" => "mp4",
        "video/quicktime" => "mov",
        "video/webm" => "webm",
        "image/png" => "png",
        "image/jpeg" => "jpg",
        "image/gif" => "gif",
        "image/tiff" => "tif",
        "image/heic" => "heic",
        "image/avif" => "avif",
        "video/x-matroska" => "mkv",
        "image/webp" => "webp",
        "image/svg+xml" => "svg",
        "audio/wav" | "audio/x-wav" => "wav",
        "audio/mpeg" => "mp3",
        "audio/ogg" => "ogg",
        "audio/flac" => "flac",
        "audio/aac" => "aac",
        "audio/mp4" => "m4a",
        "model/gltf-binary" => "glb",
        _ => "bin",
    }
}

/// Bytes identity: stable across saves, which move media to new file ranges.
pub(crate) fn content_identity(item: &MediaEntry) -> (Option<&str>, String, u64) {
    let hash = match &item.source {
        MediaSource::Embedded { sha256, .. } | MediaSource::File { sha256, .. } => {
            sha256.as_deref()
        }
        MediaSource::Memory(_) => None,
    };
    (hash, item.revision.to_string(), item.length())
}

fn object_key(item: &MediaEntry) -> String {
    let hash = match &item.source {
        MediaSource::Embedded {
            sha256: Some(h), ..
        }
        | MediaSource::File {
            sha256: Some(h), ..
        } => {
            return h.clone();
        }
        MediaSource::Embedded { file, offset, .. } => {
            Sha256::digest(format!("{}\0{offset}\0{}", file.hash, item.length()))
        }
        MediaSource::File { path, .. } => Sha256::digest(format!(
            "{}\0{}\0{}",
            path.display(),
            item.revision,
            item.length()
        )),
        MediaSource::Memory(bytes) => Sha256::digest(bytes),
    };
    format!("{hash:x}")
}

fn resolve(item: &MediaEntry, objects: &Path) -> Result<PathBuf> {
    if let MediaSource::File { path, length, .. } = &item.source
        && path.extension().is_some_and(|e| e != "media")
    {
        anyhow::ensure!(fs::metadata(path)?.len() == *length, "media source changed");
        return Ok(path.clone());
    }
    let extension = extension(&item.media_type.to_string());
    let name = format!("{}.{extension}", object_key(item));
    let object = objects.join(&name);
    if let Ok(found) = fs::metadata(&object)
        && found.is_file()
        && found.len() == item.length()
    {
        // Recency for eviction; a failure only makes it evicted sooner. A link
        // shares its inode with an immutable original, which stays untouched.
        use std::os::unix::fs::MetadataExt;
        if found.nlink() == 1
            && let Ok(file) = fs::File::options().append(true).open(&object)
        {
            let _ = file.set_modified(std::time::SystemTime::now());
        }
        return Ok(object);
    }
    // Retained originals have no extension. A link names them for the decoder
    // and costs no bytes; another volume falls through to a verified copy.
    if let MediaSource::File { path, length, .. } = &item.source
        && fs::metadata(path).is_ok_and(|m| m.len() == *length)
        && fs::hard_link(path, &object).is_ok()
    {
        return Ok(object);
    }
    let partial = objects.join(format!(".{name}.part"));
    item.copy_to(&partial)?;
    fs::rename(&partial, &object)?;
    Ok(object)
}

fn prune(objects: &Path, pinned: &BTreeSet<&Path>, budget: u64) -> Result<()> {
    let mut held = vec![];
    for entry in fs::read_dir(objects)? {
        let entry = entry?;
        let path = entry.path();
        let metadata = entry.metadata()?;
        if !metadata.is_file() || pinned.contains(path.as_path()) {
            continue;
        }
        held.push((metadata.modified()?, metadata.len(), path));
    }
    held.sort();
    let mut total: u64 = held.iter().map(|(_, length, _)| length).sum();
    for (_, length, path) in held {
        if total <= budget {
            break;
        }
        fs::remove_file(path)?;
        total -= length;
    }
    Ok(())
}

pub fn for_session(
    snapshot: &Snapshot,
    media: &MediaTable,
    path: Option<&Path>,
    cache: &Path,
) -> Result<Resolution> {
    for_session_within(snapshot, media, path, cache, CACHE_BUDGET)
}

fn for_session_within(
    snapshot: &Snapshot,
    media: &MediaTable,
    path: Option<&Path>,
    cache: &Path,
    budget: u64,
) -> Result<Resolution> {
    let _writer = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    let objects = cache.join(OBJECTS);
    fs::create_dir_all(&objects)?;
    let mut catalog = MediaCatalog::default();
    let mut failed = vec![];
    let root = snapshot.root();
    let versions = root.and_then(|r| r.get("nativeMediaVersions"));
    let originals = versions.and_then(|m| m.get("originals"));
    let owners = versions.and_then(|m| m.get("owners"));
    let assets = root.and_then(|r| r.get("assets"));
    for item in media.iter() {
        let id = item.id.to_string();
        // Immutable versions stay in PMV/history but are not renderer aliases.
        if originals.and_then(|m| m.get(&id)).is_some() && assets.and_then(|a| a.get(&id)).is_none()
        {
            continue;
        }
        let resolved = match resolve(item, &objects) {
            Ok(resolved) => resolved,
            Err(_) => {
                failed.push(id);
                continue;
            }
        };
        if pm_builtins::delivery::embedded_proxy(snapshot, &id) {
            catalog.proxies.insert(id, resolved);
        } else {
            catalog.originals.insert(id, resolved);
        }
    }
    // Linked originals already have public document descriptors. No proxy fallback.
    if let Some(root) = snapshot.document.to_value()
        && let Some(assets) = root.get("assets").and_then(|v| v.as_object())
    {
        for (id, value) in assets {
            let id = id.to_string();
            // Ownership selects verified bytes; old path metadata is not
            // authority to substitute a different original on failure.
            if value.get("originalVersion").is_some()
                || owners.and_then(|m| m.get(&id)).is_some()
                || catalog.originals.contains_key(&id)
            {
                continue;
            }
            if let Some(p) = value
                .get("sourcePath")
                .or_else(|| value.get("path"))
                .and_then(|v| v.as_str())
            {
                let p = Path::new(p);
                let p = if p.is_absolute() {
                    p.to_path_buf()
                } else {
                    path.and_then(Path::parent)
                        .unwrap_or(Path::new("."))
                        .join(p)
                };
                if p.is_file() {
                    catalog.originals.insert(id, p);
                }
            }
        }
    }
    let pinned = catalog
        .originals
        .values()
        .chain(catalog.proxies.values())
        .map(PathBuf::as_path)
        .collect();
    prune(&objects, &pinned, budget)?;
    Ok(Resolution { catalog, failed })
}

/// Remove what earlier builds left: one full copy of every project's media per
/// open, plus partial objects from an interrupted run.
pub fn sweep_cache(cache: &Path) -> Result<()> {
    let _writer = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    let entries = match fs::read_dir(cache) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e.into()),
    };
    for entry in entries {
        let entry = entry?;
        if entry.file_name() != OBJECTS && entry.file_type()?.is_dir() {
            fs::remove_dir_all(entry.path())?;
        }
    }
    let objects = cache.join(OBJECTS);
    if objects.is_dir() {
        for entry in fs::read_dir(objects)? {
            let entry = entry?;
            if entry.file_name().to_string_lossy().ends_with(".part") {
                fs::remove_file(entry.path())?;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use pm_kernel::project_io::MediaEntry;
    use std::sync::Arc;

    fn snapshot() -> Snapshot {
        let project = pm_model::json::parse(
            r##"{"id":"p","name":"Synthetic","compId":"c","w":64,"h":36,"fps":30,"dur":1,"bg":"#000000","assets":{"a":{"name":"a.png","kind":"image"},"b":{"name":"b.png","kind":"image"}},"comps":{},"layers":[]}"##,
        )
        .unwrap();
        pm_kernel::DocumentStore::new_untitled(project).snapshot()
    }
    fn table(items: &[(&str, &[u8])]) -> MediaTable {
        let mut table = MediaTable::default();
        for (id, bytes) in items {
            table.insert(MediaEntry {
                id: (*id).into(),
                media_type: "image/png".into(),
                revision: "1".into(),
                source: MediaSource::Memory(Arc::from(*bytes)),
            });
        }
        table
    }
    fn objects(cache: &Path) -> Vec<PathBuf> {
        let mut found: Vec<_> = fs::read_dir(cache.join(OBJECTS))
            .unwrap()
            .map(|e| e.unwrap().path())
            .collect();
        found.sort();
        found
    }

    #[test]
    fn reopening_reuses_one_object_per_content_instead_of_copying_the_project() {
        let directory = tempfile::tempdir().unwrap();
        let cache = directory.path().join("media-cache");
        let media = table(&[("a", b"same"), ("b", b"same")]);
        let first = for_session(&snapshot(), &media, None, &cache).unwrap();
        assert!(first.failed.is_empty());
        let held = objects(&cache);
        assert_eq!(held.len(), 1, "identical bytes share one object");
        use std::os::unix::fs::MetadataExt;
        let identity = fs::metadata(&held[0]).unwrap().ino();
        for _ in 0..20 {
            let again = for_session(&snapshot(), &media, None, &cache).unwrap();
            assert_eq!(again.catalog.originals, first.catalog.originals);
        }
        assert_eq!(objects(&cache), held, "no open adds a copy");
        assert_eq!(fs::metadata(&held[0]).unwrap().ino(), identity);
    }

    #[test]
    fn a_linked_file_is_read_in_place_and_one_bad_item_stays_offline_alone() {
        let directory = tempfile::tempdir().unwrap();
        let cache = directory.path().join("media-cache");
        let source = directory.path().join("clip.png");
        fs::write(&source, b"linked").unwrap();
        let mut media = table(&[("a", b"embedded")]);
        media.insert(MediaEntry {
            id: "b".into(),
            media_type: "image/png".into(),
            revision: "1".into(),
            source: MediaSource::File {
                path: source.clone(),
                length: 6,
                sha256: None,
            },
        });
        let resolved = for_session(&snapshot(), &media, None, &cache).unwrap();
        assert_eq!(resolved.catalog.originals["b"], source);
        assert_eq!(
            objects(&cache).len(),
            1,
            "only embedded bytes are extracted"
        );
        fs::write(&source, b"changed!").unwrap();
        let resolved = for_session(&snapshot(), &media, None, &cache).unwrap();
        assert_eq!(resolved.failed, ["b"]);
        assert!(resolved.catalog.originals.contains_key("a"));
    }

    #[test]
    fn a_retained_original_is_linked_not_copied() {
        let directory = tempfile::tempdir().unwrap();
        let cache = directory.path().join("media-cache");
        let retained = directory.path().join("0".repeat(64));
        fs::write(&retained, b"retained").unwrap();
        let mut media = MediaTable::default();
        media.insert(MediaEntry {
            id: "a".into(),
            media_type: "image/png".into(),
            revision: "1".into(),
            source: MediaSource::File {
                path: retained.clone(),
                length: 8,
                sha256: Some("0".repeat(64)),
            },
        });
        let resolved = for_session(&snapshot(), &media, None, &cache).unwrap();
        use std::os::unix::fs::MetadataExt;
        let object = &resolved.catalog.originals["a"];
        assert_eq!(object.extension().unwrap(), "png");
        assert_eq!(
            fs::metadata(object).unwrap().ino(),
            fs::metadata(&retained).unwrap().ino()
        );
    }

    #[test]
    fn eviction_keeps_the_open_project_and_removes_the_least_recently_used() {
        let directory = tempfile::tempdir().unwrap();
        let cache = directory.path().join("media-cache");
        let old = table(&[("a", b"old project bytes")]);
        for_session(&snapshot(), &old, None, &cache).unwrap();
        assert_eq!(objects(&cache).len(), 1);
        let current = table(&[("a", b"current project")]);
        let resolved = for_session_within(&snapshot(), &current, None, &cache, 0).unwrap();
        assert_eq!(objects(&cache).len(), 1, "the unused object is evicted");
        assert_eq!(objects(&cache)[0], resolved.catalog.originals["a"]);
    }

    #[test]
    fn startup_sweep_removes_per_open_copies_and_partials_but_keeps_objects() {
        let directory = tempfile::tempdir().unwrap();
        let cache = directory.path().join("media-cache");
        let media = table(&[("a", b"keep")]);
        let resolved = for_session(&snapshot(), &media, None, &cache).unwrap();
        for legacy in ["app-1", "resolved-2"] {
            fs::create_dir_all(cache.join(legacy)).unwrap();
            fs::write(cache.join(legacy).join("asset-0.png"), b"copy").unwrap();
        }
        fs::write(cache.join(OBJECTS).join(".x.png.part"), b"partial").unwrap();
        sweep_cache(&cache).unwrap();
        assert_eq!(objects(&cache), [resolved.catalog.originals["a"].clone()]);
        assert_eq!(fs::read_dir(&cache).unwrap().count(), 1);
        sweep_cache(&directory.path().join("absent")).unwrap();
    }
}
