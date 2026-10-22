//! `pm-cli scope` (plan section 8, "Scopes": an internal waveform and
//! vectorscope for the QA lane, reading a full-precision working render).
//!
//! The frame is rendered by the engine at 32-bit precision (or read from a
//! capture: a corpus `.rgba32f` float frame or an 8-bit PNG) and measured
//! on its working values: premultiplied, display encoding E, Rec.709
//! primaries, unclamped, the background composited (a transparent frame is
//! measured over black, as premultiplied values are). Luma and color
//! difference use BT.709 coefficients on the E-encoded values (Y′, Cb, Cr
//! in full range, Cb and Cr in −0.5..0.5 for legal colors).
//!
//! Outputs, in the output directory:
//!
//! - `waveform.png`: luma waveform, one column per frame column (binned to
//!   at most `--columns`), levels −0.1 to 1.1 bottom to top, graticule at
//!   0, 0.25, 0.5, 0.75, 1; log-scaled counts.
//! - `parade.png`: R′, G′, B′ waveforms side by side, same scale.
//! - `vectorscope.png`: Cb (right) and Cr (up) from −0.6 to 0.6, with the
//!   75% and 100% color-bar targets and the skin-tone line.
//! - `scope.json`: per-channel and luma statistics (minimum, maximum, mean,
//!   0.1%, 50%, and 99.9% percentiles, counts below 0 and above 1, NaN
//!   counts), the largest chroma magnitude and its hue, a 256-bin luma
//!   histogram, and where the frame came from.

use std::path::{Path, PathBuf};

use pm_protocol::color::Precision;
use pm_protocol::common::{Size, Ticks};
use pm_render::motion::MotionBlur;
use pm_render::{RenderRequest, Renderer};
use serde::Serialize;

/// Waveform levels (rows), covering [`LEVEL_LO`, `LEVEL_HI`).
pub const LEVELS: usize = 600;
pub const LEVEL_LO: f64 = -0.1;
pub const LEVEL_HI: f64 = 1.1;
/// Vectorscope bins per axis, covering ±[`VECTOR_RANGE`].
pub const VECTOR: usize = 512;
pub const VECTOR_RANGE: f64 = 0.6;

/// BT.709 luma coefficients.
const KR: f64 = 0.2126;
const KB: f64 = 0.0722;
const KG: f64 = 1.0 - KR - KB;

#[derive(Debug, Clone, Default)]
pub struct Options {
    pub input: Option<PathBuf>,
    /// Measure a captured frame instead of rendering.
    pub frame: Option<PathBuf>,
    pub time: f64,
    pub composition: Option<String>,
    pub size: Option<(u32, u32)>,
    pub scale: Option<f64>,
    pub media: Vec<(String, PathBuf)>,
    pub motion_blur: Option<String>,
    /// Render without the composition background.
    pub transparent: bool,
    pub columns: Option<u32>,
    pub output: PathBuf,
    /// The editor's display scale factor (`crate::fonts`).
    pub ui_scale: Option<f32>,
}

/// Y′, Cb, Cr (BT.709, full range) of E-encoded R′G′B′.
pub fn ycbcr(r: f64, g: f64, b: f64) -> (f64, f64, f64) {
    let y = KR * r + KG * g + KB * b;
    (
        y,
        (b - y) / (2.0 * (1.0 - KB)),
        (r - y) / (2.0 * (1.0 - KR)),
    )
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ChannelStats {
    pub min: f64,
    pub max: f64,
    pub mean: f64,
    pub p001: f64,
    pub p50: f64,
    pub p999: f64,
    pub below_zero: u64,
    pub above_one: u64,
    pub nan: u64,
}

fn channel_stats(values: &mut [f64], nan: u64) -> ChannelStats {
    values.sort_by(f64::total_cmp);
    let n = values.len();
    let at = |q: f64| {
        if n == 0 {
            f64::NAN
        } else {
            values[((q * (n - 1) as f64).round() as usize).min(n - 1)]
        }
    };
    ChannelStats {
        min: values.first().copied().unwrap_or(f64::NAN),
        max: values.last().copied().unwrap_or(f64::NAN),
        mean: if n == 0 {
            f64::NAN
        } else {
            values.iter().sum::<f64>() / n as f64
        },
        p001: at(0.001),
        p50: at(0.5),
        p999: at(0.999),
        below_zero: values.iter().filter(|v| **v < 0.0).count() as u64,
        above_one: values.iter().filter(|v| **v > 1.0).count() as u64,
        nan,
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Stats {
    pub width: u32,
    pub height: u32,
    pub red: ChannelStats,
    pub green: ChannelStats,
    pub blue: ChannelStats,
    pub luma: ChannelStats,
    /// The largest √(Cb² + Cr²) and its hue in degrees (Cr over Cb).
    pub chroma_max: f64,
    pub chroma_max_hue: f64,
    /// Luma in 256 bins over [0, 1] (values outside count in the end bins).
    pub luma_histogram: Vec<u64>,
}

/// The measured scopes of one frame.
pub struct Scopes {
    pub columns: usize,
    /// `columns × LEVELS`, column-major (column c at `c * LEVELS`).
    pub waveform: Vec<u32>,
    /// R′, G′, B′ waveforms, the same layout.
    pub parade: [Vec<u32>; 3],
    /// `VECTOR × VECTOR`, row-major, row 0 at Cr = +range.
    pub vectorscope: Vec<u32>,
    pub stats: Stats,
}

fn level_bin(v: f64) -> Option<usize> {
    if !v.is_finite() {
        return None;
    }
    let t = (v - LEVEL_LO) / (LEVEL_HI - LEVEL_LO);
    Some(((t * LEVELS as f64).floor().max(0.0) as usize).min(LEVELS - 1))
}

fn vector_bin(cb: f64, cr: f64) -> Option<(usize, usize)> {
    if !cb.is_finite() || !cr.is_finite() {
        return None;
    }
    let to = |v: f64| {
        let t = (v + VECTOR_RANGE) / (2.0 * VECTOR_RANGE);
        ((t * VECTOR as f64).floor().max(0.0) as usize).min(VECTOR - 1)
    };
    Some((to(cb), VECTOR - 1 - to(cr)))
}

/// Measures premultiplied RGBA working values (top-down rows).
pub fn analyze(rgba: &[f32], width: u32, height: u32, max_columns: usize) -> Scopes {
    let (w, h) = (width as usize, height as usize);
    assert_eq!(rgba.len(), w * h * 4);
    let columns = w.min(max_columns.max(1));
    let mut waveform = vec![0u32; columns * LEVELS];
    let mut parade = [
        vec![0u32; columns * LEVELS],
        vec![0u32; columns * LEVELS],
        vec![0u32; columns * LEVELS],
    ];
    let mut vectorscope = vec![0u32; VECTOR * VECTOR];
    let mut channels = [
        Vec::with_capacity(w * h),
        Vec::with_capacity(w * h),
        Vec::with_capacity(w * h),
    ];
    let mut nan = [0u64; 4];
    let mut lumas = Vec::with_capacity(w * h);
    let mut histogram = vec![0u64; 256];
    let (mut chroma_max, mut chroma_hue) = (0.0f64, 0.0f64);
    for y in 0..h {
        for x in 0..w {
            let i = (y * w + x) * 4;
            let col = x * columns / w;
            let px = [
                f64::from(rgba[i]),
                f64::from(rgba[i + 1]),
                f64::from(rgba[i + 2]),
            ];
            for c in 0..3 {
                if px[c].is_nan() {
                    nan[c] += 1;
                } else {
                    channels[c].push(px[c]);
                }
                if let Some(b) = level_bin(px[c]) {
                    parade[c][col * LEVELS + b] += 1;
                }
            }
            let (luma, cb, cr) = ycbcr(px[0], px[1], px[2]);
            if luma.is_nan() {
                nan[3] += 1;
                continue;
            }
            lumas.push(luma);
            histogram[((luma.clamp(0.0, 1.0) * 255.0).round()) as usize] += 1;
            if let Some(b) = level_bin(luma) {
                waveform[col * LEVELS + b] += 1;
            }
            if let Some((bx, by)) = vector_bin(cb, cr) {
                vectorscope[by * VECTOR + bx] += 1;
            }
            let chroma = cb.hypot(cr);
            if chroma > chroma_max {
                chroma_max = chroma;
                chroma_hue = cr.atan2(cb).to_degrees();
            }
        }
    }
    let [r, g, b] = channels;
    let (mut r, mut g, mut b) = (r, g, b);
    let stats = Stats {
        width,
        height,
        red: channel_stats(&mut r, nan[0]),
        green: channel_stats(&mut g, nan[1]),
        blue: channel_stats(&mut b, nan[2]),
        luma: channel_stats(&mut lumas, nan[3]),
        chroma_max,
        chroma_max_hue: chroma_hue,
        luma_histogram: histogram,
    };
    Scopes {
        columns,
        waveform,
        parade,
        vectorscope,
        stats,
    }
}

/// Log-scaled intensity of a bin count relative to the largest.
fn intensity(count: u32, max: u32) -> f64 {
    if count == 0 || max == 0 {
        0.0
    } else {
        (f64::from(count).ln_1p() / f64::from(max).ln_1p()).clamp(0.0, 1.0)
    }
}

fn put(img: &mut [u8], w: usize, x: usize, y: usize, rgb: [u8; 3]) {
    let i = (y * w + x) * 3;
    img[i..i + 3].copy_from_slice(&rgb);
}

/// A waveform as an RGB image (`tint` the trace color).
fn waveform_image(columns: usize, bins: &[u32], tint: [f64; 3]) -> Vec<u8> {
    let mut img = vec![0u8; columns * LEVELS * 3];
    let max = bins.iter().copied().max().unwrap_or(0);
    // Graticule at 0, 0.25, 0.5, 0.75, 1.
    for g in [0.0, 0.25, 0.5, 0.75, 1.0] {
        if let Some(b) = level_bin(g) {
            for x in 0..columns {
                put(&mut img, columns, x, LEVELS - 1 - b, [48, 48, 48]);
            }
        }
    }
    for x in 0..columns {
        for b in 0..LEVELS {
            let v = intensity(bins[x * LEVELS + b], max);
            if v > 0.0 {
                let c = tint.map(|t| (64.0 + 191.0 * v * t).round() as u8);
                put(&mut img, columns, x, LEVELS - 1 - b, c);
            }
        }
    }
    img
}

/// The vectorscope as an RGB image with graticule and targets.
fn vectorscope_image(bins: &[u32]) -> Vec<u8> {
    let mut img = vec![0u8; VECTOR * VECTOR * 3];
    let max = bins.iter().copied().max().unwrap_or(0);
    // The legal circle (chroma 0.5) and the axes.
    for k in 0..2048 {
        let a = f64::from(k) / 2048.0 * std::f64::consts::TAU;
        if let Some((x, y)) = vector_bin(0.5 * a.cos(), 0.5 * a.sin()) {
            put(&mut img, VECTOR, x, y, [48, 48, 48]);
        }
    }
    for k in 0..VECTOR {
        put(&mut img, VECTOR, k, VECTOR / 2, [32, 32, 32]);
        put(&mut img, VECTOR, VECTOR / 2, k, [32, 32, 32]);
    }
    // The skin-tone line (about 123° hue).
    for k in 0..256 {
        let r = f64::from(k) / 256.0 * 0.5;
        let a = 123f64.to_radians();
        if let Some((x, y)) = vector_bin(r * a.cos(), r * a.sin()) {
            put(&mut img, VECTOR, x, y, [70, 50, 40]);
        }
    }
    // Color-bar targets at 75% and 100%.
    for level in [0.75, 1.0] {
        for (r, g, b) in [
            (1.0, 0.0, 0.0),
            (1.0, 0.0, 1.0),
            (0.0, 0.0, 1.0),
            (0.0, 1.0, 1.0),
            (0.0, 1.0, 0.0),
            (1.0, 1.0, 0.0),
        ] {
            let (_, cb, cr) = ycbcr(r * level, g * level, b * level);
            if let Some((x, y)) = vector_bin(cb, cr) {
                for d in -4i64..=4 {
                    for (dx, dy) in [(d, -4), (d, 4), (-4, d), (4, d)] {
                        let (px, py) = (x as i64 + dx, y as i64 + dy);
                        if (0..VECTOR as i64).contains(&px) && (0..VECTOR as i64).contains(&py) {
                            put(&mut img, VECTOR, px as usize, py as usize, [110, 90, 30]);
                        }
                    }
                }
            }
        }
    }
    for y in 0..VECTOR {
        for x in 0..VECTOR {
            let v = intensity(bins[y * VECTOR + x], max);
            if v > 0.0 {
                let c = (64.0 + 191.0 * v).round() as u8;
                put(&mut img, VECTOR, x, y, [c, c, c]);
            }
        }
    }
    img
}

fn write_rgb(path: &Path, width: usize, height: usize, rgb: &[u8]) -> Result<(), String> {
    let file = std::fs::File::create(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut e = png::Encoder::new(std::io::BufWriter::new(file), width as u32, height as u32);
    e.set_color(png::ColorType::Rgb);
    e.set_depth(png::BitDepth::Eight);
    e.write_header()
        .and_then(|mut w| w.write_image_data(rgb))
        .map_err(|e| format!("{}: {e}", path.display()))
}

/// Writes the images and `scope.json` into `dir`.
pub fn write(scopes: &Scopes, source: &str, dir: &Path) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let c = scopes.columns;
    write_rgb(
        &dir.join("waveform.png"),
        c,
        LEVELS,
        &waveform_image(c, &scopes.waveform, [1.0, 1.0, 1.0]),
    )?;
    let tints = [[1.0, 0.3, 0.3], [0.3, 1.0, 0.3], [0.4, 0.5, 1.0]];
    let panels: Vec<Vec<u8>> = (0..3)
        .map(|k| waveform_image(c, &scopes.parade[k], tints[k]))
        .collect();
    let mut parade = vec![0u8; 3 * c * LEVELS * 3];
    for y in 0..LEVELS {
        for (k, panel) in panels.iter().enumerate() {
            let from = y * c * 3;
            let to = (y * 3 * c + k * c) * 3;
            parade[to..to + c * 3].copy_from_slice(&panel[from..from + c * 3]);
        }
    }
    write_rgb(&dir.join("parade.png"), 3 * c, LEVELS, &parade)?;
    write_rgb(
        &dir.join("vectorscope.png"),
        VECTOR,
        VECTOR,
        &vectorscope_image(&scopes.vectorscope),
    )?;
    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct Report<'a> {
        format: &'static str,
        version: u32,
        source: &'a str,
        encoding: &'static str,
        coefficients: &'static str,
        waveform: (usize, usize, f64, f64),
        vectorscope: (usize, f64),
        stats: &'a Stats,
    }
    let report = Report {
        format: "powermove-scope",
        version: 1,
        source,
        encoding: "working values: premultiplied, display encoding E, Rec.709 primaries, unclamped",
        coefficients: "BT.709 luma and color difference on E-encoded values, full range",
        waveform: (c, LEVELS, LEVEL_LO, LEVEL_HI),
        vectorscope: (VECTOR, VECTOR_RANGE),
        stats: &scopes.stats,
    };
    let json = serde_json::to_string_pretty(&report).map_err(|e| e.to_string())?;
    std::fs::write(dir.join("scope.json"), json + "\n")
        .map_err(|e| format!("{}: {e}", dir.display()))
}

/// A captured frame: a corpus `.rgba32f` (its JSON header beside it) or an
/// 8-bit RGBA PNG (values / 255, as stored).
pub fn read_frame(path: &Path) -> Result<(u32, u32, Vec<f32>), String> {
    let name = path.to_string_lossy();
    if name.ends_with(".rgba32f") {
        let header = std::fs::read_to_string(format!("{name}.json"))
            .map_err(|e| format!("{name}.json: {e}"))?;
        let header: serde_json::Value =
            serde_json::from_str(&header).map_err(|e| format!("{name}.json: {e}"))?;
        let dim = |k: &str| {
            header[k]
                .as_u64()
                .and_then(|v| u32::try_from(v).ok())
                .ok_or_else(|| format!("{name}.json: no {k}"))
        };
        let (w, h) = (dim("width")?, dim("height")?);
        let bytes = std::fs::read(path).map_err(|e| format!("{name}: {e}"))?;
        if bytes.len() != (w * h * 16) as usize {
            return Err(format!("{name}: {} bytes for {w}×{h}", bytes.len()));
        }
        let px = bytes
            .chunks_exact(4)
            .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
            .collect();
        return Ok((w, h, px));
    }
    let file = std::fs::File::open(path).map_err(|e| format!("{name}: {e}"))?;
    let mut reader = png::Decoder::new(std::io::BufReader::new(file))
        .read_info()
        .map_err(|e| format!("{name}: {e}"))?;
    let mut buf = vec![0; reader.output_buffer_size().unwrap_or(0)];
    let info = reader
        .next_frame(&mut buf)
        .map_err(|e| format!("{name}: {e}"))?;
    if info.bit_depth != png::BitDepth::Eight {
        return Err(format!("{name}: only 8-bit PNG frames are read"));
    }
    let channels = match info.color_type {
        png::ColorType::Rgba => 4,
        png::ColorType::Rgb => 3,
        other => return Err(format!("{name}: {other:?} PNG frames are not read")),
    };
    let mut px = Vec::with_capacity((info.width * info.height * 4) as usize);
    for p in buf[..info.buffer_size()].chunks_exact(channels) {
        let a = if channels == 4 {
            f32::from(p[3]) / 255.0
        } else {
            1.0
        };
        // Premultiply straight PNG values (opaque frames are unchanged).
        px.extend_from_slice(&[
            f32::from(p[0]) / 255.0 * a,
            f32::from(p[1]) / 255.0 * a,
            f32::from(p[2]) / 255.0 * a,
            a,
        ]);
    }
    Ok((info.width, info.height, px))
}

/// `pm-cli scope`: renders (or reads) the frame and writes its scopes.
pub fn run(options: &Options) -> Result<Stats, crate::render::Failure> {
    use crate::render::Failure;
    let (width, height, rgba, source) = if let Some(frame) = &options.frame {
        let (w, h, px) = read_frame(frame).map_err(Failure::Failed)?;
        (w, h, px, format!("frame {}", frame.display()))
    } else {
        let input = options
            .input
            .as_ref()
            .ok_or_else(|| Failure::Usage("a project or --frame".into()))?;
        let loaded =
            crate::render::load_project(input, options.composition.as_deref(), &options.media)?;
        let doc = &*loaded.document;
        let (cw, ch) = pm_export::render::composition_size(doc, &loaded.composition)
            .map_err(|e| Failure::Failed(e.to_string()))?;
        let size = match (options.size, options.scale) {
            (Some((w, h)), _) => Size::new(w, h),
            (None, Some(s)) => Size::new(
                ((f64::from(cw) * s).round() as u32).max(1),
                ((f64::from(ch) * s).round() as u32).max(1),
            ),
            (None, None) => Size::new(cw, ch),
        };
        let mut request = RenderRequest::sized(size, Precision::Float32);
        request.background = !options.transparent;
        request.export = true;
        request.motion_blur = match options.motion_blur.as_deref() {
            None | Some("export") => MotionBlur::Kind(pm_protocol::frame::MotionBlurKind::Export),
            Some("off") => MotionBlur::Off,
            Some(n) => MotionBlur::Samples(
                n.parse()
                    .map_err(|_| Failure::Usage(format!("--motion-blur {n:?}")))?,
            ),
        };
        let mut renderer = Renderer::headless().map_err(|e| Failure::Failed(e.to_string()))?;
        // Never write into the desktop's probe namespace (crate::angle).
        renderer.read_only_shader_reference();
        crate::fonts::install(
            &mut renderer,
            options.ui_scale.unwrap_or(crate::fonts::DEFAULT_UI_SCALE),
        );
        renderer.set_svg_rasterizer(pm_render::vector::svg_rasters::SvgRasterizer::Resvg);
        let issues = renderer.unsupported_with_media(doc, &loaded.composition, &loaded.media);
        if !issues.is_empty() {
            return Err(Failure::Unsupported(
                issues.iter().map(ToString::to_string).collect(),
            ));
        }
        let ticks = Ticks::new(
            pm_time::ticks_from_seconds(options.time)
                .map_err(|e| Failure::Usage(format!("--time: {e}")))?,
        )
        .map_err(|_| Failure::Usage("--time is out of range".into()))?;
        let frame = renderer
            .render(doc, &loaded.composition, ticks, &request, &loaded.media)
            .map_err(|e| match e {
                pm_render::RenderError::Unsupported(u) => {
                    Failure::Unsupported(u.iter().map(ToString::to_string).collect())
                }
                other => Failure::Failed(other.to_string()),
            })?;
        if renderer.svg_approximated() > 0 {
            eprintln!(
                "{}",
                crate::render::approximate_svg_notice(renderer.svg_approximated())
            );
        }
        if !frame.failures.is_empty() {
            return Err(Failure::Failed(format!(
                "layer failures: {:?}",
                frame.failures
            )));
        }
        let rgba = renderer
            .read_rgba(&frame)
            .map_err(|e| Failure::Failed(e.to_string()))?;
        (
            size.width,
            size.height,
            rgba,
            format!(
                "{} composition {} at {} s, {}×{}, 32-bit",
                input.display(),
                loaded.composition,
                options.time,
                size.width,
                size.height
            ),
        )
    };
    let columns = options.columns.unwrap_or(720) as usize;
    let scopes = analyze(&rgba, width, height, columns);
    write(&scopes, &source, &options.output).map_err(Failure::Failed)?;
    Ok(scopes.stats)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(w: u32, h: u32, f: impl Fn(u32, u32) -> [f32; 3]) -> Vec<f32> {
        let mut v = Vec::new();
        for y in 0..h {
            for x in 0..w {
                let c = f(x, y);
                v.extend_from_slice(&[c[0], c[1], c[2], 1.0]);
            }
        }
        v
    }

    #[test]
    fn color_bars_land_on_their_targets() {
        let bars: [[f32; 3]; 7] = [
            [1.0, 1.0, 1.0],
            [1.0, 1.0, 0.0],
            [0.0, 1.0, 1.0],
            [0.0, 1.0, 0.0],
            [1.0, 0.0, 1.0],
            [1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0],
        ];
        let px = frame(70, 10, |x, _| bars[(x / 10) as usize].map(|v| v * 0.75));
        let s = analyze(&px, 70, 10, 70);
        for bar in bars {
            let [r, g, b] = bar.map(|v| f64::from(v) * 0.75);
            let (y, cb, cr) = ycbcr(r, g, b);
            let (bx, by) = vector_bin(cb, cr).unwrap();
            assert_eq!(s.vectorscope[by * VECTOR + bx], 100, "{bar:?}");
            // Its luma in its own columns of the waveform.
            let col = bars.iter().position(|b| *b == bar).unwrap() * 10;
            assert_eq!(s.waveform[col * LEVELS + level_bin(y).unwrap()], 10);
        }
        // Red at 75%: Cr = 0.75 · 0.5.
        let (_, _, cr) = ycbcr(0.75, 0.0, 0.0);
        assert!((cr - 0.375).abs() < 1e-12);
        let expected = bars
            .iter()
            .map(|b| {
                let (_, cb, cr) = ycbcr(
                    f64::from(b[0] * 0.75),
                    f64::from(b[1] * 0.75),
                    f64::from(b[2] * 0.75),
                );
                cb.hypot(cr)
            })
            .fold(0.0, f64::max);
        assert!((s.stats.chroma_max - expected).abs() < 1e-6);
    }

    #[test]
    fn a_ramp_draws_the_diagonal_and_out_of_range_values_count() {
        let w = 101;
        let px = frame(w, 4, |x, _| {
            let v = x as f32 / 100.0 * 1.2 - 0.1;
            [v, v, v]
        });
        let s = analyze(&px, w, 4, 1000);
        assert_eq!(s.columns, w as usize);
        for x in 0..w as usize {
            let v = f64::from(x as f32 / 100.0 * 1.2 - 0.1);
            let b = level_bin(ycbcr(v, v, v).0).unwrap();
            assert_eq!(s.waveform[x * LEVELS + b], 4, "column {x}");
        }
        // −0.1 + 0.012·x: columns 0..=8 below 0, 92..=100 above 1.
        assert_eq!(s.stats.luma.below_zero, 4 * 9);
        assert_eq!(s.stats.luma.above_one, 4 * 9);
        assert!((s.stats.luma.min + 0.1).abs() < 1e-6);
        assert!((s.stats.luma.max - 1.1).abs() < 1e-6);
        // Grey: no chroma.
        assert!(s.stats.chroma_max < 1e-6);
        // Every pixel in the bins around the center (it lies on a bin edge).
        let c = VECTOR / 2;
        let around: u32 = [(c - 1, c - 1), (c - 1, c), (c, c - 1), (c, c)]
            .iter()
            .map(|(y, x)| s.vectorscope[y * VECTOR + x])
            .sum();
        assert_eq!(around, 4 * w);
    }

    #[test]
    fn columns_bin_and_images_write() {
        let px = frame(1000, 2, |x, _| [x as f32 / 999.0, 0.5, 0.2]);
        let s = analyze(&px, 1000, 2, 250);
        assert_eq!(s.columns, 250);
        assert_eq!(s.waveform.iter().map(|v| *v as usize).sum::<usize>(), 2000);
        let dir = std::env::temp_dir().join(format!("pm-cli-scope-test-{}", std::process::id()));
        write(&s, "test", &dir).unwrap();
        for f in [
            "waveform.png",
            "parade.png",
            "vectorscope.png",
            "scope.json",
        ] {
            assert!(dir.join(f).is_file(), "{f}");
        }
        let json: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(dir.join("scope.json")).unwrap())
                .unwrap();
        assert_eq!(json["format"], "powermove-scope");
        assert_eq!(json["stats"]["width"], 1000);
        let _ = std::fs::remove_dir_all(dir);
    }
}
