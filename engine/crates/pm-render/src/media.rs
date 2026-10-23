//! Media sources into working textures (plan sections 9, 10, 13).
//!
//! Images come from `pm-media`'s readers at their native depth; video
//! frames from the decode service, selected by section 5 rule 7. Both reach
//! the GPU without copies through `pm-gpu`'s decode-in import, are
//! normalized there (section 13), and are converted here by the source pass
//! (`shaders/source.wgsl`): chroma upsampling, matrix, transfer and
//! primaries into the working encoding, and premultiplication.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use pm_color::alpha::{shader_code, source_plan};
use pm_color::chroma::{Factors, Filter, filters, sample_origin};
use pm_color::ycbcr::Matrix;
use pm_gpu::decode_in::deinterlace::{CodeSpace, Deinterlacer, FieldPass};
use pm_gpu::decode_in::{DecodeImporter, Normalized};
use pm_gpu::{Target, working_format};
use pm_media::decode::scrub::{ScrubStats, ViewerDecode, Want};
use pm_media::decode::service::{
    DecodeService, MediaLease, MediaSource, MediaUsage, ServiceConfig,
};
use pm_media::image::icc::IccClass;
use pm_media::interpret::{Interpretation, ToneMap, resolve_video};
use pm_media::pool::FramePool;
use pm_media::probe::color::HdrFormat;
use pm_media::probe::{FieldOrder, VideoProbe};
use pm_media::vt::DecodePurpose;
use pm_protocol::color::{
    AlphaMode, ChromaSiting, ColorDescription, MatrixCoefficients, Precision, Primaries, Transfer,
    WorkingEncoding,
};

use crate::capability::Feature;
use crate::footage::{Geometry, Orient, Pick, Timing};
use crate::pipelines::{Pipelines, SourceUniform, bytes_of};
pub use crate::stills::StillReader;

#[path = "media/hdr_compat.rs"]
mod hdr_compat;
#[path = "media/prewarm.rs"]
mod prewarm;
#[path = "media/video_compat.rs"]
mod video_compat;

/// Why a media file cannot be drawn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProbeIssue {
    Unsupported(Feature),
    Unreadable(String),
}

/// What the source pass needs to know about a source.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SourceColor {
    pub transfer: Transfer,
    pub primaries: Primaries,
    pub matrix: MatrixCoefficients,
    pub siting: ChromaSiting,
    pub alpha: AlphaMode,
    /// HDR input tone mapping (PQ and HLG sources in Standard, section 9).
    pub tone_map: Option<ToneMap>,
}

pub(crate) fn stamp(path: &Path) -> Option<(u64, u128)> {
    // Within a frame, one look at each file (`StampFrame`).
    FRAME_STAMPS.with(|f| {
        let mut f = f.borrow_mut();
        let FrameStamps {
            frame,
            active,
            seen,
        } = &mut *f;
        if !*active {
            return stat_stamp(path);
        }
        if let Some((at, found)) = seen.get_mut(path) {
            if *at != *frame {
                *at = *frame;
                *found = stat_stamp(path);
            }
            return *found;
        }
        let found = stat_stamp(path);
        // Bounded: a project with more files than this looks again.
        if seen.len() >= 4096 {
            seen.clear();
        }
        seen.insert(path.to_path_buf(), (*frame, found));
        found
    })
}

/// The files a thread's frames have looked at (`StampFrame`): kept across
/// frames so a frame allocates nothing for a file seen before, and valid
/// only for the frame that read them.
#[derive(Default)]
struct FrameStamps {
    frame: u64,
    active: bool,
    seen: HashMap<PathBuf, (u64, Option<(u64, u128)>)>,
}

thread_local! {
    static FRAME_STAMPS: std::cell::RefCell<FrameStamps> = std::cell::RefCell::new(FrameStamps::default());
}

/// While alive, [`stamp`] on this thread reads each file's size and
/// modification time once (docs/engine/perfbench.md): a frame placing one
/// image in many layers asked the file system once per layer and per probe.
/// A frame then sees one state of every file; the next frame looks again.
pub(crate) struct StampFrame {
    outer: bool,
}

impl StampFrame {
    pub(crate) fn begin() -> StampFrame {
        let outer = FRAME_STAMPS.with(|f| {
            let mut f = f.borrow_mut();
            let outer = !f.active;
            if outer {
                f.active = true;
                f.frame += 1;
            }
            outer
        });
        StampFrame { outer }
    }
}

impl Drop for StampFrame {
    fn drop(&mut self) {
        if self.outer {
            FRAME_STAMPS.with(|f| f.borrow_mut().active = false);
        }
    }
}

fn stat_stamp(path: &Path) -> Option<(u64, u128)> {
    pm_perf::bump(pm_perf::Counter::FileStats);
    let m = std::fs::metadata(path).ok()?;
    let modified = m
        .modified()
        .ok()?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_nanos();
    #[cfg(unix)]
    let modified = {
        use std::hash::{Hash, Hasher};
        use std::os::unix::fs::MetadataExt;
        // Keep replacement and same-size edits observable even if a caller
        // restores mtime. Frame-local memoization still issues one stat.
        let mut version = std::collections::hash_map::DefaultHasher::new();
        (m.dev(), m.ino(), m.ctime(), m.ctime_nsec()).hash(&mut version);
        (u128::from(version.finish()) << 64) | (modified & u128::from(u64::MAX))
    };
    Some((m.len(), modified))
}

/// The color reading of a still, or why it cannot be read.
pub fn image_color(still: &pm_media::image::StillImage) -> Result<SourceColor, ProbeIssue> {
    let c = &still.color;
    let primaries = match c.icc_class {
        None | Some(IccClass::Srgb) | Some(IccClass::GraySrgb) => Primaries::Bt709,
        Some(IccClass::DisplayP3) => Primaries::DisplayP3,
        Some(IccClass::Other) => {
            return Err(ProbeIssue::Unsupported(Feature::Media(
                "an ICC profile other than sRGB or Display P3".into(),
            )));
        }
        // CMYK, Lab, and indexed stills ImageIO converted to sRGB: never
        // compared with the legacy renderer's conversion.
        Some(IccClass::ConvertedToSrgb) => {
            return Err(ProbeIssue::Unsupported(Feature::Media(
                "stills in a color model other than RGB or gray (converted to sRGB by ImageIO)"
                    .into(),
            )));
        }
    };
    if let Some(ch) = c.chromaticities {
        let rec709 = [0.64f32, 0.33, 0.30, 0.60, 0.15, 0.06, 0.3127, 0.3290];
        if ch.iter().zip(rec709).any(|(a, b)| (a - b).abs() > 1e-4) {
            return Err(ProbeIssue::Unsupported(Feature::Media(
                "OpenEXR chromaticities other than Rec.709".into(),
            )));
        }
    }
    if matches!(c.transfer, Transfer::Pq | Transfer::Hlg) {
        return Err(ProbeIssue::Unsupported(Feature::Media("HDR stills".into())));
    }
    Ok(SourceColor {
        transfer: c.transfer,
        primaries,
        matrix: MatrixCoefficients::Identity,
        siting: ChromaSiting::Center,
        alpha: c.alpha,
        tone_map: None,
    })
}

/// Answers about a file kept per path, size, and modification time:
/// capability routing asks at every document revision, and decoding a still
/// or opening and indexing a video must not repeat each time.
type StampCache<T> = OnceLock<Mutex<HashMap<(PathBuf, u64, u128), T>>>;

/// `compute`'s answer for `path` as it is on disk, computed once per size
/// and modification time; None when the file is offline.
fn per_stamp<T: Clone>(
    cache: &'static StampCache<T>,
    path: &Path,
    compute: impl FnOnce() -> T,
) -> Option<T> {
    let (len, modified) = stamp(path)?;
    let key = (path.to_path_buf(), len, modified);
    let map = cache.get_or_init(Default::default);
    if let Some(v) = map.lock().unwrap().get(&key) {
        return Some(v.clone());
    }
    let v = compute();
    map.lock().unwrap().insert(key, v.clone());
    Some(v)
}

fn offline(path: &Path) -> String {
    format!("{} is offline", path.display())
}

/// What the interpreted probe of a still reads: the reader's color facts
/// and whether the samples are float, or why the file cannot be drawn
/// (unreadable, or animated). Independent of the interpretation, so one
/// decode serves every edit of the asset's overrides.
fn still_facts(path: &Path) -> Result<(pm_media::image::ImageColor, bool), ProbeIssue> {
    static CACHE: StampCache<Result<(pm_media::image::ImageColor, bool), ProbeIssue>> =
        OnceLock::new();
    per_stamp(&CACHE, path, || {
        if let Ok(Some(a)) = pm_media::image::animation(path)
            && a.frames > 1
        {
            return Err(ProbeIssue::Unsupported(Feature::ImageSequence));
        }
        let pool = FramePool::new(0);
        let still = pm_media::image::read(path, &pool)
            .map_err(|e| ProbeIssue::Unreadable(format!("{}: {e}", path.display())))?;
        Ok((still.color, still.float))
    })
    .unwrap_or_else(|| Err(ProbeIssue::Unreadable(offline(path))))
}

/// [`probe_image`] for an asset with Interpret Footage overrides (only this
/// engine reads them, so there is no legacy look to match): readable, not
/// animated, and a reading the renderer converts (every ICC profile, through
/// ColorSync where it is neither sRGB nor Display P3; Rec.709 OpenEXR).
///
/// The file is decoded once per size and modification time
/// ([`still_facts`]); each interpretation is then resolved from its facts.
pub fn probe_interpreted_image(path: &Path, interp: &Interpretation) -> Result<(), ProbeIssue> {
    let (color, float) = still_facts(path)?;
    // A PNG gamma is the file's transfer; only a transfer override replaces
    // it. Interpretations of it are not implemented.
    if interp.transfer.is_none()
        && pm_media::image::png_gamma::imageio_gamma(path)
            .map_err(|e| ProbeIssue::Unreadable(format!("{}: {e}", path.display())))?
            .is_some()
    {
        return Err(ProbeIssue::Unsupported(Feature::Media(
            "PNG gamma (gAMA) with Interpret Footage overrides".into(),
        )));
    }
    interpreted_color(&color, float, interp).map(|_| ())
}

/// How the renderer reads a still with Interpret Footage overrides.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct InterpretedStill {
    /// The source pass's reading of the texture it receives.
    pub color: SourceColor,
    /// ColorSync converts the stored samples first, reading their alpha
    /// this way ([`pm_media::image::imageio::convert_to_linear_srgb`]); the
    /// texture is then premultiplied linear Rec.709, as `color` says.
    pub colorsync: Option<AlphaMode>,
}

/// A still's reading with Interpret Footage overrides
/// ([`pm_media::interpret::interpret_still`], which the sheet shows): every
/// override replaces the file's reading of that value. A profile that is
/// neither sRGB nor Display P3 is converted by ColorSync unless a primaries
/// or transfer override sets it aside; alpha overrides are applied by that
/// conversion, never on top of it.
pub fn interpreted_image_color(
    still: &pm_media::image::StillImage,
    interp: &Interpretation,
) -> Result<InterpretedStill, ProbeIssue> {
    interpreted_color(&still.color, still.float, interp)
}

fn interpreted_color(
    color: &pm_media::image::ImageColor,
    float: bool,
    interp: &Interpretation,
) -> Result<InterpretedStill, ProbeIssue> {
    let si = pm_media::interpret::interpret_still(color, float, interp);
    let r = &si.reading;
    if si.colorsync {
        return Ok(InterpretedStill {
            color: SourceColor {
                transfer: Transfer::Linear,
                primaries: Primaries::Bt709,
                matrix: MatrixCoefficients::Identity,
                siting: ChromaSiting::Center,
                alpha: match r.alpha.value {
                    AlphaMode::Opaque => AlphaMode::Opaque,
                    _ => AlphaMode::Premultiplied,
                },
                tone_map: None,
            },
            colorsync: Some(r.alpha.value),
        });
    }
    if interp.primaries.is_none()
        && let Some(ch) = color.chromaticities
    {
        let rec709 = [0.64f32, 0.33, 0.30, 0.60, 0.15, 0.06, 0.3127, 0.3290];
        if ch.iter().zip(rec709).any(|(a, b)| (a - b).abs() > 1e-4) {
            return Err(ProbeIssue::Unsupported(Feature::Media(
                "OpenEXR chromaticities other than Rec.709".into(),
            )));
        }
    }
    // A PQ or HLG reading, from the file or an override: the tone map
    // assumes BT.2020 and has no still test, so HDR stills stay with the
    // legacy renderer (a non-HDR transfer override reads one as SDR).
    if matches!(r.transfer.value, Transfer::Pq | Transfer::Hlg) {
        return Err(ProbeIssue::Unsupported(Feature::Media("HDR stills".into())));
    }
    Ok(InterpretedStill {
        color: SourceColor {
            transfer: r.transfer.value,
            primaries: r.primaries.value,
            matrix: MatrixCoefficients::Identity,
            siting: ChromaSiting::Center,
            alpha: r.alpha.value,
            tone_map: r.tone_map,
        },
        colorsync: None,
    })
}

/// Keep native-depth sources unchanged. At level 1, a file Chromium can
/// decode must not depend on ImageIO accepting the same AVIF container.
/// TIFF reads the shipped FFmpeg PNG rendition before compatibility color conversion.
fn image_source(
    path: &Path,
    pool: &FramePool,
    compat: bool,
) -> pm_media::Result<pm_media::image::StillImage> {
    if compat {
        use std::io::Read;
        let mut head = [0; 16];
        let n = std::fs::File::open(path)?.read(&mut head)?;
        if pm_media::image::sniff(&head[..n]) == pm_media::image::ImageFormat::Tiff {
            return pm_media::image::compat::read_tiff_source(path, pool);
        }
    }
    pm_media::image::read(path, pool).or_else(|error| {
        if compat {
            use std::io::Read;
            let mut head = [0; 512];
            let n = std::fs::File::open(path)?.read(&mut head)?;
            if pm_media::image::sniff(&head[..n]) == pm_media::image::ImageFormat::Avif {
                return pm_media::image::avif_compat::read_bitmap(path, pool);
            }
        }
        Err(error)
    })
}

/// The still samples a layer draws: Chromium's bitmap at level 1, the
/// native-depth source at level 2.
pub fn still_reader(level1: bool) -> StillReader {
    if level1 {
        StillReader::Chromium
    } else {
        StillReader::Native
    }
}

/// Whether an image file can be drawn by a level 1 layer (Chromium's
/// compatibility bitmap): [`probe_still`] with [`StillReader::Chromium`].
pub fn probe_image(path: &Path) -> Result<(), ProbeIssue> {
    probe_still(path, StillReader::Chromium)
}

/// Whether an image file can be drawn with `reader`'s samples: readable,
/// not animated, in a tested still class ([`crate::stills`]), a color
/// reading the engine converts. Level 1 layers read Chromium's bitmap;
/// level 2 layers and 3D maps read the native-depth source, whose classes
/// are narrower (it reads neither PNG cICP/cHRM nor HDR transfers, and
/// ImageIO converts CMYK and other models to sRGB).
pub fn probe_still(path: &Path, reader: StillReader) -> Result<(), ProbeIssue> {
    static CHROMIUM: StampCache<Result<(), ProbeIssue>> = OnceLock::new();
    static NATIVE: StampCache<Result<(), ProbeIssue>> = OnceLock::new();
    let cache = match reader {
        StillReader::Chromium => &CHROMIUM,
        StillReader::Native => &NATIVE,
    };
    per_stamp(cache, path, || {
        if let Ok(Some(a)) = pm_media::image::animation(path)
            && a.frames > 1
        {
            return Err(ProbeIssue::Unsupported(Feature::ImageSequence));
        }
        let pool = FramePool::new(0);
        let chromium = reader == StillReader::Chromium;
        let still = image_source(path, &pool, chromium)
            .map_err(|e| ProbeIssue::Unreadable(format!("{}: {e}", path.display())))?;
        if !chromium {
            crate::stills::check(path, &still, reader).map_err(ProbeIssue::Unsupported)?;
            // The level 2 source pass converts profiles other than sRGB and
            // Display P3 through ColorSync (`MediaCache::image`); every other
            // reading must be one it draws.
            if still.color.icc_class == Some(IccClass::Other) {
                return Ok(());
            }
            return image_color(&still).map(|_| ());
        }
        // Chromium cannot show an AVIF its decoder cannot parse, whatever
        // ImageIO reads: a named failure, not a class.
        if pm_media::image::avif_compat::applies(&still) {
            pm_media::image::avif_compat::unmeasured(path).map_err(|e| {
                ProbeIssue::Unreadable(format!(
                    "{}: Chromium's AVIF decoder cannot parse it ({e})",
                    path.display()
                ))
            })?;
        }
        crate::stills::check(path, &still, reader).map_err(ProbeIssue::Unsupported)?;
        if pm_media::image::avif_compat::applies(&still) {
            let bitmap = pm_media::image::avif_compat::read(path, &still, &pool)
                .map_err(|e| ProbeIssue::Unreadable(e.to_string()))?;
            image_color(&bitmap).map(|_| ())
        } else if pm_media::image::compat::applies(&still) {
            let bitmap = pm_media::image::compat::bitmap_at(path, &still, &pool)
                .map_err(|e| ProbeIssue::Unreadable(e.to_string()))?;
            image_color(&bitmap).map(|_| ())
        } else {
            image_color(&still).map(|_| ())
        }
    })
    .unwrap_or_else(|| Err(ProbeIssue::Unreadable(offline(path))))
}

/// Today's 8-bit Chromium bitmap of a still: the `ImageBitmap` the
/// importer keeps (`createImageBitmap(blob)`), which level 1 image layers
/// draw and today's 3D runtime hands three.js as a material map
/// (`core/scene3d/runtime.ts` `texture()`). Its rows are oriented, top row
/// first, in the bitmap's sRGB encoding, premultiplied as Chromium stores
/// them. The second value is true when today's RGBA8 upload of it fails
/// ([`pm_media::image::compat::upload_fails`]; the rows are then image
/// layers' incomplete texture).
pub fn level1_bitmap(
    path: &Path,
    pool: &FramePool,
) -> Result<(pm_media::image::StillImage, bool), ProbeIssue> {
    probe_still(path, StillReader::Chromium)?;
    let unreadable =
        |e: pm_media::MediaError| ProbeIssue::Unreadable(format!("{}: {e}", path.display()));
    let still = image_source(path, pool, true).map_err(unreadable)?;
    let fails = pm_media::image::compat::upload_fails(path, &still).map_err(unreadable)?;
    level1_bitmap_of(path, still, pool).map(|b| (b, fails))
}

fn level1_bitmap_of(
    path: &Path,
    still: pm_media::image::StillImage,
    pool: &FramePool,
) -> Result<pm_media::image::StillImage, ProbeIssue> {
    let unreadable =
        |e: pm_media::MediaError| ProbeIssue::Unreadable(format!("{}: {e}", path.display()));
    if pm_media::image::avif_compat::applies(&still) {
        pm_media::image::avif_compat::read(path, &still, pool).map_err(unreadable)
    } else if pm_media::image::compat::applies(&still) {
        pm_media::image::compat::bitmap_at(path, &still, pool).map_err(unreadable)
    } else {
        Err(ProbeIssue::Unsupported(Feature::Media(format!(
            "{} stills without Chromium's 8-bit bitmap",
            still.frame.format.label
        ))))
    }
}

/// The shipped `sequencePlaybackTime`: select an available CFR slot,
/// clamp to its ends, then seek one millisecond inside it. In particular,
/// sequences do not use ordinary video's 40 ms end padding.
pub fn sequence_playback_time(record: &pm_model::Node, time: f64) -> Option<f64> {
    use pm_eval::node;
    let sequence = node::member(Some(record), "imageSequence")?;
    let number =
        |name| node::member(Some(sequence), name).map(|v| node::js_value_opt(Some(v)).to_number());
    let (fps, frames) = (number("fps")?, number("frames")?);
    if !fps.is_finite()
        || !(1.0..=240.0).contains(&fps)
        || !frames.is_finite()
        || frames.fract() != 0.0
        || !(2.0..=9_007_199_254_740_991.0).contains(&frames)
    {
        return None;
    }
    let index = crate::js::max(
        0.0,
        crate::js::min(frames - 1.0, ((time + 1e-7) * fps).floor()),
    );
    Some(index / fps + 0.001)
}

/// Numbered/animated imports today persist a WebM rendition, even when
/// their document format is GIF or PNG. An EBML header identifies the
/// rendition independently of path extensions and original metadata.
/// The native video probe validates readability and indexability before
/// a valid rendition can use the video path.
pub fn sequence_rendition(path: &Path) -> Result<bool, String> {
    static CACHE: StampCache<Result<bool, String>> = OnceLock::new();
    per_stamp(&CACHE, path, || {
        use std::io::Read;
        let mut magic = [0; 4];
        std::fs::File::open(path)
            .and_then(|mut f| f.read_exact(&mut magic))
            .map_err(|e| format!("{}: {e}", path.display()))?;
        if magic == [0x1a, 0x45, 0xdf, 0xa3] {
            MediaSource::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
            return Ok(true);
        }
        // A readable original animation retains its parity gate. Broken
        // bytes are a named failure, not an unsupported sequence.
        pm_media::image::read(path, &FramePool::new(0))
            .map_err(|e| format!("{}: {e}", path.display()))?;
        Ok(false)
    })
    .unwrap_or_else(|| Err(offline(path)))
}

/// Whether a video file is drawn without routing: it opens and indexes,
/// the renderer can draw it ([`video_drawable`]), and it shows nothing
/// whose look the renderer has not matched with today's viewer (section 25:
/// a feature leaves `pm_render::unsupported` only with a parity test).
/// Rec.709, BT.601 and the measured BT.2020 H.264 upload are matched;
/// these other readings are drawn only when the
/// asset carries Interpret Footage overrides, which only this engine reads:
///
/// - HDR outside the captured default AV1/HEVC PQ/HLG/P3 classes (including
///   metadata-driven gain curves); native Interpret uses BT.2446 Method A;
/// - interlaced sources (bwdif, equal to FFmpeg's; today's viewer shows the
///   woven frame);
/// - display rotations, non-square pixels, and a cropping clean aperture
///   (exact, but not compared with today's viewer);
/// - other wide-gamut SDR footage. The measured BT.2020 H.264 class clips
///   and quantizes at today's upload boundary before Canvas gamut conversion.
///
/// Answers are kept per file, size, and modification time: capability
/// routing asks at every document revision, and opening a source indexes
/// it.
pub fn probe_video(path: &Path) -> Result<(), String> {
    probe_video_issue(path).map_err(|issue| match issue {
        ProbeIssue::Unreadable(message) => message,
        ProbeIssue::Unsupported(Feature::Media(message)) => message,
        ProbeIssue::Unsupported(feature) => feature.to_string(),
    })
}

/// Typed video probe: corrupt/offline media is a media failure, not a
/// capability reason that could send a project to another renderer.
pub fn probe_video_issue(path: &Path) -> Result<(), ProbeIssue> {
    static CACHE: StampCache<Result<(), ProbeIssue>> = OnceLock::new();
    per_stamp(&CACHE, path, || {
        let src = MediaSource::open(path)
            .map_err(|e| ProbeIssue::Unreadable(format!("{}: {e}", path.display())))?;
        video_supported(&src).map_err(|m| ProbeIssue::Unsupported(Feature::Media(m)))
    })
    .unwrap_or_else(|| Err(ProbeIssue::Unreadable(offline(path))))
}

/// [`probe_video`] for an asset with Interpret Footage overrides.
pub fn probe_interpreted_video(path: &Path, interp: &Interpretation) -> Result<(), String> {
    probe_interpreted_video_issue(path, interp).map_err(|issue| match issue {
        ProbeIssue::Unreadable(message) => message,
        ProbeIssue::Unsupported(Feature::Media(message)) => message,
        ProbeIssue::Unsupported(feature) => feature.to_string(),
    })
}

/// Typed interpreted probe. Facts are kept per source stamp, independently
/// of changing interpretation overrides.
pub fn probe_interpreted_video_issue(
    path: &Path,
    interp: &Interpretation,
) -> Result<(), ProbeIssue> {
    static CACHE: StampCache<Result<Option<VideoProbe>, String>> = OnceLock::new();
    let video = per_stamp(&CACHE, path, || {
        MediaSource::open(path)
            .map(|src| src.probe.video.clone())
            .map_err(|e| format!("{}: {e}", path.display()))
    })
    .unwrap_or_else(|| Err(offline(path)))
    .map_err(ProbeIssue::Unreadable)?;
    video_probe_drawable(video.as_ref(), interp)
        .map_err(|m| ProbeIssue::Unsupported(Feature::Media(m)))
}

/// Measured Chromium default Canvas tone-map class. Metadata-driven gain
/// curves and Dolby Vision retain their explicit parity gate.
fn chromium_hdr_signal(video: &VideoProbe) -> Option<hdr_compat::Signal> {
    if !matches!(video.codec.codec.as_str(), "av1" | "hevc")
        || video.codec.bit_depth != 10
        || (video.codec.log2_chroma_w, video.codec.log2_chroma_h) != (1, 1)
        || video.codec.full_range
        || video.codec.alpha_mode
        || video.codec.pix_fmt_alpha
        || video.color.mastering.is_some()
        || video.color.content_light.is_some()
        || matches!(video.color.hdr, Some(HdrFormat::DolbyVision { .. }))
        || !matches!(
            video.color.interpreted.primaries,
            Primaries::Bt2020 | Primaries::DisplayP3
        )
    {
        return None;
    }
    match video.color.interpreted.transfer {
        Transfer::Pq => Some(hdr_compat::Signal::Pq),
        Transfer::Hlg if video.color.interpreted.primaries == Primaries::Bt2020 => {
            Some(hdr_compat::Signal::Hlg)
        }
        _ => None,
    }
}

fn video_supported(src: &MediaSource) -> Result<(), String> {
    video_drawable(src, &Interpretation::default())?;
    let Some(v) = src.probe.video.as_ref() else {
        return Err("no video stream".into());
    };
    const WHEN: &str = "drawn when interpreted";
    if matches!(
        v.fields.order,
        FieldOrder::TopFirst | FieldOrder::BottomFirst
    ) {
        return Err(format!(
            "interlaced video (not compared with today's viewer; {WHEN})"
        ));
    }
    if v.rotation
        .is_some_and(|r| (0.01..359.99).contains(&r.rem_euclid(360.0)))
    {
        return Err(format!(
            "rotated video (not compared with today's viewer; {WHEN})"
        ));
    }
    if v.pixel_aspect.0 != v.pixel_aspect.1 {
        return Err(format!(
            "non-square pixels (not compared with today's viewer; {WHEN})"
        ));
    }
    if let Some(clap) = v.clean_aperture
        && (clap.width != f64::from(v.width)
            || clap.height != f64::from(v.height)
            || clap.horizontal_offset != 0.0
            || clap.vertical_offset != 0.0)
    {
        return Err(format!(
            "a cropping clean aperture (not compared with today's viewer; {WHEN})"
        ));
    }
    let c = &src.color;
    if matches!(c.transfer, Transfer::Pq | Transfer::Hlg) && chromium_hdr_signal(v).is_none() {
        return Err(format!(
            "HDR video (Chromium tone-map parity remains unresolved; {WHEN})"
        ));
    }
    // Rec.709 is the identity; BT.601 primaries read as Rec.709 in Standard
    // (`standard_video_primaries`).
    if !matches!(
        c.primaries,
        Primaries::Bt709 | Primaries::Bt601_525 | Primaries::Bt601_625
    ) && !video_compat::sdr_gamut_upload(v)
        && chromium_hdr_signal(v).is_none()
    {
        return Err(format!(
            "{:?} primaries (wide-gamut footage is {WHEN})",
            c.primaries
        ));
    }
    Ok(())
}

/// Whether the renderer can draw a video source as interpreted: every
/// reading except display rotations off the quarter turns and Dolby Vision
/// without a base layer other decoders can show.
pub fn video_drawable(src: &MediaSource, interp: &Interpretation) -> Result<(), String> {
    video_probe_drawable(src.probe.video.as_ref(), interp)
}

fn video_probe_drawable(video: Option<&VideoProbe>, interp: &Interpretation) -> Result<(), String> {
    let Some(v) = video else {
        return Err("no video stream".into());
    };
    let r = v.rotation.unwrap_or(0.0);
    if ((r / 90.0).round() * 90.0 - r).abs() > 0.01 {
        return Err(format!("a display rotation of {r}°"));
    }
    if let Some(HdrFormat::DolbyVision {
        base_compatibility: 0,
        profile,
        ..
    }) = v.color.hdr
    {
        return Err(format!(
            "Dolby Vision profile {profile} (no base layer other decoders show)"
        ));
    }
    let reading = resolve_video(v, interp);
    if reading.primaries.value == Primaries::Unspecified {
        return Err("unknown color primaries".into());
    }
    Ok(())
}

/// The primaries a video source is read with in Standard. BT.601 525- and
/// 625-line primaries (section 9's reading of untagged SD, and SD tags)
/// pass through as Rec.709: Standard keeps today's look of footage exactly
/// (section 9, "The rule"), and today's viewer (Chromium) does not convert
/// them (measured on the e2e fixtures: a BT.601 red decodes to code 254, 0,
/// 0 there; converting SMPTE C primaries in linear light would show 248, 36,
/// 0). Converting them is an Interpret option for later phases.
pub fn standard_video_primaries(p: Primaries) -> Primaries {
    match p {
        Primaries::Bt601_525 | Primaries::Bt601_625 | Primaries::Unspecified => Primaries::Bt709,
        other => other,
    }
}

/// Whether Interpret Footage overrides how a video's code values become
/// colour: primaries, transfer, matrix, range or chroma siting. Frame rate,
/// field order, pixel aspect and clean aperture only choose or place frames,
/// and an alpha choice only acts on a file with an alpha channel, so none of
/// them may move a level 1 video off today's compatibility upload.
fn interprets_video_color(i: &Interpretation) -> bool {
    i.primaries.is_some()
        || i.transfer.is_some()
        || i.matrix.is_some()
        || i.range.is_some()
        || i.siting.is_some()
}

/// Whether the overrides change the video's colour description (its
/// colour members or its alpha reading). Without them the frame's own tags
/// apply, exactly as for an uninterpreted asset.
fn interprets_video_reading(i: &Interpretation) -> bool {
    interprets_video_color(i) || i.alpha.is_some()
}

/// The source pass parameters for a source.
fn source_uniform(
    color: &SourceColor,
    working: WorkingEncoding,
    precision: Precision,
    chroma: Option<(Factors, u32, u32)>,
) -> SourceUniform {
    // HDR signals are tone mapped (section 9); without a tone map (a still
    // reaching here) they read as E, as refused stills never do.
    let tone = color
        .tone_map
        .filter(|t| matches!(t.transfer, Transfer::Pq | Transfer::Hlg));
    let transfer = match (color.transfer, tone) {
        (_, Some(t)) if t.transfer == Transfer::Pq => 5,
        (_, Some(_)) => 6,
        (Transfer::DisplayE, _) => 0,
        (Transfer::Bt1886, _) => 1,
        (Transfer::SceneReferredBt709, _) => 2,
        (Transfer::Linear, _) => 3,
        (Transfer::Gamma22, _) => 4,
        (Transfer::Pq | Transfer::Hlg, None) => 0,
    };
    let primaries = if tone.is_some() {
        // Method A works in BT.2020; its SDR result converts to Rec.709.
        pm_color::tonemap::bt2020_to_709()
    } else {
        pm_color::primaries::conversion(
            if color.primaries == Primaries::Unspecified {
                Primaries::Bt709
            } else {
                color.primaries
            },
            Primaries::Bt709,
        )
        .unwrap_or([[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]])
    };
    let convert_primaries = tone.is_none()
        && color.primaries != Primaries::Bt709
        && color.primaries != Primaries::Unspecified;
    let identity = transfer == 0 && working == WorkingEncoding::Standard && !convert_primaries;
    let plan = source_plan(color.alpha, identity);
    let matte = match color.alpha {
        AlphaMode::PremultipliedWithColor { matte } => {
            [matte[0] as f32, matte[1] as f32, matte[2] as f32, 0.0]
        }
        _ => [0.0; 4],
    };
    let mut u = SourceUniform {
        a: [
            u32::from(chroma.is_some()),
            transfer,
            u32::from(working == WorkingEncoding::LinearLight),
            shader_code(plan),
        ],
        c: [
            0,
            0,
            u32::from(precision == Precision::Float16),
            u32::from(convert_primaries),
        ],
        matte,
        hdr: [
            tone.map_or(0.0, |t| t.peak_nits as f32),
            if color.matrix == MatrixCoefficients::Bt2020Cl {
                1.0
            } else {
                0.0
            },
            0.0,
            0.0,
        ],
        ..Default::default()
    };
    // Primaries columns (the matrix is row-major).
    for (j, column) in u.p.iter_mut().enumerate() {
        *column = [
            primaries[0][j] as f32,
            primaries[1][j] as f32,
            primaries[2][j] as f32,
            0.0,
        ];
    }
    if let Some((factors, cw, ch)) = chroma {
        let (fh, fv) = filters(factors);
        let code = |f: Filter| match f {
            Filter::None => 0,
            Filter::Linear => 1,
            Filter::CatmullRom => 2,
        };
        let (ox, oy) = sample_origin(color.siting, factors);
        u.b = [factors.x, factors.y, code(fh), code(fv)];
        u.c[0] = cw;
        u.c[1] = ch;
        u.origin = [ox as f32, oy as f32, 0.0, 0.0];
        let m = Matrix::of(color.matrix).unwrap_or(Matrix::BT709);
        for (column, c) in u.m.iter_mut().zip(m.to_rgb_columns()) {
            *column = [c[0], c[1], c[2], 0.0];
        }
    }
    u
}

/// Records the source pass from normalized textures into a new working
/// texture of the source's size.
#[allow(clippy::too_many_arguments)]
pub(crate) fn convert(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    pipelines: &Pipelines,
    encoder: &mut wgpu::CommandEncoder,
    normalized: &Normalized,
    color: &SourceColor,
    working: WorkingEncoding,
    precision: Precision,
    factors: Factors,
    reuse: Option<Target>,
) -> Target {
    convert_format(
        device,
        queue,
        pipelines,
        encoder,
        normalized,
        color,
        working,
        precision,
        factors,
        reuse,
        working_format(precision),
        false,
    )
}

/// Source storage is independent of the composition's working precision.
/// Today's opaque level 1 video upload clips and quantizes to RGBA8 before
/// the layer's bilinear filter; native and interpreted sources keep floats.
#[allow(clippy::too_many_arguments)]
fn convert_format(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    pipelines: &Pipelines,
    encoder: &mut wgpu::CommandEncoder,
    normalized: &Normalized,
    color: &SourceColor,
    working: WorkingEncoding,
    precision: Precision,
    factors: Factors,
    reuse: Option<Target>,
    format: wgpu::TextureFormat,
    compatibility_p010: bool,
) -> Target {
    let (luma, chroma, chroma_info) = match normalized {
        Normalized::Rgb { rgba } => (rgba, None, None),
        Normalized::YCbCr { luma, chroma } => (
            luma,
            Some(chroma),
            Some((factors, chroma.width(), chroma.height())),
        ),
    };
    let (w, h) = (luma.width(), luma.height());
    let out = reuse
        .filter(|t| t.width() == w && t.height() == h && t.format() == format)
        .unwrap_or_else(|| {
            Target::new(
                device,
                "pm source",
                w,
                h,
                format,
                pm_gpu::target::working_usage(),
            )
        });
    let mut uniform = source_uniform(color, working, precision, chroma_info);
    if format == wgpu::TextureFormat::Rgba8Unorm {
        // Store the f32 conversion directly: an intermediate half-float
        // rounding can change which side of an 8-bit boundary it lands on.
        uniform.c[2] = 0;
        uniform.hdr[3] = u32::from(color.primaries == Primaries::Bt2020) as f32;
        if let Some((factors, _, _)) = chroma_info {
            // Chromium's Skia video upload treats subsampled chroma as
            // centered and reconstructs it bilinearly before the RGBA8
            // store. Keep the native float source's siting/filter policy.
            uniform.b[2] = u32::from(factors.x > 1);
            uniform.b[3] = u32::from(factors.y > 1);
            uniform.origin[0] = (factors.x - 1) as f32 * 0.5;
            uniform.origin[1] = (factors.y - 1) as f32 * 0.5;
        }
        if compatibility_p010 {
            // Chromium samples MSB-aligned P010 R16Unorm planes with
            // Skia's 8-bit matrix. Re-express that source upload after
            // native 10-bit normalization, keeping the decoder untouched.
            let scale = 65280.0f64 / 65535.0;
            uniform.origin[2] = scale as f32;
            uniform.origin[3] = ((128.0 / 224.0) * (scale - 1.0)) as f32;
            uniform.hdr[2] = ((16.0 / 219.0) * (scale - 1.0)) as f32;
        }
    }
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("pm source params"),
        size: size_of::<SourceUniform>() as u64,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    queue.write_buffer(&buffer, 0, bytes_of(&uniform));
    let luma_view = luma.create_view(&Default::default());
    // A 1×1 stand-in when there is no chroma plane.
    let dummy;
    let chroma_view = match chroma {
        Some(c) => c.create_view(&Default::default()),
        None => {
            dummy = Target::new(
                device,
                "pm no chroma",
                1,
                1,
                wgpu::TextureFormat::Rg32Float,
                wgpu::TextureUsages::TEXTURE_BINDING,
            );
            dummy.view.clone()
        }
    };
    let layout = pipelines.source_layout(format);
    let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("pm source"),
        layout: &layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&luma_view),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::TextureView(&chroma_view),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 3,
                resource: wgpu::BindingResource::TextureView(&out.view),
            },
        ],
    });
    let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
        label: Some("pm source"),
        timestamp_writes: None,
    });
    pass.set_pipeline(pipelines.source(format));
    pass.set_bind_group(0, &bind, &[]);
    pass.dispatch_workgroups(w.div_ceil(8), h.div_ceil(8), 1);
    drop(pass);
    out
}

/// An 8-bit RGBA still (Blink's compatibility bitmap) as an `Rgba8Unorm`
/// texture, or `None` when its pixels are not one 8-bit RGBA plane. The
/// texture is for sampling (and copying out) only: it is not a working
/// target of any precision (see [`MediaCache::image`]).
fn rgba8_bitmap(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    still: &pm_media::image::StillImage,
) -> Option<Target> {
    let (w, h) = (still.frame.format.width, still.frame.format.height);
    let rgba = still
        .frame
        .with_planes(|planes| {
            let p = planes.first()?;
            if planes.len() != 1 || p.desc.format != pm_media::frame::PlaneFormat::Rgba8 {
                return None;
            }
            let mut out = Vec::with_capacity(w * h * 4);
            for y in 0..h {
                // SAFETY: the plane holds `height` rows of `stride` bytes,
                // each with `width` RGBA8 texels.
                out.extend_from_slice(unsafe {
                    std::slice::from_raw_parts(p.ptr.add(y * p.stride), w * 4)
                });
            }
            Some(out)
        })
        .ok()??;
    let (w, h) = (u32::try_from(w).ok()?, u32::try_from(h).ok()?);
    let target = Target::new(
        device,
        "pm still bitmap",
        w,
        h,
        wgpu::TextureFormat::Rgba8Unorm,
        wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::COPY_DST
            | wgpu::TextureUsages::COPY_SRC,
    );
    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture: &target.texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        &rgba,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(w * 4),
            rows_per_image: Some(h),
        },
        wgpu::Extent3d {
            width: w,
            height: h,
            depth_or_array_layers: 1,
        },
    );
    debug_assert!(
        target.format() == wgpu::TextureFormat::Rgba8Unorm
            && !target.texture.usage().intersects(
                wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::STORAGE_BINDING
            ),
        "the still bitmap is sample-only"
    );
    Some(target)
}

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
struct ImageKey {
    path: PathBuf,
    len: u64,
    modified: u128,
    precision: u8,
    /// The interpretation it was converted with (empty: none).
    reading: String,
    /// Converted textures hold working values: linear light differs.
    linear: bool,
    compat: bool,
}

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
struct FrameKey {
    path: PathBuf,
    frame: usize,
    /// The second field (field-rate deinterlacing).
    second: bool,
    precision: u8,
    export: bool,
    /// The interpretation the frame was converted with (a digest).
    reading: u64,
    linear: bool,
}

/// The frame cache of a renderer without a lease share: export jobs,
/// headless renders, and the viewer before the host grants a lease.
pub const DEFAULT_FRAME_CACHE: usize = 64 << 20;

/// Media state kept across frames: decoded and converted stills, video
/// sources and sessions, and the last converted frame of each source.
pub struct MediaCache {
    pool: FramePool,
    service: Arc<DecodeService>,
    background: Option<prewarm::BackgroundPrepare>,
    /// Display frames decode on workers (docs/engine/scrub.md); export
    /// frames decode here, on the caller's thread.
    viewer: ViewerDecode,
    /// Scrub frames: a video whose exact frame is not decoded yet shows
    /// the nearest decoded frame while it decodes ([`Want::Scrub`]).
    pub scrub: bool,
    /// Paused/scrub/final frames use today's Canvas video snapshot; live
    /// playback uploads directly. Both readings partition the frame cache.
    pub canvas_video: bool,
    /// The frame (presented index in its source) of each video layer drawn
    /// with a stand-in since the list was reset.
    pub stand_ins: Vec<usize>,
    /// Converted video frames replaced during the current render, and those
    /// replaced earlier (free to overwrite: every read of them is
    /// submitted). A UHD frame converts into 66 MB (`rgba16float`).
    retired_frames: Vec<Target>,
    free_frames: Vec<Target>,
    importer: Option<DecodeImporter>,
    images: BTreeMap<ImageKey, Target>,
    /// Open video sources with the file's size and modification time when
    /// opened: a file replaced at the same path is reopened.
    sources: BTreeMap<PathBuf, ((u64, u128), Arc<MediaSource>)>,
    frames: BTreeMap<FrameKey, Target>,
    /// "Interpret Footage" overrides by file (section 9, per-source
    /// interpretation): the color reading replaces the probe's.
    interpretations: BTreeMap<PathBuf, ColorDescription>,
    /// bwdif on normalized planes (interlaced sources), made on first use.
    deinterlacer: Option<Deinterlacer>,
    video_upload: Option<video_compat::Upload>,
    hdr_canvas: Option<hdr_compat::ToneMap>,
    /// The display geometry pass (clean aperture, rotation), made on first use.
    orient: Option<Orient>,
    /// Bytes of converted textures held (stills and frames).
    pub bytes: u64,
}

/// A media failure while preparing a frame.
#[derive(Debug, Clone)]
pub enum MediaFailure {
    Unsupported(Feature),
    Failed(String),
}

impl MediaCache {
    pub fn new(device: &wgpu::Device) -> MediaCache {
        // Every renderer starts with a small frame cache (export jobs and
        // headless renders read their frames once). The viewer's renderer
        // takes its share of the engine's lease through
        // [`MediaCache::apply_lease`] (docs/engine/scrub.md, "Memory").
        let service = Arc::new(DecodeService::new(ServiceConfig {
            cache_bytes: DEFAULT_FRAME_CACHE,
            ..ServiceConfig::default()
        }));
        MediaCache {
            pool: FramePool::new(64 << 20),
            viewer: ViewerDecode::new(service.clone()),
            service: service.clone(),
            background: None,
            scrub: false,
            canvas_video: true,
            stand_ins: Vec::new(),
            retired_frames: Vec::new(),
            free_frames: Vec::new(),
            importer: DecodeImporter::new(device).ok(),
            images: BTreeMap::new(),
            sources: BTreeMap::new(),
            frames: BTreeMap::new(),
            interpretations: BTreeMap::new(),
            deinterlacer: None,
            video_upload: None,
            hdr_canvas: None,
            orient: None,
            bytes: 0,
        }
    }

    fn importer(&self) -> Result<&DecodeImporter, MediaFailure> {
        self.importer
            .as_ref()
            .ok_or_else(|| MediaFailure::Failed("the device cannot import decoded frames".into()))
    }

    /// The working texture of a still, read with the asset's Interpret
    /// Footage overrides when it has any.
    ///
    /// A `compat` level 1 still in the Standard encoding comes back as
    /// today's 8-bit bitmap ([`rgba8_bitmap`]): an `Rgba8Unorm` texture
    /// whatever `precision`, usable only as a sampled texture or a copy
    /// source (no `RENDER_ATTACHMENT` or `STORAGE_BINDING`). Every consumer
    /// samples it; one that copies it into a working target or renders into
    /// it must convert it first (the formats and usages differ and wgpu
    /// rejects it).
    #[allow(clippy::too_many_arguments)]
    pub fn image(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        pipelines: &Pipelines,
        encoder: &mut wgpu::CommandEncoder,
        path: &Path,
        working: WorkingEncoding,
        precision: Precision,
        interpret: Option<&Interpretation>,
        compat: bool,
    ) -> Result<Target, MediaFailure> {
        let (len, modified) = stamp(path)
            .ok_or_else(|| MediaFailure::Failed(format!("{} is offline", path.display())))?;
        let key = ImageKey {
            path: path.to_path_buf(),
            len,
            modified,
            precision: precision as u8,
            reading: interpret.map(|i| format!("{i:?}")).unwrap_or_default(),
            linear: working == WorkingEncoding::LinearLight,
            compat,
        };
        if let Some(t) = self.images.get(&key) {
            pm_perf::bump(pm_perf::Counter::MediaCacheHits);
            return Ok(t.clone());
        }
        pm_perf::bump(pm_perf::Counter::MediaCacheMisses);
        let issue = |i: ProbeIssue| match i {
            ProbeIssue::Unsupported(f) => MediaFailure::Unsupported(f),
            ProbeIssue::Unreadable(m) => MediaFailure::Failed(m),
        };
        match interpret {
            Some(i) => probe_interpreted_image(path, i),
            None => probe_still(path, still_reader(compat)),
        }
        .map_err(issue)?;
        let mut still = image_source(path, &self.pool, compat && interpret.is_none())
            .map_err(|e| MediaFailure::Failed(format!("{}: {e}", path.display())))?;
        let mut bitmap = false;
        if compat && interpret.is_none() && pm_media::image::avif_compat::applies(&still) {
            still = pm_media::image::avif_compat::read(path, &still, &self.pool)
                .map_err(|e| MediaFailure::Failed(format!("{}: {e}", path.display())))?;
            bitmap = true;
        } else if compat && interpret.is_none() && pm_media::image::compat::applies(&still) {
            still = pm_media::image::compat::bitmap_at(path, &still, &self.pool)
                .map_err(|e| MediaFailure::Failed(format!("{}: {e}", path.display())))?;
            bitmap = true;
        }
        // Today's image layers sample Blink's 8-bit premultiplied bitmap as
        // an RGBA8 texture (`texImage2D` with `UNPACK_PREMULTIPLY_ALPHA`).
        // The GPU's bilinear filter weighs 8-bit texels differently from
        // half float ones, so the compatibility bitmap is sampled as it is
        // (in the display encoding; linear light converts it below).
        if bitmap
            && working == WorkingEncoding::Standard
            && let Some(target) = rgba8_bitmap(device, queue, &still)
        {
            self.images.retain(|k, _| {
                k.path != key.path || k.precision != key.precision || k.linear != key.linear
            });
            self.images.insert(key, target.clone());
            self.recount();
            return Ok(target);
        }
        let color = match interpret {
            Some(i) => {
                let read = interpreted_image_color(&still, i).map_err(issue)?;
                if let Some(alpha) = read.colorsync {
                    still =
                        pm_media::image::imageio::convert_to_linear_srgb(&still, &self.pool, alpha)
                            .map_err(|e| {
                                MediaFailure::Failed(format!("{}: {e}", path.display()))
                            })?;
                }
                read.color
            }
            None => {
                // Level 2 reads a gamma-only PNG as ImageIO does; level 1's
                // bitmap above already applied Chromium's reading.
                if !compat
                    && still.format == pm_media::image::ImageFormat::Png
                    && let Some(gamma) = pm_media::image::png_gamma::imageio_gamma(path)
                        .map_err(|e| MediaFailure::Failed(format!("{}: {e}", path.display())))?
                {
                    still = pm_media::image::png_gamma::gamma_to_linear(&still, gamma, &self.pool)
                        .map_err(|e| MediaFailure::Failed(format!("{}: {e}", path.display())))?;
                }
                if still.color.icc_class == Some(IccClass::Other) {
                    still = pm_media::image::imageio::convert_to_linear_srgb(
                        &still,
                        &self.pool,
                        still.color.alpha,
                    )
                    .map_err(|e| MediaFailure::Failed(format!("{}: {e}", path.display())))?;
                }
                image_color(&still).map_err(issue)?
            }
        };
        let importer = self.importer()?;
        let imported = importer
            .import(&still.frame)
            .map_err(|e| MediaFailure::Failed(e.to_string()))?;
        let normalized = importer
            .normalize(queue, encoder, &imported)
            .map_err(|e| MediaFailure::Failed(e.to_string()))?;
        let target = convert(
            device,
            queue,
            pipelines,
            encoder,
            &normalized,
            &color,
            working,
            precision,
            Factors { x: 1, y: 1 },
            None,
        );
        // The decoded pixels must outlive the commands reading them; the
        // imported textures hold the frame until wgpu releases them.
        drop(imported);
        // Older versions of the same file are dropped.
        // (Dropped, not destroyed: a frame being recorded may still use it.)
        self.images.retain(|k, _| {
            k.path != key.path || k.precision != key.precision || k.linear != key.linear
        });
        self.images.insert(key, target.clone());
        self.recount();
        Ok(target)
    }

    fn source(&mut self, path: &Path) -> Result<Arc<MediaSource>, MediaFailure> {
        self.drain_prepared();
        let stamp = stamp(path)
            .ok_or_else(|| MediaFailure::Failed(format!("{} is offline", path.display())))?;
        match self.sources.get(path) {
            Some((s, src)) if *s == stamp => return Ok(src.clone()),
            Some(_) => {
                // Replaced on disk: drop the stale index and frames.
                if let Some((_, old)) = self.sources.remove(path) {
                    self.viewer.release(Some(old.id));
                    self.service.release(None, Some(old.id));
                }
                self.frames.retain(|k, _| k.path != path);
                self.recount();
            }
            None => {}
        }
        let src = MediaSource::open(path)
            .map_err(|e| MediaFailure::Failed(format!("{}: {e}", path.display())))?;
        self.sources
            .insert(path.to_path_buf(), (stamp, src.clone()));
        Ok(src)
    }

    /// The working texture of the video picture showing at `seconds` (source
    /// time), or `None` before the first frame: the frame section 5 rule 7
    /// selects (or the conformed rate's frame), deinterlaced to the field
    /// or frame rate the composition needs (section 13), converted with the
    /// source's reading (the probe and section 9's defaults, `interpret`'s
    /// overrides), tone mapped when HDR, and cropped and rotated for
    /// display.
    #[allow(clippy::too_many_arguments)]
    pub fn video(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        pipelines: &Pipelines,
        encoder: &mut wgpu::CommandEncoder,
        path: &Path,
        instance: u64,
        seconds: f64,
        export: bool,
        working: WorkingEncoding,
        precision: Precision,
        interpret: Option<&Interpretation>,
        composition: pm_time::Rate,
        sequence: bool,
        compat: bool,
    ) -> Result<Option<Target>, MediaFailure> {
        let src = self.source(path)?;
        let none = Interpretation::default();
        match interpret {
            Some(i) => video_drawable(&src, i),
            None => video_supported(&src),
        }
        .map_err(|m| MediaFailure::Unsupported(Feature::Media(m)))?;
        let interp = interpret.unwrap_or(&none);
        let v = src
            .probe
            .video
            .as_ref()
            .ok_or_else(|| MediaFailure::Failed("no video stream".into()))?;
        let reading = resolve_video(v, interp);
        let timing = Timing::new(&src, &reading, composition);
        let ticks = (seconds * pm_time::TICKS_PER_SECOND as f64).round() as i64;
        let Some(Pick { frame: n, second }) = timing.pick(&src, ticks) else {
            return Ok(None);
        };
        let reading_key = {
            use std::hash::{Hash, Hasher};
            let mut h = std::collections::hash_map::DefaultHasher::new();
            format!(
                "{interp:?}{composition:?}{sequence}{compat}{}",
                self.canvas_video
            )
            .hash(&mut h);
            self.interpretations
                .get(path)
                .map(|c| format!("{c:?}"))
                .hash(&mut h);
            h.finish()
        };
        let key_of = |frame: usize| FrameKey {
            path: path.to_path_buf(),
            frame,
            second,
            precision: precision as u8,
            export,
            reading: reading_key,
            linear: working == WorkingEncoding::LinearLight,
        };
        // The newest converted frame of the source (a paused redraw).
        if let Some(t) = self.frames.get(&key_of(n)) {
            pm_perf::bump(pm_perf::Counter::MediaCacheHits);
            return Ok(Some(t.clone()));
        }
        // Export frames, and deinterlaced frames (the current frame and its
        // neighbors: random access includes neighboring fields), decode on
        // this thread; other display frames come from the scrub workers.
        let deinterlaced = timing.deinterlaced();
        let decode_reading = self.decode_reading(
            v,
            interp,
            path,
            compat,
            sequence,
            deinterlaced,
            working,
            reading.color.transfer,
        );
        let (n, frame, fields) = if export || deinterlaced {
            let purpose = if export {
                DecodePurpose::Export
            } else {
                DecodePurpose::Display
            };
            let mut session = self
                .service
                .session_with_reading(&src, instance, purpose, decode_reading)
                .map_err(|e| MediaFailure::Failed(e.to_string()))?;
            if deinterlaced {
                let f = session
                    .fields(n)
                    .map_err(|e| MediaFailure::Failed(e.to_string()))?;
                (n, f.cur.clone(), Some(f))
            } else {
                let f = session
                    .frame(n)
                    .map_err(|e| MediaFailure::Failed(e.to_string()))?;
                (n, f, None)
            }
        } else {
            let want = if self.scrub { Want::Scrub } else { Want::Exact };
            let fetched = self
                .viewer
                .fetch_with_reading(&src, instance, n, want, decode_reading)
                .map_err(|e| MediaFailure::Failed(e.to_string()))?;
            if !fetched.exact {
                self.stand_ins.push(fetched.shown);
            }
            (fetched.shown, fetched.frame, None)
        };
        // A stand-in may be the frame converted last.
        let key = key_of(n);
        if let Some(t) = self.frames.get(&key) {
            pm_perf::bump(pm_perf::Counter::MediaCacheHits);
            return Ok(Some(t.clone()));
        }
        pm_perf::bump(pm_perf::Counter::MediaCacheMisses);
        let frame = if compat
            && !sequence
            && working == WorkingEncoding::Standard
            && v.codec.codec == "vp9"
            && frame.format.bits == 10
            && frame.format.layout == pm_media::frame::Layout::Planar
            && frame.format.alignment == pm_protocol::color::SampleAlignment::Lsb
            && !frame.format.has_alpha()
            && !interprets_video_color(interp)
            && !self.interpretations.contains_key(path)
            && fields.is_none()
            && !matches!(reading.color.transfer, Transfer::Pq | Transfer::Hlg)
        {
            pm_media::compat_video::aligned_planar_10(&frame, &self.pool)
                .map_err(|e| MediaFailure::Failed(e.to_string()))?
        } else {
            frame
        };
        let full_range_bitmap = compat
            && self.canvas_video
            && !sequence
            && working == WorkingEncoding::Standard
            && matches!(v.codec.codec.as_str(), "h264" | "vp9")
            && matches!(
                video_compat::color(v, reading.color).matrix,
                MatrixCoefficients::Bt601
                    | MatrixCoefficients::Bt709
                    | MatrixCoefficients::Bt2020Ncl
            )
            && !matches!(reading.color.transfer, Transfer::Pq | Transfer::Hlg)
            && frame.format.bits == 8
            && frame.format.range == pm_protocol::color::Range::Full
            && !frame.format.has_alpha()
            && !interprets_video_color(interp)
            && fields.is_none()
            && !self.interpretations.contains_key(path);
        let alpha_bitmap = compat
            && (self.canvas_video || v.codec.codec != "vp9")
            && !sequence
            && working == WorkingEncoding::Standard
            && frame.format.has_alpha()
            && !interprets_video_reading(interp)
            && !self.interpretations.contains_key(path)
            && fields.is_none()
            && !matches!(reading.color.transfer, Transfer::Pq | Transfer::Hlg);
        let bitmap = if full_range_bitmap {
            Some(
                pm_media::compat_video::full_range_bitmap(
                    &frame,
                    &self.pool,
                    video_compat::color(v, reading.color).matrix,
                )
                .map_err(|e| MediaFailure::Failed(e.to_string()))?,
            )
        } else if alpha_bitmap {
            pm_media::compat_video::alpha_bitmap(
                &frame,
                &self.pool,
                video_compat::color(v, reading.color).matrix,
            )
            .map_err(|e| MediaFailure::Failed(e.to_string()))?
        } else if sequence
            && compat
            && !interprets_video_reading(interp)
            && fields.is_none()
            && !self.interpretations.contains_key(path)
            && v.color.tagged.matrix.is_none()
        {
            pm_media::image::compat::video_bitmap(&frame, &self.pool)
                .map_err(|e| MediaFailure::Failed(e.to_string()))?
        } else {
            None
        };
        let source_bitmap = bitmap.is_some();
        let frame = bitmap.unwrap_or(frame);
        if self.deinterlacer.is_none() && fields.is_some() {
            self.deinterlacer = Some(Deinterlacer::new(device));
        }
        let imported = self
            .importer()?
            .import(&frame)
            .map_err(|e| MediaFailure::Failed(e.to_string()))?;
        let mut held = vec![];
        // The importer's normalizations to hand back once recorded.
        let mut recycled = vec![];
        let mut desc: ColorDescription = match self.interpretations.get(path) {
            Some(d) => *d,
            None if interprets_video_reading(interp) => reading.color,
            None => frame.color.unwrap_or(reading.color),
        };
        if compat
            && !sequence
            && working == WorkingEncoding::Standard
            && !frame.format.has_alpha()
            && !interprets_video_color(interp)
            && !self.interpretations.contains_key(path)
            && fields.is_none()
            && !matches!(desc.transfer, Transfer::Pq | Transfer::Hlg)
        {
            desc = video_compat::color(v, desc);
            if !self.canvas_video && video_compat::sdr_gamut_upload(v) {
                // texImage2D keeps encoded RGB; Canvas2D additionally
                // converts from the video's primaries into its sRGB canvas.
                desc.primaries = Primaries::Bt709;
            }
        }
        // raster.ts encodes image sequences as untagged BT.601 VP9, even
        // at HD sizes. Honor explicit file or Interpret Footage matrices.
        if sequence
            && v.color.tagged.matrix.is_none()
            && interp.matrix.is_none()
            && !self.interpretations.contains_key(path)
        {
            desc.matrix = pm_protocol::color::MatrixCoefficients::Bt601;
        }
        let color = SourceColor {
            transfer: desc.transfer,
            primaries: standard_video_primaries(desc.primaries),
            matrix: desc.matrix,
            siting: desc.siting,
            alpha: if frame.format.has_alpha() {
                match desc.alpha {
                    AlphaMode::Opaque if interp.alpha.is_none() => AlphaMode::Straight,
                    other => other,
                }
            } else {
                AlphaMode::Opaque
            },
            tone_map: match desc.transfer {
                Transfer::Pq | Transfer::Hlg => reading.tone_map.or(Some(ToneMap {
                    transfer: desc.transfer,
                    peak_nits: 1000.0,
                })),
                _ => None,
            },
        };
        let bitmap_gamut = full_range_bitmap && color.primaries == Primaries::Bt2020;
        let color = if source_bitmap {
            SourceColor {
                transfer: pm_protocol::color::Transfer::DisplayE,
                primaries: pm_protocol::color::Primaries::Bt709,
                matrix: pm_protocol::color::MatrixCoefficients::Identity,
                alpha: AlphaMode::Premultiplied,
                ..color
            }
        } else {
            color
        };
        let factors = Factors::from_log2(frame.format.log2_chroma_w, frame.format.log2_chroma_h);
        // texImage2D uploads ordinary opaque legacy video into RGBA8.
        // Clip/quantize each source texel before the layer samples it, so
        // saturated out-of-range RGB cannot bleed across a filtered edge.
        // The Canvas export and sequence paths keep their libyuv conversion;
        // live VP9/HEVC alpha samples the YUV and alpha planes before its store.
        // Level 2 and colour interpretations (Interpret Footage's
        // colour members or a file's colour override), and unmatched HDR
        // retain native precision. Overrides that only choose or place frames
        // (frame rate, field order, pixel aspect, clean aperture) keep the
        // upload, so they cannot change a level 1 video's colours.
        let compatibility_bitmap = compat
            && !sequence
            && working == WorkingEncoding::Standard
            && ((!frame.format.has_alpha() && color.alpha == AlphaMode::Opaque)
                || frame.format.label == "v0a8"
                || (!self.canvas_video
                    && v.codec.codec == "vp9"
                    && frame.format.bits == 8
                    && frame.format.has_alpha()))
            && !interprets_video_color(interp)
            && (!frame.format.has_alpha() || interp.alpha.is_none())
            && !self.interpretations.contains_key(path)
            && fields.is_none()
            && color.tone_map.is_none()
            && !matches!(color.transfer, Transfer::Pq | Transfer::Hlg)
            && (matches!(color.primaries, Primaries::Bt709 | Primaries::Unspecified)
                || video_compat::sdr_gamut_upload(v));
        let compatibility_hdr = compat
            && !sequence
            && working == WorkingEncoding::Standard
            && !frame.format.has_alpha()
            && !interprets_video_color(interp)
            && !self.interpretations.contains_key(path)
            && fields.is_none()
            && chromium_hdr_signal(v).is_some()
            && video_compat::Upload::supports(&imported, color.matrix);
        let format = if compatibility_bitmap
            || compatibility_hdr
            || full_range_bitmap
            || (alpha_bitmap && source_bitmap)
        {
            wgpu::TextureFormat::Rgba8Unorm
        } else {
            working_format(precision)
        };
        // A converted frame retired by an earlier render (its reads are
        // submitted) is overwritten instead of allocating another.
        let (w, h) = (frame.format.width as u32, frame.format.height as u32);
        let reuse = self
            .free_frames
            .iter()
            .position(|t| t.width() == w && t.height() == h && t.format() == format)
            .map(|i| self.free_frames.swap_remove(i));
        let converted = if compatibility_hdr {
            let source = self
                .video_upload
                .get_or_insert_with(|| video_compat::Upload::new(device))
                .run(
                    device,
                    encoder,
                    &imported,
                    color.matrix,
                    video_compat::UploadReading {
                        gamut: None,
                        separate_matrix: !self.canvas_video,
                        canvas_source_bias_first: self.canvas_video,
                    },
                    if self.canvas_video {
                        None
                    } else {
                        reuse.clone()
                    },
                );
            if self.canvas_video {
                self.hdr_canvas
                    .get_or_insert_with(|| hdr_compat::ToneMap::new(device))
                    .run(
                        device,
                        encoder,
                        &source,
                        chromium_hdr_signal(v).expect("checked HDR reading"),
                        color.primaries,
                        reuse,
                    )
                    .ok_or_else(|| {
                        MediaFailure::Failed("invalid Chromium HDR source texture".into())
                    })?
            } else {
                source
            }
        } else if bitmap_gamut
            || (compatibility_bitmap && video_compat::Upload::supports(&imported, color.matrix))
        {
            let upload = self
                .video_upload
                .get_or_insert_with(|| video_compat::Upload::new(device));
            upload.run(
                device,
                encoder,
                &imported,
                color.matrix,
                video_compat::UploadReading {
                    gamut: if bitmap_gamut {
                        Some(Primaries::Bt2020)
                    } else {
                        matches!(color.primaries, Primaries::Bt2020 | Primaries::DisplayP3)
                            .then_some(color.primaries)
                    },
                    separate_matrix: !self.canvas_video
                        && v.codec.codec == "vp9"
                        && frame.format.bits == 10,
                    canvas_source_bias_first: false,
                },
                reuse,
            )
        } else {
            let importer = self.importer()?;
            let import = |f: &pm_media::frame::DecodedFrame| {
                importer
                    .import(f)
                    .map_err(|e| MediaFailure::Failed(e.to_string()))
            };
            let normalized = match &fields {
                Some(f) => {
                    let prev = import(&f.prev)?;
                    let next = import(&f.next)?;
                    let norm = |i: &pm_gpu::decode_in::ImportedFrame,
                                e: &mut wgpu::CommandEncoder| {
                        importer
                            .normalize(queue, e, i)
                            .map_err(|e| MediaFailure::Failed(e.to_string()))
                    };
                    let (np, nc, nn) = (
                        norm(&prev, encoder)?,
                        norm(&imported, encoder)?,
                        norm(&next, encoder)?,
                    );
                    held.push(prev);
                    held.push(next);
                    let pass =
                        FieldPass::new(timing.fields.top_field_first(), second, f.first, f.last);
                    let out = self
                        .deinterlacer
                        .as_ref()
                        .expect("made above")
                        .run(
                            queue,
                            encoder,
                            &np,
                            &nc,
                            &nn,
                            pass,
                            &CodeSpace::of(&frame.format),
                        )
                        .map_err(|e| MediaFailure::Failed(e.to_string()))?;
                    recycled.extend([np, nc, nn]);
                    out
                }
                None => importer
                    .normalize(queue, encoder, &imported)
                    .map_err(|e| MediaFailure::Failed(e.to_string()))?,
            };
            let converted = convert_format(
                device,
                queue,
                pipelines,
                encoder,
                &normalized,
                &color,
                working,
                precision,
                factors,
                reuse,
                format,
                compatibility_bitmap && v.codec.codec == "hevc" && frame.format.label == "x420",
            );
            let importer = self.importer()?;
            if fields.is_none() {
                importer.recycle(normalized);
            }
            for n in recycled {
                importer.recycle(n);
            }
            converted
        };
        let geometry = Geometry::of(&reading, (converted.width(), converted.height()));
        let target = if geometry.is_identity() {
            converted
        } else {
            let orient = self.orient.get_or_insert_with(|| Orient::new(device));
            orient.run(device, queue, encoder, &converted, geometry, precision)
        };
        drop(imported);
        drop(held);
        // Keep only the newest frame per source and precision. The older
        // one may still be read by this render: it is reused from the next.
        let mut retired = Vec::new();
        self.frames.retain(|k, t| {
            let keep = k.path != key.path
                || k.precision != key.precision
                || k.export != key.export
                || k.linear != key.linear;
            if !keep {
                retired.push(t.clone());
            }
            keep
        });
        self.retired_frames.extend(retired);
        self.frames.insert(key, target.clone());
        self.recount();
        Ok(Some(target))
    }

    /// Reads `path` with `color` instead of the probe's interpretation (or
    /// restores the probe's with `None`). Converted frames of the file are
    /// dropped.
    pub fn interpret(&mut self, path: &Path, color: Option<ColorDescription>) {
        match color {
            Some(c) => {
                self.interpretations.insert(path.to_path_buf(), c);
            }
            None => {
                self.interpretations.remove(path);
            }
        }
        self.frames.retain(|k, _| k.path != path);
        self.recount();
    }

    /// The coded size of a video file's frames.
    pub fn video_size(&mut self, path: &Path) -> Result<(u32, u32), MediaFailure> {
        let src = self.source(path)?;
        let v = src
            .probe
            .video
            .as_ref()
            .ok_or_else(|| MediaFailure::Failed("no video stream".into()))?;
        Ok((v.width, v.height))
    }

    /// The color reading of a video file: an override, or the probe's.
    pub fn video_color(&mut self, path: &Path) -> Result<ColorDescription, MediaFailure> {
        if let Some(c) = self.interpretations.get(path) {
            return Ok(*c);
        }
        Ok(self.source(path)?.color)
    }

    /// Bytes of converted textures held.
    fn recount(&mut self) {
        self.bytes = self
            .images
            .values()
            .chain(self.frames.values())
            .chain(&self.retired_frames)
            .chain(&self.free_frames)
            .map(Target::bytes)
            .sum();
    }

    /// Changes whenever a display frame a scrub asked for has decoded.
    pub fn landed(&self) -> u64 {
        self.viewer.landed()
    }

    /// The display decode workers' counters.
    pub fn scrub_stats(&self) -> ScrubStats {
        self.viewer.stats()
    }

    /// Applies the renderer's shares of the engine's lease to its decode
    /// service: the frame cache evicts down to `lease.frame_cache` at once
    /// and idle decoders close past `lease.decoders`. Returns what the
    /// service holds afterwards.
    pub fn apply_lease(&self, lease: MediaLease) -> MediaUsage {
        self.service.apply_lease(lease)
    }

    /// What the decode service holds (frame cache, decoders, sessions).
    pub fn media_usage(&self) -> MediaUsage {
        self.service.usage()
    }

    /// After a render's commands are submitted: releases CoreVideo texture
    /// cache entries no longer in use, and makes converted frames this
    /// render replaced free for reuse (two are kept).
    pub fn flush(&mut self) {
        if let Some(i) = &self.importer {
            i.flush_cache();
        }
        self.free_frames.append(&mut self.retired_frames);
        let excess = self.free_frames.len().saturating_sub(2);
        self.free_frames.drain(..excess);
        self.recount();
    }
}

#[cfg(test)]
#[path = "media/source_storage_tests.rs"]
mod source_storage_tests;

#[cfg(test)]
#[path = "media/composed_parity_tests.rs"]
mod composed_parity_tests;

#[cfg(test)]
#[path = "media/round3_tests.rs"]
mod round3_tests;

#[cfg(test)]
#[path = "media/round4_tests.rs"]
mod round4_tests;

#[cfg(test)]
#[path = "media/hdr_parity_tests.rs"]
mod hdr_parity_tests;

#[cfg(test)]
mod stamp_frame_tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn same_size_edits_with_restored_mtime_invalidate_retained_sources() {
        let dir =
            std::env::temp_dir().join(format!("pm-render-source-version-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        struct Clean(PathBuf);
        impl Drop for Clean {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
        let _clean = Clean(dir.clone());
        let path = dir.join("source");
        std::fs::write(&path, b"one").unwrap();
        let before = stamp(&path).unwrap();
        let modified = std::fs::metadata(&path).unwrap().modified().unwrap();
        std::fs::write(&path, b"two").unwrap();
        std::fs::File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_times(std::fs::FileTimes::new().set_modified(modified))
            .unwrap();
        assert_eq!(
            std::fs::metadata(&path).unwrap().modified().unwrap(),
            modified
        );
        assert_ne!(stamp(&path).unwrap(), before);
    }

    #[test]
    fn a_frame_reads_each_file_once_and_the_next_frame_reads_again() {
        let dir = std::env::temp_dir().join(format!("pm-render-stamps-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        struct Clean(PathBuf);
        impl Drop for Clean {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
        let _clean = Clean(dir.clone());
        let path = dir.join("a.png");
        std::fs::write(&path, b"one").unwrap();
        let first = stamp(&path).unwrap();
        let scope = pm_perf::Scope::begin();
        {
            let _frame = StampFrame::begin();
            assert_eq!(stamp(&path), Some(first));
            // Changed during the frame: the frame keeps the state it read.
            std::fs::write(&path, b"three").unwrap();
            assert_eq!(stamp(&path), Some(first));
            {
                // A nested frame (a render inside a render) shares it.
                let _inner = StampFrame::begin();
                assert_eq!(stamp(&path), Some(first));
            }
            assert_eq!(stamp(&path), Some(first));
        }
        assert_eq!(scope.finish().get(pm_perf::Counter::FileStats), 1);
        // The next frame, and any read outside a frame, look again.
        let _frame = StampFrame::begin();
        assert_eq!(stamp(&path).map(|s| s.0), Some(5));
        drop(_frame);
        std::fs::remove_file(&path).unwrap();
        assert_eq!(stamp(&path), None);
    }
}
