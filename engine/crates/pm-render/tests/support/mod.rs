//! Shared test support: the parity corpus, PNG reading, comparisons.

#![allow(dead_code)]

pub mod doc;
pub mod text;

use std::path::{Path, PathBuf};

use pm_model::Document;
use pm_render::MapResolver;
use sha2::{Digest, Sha256};

/// The repository root (for fixture media).
pub fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../..")
        .canonicalize()
        .unwrap()
}

/// The newest capture whose id starts with `prefix`.
pub fn newest_capture(prefix: &str) -> Option<PathBuf> {
    let base = dirs_home()?.join("Library/Application Support/Powermove Engine Corpus");
    let mut caps: Vec<PathBuf> = std::fs::read_dir(&base)
        .ok()?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with(prefix))
        })
        .collect();
    caps.sort();
    caps.pop()
}

/// The captures to compare against: `PM_ENGINE_CORPUS` (comma-separated),
/// or the newest `v1-*` corpus capture, the newest `render-*` capture (the
/// render parity cases, `engine/corpus/src/projects/render.ts`), the
/// newest `effects-*` capture (`render-effects.ts`), and the newest
/// `composite-*` capture (`engine/corpus/src/projects/composite.ts`).
pub fn corpus_dirs() -> Vec<PathBuf> {
    if let Ok(p) = std::env::var("PM_ENGINE_CORPUS") {
        return p.split(',').map(PathBuf::from).collect();
    }
    ["v1-", "render-", "effects-", "composite-"]
        .iter()
        .filter_map(|p| newest_capture(p))
        .collect()
}

fn dirs_home() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

pub fn read_json(path: &Path) -> pm_model::Value {
    pm_model::json::parse(&std::fs::read_to_string(path).unwrap()).unwrap()
}

/// A corpus project: its loaded document, open composition, samples, media.
pub struct Project {
    pub name: String,
    pub dir: PathBuf,
    pub document: Document,
    pub composition: String,
    pub samples: Vec<Sample>,
    pub resolver: MapResolver,
    /// Media whose bytes are not available (missing or changed).
    pub missing_media: Vec<String>,
}

pub struct Sample {
    pub time: f64,
    pub width: u32,
    pub height: u32,
    pub png: PathBuf,
    pub float: Option<PathBuf>,
    /// Transparent float frames at other motion-blur sample counts
    /// (`blurSamples` cases): (samples, file).
    pub blur: Vec<(u32, PathBuf)>,
}

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn member<'a>(v: &'a pm_model::Value, k: &str) -> Option<&'a pm_model::Value> {
    match v {
        pm_model::Value::Object(m) => m.get(k),
        _ => None,
    }
}

fn string(v: Option<&pm_model::Value>) -> Option<String> {
    match v? {
        pm_model::Value::String(s) => Some(s.to_string_lossy()),
        _ => None,
    }
}

fn number(v: Option<&pm_model::Value>) -> Option<f64> {
    match v? {
        pm_model::Value::Number(n) => Some(*n),
        _ => None,
    }
}

pub fn load_project(capture: &Path, dir: &Path) -> Project {
    let loaded = read_json(&dir.join("loaded.json"));
    let record = read_json(&dir.join("project.json"));
    let composition = string(member(&loaded, "compId"))
        .or_else(|| string(member(&loaded, "id")))
        .unwrap();
    let mut resolver = MapResolver::default();
    let mut missing_media = Vec::new();
    if let Some(pm_model::Value::Object(media)) = member(&record, "media") {
        for (name, entry) in media.iter() {
            let asset = string(member(entry, "assetId")).unwrap_or_default();
            let source = member(entry, "source");
            let origin = string(source.and_then(|s| member(s, "origin"))).unwrap_or_default();
            let path = string(source.and_then(|s| member(s, "path"))).unwrap_or_default();
            let sha = string(source.and_then(|s| member(s, "sha256"))).unwrap_or_default();
            let path = if origin == "fixture" {
                repo_root().join(path)
            } else {
                let p = PathBuf::from(&path);
                if p.is_absolute() { p } else { capture.join(p) }
            };
            match std::fs::read(&path) {
                Ok(bytes) if sha256_hex(&bytes) == sha => {
                    resolver.paths.insert(asset, path);
                }
                _ => missing_media.push(name.to_string_lossy()),
            }
        }
    }
    let mut samples = Vec::new();
    if let Some(pm_model::Value::Array(list)) = member(&record, "samples") {
        for s in list {
            let png = string(member(s, "png").and_then(|p| member(p, "file")));
            let Some(png) = png else { continue };
            samples.push(Sample {
                time: number(member(s, "time")).unwrap(),
                width: number(member(s, "width")).unwrap() as u32,
                height: number(member(s, "height")).unwrap() as u32,
                png: capture.join(png),
                float: string(
                    member(s, "float")
                        .and_then(|p| member(p, "data"))
                        .and_then(|p| member(p, "file")),
                )
                .map(|f| capture.join(f)),
                blur: match member(s, "blur") {
                    Some(pm_model::Value::Array(list)) => list
                        .iter()
                        .filter_map(|b| {
                            let n = number(member(b, "samples"))? as u32;
                            let file = string(
                                member(b, "float")
                                    .and_then(|p| member(p, "data"))
                                    .and_then(|p| member(p, "file")),
                            )?;
                            Some((n, capture.join(file)))
                        })
                        .collect(),
                    _ => Vec::new(),
                },
            });
        }
    }
    Project {
        name: dir
            .strip_prefix(capture.join("projects"))
            .unwrap()
            .display()
            .to_string(),
        dir: dir.to_path_buf(),
        document: Document::new(Some(loaded), 1),
        composition,
        samples,
        resolver,
        missing_media,
    }
}

/// Every project directory of a capture, sorted.
pub fn projects(capture: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let root = capture.join("projects");
    for group in std::fs::read_dir(&root).unwrap().flatten() {
        if !group.path().is_dir() {
            continue;
        }
        for p in std::fs::read_dir(group.path()).unwrap().flatten() {
            if p.path().join("loaded.json").exists() {
                out.push(p.path());
            }
        }
    }
    out.sort();
    out
}

/// An 8-bit RGBA PNG, top-down.
pub fn read_png(path: &Path) -> (u32, u32, Vec<u8>) {
    let decoder = png::Decoder::new(std::io::BufReader::new(std::fs::File::open(path).unwrap()));
    let mut reader = decoder.read_info().unwrap();
    let mut buf = vec![0; reader.output_buffer_size().unwrap()];
    let info = reader.next_frame(&mut buf).unwrap();
    assert_eq!(info.color_type, png::ColorType::Rgba, "{}", path.display());
    assert_eq!(info.bit_depth, png::BitDepth::Eight);
    buf.truncate(info.buffer_size());
    (info.width, info.height, buf)
}

/// Writes an 8-bit RGBA PNG (diagnostics only; never into the repository).
pub fn write_png(path: &Path, width: u32, height: u32, rgba: &[u8]) {
    let file = std::fs::File::create(path).unwrap();
    let mut e = png::Encoder::new(std::io::BufWriter::new(file), width, height);
    e.set_color(png::ColorType::Rgba);
    e.set_depth(png::BitDepth::Eight);
    e.write_header().unwrap().write_image_data(rgba).unwrap();
}

/// Float to 8-bit as a unorm render-target write does: clamp, scale, round
/// to nearest even.
pub fn unorm8(v: f32) -> u8 {
    let v = if v.is_nan() { 0.0 } else { v.clamp(0.0, 1.0) };
    (v * 255.0).round_ties_even() as u8
}

/// The capture's opaque view of premultiplied working RGBA: color over black,
/// alpha 255 (`FRAG_CAPTURE_OPAQUE` into RGBA8).
pub fn opaque8(rgba: &[f32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(rgba.len());
    for px in rgba.chunks_exact(4) {
        out.extend_from_slice(&[unorm8(px[0]), unorm8(px[1]), unorm8(px[2]), 255]);
    }
    out
}

/// Section 25's render class, Standard color: per-channel 8-bit difference
/// ≤ 1 for 99.9% of pixels, ≤ 4 for any pixel.
#[derive(Debug, Clone, Copy, Default)]
pub struct Diff {
    pub pixels: usize,
    /// Pixels whose largest channel difference is ≤ 1.
    pub within_one: usize,
    pub max: u8,
    pub exact: usize,
}

impl Diff {
    pub fn of(a: &[u8], b: &[u8]) -> Diff {
        assert_eq!(a.len(), b.len());
        let mut d = Diff::default();
        for (p, q) in a.chunks_exact(4).zip(b.chunks_exact(4)) {
            let m = (0..4).map(|i| p[i].abs_diff(q[i])).max().unwrap();
            d.pixels += 1;
            if m <= 1 {
                d.within_one += 1;
            }
            if m == 0 {
                d.exact += 1;
            }
            d.max = d.max.max(m);
        }
        d
    }

    pub fn fraction_within_one(&self) -> f64 {
        self.within_one as f64 / self.pixels.max(1) as f64
    }

    pub fn passes(&self) -> bool {
        self.fraction_within_one() >= 0.999 && self.max <= 4
    }
}

/// Seconds to ticks on the flick grid.
pub fn ticks(seconds: f64) -> pm_protocol::common::Ticks {
    pm_protocol::common::Ticks::new((seconds * pm_time::TICKS_PER_SECOND as f64).round() as i64)
        .unwrap()
}

/// Pixels where the reference's 3×3 neighborhood is uniform within 1 CV
/// (patch interiors, section 25's video class).
pub fn interior(reference: &[u8], width: u32, height: u32) -> Vec<bool> {
    let (w, h) = (width as i64, height as i64);
    let at = |x: i64, y: i64, c: usize| reference[((y * w + x) * 4) as usize + c];
    let mut out = vec![false; (w * h) as usize];
    for y in 1..h - 1 {
        for x in 1..w - 1 {
            let mut uniform = true;
            'n: for dy in -1..=1 {
                for dx in -1..=1 {
                    for c in 0..3 {
                        if at(x + dx, y + dy, c).abs_diff(at(x, y, c)) > 1 {
                            uniform = false;
                            break 'n;
                        }
                    }
                }
            }
            out[(y * w + x) as usize] = uniform;
        }
    }
    out
}

/// The largest channel difference over masked pixels, and how many.
pub fn masked_max(a: &[u8], b: &[u8], mask: &[bool]) -> (u8, usize) {
    let mut max = 0;
    let mut n = 0;
    for (i, &m) in mask.iter().enumerate() {
        if !m {
            continue;
        }
        n += 1;
        for c in 0..3 {
            max = max.max(a[i * 4 + c].abs_diff(b[i * 4 + c]));
        }
    }
    (max, n)
}

/// A capture's float frame (`.rgba32f`, little-endian f32 RGBA, top-down).
pub fn read_float(path: &Path) -> Vec<f32> {
    let bytes = std::fs::read(path).unwrap();
    bytes
        .chunks_exact(4)
        .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        .collect()
}

/// The largest difference between two float frames, in 8-bit code values.
pub fn float_max_cv(a: &[f32], b: &[f32]) -> f64 {
    assert_eq!(a.len(), b.len());
    a.iter()
        .zip(b)
        .map(|(x, y)| (f64::from(*x) - f64::from(*y)).abs() * 255.0)
        .fold(0.0, f64::max)
}

impl Diff {
    /// [`Diff::of`] over the pixels where `mask` is set.
    pub fn of_masked(a: &[u8], b: &[u8], mask: &[bool]) -> Diff {
        assert_eq!(a.len(), b.len());
        let mut d = Diff::default();
        for ((p, q), &m) in a.chunks_exact(4).zip(b.chunks_exact(4)).zip(mask) {
            if !m {
                continue;
            }
            let m = (0..4).map(|i| p[i].abs_diff(q[i])).max().unwrap();
            d.pixels += 1;
            if m <= 1 {
                d.within_one += 1;
            }
            if m == 0 {
                d.exact += 1;
            }
            d.max = d.max.max(m);
        }
        d
    }
}

/// [`float_max_cv`] over the pixels where `mask` is set.
pub fn float_max_cv_masked(a: &[f32], b: &[f32], mask: &[bool]) -> f64 {
    assert_eq!(a.len(), b.len());
    let mut max = 0.0f64;
    for (i, &m) in mask.iter().enumerate() {
        if !m {
            continue;
        }
        for c in 0..4 {
            max = max.max((f64::from(a[i * 4 + c]) - f64::from(b[i * 4 + c])).abs() * 255.0);
        }
    }
    max
}

/// Premultiplied float RGBA to 8-bit RGBA (alpha kept).
pub fn rgba8(rgba: &[f32]) -> Vec<u8> {
    rgba.iter().map(|v| unorm8(*v)).collect()
}

/// Whether a document holds 3D object layers in any composition.
pub fn has_3d_objects(document: &pm_model::Document) -> bool {
    fn layers_have(v: Option<&pm_model::Node>) -> bool {
        let Some(v) = v else { return false };
        let Some(layers) = v.get("layers") else {
            return false;
        };
        match layers.kind() {
            pm_model::NodeKind::Array(list) => list.iter().any(|l| {
                l.get("d")
                    .and_then(|d| d.get("definition"))
                    .is_some_and(|d| {
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
