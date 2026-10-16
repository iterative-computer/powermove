//! The parity corpus as `pm-cli parity` reads it (docs/engine/corpus.md):
//! captures under `~/Library/Application Support/Powermove Engine Corpus/`,
//! each project's loaded document, samples, and media checked by SHA-256.

use std::path::{Path, PathBuf};

use pm_export::render::MediaPaths;
use pm_model::{Document, Value};
use sha2::{Digest, Sha256};

/// Where captures live.
pub fn corpus_root() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .map(|h| h.join("Library/Application Support/Powermove Engine Corpus"))
}

/// The newest capture whose id starts with `prefix`.
pub fn newest_capture(prefix: &str) -> Option<PathBuf> {
    let base = corpus_root()?;
    let mut caps: Vec<PathBuf> = std::fs::read_dir(&base)
        .ok()?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| {
            p.is_dir()
                && p.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.starts_with(prefix))
        })
        .collect();
    caps.sort();
    caps.pop()
}

/// The default captures: the newest `v1-*` corpus capture and the newest
/// `render-*` capture (the render parity cases).
pub fn default_captures() -> Vec<PathBuf> {
    ["v1-", "render-"]
        .iter()
        .filter_map(|p| newest_capture(p))
        .collect()
}

/// The repository root, for fixture media (`origin: "fixture"`).
pub fn repo_root() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("PM_REPO_ROOT") {
        return Some(PathBuf::from(p));
    }
    let here = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..");
    here.join("apps/desktop/e2e/fixtures")
        .exists()
        .then(|| here.canonicalize().unwrap_or(here))
}

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

pub fn member<'a>(v: &'a Value, k: &str) -> Option<&'a Value> {
    match v {
        Value::Object(m) => m.get(k),
        _ => None,
    }
}

pub fn string(v: Option<&Value>) -> Option<String> {
    match v? {
        Value::String(s) => Some(s.to_string_lossy()),
        _ => None,
    }
}

pub fn number(v: Option<&Value>) -> Option<f64> {
    match v? {
        Value::Number(n) => Some(*n),
        _ => None,
    }
}

pub fn read_json(path: &Path) -> Result<Value, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    pm_model::json::parse(&text).map_err(|e| format!("{}: {e}", path.display()))
}

/// A corpus project: its loaded document, open composition, samples, media.
pub struct Project {
    /// `<group>/<name>`.
    pub name: String,
    pub document: Document,
    pub composition: String,
    pub samples: Vec<Sample>,
    pub media: MediaPaths,
    /// Media whose bytes are not available (missing or changed).
    pub missing_media: Vec<String>,
}

pub struct Sample {
    pub time: f64,
    pub width: u32,
    pub height: u32,
    pub png: PathBuf,
    /// The float frame of transparent projects (`.rgba32f`).
    pub float: Option<PathBuf>,
}

/// Every project directory of a capture, sorted.
pub fn projects(capture: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let Ok(groups) = std::fs::read_dir(capture.join("projects")) else {
        return out;
    };
    for group in groups.flatten() {
        if !group.path().is_dir() {
            continue;
        }
        if let Ok(list) = std::fs::read_dir(group.path()) {
            for p in list.flatten() {
                if p.path().join("loaded.json").exists() {
                    out.push(p.path());
                }
            }
        }
    }
    out.sort();
    out
}

pub fn load_project(capture: &Path, dir: &Path) -> Result<Project, String> {
    let loaded = read_json(&dir.join("loaded.json"))?;
    let record = read_json(&dir.join("project.json"))?;
    let composition = string(member(&loaded, "compId"))
        .or_else(|| string(member(&loaded, "id")))
        .ok_or("the project has no composition id")?;
    let mut media = MediaPaths::default();
    let mut missing_media = Vec::new();
    let repo = repo_root();
    if let Some(Value::Object(list)) = member(&record, "media") {
        for (name, entry) in list.iter() {
            let asset = string(member(entry, "assetId")).unwrap_or_default();
            let source = member(entry, "source");
            let origin = string(source.and_then(|s| member(s, "origin"))).unwrap_or_default();
            let path = string(source.and_then(|s| member(s, "path"))).unwrap_or_default();
            let sha = string(source.and_then(|s| member(s, "sha256"))).unwrap_or_default();
            let path = if origin == "fixture" {
                match &repo {
                    Some(r) => r.join(path),
                    None => {
                        missing_media.push(name.to_string_lossy());
                        continue;
                    }
                }
            } else {
                let p = PathBuf::from(&path);
                if p.is_absolute() { p } else { capture.join(p) }
            };
            match std::fs::read(&path) {
                Ok(bytes) if sha256_hex(&bytes) == sha => media.insert(asset, path),
                _ => missing_media.push(name.to_string_lossy()),
            }
        }
    }
    let mut samples = Vec::new();
    if let Some(Value::Array(list)) = member(&record, "samples") {
        for s in list {
            let Some(png) = string(member(s, "png").and_then(|p| member(p, "file"))) else {
                continue;
            };
            samples.push(Sample {
                time: number(member(s, "time")).ok_or("a sample has no time")?,
                width: number(member(s, "width")).ok_or("a sample has no width")? as u32,
                height: number(member(s, "height")).ok_or("a sample has no height")? as u32,
                png: capture.join(png),
                float: string(
                    member(s, "float")
                        .and_then(|p| member(p, "data"))
                        .and_then(|p| member(p, "file")),
                )
                .map(|f| capture.join(f)),
            });
        }
    }
    Ok(Project {
        name: dir
            .strip_prefix(capture.join("projects"))
            .map_err(|_| "a project outside its capture")?
            .display()
            .to_string(),
        document: Document::new(Some(loaded), 1),
        composition,
        samples,
        media,
        missing_media,
    })
}

/// An 8-bit RGBA PNG, top-down.
pub fn read_png(path: &Path) -> Result<(u32, u32, Vec<u8>), String> {
    let file = std::fs::File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let decoder = png::Decoder::new(std::io::BufReader::new(file));
    let mut reader = decoder
        .read_info()
        .map_err(|e| format!("{}: {e}", path.display()))?;
    let mut buf = vec![0; reader.output_buffer_size().ok_or("PNG too large")?];
    let info = reader
        .next_frame(&mut buf)
        .map_err(|e| format!("{}: {e}", path.display()))?;
    if info.color_type != png::ColorType::Rgba || info.bit_depth != png::BitDepth::Eight {
        return Err(format!("{}: not 8-bit RGBA", path.display()));
    }
    buf.truncate(info.buffer_size());
    Ok((info.width, info.height, buf))
}

/// Writes an 8-bit RGBA PNG (diagnostics; never into the repository).
pub fn write_png(path: &Path, width: u32, height: u32, rgba: &[u8]) -> Result<(), String> {
    let file = std::fs::File::create(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut e = png::Encoder::new(std::io::BufWriter::new(file), width, height);
    e.set_color(png::ColorType::Rgba);
    e.set_depth(png::BitDepth::Eight);
    e.write_header()
        .and_then(|mut w| w.write_image_data(rgba))
        .map_err(|e| format!("{}: {e}", path.display()))
}

/// A capture's float frame (`.rgba32f`, little-endian f32 RGBA, top-down).
pub fn read_float(path: &Path) -> Result<Vec<f32>, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(bytes
        .chunks_exact(4)
        .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        .collect())
}

/// Whether a document holds 3D object layers in any composition.
pub fn has_3d_objects(document: &Document) -> bool {
    fn layers_have(v: Option<&pm_model::Node>) -> bool {
        let Some(layers) = v.and_then(|v| v.get("layers")) else {
            return false;
        };
        match layers.kind() {
            pm_model::NodeKind::Array(list) => list.iter().any(|l| {
                l.get("d").and_then(|d| d.get("definition")).is_some_and(|d| {
                    matches!(d.kind(), pm_model::NodeKind::String(s) if s.to_string_lossy() == "powermove.3d.object")
                })
            }),
            _ => false,
        }
    }
    let Some(root) = document.root() else {
        return false;
    };
    if layers_have(Some(root)) {
        return true;
    }
    match root.get("comps").map(|c| c.kind()) {
        Some(pm_model::NodeKind::Object(map)) => map.iter().any(|(_, c)| layers_have(Some(c))),
        _ => false,
    }
}

/// Video assets of a document, with whether the legacy frame used a
/// playback proxy (`playbackProxy`).
pub fn video_assets(document: &Document) -> Vec<(String, bool)> {
    let mut out = vec![];
    let Some(assets) = document.root().and_then(|r| r.get("assets")) else {
        return out;
    };
    if let pm_model::NodeKind::Object(map) = assets.kind() {
        for (id, record) in map {
            let kind = record.get("kind").and_then(|k| match k.kind() {
                pm_model::NodeKind::String(s) => Some(s.to_string_lossy()),
                _ => None,
            });
            if kind.as_deref() != Some("video") {
                continue;
            }
            let proxied = matches!(
                record.get("playbackProxy").map(|n| n.kind()),
                Some(pm_model::NodeKind::Bool(true))
            );
            out.push((id.to_string_lossy(), proxied));
        }
    }
    out
}
