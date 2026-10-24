//! Shared test support: the parity corpus, PNG reading, comparisons.

#![allow(dead_code)]

pub mod doc;
pub mod reference_substitutions;
pub mod text;
pub mod text_residuals;

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
    let base = corpus_root()?;
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
/// newest `effects-*` capture (`render-effects.ts`), the newest
/// `composite-*` capture (`engine/corpus/src/projects/composite.ts`), the
/// newest `motion-*` capture (`motion.ts`), the compositing closure
/// captures `compclose-*`, `compclose2-*` and `compclose3-*`
/// (`compclose.ts`), and the newest `threed-*` capture (`space.ts`:
/// `3d/model-group`, `3d/group-roles`, `3d/maps-level1`).
pub fn corpus_dirs() -> Vec<PathBuf> {
    if let Ok(p) = std::env::var("PM_ENGINE_CORPUS") {
        return p.split(',').map(PathBuf::from).collect();
    }
    [
        "v1-",
        "render-",
        "effects-",
        "composite-",
        "motion-",
        "compclose-",
        "compclose2-",
        "compclose3-",
        "threed-",
    ]
    .iter()
    .filter_map(|p| newest_capture(p))
    .collect()
}

/// The private corpus directory, when this machine has one (gates with
/// approved residuals require their captures there; elsewhere they report
/// and pass vacuously).
pub fn corpus_root() -> Option<PathBuf> {
    Some(dirs_home()?.join("Library/Application Support/Powermove Engine Corpus"))
        .filter(|p| p.is_dir())
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
    /// Owner-authorized reference metadata; `png` always retains the original.
    pub reference_substitution: Option<pm_model::Value>,
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
                reference_substitution: member(s, "referenceSubstitution").cloned(),
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
            .or_else(|_| dir.strip_prefix(capture))
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
    if let Ok(real) = std::fs::read_dir(capture.join("real")) {
        out.extend(
            real.flatten()
                .map(|p| p.path())
                .filter(|p| p.join("loaded.json").is_file()),
        );
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

/// Today's viewer's reading of a video where the engine corrects it
/// (docs/engine/quality-ledger.md; each correction proven against an f64
/// reference by tests/video_reading.rs): BT.601 primaries read as Rec.709
/// (VIDEO-SD-1), chroma read as centered whatever its siting, and an SD
/// H.264 stream without primaries and transfer tags read with BT.601
/// coefficients even when it tags a matrix. Gates against legacy captures
/// restate it so they keep judging everything else.
pub fn restate_today(path: &Path, c: &mut pm_protocol::color::ColorDescription) {
    use pm_protocol::color::{ChromaSiting, MatrixCoefficients, Primaries, Range};
    c.siting = ChromaSiting::Center;
    if matches!(c.primaries, Primaries::Bt601_525 | Primaries::Bt601_625) {
        c.primaries = Primaries::Bt709;
    }
    let Ok(src) = pm_media::decode::service::MediaSource::open(path) else {
        return;
    };
    let Some(v) = src.probe.video.as_ref() else {
        return;
    };
    let t = &v.color.tagged;
    if v.codec.codec == "h264"
        && v.height <= 576
        && v.codec.bit_depth == 8
        && c.range == Range::Limited
        && t.primaries.is_none()
        && t.transfer.is_none()
    {
        c.matrix = MatrixCoefficients::Bt601;
    }
}

/// [`restate_today`] for every video a project's resolver names.
pub fn restate_today_for_videos(renderer: &mut pm_render::Renderer, resolver: &MapResolver) {
    for path in resolver.paths.values() {
        let Ok(read) = renderer.video_color(path) else {
            continue;
        };
        let mut c = read;
        restate_today(path, &mut c);
        if c != read {
            renderer.interpret(path, Some(c));
        }
    }
}

/// A reference rendition of an 8-bit subsampled clip for gates against
/// legacy captures: 4:4:4 (YUV4MPEG2, in `dir`) whose chroma is today's
/// reconstruction, linear and centered, computed in f64 and rounded once.
/// Drawn by the engine it shows today's chroma filter with everything else
/// the engine's, so the gate can hold the rest to its class; the filter is
/// proven against 4:4:4 truth by tests/chroma_truth.rs.
pub fn todays_chroma_rendition(path: &Path, dir: &Path) -> PathBuf {
    use pm_media::decode::service::{DecodeService, MediaSource, ServiceConfig};
    use pm_media::frame::Layout;
    use std::io::Write;
    let src = MediaSource::open(path).unwrap();
    let service = DecodeService::new(ServiceConfig::default());
    let mut session = service
        .session(&src, 1, pm_media::vt::DecodePurpose::Export)
        .unwrap();
    let out = dir.join(format!(
        "{}.y4m",
        path.file_stem().unwrap().to_string_lossy()
    ));
    let mut file = std::io::BufWriter::new(std::fs::File::create(&out).unwrap());
    let rate = src.rate();
    for n in 0..src.pts.len() {
        let frame = session.frame(n).unwrap();
        let f = frame.format.clone();
        assert_eq!(f.bits, 8, "8-bit sources only");
        let (w, h) = (f.width, f.height);
        if n == 0 {
            write!(
                file,
                "YUV4MPEG2 W{w} H{h} F{}:{} Ip A1:1 C444 XYSCSS=444\n",
                rate.num(),
                rate.den()
            )
            .unwrap();
        }
        let (fx, fy) = (1usize << f.log2_chroma_w, 1usize << f.log2_chroma_h);
        let (cw, ch) = (w.div_ceil(fx), h.div_ceil(fy));
        let planes = frame
            .with_planes(|p| {
                let at = |plane: usize, x: usize, yy: usize, bytes: usize, c: usize| unsafe {
                    *p[plane].ptr.add(yy * p[plane].stride + x * bytes + c)
                };
                let luma: Vec<u8> = (0..w * h).map(|i| at(0, i % w, i / w, 1, 0)).collect();
                let chroma = |k: usize| -> Vec<f64> {
                    (0..cw * ch)
                        .map(|i| {
                            f64::from(match f.layout {
                                Layout::Planar => at(1 + k, i % cw, i / cw, 1, 0),
                                Layout::Biplanar => at(1, i % cw, i / cw, 2, k),
                                other => panic!("{other:?}"),
                            })
                        })
                        .collect()
                };
                (luma, chroma(0), chroma(1))
            })
            .unwrap();
        let up = |p: &[f64]| -> Vec<u8> {
            let at = |i: i64, j: i64| {
                p[(j.clamp(0, ch as i64 - 1) as usize) * cw + i.clamp(0, cw as i64 - 1) as usize]
            };
            (0..w * h)
                .map(|k| {
                    let (x, yy) = (k % w, k / w);
                    let u = (x as f64 - (fx as f64 - 1.0) / 2.0) / fx as f64;
                    let v = (yy as f64 - (fy as f64 - 1.0) / 2.0) / fy as f64;
                    let (i, j) = (u.floor() as i64, v.floor() as i64);
                    let (tu, tv) = (u - u.floor(), v - v.floor());
                    let row = |j| at(i, j) * (1.0 - tu) + at(i + 1, j) * tu;
                    (row(j) * (1.0 - tv) + row(j + 1) * tv).round() as u8
                })
                .collect()
        };
        file.write_all(b"FRAME\n").unwrap();
        file.write_all(&planes.0).unwrap();
        file.write_all(&up(&planes.1)).unwrap();
        file.write_all(&up(&planes.2)).unwrap();
    }
    file.flush().unwrap();
    out
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

/// Every extension layer definition id the document's layers use.
pub fn extension_definitions(document: &Document) -> Vec<String> {
    fn walk(n: &pm_model::Node, out: &mut Vec<String>) {
        match n.kind() {
            pm_model::NodeKind::Object(map) => {
                let is_extension = map
                    .get(b"type".as_slice())
                    .is_some_and(|t| matches!(t.kind(), pm_model::NodeKind::String(s) if s.to_string_lossy() == "extension"));
                if is_extension
                    && let Some(d) = map.get(b"d".as_slice())
                    && let Some(def) = d.get("definition")
                    && let pm_model::NodeKind::String(s) = def.kind()
                {
                    out.push(s.to_string_lossy());
                }
                for v in map.values() {
                    walk(v, out);
                }
            }
            pm_model::NodeKind::Array(items) => items.iter().for_each(|i| walk(i, out)),
            _ => {}
        }
    }
    let mut out = Vec::new();
    if let Some(root) = document.root() {
        walk(root, &mut out);
    }
    out.sort();
    out.dedup();
    out
}

/// Routing reasons of the compositing and motion classes, which the
/// renderer draws for every layer type (docs/engine/composite.md and
/// motion.md): a corpus project routed for one of them fails the corpus
/// tests. Bezier path masks are drawn on today's Canvas2D coverage route
/// and no longer have a routing reason of their own.
#[allow(dead_code)]
pub fn routing_forbidden(feature: &pm_render::Feature) -> bool {
    use pm_render::Feature;
    match feature {
        Feature::Masks
        | Feature::MotionBlur
        | Feature::TwoAndAHalfD
        | Feature::CompatLevel2
        | Feature::RequiresEngine
        | Feature::ProjectColor => true,
        // Native-only text has no semantic routing exception. The level 2
        // semantic corpus exercises this predicate as well as rendering.
        Feature::Text(why) => why.contains("compat level 2"),
        _ => false,
    }
}
