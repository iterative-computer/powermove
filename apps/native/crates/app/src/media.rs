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
    /// Linked originals still in the cloud: offline until downloaded.
    pub cloud: Vec<crate::cloud_media::Cloud>,
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

/// The cache object's extension. Decoders read better with the right one, so a
/// generic type (old projects store `application/octet-stream`) falls back
/// to the asset's `format`, then its file name.
fn extension(media_type: &str, asset: Option<&pm_model::Node>) -> String {
    let known = mime_extension(media_type);
    if known != "bin" {
        return known.into();
    }
    let field = |key| asset?.get(key).and_then(crate::node_string);
    let name_extension = field("name")
        .and_then(|name| Path::new(name).extension()?.to_str())
        .map(str::to_ascii_lowercase);
    field("format")
        .map(str::to_ascii_lowercase)
        .into_iter()
        .chain(name_extension)
        .find(|e| KNOWN_EXTENSIONS.contains(&e.as_str()))
        .unwrap_or_else(|| "bin".into())
}

const KNOWN_EXTENSIONS: [&str; 22] = [
    "mp4", "mov", "m4v", "webm", "mkv", "png", "jpg", "jpeg", "gif", "tif", "tiff", "heic", "avif",
    "webp", "svg", "wav", "mp3", "ogg", "flac", "aac", "m4a", "glb",
];

fn mime_extension(media_type: &str) -> &'static str {
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
        "model/gltf+json" => "gltf",
        "model/obj" => "obj",
        "model/ply" => "ply",
        "model/splat" => "splat",
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

fn resolve(item: &MediaEntry, asset: Option<&pm_model::Node>, objects: &Path) -> Result<PathBuf> {
    if let MediaSource::File { path, length, .. } = &item.source
        && path.extension().is_some_and(|e| e != "media")
    {
        anyhow::ensure!(fs::metadata(path)?.len() == *length, "media source changed");
        return Ok(path.clone());
    }
    let extension = extension(&item.media_type.to_string(), asset);
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
    for_session_within(
        snapshot,
        media,
        path,
        cache,
        CACHE_BUDGET,
        &*crate::cloud_media::files(),
    )
}

fn for_session_within(
    snapshot: &Snapshot,
    media: &MediaTable,
    path: Option<&Path>,
    cache: &Path,
    budget: u64,
    files: &dyn crate::cloud_media::CloudFiles,
) -> Result<Resolution> {
    let _writer = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    let objects = cache.join(OBJECTS);
    fs::create_dir_all(&objects)?;
    let mut catalog = MediaCatalog::default();
    let mut failed = vec![];
    let mut cloud = vec![];
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
        // A linked clip still in the cloud waits for the person (cloud_media).
        if let MediaSource::File { path, .. } = &item.source
            && path.extension().is_some_and(|e| e != "media")
            && let Some(provider) = files.provider(path)
        {
            cloud.push(crate::cloud_media::Cloud {
                id,
                path: path.clone(),
                provider,
            });
            continue;
        }
        let resolved = match resolve(item, assets.and_then(|a| a.get(&id)), &objects) {
            Ok(resolved) => resolved,
            Err(_) => {
                failed.push(id);
                continue;
            }
        };
        // A sequence read in place whose folder moved is offline, as a moved
        // file is: the person relinks it by choosing one of its files.
        if pm_media::image::sequence::Manifest::read(&resolved)
            .ok()
            .flatten()
            .is_some_and(|sequence| !sequence.present())
        {
            continue;
        }
        // Proxy bytes never stand in for an original (plan §6), whether the
        // asset says so or the first launch found the original gone.
        if pm_builtins::delivery::embedded_proxy(snapshot, &id) || media.is_proxy_only(&id) {
            catalog.proxies.insert(id, resolved);
        } else {
            catalog.originals.insert(id, resolved);
        }
    }
    // Linked originals already have public document descriptors. No proxy fallback.
    if let Some(pm_model::NodeKind::Object(assets)) = assets.map(pm_model::Node::kind) {
        for (id, value) in assets.iter() {
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
                .and_then(crate::node_string)
            {
                let p = Path::new(p);
                let p = if p.is_absolute() {
                    p.to_path_buf()
                } else {
                    path.and_then(Path::parent)
                        .unwrap_or(Path::new("."))
                        .join(p)
                };
                if !p.is_file() {
                    continue;
                }
                // Reading a cloud placeholder would download it unasked.
                match files.provider(&p) {
                    Some(provider) => cloud.push(crate::cloud_media::Cloud {
                        id,
                        path: p,
                        provider,
                    }),
                    None => {
                        if value.get("kind").and_then(crate::node_string) == Some("model") {
                            match pm_scene3d::import::path::prepare(&p) {
                                Ok(model) => {
                                    use sha2::{Digest, Sha256};
                                    let hash = format!("{:x}", Sha256::digest(&model.bytes));
                                    let ext = Path::new(&model.name)
                                        .extension()
                                        .unwrap_or_default()
                                        .to_string_lossy();
                                    let cached = objects.join(format!("{hash}.{ext}"));
                                    if !cached.exists() {
                                        fs::write(&cached, &model.bytes)?;
                                    }
                                    catalog.originals.insert(id, cached);
                                }
                                Err(_) => {
                                    continue;
                                }
                            }
                        } else {
                            catalog.originals.insert(id, p);
                        }
                    }
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
    Ok(Resolution {
        catalog,
        failed,
        cloud,
    })
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
    fn proxy_only_bytes_are_a_proxy_and_never_a_normal_export_original() {
        let directory = tempfile::tempdir().unwrap();
        let cache = directory.path().join("media-cache");
        let mut media = table(&[("b", b"original")]);
        media.insert_proxy_only(MediaEntry {
            id: "a".into(),
            media_type: "image/png".into(),
            revision: "1".into(),
            source: MediaSource::Memory(Arc::from(&b"proxy"[..])),
        });
        let catalog = for_session(&snapshot(), &media, None, &cache)
            .unwrap()
            .catalog;
        assert!(catalog.proxies.contains_key("a"));
        assert!(!catalog.originals.contains_key("a"));
        assert!(!catalog.export_paths(false).paths.contains_key("a"));
        assert!(catalog.export_paths(true).paths.contains_key("a"));
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

    /// A sequence read in place whose folder was moved or renamed is
    /// offline, as a moved file is; one missing number leaves it online
    /// (that frame shows Media Offline).
    #[test]
    fn a_sequence_whose_folder_moved_is_offline_until_relinked() {
        let directory = tempfile::tempdir().unwrap();
        let cache = directory.path().join("media-cache");
        let frames = directory.path().join("renders");
        fs::create_dir(&frames).unwrap();
        for n in 1..=3 {
            fs::write(frames.join(format!("shot.{n:04}.png")), b"frame").unwrap();
        }
        let manifest = pm_media::image::sequence::Manifest {
            powermove_image_sequence: 1,
            dir: frames.clone(),
            prefix: "shot.".into(),
            suffix: ".png".into(),
            padding: 4,
            first: 1,
            last: 3,
            rate: [24, 1],
        };
        let bytes = manifest.to_bytes();
        let media = table(&[("a", &bytes)]);
        let online = |media: &MediaTable| {
            for_session(&snapshot(), media, None, &cache)
                .unwrap()
                .catalog
                .originals
                .contains_key("a")
        };
        assert!(online(&media));
        fs::remove_file(frames.join("shot.0002.png")).unwrap();
        assert!(online(&media), "one missing number is that frame's card");
        fs::rename(&frames, directory.path().join("renders v2")).unwrap();
        assert!(!online(&media), "a moved folder is offline");
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
        let resolved = for_session_within(
            &snapshot(),
            &current,
            None,
            &cache,
            0,
            &crate::cloud_media::Fixture::new(&[], &[], 0),
        )
        .unwrap();
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

/// Only a whole, existing media/document path claims a textual paste.
pub fn agent_pasted_path(text: &str) -> Option<std::path::PathBuf> {
    let text = text.trim();
    let file_url = text
        .get(..7)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("file://"));
    let path = if file_url {
        url::Url::parse(text).ok()?.to_file_path().ok()?
    } else {
        std::path::PathBuf::from(text)
    };
    if !path.is_absolute() || !path.is_file() {
        return None;
    }
    if file_url {
        return Some(path);
    }
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    (matches!(
        ext.as_str(),
        "png"
            | "jpg"
            | "jpeg"
            | "heic"
            | "webp"
            | "gif"
            | "mov"
            | "mp4"
            | "webm"
            | "m4v"
            | "wav"
            | "mp3"
            | "m4a"
            | "aiff"
            | "flac"
            | "pdf"
            | "txt"
            | "md"
            | "rtf"
            | "doc"
            | "docx"
            | "csv"
            | "xls"
            | "xlsx"
            | "json"
            | "js"
            | "ts"
            | "tsx"
            | "html"
            | "css"
            | "py"
            | "rs"
    ))
    .then_some(path)
}

pub fn agent_pasted_records(
    media: crate::platform::PastedMedia,
) -> Result<Vec<serde_json::Value>, String> {
    match media {
        crate::platform::PastedMedia::Files(paths) => attachment_records(&paths),
        crate::platform::PastedMedia::Image { data, png } => {
            let (hold, path) = crate::media_import::stage_pasted_image(data, png)
                .ok_or("The pasted image could not be read")?;
            let result = attachment_records(&[path]);
            drop(hold);
            result
        }
    }
}

/// Quick Look's file, held in an app-lifetime temporary cache (never the user's project).
pub fn attachment_preview_path(record: &serde_json::Value) -> Result<std::path::PathBuf, String> {
    use base64::Engine;
    let name = record["name"].as_str().unwrap_or("Attachment");
    let read_error = || format!("“{name}” could not be opened.");
    let bytes = if let Some(data) = record["dataUrl"].as_str() {
        base64::engine::general_purpose::STANDARD
            .decode(data.split_once(',').ok_or_else(read_error)?.1)
            .map_err(|_| read_error())?
    } else if let Some(data) = record["dataBase64"].as_str() {
        base64::engine::general_purpose::STANDARD
            .decode(data)
            .map_err(|_| read_error())?
    } else if let Some(text) = record["content"].as_str() {
        text.as_bytes().to_vec()
    } else if let Some(path) = record["sourcePath"]
        .as_str()
        .map(std::path::PathBuf::from)
        .filter(|p| p.is_absolute() && p.is_file())
    {
        return Ok(path);
    } else {
        return Err(read_error());
    };
    static CACHE: std::sync::OnceLock<std::sync::Mutex<Option<tempfile::TempDir>>> =
        std::sync::OnceLock::new();
    let mut cache = CACHE
        .get_or_init(Default::default)
        .lock()
        .map_err(|_| read_error())?;
    if cache.is_none() {
        *cache = Some(tempfile::tempdir().map_err(|_| read_error())?);
    }
    let ext = std::path::Path::new(name)
        .extension()
        .and_then(|e| e.to_str())
        .filter(|e| e.chars().all(|c| c.is_ascii_alphanumeric()))
        .unwrap_or("bin");
    let path =
        cache
            .as_ref()
            .unwrap()
            .path()
            .join(format!("{}.{}", blake3::hash(&bytes).to_hex(), ext));
    if !path.is_file() {
        std::fs::write(&path, bytes).map_err(|_| read_error())?;
    }
    Ok(path)
}

/// The Agent panel's attachment records for chosen files, as main keeps them:
/// images as data URLs, text as content, anything else as base64. Files over
/// 20 MB are refused by name.
pub fn attachment_records(paths: &[std::path::PathBuf]) -> Result<Vec<serde_json::Value>, String> {
    use base64::Engine;
    const LIMIT: u64 = 20 * 1024 * 1024;
    if paths.len() > 6 {
        return Err("A message can carry six attachments.".into());
    }
    let mut records = vec![];
    for path in paths {
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "Attachment".into());
        let size = std::fs::metadata(path)
            .map_err(|e| format!("“{name}” can’t be read: {e}"))?
            .len();
        if size > LIMIT {
            return Err(format!(
                "“{name}” is larger than 20 MB, too large to attach."
            ));
        }
        let bytes = std::fs::read(path).map_err(|e| format!("“{name}” can’t be read: {e}"))?;
        let extension = path
            .extension()
            .map(|e| e.to_string_lossy().to_ascii_lowercase())
            .unwrap_or_default();
        let image = match extension.as_str() {
            "png" => Some("image/png"),
            "jpg" | "jpeg" => Some("image/jpeg"),
            "gif" => Some("image/gif"),
            "webp" => Some("image/webp"),
            "heic" => Some("image/heic"),
            _ => None,
        };
        let mut record =
            serde_json::json!({"id":pm_kernel::new_id("attachment"),"name":name,"size":size});
        let encoded = || base64::engine::general_purpose::STANDARD.encode(&bytes);
        match (image, std::str::from_utf8(&bytes)) {
            (Some(mime), _) => {
                record["type"] = mime.into();
                record["dataUrl"] = format!("data:{mime};base64,{}", encoded()).into();
            }
            (None, Ok(text)) => {
                record["type"] = "text/plain".into();
                record["content"] = text.into();
            }
            (None, Err(_)) => {
                record["type"] = "application/octet-stream".into();
                record["dataBase64"] = encoded().into();
            }
        }
        let kind = if image.is_some() {
            "image"
        } else if matches!(extension.as_str(), "mov" | "mp4" | "webm" | "m4v") {
            "video"
        } else if matches!(extension.as_str(), "wav" | "mp3" | "m4a" | "aiff" | "flac") {
            "audio"
        } else {
            "document"
        };
        let mime = match extension.as_str() {
            "mov" => Some("video/quicktime"),
            "mp4" | "m4v" => Some("video/mp4"),
            "webm" => Some("video/webm"),
            "mp3" => Some("audio/mpeg"),
            "wav" => Some("audio/wav"),
            "m4a" => Some("audio/mp4"),
            "aiff" => Some("audio/aiff"),
            "flac" => Some("audio/flac"),
            "pdf" => Some("application/pdf"),
            _ => None,
        };
        if let Some(mime) = mime {
            record["type"] = mime.into();
        }
        if kind == "audio" || kind == "video" {
            if let Some(us) = pm_media::probe::probe(path)
                .ok()
                .and_then(|p| p.probe.duration_us)
            {
                let seconds = us.max(0) / 1_000_000;
                record["duration"] = format!("{}:{:02}", seconds / 60, seconds % 60).into();
            }
        }
        if kind == "image" || kind == "video" {
            if let Some((key, file)) = crate::media_thumbs::make_attachment(
                path,
                kind == "video",
                crate::pro_media::ffmpeg()
                    .as_ref()
                    .map(|(p, _)| p.as_path()),
            ) {
                crate::media_thumbs::register(&key, file);
                record["thumbnail"] = key.into();
            }
        }
        records.push(record);
    }
    Ok(records)
}

#[cfg(test)]
mod attachment_tests {
    use super::attachment_records;
    #[::core::prelude::v1::test]
    fn chosen_files_become_main_shaped_attachment_records() {
        let dir = tempfile::tempdir().unwrap();
        let (png, text, data) = (
            dir.path().join("Frame.PNG"),
            dir.path().join("notes.md"),
            dir.path().join("blob.bin"),
        );
        std::fs::write(&png, [0x89, b'P', b'N', b'G']).unwrap();
        std::fs::write(&text, "hello").unwrap();
        std::fs::write(&data, [0xff, 0xfe, 0x00]).unwrap();
        let records = attachment_records(&[png, text, data]).unwrap();
        assert_eq!(records[0]["type"], "image/png");
        assert!(
            records[0]["dataUrl"]
                .as_str()
                .unwrap()
                .starts_with("data:image/png;base64,iVBORw")
        );
        assert_eq!(records[1]["content"], "hello");
        assert_eq!(records[2]["dataBase64"], "//4A");
        assert!(
            records
                .iter()
                .all(|r| r["id"].is_string() && r["size"].is_u64())
        );
    }

    #[test]
    fn attachment_previews_keep_original_bytes_and_report_missing_payloads() {
        let record = serde_json::json!({"id":"preview","name":"Notes.txt","content":"hello"});
        let file = super::attachment_preview_path(&record).unwrap();
        assert_eq!(std::fs::read(file).unwrap(), b"hello");
        let missing = serde_json::json!({"name":"Missing.png"});
        assert!(
            super::attachment_preview_path(&missing)
                .unwrap_err()
                .contains("Missing.png")
        );
        assert!(
            super::attachment_preview_path(
                &serde_json::json!({"name":"Bad.png","dataBase64":"%%%"})
            )
            .is_err()
        );
    }
    #[test]
    fn agent_paste_paths_claim_only_whole_existing_supported_files() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("with space.PNG");
        std::fs::write(&path, b"image").unwrap();
        assert_eq!(
            super::agent_pasted_path(path.to_str().unwrap()),
            Some(path.clone())
        );
        assert_eq!(
            super::agent_pasted_path(url::Url::from_file_path(&path).unwrap().as_str()),
            Some(path.clone())
        );
        assert!(
            super::agent_pasted_path(dir.path().join("missing.png").to_str().unwrap()).is_none()
        );
        assert!(super::agent_pasted_path(&format!("look at {}", path.display())).is_none());
        let binary = dir.path().join("file.bin");
        std::fs::write(&binary, [0, 255]).unwrap();
        assert!(super::agent_pasted_path(binary.to_str().unwrap()).is_none());
        assert_eq!(
            super::agent_pasted_path(url::Url::from_file_path(&binary).unwrap().as_str()),
            Some(binary)
        );
        assert!(super::agent_pasted_path(dir.path().to_str().unwrap()).is_none());
    }
    #[test]
    fn agent_clipboard_image_and_file_drop_keep_bytes_and_make_a_thumbnail() {
        let png = crate::region_attach::encode_png(2, 2, &[255; 16]).unwrap();
        let pasted = super::agent_pasted_records(crate::platform::PastedMedia::Image {
            data: png.clone(),
            png: true,
        })
        .unwrap();
        assert_eq!(pasted[0]["type"], "image/png");
        let thumbnail = pasted[0]["thumbnail"]
            .as_str()
            .and_then(crate::media_thumbs::resolve)
            .unwrap();
        let (width, height, _) =
            crate::region_attach::decode_png(&std::fs::read(thumbnail).unwrap()).unwrap();
        assert_eq!((width, height), (112, 112));
        assert!(
            pasted[0]["thumbnail"]
                .as_str()
                .and_then(crate::media_thumbs::resolve)
                .is_some()
        );
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("drop.png");
        std::fs::write(&path, png).unwrap();
        let dropped =
            super::agent_pasted_records(crate::platform::PastedMedia::Files(vec![path])).unwrap();
        assert_eq!(dropped[0]["dataUrl"], pasted[0]["dataUrl"]);
    }

    #[test]
    fn a_cloud_only_original_stays_offline_and_named_until_downloaded() {
        let dir = tempfile::tempdir().unwrap();
        let (local, remote) = (dir.path().join("a.png"), dir.path().join("b.png"));
        std::fs::write(&local, b"a").unwrap();
        std::fs::write(&remote, b"b").unwrap();
        let project = pm_model::json::parse(&format!(
            r##"{{"id":"p","name":"Synthetic","compId":"c","w":64,"h":36,"fps":30,"dur":1,"bg":"#000000","assets":{{"a":{{"name":"a.png","kind":"image","sourcePath":{a:?}}},"b":{{"name":"b.png","kind":"image","sourcePath":{b:?}}}}},"comps":{{}},"layers":[]}}"##,
            a = local.to_string_lossy(),
            b = remote.to_string_lossy(),
        ))
        .unwrap();
        let snapshot = pm_kernel::DocumentStore::new_untitled(project).snapshot();
        let cache = dir.path().join("cache");
        let cloud = crate::cloud_media::Fixture::new(std::slice::from_ref(&remote), &[], 0);
        let resolve = |files: &dyn crate::cloud_media::CloudFiles| {
            crate::media::for_session_within(
                &snapshot,
                &pm_kernel::project_io::MediaTable::default(),
                None,
                &cache,
                0,
                files,
            )
            .unwrap()
        };
        let first = resolve(&cloud);
        assert!(first.catalog.originals.contains_key("a"));
        assert!(!first.catalog.originals.contains_key("b"));
        assert_eq!(first.cloud.len(), 1);
        assert_eq!(
            (first.cloud[0].id.as_str(), first.cloud[0].provider.as_str()),
            ("b", "iCloud")
        );
        use crate::cloud_media::CloudFiles;
        cloud.download(&remote, &|_, _| {}).unwrap();
        let after = resolve(&cloud);
        assert!(after.catalog.originals.contains_key("b"));
        assert!(after.cloud.is_empty());
    }
}
