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
use pm_media::clip::ClipTiming;
use pm_media::decode::scrub::{ScrubStats, ViewerDecode, Want};
use pm_media::decode::service::{
    DecodeService, MediaLease, MediaSource, MediaUsage, ServiceConfig,
};
use pm_media::frame::DecodedFrame;
use pm_media::interpret::{Interpretation, ToneMap, resolve_video};
use pm_media::pool::FramePool;
use pm_media::probe::color::{ContentLight, HdrFormat, MasteringDisplay};
use pm_media::probe::{FieldOrder, VideoProbe};
use pm_media::vt::DecodePurpose;
use pm_protocol::color::{
    AlphaMode, ChromaSiting, ColorDescription, MatrixCoefficients, Precision, Primaries, Transfer,
    WorkingEncoding,
};
use pm_protocol::common::Ticks;

use crate::capability::Feature;
use crate::footage::{Geometry, Orient, Pick, Timing};
use crate::pipelines::{Pipelines, SourceUniform, bytes_of};

#[path = "media/prewarm.rs"]
mod prewarm;
mod stills;
pub(crate) use prewarm::PlannedVideoDraw;

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
    /// Chromaticities (red, green, blue, white x/y) replacing `primaries`
    /// (PNG `cHRM`, OpenEXR): converted to Rec.709 in linear light, the
    /// white adapted to D65 (Bradford, ICC.1 Annex E).
    pub chromaticities: Option<[f64; 8]>,
    /// A pure power decoding exponent replacing `transfer` (PNG `gAMA`
    /// alone: linear = code^(1/gamma)).
    pub power: Option<f32>,
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

/// How the source pass reads a still: its colour, and whether ColorSync
/// converts the samples first.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct InterpretedStill {
    /// The source pass's reading of the texture it receives.
    pub color: SourceColor,
    /// The embedded ICC profile is neither sRGB nor Display P3: ColorSync
    /// converts the samples first (ICC.1 in float), reading their alpha this
    /// way ([`pm_media::image::imageio::convert_to_linear_srgb`]); the
    /// texture is then premultiplied linear Rec.709, as `color` says.
    pub colorsync: Option<AlphaMode>,
}

/// The facts of a still the reading needs, without its samples.
#[derive(Debug, Clone, PartialEq)]
struct StillFacts {
    color: pm_media::image::ImageColor,
    float: bool,
    /// Y′CbCr planes (AVIF).
    planar: bool,
    /// Deeper than 8 bits per sample (or float).
    deep: bool,
}

impl StillFacts {
    /// Whether a viewer frame reads a preview of this still (decoded no
    /// larger than it is drawn) instead of the whole of it. Only ordinary
    /// SDR is subsampled: deep, HDR, custom gamma or profile and interpreted
    /// sources keep their native reader and float storage.
    fn previewed(&self, interpreted: bool) -> bool {
        !self.deep
            && !self.planar
            && self.color.gamma.is_none()
            && self.color.cicp.is_none()
            && self.color.chromaticities.is_none()
            && matches!(
                self.color.icc_class,
                None | Some(pm_media::image::icc::IccClass::Srgb)
            )
            && !interpreted
    }
    fn of(still: &pm_media::image::StillImage) -> StillFacts {
        StillFacts {
            color: still.color.clone(),
            float: still.float,
            planar: still.frame.format.layout == pm_media::frame::Layout::Planar,
            deep: still.float || still.bit_depth > 8,
        }
    }
}

/// How a still is read, with Interpret Footage overrides (empty for none),
/// from its file's color facts (`pm_media::interpret::interpret_still`, the
/// reading the sheet shows):
///
/// - an ICC profile: sRGB and Display P3 by matrix; any other through
///   ColorSync (ICC.1, relative colorimetric, in float);
/// - H.273 code points (PNG `cICP`, AVIF `colr`): primaries, transfer and,
///   for Y′CbCr, the matrix; PQ and HLG decoded per BT.2100 and tone mapped
///   to SDR white (BT.2390 EETF, 203 cd/m², BT.2408) as video is;
/// - chromaticities (PNG `cHRM`, OpenEXR): converted to Rec.709 with the
///   white adapted to D65 (Bradford); PNG `gAMA` alone: a pure power;
/// - otherwise sRGB.
///
/// Every override replaces the file's reading of that value.
fn still_reading(
    facts: &StillFacts,
    interp: &Interpretation,
) -> Result<InterpretedStill, ProbeIssue> {
    let color = &facts.color;
    let unsupported = |what: String| Err(ProbeIssue::Unsupported(Feature::Media(what)));
    // Y′CbCr samples (AVIF): the matrix of their code points (a profile
    // replaces only primaries and transfer), sited as video's are.
    let matrix = if facts.planar {
        let m = color.cicp.map_or(2, |c| c[2]);
        match pm_media::interpret::still_cicp([1, 13, m, 1]) {
            Some((_, _, m)) => m,
            None => return unsupported(format!("AVIF stills with matrix coefficients {m}")),
        }
    } else {
        MatrixCoefficients::Identity
    };
    let siting = match color.chroma_location {
        Some(1) => ChromaSiting::Center,
        Some(2) => ChromaSiting::TopLeft,
        Some(3) => ChromaSiting::Top,
        Some(4) => ChromaSiting::BottomLeft,
        Some(5) => ChromaSiting::Bottom,
        // Untagged chroma is sited left (type 0), as video's.
        _ => ChromaSiting::Left,
    };
    let si = pm_media::interpret::interpret_still(color, facts.float, interp);
    let r = &si.reading;
    if si.colorsync {
        return Ok(InterpretedStill {
            color: SourceColor {
                transfer: Transfer::Linear,
                primaries: Primaries::Bt709,
                matrix,
                siting,
                alpha: match r.alpha.value {
                    AlphaMode::Opaque => AlphaMode::Opaque,
                    _ => AlphaMode::Premultiplied,
                },
                tone_map: None,
                chromaticities: None,
                power: None,
            },
            colorsync: Some(r.alpha.value),
        });
    }
    if let Some(c) = color.cicp.filter(|_| color.icc_class.is_none())
        && pm_media::interpret::still_cicp(c).is_none()
    {
        return unsupported(format!(
            "stills with color primaries {} and transfer {}",
            c[0], c[1]
        ));
    }
    let profile = color.icc_class.is_some();
    Ok(InterpretedStill {
        color: SourceColor {
            transfer: r.transfer.value,
            primaries: r.primaries.value,
            matrix,
            siting,
            alpha: r.alpha.value,
            tone_map: r.tone_map,
            chromaticities: color
                .chromaticities
                .filter(|_| interp.primaries.is_none() && !profile)
                .map(|c| c.map(f64::from)),
            power: color
                .gamma
                .filter(|_| interp.transfer.is_none() && !profile)
                .map(|g| 1.0 / g),
        },
        colorsync: None,
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

/// What a still reads as, or why it cannot be drawn: unreadable (including
/// the files today's importer rejects), animated, or outside the still
/// classes ([`crate::stills::check`]). Decoded once per size and
/// modification time; each interpretation is then resolved from its facts.
fn still_facts(path: &Path) -> Result<StillFacts, ProbeIssue> {
    static CACHE: StampCache<Result<StillFacts, ProbeIssue>> = OnceLock::new();
    per_stamp(&CACHE, path, || {
        if let Ok(Some(a)) = pm_media::image::animation(path)
            && a.frames > 1
        {
            return Err(ProbeIssue::Unsupported(Feature::ImageSequence));
        }
        crate::stills::rejected(path).map_err(ProbeIssue::Unreadable)?;
        let pool = FramePool::new(0);
        pm_perf::bump(pm_perf::Counter::StillDecodes);
        let still = pm_media::image::read(path, &pool)
            .map_err(|e| ProbeIssue::Unreadable(format!("{}: {e}", path.display())))?;
        crate::stills::check(path, &still).map_err(ProbeIssue::Unsupported)?;
        Ok(StillFacts::of(&still))
    })
    .unwrap_or_else(|| Err(ProbeIssue::Unreadable(offline(path))))
}

/// Whether an image file can be drawn: readable, not animated, in a tested
/// still class ([`crate::stills`]), with a color reading the engine
/// converts. Image layers at every level and 3D material maps read the same
/// native-depth samples.
pub fn probe_image(path: &Path) -> Result<(), ProbeIssue> {
    let facts = still_facts(path)?;
    still_reading(&facts, &Interpretation::default()).map(|_| ())
}

/// [`probe_image`] for an asset with Interpret Footage overrides.
pub fn probe_interpreted_image(path: &Path, interp: &Interpretation) -> Result<(), ProbeIssue> {
    let facts = still_facts(path)?;
    still_reading(&facts, interp).map(|_| ())
}

/// A still's reading with Interpret Footage overrides
/// ([`pm_media::interpret::interpret_still`], which the sheet shows).
pub fn interpreted_image_color(
    still: &pm_media::image::StillImage,
    interp: &Interpretation,
) -> Result<InterpretedStill, ProbeIssue> {
    still_reading(&StillFacts::of(still), interp)
}

/// Whether a still's samples are deeper than 8 bits (or HDR): its texture is
/// then never stored below half float.
pub(crate) fn deep_still(path: &Path) -> bool {
    still_facts(path).is_ok_and(|f| {
        f.deep
            || f.color
                .cicp
                .is_some_and(|c| matches!(c[1], 16 | 18) && f.color.icc_class.is_none())
    })
}

/// What the HDR media in a document holds, for delivery and the viewer.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct HdrContent {
    /// The brightest light the sources can hold, in cd/m².
    pub peak_nits: f64,
    /// The brightest source's mastering display.
    pub mastering: Option<MasteringDisplay>,
    /// The highest levels, when every HDR source states them.
    pub content_light: Option<ContentLight>,
}

/// One source's reading of its light, per file stamp.
#[derive(Clone)]
struct SourceLight {
    peak_nits: f64,
    mastering: Option<MasteringDisplay>,
    content_light: Option<ContentLight>,
}

/// Summarizes the HDR video `composition` can show, through its precomps and
/// read through Interpret Footage. Comp-wide rather than per frame, so the tone
/// map doesn't move between shots. Only headers are read, one file per thread
/// up to four, once per file stamp.
pub fn hdr_content(
    document: &pm_model::Document,
    composition: &str,
    resolver: &dyn crate::assets::AssetResolver,
) -> HdrContent {
    use pm_eval::node;
    let mut content = HdrContent::default();
    let Some(root) = document.root() else {
        return content;
    };
    let clips: Vec<_> = crate::capability::shown_assets(root, composition)
        .into_iter()
        .filter_map(|id| {
            let record = node::member(node::member(Some(root), "assets"), &id);
            if !node::is_str(node::member(record, "kind"), "video") {
                return None;
            }
            let path = resolver.path(&id, record)?;
            Some((path, record.and_then(crate::footage::interpretation)))
        })
        .collect();
    let lights: Vec<_> = std::thread::scope(|scope| {
        let workers: Vec<_> = clips
            .chunks(clips.len().div_ceil(4).max(1))
            .map(|clips| {
                scope.spawn(move || {
                    clips
                        .iter()
                        .filter_map(|(path, interpret)| source_light(path, interpret.as_ref()))
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        workers
            .into_iter()
            .flat_map(|worker| worker.join().unwrap_or_default())
            .collect()
    });
    let mut all_stated = true;
    let mut levels = (0, 0);
    for light in lights {
        if light.peak_nits > content.peak_nits {
            content.peak_nits = light.peak_nits;
            content.mastering = light.mastering;
        }
        match light.content_light {
            Some(c) => levels = (levels.0.max(c.max_cll), levels.1.max(c.max_fall)),
            None => all_stated = false,
        }
    }
    content.content_light = (all_stated && levels.0 > 0).then_some(ContentLight {
        max_cll: levels.0,
        max_fall: levels.1,
    });
    content
}

/// `None` for SDR sources and unreadable files.
fn source_light(path: &Path, interp: Option<&Interpretation>) -> Option<SourceLight> {
    static CACHE: StampCache<Option<VideoProbe>> = OnceLock::new();
    let video = per_stamp(&CACHE, path, || {
        pm_media::probe::probe(path).ok()?.probe.video
    })??;
    let none = Interpretation::default();
    let reading = resolve_video(&video, interp.unwrap_or(&none));
    match reading.transfer.value {
        Transfer::Pq | Transfer::Hlg => Some(SourceLight {
            peak_nits: reading.tone_map.map_or(1000.0, |t| t.peak_nits),
            mastering: video.color.mastering.clone(),
            content_light: video.color.content_light,
        }),
        camera if pm_color::camera_log::is_log(camera) => Some(SourceLight {
            peak_nits: (pm_color::camera_log::decode(camera, 1.0)? * pm_color::tonemap::SDR_WHITE)
                .min(10000.0),
            mastering: None,
            content_light: None,
        }),
        _ => None,
    }
}

/// Y′CbCr planes to straight R′G′B′A in the stored encoding (no transfer or
/// primaries conversion), on the CPU: chroma upsampled with section 9's
/// filters at the given siting, then the matrix. For the consumers that
/// need packed samples (glTF textures; ColorSync conversions).
pub(crate) fn ycbcr_rgba(
    still: &pm_media::image::StillImage,
    color: &SourceColor,
) -> Result<Vec<f32>, String> {
    let f = &still.frame;
    let (w, h) = (f.format.width, f.format.height);
    let (ya, cc) = f.reference().map_err(|e| e.to_string())?;
    let factors = Factors::from_log2(f.format.log2_chroma_w, f.format.log2_chroma_h);
    let (cw, ch) = factors.chroma_size(w as u32, h as u32);
    let up = |k: usize| {
        let samples: Vec<f32> = cc.iter().map(|p| p[k]).collect();
        pm_color::chroma::upsample(
            &pm_color::chroma::Plane {
                width: cw,
                height: ch,
                samples: &samples,
            },
            w as u32,
            h as u32,
            factors,
            color.siting,
        )
    };
    let (cb, cr) = (up(0), up(1));
    let rgb_planes = f.format.layout == pm_media::frame::Layout::PlanarRgb;
    let matrix = Matrix::of(color.matrix);
    let mut out = Vec::with_capacity(w * h * 4);
    for i in 0..w * h {
        let (y, b, r) = (f64::from(ya[i][0]), f64::from(cb[i]), f64::from(cr[i]));
        let rgb = if rgb_planes {
            // G, B, R planes (matrix 0).
            [r, y, b]
        } else if color.matrix == MatrixCoefficients::Bt2020Cl {
            pm_color::ycbcr::bt2020_cl_to_rgb([y, b, r])
        } else {
            matrix
                .ok_or("an unsupported matrix")?
                .ycbcr_to_rgb([y, b, r])
        };
        out.extend_from_slice(&[rgb[0] as f32, rgb[1] as f32, rgb[2] as f32, ya[i][1]]);
    }
    Ok(out)
}

/// A Y′CbCr still as packed straight R′G′B′A in float with the same color
/// facts ([`ycbcr_rgba`]), for ColorSync's conversion.
fn packed(
    still: &pm_media::image::StillImage,
    color: &SourceColor,
    pool: &FramePool,
) -> Result<pm_media::image::StillImage, String> {
    let rgba = ycbcr_rgba(still, color)?;
    let (w, h) = (still.frame.format.width, still.frame.format.height);
    let frame = pm_media::image::pool_rgba(
        pool,
        w,
        h,
        pm_media::image::Rgba::F32(&rgba),
        false,
        32,
        format!("{} as R′G′B′A", still.frame.format.label),
    )
    .map_err(|e| e.to_string())?;
    Ok(pm_media::image::StillImage {
        frame,
        bit_depth: 32,
        float: true,
        color: pm_media::image::ImageColor {
            alpha: match still.color.alpha {
                AlphaMode::Opaque => AlphaMode::Opaque,
                a => a,
            },
            ..still.color.clone()
        },
        ..still.clone()
    })
}

/// The shipped `sequencePlaybackTime`: select an available CFR slot,
/// clamp to its ends, then seek one millisecond inside it. Container video
/// instead selects directly from native source timestamps.
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

/// Whether a video file can be drawn: it opens and indexes and the renderer
/// has a reading for it ([`video_drawable`]). Every SDR, HDR, wide-gamut,
/// interlaced (bwdif, [`crate::footage::Timing`]), anamorphic and cropped
/// source goes through the one video path; docs/engine/video-parity.md.
///
/// Answers are kept per file stamp: capability routing asks at every
/// document revision, and opening a source indexes it.
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
        video_drawable(&src, &Interpretation::default())
            .map_err(|m| ProbeIssue::Unsupported(Feature::Media(m)))
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

/// Typed interpreted probe. Facts are kept per file stamp (path, size,
/// modification and change time, inode), independently
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

/// What a source monitor can't show from the decoded planes alone: fields,
/// pixel aspect, a cropped aperture, HDR light or wide-gamut primaries.
fn needs_engine_conversion(src: &MediaSource) -> bool {
    let Some(v) = src.probe.video.as_ref() else {
        return true;
    };
    let cropped = v.clean_aperture.is_some_and(|c| {
        c.width != f64::from(v.width)
            || c.height != f64::from(v.height)
            || c.horizontal_offset != 0.0
            || c.vertical_offset != 0.0
    });
    matches!(
        v.fields.order,
        FieldOrder::TopFirst | FieldOrder::BottomFirst
    ) || v.pixel_aspect.0 != v.pixel_aspect.1
        || cropped
        || is_hdr_signal(src.color.transfer)
}

/// How a video frame's code values become colour: the probe's reading
/// ([`pm_media::probe::color::video_reading`], the table the source monitor
/// shares), Interpret Footage's overrides, or the host's per-file override.
/// Every surface (playback, pause, scrub, thumbnails, export) reads a frame
/// this way. `hdr_light` keeps PQ and HLG whole for an HDR project.
fn video_color(
    v: &VideoProbe,
    reading: &pm_media::interpret::VideoReading,
    interp: &Interpretation,
    override_color: Option<ColorDescription>,
    has_alpha: bool,
    sequence: bool,
    hdr_light: bool,
) -> SourceColor {
    // The file's reading is the source monitor's (`frame_reading`, with its
    // image-sequence rule); Interpret Footage members replace their fields,
    // and the host's per-file override replaces it all.
    let file = pm_media::probe::color::frame_reading(&v.color, sequence);
    let mut desc = reading.color;
    if interp.matrix.is_none() {
        desc.matrix = file.matrix;
    }
    if interp.primaries.is_none() {
        desc.primaries = file.primaries;
    }
    let desc = override_color.unwrap_or(desc);
    SourceColor {
        transfer: desc.transfer,
        primaries: desc.primaries,
        matrix: desc.matrix,
        siting: desc.siting,
        alpha: match (has_alpha, desc.alpha) {
            (false, _) => AlphaMode::Opaque,
            (true, AlphaMode::Opaque) if interp.alpha.is_none() => AlphaMode::Straight,
            (true, other) => other,
        },
        tone_map: (is_hdr_signal(desc.transfer) && !hdr_light).then(|| {
            reading.tone_map.unwrap_or(ToneMap {
                transfer: desc.transfer,
                peak_nits: 1000.0,
            })
        }),
        chromaticities: None,
        power: None,
    }
}

const fn is_hdr_signal(transfer: Transfer) -> bool {
    matches!(transfer, Transfer::Pq | Transfer::Hlg)
}

/// Sources whose light goes past SDR white: stored in float at every depth.
const fn extended_light(transfer: Transfer) -> bool {
    is_hdr_signal(transfer) || pm_color::camera_log::is_log(transfer)
}

fn preview_size(native: (u32, u32), edge: Option<u32>) -> (u32, u32) {
    let scale = edge.map_or(1., |e| {
        (f64::from(e) / f64::from(native.0.max(native.1).max(1))).min(1.)
    });
    (
        ((f64::from(native.0) * scale).round() as u32).max(1),
        ((f64::from(native.1) * scale).round() as u32).max(1),
    )
}

/// The texture format a converted video frame is stored in: the project's
/// working format, and never below half float for deeper-than-8-bit or HDR
/// sources, so 10-bit codes and light above white are not cut to 8 bits.
fn video_storage(precision: Precision, bits: u8, hdr: bool) -> wgpu::TextureFormat {
    match precision {
        Precision::Unorm8 if bits > 8 || hdr => working_format(Precision::Float16),
        p => working_format(p),
    }
}

/// The source pass parameters for a source.
fn source_uniform(
    color: &SourceColor,
    working: WorkingEncoding,
    precision: Precision,
    chroma: Option<(Factors, u32, u32)>,
) -> SourceUniform {
    // HDR signals are tone mapped for an SDR target (section 9); an HDR
    // project's sources carry no tone map and keep their light.
    let tone = color
        .tone_map
        .filter(|t| matches!(t.transfer, Transfer::Pq | Transfer::Hlg));
    let transfer = match (color.transfer, tone) {
        (_, Some(t)) if t.transfer == Transfer::Pq => 5,
        (_, Some(_)) => 6,
        _ if color.power.is_some() => 7,
        (Transfer::DisplayE, _) => 0,
        (Transfer::Bt1886, _) => 1,
        (Transfer::SceneReferredBt709, _) => 2,
        (Transfer::Linear, _) => 3,
        (Transfer::Gamma22, _) => 4,
        (Transfer::Log100, _) => 8,
        (Transfer::Log316, _) => 9,
        // No tone map: an HDR project keeps BT.2100 light.
        (Transfer::Pq, None) => 5,
        (Transfer::Hlg, None) => 6,
        (camera, _) => pm_color::camera_log::shader_curve(camera).expect("a camera log curve"),
    };
    // The source's primaries to Rec.709 in linear light: after the tone
    // map for HDR, after the transfer for SDR.
    let chromaticities = color
        .chromaticities
        .map(|c| pm_color::primaries::Chromaticities {
            r: [c[0], c[1]],
            g: [c[2], c[3]],
            b: [c[4], c[5]],
            w: [c[6], c[7]],
        });
    let primaries = match &chromaticities {
        Some(c) => pm_color::primaries::conversion_from(c, Primaries::Bt709),
        None => pm_color::primaries::conversion(
            if color.primaries == Primaries::Unspecified {
                Primaries::Bt709
            } else {
                color.primaries
            },
            Primaries::Bt709,
        ),
    }
    .unwrap_or([[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]]);
    let hdr_signal = matches!(transfer, 5 | 6);
    let convert_primaries = !hdr_signal
        && (chromaticities.is_some()
            || color.primaries != Primaries::Bt709 && color.primaries != Primaries::Unspecified);
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
            color.power.unwrap_or(1.0),
            if tone.is_none() { 1.0 } else { 0.0 },
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

/// The source pass into `format` storage, which may be deeper than the
/// working precision ([`video_storage`]).
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
    still_chroma: bool,
    packed: Option<&pm_media::frame::PixelFormat>,
) -> Target {
    convert_format_scaled(
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
        format,
        still_chroma,
        packed,
        None,
    )
}

#[allow(clippy::too_many_arguments)]
fn convert_format_scaled(
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
    still_chroma: bool,
    packed: Option<&pm_media::frame::PixelFormat>,
    size: Option<(u32, u32)>,
) -> Target {
    let (luma, chroma, chroma_info) = match normalized {
        Normalized::Rgb { rgba } => (rgba, None, None),
        Normalized::YCbCr { luma, chroma } => (
            luma,
            Some(chroma),
            Some((factors, chroma.width(), chroma.height())),
        ),
    };
    let (w, h) = size.unwrap_or((luma.width(), luma.height()));
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
    if let Some(packed) = packed {
        // Packed stills normalize in this same pass. m0 is otherwise unused
        // for RGB; preserve the decode-in shader's exact code-value arithmetic.
        let n = packed.norm();
        uniform.a[0] = 2;
        let max = match packed.planes[0].format {
            pm_media::frame::PlaneFormat::Rgba8 => 255.0,
            pm_media::frame::PlaneFormat::Rgba16 => 65535.0,
            _ => 0.0,
        };
        uniform.m[0] = [max, n.y_off as f32, n.y_div as f32, n.a_div as f32];
    }
    if still_chroma && chroma_info.is_some() {
        // Reconstruct original AVIF stills with the full Lanczos-3 source
        // filter. Video keeps its separately qualified playback kernel.
        uniform.b[2] = if factors.x > 1 { 3 } else { 0 };
        uniform.b[3] = if factors.y > 1 { 3 } else { 0 };
    }
    if format != working_format(Precision::Float16) {
        // Only half-float storage rounds through f16; any other format
        // stores the f32 conversion directly.
        uniform.c[2] = 0;
    }
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("pm source params"),
        size: size_of::<SourceUniform>() as u64,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    queue.write_buffer(&buffer, 0, bytes_of(&uniform));
    let luma_view = luma.create_view(&Default::default());
    // RGB never samples the chroma binding (source_uniform sets kind=0).
    // Both bindings accept an unfilterable float texture, so reuse its view.
    let chroma_view =
        chroma.map_or_else(|| luma_view.clone(), |c| c.create_view(&Default::default()));
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
    /// HDR light kept (an HDR project) rather than tone mapped.
    hdr_light: bool,
    edge: Option<u32>,
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
    hdr_light: bool,
    edge: Option<u32>,
}

/// The frame cache of a renderer without a lease share: export jobs,
/// headless renders, and the viewer before the host grants a lease.
pub const DEFAULT_FRAME_CACHE: usize = 64 << 20;

/// Media state kept across frames: decoded and converted stills, video
/// sources and sessions, and the last converted frame of each source.
pub struct MediaCache {
    pool: FramePool,
    service: Arc<DecodeService>,
    lease: Mutex<MediaLease>,
    idle_media: bool,
    background: Option<prewarm::BackgroundPrepare>,
    /// Display frames decode on workers (docs/engine/scrub.md); export
    /// frames decode here, on the caller's thread.
    viewer: ViewerDecode,
    /// Interactive source frames use exact Export-purpose decode on a warm
    /// worker. Batch jobs continue to decode on their caller's thread.
    source_viewer: ViewerDecode,
    /// Scrub frames: a video whose exact frame is not decoded yet shows
    /// the nearest decoded frame while it decodes ([`Want::Scrub`]).
    pub scrub: bool,
    /// Frames while playing: until this instant a render may wait for exact
    /// frames, then video layers take stand-ins ([`Want::Playback`]).
    pub playback: Option<std::time::Instant>,
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
    image_seen: BTreeMap<ImageKey, u64>,
    image_budget: u64,
    frame_budget: u64,
    /// Stills at the view's resolution (ImageIO's filtered thumbnail).
    pub(crate) preview_media: bool,
    /// Video frames converted at the view's resolution.
    pub(crate) preview_video: bool,
    pub(crate) draw_edge: Option<u32>,
    /// Stills decoded ahead of the playhead ([`Self::prepare_image`]).
    stills: stills::StillPrefetch,
    /// Open video sources with the file's size and modification time when
    /// opened: a file replaced at the same path is reopened.
    sources: BTreeMap<PathBuf, ((u64, u128), Arc<MediaSource>)>,
    source_seen: BTreeMap<PathBuf, std::time::Instant>,
    frames: BTreeMap<FrameKey, (Target, u64)>,
    frame_epoch: u64,
    frame_seen: BTreeMap<PathBuf, u64>,
    /// The last presented frame asked of each source while scrubbing
    /// (PM_FIX_WAIT_STEP).
    scrub_last: BTreeMap<PathBuf, usize>,
    next_texture_revision: u64,
    /// "Interpret Footage" overrides by file (section 9, per-source
    /// interpretation): the color reading replaces the probe's.
    interpretations: BTreeMap<PathBuf, ColorDescription>,
    /// bwdif on normalized planes (interlaced sources), made on first use.
    deinterlacer: Option<Deinterlacer>,
    /// The display geometry pass (clean aperture, rotation), made on first use.
    orient: Option<Orient>,
    /// Bytes of converted textures held (stills and frames).
    pub bytes: u64,
    /// The render's project keeps BT.2100 light: PQ and HLG sources are not
    /// tone mapped at input ([`pm_protocol::color::ColorMode::Hdr`]).
    pub(crate) hdr_light: bool,
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
            pool: FramePool::new(8 << 20),
            viewer: ViewerDecode::new(service.clone()),
            source_viewer: ViewerDecode::with_purpose(service.clone(), DecodePurpose::Export),
            service,
            lease: Mutex::new(MediaLease {
                frame_cache: DEFAULT_FRAME_CACHE,
                decoders: 32 << 20,
            }),
            idle_media: false,
            background: None,
            scrub: false,
            playback: None,
            stand_ins: Vec::new(),
            retired_frames: Vec::new(),
            free_frames: Vec::new(),
            importer: DecodeImporter::new(device).ok(),
            images: BTreeMap::new(),
            image_seen: BTreeMap::new(),
            image_budget: 48 << 20,
            frame_budget: 24 << 20,
            preview_media: false,
            preview_video: false,
            draw_edge: None,
            stills: Default::default(),
            sources: BTreeMap::new(),
            source_seen: BTreeMap::new(),
            frames: BTreeMap::new(),
            frame_epoch: 0,
            frame_seen: BTreeMap::new(),
            scrub_last: BTreeMap::new(),
            next_texture_revision: 0,
            interpretations: BTreeMap::new(),
            deinterlacer: None,
            orient: None,
            bytes: 0,
            hdr_light: false,
        }
    }

    fn importer(&self) -> Result<&DecodeImporter, MediaFailure> {
        self.importer
            .as_ref()
            .ok_or_else(|| MediaFailure::Failed("the device cannot import decoded frames".into()))
    }

    /// The working texture of a still, read with the asset's Interpret
    /// Footage overrides when it has any: the native-depth samples
    /// ([`pm_media::image::read`]) converted in float by the source pass
    /// ([`still_reading`]), premultiplied, oriented as the file says, and
    /// stored at the project's precision but never below half float for a
    /// deep or HDR still. Every still consumer reads this one path.
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
    ) -> Result<Target, MediaFailure> {
        self.image_scaled(
            device, queue, pipelines, encoder, path, working, precision, interpret, None,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn image_scaled(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        pipelines: &Pipelines,
        encoder: &mut wgpu::CommandEncoder,
        path: &Path,
        working: WorkingEncoding,
        precision: Precision,
        interpret: Option<&Interpretation>,
        edge: Option<u32>,
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
            hdr_light: self.hdr_light,
            edge,
        };
        if let Some(t) = self.images.get(&key) {
            self.image_seen.insert(key.clone(), self.frame_epoch);
            pm_perf::bump(pm_perf::Counter::MediaCacheHits);
            return Ok(t.clone());
        }
        pm_perf::bump(pm_perf::Counter::MediaCacheMisses);
        let issue = |i: ProbeIssue| match i {
            ProbeIssue::Unsupported(f) => MediaFailure::Unsupported(f),
            ProbeIssue::Unreadable(m) => MediaFailure::Failed(m),
        };
        let failed =
            |e: pm_media::MediaError| MediaFailure::Failed(format!("{}: {e}", path.display()));
        let none = Interpretation::default();
        let interp = interpret.unwrap_or(&none);
        probe_interpreted_image(path, interp).map_err(issue)?;
        let facts = still_facts(path).map_err(issue)?;
        let ask = stills::StillAsk {
            edge,
            interpreted: interpret.is_some(),
        };
        let preview = edge.filter(|_| facts.previewed(ask.interpreted));
        // Decoded ahead by the worker as this frame reads it, or here.
        let mut still = match self.stills.take(path, (len, modified), ask, preview) {
            Some(still) => still,
            None => {
                pm_perf::bump(pm_perf::Counter::StillDecodes);
                match preview {
                    Some(edge) => pm_media::image::imageio::read_preview(path, &self.pool, edge)
                        .or_else(|_| pm_media::image::read(path, &self.pool)),
                    None => pm_media::image::read(path, &self.pool),
                }
                .map_err(failed)?
            }
        };
        let mut read = interpreted_image_color(&still, interp).map_err(issue)?;
        if self.hdr_light {
            read.color.tone_map = None;
        }
        let deep = still.float || still.bit_depth > 8 || extended_light(read.color.transfer);
        if let Some(alpha) = read.colorsync {
            if still.frame.format.layout != pm_media::frame::Layout::Rgba {
                still = packed(&still, &read.color, &self.pool).map_err(MediaFailure::Failed)?;
            }
            still = pm_media::image::imageio::convert_to_linear_srgb(&still, &self.pool, alpha)
                .map_err(failed)?;
        }
        let importer = self.importer()?;
        let imported = importer
            .import(&still.frame)
            .map_err(|e| MediaFailure::Failed(e.to_string()))?;
        let packed = (still.frame.format.layout == pm_media::frame::Layout::Rgba)
            .then_some(still.frame.format.as_ref());
        let normalized = if packed.is_some() {
            Normalized::Rgb {
                rgba: imported.textures[0].clone(),
            }
        } else {
            importer
                .normalize(queue, encoder, &imported)
                .map_err(|e| MediaFailure::Failed(e.to_string()))?
        };
        // Standard stores sRGB code values in UNORM (an sRGB GPU view
        // would decode them a second time). Opaque identity conversion is
        // lossless for these 8-bit sources; wide/linear/HDR stay in float.
        let compact = preview.is_some()
            && !deep
            && working == WorkingEncoding::Standard
            && read.color.primaries == Primaries::Bt709
            && read.color.transfer == Transfer::DisplayE
            && read.color.alpha == AlphaMode::Opaque
            && read.colorsync.is_none();
        let format = if compact {
            wgpu::TextureFormat::Rgba8Unorm
        } else {
            video_storage(precision, if deep { 16 } else { 8 }, false)
        };
        let mut target = convert_format(
            device,
            queue,
            pipelines,
            encoder,
            &normalized,
            &read.color,
            working,
            precision,
            Factors::from_log2(
                still.frame.format.log2_chroma_w,
                still.frame.format.log2_chroma_h,
            ),
            Some(Target::with_mips(
                device,
                "pm still source",
                still.frame.format.width as u32,
                still.frame.format.height as u32,
                format,
                pm_gpu::target::working_usage(),
            )),
            format,
            true,
            packed,
        );
        // The normalization is scratch. Reuse it for the next source only
        // after these reads; the queue preserves command order.
        if packed.is_none() {
            importer.recycle(normalized);
        }
        pipelines
            .resample
            .fill_mips(device, encoder, &target.texture);
        // The decoded pixels must outlive the commands reading them; the
        // imported textures hold the frame until wgpu releases them.
        drop(imported);
        if !still.oriented {
            let orient = self.orient.get_or_insert_with(|| Orient::new(device));
            target = orient.run(
                device,
                queue,
                encoder,
                &target,
                Geometry::exif(still.orientation),
                format,
            );
        }
        // Older versions of the same file are dropped.
        // (Dropped, not destroyed: a frame being recorded may still use it.)
        self.images.retain(|k, _| {
            k.path != key.path
                || k.precision != key.precision
                || k.linear != key.linear
                || k.hdr_light != key.hdr_light
        });
        self.image_seen.insert(key.clone(), self.frame_epoch);
        self.images.insert(key, target.clone());
        self.trim_images(self.image_budget);
        self.recount();
        Ok(target)
    }

    /// Decodes the still at `path` on a worker when no converted texture of
    /// it is cached, so the frame that first draws it does not (see
    /// `media/stills.rs`). `edge` is the edge its draw reads it at
    /// (`exec::source_edge`): a viewer frame's still is decoded as a preview
    /// of that size, never whole (the decode a preview exists to avoid).
    pub(crate) fn prepare_image(&mut self, path: &Path, edge: Option<u32>, interpreted: bool) {
        let edge = edge.filter(|_| self.preview_media);
        let Some((len, modified)) = stamp(path) else {
            return;
        };
        if self
            .images
            .keys()
            .any(|k| k.path == path && k.len == len && k.modified == modified && k.edge == edge)
        {
            return;
        }
        let ask = stills::StillAsk { edge, interpreted };
        self.stills.request(path, (len, modified), ask, &self.pool);
    }

    /// Waits for the stills asked for ahead (headless tests and performance
    /// runs; a frame never waits).
    pub(crate) fn wait_for_stills(&mut self) {
        self.stills.settle();
    }

    /// Drops the converted texture of `path` at `precision` and `working`
    /// without overrides (a consumer that keeps its own copy, as 3D maps do).
    pub(crate) fn release_image(
        &mut self,
        path: &Path,
        precision: Precision,
        working: WorkingEncoding,
    ) {
        self.images.retain(|k, _| {
            k.path != path
                || k.precision != precision as u8
                || k.linear != (working == WorkingEncoding::LinearLight)
                || !k.reading.is_empty()
        });
        self.recount();
    }

    fn source(&mut self, path: &Path) -> Result<Arc<MediaSource>, MediaFailure> {
        self.frame_seen.insert(path.to_path_buf(), self.frame_epoch);
        self.source_seen
            .insert(path.to_path_buf(), std::time::Instant::now());
        self.drain_prepared();
        let stamp = stamp(path)
            .ok_or_else(|| MediaFailure::Failed(format!("{} is offline", path.display())))?;
        match self.sources.get(path) {
            Some((s, src)) if *s == stamp => return Ok(src.clone()),
            Some(_) => {
                // Replaced on disk: drop the stale index and frames.
                if let Some((_, old)) = self.sources.remove(path) {
                    self.viewer.release(Some(old.id));
                    self.source_viewer.release(Some(old.id));
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

    /// Display dimensions of an already converted source texture. Crop and
    /// rotation are in the texture; pixel aspect remains a presentation scale.
    /// Uses the source just decoded, without another file stat or probe.
    pub(crate) fn source_display_size(
        &self,
        path: &Path,
        interpret: Option<&Interpretation>,
        size: (u32, u32),
    ) -> (f64, f64) {
        let Some((_, source)) = self.sources.get(path) else {
            return (size.0 as f64, size.1 as f64);
        };
        let Some(video) = source.probe.video.as_ref() else {
            return (size.0 as f64, size.1 as f64);
        };
        let reading = resolve_video(video, interpret.unwrap_or(&Interpretation::default()));
        let (n, d) = reading.pixel_aspect.value;
        let ratio = n as f64 / d as f64;
        if reading.quarter_turns() % 2 == 1 {
            (size.0 as f64, (size.1 as f64 * ratio).round().max(1.0))
        } else {
            ((size.0 as f64 * ratio).round().max(1.0), size.1 as f64)
        }
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
    ) -> Result<Option<Target>, MediaFailure> {
        self.video_inner(
            device,
            queue,
            pipelines,
            encoder,
            path,
            instance,
            seconds,
            export,
            working,
            precision,
            interpret,
            Some(composition),
            sequence,
            false,
        )
    }

    /// Source-only conversion has no composition cadence: select all source
    /// fields. Interactive requests keep the exact Export worker warm;
    /// admitted job batches decode directly without worker look-ahead.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn source_video(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        pipelines: &Pipelines,
        encoder: &mut wgpu::CommandEncoder,
        path: &Path,
        instance: u64,
        seconds: f64,
        working: WorkingEncoding,
        precision: Precision,
        interpret: Option<&Interpretation>,
        sequence: bool,
        exact_worker: bool,
    ) -> Result<Option<Target>, MediaFailure> {
        self.video_inner(
            device,
            queue,
            pipelines,
            encoder,
            path,
            instance,
            seconds,
            true,
            working,
            precision,
            interpret,
            None,
            sequence,
            exact_worker,
        )
    }

    /// The original decoded code planes for the source monitor's video
    /// presenter, from the exact Export worker shared with RGBA source
    /// frames. Composition decode is unaffected. `None` when the source needs
    /// the engine's own conversion instead (`presentable` refuses it, or it
    /// needs more than a matrix and tags: rotation, fields, pixel aspect,
    /// cropping, HDR or wide gamut).
    pub(crate) fn source_exact_decoded(
        &mut self,
        path: &Path,
        time: Ticks,
        clip: ClipTiming,
        presentable: impl FnOnce(&MediaSource) -> bool,
    ) -> Result<Option<(Arc<MediaSource>, DecodedFrame)>, MediaFailure> {
        let source = self.source(path)?;
        // What the engine cannot draw at all fails as composition would.
        video_drawable(&source, &Interpretation::default())
            .map_err(|m| MediaFailure::Unsupported(Feature::Media(m)))?;
        if needs_engine_conversion(&source) || !presentable(&source) {
            return Ok(None);
        }
        let frame = source
            .select_as(clip, time.get())
            .ok_or_else(|| MediaFailure::Failed("No source frame at this time".into()))?;
        let fetched = self
            .source_viewer
            .fetch(&source, 0, frame, Want::Exact)
            .map_err(|e| MediaFailure::Failed(e.to_string()))?;
        if !fetched.exact || fetched.shown != frame {
            return Err(MediaFailure::Failed(
                "The source decoder returned a non-exact frame".into(),
            ));
        }
        Ok(Some((source, fetched.frame)))
    }

    #[allow(clippy::too_many_arguments)]
    fn video_inner(
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
        composition: Option<pm_time::Rate>,
        sequence: bool,
        source_exact: bool,
    ) -> Result<Option<Target>, MediaFailure> {
        let src = self.source(path)?;
        let none = Interpretation::default();
        let interp = interpret.unwrap_or(&none);
        video_drawable(&src, interp).map_err(|m| MediaFailure::Unsupported(Feature::Media(m)))?;
        let v = src
            .probe
            .video
            .as_ref()
            .ok_or_else(|| MediaFailure::Failed("no video stream".into()))?;
        let reading = resolve_video(v, interp);
        let timing = match composition {
            Some(rate) => Timing::new(&src, &reading, rate),
            None => Timing::source(&reading),
        };
        let ticks = (seconds * pm_time::TICKS_PER_SECOND as f64).round() as i64;
        let Some(Pick { frame: n, second }) = timing.pick(&src, ticks) else {
            return Ok(None);
        };
        let override_color = self.interpretations.get(path).copied();
        let reading_key = {
            use std::hash::{Hash, Hasher};
            let mut h = std::collections::hash_map::DefaultHasher::new();
            format!("{interp:?}{composition:?}{sequence}{override_color:?}").hash(&mut h);
            h.finish()
        };
        let edge = if self.preview_video
            && !export
            && !source_exact
            && Geometry::of(&reading, (v.width, v.height)).is_identity()
            && !timing.deinterlaced()
        {
            self.draw_edge
        } else {
            None
        };
        let key_of = |frame: usize| FrameKey {
            path: path.to_path_buf(),
            frame,
            second,
            precision: precision as u8,
            export,
            reading: reading_key,
            linear: working == WorkingEncoding::LinearLight,
            hdr_light: self.hdr_light,
            edge,
        };
        // The newest converted frame of the source (a paused redraw).
        if let Some((t, _)) = self.frames.get(&key_of(n)) {
            pm_perf::bump(pm_perf::Counter::MediaCacheHits);
            return Ok(Some(t.clone()));
        }
        // Export frames, and deinterlaced frames (the current frame and its
        // neighbors: random access includes neighboring fields), decode on
        // this thread; other display frames come from the scrub workers.
        // Both decode the same pixels: one decoder policy serves playback,
        // pause, scrub and export.
        let deinterlaced = timing.deinterlaced();
        let (n, frame, fields) = if source_exact && !deinterlaced {
            let fetched = self
                .source_viewer
                .fetch(&src, instance, n, Want::Exact)
                .map_err(|e| MediaFailure::Failed(e.to_string()))?;
            (n, fetched.frame, None)
        } else if export || deinterlaced {
            let purpose = if export {
                DecodePurpose::Export
            } else {
                DecodePurpose::Display
            };
            let mut session = self
                .service
                .session(&src, instance, purpose)
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
            let want = match self.playback {
                Some(deadline) => {
                    Want::Playback(deadline.saturating_duration_since(std::time::Instant::now()))
                }
                // PM_FIX_SCRUB_WAIT_MS: a drag waits this long for the exact
                // frame before a stand-in, as playback does.
                None if self.scrub => {
                    // PM_FIX_WAIT_STEP: wait only for a frame that continues
                    // the drag's run (one or two frames on); a jump stands in.
                    let step = self.scrub_last.insert(path.to_path_buf(), n);
                    let continues = step.is_some_and(|last| n > last && n - last <= 2);
                    let video2 = pm_media::decode::diag::knob("PM_FIX_VIDEO2") == Some(1);
                    let gated = video2 || pm_media::decode::diag::knob("PM_FIX_WAIT_STEP") == Some(1);
                    match pm_media::decode::diag::knob("PM_FIX_SCRUB_WAIT_MS").or(video2.then_some(8)) {
                        Some(ms) if ms > 0 && (!gated || continues) => {
                            Want::Playback(std::time::Duration::from_millis(ms))
                        }
                        _ => Want::Scrub,
                    }
                }
                None => Want::Exact,
            };
            let fetched = self
                .viewer
                .fetch(&src, instance, n, want)
                .map_err(|e| MediaFailure::Failed(e.to_string()))?;
            if !fetched.exact {
                self.stand_ins.push(fetched.shown);
            }
            (fetched.shown, fetched.frame, None)
        };
        // A stand-in may be the frame converted last.
        let key = key_of(n);
        if let Some((t, _)) = self.frames.get(&key) {
            pm_perf::bump(pm_perf::Counter::MediaCacheHits);
            return Ok(Some(t.clone()));
        }
        pm_perf::bump(pm_perf::Counter::MediaCacheMisses);
        let color = video_color(
            v,
            &reading,
            interp,
            override_color,
            frame.format.has_alpha(),
            sequence,
            self.hdr_light,
        );
        if self.deinterlacer.is_none() && fields.is_some() {
            self.deinterlacer = Some(Deinterlacer::new(device));
        }
        let preview_edge = key_of(n).edge;
        let compact = preview_edge.is_some()
            && frame.format.bits <= 8
            && !self.hdr_light
            && working == WorkingEncoding::Standard
            && color.primaries == Primaries::Bt709
            && !extended_light(color.transfer);
        let format = if compact {
            wgpu::TextureFormat::Rgba8Unorm
        } else {
            video_storage(precision, frame.format.bits, extended_light(color.transfer))
        };
        let factors = Factors::from_log2(frame.format.log2_chroma_w, frame.format.log2_chroma_h);
        // A converted frame retired by an earlier render (its reads are
        // submitted) is overwritten instead of allocating another.
        let native = (frame.format.width as u32, frame.format.height as u32);
        let (w, h) = preview_size(native, preview_edge);
        let reuse = self
            .free_frames
            .iter()
            .position(|t| t.width() == w && t.height() == h && t.format() == format)
            .map(|i| self.free_frames.swap_remove(i));
        let importer = self.importer()?;
        let import = |f: &pm_media::frame::DecodedFrame| {
            importer
                .import(f)
                .map_err(|e| MediaFailure::Failed(e.to_string()))
        };
        let imported = import(&frame)?;
        let mut held = vec![];
        // The importer's normalizations to hand back once recorded.
        let mut recycled = vec![];
        let normalized = match &fields {
            Some(f) => {
                let prev = import(&f.prev)?;
                let next = import(&f.next)?;
                let norm = |i: &pm_gpu::decode_in::ImportedFrame, e: &mut wgpu::CommandEncoder| {
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
                let pass = FieldPass::new(timing.fields.top_field_first(), second, f.first, f.last);
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
        let converted = convert_format_scaled(
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
            false,
            None,
            Some((w, h)),
        );
        if fields.is_none() {
            importer.recycle(normalized);
        }
        for n in recycled {
            importer.recycle(n);
        }
        let geometry = Geometry::of(&reading, (converted.width(), converted.height()));
        let target = if geometry.is_identity() {
            converted
        } else {
            let orient = self.orient.get_or_insert_with(|| Orient::new(device));
            orient.run(
                device,
                queue,
                encoder,
                &converted,
                geometry,
                working_format(precision),
            )
        };
        drop(imported);
        drop(held);
        // Keep only the newest frame per source and precision. The older
        // one may still be read by this render: it is reused from the next.
        let mut retired = Vec::new();
        self.frames.retain(|k, (t, _)| {
            let keep = k.path != key.path
                || k.precision != key.precision
                || k.export != key.export
                || k.linear != key.linear
                || k.hdr_light != key.hdr_light;
            if !keep {
                retired.push(t.clone());
            }
            keep
        });
        self.retired_frames.extend(retired);
        self.next_texture_revision += 1;
        self.frames
            .insert(key, (target.clone(), self.next_texture_revision));
        self.recount();
        Ok(Some(target))
    }

    /// Whether a per-file colour override ([`Self::interpret`]) reads `path`.
    pub(crate) fn overrides_color(&self, path: &Path) -> bool {
        self.interpretations.contains_key(path)
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

    /// Bytes of converted textures held (images and decoded frames).
    pub fn texture_bytes(&self) -> u64 {
        self.bytes
    }

    pub fn trim_images(&mut self, budget: u64) {
        self.image_budget = budget;
        self.image_seen
            .retain(|key, _| self.images.contains_key(key));
        while self.image_bytes() > budget {
            let Some(key) = self
                .image_seen
                .iter()
                .min_by_key(|(_, at)| **at)
                .map(|(k, _)| k.clone())
            else {
                break;
            };
            self.images.remove(&key);
            self.image_seen.remove(&key);
        }
        self.recount();
    }

    pub fn trim_frames(&mut self, budget: u64) {
        self.frame_budget = budget;
        self.free_frames.clear();
        self.retired_frames.clear();
        while self.frame_bytes() > budget {
            let Some(key) = self
                .frames
                .keys()
                .min_by_key(|k| self.frame_seen.get(&k.path).copied().unwrap_or(0))
                .cloned()
            else {
                break;
            };
            self.frames.remove(&key);
        }
        self.recount();
    }

    pub fn image_bytes(&self) -> u64 {
        self.images.values().map(Target::bytes).sum()
    }

    pub fn frame_bytes(&self) -> u64 {
        self.frames.values().map(|(t, _)| t.bytes()).sum::<u64>()
            + self
                .retired_frames
                .iter()
                .chain(&self.free_frames)
                .map(Target::bytes)
                .sum::<u64>()
    }

    /// A drag frame is drawing (PM_FIX_VIDEO2's scrub-only retention).
    pub(crate) fn note_scrub(&self) {
        self.service.note_scrub();
    }
    pub(crate) fn scrubbed_within(&self, within: std::time::Duration) -> bool {
        self.service.scrubbed_within(within)
    }
    pub(crate) fn clear_scrub_mark(&self) {
        self.service.clear_scrub_mark();
    }

    /// Bytes of converted textures held.
    fn recount(&mut self) {
        self.bytes = self
            .images
            .values()
            .chain(self.frames.values().map(|(target, _)| target))
            .chain(&self.retired_frames)
            .chain(&self.free_frames)
            .map(Target::bytes)
            .sum();
    }

    /// The converted texels change only on a cache miss, including when a
    /// retired texture is overwritten. A repeated or held video frame keeps
    /// its revision, so downstream mip texels remain reusable.
    pub(crate) fn texture_revision(&self, texture: &wgpu::Texture) -> Option<u64> {
        self.frames
            .values()
            .find_map(|(target, revision)| (&target.texture == texture).then_some(*revision))
    }

    /// Changes whenever a display frame a scrub asked for has decoded.
    pub fn landed(&self) -> u64 {
        self.viewer.landed()
    }

    /// The display decode workers' counters.
    pub fn scrub_stats(&self) -> ScrubStats {
        self.viewer.stats()
    }

    /// Exact Export-purpose source decoder counters, separate from the
    /// composition viewer and its Display-purpose cache.
    pub fn source_scrub_stats(&self) -> ScrubStats {
        self.source_viewer.stats()
    }

    /// Applies the renderer's shares of the engine's lease to its decode
    /// service: the frame cache evicts down to `lease.frame_cache` at once
    /// and idle decoders close past `lease.decoders`. Returns what the
    /// service holds afterwards.
    pub fn apply_lease(&self, lease: MediaLease) -> MediaUsage {
        *self.lease.lock().unwrap() = lease;
        self.service.apply_lease(lease)
    }

    /// What the decode service holds (frame cache, decoders, sessions).
    pub fn media_usage(&self) -> MediaUsage {
        self.service.usage()
    }

    pub(crate) fn begin_frame(&mut self) {
        if self.idle_media {
            self.service.apply_lease(*self.lease.lock().unwrap());
            self.idle_media = false;
        }
        self.frame_epoch = self.frame_epoch.wrapping_add(1);
        let frame = self.frame_epoch;
        self.frames.retain(|key, _| {
            self.frame_seen
                .get(&key.path)
                .is_some_and(|seen| frame.wrapping_sub(*seen) <= 30)
        });
        self.frame_seen
            .retain(|_, seen| frame.wrapping_sub(*seen) <= 30);
        self.recount();
    }

    /// Queue-owned GPU references remain valid; this only drops cache handles.
    pub fn release_idle_frames(&mut self) {
        self.frames.clear();
        self.retired_frames.clear();
        self.free_frames.clear();
        self.frame_seen.clear();
        self.viewer.release(None);
        self.source_viewer.release(None);
        self.service.release(None, None);
        self.service.apply_lease(MediaLease {
            frame_cache: 0,
            decoders: 0,
        });
        self.idle_media = true;
        self.service.pool().trim(0);
        self.recount();
    }

    /// Plans and their two-second look-ahead touch wanted sources. Close
    /// decoders for clips no recent plan wants; retain only their cheap index.
    pub(crate) fn retire_cold_sessions(&mut self) {
        let cold: Vec<_> = self
            .source_seen
            .iter()
            .filter(|(_, at)| at.elapsed() >= std::time::Duration::from_secs(2))
            .map(|(p, _)| p.clone())
            .collect();
        for path in cold {
            self.source_seen.remove(&path);
            if let Some((_, source)) = self.sources.get(&path) {
                self.viewer.release(Some(source.id));
                self.source_viewer.release(Some(source.id));
                self.service.release(None, Some(source.id));
            }
        }
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
        while self.free_frames.iter().map(Target::bytes).sum::<u64>() > 16 << 20 {
            self.free_frames.pop();
        }
        self.recount();
        if self.frame_bytes() > self.frame_budget {
            self.trim_frames(self.frame_budget);
        }
    }
}

#[cfg(test)]
#[path = "media/source_storage_tests.rs"]
mod source_storage_tests;

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
