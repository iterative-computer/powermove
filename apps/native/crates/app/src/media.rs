use anyhow::Result;
use pm_engine_service::MediaCatalog;
use pm_kernel::{Snapshot, project_io::MediaTable};
use std::{fs, path::Path};

/// Worker-only: portable media is streamed to a private cache, never to the source.
pub fn for_session(
    snapshot: &Snapshot,
    media: &MediaTable,
    path: Option<&Path>,
    cache: &Path,
) -> Result<MediaCatalog> {
    fs::create_dir_all(cache)?;
    let mut catalog = MediaCatalog::default();
    let root = snapshot.root();
    for (index, item) in media.iter().enumerate() {
        // Immutable versions remain in PMV/history, but are not renderer asset
        // aliases. A corrupt retired version must not prevent another selected
        // original from being displayed.
        if let Some(id) = item.id.as_str()
            && root.and_then(|r| r.get("nativeMediaVersions"))
                .and_then(|m| m.get("originals")).and_then(|m| m.get(id)).is_some()
            && root.and_then(|r| r.get("assets")).and_then(|a| a.get(id)).is_none()
        { continue; }
        let extension = match item.media_type.to_string().as_str() {
            "video/mp4" => "mp4",
            "video/quicktime" => "mov",
            "image/png" => "png",
            "image/jpeg" => "jpg",
            "image/svg+xml" => "svg",
            "audio/wav" => "wav",
            "model/gltf-binary" => "glb",
            _ => "bin",
        };
        let destination = cache.join(format!("asset-{index}.{extension}"));
        item.copy_to(&destination)?;
        let resolved = destination;
        if pm_builtins::delivery::embedded_proxy(snapshot, &item.id.to_string()) {
            catalog.proxies.insert(item.id.to_string(), resolved);
        } else {
            catalog.originals.insert(item.id.to_string(), resolved);
        }
    }
    // Linked originals already have public document descriptors. No proxy fallback.
    if let Some(root) = snapshot.document.to_value()
        && let Some(assets) = root.get("assets").and_then(|v| v.as_object())
    {
        for (id, value) in assets {
            if value.get("originalVersion").is_some()
                || snapshot.root().and_then(|r| r.get("nativeMediaVersions"))
                    .and_then(|m| m.get("owners")).and_then(|m| m.get(id.as_str().unwrap_or(""))).is_some()
            {
                // Ownership selects verified bytes; old path metadata is not
                // authority to substitute a different original on failure.
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
                    catalog.originals.entry(id.to_string()).or_insert(p);
                }
            }
        }
    }
    Ok(catalog)
}
