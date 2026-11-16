//! Local-file admission shared by UI, linked folders and agent imports.
//! Resolve companions once, before the source is moved into project storage.
use super::{ImportFile, MAX_BYTES, Prepared};
use crate::{Error, Result};
use base64::Engine;
use std::path::Path;

pub const EXTENSIONS: &[&str] = &["glb", "gltf", "obj", "fbx", "ply", "splat"];
pub const UNSUPPORTED: &[&str] = &["usdz", "usd", "usda", "usdc", "stl", "blend"];
pub fn unsupported(path: &Path) -> Option<String> {
    let e = path.extension()?.to_str()?.to_ascii_lowercase();
    UNSUPPORTED.contains(&e.as_str()).then(|| {
        format!(
            "{} isn't supported yet. Export as glTF (.glb) and import that.",
            e.to_ascii_uppercase()
        )
    })
}
fn read(path: &Path, total: &mut usize) -> Result<Vec<u8>> {
    let size = std::fs::metadata(path)
        .map_err(|_| {
            Error(format!(
                "Missing model file: {}. Keep the model, .bin, materials and textures together.",
                path.file_name().unwrap_or_default().to_string_lossy()
            ))
        })?
        .len();
    if size > MAX_BYTES as u64 || total.saturating_add(size as usize) > MAX_BYTES {
        return Err(Error("Model and companion files exceed 512 MB".into()));
    }
    *total += size as usize;
    std::fs::read(path).map_err(|e| Error(format!("Cannot read model file: {e}")))
}
fn companion(root: &Path, uri: &str) -> Result<std::path::PathBuf> {
    // Local references only; never fetch URLs or open absolute paths from a model.
    let mut decoded = Vec::new();
    let mut bytes = uri.bytes();
    while let Some(b) = bytes.next() {
        decoded.push(if b == b'%' {
            let h = bytes.next().and_then(|c| (c as char).to_digit(16));
            let l = bytes.next().and_then(|c| (c as char).to_digit(16));
            let (h, l) = h
                .zip(l)
                .ok_or_else(|| Error("Invalid model file reference".into()))?;
            (h * 16 + l) as u8
        } else {
            b
        });
    }
    let name = String::from_utf8(decoded)
        .map_err(|_| Error("Invalid model file reference".into()))?
        .replace('\\', "/");
    let p = Path::new(&name);
    if name.contains(':')
        || p.is_absolute()
        || p.components()
            .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        return Err(Error(
            "Model companions must be relative files beside the model".into(),
        ));
    }
    Ok(root.join(p))
}
pub fn prepare(path: &Path) -> Result<Prepared> {
    if let Some(message) = unsupported(path) {
        return Err(Error(message));
    }
    let name = path
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned();
    let ext = path
        .extension()
        .unwrap_or_default()
        .to_string_lossy()
        .to_ascii_lowercase();
    let root = path.parent().unwrap_or(Path::new("."));
    let mut total = 0;
    let bytes = read(path, &mut total)?;
    if ext == "gltf" {
        let mut doc: serde_json::Value =
            serde_json::from_slice(&bytes).map_err(|e| Error(format!("Cannot read glTF: {e}")))?;
        for key in ["buffers", "images"] {
            if let Some(list) = doc[key].as_array_mut() {
                for item in list {
                    if let Some(uri) = item["uri"].as_str().filter(|u| !u.starts_with("data:")) {
                        let resource = companion(root, uri)?;
                        let data = read(&resource, &mut total)?;
                        let mime = if key == "images" {
                            match resource
                                .extension()
                                .unwrap_or_default()
                                .to_string_lossy()
                                .to_ascii_lowercase()
                                .as_str()
                            {
                                "png" => "image/png",
                                "jpg" | "jpeg" => "image/jpeg",
                                "webp" => "image/webp",
                                _ => "application/octet-stream",
                            }
                        } else {
                            "application/octet-stream"
                        };
                        item["uri"] = serde_json::Value::String(format!(
                            "data:{mime};base64,{}",
                            base64::engine::general_purpose::STANDARD.encode(data)
                        ));
                    }
                }
            }
        }
        return super::prepare(
            vec![ImportFile {
                name,
                mime: "model/gltf+json".into(),
                bytes: serde_json::to_vec(&doc).map_err(|e| Error(e.to_string()))?,
            }],
            None,
        );
    }
    let mut files = vec![ImportFile {
        name,
        mime: String::new(),
        bytes,
    }];
    if ext == "obj" {
        let text = String::from_utf8_lossy(&files[0].bytes).into_owned();
        for line in text
            .lines()
            .filter_map(|l| l.trim().strip_prefix("mtllib "))
        {
            for mtl in line.split_whitespace() {
                let p = companion(root, mtl)?;
                let data = read(&p, &mut total)?;
                let text = String::from_utf8_lossy(&data).into_owned();
                files.push(ImportFile {
                    name: mtl.into(),
                    mime: String::new(),
                    bytes: data,
                });
                for line in text.lines() {
                    let mut words = line.split_whitespace();
                    if words
                        .next()
                        .is_some_and(|w| w.starts_with("map_") || matches!(w, "bump" | "norm"))
                    {
                        if let Some(uri) = words.last() {
                            let texture = companion(p.parent().unwrap_or(root), uri)?;
                            if !files.iter().any(|f| f.name == uri) {
                                files.push(ImportFile {
                                    name: uri.into(),
                                    mime: String::new(),
                                    bytes: read(&texture, &mut total)?,
                                });
                            }
                        }
                    }
                }
            }
        }
    }
    super::prepare(files, None)
}
