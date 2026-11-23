//! Local-file admission shared by UI, linked folders and agent imports.
//! Resolve companions once, before the source is moved into project storage.
use super::{ImportFile, MAX_BYTES, Prepared};
use crate::{Error, Result};
use base64::Engine;
use std::path::Path;

pub const EXTENSIONS: &[&str] = &[
    "glb",
    "gltf",
    "obj",
    "fbx",
    "ply",
    "splat",
    "zip",
    "stl",
    "off",
    "3mf",
    "dae",
    "3ds",
    "vrm",
    "step",
    "stp",
    "iges",
    "igs",
    "brep",
    #[cfg(target_os = "macos")]
    "usd",
    #[cfg(target_os = "macos")]
    "usda",
    #[cfg(target_os = "macos")]
    "usdc",
    #[cfg(target_os = "macos")]
    "usdz",
    #[cfg(target_os = "macos")]
    "abc",
];
#[cfg(target_os = "macos")]
pub const UNSUPPORTED: &[&str] = &["blend"];
#[cfg(not(target_os = "macos"))]
pub const UNSUPPORTED: &[&str] = &["blend", "usd", "usda", "usdc", "usdz", "abc"];

pub fn supports_extension(extension: &str) -> bool {
    if EXTENSIONS.contains(&extension) {
        return true;
    }
    #[cfg(target_os = "macos")]
    if !UNSUPPORTED.contains(&extension) {
        return super::modelio::supports(extension);
    }
    false
}

/// The source's coordinates are in metres after canonical conversion.
pub fn has_metre_units(extension: &str) -> bool {
    matches!(
        extension,
        "glb" | "gltf" | "fbx" | "vrm" | "3mf" | "dae" | "step" | "stp" | "iges" | "igs" | "brep"
    )
}

pub fn mime(extension: &str) -> &'static str {
    match extension {
        "glb" => "model/gltf-binary",
        "vrm" => "model/vrm",
        "gltf" => "model/gltf+json",
        "obj" => "model/obj",
        "fbx" => "model/fbx",
        "ply" => "model/ply",
        "splat" => "model/splat",
        "stl" => "model/stl",
        "3mf" => "model/3mf",
        "dae" => "model/vnd.collada+xml",
        "3ds" => "image/x-3ds",
        "usd" | "usda" | "usdc" => "model/vnd.usd",
        "usdz" => "model/vnd.usdz+zip",
        "step" | "stp" => "model/step",
        "iges" | "igs" => "model/iges",
        "zip" => "application/zip",
        _ => "application/octet-stream",
    }
}

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
    Ok(root.join(relative_reference(uri)?))
}

pub(super) fn relative_reference(uri: &str) -> Result<String> {
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
    Ok(name)
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
    if ext == "gltf" || (ext == "vrm" && !bytes.starts_with(b"glTF")) {
        let bytes = embed_gltf(&bytes, |uri| read(&companion(root, uri)?, &mut total))?;
        return super::prepare(
            vec![ImportFile {
                name,
                mime: mime(&ext).into(),
                bytes,
            }],
            None,
        );
    }
    let mut files = vec![ImportFile {
        name,
        mime: mime(&ext).into(),
        bytes,
    }];
    if ext == "obj" {
        let companions = obj_companions(&files[0].bytes, |uri| {
            read(&companion(root, uri)?, &mut total)
        })?;
        files.extend(companions);
    }
    super::prepare(files, None)
}

pub(super) fn embed_gltf(
    bytes: &[u8],
    mut resolve: impl FnMut(&str) -> Result<Vec<u8>>,
) -> Result<Vec<u8>> {
    let mut doc: serde_json::Value =
        serde_json::from_slice(bytes).map_err(|e| Error(format!("Cannot read glTF: {e}")))?;
    let mut total = bytes.len();
    for key in ["buffers", "images"] {
        if let Some(list) = doc[key].as_array_mut() {
            for item in list {
                if let Some(uri) = item["uri"].as_str().filter(|u| !u.starts_with("data:")) {
                    let data = resolve(uri)?;
                    total = total.saturating_add(data.len().div_ceil(3) * 4);
                    if total > MAX_BYTES {
                        return Err(Error("Embedded glTF exceeds 512 MB".into()));
                    }
                    let mime = if key == "images" {
                        match super::ext(uri).as_str() {
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
    serde_json::to_vec(&doc).map_err(|e| Error(e.to_string()))
}

pub(super) fn obj_companions(
    bytes: &[u8],
    mut read: impl FnMut(&str) -> Result<Vec<u8>>,
) -> Result<Vec<ImportFile>> {
    let mut files: Vec<ImportFile> = Vec::new();
    let text = String::from_utf8_lossy(bytes);
    for line in text
        .lines()
        .filter_map(|l| l.trim().strip_prefix("mtllib "))
    {
        for mtl in line.split_whitespace() {
            let data = read(mtl)?;
            let text = String::from_utf8_lossy(&data);
            for line in text.lines() {
                let mut words = line.split_whitespace();
                if words
                    .next()
                    .is_some_and(|w| w.starts_with("map_") || matches!(w, "bump" | "norm"))
                    && let Some(uri) = words.last()
                    && !files.iter().any(|f| f.name == uri)
                {
                    let base = mtl.rsplit_once(['/', '\\']).map(|(base, _)| base);
                    let relative =
                        base.map_or_else(|| uri.to_owned(), |base| format!("{base}/{uri}"));
                    files.push(ImportFile {
                        name: uri.into(),
                        mime: String::new(),
                        bytes: read(&relative)?,
                    });
                }
            }
            files.push(ImportFile {
                name: mtl.into(),
                mime: String::new(),
                bytes: data,
            });
            if files.len() >= 128 {
                return Err(Error("Choose at most 128 model files".into()));
            }
        }
    }
    Ok(files)
}
